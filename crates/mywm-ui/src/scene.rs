//! What a piece of the shell shows, as a list of shapes and texts in logical pixels, and where it reacts to
//! the pointer. Scenes are rebuilt from their model whenever something changes; painting them is the
//! expensive part and happens only then.
use std::sync::Arc;

use crate::markup::Span;

/// A color with straight (not premultiplied) alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// `#rrggbb`, or `#aarrggbb` as in Qt.
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.trim().strip_prefix('#')?;
        let value = u32::from_str_radix(hex, 16).ok()?;
        let byte = |shift: u32| (value >> shift & 0xff) as u8;
        match hex.len() {
            6 => Some(Self::rgba(byte(16), byte(8), byte(0), 255)),
            8 => Some(Self::rgba(byte(16), byte(8), byte(0), byte(24))),
            _ => None,
        }
    }

    /// The same color with `alpha` (0.0 to 1.0) as its opacity, like `Qt.rgba(c.r, c.g, c.b, alpha)`.
    pub fn with_alpha(self, alpha: f32) -> Self {
        Self { a: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8, ..self }
    }

    /// The opacity multiplied by `factor`, like QML's `opacity` on an item of this color.
    pub fn faded(self, factor: f32) -> Self {
        Self { a: (f32::from(self.a) * factor.clamp(0.0, 1.0)).round() as u8, ..self }
    }

    /// `#rrggbb`.
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// The rectangle shrunk by `by` on every side.
    pub fn inset(&self, by: f32) -> Self {
        Self::new(self.x + by, self.y + by, (self.w - 2.0 * by).max(0.0), (self.h - 2.0 * by).max(0.0))
    }

    pub fn translated(&self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.w, self.h)
    }
}

/// Font of a text: pixel size, weight and family (`theme::FONT_FAMILY`; for icons that must not overhang their
/// cell `theme::ICON_FONT_FAMILY`; for Qt Quick's own controls, which do not take the shell's font, the system's
/// sans-serif font as fontconfig names it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub bold: bool,
    pub icons: bool,
    pub controls: bool,
}

impl TextStyle {
    pub const fn new(size: f32) -> Self {
        Self { size, bold: false, icons: false, controls: false }
    }

    pub const fn bold(self, bold: bool) -> Self {
        Self { bold, ..self }
    }

    pub const fn icons(self, icons: bool) -> Self {
        Self { icons, ..self }
    }

    pub const fn controls(self, controls: bool) -> Self {
        Self { controls, ..self }
    }
}

/// A decoded picture (premultiplied pixels); pictures are equal when they are the same one.
#[derive(Clone, Debug)]
pub struct Picture(pub Arc<tiny_skia::Pixmap>);

impl PartialEq for Picture {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Picture {
    /// A picture of straight-alpha RGBA pixels, row by row.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Self> {
        if rgba.len() != width as usize * height as usize * 4 {
            return None;
        }
        let pixels = rgba
            .chunks(4)
            .flat_map(|p| {
                let alpha = u16::from(p[3]);
                let premultiply = |c: u8| ((u16::from(c) * alpha + 127) / 255) as u8;
                [premultiply(p[0]), premultiply(p[1]), premultiply(p[2]), p[3]]
            })
            .collect();
        let size = tiny_skia::IntSize::from_wh(width, height)?;
        Some(Self(Arc::new(tiny_skia::Pixmap::from_vec(pixels, size)?)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Left,
    Center,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// A QML `Rectangle`: `radius` rounds the corners, the border lies inside the rectangle.
    Rect { rect: Rect, radius: f32, fill: Color, border: Option<(f32, Color)> },
    /// One line centered vertically in `rect`, cut off with "…" where it is wider.
    Label { rect: Rect, text: String, style: TextStyle, color: Color, align: Align },
    /// Lines broken at words to the width of `rect`, from its top (`wrapMode: Text.Wrap`).
    Paragraph { rect: Rect, text: String, style: TextStyle, color: Color },
    /// A stroked circular arc with round caps: angles in radians, 0 to the right, growing clockwise.
    Arc { center: (f32, f32), radius: f32, width: f32, color: Color, start: f32, sweep: f32 },
    /// Styled runs broken at words to the width of `rect`, at most `max_lines` lines, the last one cut off with
    /// "…" (`textFormat: Text.StyledText`, `maximumLineCount`, `elide: Text.ElideRight`).
    RichText { rect: Rect, spans: Vec<Span>, style: TextStyle, color: Color, max_lines: usize },
    /// A picture filling `rect`, cut to its proportions (`fillMode: Image.PreserveAspectCrop`).
    Image { rect: Rect, picture: Picture },
    /// From here on only what lies in the rectangle shows (`clip: true`); `None` ends that.
    Clip(Option<Rect>),
}

impl Node {
    /// The node moved by (`dx`, `dy`).
    pub fn translated(self, dx: f32, dy: f32) -> Self {
        match self {
            Node::Rect { rect, radius, fill, border } => Node::Rect { rect: rect.translated(dx, dy), radius, fill, border },
            Node::Label { rect, text, style, color, align } => Node::Label { rect: rect.translated(dx, dy), text, style, color, align },
            Node::Paragraph { rect, text, style, color } => Node::Paragraph { rect: rect.translated(dx, dy), text, style, color },
            Node::Arc { center, radius, width, color, start, sweep } => {
                Node::Arc { center: (center.0 + dx, center.1 + dy), radius, width, color, start, sweep }
            }
            Node::RichText { rect, spans, style, color, max_lines } => {
                Node::RichText { rect: rect.translated(dx, dy), spans, style, color, max_lines }
            }
            Node::Image { rect, picture } => Node::Image { rect: rect.translated(dx, dy), picture },
            Node::Clip(rect) => Node::Clip(rect.map(|r| r.translated(dx, dy))),
        }
    }

    /// The node with its colors' opacity multiplied by `factor` (QML's `opacity`).
    pub fn faded(self, factor: f32) -> Self {
        match self {
            Node::Rect { rect, radius, fill, border } => {
                Node::Rect { rect, radius, fill: fill.faded(factor), border: border.map(|(w, c)| (w, c.faded(factor))) }
            }
            Node::Label { rect, text, style, color, align } => Node::Label { rect, text, style, color: color.faded(factor), align },
            Node::Paragraph { rect, text, style, color } => Node::Paragraph { rect, text, style, color: color.faded(factor) },
            Node::Arc { center, radius, width, color, start, sweep } => {
                Node::Arc { center, radius, width, color: color.faded(factor), start, sweep }
            }
            Node::RichText { rect, spans, style, color, max_lines } => {
                Node::RichText { rect, spans, style, color: color.faded(factor), max_lines }
            }
            node @ (Node::Image { .. } | Node::Clip(_)) => node,
        }
    }
}

/// Shapes in painting order (later ones on top) and the areas that react to the pointer.
#[derive(Clone, Debug)]
pub struct Scene<T> {
    pub width: f32,
    pub height: f32,
    pub nodes: Vec<Node>,
    pub targets: Vec<(Rect, T)>,
}

impl<T: Clone> Scene<T> {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height, nodes: Vec::new(), targets: Vec::new() }
    }

