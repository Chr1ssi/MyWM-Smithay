//! Notifications: the card (`NotificationCard.qml`) that toasts and the notification center show, and the
//! notification center itself (`NotificationCenter.qml` in the panel frame of `bar.qml`).
use crate::{
    Align, Measure, Node, Rect, Scene, TextStyle, Theme,
    bar::{BUTTON_HEIGHT, button_shape},
    i18n::Language,
    markup::Span,
    scene::Picture,
    theme::{BAR_HEIGHT, PANEL_RADIUS},
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
    if !view.app_name.is_empty() {
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
    let border = if view.critical { (2.0, theme.accent) } else { (1.0, theme.glass_border()) };
    nodes.insert(0, Node::Rect { rect: Rect::new(0.0, 0.0, width, height), radius: 18.0, fill, border: Some(border) });
    targets.insert(0, (Rect::new(0.0, 0.0, width, height), CardTarget::Card(id)));
    Card { height, nodes, targets }
}

/// Distance of panels from the top of the monitor (below the bar) and from its right edge.
pub const PANEL_TOP: f32 = BAR_HEIGHT + 6.0;
pub const PANEL_RIGHT: f32 = 8.0;
const PANEL_MARGIN: f32 = 20.0;
const LIST_SPACING: f32 = 8.0;
/// How far the wheel moves the list per notch.
pub const SCROLL_STEP: f32 = 60.0;

#[derive(Clone, Debug, PartialEq)]
pub struct CenterModel {
    /// The monitor's size.
    pub output: (f32, f32),
    /// Newest first.
    pub items: Vec<NotificationView>,
    /// How far the list is scrolled down.
    pub scroll: f32,
    pub hover: Option<PanelTarget>,
    pub language: Language,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelTarget {
    /// The panel itself: nothing.
    Background,
    /// The list (the wheel scrolls it).
    List,
    ClearAll,
    Card(CardTarget),
}

/// The notification center: its picture with the panel at (0, 0), and how far its list can scroll.
pub fn center(model: &CenterModel, theme: &Theme, measure: &mut impl Measure) -> (Scene<PanelTarget>, f32) {
    let tr = |text| model.language.tr(text);
    let width = (model.output.0 - 24.0).min(400.0);
    let inner = width - 2.0 * PANEL_MARGIN;
    let count = model.items.len();

    let heading_style = TextStyle::new(15.0);
    let heading = if count == 1 { tr("1 Benachrichtigung").to_owned() } else { model.language.trf("%1 Benachrichtigungen", &[&count.to_string()]) };
    let clear_text = tr("Alle löschen");
    let clear_width = measure.width(clear_text, TEXT).ceil() + 16.0;
    let heading_height = measure.line_height(heading_style).max(BUTTON_HEIGHT);

    // The list's cards; the list scrolls when they need more than 470 pixels.
    let mut cards: Vec<Card> = Vec::new();
    let card_hover = match model.hover {
        Some(PanelTarget::Card(target)) => Some(target),
        _ => None,
    };
    let natural = |cards: &[Card]| cards.iter().map(|c| c.height).sum::<f32>() + LIST_SPACING * cards.len().saturating_sub(1) as f32;
    for view in &model.items {
        cards.push(card(view, theme, measure, inner, false, card_hover, model.language));
    }
    let mut overflow = natural(&cards) > 470.0;
    if overflow {
        // A visible scroll bar takes ten pixels off the cards.
        cards = model.items.iter().map(|view| card(view, theme, measure, inner - 10.0, false, card_hover, model.language)).collect();
        overflow = natural(&cards) > 470.0;
    }
    let content = natural(&cards);
    let list_height = content.clamp(80.0, 470.0);
    let max_scroll = (content - list_height).max(0.0);
    let scroll = model.scroll.clamp(0.0, max_scroll);

    let title_style = TextStyle::new(20.0);
    let title_height = measure.line_height(title_style);
    let loaded = heading_height + 12.0 + list_height;
    let height = (loaded + 82.0).max(150.0).min(model.output.1 - BAR_HEIGHT - 18.0);
    let mut scene = Scene::new(width, height);
    let panel = Rect::new(0.0, 0.0, width, height);
    scene.push(Node::Rect { rect: panel, radius: PANEL_RADIUS, fill: theme.glass(), border: Some((1.0, theme.glass_border())) });
    scene.target(panel, PanelTarget::Background);
    let title = tr("Benachrichtigungen").to_owned();
    let title_rect = Rect::new(PANEL_MARGIN, PANEL_MARGIN, inner, title_height);
    scene.push(Node::Label { rect: title_rect, text: title, style: title_style, color: theme.text, align: Align::Left });

    // The panel's own scroll area holds the center; it is cut off at the panel's height.
    let area = Rect::new(PANEL_MARGIN, PANEL_MARGIN + 42.0, inner, (height - 82.0).max(0.0));
    scene.push(Node::Clip(Some(area)));
    let top = area.y;
    let heading_rect = Rect::new(area.x, top, inner - clear_width - 8.0, measure.line_height(heading_style));
    scene.push(Node::Label { rect: heading_rect, text: heading, style: heading_style, color: theme.text, align: Align::Left });
    let clear = Rect::new(area.x + inner - clear_width, top, clear_width, BUTTON_HEIGHT);
    let mut button: Scene<()> = Scene::new(width, height);
    button_shape(&mut button, theme, clear, clear_text, false, false, false, count > 0 && model.hover == Some(PanelTarget::ClearAll));
    // Disabled without notifications, like a QML item with `enabled: false`.
    let fade = if count > 0 { 1.0 } else { 0.45 };
    scene.nodes.extend(button.nodes.into_iter().map(|node| node.faded(fade)));
    if count > 0 {
        scene.target(clear, PanelTarget::ClearAll);
    }

    let list = Rect::new(area.x, top + heading_height + 12.0, inner, list_height);
    let visible = Rect::new(list.x, list.y, list.w, list.h.min((area.bottom() - list.y).max(0.0)));
    scene.target(visible, PanelTarget::List);
    scene.push(Node::Clip(Some(visible)));
    if cards.is_empty() {
        let style = TextStyle::new(14.0);
        let rect = Rect::new(list.x, list.y, list.w, list.h);
        scene.push(Node::Label { rect, text: tr("Keine Benachrichtigungen").into(), style, color: theme.muted, align: Align::Center });
    }
    let mut y = list.y - scroll;
    for card in cards {
        for node in card.nodes {
            scene.push(node.translated(list.x, y));
        }
        for (rect, target) in card.targets {
            // Only what shows in the list reacts to the pointer.
            let rect = rect.translated(list.x, y);
            let top = rect.y.max(visible.y);
            let bottom = rect.bottom().min(visible.bottom());
            if bottom > top {
                scene.target(Rect::new(rect.x, top, rect.w, bottom - top), PanelTarget::Card(target));
            }
        }
        y += card.height + LIST_SPACING;
    }
    // The scroll bar shows while the pointer is over the list (as Qt's does while it is active).
    if overflow && model.hover.is_some_and(|h| h != PanelTarget::Background && h != PanelTarget::ClearAll) {
        let length = (list.h * list.h / content).max(20.0);
        let position = list.y + (list.h - length) * if max_scroll > 0.0 { scroll / max_scroll } else { 0.0 };
        let handle = Rect::new(list.right() - 2.0 - 6.0, position, 6.0, length);
        scene.push(Node::Rect { rect: handle, radius: 3.0, fill: theme.muted.faded(0.75), border: None });
    }
    scene.push(Node::Clip(None));
    (scene, max_scroll)
}

/// Where a panel of `width` sits on a monitor `output_width` wide (logical, relative to the monitor).
pub fn panel_origin(output_width: f32, width: f32) -> (f32, f32) {
    (output_width - PANEL_RIGHT - width, PANEL_TOP)
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
        let model = CenterModel { output: (1280.0, 800.0), items: (1..=8).rev().map(view).collect(), scroll: 0.0, hover: None, language: Language::English };
        let (scene, max_scroll) = center(&model, &theme(), &mut Mono);
        assert_eq!(scene.width, 400.0);
        assert!(max_scroll > 0.0, "eight cards overflow 470 pixels");
        let texts: Vec<_> = scene.nodes.iter().filter_map(|n| match n { Node::Label { text, .. } => Some(text.clone()), _ => None }).collect();
        assert!(texts.contains(&"8 notifications".to_owned()) && texts.contains(&"Notifications".to_owned()), "{texts:?}");
        let first = texts.iter().position(|t| t == "Titel 8").unwrap();
        assert!(texts.iter().position(|t| t == "Titel 7").unwrap() > first);
        assert!(scene.target_rect(&PanelTarget::ClearAll).is_some());
        // Scrolled to the end, the last card's close button can be reached; scrolling further changes nothing.
        let end = center(&CenterModel { scroll: 10_000.0, ..model.clone() }, &theme(), &mut Mono).0;
        assert!(end.target_rect(&PanelTarget::Card(CardTarget::Dismiss(1))).is_some());
        assert!(scene.target_rect(&PanelTarget::Card(CardTarget::Dismiss(1))).is_none());
    }

    #[test]
    fn an_empty_center_says_so_and_cannot_clear() {
        let model = CenterModel { output: (1280.0, 800.0), items: Vec::new(), scroll: 0.0, hover: None, language: Language::English };
        let (scene, max_scroll) = center(&model, &theme(), &mut Mono);
        assert_eq!(max_scroll, 0.0);
        assert!(scene.target_rect(&PanelTarget::ClearAll).is_none());
        assert!(scene.nodes.iter().any(|n| matches!(n, Node::Label { text, .. } if text == "No notifications")));
        // 26 + 12 + 80 of content and 82 of frame.
        assert_eq!(scene.height, 26.0 + 12.0 + 80.0 + 82.0);
    }
}
