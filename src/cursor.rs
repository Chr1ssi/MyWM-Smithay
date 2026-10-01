//! Pointer cursor: themed images from xcursor, or a client-provided surface.
use std::collections::HashMap;

use smithay::{
    reexports::wayland_server::Resource,
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

/// The memory format of the cursor images: `Argb8888` is the layout of Xcursor's pixels (B, G, R, A, premultiplied)
/// and the only one the DRM cursor plane takes; another format keeps the cursor on the primary plane.
const CURSOR_FORMAT: Fourcc = Fourcc::Argb8888;

/// Cursors of the X cursor font that Xwayland hands over as plain images: a client that makes its cursors
/// with `XCreateFontCursor` (Steam does) bypasses the theme. Image size and hotspot of such a cursor, and
/// the themed icon to draw instead.
const CORE_CURSORS: &[CoreCursor] = &[((10, 16), (1, 1), CursorIcon::Default)];

/// Size, hotspot and the themed icon of one such cursor.
type CoreCursor = ((i32, i32), (i32, i32), CursorIcon);

/// The themed icon for an Xwayland cursor image that is one of the X cursor font's glyphs.
pub fn core_cursor_icon(size: (i32, i32), hotspot: (i32, i32)) -> Option<CursorIcon> {
    CORE_CURSORS.iter().find(|(s, h, _)| *s == size && *h == hotspot).map(|(_, _, icon)| *icon)
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
            // Despite its name `pixels_rgba` holds the file's bytes: B, G, R, A, premultiplied, i.e. the memory layout
            // of `Argb8888`. (Declared as `Abgr8888` the colors of a colored theme are swapped, and the DRM
            // cursor plane, which only takes `Argb8888` buffers, cannot be used: the cursor then sits on the
            // primary plane and a game cannot be scanned out directly.)
            buffer: MemoryRenderBuffer::from_slice(
                &image.pixels_rgba,
                CURSOR_FORMAT,
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
            buffer: MemoryRenderBuffer::from_slice(&pixels, CURSOR_FORMAT, (W as i32, H as i32), 1, Transform::Normal, None),
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
    /// The themed icon to draw instead of an Xwayland cursor image that is a glyph of the X cursor font.
    pub fn core_cursor_of(&self, image: &CursorImageStatus) -> Option<CursorIcon> {
        let CursorImageStatus::Surface(surface) = image else { return None };
        let from_xwayland = self
            .display_handle
            .get_client(surface.id())
            .ok()
            .is_some_and(|client| client.get_data::<smithay::xwayland::XWaylandClientData>().is_some());
        if !from_xwayland {
            return None;
        }
        let size = smithay::backend::renderer::utils::with_renderer_surface_state(surface, |state| state.buffer_size()).flatten()?;
        let hotspot = with_states(surface, |states| {
            states.data_map.get::<std::sync::Mutex<CursorImageAttributes>>().map(|attrs| attrs.lock().unwrap().hotspot).unwrap_or_default()
        });
        core_cursor_icon((size.w, size.h), (hotspot.x, hotspot.y))
    }

    /// What is drawn: the cursor the client set, or the themed icon for a core X cursor.
    fn effective_cursor(&self) -> CursorImageStatus {
        match self.core_cursor_of(&self.cursor_status) {
            Some(icon) => CursorImageStatus::Named(icon),
            None => self.cursor_status.clone(),
        }
    }

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
        match self.effective_cursor() {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The DRM cursor plane takes `Argb8888` only: a cursor buffer of any other format stays on the primary plane.
    #[test]
    fn cursor_buffers_are_argb8888() {
        assert_eq!(CURSOR_FORMAT, Fourcc::Argb8888);
    }

    #[test]
    fn the_x_core_arrow_becomes_the_themed_default() {
        assert_eq!(core_cursor_icon((10, 16), (1, 1)), Some(CursorIcon::Default));
        // The theme's own arrow (24x24) and other sizes are left alone.
        assert_eq!(core_cursor_icon((24, 24), (5, 1)), None);
        assert_eq!(core_cursor_icon((10, 16), (4, 4)), None);
    }
}
