//! A tray item's menu (`TrayMenu.qml` in the menu window of `bar.qml`): below the bar at the right edge, the
//! item's title with a way back out of submenus, and the entries: separators, check marks and radio buttons, an
//! icon (a glyph for symbolic ones), the label with its mnemonic, and "›" before a submenu.
use crate::{
    Align, Measure, Node, Rect, Scene, TextStyle, Theme,
    bar::{BUTTON_HEIGHT, button_shape},
    i18n::Language,
    markup::Span,
    scene::Picture,
    theme::{BAR_HEIGHT, PANEL_RADIUS},
};

pub const TOP: f32 = BAR_HEIGHT + 6.0;
pub const RIGHT: f32 = 8.0;
const PADDING: f32 = 12.0;
const ENTRY: f32 = 34.0;
const SEPARATOR: f32 = 9.0;
const SPACING: f32 = 2.0;
const MAX_LIST: f32 = 440.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Toggle {
    #[default]
    None,
    Check,
    Radio,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntryView {
    pub label: Vec<Span>,
    pub separator: bool,
    pub enabled: bool,
    pub toggle: Toggle,
    pub checked: bool,
    /// The entry's icon name, which decides whether it shows as a glyph (symbolic icons do).
    pub icon_name: String,
    pub picture: Option<Picture>,
    pub submenu: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrayMenuModel {
    pub output_width: f32,
    pub title: String,
    pub entries: Vec<EntryView>,
    /// A submenu shows: the "←" button leads back.
    pub can_go_back: bool,
    pub scroll: f32,
    pub hover: Option<MenuTarget>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuTarget {
    /// The menu itself: nothing.
    Background,
    Back,
    /// The entries (the wheel scrolls them).
    List,
    Entry(usize),
}

/// The glyph a symbolic icon shows as, chosen by what the icon and label say (as the QML menu does).
pub fn symbolic_glyph(icon: &str, label: &str) -> Option<&'static str> {
    if !icon.contains("symbolic") {
        return None;
    }
    let words = format!("{icon} {label}").to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| words.contains(n));
    Some(if has(&["exit", "quit", "beenden"]) {
        "󰍃"
    } else if has(&["setting", "preference", "einstellung"]) {
        "󰒓"
    } else if has(&["window", "show", "open", "anzeigen"]) {
        "󰖯"
    } else if has(&["about", "help", "info"]) {
        "󰋼"
    } else if has(&["bypass", "enable", "aktiv"]) {
        "󰈈"
    } else {
        "•"
    })
}

/// The menu's title: the item's title, else its id, else "Anwendung".
pub fn title(title: &str, id: &str, language: Language) -> String {
    [title, id].into_iter().find(|t| !t.is_empty()).unwrap_or(language.tr("Anwendung")).to_owned()
}

pub fn width(output_width: f32) -> f32 {
    (output_width - 16.0).min(320.0)
}

/// The menu's picture with the menu at (0, 0), and how far its entries can scroll.
pub fn scene(model: &TrayMenuModel, theme: &Theme, measure: &mut impl Measure) -> (Scene<MenuTarget>, f32) {
    let width = width(model.output_width);
    let inner = width - 2.0 * PADDING;
    let title_style = TextStyle::new(15.0).bold(true);
    let title_height = measure.line_height(title_style);
    let header = if model.can_go_back { title_height.max(BUTTON_HEIGHT) } else { title_height };
    let entries_height: f32 = model.entries.iter().map(|e| if e.separator { SEPARATOR } else { ENTRY }).sum::<f32>()
        + SPACING * model.entries.len().saturating_sub(1) as f32;
    let list_height = entries_height.min(MAX_LIST);
    let max_scroll = (entries_height - list_height).max(0.0);
    let scroll = model.scroll.clamp(0.0, max_scroll);
    let height = (header + 4.0 + list_height + 2.0 * PADDING).min(500.0);

    let mut scene = Scene::new(width, height);
    let menu = Rect::new(0.0, 0.0, width, height);
    scene.push(Node::Rect { rect: menu, radius: PANEL_RADIUS, fill: theme.glass(), border: Some((1.0, theme.glass_border())) });
    scene.target(menu, MenuTarget::Background);

    let mut title_x = PADDING;
    if model.can_go_back {
        let back_width = measure.width("←", TextStyle::new(14.0)).ceil() + 16.0;
        let back = Rect::new(PADDING, PADDING, back_width, BUTTON_HEIGHT);
        button_shape(&mut scene, theme, back, "←", false, false, false, model.hover == Some(MenuTarget::Back));
        scene.target(back, MenuTarget::Back);
        title_x += back_width + 6.0;
    }
    let title_width = inner - if model.can_go_back { 38.0 } else { 0.0 };
    let title_rect = Rect::new(title_x, PADDING + (header - title_height) / 2.0, title_width, title_height);
    scene.push(Node::Label { rect: title_rect, text: model.title.clone(), style: title_style, color: theme.text, align: Align::Left });

    let list = Rect::new(PADDING, PADDING + header + 4.0, inner, list_height);
    scene.target(list, MenuTarget::List);
    scene.push(Node::Clip(Some(list)));
    let mut y = list.y - scroll;
    for (index, entry) in model.entries.iter().enumerate() {
        if entry.separator {
            scene.push(Node::Rect { rect: Rect::new(list.x, y + 4.0, inner, 1.0), radius: 0.0, fill: theme.border, border: None });
            y += SEPARATOR + SPACING;
            continue;
        }
        let rect = Rect::new(list.x, y, inner, ENTRY);
        let target = MenuTarget::Entry(index);
        let mut nodes = Vec::new();
        let hovered = entry.enabled && model.hover == Some(target);
        if hovered {
            nodes.push(Node::Rect { rect, radius: 12.0, fill: theme.surface, border: None });
        }
        let row = |x: f32, w: f32| Rect::new(rect.x + 8.0 + x, rect.y, w, ENTRY);
        let indicator = match (entry.toggle, entry.checked) {
            (Toggle::None, _) => "",
            (_, true) => "✓",
            (_, false) => "○",
        };
        let style = TextStyle::new(14.0);
        nodes.push(Node::Label { rect: row(0.0, 18.0), text: indicator.into(), style, color: theme.accent, align: Align::Center });
        let glyph = symbolic_glyph(&entry.icon_name, &crate::markup::plain(&entry.label));
        let has_icon = glyph.is_some() || entry.picture.is_some();
        if let Some(glyph) = glyph {
            nodes.push(Node::Label { rect: row(26.0, 18.0), text: glyph.into(), style: TextStyle::new(15.0), color: theme.text, align: Align::Center });
        } else if let Some(picture) = &entry.picture {
            nodes.push(Node::Image { rect: Rect::new(rect.x + 8.0 + 26.0, rect.y + 8.0, 18.0, 18.0), picture: picture.clone() });
        }
        let label_x = if has_icon { 52.0 } else { 26.0 };
        let label_width = (inner - 16.0 - 52.0 - if has_icon { 26.0 } else { 0.0 }).max(0.0);
        let line = measure.line_height(style);
        let label = Rect::new(rect.x + 8.0 + label_x, rect.y + (ENTRY - line) / 2.0, label_width, line);
        nodes.push(Node::RichText { rect: label, spans: entry.label.clone(), style, color: theme.text, max_lines: 1 });
        if entry.submenu {
            let arrow = row(label_x + label_width + 8.0, 18.0);
            nodes.push(Node::Label { rect: arrow, text: "›".into(), style: TextStyle::new(18.0), color: theme.muted, align: Align::Left });
        }
        // A disabled entry is faded, as QML's `opacity: 0.45`.
        let fade = if entry.enabled { 1.0 } else { 0.45 };
        scene.nodes.extend(nodes.into_iter().map(|node| node.faded(fade)));
        let visible = crate::panel::intersect(rect, list);
        if entry.enabled && visible.h > 0.0 {
            scene.target(visible, target);
        }
        y += ENTRY + SPACING;
    }
    scene.push(Node::Clip(None));
    (scene, max_scroll)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Color, markup::mnemonic, scene::tests::Mono, theme::Palette};

    fn theme() -> Theme {
        let c = |v| Color::rgba(v, v, v, 255);
        Theme::new(Palette { background: c(10), surface: c(20), text: c(200), muted: c(150), accent: c(90), border: c(60) }, None)
    }

    fn entry(label: &str) -> EntryView {
        EntryView { label: mnemonic(label), enabled: true, ..EntryView::default() }
    }

    #[test]
    fn symbolic_icons_become_glyphs() {
        assert_eq!(symbolic_glyph("application-exit-symbolic", "Quit"), Some("󰍃"));
        assert_eq!(symbolic_glyph("emblem-system-symbolic", "Einstellungen"), Some("󰒓"));
        assert_eq!(symbolic_glyph("x-symbolic", "Something"), Some("•"));
        assert_eq!(symbolic_glyph("firefox", "Quit"), None);
    }

    #[test]
    fn entries_stack_with_separators_and_only_enabled_ones_react() {
        let model = TrayMenuModel {
            output_width: 1280.0,
            title: "Steam".into(),
            entries: vec![
                entry("_Library"),
                EntryView { separator: true, ..EntryView::default() },
                EntryView { enabled: false, ..entry("Store") },
                EntryView { submenu: true, toggle: Toggle::Check, checked: true, ..entry("Friends") },
            ],
            can_go_back: false,
            scroll: 0.0,
            hover: None,
        };
        let (scene, max_scroll) = scene(&model, &theme(), &mut Mono);
        assert_eq!(scene.width, 320.0);
        assert_eq!(max_scroll, 0.0);
        // Title (20), 4, then 34 + 2 + 9 + 2 + 34 + 2 + 34, and 12 around.
        assert_eq!(scene.height, 12.0 + 20.0 + 4.0 + 34.0 + 2.0 + 9.0 + 2.0 + 34.0 + 2.0 + 34.0 + 12.0);
        let first = scene.target_rect(&MenuTarget::Entry(0)).unwrap();
        assert_eq!(first, Rect::new(12.0, 12.0 + 20.0 + 4.0, 296.0, 34.0));
        assert!(scene.target_rect(&MenuTarget::Entry(2)).is_none(), "disabled entries do not react");
        assert_eq!(scene.target_rect(&MenuTarget::Entry(3)).unwrap().y, first.y + 34.0 + 2.0 + 9.0 + 2.0 + 34.0 + 2.0);
        let texts: Vec<_> = scene.nodes.iter().filter_map(|n| match n { Node::Label { text, .. } => Some(text.as_str()), _ => None }).collect();
        assert!(texts.contains(&"✓") && texts.contains(&"›"), "{texts:?}");
        assert!(scene.target_rect(&MenuTarget::Back).is_none());
    }

    #[test]
    fn long_menus_scroll_and_submenus_lead_back() {
        let model = TrayMenuModel {
            output_width: 1280.0,
            title: "Lang".into(),
            entries: (0..20).map(|i| entry(&format!("Eintrag {i}"))).collect(),
            can_go_back: true,
            scroll: 1e6,
            hover: None,
        };
        let (scene, max_scroll) = scene(&model, &theme(), &mut Mono);
        assert_eq!(max_scroll, 20.0 * 34.0 + 19.0 * 2.0 - 440.0);
        // The back button makes the header 26 high: 26 + 4 + 440 + 24, below the 500 the window may take.
        assert_eq!(scene.height, 494.0);
        assert!(scene.target_rect(&MenuTarget::Back).is_some());
        assert!(scene.target_rect(&MenuTarget::Entry(19)).is_some() && scene.target_rect(&MenuTarget::Entry(0)).is_none());
    }
}
