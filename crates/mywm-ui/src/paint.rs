//! Painting a `Scene` into pixels: premultiplied RGBA (byte order R, G, B, A), `scale` physical pixels per
//! logical one.
use tiny_skia::{FillRule, LineCap, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

use crate::{Align, Color, Measure, Node, Rect, Scene, text::Fonts};

/// The scene's pixels; `None` if it is empty or absurdly large.
pub fn paint<T>(scene: &Scene<T>, fonts: &mut Fonts, scale: f32) -> Option<Pixmap> {
    let (width, height) = pixel_size(scene.width, scene.height, scale);
    let mut pixmap = Pixmap::new(width, height)?;
    let transform = Transform::from_scale(scale, scale);
    for node in &scene.nodes {
        match node {
            Node::Rect { rect, radius, fill, border } => {
                let radius = radius.min(rect.w / 2.0).min(rect.h / 2.0).max(0.0);
                match border {
                    Some((width, color)) if *width > 0.0 && rect.w > 2.0 * width && rect.h > 2.0 * width => {
                        let inner = rect.inset(*width);
                        let inner_radius = (radius - width).max(0.0);
                        fill_path(&mut pixmap, rounded(&inner, inner_radius), *fill, FillRule::Winding, transform);
                        // The ring between the outline and the inner edge, so a translucent fill and border do
                        // not add up where they would overlap.
                        let mut ring = PathBuilder::new();
                        push_rounded(&mut ring, rect, radius);
                        push_rounded(&mut ring, &inner, inner_radius);
                        if let Some(ring) = ring.finish() {
                            fill_path(&mut pixmap, Some(ring), *color, FillRule::EvenOdd, transform);
                        }
                    }
                    _ => fill_path(&mut pixmap, rounded(rect, radius), *fill, FillRule::Winding, transform),
                }
            }
            Node::Label { rect, text, style, color, align } => {
                if color.a == 0 || text.is_empty() {
                    continue;
                }
                let text = fonts.elided(text, *style, rect.w);
                let width = fonts.width(&text, *style);
                let x = match align {
                    Align::Left => rect.x,
                    Align::Center => rect.x + (rect.w - width) / 2.0,
                };
                let top = rect.y + (rect.h - fonts.line_height(*style)) / 2.0;
                fonts.draw_line(&mut pixmap, &text, *style, *color, (x, top), scale);
            }
            Node::Paragraph { rect, text, style, color } => {
                if color.a > 0 && !text.is_empty() {
                    fonts.draw_paragraph(&mut pixmap, text, *style, *color, (rect.x, rect.y), rect.w, scale);
                }
            }
            Node::Arc { center, radius, width, color, start, sweep } => {
                let Some(path) = arc(*center, *radius, *start, *sweep) else { continue };
                let stroke = Stroke { width: *width, line_cap: LineCap::Round, ..Stroke::default() };
                pixmap.stroke_path(&path, &paint_of(*color), &stroke, transform, None);
            }
        }
    }
    Some(pixmap)
}

/// The size in pixels of something `width` × `height` logical pixels large at `scale`.
pub fn pixel_size(width: f32, height: f32, scale: f32) -> (u32, u32) {
    let pixels = |logical: f32| (logical * scale).ceil().clamp(0.0, 16384.0) as u32;
    (pixels(width), pixels(height))
}

fn paint_of(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.r, color.g, color.b, color.a);
    paint.anti_alias = true;
    paint
}

fn fill_path(pixmap: &mut Pixmap, path: Option<Path>, color: Color, rule: FillRule, transform: Transform) {
    if let Some(path) = path.filter(|_| color.a > 0) {
        pixmap.fill_path(&path, &paint_of(color), rule, transform, None);
    }
}

fn rounded(rect: &Rect, radius: f32) -> Option<Path> {
    let mut builder = PathBuilder::new();
    push_rounded(&mut builder, rect, radius);
    builder.finish()
}

/// The control points of a quarter circle drawn as one cubic curve lie this far along the tangents.
const KAPPA: f32 = 0.552_284_8;

