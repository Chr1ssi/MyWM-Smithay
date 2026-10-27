//! The lock screen and the login screen (as splice-shell's `lock.qml` and `greeter.qml` with `LoginScreen.qml`): on
//! the primary monitor the clock and the date at 16 % of its height and the card with avatar, name (or a field for
//! it), password field, button and message at 52 %, all centered; the login screen adds the sessions to choose from
//! at the bottom left and restart and shut down at the bottom right. The background (an image cut to the monitor,
//! dimmed) is the compositor's: it is the same on every monitor and needs no painting on the CPU.
use crate::{
    Align, Measure, Node, Rect, Scene, TextStyle, Theme,
    bar::{BUTTON_HEIGHT, BUTTON_TEXT, button_shape},
    i18n::Language,
    theme::ISLAND_HEIGHT,
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
/// Room for the submit button at the password field's right end.
const FIELD_RIGHT: f32 = 56.0;
const SUBMIT_SIZE: f32 = 36.0;
const SUBMIT_MARGIN: f32 = 6.0;
/// The islands' distance from the monitor's edges.
const ISLAND_MARGIN: f32 = 24.0;
const BUTTON_SPACING: f32 = 2.0;
const REBOOT: &str = "󰜉";
const POWEROFF: &str = "⏻";
/// The black over the background, so text and glass stay readable on any wallpaper.
pub const DIM: f32 = 0.35;

/// What the card says under the fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockMessage {
    WrongPassword,
    TooManyTries,
    /// greetd could not be asked (its description).
    CannotSignIn(String),
    /// The password was accepted; the session starts.
    Starting,
    /// What PAM said through greetd.
    Said(String),
}

impl LockMessage {
    pub fn text(&self, language: Language) -> String {
        match self {
            Self::WrongPassword => language.tr("Falsches Passwort").to_owned(),
            Self::TooManyTries => language.tr("Zu viele Versuche").to_owned(),
            Self::CannotSignIn(description) => language.trf("Anmeldung nicht möglich: %1", &[description]),
            Self::Starting => language.tr("Sitzung startet …").to_owned(),
            Self::Said(text) => text.clone(),
        }
    }
}

