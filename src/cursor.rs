//! Pointer cursor: themed images from xcursor, or a client-provided surface.
use std::collections::HashMap;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                render_elements,
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            },
            gles::GlesRenderer,
        },
    },
    input::pointer::{CursorIcon, CursorImageAttributes, CursorImageStatus},
    output::Output,
    utils::{Logical, Physical, Point, Transform},
    wayland::compositor::with_states,
};
use xcursor::{CursorTheme, parser::parse_xcursor};

use crate::State;

render_elements! {
    pub CursorElement<=GlesRenderer>;
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
}

struct Loaded {
    buffer: MemoryRenderBuffer,
    hotspot: Point<i32, Logical>,
}

/// The cursor theme and size from the GTK settings, for sessions that only configure them there.
fn gtk_cursor_settings() -> (Option<String>, Option<String>) {
    let config = std::env::var("XDG_CONFIG_HOME").ok().filter(|v| !v.is_empty()).or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.config")));
    let Some(text) = config.and_then(|dir| std::fs::read_to_string(format!("{dir}/gtk-3.0/settings.ini")).ok()) else { return (None, None) };
    let value = |key: &str| {
        text.lines().find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name.trim() == key).then(|| value.trim().to_owned()).filter(|v| !v.is_empty())
        })
    };
    (value("gtk-cursor-theme-name"), value("gtk-cursor-theme-size"))
}

/// Export `XCURSOR_THEME` and `XCURSOR_SIZE` (from the GTK settings when unset) so that this
/// compositor, Xwayland and everything started from here draw the same cursor.
///
/// # Safety
/// Changes the process environment: call while no other thread runs.
pub unsafe fn export_theme() {
    let (theme, size) = gtk_cursor_settings();
    for (name, value) in [("XCURSOR_THEME", theme), ("XCURSOR_SIZE", size)] {
        if std::env::var_os(name).is_none()
            && let Some(value) = value
        {
            tracing::info!("{name}={value} (from the GTK settings)");
            // SAFETY: the caller guarantees a single thread.
            unsafe { std::env::set_var(name, value) };
        }
    }
}

/// Cursor images of the configured theme, loaded on first use.
pub struct CursorAssets {
    theme: CursorTheme,
    size: u32,
    cache: HashMap<&'static str, Loaded>,
}

impl CursorAssets {
    pub fn new() -> Self {
        let name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
        let size = std::env::var("XCURSOR_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(24);
        Self { theme: CursorTheme::load(&name), size, cache: HashMap::new() }
    }

    fn load(&self, icon: CursorIcon) -> Option<Loaded> {
        let path = std::iter::once(icon.name())
            .chain(icon.alt_names().iter().copied())
            .chain(["default", "left_ptr"])
            .find_map(|name| self.theme.load_icon(name))?;
        let images = parse_xcursor(&std::fs::read(path).ok()?)?;
        let nearest = images.iter().min_by_key(|image| (self.size as i32 - image.size as i32).abs())?;
        // The first frame; animated cursors stay still.
        let image = images.iter().find(|image| image.size == nearest.size)?;
        Some(Loaded {
            buffer: MemoryRenderBuffer::from_slice(
                &image.pixels_rgba,
                Fourcc::Abgr8888,
                (image.width as i32, image.height as i32),
                1,
                Transform::Normal,
                None,
            ),
            hotspot: (image.xhot as i32, image.yhot as i32).into(),
        })
    }

    /// A plain arrow for systems without any cursor theme.
    fn fallback() -> Loaded {
        const W: usize = 12;
        const H: usize = 19;
        let mut pixels = vec![0u8; W * H * 4];
        for y in 0..H.min(W + 7) {
            for x in 0..=(y / 2).min(W - 1) {
                let edge = x == 0 || x == (y / 2).min(W - 1) || y == H - 1;
                let color = if edge { [0, 0, 0, 255] } else { [255, 255, 255, 255] };
                pixels[(y * W + x) * 4..][..4].copy_from_slice(&color);
            }
        }
        Loaded {
            buffer: MemoryRenderBuffer::from_slice(&pixels, Fourcc::Abgr8888, (W as i32, H as i32), 1, Transform::Normal, None),
            hotspot: (0, 0).into(),
        }
    }

    fn ensure(&mut self, icon: CursorIcon) {
        let key = icon.name();
        if !self.cache.contains_key(key) {
            let loaded = self.load(icon).unwrap_or_else(|| {
                tracing::warn!("no cursor image for {key:?}; using a plain arrow");
                Self::fallback()
            });
            self.cache.insert(key, loaded);
        }
    }

    fn get(&self, icon: CursorIcon) -> &Loaded {
        &self.cache[icon.name()]
    }
}

impl State {
    /// Cursor elements for `output`: empty when the pointer is elsewhere or hidden.
    pub fn cursor_elements(&mut self, renderer: &mut GlesRenderer, output: &Output) -> Vec<CursorElement> {
        let Some(geo) = self.space.output_geometry(output) else { return Vec::new() };
        let scale = output.current_scale().fractional_scale();
        let to_physical = |p: Point<f64, Logical>, hotspot: Point<i32, Logical>| -> Point<f64, Physical> {
            ((p.x - f64::from(geo.loc.x) - f64::from(hotspot.x)) * scale, (p.y - f64::from(geo.loc.y) - f64::from(hotspot.y)) * scale).into()
        };
        let pointer = self.pointer_location;
        // A cursor further outside than any image is large cannot show up here.
        let near = pointer.x > f64::from(geo.loc.x - 256)
            && pointer.x < f64::from(geo.loc.x + geo.size.w + 256)
            && pointer.y > f64::from(geo.loc.y - 256)
            && pointer.y < f64::from(geo.loc.y + geo.size.h + 256);
        if !near {
            return Vec::new();
        }
        // A client that went away (or dropped its cursor surface) must not leave an invisible cursor.
        if matches!(&self.cursor_status, CursorImageStatus::Surface(surface) if !smithay::reexports::wayland_server::Resource::is_alive(surface)) {
            self.cursor_status = CursorImageStatus::default_named();
        }
        match self.cursor_status.clone() {
            CursorImageStatus::Hidden => Vec::new(),
            CursorImageStatus::Surface(surface) => {
                let hotspot = with_states(&surface, |states| {
                    states
                        .data_map
                        .get::<std::sync::Mutex<CursorImageAttributes>>()
                        .map(|attrs| attrs.lock().unwrap().hotspot)
                        .unwrap_or_default()
                });
                let location = to_physical(pointer, hotspot).to_i32_round();
                render_elements_from_surface_tree(renderer, &surface, location, scale, 1.0, Kind::Cursor)
            }
            CursorImageStatus::Named(icon) => {
                self.cursor_assets.ensure(icon);
                let loaded = self.cursor_assets.get(icon);
                MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    to_physical(pointer, loaded.hotspot),
                    &loaded.buffer,
                    None,
                    None,
                    None,
                    Kind::Cursor,
                )
                .map(|element| vec![CursorElement::from(element)])
                .unwrap_or_default()
            }
        }
    }
}