fn push_rounded(builder: &mut PathBuilder, rect: &Rect, radius: f32) {
    let (x, y, r, b) = (rect.x, rect.y, rect.right(), rect.bottom());
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let k = radius * KAPPA;
    builder.move_to(x + radius, y);
    builder.line_to(r - radius, y);
    builder.cubic_to(r - radius + k, y, r, y + radius - k, r, y + radius);
    builder.line_to(r, b - radius);
    builder.cubic_to(r, b - radius + k, r - radius + k, b, r - radius, b);
    builder.line_to(x + radius, b);
    builder.cubic_to(x + radius - k, b, x, b - radius + k, x, b - radius);
    builder.line_to(x, y + radius);
    builder.cubic_to(x, y + radius - k, x + radius - k, y, x + radius, y);
    builder.close();
}

/// A circular arc from `start` over `sweep` radians, as cubic curves of at most a quarter circle each.
fn arc((cx, cy): (f32, f32), radius: f32, start: f32, sweep: f32) -> Option<Path> {
    if radius <= 0.0 || sweep.abs() < 1e-4 {
        return None;
    }
    let segments = (sweep.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = sweep / segments as f32;
    // Tangent length for a segment of `step` radians.
    let k = 4.0 / 3.0 * (step / 4.0).tan() * radius;
    let point = |angle: f32| (cx + radius * angle.cos(), cy + radius * angle.sin());
    let mut builder = PathBuilder::new();
    let (x, y) = point(start);
    builder.move_to(x, y);
    for segment in 0..segments {
        let a0 = start + step * segment as f32;
        let a1 = a0 + step;
        let (x0, y0) = point(a0);
        let (x1, y1) = point(a1);
        builder.cubic_to(x0 - k * a0.sin(), y0 + k * a0.cos(), x1 + k * a1.sin(), y1 - k * a1.cos(), x1, y1);
    }
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(pixmap: &Pixmap, x: u32, y: u32) -> [u8; 4] {
        let c = pixmap.pixel(x, y).expect("inside");
        [c.red(), c.green(), c.blue(), c.alpha()]
    }

    #[test]
    fn rectangles_have_their_border_inside_and_round_corners() {
        let mut scene: Scene<()> = Scene::new(40.0, 20.0);
        scene.push(Node::Rect {
            rect: Rect::new(0.0, 0.0, 40.0, 20.0),
            radius: 10.0,
            fill: Color::rgba(0, 0, 255, 255),
            border: Some((1.0, Color::rgba(255, 0, 0, 255))),
        });
        let mut fonts = Fonts::load();
        let pixmap = paint(&scene, &mut fonts, 1.0).expect("pixels");
        assert_eq!(pixel(&pixmap, 20, 10), [0, 0, 255, 255]);
        assert_eq!(pixel(&pixmap, 20, 0), [255, 0, 0, 255]);
        // The corner is cut off.
        assert_eq!(pixel(&pixmap, 0, 0)[3], 0);
        // Twice the scale: twice the pixels, same picture.
        let pixmap = paint(&scene, &mut fonts, 2.0).expect("pixels");
        assert_eq!((pixmap.width(), pixmap.height()), (80, 40));
        assert_eq!(pixel(&pixmap, 40, 20), [0, 0, 255, 255]);
    }

    #[test]
    fn arcs_cover_only_their_sweep() {
        let mut scene: Scene<()> = Scene::new(20.0, 20.0);
        let color = Color::rgba(255, 255, 255, 255);
        // The right half, clockwise from the top.
        let start = -std::f32::consts::FRAC_PI_2;
        scene.push(Node::Arc { center: (10.0, 10.0), radius: 8.0, width: 2.0, color, start, sweep: std::f32::consts::PI });
        let pixmap = paint(&scene, &mut Fonts::load(), 1.0).expect("pixels");
        assert!(pixel(&pixmap, 18, 10)[3] > 200, "right {:?}", pixel(&pixmap, 18, 10));
        assert_eq!(pixel(&pixmap, 1, 10)[3], 0);
    }
}
