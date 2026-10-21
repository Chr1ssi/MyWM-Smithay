//! Text with the system's fonts: measuring (for laying out scenes) and drawing glyphs into a pixmap.
//!
//! Lines are as high as the font's ascent plus descent, rounded up, which is a QML `Text`'s implicit
//! height (19 pixels for 14 pixel JetBrains Mono); one-line texts sit in the middle of their box like
//! `verticalAlignment: Text.AlignVCenter`.
use std::collections::HashMap;

use cosmic_text::{
    Attrs, Buffer, Ellipsize, EllipsizeHeightLimit, Family, FontSystem, Metrics, PhysicalGlyph, Renderer, Shaping, Style,
    SwashCache, SwashContent, UnderlineStyle, Weight, Wrap,
};
use tiny_skia::Pixmap;

use crate::{Align, Color, Measure, TextStyle, markup::Span, theme};

const ELLIPSIS: &str = "…";

pub struct Fonts {
    system: FontSystem,
    cache: SwashCache,
    /// The family fontconfig gives Qt's default font, for Qt's controls; `None`: the generic sans-serif family.
    sans: Option<String>,
    /// Ascent and descent per font, in pixels at its size.
    metrics: HashMap<StyleKey, (f32, f32)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct StyleKey {
    size_bits: u32,
    bold: bool,
    light: bool,
    icons: bool,
    controls: bool,
}

impl From<TextStyle> for StyleKey {
    fn from(style: TextStyle) -> Self {
        Self { size_bits: style.size.to_bits(), bold: style.bold, light: style.light, icons: style.icons, controls: style.controls }
    }
}

/// How a text is laid out.
#[derive(Clone, Copy)]
enum Lines {
    One,
    /// Broken at words to a width.
    Wrapped(f32),
    /// Broken at words to a width, each line centered in it.
    Centered(f32),
    /// Broken at words to a width, at most this many lines, the last one cut off with "…".
    Limited(f32, usize),
}

/// Physical pixels outside of which nothing is drawn: left, top, right, bottom.
pub type Clip = (i32, i32, i32, i32);

fn attrs(style: TextStyle, sans: Option<&str>) -> Attrs<'_> {
    let family = match (style.controls, sans) {
        (true, Some(sans)) => Family::Name(sans),
        (true, None) => Family::SansSerif,
        (false, _) if style.icons => Family::Name(theme::ICON_FONT_FAMILY),
        (false, _) => Family::Name(theme::FONT_FAMILY),
    };
    let weight = match (style.bold, style.light) {
        (true, _) => Weight::BOLD,
        (false, true) => Weight::LIGHT,
        (false, false) => Weight::NORMAL,
    };
    Attrs::new().family(family).weight(weight)
}

fn span_attrs<'a>(style: TextStyle, span: &Span, sans: Option<&'a str>) -> Attrs<'a> {
    let attrs = attrs(TextStyle { bold: style.bold || span.bold, ..style }, sans);
    let attrs = if span.italic { attrs.style(Style::Italic) } else { attrs };
    if span.underline { attrs.underline(UnderlineStyle::Single) } else { attrs }
}

/// The family fontconfig matches for "Sans Serif", which Qt asks for its default font. fontdb's own reading of
/// fontconfig's configuration keeps the family the last alias prefers, not the one fontconfig chooses (FreeSans
/// instead of DejaVu Sans on NixOS).
fn sans_family() -> Option<String> {
    let output = std::process::Command::new("fc-match").args(["--format", "%{family[0]}", "Sans Serif"]).output();
    match output {
        Ok(output) if output.status.success() => Some(String::from_utf8_lossy(&output.stdout).trim().to_owned()).filter(|f| !f.is_empty()),
        Ok(output) => {
            tracing::warn!("Qt's controls get the generic sans-serif font: fc-match failed ({})", output.status);
            None
        }
        Err(error) => {
            tracing::warn!("Qt's controls get the generic sans-serif font: cannot run fc-match: {error}");
            None
        }
    }
}

/// Glyphs and decorations into a pixmap, in one color, within a clip.
struct Painter<'a> {
    pixmap: &'a mut Pixmap,
    system: &'a mut FontSystem,
    cache: &'a mut SwashCache,
    clip: Clip,
}

