//! The bar at the top of every monitor (`bar.qml`): rounded islands with the workspaces on the left, media
//! and clock in the middle, recording, status and power on the right.
use crate::{
    Align, Color, Measure, Node, Rect, Scene, TextStyle, Theme,
    i18n::Language,
    theme::{BAR_HEIGHT, BAR_MARGIN, ISLAND_HEIGHT},
};

/// Height of a button (`BarButton.qml`).
pub const BUTTON_HEIGHT: f32 = 26.0;
const BUTTON_TEXT: TextStyle = TextStyle::new(14.0);
/// Space between the buttons of an island.
const SPACING: f32 = 2.0;
/// Space between islands.
const ISLAND_SPACING: f32 = 8.0;
/// Distance of the outer islands from the monitor's edges.
const EDGE: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Workspace {
    /// 0 is the gaming workspace.
    pub number: u32,
    pub occupied: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Audio {
    /// 0.0 to 1.0.
    pub volume: f32,
    pub muted: bool,
}

/// The panels the bar's buttons open; their button looks selected while one is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Media,
    Calendar,
    Control,
    Notifications,
    Audio,
}

/// What the bar of one monitor shows.
#[derive(Clone, Debug, PartialEq)]
pub struct BarModel {
    /// The monitor's width in logical pixels.
    pub width: f32,
    pub workspaces: Vec<Workspace>,
    pub active: Option<u32>,
    pub scratchpad_visible: bool,
    pub scratchpad_occupied: bool,
    /// The title of the playing media player; no media island without one.
    pub media: Option<String>,
    pub clock: String,
    pub capturing: bool,
    /// Bluetooth and night light, as icons.
    pub control_icon: String,
    pub notifications: usize,
    /// The default output's volume; `None` without one.
    pub audio: Option<Audio>,
    pub open_panel: Option<Panel>,
    pub power_open: bool,
    pub hover: Option<BarTarget>,
    pub language: Language,
}

/// What a click on the bar does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarTarget {
    /// Between the islands: nothing, but the click does not reach a window either.
    Background,
    Workspace(u32),
    NewWorkspace,
    Scratchpad,
    Panel(Panel),
    Power,
}

/// A `BarButton` before it is placed.
struct Button {
    text: String,
    width: f32,
    icons: bool,
    selected: bool,
    marked: bool,
    target: BarTarget,
}

/// What an island holds.
enum Content {
    Buttons(Vec<Button>),
    /// A plain text with eight pixels of padding on either side (the recording indicator).
    Text { text: String, color: Color },
}

impl Button {
    /// A button as wide as its text plus 16 pixels (`implicitWidth`).
    fn fit(measure: &mut impl Measure, text: String, icons: bool, target: BarTarget) -> Self {
        let width = measure.width(&text, BUTTON_TEXT.icons(icons)).ceil() + 16.0;
        Self { text, width, icons, selected: false, marked: false, target }
    }

    fn fixed(text: String, width: f32, target: BarTarget) -> Self {
        Self { text, width, icons: false, selected: false, marked: false, target }
    }

    fn selected(self, selected: bool) -> Self {
        Self { selected, ..self }
    }

    fn marked(self, marked: bool) -> Self {
        Self { marked, ..self }
    }
}

impl Content {
    fn width(&self, measure: &mut impl Measure) -> f32 {
        let inner = match self {
            Content::Buttons(buttons) => {
                buttons.iter().map(|b| b.width).sum::<f32>() + SPACING * buttons.len().saturating_sub(1) as f32
            }
            Content::Text { text, .. } => measure.width(text, BUTTON_TEXT.bold(true)).ceil() + 16.0,
        };
        inner + 8.0
    }
}

