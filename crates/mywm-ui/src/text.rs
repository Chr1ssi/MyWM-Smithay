//! Text with the system's fonts: measuring (for laying out scenes) and drawing glyphs into a pixmap.
//!
//! Lines are as high as the font's ascent plus descent, rounded up, which is a QML `Text`'s implicit
//! height (19 pixels for 14 pixel JetBrains Mono); one-line texts sit in the middle of their box like
//! `verticalAlignment: Text.AlignVCenter`.
use std::collections::HashMap;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight, Wrap};
use tiny_skia::Pixmap;

use crate::{Color, Measure, TextStyle, theme};

const ELLIPSIS: &str = "…";

pub struct Fonts {
    system: FontSystem,
    cache: SwashCache,
    /// Ascent and descent per font, in pixels at its size.
    metrics: HashMap<StyleKey, (f32, f32)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct StyleKey {
    size_bits: u32,
    bold: bool,
    icons: bool,
}

impl From<TextStyle> for StyleKey {
    fn from(style: TextStyle) -> Self {
        Self { size_bits: style.size.to_bits(), bold: style.bold, icons: style.icons }
    }
}

/// Where a text's lines go: one line or broken at words to a width.
#[derive(Clone, Copy)]
enum Lines {
    One,
    Wrapped(f32),
}

fn attrs(style: TextStyle) -> Attrs<'static> {
    let family = if style.icons { theme::ICON_FONT_FAMILY } else { theme::FONT_FAMILY };
    Attrs::new().family(Family::Name(family)).weight(if style.bold { Weight::BOLD } else { Weight::NORMAL })
}

impl Fonts {
    /// The fonts fontconfig's configuration lists. Reading them takes a while: do it on a worker thread.
    pub fn load() -> Self {
        Self { system: FontSystem::new(), cache: SwashCache::new(), metrics: HashMap::new() }
    }

    /// `text` shaped at `scale` times the style's size, with lines `line_height` (unscaled) apart.
    fn shape(&mut self, text: &str, style: TextStyle, lines: Lines, scale: f32, line_height: f32) -> Buffer {
        let mut buffer = Buffer::new(&mut self.system, Metrics::new(style.size * scale, (line_height * scale).max(1.0)));
        let (width, wrap) = match lines {
            Lines::One => (None, Wrap::None),
            Lines::Wrapped(width) => (Some((width * scale).max(1.0)), Wrap::WordOrGlyph),
        };
        buffer.set_wrap(wrap);
        buffer.set_size(width, None);
        buffer.set_text(text, &attrs(style), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.system, false);
        buffer
    }

    /// Ascent and descent of the style's font.
    fn ascent_descent(&mut self, style: TextStyle) -> (f32, f32) {
        if let Some(metrics) = self.metrics.get(&style.into()) {
            return *metrics;
        }
        let mut buffer = self.shape("X", style, Lines::One, 1.0, style.size);
        let metrics = buffer
            .line_layout(&mut self.system, 0)
            .and_then(|layout| layout.first())
            .map_or((style.size, 0.0), |line| (line.max_ascent, line.max_descent));
        self.metrics.insert(style.into(), metrics);
        metrics
    }