/// What the primary monitor's lock or login screen shows. Of the password only its length is known here.
#[derive(Clone, Debug, PartialEq)]
pub struct LockModel {
    /// The monitor's logical size.
    pub output: (f32, f32),
    pub time: String,
    pub date: String,
    /// The user's name; on the login screen what was typed into its field.
    pub user: String,
    /// The login screen asks for the user: a field (with the cursor at this byte) instead of the name.
    pub ask_user: Option<usize>,
    pub password_len: usize,
    /// A check runs: the fields take no keys and the button shows an hourglass.
    pub busy: bool,
    pub message: Option<LockMessage>,
    /// The message is an error: red, and so is the password field's border.
    pub failed: bool,
    /// What has the keyboard: a field or, on the login screen, a button.
    pub focus: LockTarget,
    pub hover: Option<LockTarget>,
    /// The login screen's sessions (shown when there is a choice) and the one chosen.
    pub sessions: Vec<String>,
    pub session: usize,
    /// The login screen's restart and shut down.
    pub show_power: bool,
    pub language: Language,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LockTarget {
    Card,
    /// The login screen's user field.
    User,
    /// The password field.
    #[default]
    Password,
    Submit,
    Session(usize),
    Reboot,
    Poweroff,
}

/// What Tab moves through, in order (`activeFocusOnTab` of the fields and the `BarButton`s).
pub fn focus_chain(ask_user: bool, sessions: usize, show_power: bool) -> Vec<LockTarget> {
    let mut chain = Vec::new();
    if ask_user {
        chain.push(LockTarget::User);
    }
    chain.push(LockTarget::Password);
    if sessions > 1 {
        chain.extend((0..sessions).map(LockTarget::Session));
    }
    if show_power {
        chain.extend([LockTarget::Reboot, LockTarget::Poweroff]);
    }
    chain
}

/// A text field's text with its cursor: the placeholder when empty, scrolled so that the cursor stays in view.
#[allow(clippy::too_many_arguments)]
fn field_text(
    scene: &mut Scene<LockTarget>,
    measure: &mut impl Measure,
    theme: &Theme,
    area: Rect,
    text: &str,
    cursor: usize,
    placeholder: &str,
    focused: bool,
) {
    let cursor_x = measure.width(&text[..cursor], FIELD_TEXT);
    let scrolled = (cursor_x - area.w + 2.0).max(0.0);
    scene.push(Node::Clip(Some(area)));
    if text.is_empty() {
        scene.push(Node::Label { rect: area, text: placeholder.into(), style: FIELD_TEXT, color: theme.muted, align: Align::Left });
    } else {
        // Wide enough not to be cut off with "…": the clip cuts it.
        let line = Rect::new(area.x - scrolled, area.y, measure.width(text, FIELD_TEXT).ceil() + 2.0, area.h);
        scene.push(Node::Label { rect: line, text: text.into(), style: FIELD_TEXT, color: theme.text, align: Align::Left });
    }
    if focused {
        let line_h = measure.line_height(FIELD_TEXT);
        let cursor = Rect::new((area.x + cursor_x - scrolled).round(), area.y + ((area.h - line_h) / 2.0).round(), 2.0, line_h);
        scene.push(Node::Rect { rect: cursor, radius: 0.0, fill: theme.text, border: None });
    }
    scene.push(Node::Clip(None));
}

/// The clock, the date and the card, and where the scene's origin lies on the monitor (whole pixels). The scene
/// is only as wide and high as they need: the background lies behind it.
fn card_scene(model: &LockModel, theme: &Theme, measure: &mut impl Measure) -> (Scene<LockTarget>, (f32, f32)) {
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
    let inner = card_w - 2.0 * CARD_PADDING;
    let message = model.message.as_ref().map(|m| m.text(model.language));
    let message_h = message.as_ref().map(|text| measure.paragraph_height(text, MESSAGE, inner));
    let name_row = if model.ask_user.is_some() { FIELD_HEIGHT } else { name_h };
    let mut column = AVATAR_SIZE + SPACING + name_row + SPACING + FIELD_HEIGHT;
    if let Some(message_h) = message_h {
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

    // A field has the keyboard (a 2 px border) unless a check runs, which disables both.
    let focused = |target| !model.busy && model.focus == target;
    match model.ask_user {
        Some(cursor) => {
            let field = Rect::new(x, y, inner, FIELD_HEIGHT);
            let border = if focused(LockTarget::User) { (2.0, theme.accent) } else { (1.0, theme.border) };
            scene.push(Node::Rect { rect: field, radius: FIELD_HEIGHT / 2.0, fill: theme.glass_surface(), border: Some(border) });
            scene.target(field, LockTarget::User);
            let area = Rect::new(field.x + FIELD_LEFT, field.y, (field.w - 2.0 * FIELD_LEFT).max(0.0), field.h);
            let placeholder = model.language.tr("Benutzername");
            field_text(&mut scene, measure, theme, area, &model.user, cursor, placeholder, focused(LockTarget::User));
            y += FIELD_HEIGHT + SPACING;
        }
        None => {
            scene.push(Node::Label { rect: Rect::new(x, y, inner, name_h), text: model.user.clone(), style: NAME, color: theme.text, align: Align::Center });
            y += name_h + SPACING;
        }
    }

    // A refused password makes the password field's border red.
    let field = Rect::new(x, y, inner, FIELD_HEIGHT);
    let has_focus = focused(LockTarget::Password);
    let color = match (model.failed, has_focus) {
        (true, _) => theme.alert,
        (false, true) => theme.accent,
        (false, false) => theme.border,
    };
    let border = (if has_focus { 2.0 } else { 1.0 }, color);
    scene.push(Node::Rect { rect: field, radius: FIELD_HEIGHT / 2.0, fill: theme.glass_surface(), border: Some(border) });
    scene.target(field, LockTarget::Password);
    let area = Rect::new(field.x + FIELD_LEFT, field.y, (field.w - FIELD_LEFT - FIELD_RIGHT).max(0.0), field.h);
    let dots = "●".repeat(model.password_len);
    field_text(&mut scene, measure, theme, area, &dots, dots.len(), model.language.tr("Passwort"), has_focus);

    let submit = Rect::new(field.x + field.w - SUBMIT_MARGIN - SUBMIT_SIZE, field.y + (field.h - SUBMIT_SIZE) / 2.0, SUBMIT_SIZE, SUBMIT_SIZE);
    let hovered = model.hover == Some(LockTarget::Submit);
    let (fill, icon_color) = if hovered { (theme.accent, theme.background) } else { (theme.surface, theme.accent) };
    scene.push(Node::Rect { rect: submit, radius: SUBMIT_SIZE / 2.0, fill, border: None });
    let icon = if model.busy { "󰔟" } else { "󰁔" };
    scene.push(Node::Label { rect: submit, text: icon.into(), style: SUBMIT, color: icon_color, align: Align::Center });
    scene.target(submit, LockTarget::Submit);
    y += FIELD_HEIGHT;

    if let (Some(text), Some(message_h)) = (message, message_h) {
        y += SPACING;
        let color = if model.failed { theme.alert } else { theme.muted };
        scene.push(Node::Paragraph { rect: Rect::new(x, y, inner, message_h), text, style: MESSAGE, color, align: Align::Center });
    }
    (scene, origin)
}

/// A row of `BarButton`s on a glass island (`x` is its left edge, or its right edge if `from_right`).
fn island(
    model: &LockModel,
    theme: &Theme,
    measure: &mut impl Measure,
    buttons: &[(String, bool, bool, LockTarget)],
    x: f32,
    from_right: bool,
) -> (Scene<LockTarget>, (f32, f32)) {
    // Unrounded, like the `Row` of `BarButton`s (the bar's own rounds them; there the islands' widths match).
    let widths: Vec<f32> =
        buttons.iter().map(|(text, icons, selected, _)| measure.width(text, BUTTON_TEXT.icons(*icons).bold(*selected)) + 16.0).collect();
    let row = widths.iter().sum::<f32>() + BUTTON_SPACING * (buttons.len().saturating_sub(1)) as f32;
    let width = row + 8.0;
    let left = if from_right { x - width } else { x };
    let origin = (left.floor(), (model.output.1 - ISLAND_MARGIN - ISLAND_HEIGHT).floor());
    // Where Qt puts it, which the scene's whole pixels may miss by a fraction of one.
    let shift = left - origin.0;
    let mut scene = Scene::new((shift + width).ceil(), ISLAND_HEIGHT);
    let rect = Rect::new(shift, 0.0, width, ISLAND_HEIGHT);
    scene.push(Node::Rect { rect, radius: ISLAND_HEIGHT / 2.0, fill: theme.glass(), border: Some((1.0, theme.glass_border())) });
    let mut x = shift + (width - row) / 2.0;
    for ((text, icons, selected, target), width) in buttons.iter().zip(widths) {
        let rect = Rect::new(x, (ISLAND_HEIGHT - BUTTON_HEIGHT) / 2.0, width, BUTTON_HEIGHT);
        let lit = model.hover == Some(*target) || model.focus == *target;
        button_shape(&mut scene, theme, rect, text, *icons, *selected, false, lit);
        scene.target(rect, *target);
        x += width + BUTTON_SPACING;
    }
    (scene, origin)
}

/// What the primary monitor shows in front of the background: the card's scene and, on the login screen, the
/// islands, each with where its origin lies on the monitor.
pub fn scenes(model: &LockModel, theme: &Theme, measure: &mut impl Measure) -> Vec<(Scene<LockTarget>, (f32, f32))> {
    let mut scenes = vec![card_scene(model, theme, measure)];
    if model.sessions.len() > 1 {
        let buttons: Vec<_> =
            model.sessions.iter().enumerate().map(|(i, name)| (name.clone(), false, i == model.session, LockTarget::Session(i))).collect();
        scenes.push(island(model, theme, measure, &buttons, ISLAND_MARGIN, false));
    }
    if model.show_power {
        let buttons = [(REBOOT.to_owned(), true, false, LockTarget::Reboot), (POWEROFF.to_owned(), true, false, LockTarget::Poweroff)];
        scenes.push(island(model, theme, measure, &buttons, model.output.0 - ISLAND_MARGIN, true));
    }
    scenes
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
            ask_user: None,
            password_len: 0,
            busy: false,
            message: None,
            failed: false,
            focus: LockTarget::Password,
            hover: None,
            sessions: Vec::new(),
            session: 0,
            show_power: false,
            language: Language::German,
        }
    }

