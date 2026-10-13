//! Color picker: a magnifier next to the pointer shows the pixels around it; a click copies the middle one as
//! `#rrggbb` (wl-copy) and says so in a notification with a swatch. Escape or another button cancels.
//!
//! The pixels come from the composed frame of the output under the pointer, without cursor and overlays (the
//! magnifier is one), as for screenshots. Each frame of that output takes a new sample while picking; a sample
//! that differs from the last redraws the magnifier, so the frames settle once nothing changes.
use std::process::{Command, Stdio};

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
    utils::{Physical, Point, Rectangle},
};

use crate::{State, render::OutputElement, screencopy::render_to_pixels, screenshot::ModalKey};

const BTN_LEFT: u32 = 0x110;
/// Pixels shown each way around the middle one.
const RADIUS: i32 = 5;
const SIDE: i32 = 2 * RADIUS + 1;
/// Size of one magnified pixel and the gap to the pointer, in logical pixels.
const CELL: f64 = 12.0;
const OFFSET: f64 = 24.0;

pub struct Picking {
    sample: Option<Sample>,
}

/// The pixels around the pointer from the last frame of its output.
struct Sample {
    output: Output,
    /// RGB, row by row, `SIDE` × `SIDE`; `None` beyond the edge of the output.
    pixels: Vec<Option<[u8; 3]>>,
}

impl Sample {
    fn middle(&self) -> Option<[u8; 3]> {
        self.pixels.get((RADIUS * SIDE + RADIUS) as usize).copied().flatten()
    }
}

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

impl State {
    pub fn start_color_picker(&mut self) {
        if self.session_lock.is_active() || self.selecting.is_some() || self.overview_open() {
            return;
        }
        self.picking = Some(Picking { sample: None });
        self.ipc_dirty = true;
        self.queue_redraw_all();
    }

    pub fn cancel_color_picker(&mut self) {
        if self.picking.take().is_some() {
            self.ipc_dirty = true;
            self.queue_redraw_all();
        }
    }

    /// The color under the pointer as the bar shows it: `Some` while picking, with the color once sampled.
    pub fn picked_color(&self) -> Option<Option<String>> {
        let picking = self.picking.as_ref()?;
        Some(picking.sample.as_ref().and_then(Sample::middle).map(hex))
    }

    pub fn picker_key(&mut self, key: ModalKey) {
        match key {
            ModalKey::Cancel => self.cancel_color_picker(),
            ModalKey::Confirm => self.pick_color(),
            _ => {}
        }
    }

    pub fn picker_button(&mut self, button: u32, state: ButtonState) {
        if state != ButtonState::Pressed {
            return;
        }
        if button == BTN_LEFT {
            self.pick_color();
        } else {
            self.cancel_color_picker();
        }
    }

    /// Copy the color under the pointer and end picking.
    fn pick_color(&mut self) {
        let Some(color) = self.picking.as_ref().and_then(|p| p.sample.as_ref()).and_then(Sample::middle) else {
            // No frame was sampled yet (it comes with the next one).
            return;
        };
        self.cancel_color_picker();
        let text = hex(color);
        tracing::info!("color picked: {text}");
        let wayland_display = self.socket_name.to_string_lossy().into_owned();
        let swatch = std::env::var_os("XDG_RUNTIME_DIR").map(|dir| std::path::PathBuf::from(dir).join("mywm-color.png"));
        // Off the compositor's thread: wl-copy and notify-send may take a moment.
        std::thread::spawn(move || {
            let copied = Command::new("wl-copy")
                .arg(&text)
                .env("WAYLAND_DISPLAY", &wayland_display)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if copied.is_err() {
                tracing::warn!("wl-copy is not installed: the color is not in the clipboard");
            }
            let mut notify = Command::new("notify-send");
            notify.args(["-a", "mywm"]);
            if let Some(path) = swatch.filter(|path| write_swatch(path, color).is_ok()) {
                notify.arg("-i").arg(path);
            }
            let _ = notify
                .arg("Farbe kopiert")
                .arg(&text)
                .env("WAYLAND_DISPLAY", &wayland_display)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        });
    }