impl Renderer for Painter<'_> {
    fn rectangle(&mut self, x: i32, y: i32, w: u32, h: u32, color: cosmic_text::Color) {
        let [r, g, b, a] = [color.r(), color.g(), color.b(), color.a()];
        blend(self.pixmap, self.clip, (x, y, w as i32, h as i32), |_| [r, g, b, a]);
    }

    fn glyph(&mut self, glyph: PhysicalGlyph, color: cosmic_text::Color) {
        let Some(image) = self.cache.get_image(self.system, glyph.cache_key) else { return };
        let area = (glyph.x + image.placement.left, glyph.y - image.placement.top, image.placement.width as i32, image.placement.height as i32);
        let (r, g, b, a) = (color.r(), color.g(), color.b(), u32::from(color.a()));
        match image.content {
            SwashContent::Mask => blend(self.pixmap, self.clip, area, |i| [r, g, b, (a * u32::from(image.data[i]) / 255) as u8]),
            // Emoji keep their own colors; only the text's opacity applies.
            SwashContent::Color => blend(self.pixmap, self.clip, area, |i| {
                let pixel = &image.data[i * 4..i * 4 + 4];
                [pixel[0], pixel[1], pixel[2], (u32::from(pixel[3]) * a / 255) as u8]
            }),
            SwashContent::SubpixelMask => {}
        }
    }
}

/// A renderer whose rectangles move by an offset (decorations come relative to the text's origin).
struct Moved<'a, 'b> {
    inner: &'a mut Painter<'b>,
    dx: i32,
    dy: i32,
}

impl Renderer for Moved<'_, '_> {
    fn rectangle(&mut self, x: i32, y: i32, w: u32, h: u32, color: cosmic_text::Color) {
        self.inner.rectangle(x + self.dx, y + self.dy, w, h, color);
    }

    fn glyph(&mut self, glyph: PhysicalGlyph, color: cosmic_text::Color) {
        self.inner.glyph(glyph, color);
    }
}

impl Fonts {
    /// The fonts fontconfig's configuration lists. Reading them takes a while: do it on a worker thread.
    pub fn load() -> Self {
        Self { system: FontSystem::new(), cache: SwashCache::new(), sans: sans_family(), metrics: HashMap::new() }
    }

