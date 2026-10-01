//! Built-in screenshots: drag a region, click a window, or take a whole output or the
//! focused window with one key. The picture is saved as a PNG and copied to the clipboard.
//!
//! Selecting dims the outputs with an overlay (never part of the picture). The shot is taken
//! at the next redraw of the output, from the scene without cursor and overlay.
use std::{io::BufWriter, path::PathBuf};

use smithay::{
    backend::{
        input::ButtonState,
        renderer::{
            damage::OutputDamageTracker,
            element::{Id, Kind, solid::SolidColorRenderElement},
            gles::GlesRenderer,
            utils::CommitCounter,
        },
    },
    output::Output,
    utils::{Logical, Physical, Point, Rectangle},
};

use crate::{State, render::OutputElement, screencopy::render_to_pixels};

const BTN_LEFT: u32 = 0x110;
/// A drag shorter than this (in logical pixels) counts as a click.
const CLICK_SLOP: f64 = 4.0;

/// Interactive selection in progress.
pub struct Selecting {
    /// Where the button went down; `None` while hovering.
    anchor: Option<Point<f64, Logical>>,
    purpose: Purpose,
}

/// Why the user is picking something on screen.
enum Purpose {
    Screenshot,
    /// A screen-sharing portal asked (IPC client `client`) for a monitor or window.
    Share { client: u64, kinds: mywm_ipc::SourceKinds },
}

/// A shot waiting for the next frame of its output.
pub struct PendingShot {
    output: Output,
    /// In pixels of the output as displayed.
    region: Rectangle<i32, Physical>,
}

/// What a key does while selecting.
#[derive(Clone, Copy)]
pub enum ModalKey {
    Cancel,
    Confirm,
    Left,
    Right,
    Up,
    Down,
    Ignore,
}

impl PendingShot {
    pub fn output(&self) -> &Output {
        &self.output
    }
}

impl State {
    pub fn start_selection(&mut self) {
        if self.selecting.is_some() {
            return self.cancel_selection();
        }
        self.selecting = Some(Selecting { anchor: None, purpose: Purpose::Screenshot });
        self.queue_redraw_all();
    }

    /// `choose-source` from the screen-sharing portal: pick a monitor or window to share.
    pub fn start_share_chooser(&mut self, client: u64, kinds: mywm_ipc::SourceKinds) -> bool {
        if self.selecting.is_some() || self.overview.is_some() || self.session_lock.is_active() {
            return false;
        }
        self.selecting = Some(Selecting { anchor: None, purpose: Purpose::Share { client, kinds } });
        self.queue_redraw_all();
        true
    }

    /// End the selection; a screen-sharing request is answered with `chosen`.
    fn finish_selection(&mut self, chosen: Option<mywm_ipc::Chosen>) {
        if let Some(Selecting { purpose: Purpose::Share { client, .. }, .. }) = self.selecting.take() {
            self.ipc_send_chosen(client, &chosen.unwrap_or(mywm_ipc::Chosen::Nothing));
        }
        self.queue_redraw_all();
    }

    /// What a click (or Enter) at the pointer picks for screen sharing; `None` if nothing allowed is there.
    fn share_choice(&self, kinds: mywm_ipc::SourceKinds, enter: bool) -> Option<mywm_ipc::Chosen> {
        if kinds.windows() && !enter {
            let (window, _) = self.window_at(self.pointer_location)?;
            if let Some(id) = self.desktop.windows.iter().find(|m| m.window == window).and_then(|m| m.foreign.as_ref()) {
                return Some(mywm_ipc::Chosen::Window(id.identifier()));
            }
        }
        if kinds.monitors() {
            let output = self.space.output_under(self.pointer_location).next()?;
            return Some(mywm_ipc::Chosen::Monitor(output.name()));
        }
        None
    }

    pub fn cancel_selection(&mut self) {
        self.finish_selection(None);
    }

    pub fn selection_moved(&mut self) {
        self.queue_redraw_all();
    }

