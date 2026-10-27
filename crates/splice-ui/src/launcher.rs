//! The application launcher (`shell.qml`): a glass card in the middle of the monitor with a search field, the
//! applications whose texts contain every word typed (those whose name starts with the search first), and a
//! footer with the count and the keys.
use crate::{
    Align, Measure, Node, Rect, Scene, TextStyle, Theme,
    edit::Input,
    i18n::Language,
    scene::Picture,
    theme::{PANEL_PADDING, PANEL_RADIUS},
};

/// What the launcher searches in, of an application.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Searchable {
    pub id: String,
    pub name: String,
    pub generic_name: String,
    pub comment: String,
    pub keywords: Vec<String>,
    /// It has a command (entries without one are not listed).
    pub launchable: bool,
}

/// The applications that match `query`, as indices into `apps`, in the launcher's order: every word of the
/// query appears in the name, generic name, comment, id or keywords (ignoring case); those whose name starts with
/// the whole query come first, then by name.
pub fn results(apps: &[Searchable], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let terms: Vec<&str> = query.split_whitespace().collect();
    let mut found: Vec<usize> = apps
        .iter()
        .enumerate()
        .filter(|(_, app)| app.launchable)
        .filter(|(_, app)| {
            let words = [&app.name, &app.generic_name, &app.comment, &app.id].into_iter().chain(&app.keywords);
            let haystack = words.map(String::as_str).collect::<Vec<_>>().join(" ").to_lowercase();
            terms.iter().all(|term| haystack.contains(term))
        })
        .map(|(index, _)| index)
        .collect();
    // JavaScript's `localeCompare` needs ICU; ignoring case first comes close to it for names.
    let key = |index: &usize| {
        let name = &apps[*index].name;
        (!name.to_lowercase().starts_with(&query), name.to_lowercase(), name.clone())
    };
    found.sort_by_cached_key(key);
    found
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppView {
    pub name: String,
    /// The generic name, else the comment, else the id.
    pub detail: String,
    pub icon: Option<Picture>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LauncherModel {
    /// The monitor's size.
    pub output: (f32, f32),
    pub query: Input,
    /// The matching applications, in order.
    pub apps: Vec<AppView>,
    pub selected: usize,
    /// How far the list is scrolled.
    pub scroll: f32,
    pub hover: Option<LauncherTarget>,
    pub language: Language,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LauncherTarget {
    /// The card itself: nothing.
    Background,
    Search,
    /// The list (the wheel scrolls it).
    List,
    App(usize),
}

const SPACING: f32 = 12.0;
const SEARCH_HEIGHT: f32 = 48.0;
const ROW_HEIGHT: f32 = 52.0;
const ROW_SPACING: f32 = 2.0;
const SEARCH_TEXT: TextStyle = TextStyle::new(18.0);

/// The card's size on a monitor `output` large.
pub fn size(output: (f32, f32)) -> (f32, f32) {
    ((output.0 - 32.0).min(640.0), (output.1 - 32.0).min(560.0))
}

/// Where the card sits on the monitor: in the middle, on whole pixels.
pub fn origin(output: (f32, f32)) -> (f32, f32) {
    let (width, height) = size(output);
    (((output.0 - width) / 2.0).floor(), ((output.1 - height) / 2.0).floor())
}

fn list_height(card_height: f32) -> f32 {
    (card_height - 148.0).max(40.0)
}

/// How far the list can scroll with `count` rows on a card `card_height` high.
pub fn max_scroll(count: usize, card_height: f32) -> f32 {
    let content = count as f32 * (ROW_HEIGHT + ROW_SPACING) - if count > 0 { ROW_SPACING } else { 0.0 };
    (content - list_height(card_height)).max(0.0)
}

/// `scroll` changed as little as possible so that row `selected` is in view (`positionViewAtIndex(…, Contain)`).
pub fn scroll_to(scroll: f32, selected: usize, count: usize, card_height: f32) -> f32 {
    let top = selected as f32 * (ROW_HEIGHT + ROW_SPACING);
    let bottom = top + ROW_HEIGHT;
    let height = list_height(card_height);
    let scroll = if top < scroll { top } else if bottom > scroll + height { bottom - height } else { scroll };
    scroll.clamp(0.0, max_scroll(count, card_height))
}

/// Where the field's text starts and how far it is scrolled to the left (to keep the cursor in view).
fn search_text(measure: &mut impl Measure, query: &Input, field_width: f32) -> (f32, f32) {
    let cursor_x = measure.width(&query.text[..query.cursor], SEARCH_TEXT);
    (20.0, (cursor_x - (field_width - 40.0) + 2.0).max(0.0))
}

/// The cursor position nearest to `x` in the search field (`x` from the field's left edge).
pub fn search_cursor_at(measure: &mut impl Measure, query: &Input, output: (f32, f32), x: f32) -> usize {
    let (start, scrolled) = search_text(measure, query, size(output).0 - 2.0 * PANEL_PADDING);
    query.nearest(measure, SEARCH_TEXT, x - start + scrolled)
}

/// The card, with its origin at (0, 0).
pub fn scene(model: &LauncherModel, theme: &Theme, measure: &mut impl Measure) -> Scene<LauncherTarget> {
    let tr = |text| model.language.tr(text);
    let (width, height) = size(model.output);
    let mut scene = Scene::new(width, height);
    let card = Rect::new(0.0, 0.0, width, height);
    scene.push(Node::Rect { rect: card, radius: PANEL_RADIUS, fill: theme.glass(), border: Some((1.0, theme.glass_border())) });
    scene.target(card, LauncherTarget::Background);
    let inner = width - 2.0 * PANEL_PADDING;
    let x = PANEL_PADDING;
    let mut y = PANEL_PADDING;

    let header = TextStyle::new(14.0).bold(true);
    let header_h = measure.line_height(header);
    scene.push(Node::Label { rect: Rect::new(x, y, inner, header_h), text: tr("ANWENDUNGEN").into(), style: header, color: theme.text, align: Align::Left });
    y += header_h + SPACING;

    // The search field, which always has the keyboard.
    let field = Rect::new(x, y, inner, SEARCH_HEIGHT);
    scene.push(Node::Rect { rect: field, radius: SEARCH_HEIGHT / 2.0, fill: theme.glass_surface(), border: Some((1.0, theme.border)) });
    let (start, scrolled) = search_text(measure, &model.query, field.w);
    let text_area = Rect::new(field.x + start, field.y, field.w - 2.0 * start, field.h);
    let cursor_x = measure.width(&model.query.text[..model.query.cursor], SEARCH_TEXT);
    scene.push(Node::Clip(Some(text_area)));
    if model.query.text.is_empty() {
        let placeholder = tr("App suchen …").into();
        scene.push(Node::Label { rect: text_area, text: placeholder, style: SEARCH_TEXT, color: theme.muted, align: Align::Left });
    } else {
        // Wide enough not to be cut off with "…": the clip cuts it.
        let line = Rect::new(text_area.x - scrolled, field.y, measure.width(&model.query.text, SEARCH_TEXT).ceil() + 2.0, field.h);
        scene.push(Node::Label { rect: line, text: model.query.text.clone(), style: SEARCH_TEXT, color: theme.text, align: Align::Left });
    }
    let line_h = measure.line_height(SEARCH_TEXT);
    let cursor = Rect::new((text_area.x + cursor_x - scrolled).round(), field.y + ((field.h - line_h) / 2.0).round(), 2.0, line_h);
    scene.push(Node::Rect { rect: cursor, radius: 0.0, fill: theme.text, border: None });
    scene.push(Node::Clip(None));
    scene.target(field, LauncherTarget::Search);
    y += SEARCH_HEIGHT + SPACING;

    // The list.
    let list = Rect::new(x, y, inner, list_height(height));
    scene.target(list, LauncherTarget::List);
    scene.push(Node::Clip(Some(list)));
    let (name_style, detail_style) = (TextStyle::new(16.0), TextStyle::new(14.0));
    let (name_h, detail_h) = (measure.line_height(name_style), measure.line_height(detail_style));
    let scroll = model.scroll.clamp(0.0, max_scroll(model.apps.len(), height));
    for (index, app) in model.apps.iter().enumerate() {
        let row = Rect::new(x, list.y + index as f32 * (ROW_HEIGHT + ROW_SPACING) - scroll, inner, ROW_HEIGHT);
        if row.bottom() <= list.y || row.y >= list.bottom() {
            continue;
        }
        let target = LauncherTarget::App(index);
        if index == model.selected || model.hover == Some(target) {
            scene.push(Node::Rect { rect: row, radius: 14.0, fill: theme.surface, border: None });
        }
        if let Some(picture) = &app.icon {
            scene.push(Node::Image { rect: Rect::new(row.x + 12.0, row.y + (ROW_HEIGHT - 32.0) / 2.0, 32.0, 32.0), picture: picture.clone() });
        }
        let column_w = (inner - 70.0).max(0.0);
        let top = row.y + (ROW_HEIGHT - name_h - 3.0 - detail_h) / 2.0;
        scene.push(Node::Label { rect: Rect::new(row.x + 56.0, top, column_w, name_h), text: app.name.clone(), style: name_style, color: theme.text, align: Align::Left });
        let detail = Rect::new(row.x + 56.0, top + name_h + 3.0, column_w, detail_h);
        scene.push(Node::Label { rect: detail, text: app.detail.clone(), style: detail_style, color: theme.muted, align: Align::Left });
        let reachable = crate::panel::intersect(row, list);
        scene.target(reachable, target);
    }
    if model.apps.is_empty() {
        let text = tr("Keine passenden Anwendungen").into();
        scene.push(Node::Label { rect: list, text, style: TextStyle::new(18.0), color: theme.muted, align: Align::Center });
    }
    scene.push(Node::Clip(None));
    y += list.h + SPACING;

    let footer = TextStyle::new(14.0);
    let text = model.language.trf("%1 Apps  ·  ↑↓ Auswahl  ·  ↵ Start  ·  Esc", &[&model.apps.len().to_string()]);
    scene.push(Node::Label { rect: Rect::new(x, y, inner, measure.line_height(footer)), text, style: footer, color: theme.muted, align: Align::Left });
    scene
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Color, scene::tests::Mono, theme::Palette};

    fn theme() -> Theme {
        let c = |v| Color::rgba(v, v, v, 255);
        Theme::new(Palette { background: c(10), surface: c(20), text: c(200), muted: c(150), accent: c(90), border: c(60) }, None)
    }

    fn app(id: &str, name: &str, generic: &str, keywords: &[&str]) -> Searchable {
        Searchable {
            id: id.into(),
            name: name.into(),
            generic_name: generic.into(),
            keywords: keywords.iter().map(|k| (*k).to_owned()).collect(),
            launchable: true,
            ..Searchable::default()
        }
    }

    #[test]
    fn every_word_must_match_and_the_name_start_comes_first() {
        let apps = vec![
            app("probe-browser", "Probe Browser", "Web Browser", &["internet", "web"]),
            app("probe-terminal", "Probe Terminal", "", &[]),
            app("webcam", "Webcam", "", &[]),
            Searchable { launchable: false, ..app("broken", "Web Broken", "", &[]) },
        ];
        assert_eq!(results(&apps, ""), [0, 1, 2], "all launchable ones, by name");
        assert_eq!(results(&apps, "  INTERNET web "), [0]);
        assert_eq!(results(&apps, "web"), [2, 0], "the name starting with the search first");
        assert_eq!(results(&apps, "probe term"), [1]);
        assert!(results(&apps, "no-such-application").is_empty());
        assert_eq!(results(&apps, "terminal"), [1], "the id counts too");
    }

    #[test]
    fn names_sort_ignoring_case() {
        let apps = vec![app("b", "beta", "", &[]), app("a", "Alpha", "", &[]), app("c", "Gamma", "", &[])];
        assert_eq!(results(&apps, ""), [1, 0, 2]);
    }

    #[test]
    fn the_card_has_the_qml_measures() {
        assert_eq!(size((1280.0, 800.0)), (640.0, 560.0));
        assert_eq!(origin((1280.0, 800.0)), (320.0, 120.0));
        assert_eq!(size((500.0, 400.0)), (468.0, 368.0));
        let model = LauncherModel {
            output: (1280.0, 800.0),
            query: Input::new("web"),
            apps: vec![AppView { name: "Probe Browser".into(), detail: "Web Browser".into(), icon: None }],
            selected: 0,
            scroll: 0.0,
            hover: None,
            language: Language::German,
        };
        let scene = scene(&model, &theme(), &mut Mono);
        // Header 19 high (Mono's 14 px line), then 12, the field 48, 12: the list starts at 107.
        assert_eq!(scene.target_rect(&LauncherTarget::Search), Some(Rect::new(16.0, 47.0, 608.0, 48.0)));
        assert_eq!(scene.target_rect(&LauncherTarget::App(0)), Some(Rect::new(16.0, 107.0, 608.0, 52.0)));
        let texts: Vec<&str> = scene.nodes.iter().filter_map(|n| match n { Node::Label { text, .. } => Some(text.as_str()), _ => None }).collect();
        assert_eq!(texts, ["ANWENDUNGEN", "web", "Probe Browser", "Web Browser", "1 Apps  ·  ↑↓ Auswahl  ·  ↵ Start  ·  Esc"]);
        assert!(scene.nodes.iter().any(|n| matches!(n, Node::Rect { fill, .. } if *fill == theme().surface)), "the selected row");

        let empty = super::scene(&LauncherModel { apps: Vec::new(), query: Input::default(), ..model }, &theme(), &mut Mono);
        let texts: Vec<&str> = empty.nodes.iter().filter_map(|n| match n { Node::Label { text, .. } => Some(text.as_str()), _ => None }).collect();
        assert!(texts.contains(&"App suchen …") && texts.contains(&"Keine passenden Anwendungen"), "{texts:?}");
    }

    #[test]
    fn the_selection_stays_in_view() {
        // 412 pixels of list: rows 54 apart, so seven and a half show.
        let height = 560.0;
        assert_eq!(max_scroll(5, height), 0.0);
        assert_eq!(max_scroll(20, height), 20.0 * 54.0 - 2.0 - 412.0);
        assert_eq!(scroll_to(0.0, 3, 20, height), 0.0);
        assert_eq!(scroll_to(0.0, 8, 20, height), 8.0 * 54.0 + 52.0 - 412.0);
        assert_eq!(scroll_to(300.0, 2, 20, height), 108.0);
        assert_eq!(scroll_to(0.0, 19, 20, height), max_scroll(20, height));
    }
}