    pub fn push(&mut self, node: Node) {
        self.nodes.push(node);
    }

    pub fn target(&mut self, rect: Rect, target: T) {
        self.targets.push((rect, target));
    }

    /// What the pointer at (`x`, `y`) is over; of overlapping areas the one added last.
    pub fn target_at(&self, x: f32, y: f32) -> Option<T> {
        self.targets.iter().rev().find(|(rect, _)| rect.contains(x, y)).map(|(_, t)| t.clone())
    }

    /// Where `target` is, if the scene has it.
    pub fn target_rect(&self, target: &T) -> Option<Rect>
    where
        T: PartialEq,
    {
        self.targets.iter().find(|(_, t)| t == target).map(|(rect, _)| *rect)
    }
}

/// Text measurement for laying out scenes; `text::Fonts` measures with the real fonts.
pub trait Measure {
    /// Width of `text` on one line.
    fn width(&mut self, text: &str, style: TextStyle) -> f32;
    /// Height of one line, like a QML `Text`'s implicit height.
    fn line_height(&mut self, style: TextStyle) -> f32;
    /// Height of `text` broken into lines of at most `width`.
    fn paragraph_height(&mut self, text: &str, style: TextStyle, width: f32) -> f32;
    /// Height of styled runs broken into at most `max_lines` lines of at most `width`.
    fn rich_height(&mut self, spans: &[Span], style: TextStyle, width: f32, max_lines: usize) -> f32;
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Every character as wide as half the font size, lines 1.32 times the size (JetBrains Mono's
    /// proportions), words broken like a monospace paragraph.
    pub struct Mono;

    impl Measure for Mono {
        fn width(&mut self, text: &str, style: TextStyle) -> f32 {
            text.chars().count() as f32 * style.size * 0.5
        }

        fn line_height(&mut self, style: TextStyle) -> f32 {
            (style.size * 1.32).ceil()
        }

        fn paragraph_height(&mut self, text: &str, style: TextStyle, width: f32) -> f32 {
            let per_line = ((width / (style.size * 0.5)).floor() as usize).max(1);
            let mut lines = 1;
            let mut used = 0;
            for word in text.split(' ') {
                let len = word.chars().count();
                let needed = if used == 0 { len } else { used + 1 + len };
                if needed > per_line && used > 0 {
                    lines += 1;
                    used = len;
                } else {
                    used = needed;
                }
            }
            lines as f32 * self.line_height(style)
        }

        fn rich_height(&mut self, spans: &[Span], style: TextStyle, width: f32, max_lines: usize) -> f32 {
            let text = crate::markup::plain(spans);
            let line = self.line_height(style);
            let lines: f32 = text.split('\n').map(|part| (self.paragraph_height(part, style, width) / line).round()).sum();
            lines.min(max_lines as f32).max(1.0) * line
        }
    }

    #[test]
    fn colors_parse_like_qt() {
        assert_eq!(Color::parse("#89b4fa"), Some(Color::rgba(0x89, 0xb4, 0xfa, 255)));
        assert_eq!(Color::parse("#66000000"), Some(Color::rgba(0, 0, 0, 0x66)));
        assert_eq!(Color::parse("89b4fa"), None);
        assert_eq!(Color::parse("#12345"), None);
        assert_eq!(Color::rgba(1, 2, 3, 255).with_alpha(0.7).a, 179);
        assert_eq!(Color::rgba(0x12, 0xab, 0xff, 9).hex(), "#12abff");
    }

    #[test]
    fn the_last_target_added_is_on_top() {
        let mut scene = Scene::new(100.0, 100.0);
        scene.target(Rect::new(0.0, 0.0, 100.0, 100.0), 1);
        scene.target(Rect::new(10.0, 10.0, 10.0, 10.0), 2);
        assert_eq!(scene.target_at(15.0, 15.0), Some(2));
        assert_eq!(scene.target_at(50.0, 50.0), Some(1));
        assert_eq!(scene.target_at(100.0, 50.0), None);
        assert_eq!(scene.target_rect(&2), Some(Rect::new(10.0, 10.0, 10.0, 10.0)));
    }
}