/// The bar's picture and click targets for `model`.
pub fn scene(model: &BarModel, theme: &Theme, measure: &mut impl Measure) -> Scene<BarTarget> {
    let mut scene = Scene::new(model.width, BAR_HEIGHT);
    scene.target(Rect::new(0.0, 0.0, model.width, BAR_HEIGHT), BarTarget::Background);
    let tr = |text| model.language.tr(text);

    let mut workspaces: Vec<Button> = model
        .workspaces
        .iter()
        .map(|w| {
            let text = if w.number == 0 { "G".to_owned() } else { w.number.to_string() };
            Button::fixed(text, 26.0, BarTarget::Workspace(w.number))
                .selected(model.active == Some(w.number))
                .marked(w.occupied)
        })
        .collect();
    workspaces.push(Button::fixed("+".into(), 26.0, BarTarget::NewWorkspace));
    if model.scratchpad_occupied {
        workspaces.push(
            Button::fixed("S".into(), 26.0, BarTarget::Scratchpad).selected(model.scratchpad_visible).marked(true),
        );
    }

    let mut middle = Vec::new();
    if let Some(title) = &model.media {
        let button = Button::fixed(format!("󰎆 {title}"), (model.width * 0.24).min(260.0), BarTarget::Panel(Panel::Media));
        middle.push(Content::Buttons(vec![button.selected(model.open_panel == Some(Panel::Media))]));
    }
    let clock = Button::fit(measure, model.clock.clone(), false, BarTarget::Panel(Panel::Calendar));
    middle.push(Content::Buttons(vec![clock.selected(model.open_panel == Some(Panel::Calendar))]));

    let mut right = Vec::new();
    if model.capturing {
        right.push(Content::Text { text: format!("󰑊 {}", tr("Aufnahme")), color: theme.alert });
    }
    let panel = |panel: Panel| BarTarget::Panel(panel);
    let notifications = if model.notifications > 0 { format!("󰂚 {}", model.notifications) } else { "󰂜".into() };
    let audio = match model.audio {
        None => "󰕾 —".to_owned(),
        Some(Audio { muted: true, .. }) => format!("󰖁 {}", tr("Stumm")),
        Some(Audio { volume, .. }) => format!("󰕾 {} %", (volume * 100.0).round()),
    };
    let open = |p: Panel| model.open_panel == Some(p);
    right.push(Content::Buttons(vec![
        Button::fit(measure, model.control_icon.clone(), true, panel(Panel::Control)).selected(open(Panel::Control)),
        Button::fit(measure, notifications, false, panel(Panel::Notifications)).selected(open(Panel::Notifications)),
        Button::fit(measure, audio, false, panel(Panel::Audio)).selected(open(Panel::Audio)),
    ]));
    right.push(Content::Buttons(vec![Button::fit(measure, "⏻".into(), true, BarTarget::Power).selected(model.power_open)]));

    let row_width = |islands: &[Content], measure: &mut _| {
        islands.iter().map(|i| i.width(measure)).sum::<f32>() + ISLAND_SPACING * islands.len().saturating_sub(1) as f32
    };
    island(&mut scene, model, theme, EDGE, Content::Buttons(workspaces), measure);
    let mut x = (model.width - row_width(&middle, measure)) / 2.0;
    for content in middle {
        x += island(&mut scene, model, theme, x, content, measure) + ISLAND_SPACING;
    }
    let mut x = model.width - EDGE - row_width(&right, measure);
    for content in right {
        x += island(&mut scene, model, theme, x, content, measure) + ISLAND_SPACING;
    }
    scene
}

/// Draw an island at `x` with its content (`BarIsland.qml`) and return its width.
fn island(scene: &mut Scene<BarTarget>, model: &BarModel, theme: &Theme, x: f32, content: Content, measure: &mut impl Measure) -> f32 {
    let width = content.width(measure);
    let rect = Rect::new(x, BAR_MARGIN, width, ISLAND_HEIGHT);
    scene.push(Node::Rect { rect, radius: ISLAND_HEIGHT / 2.0, fill: theme.glass(), border: Some((1.0, theme.glass_border())) });
    let top = rect.y + (ISLAND_HEIGHT - BUTTON_HEIGHT) / 2.0;
    match content {
        Content::Text { text, color } => {
            let rect = Rect::new(x + 4.0 + 8.0, top, width - 8.0 - 16.0, BUTTON_HEIGHT);
            scene.push(Node::Label { rect, text, style: BUTTON_TEXT.bold(true), color, align: Align::Left });
        }
        Content::Buttons(buttons) => {
            let mut left = x + 4.0;
            for button in buttons {
                let rect = Rect::new(left, top, button.width, BUTTON_HEIGHT);
                draw_button(scene, theme, rect, &button, model.hover == Some(button.target));
                scene.target(rect, button.target);
                left += button.width + SPACING;
            }
        }
    }
    width
}

fn draw_button(scene: &mut Scene<BarTarget>, theme: &Theme, rect: Rect, button: &Button, hovered: bool) {
    button_shape(scene, theme, rect, &button.text, button.icons, button.selected, button.marked, hovered);
}