    pub fn selection_key(&mut self, key: ModalKey) {
        match key {
            ModalKey::Cancel => self.cancel_selection(),
            ModalKey::Confirm => match self.selecting.as_ref().map(|s| &s.purpose) {
                Some(Purpose::Share { kinds, .. }) => {
                    let kinds = *kinds;
                    // Enter shares the monitor under the pointer.
                    if let Some(choice) = self.share_choice(kinds, true) {
                        self.finish_selection(Some(choice));
                    }
                }
                _ => {
                    self.selecting = None;
                    self.queue_redraw_all();
                    self.shoot_output_under_pointer();
                }
            },
            ModalKey::Ignore | ModalKey::Left | ModalKey::Right | ModalKey::Up | ModalKey::Down => {}
        }
    }

    /// Pointer button while selecting: left drags out a region or clicks a window, others cancel.
    pub fn selection_button(&mut self, button: u32, state: ButtonState) {
        let Some(selecting) = &mut self.selecting else { return };
        if button != BTN_LEFT {
            if state == ButtonState::Pressed {
                self.cancel_selection();
            }
            return;
        }
        if let Purpose::Share { kinds, .. } = selecting.purpose {
            // Sharing is a plain click: on a window, or on the desktop for its monitor.
            if state == ButtonState::Released
                && let Some(choice) = self.share_choice(kinds, false)
            {
                self.finish_selection(Some(choice));
            }
            return;
        }
        match state {
            ButtonState::Pressed => {
                selecting.anchor = Some(self.pointer_location);
                self.queue_redraw_all();
            }
            ButtonState::Released => {
                let anchor = selecting.anchor.unwrap_or(self.pointer_location);
                self.selecting = None;
                self.queue_redraw_all();
                let moved = (self.pointer_location - anchor).x.abs().max((self.pointer_location - anchor).y.abs());
                if moved > CLICK_SLOP {
                    self.shoot(normalized(anchor, self.pointer_location));
                } else if let Some(frame) = self.window_frame_at_pointer() {
                    self.shoot(frame);
                } else {
                    self.shoot_output_under_pointer();
                }
            }
        }
    }

    pub fn screenshot_screen(&mut self) {
        self.shoot_output_under_pointer();
    }

    pub fn screenshot_window(&mut self) {
        let frame = self.desktop.focused().and_then(|id| self.desktop.get(id)).and_then(|m| m.frame);
        match frame {
            Some((rect, _)) => self.shoot(Rectangle::new((rect.x, rect.y).into(), (rect.width, rect.height).into())),
            None => tracing::info!("screenshot: no focused window"),
        }
    }

    fn shoot_output_under_pointer(&mut self) {
        let geo = self.space.output_under(self.pointer_location).next().and_then(|o| self.space.output_geometry(o));
        if let Some(geo) = geo {
            self.shoot(geo);
        }
    }

    fn window_frame_at_pointer(&self) -> Option<Rectangle<i32, Logical>> {
        let (window, _) = self.window_at(self.pointer_location)?;
        let (rect, _) = self.desktop.windows.iter().find(|m| m.window == window)?.frame?;
        Some(Rectangle::new((rect.x, rect.y).into(), (rect.width, rect.height).into()))
    }

    /// Queue a shot of `area` (global logical pixels) on the output holding most of it.
    fn shoot(&mut self, area: Rectangle<i32, Logical>) {
        let center: Point<i32, Logical> = (area.loc.x + area.size.w / 2, area.loc.y + area.size.h / 2).into();
        let Some(output) = self.space.output_under(center.to_f64()).next().cloned() else { return };
        let Some(geo) = self.space.output_geometry(&output) else { return };
        let Some(size) = self.capture_geometry(&output).map(|g| g.size) else { return };
        let scale = output.current_scale().fractional_scale();
        let local = Rectangle::<i32, Logical>::new(area.loc - geo.loc, area.size);
        let region = Rectangle::<i32, Physical>::new(
            ((f64::from(local.loc.x) * scale).round() as i32, (f64::from(local.loc.y) * scale).round() as i32).into(),
            ((f64::from(local.size.w) * scale).round() as i32, (f64::from(local.size.h) * scale).round() as i32).into(),
        );
        let Some(region) = region.intersection(Rectangle::from_size(size)).filter(|r| r.size.w > 0 && r.size.h > 0) else {
            return;
        };
        self.pending_shots.push(PendingShot { output: output.clone(), region });
        self.queue_redraw_output(&output);
    }

