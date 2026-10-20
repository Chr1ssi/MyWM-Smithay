//! The panels the bar's buttons open (the details panel of `bar.qml`): a glass card below the bar, at the right
//! edge for audio, command center and notifications, in the middle for media and calendar, with a title and
//! the panel's content in an area that scrolls when the monitor is too low for it.
use crate::{
    Align, Measure, Node, Rect, Scene, TextStyle, Theme,
    bar::Panel,
    i18n::Language,
    media::{self, MediaTarget, PlayerView},
    notifications::{self, CardTarget, NotificationView},
    theme::{BAR_HEIGHT, PANEL_RADIUS},
};

/// Distance from the top of the monitor (below the bar), and of right-hand panels from its right edge.
pub const TOP: f32 = BAR_HEIGHT + 6.0;
pub const RIGHT: f32 = 8.0;
pub const MARGIN: f32 = 20.0;
/// How far the wheel moves a list per notch.
pub const SCROLL_STEP: f32 = 60.0;

/// What a click in a panel means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelTarget {
    /// The panel itself: nothing.
    Background,
    /// The notification center's list (the wheel scrolls it rather than the panel).
    List,
    ClearAll,
    Card(CardTarget),
    Media(MediaTarget),
}

/// A panel's content, laid out at the panel's inner width from its own top left corner.
#[derive(Default)]
pub struct Content {
    pub height: f32,
    pub nodes: Vec<Node>,
    pub targets: Vec<(Rect, PanelTarget)>,
}

impl Content {
    pub fn push(&mut self, node: Node) {
        self.nodes.push(node);
    }

    pub fn target(&mut self, rect: Rect, target: PanelTarget) {
        self.targets.push((rect, target));
    }

