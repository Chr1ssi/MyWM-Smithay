//! The lock screen and the login screen (as splice-shell's `lock.qml` and `greeter.qml` with `LoginScreen.qml`): on
//! the primary monitor the clock and the date at 16 % of its height and the card with avatar, name (or a field for
//! it), password field, button and message at 52 %, all centered; the login screen adds the sessions to choose from
//! at the bottom left and restart and shut down at the bottom right. The background (an image cut to the monitor,
//! dimmed) is the compositor's: it is the same on every monitor and needs no painting on the CPU.
use crate::{
    Align, Measure, Node, Rect, Scene, Shape, TextStyle, Theme,
    bar::{BUTTON_HEIGHT, BUTTON_TEXT, button_shape_tinted},
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
/// The cyberdeck's console lines: the card's tag, and the texts beside the login card, whose size and width grow with
/// the monitor (from 1080 pixels of height, up to 1.5 times) so that they hold their own on large screens.
const TAG: TextStyle = TextStyle::new(12.0);
const SIDE: TextStyle = TextStyle::new(13.0);
const SIDE_WIDTH: f32 = 330.0;
const SIDE_GAP: f32 = 32.0;
/// The status column keeps more room from the card: its text starts at the card's side, the code's ends ragged.
const SIDE_GAP_RIGHT: f32 = 72.0;
/// How dark the cyberdeck's scanlines are (of 255), and how many rows lie between two of them.
const SCANLINE_ALPHA: u8 = 46;
const SCANLINE_STEP: usize = 3;
/// What the login screen's left column shows: the flow of a login, as decoration.
const LOGIN_FLOW: [&str; 21] = [
    "// splice::greeter",
    "use greetd::{Request, Response};",
    "",
    "fn run() -> Result<()> {",
    "  let s = greetd::connect()?;",
    "  s.create_session(user)?;",
    "  loop {",
    "    match s.read()? {",
    "      Response::AuthMessage(_) => {",
    "        s.post_auth(pw.take())?;",
    "      }",
    "      Response::Success => {",
    "        return s.start(session);",
    "      }",
    "      Response::Error(e) => {",
    "        delay(2s);",
    "        retry(e);",
    "      }",
    "    }",
    "  }",
    "}",
];

/// The cyberdeck's scanlines for a screen `width` × `height` pixels large: every third row a faint black line
/// (premultiplied pixels, byte order R, G, B, A).
pub fn scanline_pixels(width: usize, height: usize) -> Vec<u8> {
    let mut pixels = vec![0u8; width * height * 4];
    for row in (0..height).step_by(SCANLINE_STEP) {
        for pixel in pixels[row * width * 4..(row + 1) * width * 4].as_chunks_mut::<4>().0 {
            pixel[3] = SCANLINE_ALPHA;
        }
    }
    pixels
}

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
    let deck = theme.shape == Shape::Chamfer;
    let date = if deck { model.date.to_uppercase() } else { model.date.clone() };
    let date_w = measure.width(&date, DATE).ceil();
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
    // The cyberdeck's card starts with a tag line.
    let tag_h = if deck { measure.line_height(TAG) + SPACING } else { 0.0 };
    column += tag_h;
    // The cyberdeck's card sits higher, with room for a gap below the date at any monitor height.
    let card_top = if deck { (h * 0.42).max(h * 0.16 + clock_h + 4.0 + date_h + 48.0).floor() } else { h * 0.52 };
    let card = Rect::new((w - card_w) / 2.0 - origin.0, top(card_top), card_w, column + 2.0 * CARD_PADDING);
    let mut scene = Scene::new(width, card.y + card.h + 1.0);

    let y = top(h * 0.16);
    scene.push(Node::Label { rect: Rect::new(left, y, width, clock_h), text: model.time.clone(), style: CLOCK, color: theme.text, align: Align::Center });
    let y = y + clock_h + 4.0;
    let date_color = if deck { theme.secondary } else { theme.text };
    let date = Node::Label { rect: Rect::new(left, y, width, date_h), text: date, style: DATE, color: date_color, align: Align::Center };
    scene.push(date.faded(0.8));

    scene.push(Node::Rect { rect: card, radius: CARD_RADIUS, fill: theme.glass(), border: Some((1.0, theme.glass_border())) });
    scene.target(card, LockTarget::Card);
    let x = card.x + CARD_PADDING;
    let mut y = card.y + CARD_PADDING;
    if deck {
        let (kind, size) = (if model.show_power { "SPLICE // LOGIN" } else { "SPLICE // LOCK" }, format!("{}×{}", model.output.0 as i32, model.output.1 as i32));
        let height = measure.line_height(TAG);
        scene.push(Node::Label { rect: Rect::new(x, y, inner, height), text: kind.into(), style: TAG, color: theme.accent, align: Align::Left });
        let size_w = measure.width(&size, TAG).ceil();
        scene.push(Node::Label { rect: Rect::new(x + inner - size_w, y, size_w, height), text: size, style: TAG, color: theme.muted, align: Align::Left });
        y += tag_h;
    }

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
        // The cyberdeck sets the restart and shut down buttons in the alert color, like the bar's.
        let tint = (theme.shape == Shape::Chamfer && matches!(target, LockTarget::Reboot | LockTarget::Poweroff)).then_some(theme.alert);
        button_shape_tinted(&mut scene, theme, rect, text, tint, *icons, *selected, false, lit);
        scene.target(rect, *target);
        x += width + BUTTON_SPACING;
    }
    (scene, origin)
}