    /// `text`, cut off with "…" so that it is at most `max_width` wide.
    pub fn elided(&mut self, text: &str, style: TextStyle, max_width: f32) -> String {
        if self.width(text, style) <= max_width {
            return text.to_owned();
        }
        let ends: Vec<usize> = text.char_indices().map(|(i, _)| i).skip(1).chain([text.len()]).collect();
        // The longest start that still fits with the ellipsis.
        let (mut low, mut high) = (0, ends.len());
        while low < high {
            let middle = (low + high).div_ceil(2);
            let candidate = format!("{}{ELLIPSIS}", &text[..ends[middle - 1]]);
            if self.width(&candidate, style) <= max_width {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        match low {
            0 => ELLIPSIS.to_owned(),
            n => format!("{}{ELLIPSIS}", text[..ends[n - 1]].trim_end()),
        }
    }

    /// Draw `text` on one line with its left end and the top of its line at `at` (logical pixels).
    pub fn draw_line(&mut self, pixmap: &mut Pixmap, text: &str, style: TextStyle, color: Color, at: (f32, f32), scale: f32) {
        let line_height = self.line_height(style);
        self.draw(pixmap, text, style, Lines::One, color, at, scale, line_height);
    }

    /// Draw `text` broken at words to `width`, from `at`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_paragraph(&mut self, pixmap: &mut Pixmap, text: &str, style: TextStyle, color: Color, at: (f32, f32), width: f32, scale: f32) {
        let line_height = self.line_height(style);
        self.draw(pixmap, text, style, Lines::Wrapped(width), color, at, scale, line_height);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        pixmap: &mut Pixmap,
        text: &str,
        style: TextStyle,
        lines: Lines,
        color: Color,
        (x, top): (f32, f32),
        scale: f32,
        line_height: f32,
    ) {
        let buffer = self.shape(text, style, lines, scale, line_height);
        let origin = (x * scale, top * scale);
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((origin.0, origin.1 + run.line_y), 1.0);
                let Some(image) = self.cache.get_image(&mut self.system, physical.cache_key) else { continue };
                let left = physical.x + image.placement.left;
                let top = physical.y - image.placement.top;
                let (width, height) = (image.placement.width as i32, image.placement.height as i32);
                match image.content {
                    SwashContent::Mask => blend(pixmap, left, top, width, height, |i| {
                        let coverage = u32::from(image.data[i]);
                        [color.r, color.g, color.b, (u32::from(color.a) * coverage / 255) as u8]
                    }),
                    // Emoji keep their own colors; only the text's opacity applies.
                    SwashContent::Color => blend(pixmap, left, top, width, height, |i| {
                        let pixel = &image.data[i * 4..i * 4 + 4];
                        [pixel[0], pixel[1], pixel[2], (u32::from(pixel[3]) * u32::from(color.a) / 255) as u8]
                    }),
                    SwashContent::SubpixelMask => {}
                }
            }
        }
    }
}

/// Blend a `width` × `height` image at (`left`, `top`) over the pixmap; `source(i)` is pixel `i` of the
/// image (row by row) with straight alpha. The pixmap holds premultiplied RGBA.
fn blend(pixmap: &mut Pixmap, left: i32, top: i32, width: i32, height: i32, source: impl Fn(usize) -> [u8; 4]) {
    let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
    let data = pixmap.data_mut();
    for row in 0..height {
        let y = top + row;
        if y < 0 || y >= ph {
            continue;
        }
        for column in 0..width {
            let x = left + column;
            if x < 0 || x >= pw {
                continue;
            }
            let [r, g, b, a] = source((row * width + column) as usize);
            if a == 0 {
                continue;
            }
            let at = ((y * pw + x) * 4) as usize;
            let (a, keep) = (u32::from(a), 255 - u32::from(a));
            for (channel, value) in [r, g, b].into_iter().enumerate() {
                let dst = u32::from(data[at + channel]);
                data[at + channel] = ((u32::from(value) * a + dst * keep + 127) / 255) as u8;
            }
            let dst = u32::from(data[at + 3]);
            data[at + 3] = ((a * 255 + dst * keep + 127) / 255) as u8;
        }
    }
}

impl Measure for Fonts {
    fn width(&mut self, text: &str, style: TextStyle) -> f32 {
        let line_height = self.line_height(style);
        let buffer = self.shape(text, style, Lines::One, 1.0, line_height);
        buffer.layout_runs().map(|run| run.line_w).fold(0.0, f32::max)
    }

    fn line_height(&mut self, style: TextStyle) -> f32 {
        let (ascent, descent) = self.ascent_descent(style);
        (ascent + descent).ceil()
    }

    fn paragraph_height(&mut self, text: &str, style: TextStyle, width: f32) -> f32 {
        let line_height = self.line_height(style);
        let buffer = self.shape(text, style, Lines::Wrapped(width), 1.0, line_height);
        (buffer.layout_runs().count().max(1) as f32) * line_height
    }
}
