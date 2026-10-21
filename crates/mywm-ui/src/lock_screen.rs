//! The lock screen (as mywm-shell's `lock.qml` with `LoginScreen.qml`): on the primary monitor the clock and the
//! date at 16 % of its height and the card with avatar, name, password field and message at 52 %, all centered.
//! The background (the theme's image, cut to the monitor, dimmed) is the compositor's: it is the same on every
//! monitor and needs no painting on the CPU.
use crate::{
    Align, Measure, Node, Rect, Scene, TextStyle, Theme,
    i18n::Language,
};

const CLOCK: TextStyle = TextStyle::new(112.0).light(true);
const DATE: TextStyle = TextStyle::new(22.0);
const AVATAR: TextStyle = TextStyle::new(32.0).bold(true);
const NAME: TextStyle = TextStyle::new(20.0);
const FIELD_TEXT: TextStyle = TextStyle::new(16.0);
const SUBMIT: TextStyle = TextStyle::new(18.0).icons(true);
const MESSAGE: TextStyle = TextStyle::new(14.0);
const CARD_WIDTH: f32 = 400.0;
const CARD_PADDING: f32 = 28.0;
const CARD_RADIUS: f32 = 28.0;
const SPACING: f32 = 16.0;
const AVATAR_SIZE: f32 = 72.0;
const FIELD_HEIGHT: f32 = 48.0;
const FIELD_LEFT: f32 = 20.0;
/// Room for the submit button at the field's right end.
const FIELD_RIGHT: f32 = 56.0;
const SUBMIT_SIZE: f32 = 36.0;
const SUBMIT_MARGIN: f32 = 6.0;
/// The black over the background, so text and glass stay readable on any wallpaper.
pub const DIM: f32 = 0.35;

/// What the lock screen says after a password was not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockMessage {
    WrongPassword,
    TooManyTries,
}

impl LockMessage {
    pub fn text(self, language: Language) -> &'static str {
        match self {
            Self::WrongPassword => language.tr("Falsches Passwort"),
            Self::TooManyTries => language.tr("Zu viele Versuche"),
        }
    }
}