/// What a stretch of the login flow's code is, for its color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Code {
    Plain,
    Keyword,
    Type,
    Comment,
}

/// `line` cut into runs of one kind each (their texts joined give `line` again): keywords, capitalized names
/// (types and variants), and a trailing `//` comment, as far as a few lines of decoration need.
fn code_runs(line: &str) -> Vec<(Code, String)> {
    const KEYWORDS: [&str; 6] = ["use", "fn", "let", "loop", "match", "return"];
    let mut runs: Vec<(Code, String)> = Vec::new();
    let mut push = |kind: Code, text: &str| match runs.last_mut() {
        Some((last, run)) if *last == kind => run.push_str(text),
        _ => runs.push((kind, text.to_owned())),
    };
    let mut rest = line;
    while !rest.is_empty() {
        if rest.starts_with("//") {
            push(Code::Comment, rest);
            break;
        }
        let first = rest.chars().next().unwrap_or(' ');
        if first.is_alphabetic() || first == '_' {
            let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
            let (word, tail) = rest.split_at(end);
            let kind = if KEYWORDS.contains(&word) {
                Code::Keyword
            } else if first.is_uppercase() {
                Code::Type
            } else {
                Code::Plain
            };
            push(kind, word);
            rest = tail;
        } else {
            let len = first.len_utf8();
            push(Code::Plain, &rest[..len]);
            rest = &rest[len..];
        }
    }
    runs
}

/// How much larger than at 1080 pixels of height the side texts are on a monitor `h` high.
fn side_scale(h: f32) -> f32 {
    (h / 1080.0).clamp(1.0, 1.5)
}

