//! Notifications: the card (`NotificationCard.qml`) that toasts and the notification center show, and the
//! notification center itself (`NotificationCenter.qml` in the panel frame of `bar.qml`).
use crate::{
    Align, Measure, Node, Rect, Scene, Shape, TextStyle, Theme,
    bar::{BUTTON_HEIGHT, button_shape},
    i18n::Language,
    markup::Span,
    panel::{Content, PanelTarget},
    scene::Picture,
    theme::CARD_RADIUS,
};

/// A notification as the cards show it.
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationView {
    pub id: u32,
    pub summary: String,
    pub app_name: String,
    pub body: Vec<Span>,
    pub critical: bool,
    /// Identifier and label of each action.
    pub actions: Vec<(String, String)>,
    pub picture: Option<Picture>,
}

/// What a click on a card means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardTarget {
    /// The card itself: nothing (but a toast under the pointer keeps showing).
    Card(u32),
    Dismiss(u32),
    /// The notification's action at this index.
    Action(u32, usize),
}

impl CardTarget {
    pub fn id(self) -> u32 {
        match self {
            CardTarget::Card(id) | CardTarget::Dismiss(id) | CardTarget::Action(id, _) => id,
        }
    }
}

/// A card laid out from its own top left corner.
pub struct Card {
    pub height: f32,
    pub nodes: Vec<Node>,
    pub targets: Vec<(Rect, CardTarget)>,
}

const PADDING: f32 = 12.0;
const IMAGE: f32 = 42.0;
const SPACING: f32 = 4.0;
const ACTION_SPACING: f32 = 6.0;
const TEXT: TextStyle = TextStyle::new(14.0);

/// The card of `view`, `width` wide. Toasts are `compact` (on their own: glass, three lines of body); in the
/// notification center cards sit on the panel's glass and show eight lines.
pub fn card(view: &NotificationView, theme: &Theme, measure: &mut impl Measure, width: f32, compact: bool, hover: Option<CardTarget>, language: Language) -> Card {
    let id = view.id;
    let mut nodes = Vec::new();
    let mut targets = Vec::new();
    let has_image = view.picture.is_some();
    let left = PADDING + if has_image { IMAGE + PADDING } else { 0.0 };
    // The column is as wide as the QML card's: the row less the picture and another 32 pixels.
    let column = (width - 2.0 * PADDING - if has_image { IMAGE + PADDING } else { 0.0 } - 32.0).max(0.0);
    let mut y = PADDING;

    // The cyberdeck gives a card a console tag (`MSG·07 · APP`) on top, in place of the app name below the text.
    let deck = theme.shape == Shape::Chamfer;
    if deck {
        let style = TextStyle::new(12.0);
        let height = measure.line_height(style);
        let app = if view.app_name.is_empty() { String::new() } else { format!(" · {}", view.app_name.to_uppercase()) };
        let color = if view.critical { theme.alert } else { theme.secondary };
        let rect = Rect::new(left, y, column, height);
        nodes.push(Node::Label { rect, text: format!("MSG·{:02}{app}", id % 100), style, color, align: Align::Left });
        y += height + SPACING;
    }

    let title_style = TextStyle::new(15.0).bold(true);
    let title_height = measure.line_height(title_style);
    let title = [&view.summary, &view.app_name].into_iter().find(|t| !t.is_empty()).cloned();
    let title = title.unwrap_or_else(|| language.tr("Benachrichtigung").to_owned());
    let close = Rect::new(left + column - 24.0, y, 24.0, BUTTON_HEIGHT);
    let title_rect = Rect::new(left, y, (column - close.w - 8.0).max(0.0), title_height);
    nodes.push(Node::Label { rect: title_rect, text: title, style: title_style, color: theme.text, align: Align::Left });
    let mut close_scene: Scene<()> = Scene::new(width, 0.0);
    button_shape(&mut close_scene, theme, close, "×", false, false, false, hover == Some(CardTarget::Dismiss(id)));
    nodes.extend(close_scene.nodes);
    targets.push((close, CardTarget::Dismiss(id)));
    y += title_height.max(BUTTON_HEIGHT);

    if !view.body.is_empty() {
        let max_lines = if compact { 3 } else { 8 };
        let height = measure.rich_height(&view.body, TEXT, column, max_lines);
        let rect = Rect::new(left, y + SPACING, column, height);
        nodes.push(Node::RichText { rect, spans: view.body.clone(), style: TEXT, color: theme.text, max_lines });
        y += SPACING + height;
    }
    if !view.app_name.is_empty() && !deck {
        let style = TextStyle::new(12.0);
        let height = measure.line_height(style);
        let rect = Rect::new(left, y + SPACING, column, height);
        nodes.push(Node::Label { rect, text: view.app_name.clone(), style, color: theme.muted, align: Align::Left });
        y += SPACING + height;
    }
    if !view.actions.is_empty() {
        // A `Flow`: buttons as wide as their labels, wrapping to the column's width.
        y += SPACING;
        let (mut x, mut row) = (0.0, 0.0);
        for (index, (identifier, text)) in view.actions.iter().enumerate() {
            let button = measure.width(text, TEXT).ceil() + 16.0;
            if x > 0.0 && x + button > column {
                x = 0.0;
                row += BUTTON_HEIGHT + ACTION_SPACING;
            }
            let rect = Rect::new(left + x, y + row, button, BUTTON_HEIGHT);
            let target = CardTarget::Action(id, index);
            let mut action: Scene<()> = Scene::new(width, 0.0);
            button_shape(&mut action, theme, rect, text, false, identifier == "default", false, hover == Some(target));
            nodes.extend(action.nodes);
            targets.push((rect, target));
            x += button + ACTION_SPACING;
        }
        y += row + BUTTON_HEIGHT;
    }
    let content = (y - PADDING).max(if has_image { IMAGE } else { 0.0 });
    let height = content + 2.0 * PADDING;
    if let Some(picture) = &view.picture {
        nodes.insert(0, Node::Image { rect: Rect::new(PADDING, PADDING, IMAGE, IMAGE), picture: picture.clone() });
    }
    let fill = if compact { theme.glass() } else { theme.glass_surface() };
    // A card in the center sits on the panel's glass; a toast floats, so it needs an edge.
    let border = if view.critical { Some((2.0, theme.accent)) } else { compact.then(|| (1.0, theme.glass_border())) };
    nodes.insert(0, Node::Rect { rect: Rect::new(0.0, 0.0, width, height), radius: CARD_RADIUS, fill, border });
    targets.insert(0, (Rect::new(0.0, 0.0, width, height), CardTarget::Card(id)));
    Card { height, nodes, targets }
}