    /// `spans` laid out at `scale` times the style's size, with lines `line_height` (unscaled) apart.
    fn shape(&mut self, spans: &[Span], style: TextStyle, lines: Lines, scale: f32, line_height: f32) -> Buffer {
        let mut buffer = Buffer::new(&mut self.system, Metrics::new(style.size * scale, (line_height * scale).max(1.0)));
        let (width, wrap) = match lines {
            Lines::One => (None, Wrap::None),
            Lines::Wrapped(width) | Lines::Centered(width) | Lines::Limited(width, _) => {
                (Some((width * scale).max(1.0)), Wrap::WordOrGlyph)
            }
        };
        buffer.set_wrap(wrap);
        if let Lines::Limited(_, max_lines) = lines {
            buffer.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(max_lines.max(1))));
        }
        buffer.set_size(width, None);
        let sans = self.sans.as_deref();
        let base = attrs(style, sans);
        let align = matches!(lines, Lines::Centered(_)).then_some(cosmic_text::Align::Center);
        buffer.set_rich_text(spans.iter().map(|span| (span.text.as_str(), span_attrs(style, span, sans))), &base, Shaping::Advanced, align);
        buffer.shape_until_scroll(&mut self.system, false);
        buffer
    }

    fn plain(text: &str) -> [Span; 1] {
        [Span { text: text.to_owned(), ..Span::default() }]
    }

    /// Ascent and descent of the style's font.
    fn ascent_descent(&mut self, style: TextStyle) -> (f32, f32) {
        if let Some(metrics) = self.metrics.get(&style.into()) {
            return *metrics;
        }
        let mut buffer = self.shape(&Self::plain("X"), style, Lines::One, 1.0, style.size);
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
    #[allow(clippy::too_many_arguments)]
    pub fn draw_line(&mut self, pixmap: &mut Pixmap, text: &str, style: TextStyle, color: Color, at: (f32, f32), scale: f32, clip: Clip) {
        self.draw(pixmap, &Self::plain(text), style, Lines::One, color, at, scale, clip);
    }

    /// Draw `text` broken at words to `width`, from `at`, each line aligned in the width.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_paragraph(&mut self, pixmap: &mut Pixmap, text: &str, style: TextStyle, color: Color, at: (f32, f32), (width, align): (f32, Align), scale: f32, clip: Clip) {
        let lines = match align {
            Align::Left => Lines::Wrapped(width),
            Align::Center => Lines::Centered(width),
        };
        self.draw(pixmap, &Self::plain(text), style, lines, color, at, scale, clip);
    }

    /// Draw styled runs broken at words to `width`, at most `max_lines` lines, from `at`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_rich(
        &mut self,
        pixmap: &mut Pixmap,
        spans: &[Span],
        style: TextStyle,
        color: Color,
        at: (f32, f32),
        (width, max_lines): (f32, usize),
        scale: f32,
        clip: Clip,
    ) {
        self.draw(pixmap, spans, style, Lines::Limited(width, max_lines), color, at, scale, clip);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(&mut self, pixmap: &mut Pixmap, spans: &[Span], style: TextStyle, lines: Lines, color: Color, at: (f32, f32), scale: f32, clip: Clip) {
        let line_height = self.line_height(style);
        let buffer = self.shape(spans, style, lines, scale, line_height);
        let origin = (at.0 * scale, at.1 * scale);
        let color = cosmic_text::Color::rgba(color.r, color.g, color.b, color.a);
        let mut painter = Painter { pixmap, system: &mut self.system, cache: &mut self.cache, clip };
        let mut previous_line = None;
        for run in buffer.layout_runs() {
            // Workaround: cosmic-text 0.19 keeps the space it broke at in front of the line it ellipsizes (the
            // other wrapped lines start at the next word). A line continuing a paragraph starts at its first
            // word here, as Qt's do. It can go once cosmic-text lays out ellipsized lines like the others.
            let continued = previous_line == Some(run.line_i);
            previous_line = Some(run.line_i);
            let blank = |g: &&cosmic_text::LayoutGlyph| run.text.get(g.start..g.end).is_some_and(|t| t.trim().is_empty());
            let skip = if continued { run.glyphs.iter().take_while(|g| blank(g)).count() } else { 0 };
            let shift = run.glyphs.get(skip).filter(|_| skip > 0).map_or(0.0, |g| g.x);
            for glyph in &run.glyphs[skip..] {
                painter.glyph(glyph.physical((origin.0 - shift, origin.1 + run.line_y), 1.0), color);
            }
            // Underlines come in the text's own coordinates.
            let mut moved = Moved { inner: &mut painter, dx: (origin.0 - shift) as i32, dy: origin.1 as i32 };
            cosmic_text::render_decoration(&mut moved, &run, color);
        }
    }
}

/// Blend a `width` × `height` image at (`left`, `top`) over the pixmap within `clip`; `source(i)` is pixel `i`
/// of the image (row by row) with straight alpha. The pixmap holds premultiplied RGBA.
fn blend(pixmap: &mut Pixmap, clip: Clip, (left, top, width, height): (i32, i32, i32, i32), source: impl Fn(usize) -> [u8; 4]) {
    let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
    let (x0, y0, x1, y1) = (clip.0.max(0), clip.1.max(0), clip.2.min(pw), clip.3.min(ph));
    let data = pixmap.data_mut();
    for row in 0..height {
        let y = top + row;
        if y < y0 || y >= y1 {
            continue;
        }
        for column in 0..width {
            let x = left + column;
            if x < x0 || x >= x1 {
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
        let buffer = self.shape(&Self::plain(text), style, Lines::One, 1.0, line_height);
        buffer.layout_runs().map(|run| run.line_w).fold(0.0, f32::max)
    }

    fn line_height(&mut self, style: TextStyle) -> f32 {
        let (ascent, descent) = self.ascent_descent(style);
        (ascent + descent).ceil()
    }

    fn paragraph_height(&mut self, text: &str, style: TextStyle, width: f32) -> f32 {
        let line_height = self.line_height(style);
        let buffer = self.shape(&Self::plain(text), style, Lines::Wrapped(width), 1.0, line_height);
        (buffer.layout_runs().count().max(1) as f32) * line_height
    }

    fn rich_height(&mut self, spans: &[Span], style: TextStyle, width: f32, max_lines: usize) -> f32 {
        let line_height = self.line_height(style);
        let buffer = self.shape(spans, style, Lines::Limited(width, max_lines), 1.0, line_height);
        (buffer.layout_runs().count().clamp(1, max_lines.max(1)) as f32) * line_height
    }
}