/// The cyberdeck's login screen: beside the card the flow of a login on the left and what the screen knows on the
/// right, as console text, both centered on the card's height. Only where the monitor is wide enough for both.
fn side_scenes(model: &LockModel, theme: &Theme, measure: &mut impl Measure, card_middle: f32) -> Vec<(Scene<LockTarget>, (f32, f32))> {
    let (w, h) = model.output;
    let card_w = CARD_WIDTH.min(w - 48.0).max(0.0);
    let k = side_scale(h);
    let side_w = (SIDE_WIDTH * k).floor();
    if theme.shape != Shape::Chamfer || !model.show_power || w < CARD_WIDTH + 2.0 * side_w + SIDE_GAP + SIDE_GAP_RIGHT + 2.0 * ISLAND_MARGIN {
        return Vec::new();
    }
    let style = TextStyle::new(SIDE.size * k);
    let line_h = measure.line_height(style);
    let card_x = (w - card_w) / 2.0;
    let place = |height: f32, x: f32| (x, (card_middle - height / 2.0).floor().max(0.0));

    let mut flow = Scene::new(side_w, LOGIN_FLOW.len() as f32 * line_h);
    for (index, line) in LOGIN_FLOW.iter().enumerate() {
        let y = index as f32 * line_h;
        if index == 0 {
            flow.push(Node::Label { rect: Rect::new(0.0, y, side_w, line_h), text: (*line).into(), style, color: theme.accent, align: Align::Left });
            continue;
        }
        // Colored like a code editor does: keywords and types stand out, comments sink.
        let mut shown = String::new();
        for (kind, run) in code_runs(line) {
            let color = match kind {
                Code::Comment => theme.muted,
                Code::Keyword => theme.tertiary,
                Code::Type => theme.secondary,
                Code::Plain => theme.text.faded(0.8),
            };
            let x = measure.width(&shown, style);
            let width = measure.width(&run, style).ceil() + 1.0;
            shown.push_str(&run);
            flow.push(Node::Label { rect: Rect::new(x, y, width, line_h), text: run, style, color, align: Align::Left });
        }
    }

    let session = model.sessions.get(model.session).cloned().unwrap_or_else(|| "—".into());
    let (marker, marker_color, auth, auth_color) = match (model.failed, model.busy) {
        (true, _) => ("[ !! ]", theme.alert, "denied", theme.alert),
        (false, true) => ("[ .. ]", theme.accent, "checking", theme.text),
        (false, false) => ("[ .. ]", theme.muted, "waiting for input", theme.muted),
    };
    let user = if model.user.is_empty() { "—".to_owned() } else { model.user.clone() };
    let rows = [
        ("", theme.accent, "// splice::status", String::new(), theme.text),
        ("[ ok ]", theme.secondary, "output", format!("{}x{}", w as i32, h as i32), theme.text),
        ("[ ok ]", theme.secondary, "user", user, theme.text),
        ("[ ok ]", theme.secondary, "session", session, theme.text),
        (marker, marker_color, "auth", auth.to_owned(), auth_color),
    ];
    let mut status = Scene::new(side_w, rows.len() as f32 * line_h);
    for (index, (marker, marker_color, key, value, value_color)) in rows.into_iter().enumerate() {
        let y = index as f32 * line_h;
        if index == 0 {
            status.push(Node::Label { rect: Rect::new(0.0, y, side_w, line_h), text: key.into(), style, color: theme.accent, align: Align::Left });
            continue;
        }
        status.push(Node::Label { rect: Rect::new(0.0, y, 62.0 * k, line_h), text: marker.into(), style, color: marker_color, align: Align::Left });
        status.push(Node::Label { rect: Rect::new(68.0 * k, y, 80.0 * k, line_h), text: key.into(), style, color: theme.muted, align: Align::Left });
        status.push(Node::Label { rect: Rect::new(154.0 * k, y, side_w - 154.0 * k, line_h), text: value, style, color: value_color, align: Align::Left });
    }
    let (left, right) = ((card_x - SIDE_GAP - side_w).floor(), (card_x + card_w + SIDE_GAP_RIGHT).ceil());
    let (flow_at, status_at) = (place(flow.height, left), place(status.height, right));
    vec![(flow, flow_at), (status, status_at)]
}