const LIST_SPACING: f32 = 8.0;

/// The notification center's content (`NotificationCenter.qml`), `width` wide: the count with "Clear all", and
/// the list, which scrolls by itself once its cards need more than 470 pixels. Also how far the list can scroll.
pub fn center(
    items: &[NotificationView],
    list_scroll: f32,
    hover: Option<&PanelTarget>,
    theme: &Theme,
    measure: &mut impl Measure,
    width: f32,
    language: Language,
) -> (Content, f32) {
    let tr = |text| language.tr(text);
    let count = items.len();
    let mut content = Content::default();

    let heading_style = crate::panel::HEADING;
    let heading = if count == 1 { tr("1 Benachrichtigung").to_owned() } else { language.trf("%1 Benachrichtigungen", &[&count.to_string()]) };
    let clear_text = tr("Alle löschen");
    let clear_width = measure.width(clear_text, TEXT).ceil() + 16.0;
    let heading_height = measure.line_height(heading_style).max(BUTTON_HEIGHT);
    let heading_rect = Rect::new(0.0, (heading_height - measure.line_height(heading_style)) / 2.0, width - clear_width - 8.0, measure.line_height(heading_style));
    content.push(Node::Label { rect: heading_rect, text: heading, style: heading_style, color: theme.muted, align: Align::Left });
    let clear = Rect::new(width - clear_width, 0.0, clear_width, BUTTON_HEIGHT);
    let mut button: Scene<()> = Scene::new(width, 0.0);
    button_shape(&mut button, theme, clear, clear_text, false, false, false, count > 0 && hover == Some(&PanelTarget::ClearAll));
    // Disabled without notifications, like a QML item with `enabled: false`.
    let fade = if count > 0 { 1.0 } else { 0.45 };
    content.nodes.extend(button.nodes.into_iter().map(|node| node.faded(fade)));
    if count > 0 {
        content.target(clear, PanelTarget::ClearAll);
    }

    let card_hover = match hover {
        Some(PanelTarget::Card(target)) => Some(*target),
        _ => None,
    };
    let natural = |cards: &[Card]| cards.iter().map(|c| c.height).sum::<f32>() + LIST_SPACING * cards.len().saturating_sub(1) as f32;
    let mut cards: Vec<Card> = items.iter().map(|view| card(view, theme, measure, width, false, card_hover, language)).collect();
    let mut overflow = natural(&cards) > 470.0;
    if overflow {
        // A visible scroll bar takes ten pixels off the cards.
        cards = items.iter().map(|view| card(view, theme, measure, width - 10.0, false, card_hover, language)).collect();
        overflow = natural(&cards) > 470.0;
    }
    let cards_height = natural(&cards);
    let list_height = cards_height.clamp(80.0, 470.0);
    let max_list_scroll = (cards_height - list_height).max(0.0);
    let scroll = list_scroll.clamp(0.0, max_list_scroll);

    let list = Rect::new(0.0, heading_height + 12.0, width, list_height);
    content.target(list, PanelTarget::List);
    content.push(Node::Clip(Some(list)));
    if cards.is_empty() {
        let style = TextStyle::new(14.0);
        content.push(Node::Label { rect: list, text: tr("Keine Benachrichtigungen").into(), style, color: theme.muted, align: Align::Center });
    }
    let mut y = list.y - scroll;
    for card in cards {
        for node in card.nodes {
            content.push(node.translated(0.0, y));
        }
        for (rect, target) in card.targets {
            // Only what shows in the list reacts to the pointer.
            let rect = crate::panel::intersect(rect.translated(0.0, y), list);
            if rect.h > 0.0 {
                content.target(rect, PanelTarget::Card(target));
            }
        }
        y += card.height + LIST_SPACING;
    }
    // The scroll bar shows while the pointer is over the list (as Qt's does while it is active).
    if overflow && hover.is_some_and(|h| matches!(h, PanelTarget::List | PanelTarget::Card(_))) {
        let length = (list.h * list.h / cards_height).max(20.0);
        let position = list.y + (list.h - length) * if max_list_scroll > 0.0 { scroll / max_list_scroll } else { 0.0 };
        let handle = Rect::new(list.right() - 2.0 - 6.0, position, 6.0, length);
        content.push(Node::Rect { rect: handle, radius: 3.0, fill: theme.muted.faded(0.75), border: None });
    }
    content.push(Node::Clip(None));
    content.height = list.bottom();
    (content, max_list_scroll)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Color, scene::tests::Mono, theme::Palette};

    fn theme() -> Theme {
        let c = |v| Color::rgba(v, v, v, 255);
        Theme::new(Palette { background: c(10), surface: c(20), text: c(200), muted: c(150), accent: c(90), border: c(60) }, None)
    }

    fn view(id: u32) -> NotificationView {
        NotificationView {
            id,
            summary: format!("Titel {id}"),
            app_name: "App".into(),
            body: crate::markup::parse("Text"),
            critical: false,
            actions: vec![("default".into(), "Öffnen".into()), ("later".into(), "Später".into())],
            picture: None,
        }
    }

    #[test]
    fn a_card_stacks_title_body_app_and_actions() {
        let card = card(&view(1), &theme(), &mut Mono, 390.0, true, None, Language::English);
        // Title row (26), body (19), app name (16), actions (26), with 4 between and 12 around.
        assert_eq!(card.height, 12.0 + 26.0 + 4.0 + 19.0 + 4.0 + 16.0 + 4.0 + 26.0 + 12.0);
        let close = card.targets.iter().find(|(_, t)| *t == CardTarget::Dismiss(1)).unwrap().0;
        // The close button ends 32 pixels before the card's padding, as in the QML card.
        assert_eq!(close.right(), 390.0 - 12.0 - 32.0);
        let first = card.targets.iter().find(|(_, t)| *t == CardTarget::Action(1, 0)).unwrap().0;
        let second = card.targets.iter().find(|(_, t)| *t == CardTarget::Action(1, 1)).unwrap().0;
        assert_eq!(second.x, first.right() + 6.0);
        assert_eq!(card.targets[0], (Rect::new(0.0, 0.0, 390.0, card.height), CardTarget::Card(1)));
    }

    #[test]
    fn the_cyberdeck_card_has_a_console_tag_in_place_of_the_app_name() {
        let deck = theme().with_shape(Shape::Chamfer);
        let card = card(&view(107), &deck, &mut Mono, 390.0, true, None, Language::English);
        let labels: Vec<(&str, crate::Color)> =
            card.nodes.iter().filter_map(|n| match n { Node::Label { text, color, .. } => Some((text.as_str(), *color)), _ => None }).collect();
        // The tag on top in the accent color (the id modulo 100), the app name not repeated below.
        assert_eq!(labels[0], ("MSG·07 · APP", deck.accent));
        assert!(!labels.iter().any(|(text, _)| *text == "App"), "{labels:?}");
        // The same height as the normal card: the tag takes the app name's row.
        let plain = super::card(&view(107), &theme(), &mut Mono, 390.0, true, None, Language::English);
        assert_eq!(card.height, plain.height);
        let critical = NotificationView { critical: true, ..view(1) };
        let card = super::card(&critical, &deck, &mut Mono, 390.0, true, None, Language::English);
        assert!(card.nodes.iter().any(|n| matches!(n, Node::Label { text, color, .. } if text.starts_with("MSG·01") && *color == deck.alert)));
    }

    #[test]
    fn a_picture_moves_the_text_and_counts_as_content() {
        let picture = Picture::from_rgba(1, 1, &[0, 0, 0, 255]).unwrap();
        let bare = NotificationView { body: Vec::new(), app_name: String::new(), actions: Vec::new(), picture: Some(picture), ..view(2) };
        let card = card(&bare, &theme(), &mut Mono, 390.0, false, None, Language::English);
        assert_eq!(card.height, 12.0 + 42.0 + 12.0);
        let title = card.nodes.iter().find_map(|n| match n { Node::Label { rect, .. } => Some(*rect), _ => None }).unwrap();
        assert_eq!(title.x, 12.0 + 42.0 + 12.0);
    }

    #[test]
    fn the_center_lists_newest_first_and_scrolls_when_full() {
        let items: Vec<_> = (1..=8).rev().map(view).collect();
        let (content, max_scroll) = center(&items, 0.0, None, &theme(), &mut Mono, 360.0, Language::English);
        assert!(max_scroll > 0.0, "eight cards overflow 470 pixels");
        assert_eq!(content.height, 26.0 + 12.0 + 470.0);
        let texts: Vec<_> = content.nodes.iter().filter_map(|n| match n { Node::Label { text, .. } => Some(text.clone()), _ => None }).collect();
        assert!(texts.contains(&"8 notifications".to_owned()), "{texts:?}");
        let first = texts.iter().position(|t| t == "Titel 8").unwrap();
        assert!(texts.iter().position(|t| t == "Titel 7").unwrap() > first);
        let has = |content: &Content, target: PanelTarget| content.targets.iter().any(|(_, t)| *t == target);
        assert!(has(&content, PanelTarget::ClearAll));
        // Scrolled to the end, the last card's close button can be reached; scrolling further changes nothing.
        let end = center(&items, 10_000.0, None, &theme(), &mut Mono, 360.0, Language::English).0;
        assert!(has(&end, PanelTarget::Card(CardTarget::Dismiss(1))));
        assert!(!has(&content, PanelTarget::Card(CardTarget::Dismiss(1))));
    }

    #[test]
    fn an_empty_center_says_so_and_cannot_clear() {
        let (content, max_scroll) = center(&[], 0.0, None, &theme(), &mut Mono, 360.0, Language::English);
        assert_eq!(max_scroll, 0.0);
        assert!(!content.targets.iter().any(|(_, t)| *t == PanelTarget::ClearAll));
        assert!(content.nodes.iter().any(|n| matches!(n, Node::Label { text, .. } if text == "No notifications")));
        assert_eq!(content.height, 26.0 + 12.0 + 80.0);
    }
}