    /// Take the shots waiting on `output` from the elements just composed.
    pub fn fulfill_screenshots(&mut self, renderer: &mut GlesRenderer, output: &Output, elements: &[OutputElement]) {
        if !self.pending_shots.iter().any(|p| &p.output == output) {
            return;
        }
        let (ready, waiting): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.pending_shots).into_iter().partition(|p| &p.output == output);
        self.pending_shots = waiting;
        let Some(geometry) = self.capture_geometry(output) else { return };
        let scale = output.current_scale().fractional_scale();
        let visible: Vec<&OutputElement> =
            elements.iter().filter(|e| !matches!(e, OutputElement::Cursor(_) | OutputElement::Overlay(_))).collect();
        for shot in ready {
            let mut tracker = OutputDamageTracker::new(geometry.mode_size, scale, geometry.transform);
            match render_to_pixels(renderer, &mut tracker, &visible, self.clear_color(), geometry.size, shot.region) {
                Ok(pixels) => save(
                    PathBuf::from(&self.config.screenshot_directory),
                    shot.region.size.w as u32,
                    shot.region.size.h as u32,
                    pixels,
                    self.socket_name.to_string_lossy().into_owned(),
                ),
                Err(error) => tracing::warn!("screenshot failed: {error}"),
            }
        }
    }

    /// The dimming and selection frame drawn over `output` while selecting.
    pub fn overlay_elements(&self, output: &Output) -> Vec<SolidColorRenderElement> {
        let Some(selecting) = &self.selecting else { return Vec::new() };
        let Some(geo) = self.space.output_geometry(output) else { return Vec::new() };
        let Some(size) = self.capture_geometry(output).map(|g| g.size) else { return Vec::new() };
        let scale = output.current_scale().fractional_scale();
        let to_local = |r: Rectangle<i32, Logical>| {
            Rectangle::<i32, Physical>::new(
                (
                    (f64::from(r.loc.x - geo.loc.x) * scale).round() as i32,
                    (f64::from(r.loc.y - geo.loc.y) * scale).round() as i32,
                )
                    .into(),
                ((f64::from(r.size.w) * scale).round() as i32, (f64::from(r.size.h) * scale).round() as i32).into(),
            )
        };
        let full = Rectangle::<i32, Physical>::from_size(size);
        let dim = [0.0, 0.0, 0.0, 0.5];
        let frame_color = [1.0, 1.0, 1.0, 1.0];
        let accent = self.desktop.appearance.active_border.0;
        let mut elements = Vec::new();
        let mut add = |area: Rectangle<i32, Physical>, color: [f32; 4]| {
            if let Some(area) = area.intersection(full).filter(|a| a.size.w > 0 && a.size.h > 0) {
                elements.push(SolidColorRenderElement::new(Id::new(), area, CommitCounter::default(), color, Kind::Unspecified));
            }
        };
        let thickness = (2.0 * scale).round().max(1.0) as i32;
        let selection = match selecting.anchor {
            Some(anchor) => Some((to_local(normalized(anchor, self.pointer_location)), frame_color)),
            None => match selecting.purpose {
                Purpose::Share { kinds, .. } if !kinds.windows() => None,
                _ => self.window_frame_at_pointer().map(|frame| (to_local(frame), accent)),
            },
        };
        match selection {
            Some((rect, color)) => {
                // Dim everything around the selection, frame it.
                let (x0, y0, x1, y1) = (rect.loc.x, rect.loc.y, rect.loc.x + rect.size.w, rect.loc.y + rect.size.h);
                add(Rectangle::new((0, 0).into(), (full.size.w, y0).into()), dim);
                add(Rectangle::new((0, y1).into(), (full.size.w, full.size.h - y1).into()), dim);
                add(Rectangle::new((0, y0).into(), (x0, y1 - y0).into()), dim);
                add(Rectangle::new((x1, y0).into(), (full.size.w - x1, y1 - y0).into()), dim);
                add(Rectangle::new((x0 - thickness, y0 - thickness).into(), (x1 - x0 + 2 * thickness, thickness).into()), color);
                add(Rectangle::new((x0 - thickness, y1).into(), (x1 - x0 + 2 * thickness, thickness).into()), color);
                add(Rectangle::new((x0 - thickness, y0).into(), (thickness, y1 - y0).into()), color);
                add(Rectangle::new((x1, y0).into(), (thickness, y1 - y0).into()), color);
            }
            None => {
                add(full, [0.0, 0.0, 0.0, 0.3]);
                // Sharing a monitor: frame the one under the pointer.
                let here = self.space.output_under(self.pointer_location).next() == Some(output);
                if here && matches!(selecting.purpose, Purpose::Share { kinds, .. } if kinds.monitors()) {
                    let (w, h, t) = (full.size.w, full.size.h, thickness * 2);
                    add(Rectangle::new((0, 0).into(), (w, t).into()), accent);
                    add(Rectangle::new((0, h - t).into(), (w, t).into()), accent);
                    add(Rectangle::new((0, 0).into(), (t, h).into()), accent);
                    add(Rectangle::new((w - t, 0).into(), (t, h).into()), accent);
                }
            }
        }
        elements
    }
}