    fn greeter() -> LockModel {
        LockModel { ask_user: Some(0), user: String::new(), focus: LockTarget::User, sessions: vec!["splice".into(), "Andere".into()], show_power: true, ..model() }
    }

    /// Where `target` lies on the monitor, in whichever scene has it.
    fn on_monitor(model: &LockModel, target: LockTarget) -> Rect {
        scenes(model, &theme(), &mut Mono)
            .iter()
            .find_map(|(scene, origin)| Some(scene.target_rect(&target)?.translated(origin.0, origin.1)))
            .expect("target")
    }

    fn labels(model: &LockModel) -> Vec<String> {
        scenes(model, &theme(), &mut Mono)
            .into_iter()
            .flat_map(|(scene, _)| scene.nodes)
            .filter_map(|node| match node {
                Node::Label { text, .. } | Node::Paragraph { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_card_sits_where_qt_puts_it() {
        let (shown, origin) = card_scene(&model(), &theme(), &mut Mono);
        assert_eq!(origin.1, 128.0);
        let card = shown.target_rect(&LockTarget::Card).expect("card").translated(origin.0, origin.1);
        assert_eq!((card.x, card.y, card.w), (440.0, 416.0, 400.0));
        // Padding, avatar, name (20 px at 1.32), field, padding.
        assert_eq!(card.h, 28.0 + 72.0 + 16.0 + (20.0f32 * 1.32).ceil() + 16.0 + 48.0 + 28.0);
        let field = on_monitor(&model(), LockTarget::Password);
        assert_eq!((field.x, field.w, field.h), (468.0, 344.0, 48.0));
        let submit = on_monitor(&model(), LockTarget::Submit);
        assert_eq!((submit.x, submit.y, submit.w), (field.x + 344.0 - 42.0, field.y + 6.0, 36.0));
        // A message makes the card taller; a narrow monitor makes it narrower.
        let failed = LockModel { message: Some(LockMessage::WrongPassword), failed: true, ..model() };
        assert!(on_monitor(&failed, LockTarget::Card).h > card.h);
        assert_eq!(on_monitor(&LockModel { output: (300.0, 800.0), ..model() }, LockTarget::Card).w, 252.0);
        // The lock screen has no islands.
        assert_eq!(scenes(&model(), &theme(), &mut Mono).len(), 1);
    }

    #[test]
    fn the_field_shows_dots_and_the_hint_only_when_empty() {
        assert!(labels(&model()).contains(&"Passwort".to_owned()));
        let typed = labels(&LockModel { password_len: 3, ..model() });
        assert!(typed.contains(&"●●●".to_owned()) && !typed.contains(&"Passwort".to_owned()));
        assert!(labels(&LockModel { user: String::new(), ..model() }).contains(&"󰀄".to_owned()));
        assert!(labels(&model()).contains(&"C".to_owned()));
        assert!(labels(&LockModel { busy: true, ..model() }).contains(&"󰔟".to_owned()));
    }

    #[test]
    fn the_login_screen_asks_for_the_user_and_offers_sessions_and_power() {
        let shown = labels(&greeter());
        assert!(shown.contains(&"Benutzername".to_owned()) && !shown.contains(&"chris".to_owned()));
        let user = on_monitor(&greeter(), LockTarget::User);
        let password = on_monitor(&greeter(), LockTarget::Password);
        assert_eq!((user.y + 48.0 + 16.0, user.h), (password.y, 48.0));
        // The sessions at the bottom left, restart and shut down at the bottom right, 24 px from the edges.
        let first = on_monitor(&greeter(), LockTarget::Session(0));
        assert_eq!((first.y, first.h), (800.0 - 24.0 - 32.0 + 3.0, 26.0));
        assert!(first.x > 24.0 && first.x < 32.0);
        let off = on_monitor(&greeter(), LockTarget::Poweroff);
        assert!(off.right() < 1280.0 - 24.0 && off.right() > 1280.0 - 32.0);
        // One session is no choice.
        let single = LockModel { sessions: vec!["splice".into()], ..greeter() };
        assert_eq!(scenes(&single, &theme(), &mut Mono).len(), 2);
        assert_eq!(focus_chain(true, 2, true), [
            LockTarget::User,
            LockTarget::Password,
            LockTarget::Session(0),
            LockTarget::Session(1),
            LockTarget::Reboot,
            LockTarget::Poweroff
        ]);
        assert_eq!(focus_chain(false, 1, false), [LockTarget::Password]);
    }

    #[test]
    fn messages_read_like_greeter_qml() {
        let message = |m: LockMessage, language| m.text(language);
        assert_eq!(message(LockMessage::CannotSignIn("x".into()), Language::English), "Cannot sign in: x");
        assert_eq!(message(LockMessage::Starting, Language::German), "Sitzung startet …");
        let failed = LockModel { message: Some(LockMessage::Said("Konto gesperrt".into())), ..greeter() };
        assert!(labels(&failed).contains(&"Konto gesperrt".to_owned()));
    }
}