/// What the primary monitor's lock screen shows. Of the password only its length is known here.
#[derive(Clone, Debug, PartialEq)]
pub struct LockModel {
    /// The monitor's logical size.
    pub output: (f32, f32),
    pub time: String,
    pub date: String,
    pub user: String,
    pub password_len: usize,
    /// A check runs: the field takes no keys and the button shows an hourglass.
    pub busy: bool,
    pub message: Option<LockMessage>,
    pub submit_hovered: bool,
    pub language: Language,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockTarget {
    Card,
    Field,
    Submit,
}

/// The clock, the date and the card, and where the scene's origin lies on the monitor (whole pixels). The scene
/// is only as wide and high as they need: the background lies behind it.
pub fn scene(model: &LockModel, theme: &Theme, measure: &mut impl Measure) -> (Scene<LockTarget>, (f32, f32)) {
    let (w, h) = model.output;
    let card_w = CARD_WIDTH.min(w - 48.0).max(0.0);
    let clock_w = measure.width(&model.time, CLOCK).ceil();
    let date_w = measure.width(&model.date, DATE).ceil();
    // A pixel more on each side for the card's smoothed edge.
    let width = (card_w.max(clock_w).max(date_w) + 2.0).min(w);
    let origin = (((w - width) / 2.0).floor(), (h * 0.16).floor());
    // Everything is centered on the monitor, which the scene's whole pixels may miss by half of one.
    let left = (w - width) / 2.0 - origin.0;
    let top = |y: f32| y - origin.1;

    let clock_h = measure.line_height(CLOCK);
    let date_h = measure.line_height(DATE);
    let name_h = measure.line_height(NAME);
    let message_h = measure.line_height(MESSAGE);
    let inner = card_w - 2.0 * CARD_PADDING;
    let mut column = AVATAR_SIZE + SPACING + name_h + SPACING + FIELD_HEIGHT;
    if model.message.is_some() {
        column += SPACING + message_h;
    }
    let card = Rect::new((w - card_w) / 2.0 - origin.0, top(h * 0.52), card_w, column + 2.0 * CARD_PADDING);
    let mut scene = Scene::new(width, card.y + card.h + 1.0);

    let y = top(h * 0.16);
    scene.push(Node::Label { rect: Rect::new(left, y, width, clock_h), text: model.time.clone(), style: CLOCK, color: theme.text, align: Align::Center });
    let y = y + clock_h + 4.0;
    let date = Node::Label { rect: Rect::new(left, y, width, date_h), text: model.date.clone(), style: DATE, color: theme.text, align: Align::Center };
    scene.push(date.faded(0.8));

    scene.push(Node::Rect { rect: card, radius: CARD_RADIUS, fill: theme.glass(), border: Some((1.0, theme.glass_border())) });
    scene.target(card, LockTarget::Card);
    let x = card.x + CARD_PADDING;
    let mut y = card.y + CARD_PADDING;

    let avatar = Rect::new(card.x + (card_w - AVATAR_SIZE) / 2.0, y, AVATAR_SIZE, AVATAR_SIZE);
    scene.push(Node::Rect { rect: avatar, radius: AVATAR_SIZE / 2.0, fill: theme.accent, border: None });
    let (initial, style) = match model.user.chars().next() {
        Some(first) => (first.to_uppercase().collect(), AVATAR),
        None => ("󰀄".to_owned(), AVATAR.icons(true)),
    };
    scene.push(Node::Label { rect: avatar, text: initial, style, color: theme.background, align: Align::Center });
    y += AVATAR_SIZE + SPACING;

    scene.push(Node::Label { rect: Rect::new(x, y, inner, name_h), text: model.user.clone(), style: NAME, color: theme.text, align: Align::Center });
    y += name_h + SPACING;

    // The field has the keyboard (a 2 px accent border) unless a check runs; a refused password makes it red.
    let field = Rect::new(x, y, inner, FIELD_HEIGHT);
    let border = match (model.message.is_some(), model.busy) {
        (true, _) => (2.0, theme.alert),
        (false, true) => (1.0, theme.border),
        (false, false) => (2.0, theme.accent),
    };
    scene.push(Node::Rect { rect: field, radius: FIELD_HEIGHT / 2.0, fill: theme.glass_surface(), border: Some(border) });
    scene.target(field, LockTarget::Field);
    let text_area = Rect::new(field.x + FIELD_LEFT, field.y, (field.w - FIELD_LEFT - FIELD_RIGHT).max(0.0), field.h);
    let dots = "●".repeat(model.password_len);
    let dots_w = measure.width(&dots, FIELD_TEXT);
    // Like a `TextField`, a long password scrolls so that its end (the cursor) stays in view.
    let scrolled = (dots_w - text_area.w + 2.0).max(0.0);
    scene.push(Node::Clip(Some(text_area)));
    if model.password_len == 0 {
        let placeholder = model.language.tr("Passwort").into();
        scene.push(Node::Label { rect: text_area, text: placeholder, style: FIELD_TEXT, color: theme.muted, align: Align::Left });
    } else {
        // Wide enough not to be cut off with "…": the clip cuts it.
        let line = Rect::new(text_area.x - scrolled, field.y, dots_w.ceil() + 2.0, field.h);
        scene.push(Node::Label { rect: line, text: dots, style: FIELD_TEXT, color: theme.text, align: Align::Left });
    }
    if !model.busy {
        let line_h = measure.line_height(FIELD_TEXT);
        let cursor = Rect::new((text_area.x + dots_w - scrolled).round(), field.y + ((field.h - line_h) / 2.0).round(), 2.0, line_h);
        scene.push(Node::Rect { rect: cursor, radius: 0.0, fill: theme.text, border: None });
    }
    scene.push(Node::Clip(None));

    let submit = Rect::new(field.x + field.w - SUBMIT_MARGIN - SUBMIT_SIZE, field.y + (field.h - SUBMIT_SIZE) / 2.0, SUBMIT_SIZE, SUBMIT_SIZE);
    let (fill, icon_color) = if model.submit_hovered { (theme.accent, theme.background) } else { (theme.surface, theme.accent) };
    scene.push(Node::Rect { rect: submit, radius: SUBMIT_SIZE / 2.0, fill, border: None });
    let icon = if model.busy { "󰔟" } else { "󰁔" };
    scene.push(Node::Label { rect: submit, text: icon.into(), style: SUBMIT, color: icon_color, align: Align::Center });
    scene.target(submit, LockTarget::Submit);
    y += FIELD_HEIGHT;

    if let Some(message) = model.message {
        y += SPACING;
        let rect = Rect::new(x, y, inner, message_h);
        scene.push(Node::Label { rect, text: message.text(model.language).into(), style: MESSAGE, color: theme.alert, align: Align::Center });
    }
    (scene, origin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Color, scene::tests::Mono, theme::Palette};

    fn theme() -> Theme {
        let c = |v| Color::rgba(v, v, v, 255);
        Theme::new(Palette { background: c(1), surface: c(2), text: c(3), muted: c(4), accent: c(5), border: c(6) }, None)
    }

    fn model() -> LockModel {
        LockModel {
            output: (1280.0, 800.0),
            time: "09:05".into(),
            date: "Dienstag, 20. Oktober".into(),
            user: "chris".into(),
            password_len: 0,
            busy: false,
            message: None,
            submit_hovered: false,
            language: Language::German,
        }
    }

    fn on_monitor(scene: &Scene<LockTarget>, origin: (f32, f32), target: LockTarget) -> Rect {
        scene.target_rect(&target).expect("target").translated(origin.0, origin.1)
    }

    #[test]
    fn the_card_sits_where_qt_puts_it() {
        let (shown, origin) = scene(&model(), &theme(), &mut Mono);
        assert_eq!(origin.1, 128.0);
        let card = on_monitor(&shown, origin, LockTarget::Card);
        assert_eq!((card.x, card.y, card.w), (440.0, 416.0, 400.0));
        // Padding, avatar, name (20 px at 1.32), field, padding.
        assert_eq!(card.h, 28.0 + 72.0 + 16.0 + (20.0f32 * 1.32).ceil() + 16.0 + 48.0 + 28.0);
        let field = on_monitor(&shown, origin, LockTarget::Field);
        assert_eq!((field.x, field.w, field.h), (468.0, 344.0, 48.0));
        let submit = on_monitor(&shown, origin, LockTarget::Submit);
        assert_eq!((submit.x, submit.y, submit.w), (field.x + 344.0 - 42.0, field.y + 6.0, 36.0));
        // A message makes the card taller; a narrow monitor makes it narrower.
        let failed = LockModel { message: Some(LockMessage::WrongPassword), ..model() };
        let (taller, origin) = scene(&failed, &theme(), &mut Mono);
        assert!(on_monitor(&taller, origin, LockTarget::Card).h > card.h);
        let narrow = LockModel { output: (300.0, 800.0), ..model() };
        let (narrow, origin) = scene(&narrow, &theme(), &mut Mono);
        assert_eq!(on_monitor(&narrow, origin, LockTarget::Card).w, 252.0);
    }

    #[test]
    fn the_field_shows_dots_and_the_hint_only_when_empty() {
        let labels = |model: &LockModel| -> Vec<String> {
            scene(model, &theme(), &mut Mono).0.nodes.into_iter().filter_map(|node| match node {
                Node::Label { text, .. } => Some(text),
                _ => None,
            }).collect()
        };
        assert!(labels(&model()).contains(&"Passwort".to_owned()));
        let typed = labels(&LockModel { password_len: 3, ..model() });
        assert!(typed.contains(&"●●●".to_owned()) && !typed.contains(&"Passwort".to_owned()));
        assert!(labels(&LockModel { user: String::new(), ..model() }).contains(&"󰀄".to_owned()));
        assert!(labels(&model()).contains(&"C".to_owned()));
        assert!(labels(&LockModel { busy: true, ..model() }).contains(&"󰔟".to_owned()));
    }
}