fn normalized(a: Point<f64, Logical>, b: Point<f64, Logical>) -> Rectangle<i32, Logical> {
    let (x0, x1) = (a.x.min(b.x).floor() as i32, a.x.max(b.x).ceil() as i32);
    let (y0, y1) = (a.y.min(b.y).floor() as i32, a.y.max(b.y).ceil() as i32);
    Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into())
}

/// `YYYY-MM-DD_HH-MM-SS` in local time.
fn timestamp() -> String {
    // SAFETY: `localtime_r` only writes into the `tm` we pass.
    let tm = unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// Encode, write and publish the picture without holding up the compositor.
fn save(directory: PathBuf, width: u32, height: u32, mut pixels: Vec<u8>, wayland_display: String) {
    std::thread::spawn(move || {
        // The renderer hands over B, G, R, A; the output is opaque.
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        let path = directory.join(format!("Screenshot_{}.png", timestamp()));
        // Written under a hidden name and renamed, so that nobody (a file manager, a watcher) sees a half-written picture.
        let partial = directory.join(format!(".{}.part", path.file_name().map_or_else(|| "Screenshot".into(), |name| name.to_string_lossy())));
        let written = (|| -> Result<(), Box<dyn std::error::Error>> {
            std::fs::create_dir_all(&directory)?;
            let mut encoder = png::Encoder::new(BufWriter::new(std::fs::File::create(&partial)?), width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&pixels)?;
            writer.finish()?;
            std::fs::rename(&partial, &path)?;
            Ok(())
        })();
        if let Err(error) = written {
            let _ = std::fs::remove_file(&partial);
            tracing::warn!("cannot save the screenshot to {}: {error}", path.display());
            return;
        }
        tracing::info!("screenshot saved: {} ({width}x{height})", path.display());
        use std::process::{Command, Stdio};
        if let Ok(file) = std::fs::File::open(&path) {
            let copied = Command::new("wl-copy")
                .args(["--type", "image/png"])
                .env("WAYLAND_DISPLAY", &wayland_display)
                .stdin(file)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if copied.is_err() {
                tracing::debug!("wl-copy is not installed: the screenshot is not in the clipboard");
            }
        }
        let _ = Command::new("notify-send")
            .args(["-a", "mywm", "-i"])
            .arg(&path)
            .arg("Screenshot")
            .arg(path.display().to_string())
            .env("WAYLAND_DISPLAY", &wayland_display)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
}