/// What the primary monitor shows in front of the background: the card's scene and, on the login screen, the
/// islands (and the cyberdeck's side texts), each with where its origin lies on the monitor.
pub fn scenes(model: &LockModel, theme: &Theme, measure: &mut impl Measure) -> Vec<(Scene<LockTarget>, (f32, f32))> {
    let (card, origin) = card_scene(model, theme, measure);
    // The scene also holds the clock and the date above the card itself.
    let card_middle = card.target_rect(&LockTarget::Card).map_or(origin.1 + card.height / 2.0, |rect| origin.1 + rect.y + rect.h / 2.0);
    let mut scenes = vec![(card, origin)];
    scenes.extend(side_scenes(model, theme, measure, card_middle));
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

    fn deck() -> Theme {
        theme().with_shape(Shape::Chamfer)
    }

    fn deck_labels(model: &LockModel) -> Vec<String> {
        scenes(model, &deck(), &mut Mono)
            .into_iter()
            .flat_map(|(scene, _)| scene.nodes)
            .filter_map(|node| match node {
                Node::Label { text, .. } | Node::Paragraph { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_cyberdeck_card_has_a_tag_line_and_the_date_in_capitals() {
        let plain = scenes(&model(), &theme(), &mut Mono);
        let shown = scenes(&model(), &deck(), &mut Mono);
        let card = |scenes: &[(Scene<LockTarget>, (f32, f32))]| scenes[0].0.target_rect(&LockTarget::Card).unwrap();
        // The tag line (12 px at 1.32, rounded up) and the space after it make the card taller.
        assert_eq!(card(&shown).h, card(&plain).h + (12.0f32 * 1.32).ceil() + 16.0);
        let labels = deck_labels(&model());
        assert!(labels.contains(&"SPLICE // LOCK".to_owned()) && labels.contains(&"1280×800".to_owned()), "{labels:?}");
        assert!(labels.contains(&"DIENSTAG, 20. OKTOBER".to_owned()), "{labels:?}");
        assert!(deck_labels(&greeter()).contains(&"SPLICE // LOGIN".to_owned()));
        // The normal card has neither.
        assert!(!labels_of(&model()).iter().any(|text| text.starts_with("SPLICE")));
    }

    fn labels_of(model: &LockModel) -> Vec<String> {
        labels(model)
    }

    #[test]
    fn the_cyberdeck_login_screen_shows_console_text_beside_the_card() {
        // 1000 px is too narrow for the card and a column on either side; 1920 is wide enough.
        let narrow = LockModel { output: (1000.0, 800.0), ..greeter() };
        assert_eq!(scenes(&narrow, &deck(), &mut Mono).len(), scenes(&narrow, &theme(), &mut Mono).len());
        let wide = LockModel { output: (1920.0, 1080.0), ..greeter() };
        let shown = scenes(&wide, &deck(), &mut Mono);
        assert_eq!(shown.len(), scenes(&wide, &theme(), &mut Mono).len() + 2);
        let (flow, status) = (&shown[1], &shown[2]);
        let card = shown[0].0.target_rect(&LockTarget::Card).unwrap().translated(shown[0].1 .0, shown[0].1 .1);
        // Left and right of the card, the status column further away, both centered on its height.
        assert_eq!(flow.1 .0 + flow.0.width + SIDE_GAP, card.x);
        assert_eq!(status.1 .0, card.right() + SIDE_GAP_RIGHT);
        let middle = card.y + card.h / 2.0;
        assert!((flow.1 .1 + flow.0.height / 2.0 - middle).abs() <= 1.0 && (status.1 .1 + status.0.height / 2.0 - middle).abs() <= 1.0);
        // On a monitor of 1440 pixels the texts are a third larger, and the columns still fit beside the card.
        let large = LockModel { output: (2560.0, 1440.0), ..greeter() };
        let big = scenes(&large, &deck(), &mut Mono);
        let size_of = |scene: &Scene<LockTarget>| match &scene.nodes[0] {
            Node::Label { style, .. } => style.size,
            other => panic!("{other:?}"),
        };
        assert_eq!(size_of(&shown[1].0), SIDE.size);
        assert_eq!(size_of(&big[1].0), SIDE.size * 1440.0 / 1080.0);
        assert!(big[1].0.width > shown[1].0.width && big[1].1 .0 >= 0.0);
        let labels = deck_labels(&wide);
        assert!(labels.contains(&"fn".to_owned()) && labels.contains(&"Result".to_owned()) && labels.contains(&"1920x1080".to_owned()), "{labels:?}");
        assert!(labels.contains(&"waiting for input".to_owned()), "{labels:?}");
        // What the status says follows the screen: a check running, a refused password.
        assert!(deck_labels(&LockModel { busy: true, ..wide.clone() }).contains(&"checking".to_owned()));
        assert!(deck_labels(&LockModel { failed: true, ..wide.clone() }).contains(&"denied".to_owned()));
        // The lock screen has no such columns, whatever the monitor.
        assert_eq!(scenes(&LockModel { output: (1920.0, 1080.0), ..model() }, &deck(), &mut Mono).len(), 1);
    }

    /// The cyberdeck theme with three different accents, as a wallpaper gives them.
    fn colorful() -> Theme {
        let json = r##"{"version":1,"colors":{"primary":{"default":{"hex":"#8799ff"}},"secondary":{"default":{"hex":"#00a9fa"}},"tertiary":{"default":{"hex":"#c085ff"}},"error":{"default":{"hex":"#ff291a"}}}}"##;
        let palette = Palette { background: Color::rgba(10, 10, 10, 255), surface: Color::rgba(20, 20, 20, 255), text: Color::rgba(240, 240, 240, 255), muted: Color::rgba(150, 150, 150, 255), accent: Color::rgba(90, 90, 90, 255), border: Color::rgba(60, 60, 60, 255) };
        Theme::new(palette, Some(json)).with_shape(Shape::Chamfer)
    }

    fn color_of(scenes: &[(Scene<LockTarget>, (f32, f32))], text: &str) -> Color {
        scenes
            .iter()
            .flat_map(|(scene, _)| &scene.nodes)
            .find_map(|node| match node {
                Node::Label { text: label, color, .. } if label == text => Some(*color),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no label {text:?}"))
    }

    #[test]
    fn the_cyberdeck_sets_some_text_in_the_wallpapers_other_accents() {
        let theme = colorful();
        let shown = scenes(&LockModel { output: (1920.0, 1080.0), ..greeter() }, &theme, &mut Mono);
        // The date is a little faded.
        assert_eq!(color_of(&shown, "DIENSTAG, 20. OKTOBER"), theme.secondary.faded(0.8));
        // Code: keywords in the third accent, capitalized names in the second, the rest quiet.
        assert_eq!(color_of(&shown, "fn"), theme.tertiary);
        assert_eq!(color_of(&shown, "Result"), theme.secondary);
        assert_eq!(color_of(&shown, "// splice::greeter"), theme.accent);
        assert_eq!(color_of(&shown, "[ ok ]"), theme.secondary);
        // Restart and shut down are red; the normal theme keeps them like the other buttons.
        assert_eq!(color_of(&shown, POWEROFF), theme.alert);
        let normal = scenes(&LockModel { output: (1920.0, 1080.0), ..greeter() }, &colorful().with_shape(Shape::Round), &mut Mono);
        assert_eq!(color_of(&normal, POWEROFF), theme.text);
    }

    #[test]
    fn code_runs_join_to_the_line_and_name_what_they_are() {
        for line in LOGIN_FLOW {
            assert_eq!(code_runs(line).into_iter().map(|(_, run)| run).collect::<String>(), line);
        }
        assert_eq!(
            code_runs("  let s = Foo::new(); // go"),
            [
                (Code::Plain, "  ".to_owned()),
                (Code::Keyword, "let".to_owned()),
                (Code::Plain, " s = ".to_owned()),
                (Code::Type, "Foo".to_owned()),
                (Code::Plain, "::new(); ".to_owned()),
                (Code::Comment, "// go".to_owned()),
            ]
        );
    }

    #[test]
    fn the_cyberdeck_card_sits_higher_than_the_normal_one() {
        let (deck_card, normal_card) = (card_top(&deck(), 1440.0), card_top(&theme(), 1440.0));
        assert!(deck_card < normal_card, "{deck_card} {normal_card}");
        // On a short monitor it keeps its distance from the date instead of overlapping it.
        assert!(card_top(&deck(), 800.0) >= 800.0 * 0.16 + (112.0 * 1.32) + 4.0 + 22.0 * 1.32 + 48.0 - 4.0);
    }

    fn card_top(theme: &Theme, height: f32) -> f32 {
        let model = LockModel { output: (2560.0, height), ..greeter() };
        let (scene, origin) = card_scene(&model, theme, &mut Mono);
        origin.1 + scene.target_rect(&LockTarget::Card).unwrap().y
    }

    #[test]
    fn scanlines_darken_every_third_row() {
        let pixels = scanline_pixels(4, 7);
        assert_eq!(pixels.len(), 4 * 7 * 4);
        let alpha = |x: usize, y: usize| pixels[(y * 4 + x) * 4 + 3];
        assert_eq!([alpha(0, 0), alpha(3, 0), alpha(0, 3), alpha(2, 6)], [SCANLINE_ALPHA; 4]);
        assert_eq!([alpha(0, 1), alpha(3, 2), alpha(1, 4), alpha(2, 5)], [0; 4]);
        // Black, and nothing else but alpha.
        assert!(pixels.as_chunks::<4>().0.iter().all(|p| p[..3] == [0, 0, 0]));
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