/// A `BarButton` in `rect`: a pill that lights up when selected or hovered, its text in the middle and,
/// for something occupied that is not shown, a small mark at the bottom.
#[allow(clippy::too_many_arguments)]
pub fn button_shape<T: Clone>(
    scene: &mut Scene<T>,
    theme: &Theme,
    rect: Rect,
    text: &str,
    icons: bool,
    selected: bool,
    marked: bool,
    hovered: bool,
) {
    if selected || hovered {
        scene.push(Node::Rect { rect, radius: rect.h / 2.0, fill: theme.surface, border: None });
    }
    let label = Rect::new(rect.x + 6.0, rect.y, (rect.w - 12.0).max(0.0), rect.h);
    let color = if selected { theme.accent } else { theme.text };
    let style = BUTTON_TEXT.icons(icons).bold(marked || selected);
    scene.push(Node::Label { rect: label, text: text.to_owned(), style, color, align: Align::Center });
    if marked && !selected {
        let mark = Rect::new(rect.x + (rect.w - 4.0) / 2.0, rect.bottom() - 2.0 - 3.0, 4.0, 3.0);
        scene.push(Node::Rect { rect: mark, radius: 1.0, fill: theme.accent, border: None });
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

    fn model() -> BarModel {
        BarModel {
            width: 1280.0,
            workspaces: vec![Workspace { number: 1, occupied: true }, Workspace { number: 2, occupied: true }],
            active: Some(1),
            scratchpad_visible: false,
            scratchpad_occupied: false,
            media: None,
            clock: "Di. 20. Okt.  19:15".into(),
            capturing: false,
            control_icon: "󰒓".into(),
            notifications: 0,
            audio: None,
            open_panel: None,
            power_open: false,
            hover: None,
            language: Language::English,
        }
    }

    fn islands(scene: &Scene<BarTarget>) -> Vec<Rect> {
        scene.nodes.iter().filter_map(|n| match n { Node::Rect { rect, border: Some(_), .. } => Some(*rect), _ => None }).collect()
    }

    fn label(scene: &Scene<BarTarget>, target: BarTarget) -> String {
        let rect = scene.target_rect(&target).expect("target");
        scene
            .nodes
            .iter()
            .find_map(|n| match n { Node::Label { rect: r, text, .. } if rect.contains(r.x, r.y + 1.0) => Some(text.clone()), _ => None })
            .expect("label")
    }

    #[test]
    fn islands_sit_like_in_the_quickshell_bar() {
        let scene = scene(&model(), &theme(), &mut Mono);
        let islands = islands(&scene);
        // Workspaces: two buttons and "+", 26 wide with 2 between, 4 of padding each side.
        assert_eq!(islands[0], Rect::new(8.0, 6.0, 3.0 * 26.0 + 2.0 * 2.0 + 8.0, 32.0));
        // The clock in the middle: 19 characters of 7 pixels, 16 of padding in the button, 8 in the island.
        let clock = 19.0 * 7.0 + 16.0 + 8.0;
        assert_eq!(islands[1], Rect::new((1280.0 - clock) / 2.0, 6.0, clock, 32.0));
        // Power at the right edge, status before it.
        let power = islands.last().unwrap();
        assert_eq!(power.right(), 1280.0 - 8.0);
        assert_eq!(islands[islands.len() - 2].right(), power.x - 8.0);
        assert_eq!(scene.target_at(8.0 + 4.0 + 26.0 + 2.0 + 1.0, 20.0), Some(BarTarget::Workspace(2)));
        assert_eq!(scene.target_at(8.0 + 4.0 + 26.0 + 1.0, 20.0), Some(BarTarget::Background));
        assert_eq!(scene.target_at(power.x + 10.0, 20.0), Some(BarTarget::Power));
    }

    #[test]
    fn optional_islands_come_and_go() {
        let plain = islands(&scene(&model(), &theme(), &mut Mono)).len();
        let busy = BarModel { media: Some("Song".into()), capturing: true, scratchpad_occupied: true, ..model() };
        let scene = scene(&busy, &theme(), &mut Mono);
        assert_eq!(islands(&scene).len(), plain + 2);
        assert!(scene.target_rect(&BarTarget::Scratchpad).is_some());
        assert_eq!(label(&scene, BarTarget::Panel(Panel::Media)), "󰎆 Song");
        assert!(scene.nodes.iter().any(|n| matches!(n, Node::Label { text, .. } if text == "󰑊 Recording")));
    }

    #[test]
    fn status_texts_follow_the_model() {
        let texts = |model: BarModel| {
            let scene = scene(&model, &theme(), &mut Mono);
            (label(&scene, BarTarget::Panel(Panel::Notifications)), label(&scene, BarTarget::Panel(Panel::Audio)))
        };
        assert_eq!(texts(model()), ("󰂜".into(), "󰕾 —".into()));
        let audio = Some(Audio { volume: 0.8, muted: false });
        assert_eq!(texts(BarModel { notifications: 3, audio, ..model() }), ("󰂚 3".into(), "󰕾 80 %".into()));
        let audio = Some(Audio { volume: 0.8, muted: true });
        assert_eq!(texts(BarModel { audio, language: Language::German, ..model() }).1, "󰖁 Stumm");
    }

    #[test]
    fn the_active_workspace_is_selected_and_occupied_ones_are_marked() {
        let scene = scene(&BarModel { active: Some(2), ..model() }, &theme(), &mut Mono);
        let first = scene.target_rect(&BarTarget::Workspace(1)).unwrap();
        let second = scene.target_rect(&BarTarget::Workspace(2)).unwrap();
        let mark = |r: Rect| scene.nodes.iter().any(|n| matches!(n, Node::Rect { rect, .. } if rect.w == 4.0 && r.contains(rect.x, rect.y)));
        let lit = |r: Rect| scene.nodes.iter().any(|n| matches!(n, Node::Rect { rect, .. } if *rect == r));
        assert!(mark(first) && !lit(first));
        assert!(!mark(second) && lit(second));
    }
}
