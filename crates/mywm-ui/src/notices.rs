//! The cards at the top right of the first monitor (the toast column of `bar.qml`): what to do while an app
//! asks what to share, the color under the pointer while the color picker is on, and why the configuration
//! file did not load.
use crate::{Align, Color, Measure, Node, Rect, Scene, TextStyle, Theme, bar::button_shape, i18n::Language};

/// Distance from the top of the monitor (below the bar) and from its right edge.
pub const TOP: f32 = crate::theme::BAR_HEIGHT + 6.0;
pub const RIGHT: f32 = 8.0;
const SPACING: f32 = 8.0;
const RADIUS: f32 = 18.0;
const PADDING: f32 = 12.0;

#[derive(Clone, Debug, PartialEq)]
pub struct NoticesModel {
    /// The monitor's width; the column is at most 390 wide.
    pub output_width: f32,
    pub choosing: bool,
    /// The color picker is on, with the color under the pointer once it has one (`#rrggbb`).
    pub picking: Option<Option<String>>,
    pub config_error: Option<String>,
    pub hover: Option<NoticeTarget>,
    pub language: Language,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeTarget {
    /// Anywhere on the column: nothing, and the click does not reach a window either.
    Background,
    DismissConfigError,
}

/// One card of the column, laid out from its own top.
struct Card {
    height: f32,
    nodes: Vec<Node>,
    targets: Vec<(Rect, NoticeTarget)>,
}

/// The column's width on a monitor `output_width` wide.
pub fn width(output_width: f32) -> f32 {
    (output_width - 24.0).min(390.0)
}

/// The column's picture and targets, `None` while it has nothing to show.
pub fn scene(model: &NoticesModel, theme: &Theme, measure: &mut impl Measure) -> Option<Scene<NoticeTarget>> {
    if !model.choosing && model.picking.is_none() && model.config_error.is_none() {
        return None;
    }
    let width = width(model.output_width);
    let tr = |text| model.language.tr(text);
    let inner = width - 2.0 * PADDING;
    let mut cards: Vec<Card> = Vec::new();

    if model.choosing {
        let title = TextStyle::new(15.0).bold(true);
        let body = TextStyle::new(13.0);
        let text = tr("Fenster anklicken · Kachel daneben oder Enter: ganzer Monitor · D: dynamisch, mit den Cast-Tasten umschaltbar · Esc: abbrechen");
        let title_h = measure.line_height(title);
        let body_h = measure.paragraph_height(text, body, inner);
        let height = title_h + 4.0 + body_h + 2.0 * PADDING;
        let nodes = vec![
            Node::Rect { rect: Rect::new(0.0, 0.0, width, height), radius: RADIUS, fill: theme.glass(), border: Some((2.0, theme.alert)) },
            label(PADDING, PADDING, inner, title_h, format!("󰑊 {}", tr("Was soll geteilt werden?")), title, theme.text),
            Node::Paragraph { rect: Rect::new(PADDING, PADDING + title_h + 4.0, inner, body_h), text: text.into(), style: body, color: theme.text },
        ];
        cards.push(Card { height, nodes, targets: Vec::new() });
    }

    if let Some(picked) = &model.picking {
        let height = 72.0;
        let swatch = Rect::new(PADDING, (height - 48.0) / 2.0, 48.0, 48.0);
        let swatch_color = picked.as_deref().and_then(Color::parse).unwrap_or(Color::TRANSPARENT);
        let title = TextStyle::new(15.0).bold(true);
        let body = TextStyle::new(13.0);
        let left = swatch.right() + PADDING;
        let column = width - left - PADDING;
        let text = tr("Klick kopiert die Farbe · Esc: abbrechen");
        let (title_h, body_h) = (measure.line_height(title), measure.paragraph_height(text, body, column));
        let top = (height - (title_h + 4.0 + body_h)) / 2.0;
        let heading = format!("󰈊 {}", picked.as_deref().filter(|c| !c.is_empty()).unwrap_or(tr("Farbpipette")));
        let nodes = vec![
            Node::Rect { rect: Rect::new(0.0, 0.0, width, height), radius: RADIUS, fill: theme.glass(), border: Some((1.0, theme.glass_border())) },
            Node::Rect { rect: swatch, radius: 12.0, fill: swatch_color, border: Some((1.0, theme.border)) },
            label(left, top, column, title_h, heading, title, theme.text),
            Node::Paragraph { rect: Rect::new(left, top + title_h + 4.0, column, body_h), text: text.into(), style: body, color: theme.muted },
        ];
        cards.push(Card { height, nodes, targets: Vec::new() });
    }

    if let Some(error) = &model.config_error {
        let title = TextStyle::new(15.0).bold(true);
        let (body, note) = (TextStyle::new(14.0), TextStyle::new(12.0));
        let note_text = tr("Bis zur Korrektur gilt die zuletzt geladene Konfiguration, beim Start die Standardwerte.");
        let row_h = measure.line_height(title).max(crate::bar::BUTTON_HEIGHT);
        let body_h = measure.paragraph_height(error, body, inner);
        let note_h = measure.paragraph_height(note_text, note, inner);
        let height = row_h + 4.0 + body_h + 4.0 + note_h + 2.0 * PADDING;
        let close = Rect::new(width - PADDING - 24.0, PADDING, 24.0, crate::bar::BUTTON_HEIGHT);
        let mut nodes = vec![
            Node::Rect { rect: Rect::new(0.0, 0.0, width, height), radius: RADIUS, fill: theme.glass(), border: Some((2.0, theme.accent)) },
            label(PADDING, PADDING, inner - close.w - 8.0, measure.line_height(title), tr("Konfiguration fehlerhaft").into(), title, theme.text),
        ];
        let mut button: Scene<NoticeTarget> = Scene::new(width, height);
        button_shape(&mut button, theme, close, "×", false, false, false, model.hover == Some(NoticeTarget::DismissConfigError));
        nodes.extend(button.nodes);
        let body_top = PADDING + row_h + 4.0;
        nodes.push(Node::Paragraph { rect: Rect::new(PADDING, body_top, inner, body_h), text: error.clone(), style: body, color: theme.text });
        let note_rect = Rect::new(PADDING, body_top + body_h + 4.0, inner, note_h);
        nodes.push(Node::Paragraph { rect: note_rect, text: note_text.into(), style: note, color: theme.muted });
        cards.push(Card { height, nodes, targets: vec![(close, NoticeTarget::DismissConfigError)] });
    }

    let height = cards.iter().map(|card| card.height).sum::<f32>() + SPACING * cards.len().saturating_sub(1) as f32;
    let mut scene = Scene::new(width, height);
    scene.target(Rect::new(0.0, 0.0, width, height), NoticeTarget::Background);
    let mut y = 0.0;
    for card in cards {
        for node in card.nodes {
            scene.push(shifted(node, y));
        }
        for (rect, target) in card.targets {
            scene.target(rect.translated(0.0, y), target);
        }
        y += card.height + SPACING;
    }
    Some(scene)
}

fn label(x: f32, y: f32, w: f32, h: f32, text: String, style: TextStyle, color: Color) -> Node {
    Node::Label { rect: Rect::new(x, y, w, h), text, style, color, align: Align::Left }
}

/// `node` moved down by `dy`.
fn shifted(node: Node, dy: f32) -> Node {
    match node {
        Node::Rect { rect, radius, fill, border } => Node::Rect { rect: rect.translated(0.0, dy), radius, fill, border },
        Node::Label { rect, text, style, color, align } => Node::Label { rect: rect.translated(0.0, dy), text, style, color, align },
        Node::Paragraph { rect, text, style, color } => Node::Paragraph { rect: rect.translated(0.0, dy), text, style, color },
        Node::Arc { center, radius, width, color, start, sweep } => {
            Node::Arc { center: (center.0, center.1 + dy), radius, width, color, start, sweep }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{scene::tests::Mono, theme::Palette};

    fn theme() -> Theme {
        let c = |v| Color::rgba(v, v, v, 255);
        Theme::new(Palette { background: c(10), surface: c(20), text: c(200), muted: c(150), accent: c(90), border: c(60) }, None)
    }

    fn model() -> NoticesModel {
        NoticesModel { output_width: 1280.0, choosing: false, picking: None, config_error: None, hover: None, language: Language::English }
    }

    #[test]
    fn nothing_to_say_shows_nothing() {
        assert!(scene(&model(), &theme(), &mut Mono).is_none());
    }

    #[test]
    fn cards_stack_in_their_order() {
        let all = NoticesModel {
            choosing: true,
            picking: Some(Some("#112233".into())),
            config_error: Some("[appearance] gaps_inner: invalid type".into()),
            ..model()
        };
        let scene = scene(&all, &theme(), &mut Mono).expect("cards");
        assert_eq!(scene.width, 390.0);
        let cards: Vec<Rect> = scene
            .nodes
            .iter()
            .filter_map(|n| match n { Node::Rect { rect, radius, .. } if *radius == RADIUS => Some(*rect), _ => None })
            .collect();
        assert_eq!(cards.len(), 3);
        assert_eq!(cards[1].y, cards[0].bottom() + SPACING);
        assert_eq!(cards[1].h, 72.0);
        assert_eq!(scene.height, cards[2].bottom());
        assert!(scene.nodes.iter().any(|n| matches!(n, Node::Label { text, .. } if text == "󰈊 #112233")));
        let close = scene.target_rect(&NoticeTarget::DismissConfigError).expect("close button");
        assert_eq!(close.right(), 390.0 - PADDING);
        assert!(close.y > cards[2].y);
    }

    #[test]
    fn the_picker_names_itself_until_it_has_a_color() {
        let scene = scene(&NoticesModel { picking: Some(None), ..model() }, &theme(), &mut Mono).expect("card");
        assert!(scene.nodes.iter().any(|n| matches!(n, Node::Label { text, .. } if text == "󰈊 Color picker")));
        assert_eq!(width(300.0), 276.0);
    }
}