    /// Sample the pixels around the pointer from the elements just composed for `output`.
    pub fn sample_color(&mut self, renderer: &mut GlesRenderer, output: &Output, elements: &[OutputElement]) {
        if self.picking.is_none() || self.space.output_under(self.pointer_location).next() != Some(output) {
            return;
        }
        let (Some(geo), Some(geometry)) = (self.space.output_geometry(output), self.capture_geometry(output)) else {
            return;
        };
        let scale = output.current_scale().fractional_scale();
        let local = (self.pointer_location - geo.loc.to_f64()).to_physical(scale);
        let middle = Point::<i32, Physical>::from((local.x.floor() as i32, local.y.floor() as i32));
        let wanted = Rectangle::<i32, Physical>::new(middle - Point::from((RADIUS, RADIUS)), (SIDE, SIDE).into());
        let Some(region) = wanted.intersection(Rectangle::from_size(geometry.size)) else { return };
        let blanks = self.capture_blanks(output);
        let visible: Vec<&OutputElement> = self
            .capture_view(output, elements, &blanks)
            .into_iter()
            .filter(|e| !matches!(e, OutputElement::Cursor(_) | OutputElement::Overlay(_)))
            .collect();
        let mut tracker = OutputDamageTracker::new(geometry.mode_size, scale, geometry.transform);
        let bgra = match render_to_pixels(renderer, &mut tracker, &visible, self.clear_color(), geometry.size, region) {
            Ok(bgra) => bgra,
            Err(error) => return tracing::warn!("color picker: {error}"),
        };
        // Place the region's pixels into the full square; the part beyond the output stays empty.
        let mut pixels = vec![None; (SIDE * SIDE) as usize];
        for row in 0..region.size.h {
            for column in 0..region.size.w {
                let at = ((row * region.size.w + column) * 4) as usize;
                let Some(&[b, g, r, _]) = bgra.get(at..at + 4).and_then(|p| <&[u8; 4]>::try_from(p).ok()) else { continue };
                let (x, y) = (region.loc.x - wanted.loc.x + column, region.loc.y - wanted.loc.y + row);
                pixels[(y * SIDE + x) as usize] = Some([r, g, b]);
            }
        }
        let Some(picking) = &mut self.picking else { return };
        let changed = picking.sample.as_ref().is_none_or(|s| &s.output != output || s.pixels != pixels);
        if changed {
            picking.sample = Some(Sample { output: output.clone(), pixels });
            self.ipc_dirty = true;
            self.queue_redraw_output(output);
        }
    }

    /// The magnifier on `output`, front to back: the frame of the middle pixel, the pixels, the border.
    pub fn magnifier_elements(&self, output: &Output) -> Vec<SolidColorRenderElement> {
        let Some(sample) = self.picking.as_ref().and_then(|p| p.sample.as_ref()).filter(|s| &s.output == output) else {
            return Vec::new();
        };
        let (Some(geo), Some(size)) = (self.space.output_geometry(output), self.capture_geometry(output).map(|g| g.size)) else {
            return Vec::new();
        };
        let scale = output.current_scale().fractional_scale();
        let box_side = CELL * f64::from(SIDE);
        // Below right of the pointer, or on the other side where the output ends.
        let local = self.pointer_location - geo.loc.to_f64();
        let (w, h) = (f64::from(geo.size.w), f64::from(geo.size.h));
        let x = if local.x + OFFSET + box_side <= w { local.x + OFFSET } else { local.x - OFFSET - box_side };
        let y = if local.y + OFFSET + box_side <= h { local.y + OFFSET } else { local.y - OFFSET - box_side };
        let at = |lx: f64, ly: f64, lw: f64, lh: f64| {
            Rectangle::<i32, Physical>::new(
                ((lx * scale).round() as i32, (ly * scale).round() as i32).into(),
                ((lw * scale).round() as i32, (lh * scale).round() as i32).into(),
            )
        };
        let full = Rectangle::<i32, Physical>::from_size(size);
        let mut elements = Vec::new();
        let mut add = |area: Rectangle<i32, Physical>, color: [f32; 4]| {
            if let Some(area) = area.intersection(full).filter(|a| a.size.w > 0 && a.size.h > 0) {
                elements.push(SolidColorRenderElement::new(Id::new(), area, CommitCounter::default(), color, Kind::Unspecified));
            }
        };
        // The middle pixel framed in black or white, whichever stands out from it.
        let middle = sample.middle().unwrap_or([0, 0, 0]);
        let light = 0.299 * f64::from(middle[0]) + 0.587 * f64::from(middle[1]) + 0.114 * f64::from(middle[2]) > 140.0;
        let mark = if light { [0.0, 0.0, 0.0, 1.0] } else { [1.0, 1.0, 1.0, 1.0] };
        let t = 2.0;
        let (mx, my) = (x + CELL * f64::from(RADIUS), y + CELL * f64::from(RADIUS));
        add(at(mx, my, CELL, t), mark);
        add(at(mx, my + CELL - t, CELL, t), mark);
        add(at(mx, my, t, CELL), mark);
        add(at(mx + CELL - t, my, t, CELL), mark);
        for (index, pixel) in sample.pixels.iter().enumerate() {
            let (column, row) = (index as i32 % SIDE, index as i32 / SIDE);
            let color = pixel.map_or([0.0, 0.0, 0.0, 1.0], |[r, g, b]| {
                [f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, 1.0]
            });
            add(at(x + CELL * f64::from(column), y + CELL * f64::from(row), CELL, CELL), color);
        }
        let border = 2.0;
        add(at(x - border, y - border, box_side + 2.0 * border, box_side + 2.0 * border), [1.0, 1.0, 1.0, 1.0]);
        elements
    }
}

/// A 64×64 PNG of `color` for the notification.
fn write_swatch(path: &std::path::Path, [r, g, b]: [u8; 3]) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path)?), 64, 64);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&[r, g, b].repeat(64 * 64))?;
    writer.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_lowercase_hex() {
        assert_eq!(hex([255, 0, 171]), "#ff00ab");
    }
}