    /// The nodes of a scene built for this content (helpers that draw into a `Scene`).
    pub fn extend<T>(&mut self, scene: Scene<T>) {
        self.nodes.extend(scene.nodes);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ContentModel {
    /// Newest first, with how far the list is scrolled.
    Notifications { items: Vec<NotificationView>, list_scroll: f32 },
    Media { players: Vec<PlayerView> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PanelModel {
    pub kind: Panel,
    /// The monitor's size.
    pub output: (f32, f32),
    pub content: ContentModel,
    /// How far the panel's area is scrolled.
    pub scroll: f32,
    pub hover: Option<PanelTarget>,
    pub language: Language,
}

/// A panel's picture (the panel at (0, 0)) and how far its area and its list can scroll.
pub struct PanelScene {
    pub scene: Scene<PanelTarget>,
    pub max_scroll: f32,
    pub max_list_scroll: f32,
}

/// The panel's width on a monitor `output_width` wide.
pub fn width(output_width: f32) -> f32 {
    (output_width - 24.0).min(400.0)
}

/// Where a panel of `kind`, `width` wide, sits on a monitor `output_width` wide (relative to the monitor).
pub fn origin(kind: Panel, output_width: f32, width: f32) -> (f32, f32) {
    match kind {
        Panel::Audio | Panel::Control | Panel::Notifications => (output_width - RIGHT - width, TOP),
        // A layer surface anchored only at the top: in the middle, on whole pixels.
        Panel::Media | Panel::Calendar => (((output_width - width) / 2.0).floor(), TOP),
    }
}

fn title(kind: Panel, language: Language) -> &'static str {
    match kind {
        Panel::Audio => "Audio",
        Panel::Control => language.tr("Commandcenter"),
        Panel::Media => language.tr("Medien"),
        Panel::Calendar => language.tr("Kalender"),
        Panel::Notifications => language.tr("Benachrichtigungen"),
    }
}

pub fn scene(model: &PanelModel, theme: &Theme, measure: &mut impl Measure) -> PanelScene {
    let width = width(model.output.0);
    let inner = width - 2.0 * MARGIN;
    let (content, max_list_scroll) = match &model.content {
        ContentModel::Notifications { items, list_scroll } => {
            notifications::center(items, *list_scroll, model.hover.as_ref(), theme, measure, inner, model.language)
        }
        ContentModel::Media { players } => (media::content(players, model.hover.as_ref(), theme, measure, inner, model.language), 0.0),
    };
    let title_style = TextStyle::new(20.0);
    let title_height = measure.line_height(title_style);
    let height = (content.height + 82.0).max(150.0).min(model.output.1 - BAR_HEIGHT - 18.0);
    let mut scene = Scene::new(width, height);
    let panel = Rect::new(0.0, 0.0, width, height);
    scene.push(Node::Rect { rect: panel, radius: PANEL_RADIUS, fill: theme.glass(), border: Some((1.0, theme.glass_border())) });
    scene.target(panel, PanelTarget::Background);
    let title_rect = Rect::new(MARGIN, MARGIN, inner, title_height);
    let text = title(model.kind, model.language).to_owned();
    scene.push(Node::Label { rect: title_rect, text, style: title_style, color: theme.text, align: Align::Left });

    // The area below the title holds the content, cut off at the panel's height; it scrolls if it must.
    let area = Rect::new(MARGIN, MARGIN + title_height + 14.0, inner, (height - 82.0).max(0.0));
    let max_scroll = (content.height - area.h).max(0.0);
    let offset = area.y - model.scroll.clamp(0.0, max_scroll);
    scene.push(Node::Clip(Some(area)));
    for node in content.nodes {
        // Content clips (a list's) stay within the area.
        let node = match node.translated(area.x, offset) {
            Node::Clip(Some(rect)) => Node::Clip(Some(intersect(rect, area))),
            Node::Clip(None) => Node::Clip(Some(area)),
            node => node,
        };
        scene.push(node);
    }
    scene.push(Node::Clip(None));
    for (rect, target) in content.targets {
        let rect = intersect(rect.translated(area.x, offset), area);
        if rect.w > 0.0 && rect.h > 0.0 {
            scene.target(rect, target);
        }
    }
    PanelScene { scene, max_scroll, max_list_scroll }
}

/// The part of `a` inside `b` (empty when they do not meet).
pub fn intersect(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.x.max(b.x), a.y.max(b.y));
    let (right, bottom) = (a.right().min(b.right()), a.bottom().min(b.bottom()));
    Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Color, scene::tests::Mono, theme::Palette};

    fn theme() -> Theme {
        let c = |v| Color::rgba(v, v, v, 255);
        Theme::new(Palette { background: c(10), surface: c(20), text: c(200), muted: c(150), accent: c(90), border: c(60) }, None)
    }

    #[test]
    fn panels_sit_below_the_bar_right_or_in_the_middle() {
        assert_eq!(width(1280.0), 400.0);
        assert_eq!(width(300.0), 276.0);
        assert_eq!(origin(Panel::Audio, 1280.0, 400.0), (872.0, 46.0));
        assert_eq!(origin(Panel::Media, 1280.0, 400.0), (440.0, 46.0));
        assert_eq!(origin(Panel::Calendar, 1281.0, 400.0), (440.0, 46.0));
    }

    #[test]
    fn a_low_monitor_makes_the_area_scroll() {
        let players = (0..6).map(|i| PlayerView { title: format!("Titel {i}"), can_next: true, ..PlayerView::default() }).collect();
        let model = PanelModel {
            kind: Panel::Media,
            output: (1280.0, 300.0),
            content: ContentModel::Media { players },
            scroll: 0.0,
            hover: None,
            language: Language::English,
        };
        let panel = scene(&model, &theme(), &mut Mono);
        assert_eq!(panel.scene.height, 300.0 - 40.0 - 18.0);
        assert!(panel.max_scroll > 0.0);
        let scrolled = scene(&PanelModel { scroll: 1e6, ..model }, &theme(), &mut Mono);
        // Scrolled to the end, the first player's buttons are out of reach, the last one's within it.
        let reachable = |s: &PanelScene, i| s.scene.targets.iter().any(|(_, t)| *t == PanelTarget::Media(MediaTarget::Next(i)));
        assert!(reachable(&panel, 0) && !reachable(&scrolled, 0));
        assert!(reachable(&scrolled, 5));
        // The title stays where it is.
        let title = |s: &PanelScene| s.scene.nodes.iter().find_map(|n| match n { Node::Label { rect, text, .. } if text == "Media" => Some(*rect), _ => None });
        assert_eq!(title(&panel), title(&scrolled));
    }
}
