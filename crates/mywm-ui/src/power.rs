//! The power menu (`bar.qml`): a card in the middle of a dimmed monitor with tiles for lock, log out, restart
//! and shut down. The last three count down five seconds around their icon before they happen.
use std::f32::consts::{FRAC_PI_2, PI};

use crate::{Align, Color, Measure, Node, Rect, Scene, TextStyle, Theme, bar::button_shape, i18n::Language};

/// The dimming over the whole monitor behind the card.
pub const BACKDROP: Color = Color::rgba(0, 0, 0, 0x66);
/// How long the countdown runs.
pub const COUNTDOWN_MS: u32 = 5000;
/// How long the card takes to change its size.
pub const RESIZE_MS: u32 = 160;
const RADIUS: f32 = 28.0;
const MARGIN: f32 = 24.0;
const SPACING: f32 = 22.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerAction {
    Lock,
    Logout,
    Reboot,
    Poweroff,
}

impl PowerAction {
    pub const TILES: [PowerAction; 4] = [PowerAction::Lock, PowerAction::Logout, PowerAction::Reboot, PowerAction::Poweroff];

    pub fn icon(self) -> &'static str {
        match self {
            PowerAction::Lock => "󰌾",
            PowerAction::Logout => "󰍃",
            PowerAction::Reboot => "󰜉",
            PowerAction::Poweroff => "⏻",
        }
    }

    /// The tile's label.
    fn label(self, language: Language) -> &'static str {
        match self {
            PowerAction::Lock => language.tr("Sperren"),
            PowerAction::Logout => language.tr("Abmelden"),
            PowerAction::Reboot => language.tr("Neustart"),
            PowerAction::Poweroff => language.tr("Ausschalten"),
        }
    }

    /// The title while counting down.
    fn title(self, language: Language) -> &'static str {
        match self {
            PowerAction::Lock => language.tr("Sperren"),
            PowerAction::Logout => language.tr("Abmelden"),
            PowerAction::Reboot => language.tr("Neu starten"),
            PowerAction::Poweroff => language.tr("Ausschalten"),
        }
    }

    /// Whether it counts down first (all but locking, which happens at once).
    pub fn counts_down(self) -> bool {
        self != PowerAction::Lock
    }

    /// The `systemctl` verb, for the actions that are not the compositor's own.
    pub fn systemctl(self) -> Option<&'static str> {
        match self {
            PowerAction::Reboot => Some("reboot"),
            PowerAction::Poweroff => Some("poweroff"),
            PowerAction::Lock | PowerAction::Logout => None,
        }
    }
}

/// What the menu says when `systemctl` ended with `code` instead of restarting or shutting down.
pub fn failure(language: Language, code: i32) -> String {
    language.trf("Aktion fehlgeschlagen (%1)", &[&code.to_string()])
}

/// What a click or a focused key press in the menu means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerTarget {
    /// The card itself: clicks there do nothing (outside it they close the menu).
    Card,
    Tile(PowerAction),
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerModel {
    /// The action counting down; `None` while the tiles show.
    pub pending: Option<PowerAction>,
    /// The countdown's remaining part, 1.0 to 0.0.
    pub progress: f32,
    /// Why the last action failed.
    pub error: Option<String>,
    /// The card's current size (it animates between the two layouts, see `card_size`).
    pub size: (f32, f32),
    pub hover: Option<PowerTarget>,
    /// The item Tab moved the keyboard focus to.
    pub focus: Option<PowerTarget>,
    pub language: Language,
}

/// The card's size for the tiles or for the countdown on a monitor `output` large.
pub fn card_size(pending: bool, output: (f32, f32)) -> (f32, f32) {
    if pending { (380.0, 330.0) } else { ((output.0 - 32.0).min(620.0), 260.0) }
}

/// `Easing.OutCubic` at `t` (0.0 to 1.0).
pub fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// The items Tab cycles through, in order.
pub fn focus_chain(pending: bool) -> Vec<PowerTarget> {
    if pending { vec![PowerTarget::Cancel] } else { PowerAction::TILES.map(PowerTarget::Tile).to_vec() }
}

/// A one-line text centered in the card's inner width at `y`; returns its height.
fn centered(scene: &mut Scene<PowerTarget>, y: f32, text: String, style: TextStyle, color: Color, measure: &mut impl Measure) -> f32 {
    let height = measure.line_height(style);
    let rect = Rect::new(MARGIN, y, scene.width - 2.0 * MARGIN, height);
    scene.push(Node::Label { rect, text, style, color, align: Align::Center });
    height
}

/// The card's picture and targets, with the card at (0, 0).
pub fn scene(model: &PowerModel, theme: &Theme, measure: &mut impl Measure) -> Scene<PowerTarget> {
    let (width, height) = model.size;
    let mut scene = Scene::new(width, height);
    let card = Rect::new(0.0, 0.0, width, height);
    scene.push(Node::Rect { rect: card, radius: RADIUS, fill: theme.glass(), border: Some((1.0, theme.glass_border())) });
    scene.target(card, PowerTarget::Card);
    let tr = |text| model.language.tr(text);
    let mut y = MARGIN;
    match model.pending {
        None => {
            y += centered(&mut scene, y, tr("Sitzung und System").into(), TextStyle::new(22.0), theme.text, measure) + SPACING;
            let tile_width = (width - 2.0 * MARGIN - 36.0) / 4.0;
            for (index, action) in PowerAction::TILES.into_iter().enumerate() {
                let rect = Rect::new(MARGIN + index as f32 * (tile_width + 12.0), y, tile_width, 112.0);
                let target = PowerTarget::Tile(action);
                let lit = model.hover == Some(target) || model.focus == Some(target);
                let fill = if lit { theme.accent } else { theme.glass_surface() };
                scene.push(Node::Rect { rect, radius: 22.0, fill, border: None });
                // Icon and label as one column in the middle of the tile, 10 apart.
                let (icon, label) = (TextStyle::new(34.0), TextStyle::new(13.0));
                let (icon_h, label_h) = (measure.line_height(icon), measure.line_height(label));
                let top = rect.y + (rect.h - icon_h - 10.0 - label_h) / 2.0;
                let (icon_color, label_color) = if lit { (theme.background, theme.background) } else { (theme.accent, theme.text) };
                let row = |top, h| Rect::new(rect.x, top, rect.w, h);
                let text = action.icon().into();
                scene.push(Node::Label { rect: row(top, icon_h), text, style: icon, color: icon_color, align: Align::Center });
                let text = action.label(model.language).into();
                scene.push(Node::Label { rect: row(top + icon_h + 10.0, label_h), text, style: label, color: label_color, align: Align::Center });
                scene.target(rect, target);
            }
            y += 112.0 + SPACING;
        }
        Some(action) => {
            let ring = (width / 2.0, y + 56.0);
            let full = 2.0 * PI;
            scene.push(Node::Arc { center: ring, radius: 50.0, width: 5.0, color: theme.border, start: 0.0, sweep: full });
            let sweep = full * model.progress.clamp(0.0, 1.0);
            scene.push(Node::Arc { center: ring, radius: 50.0, width: 5.0, color: theme.accent, start: -FRAC_PI_2, sweep });
            let disc = Rect::new(ring.0 - 44.0, ring.1 - 44.0, 88.0, 88.0);
            scene.push(Node::Rect { rect: disc, radius: 44.0, fill: theme.surface, border: None });
            let style = TextStyle::new(42.0);
            scene.push(Node::Label { rect: disc, text: action.icon().into(), style, color: theme.accent, align: Align::Center });
            y += 112.0 + 14.0;
            let title = action.title(model.language).into();
            y += centered(&mut scene, y, title, TextStyle::new(22.0).bold(true), theme.text, measure) + 14.0;
            let seconds = ((model.progress * (COUNTDOWN_MS / 1000) as f32).ceil() as u32).max(1).to_string();
            let text = model.language.trf("Automatisch in %1 Sekunden", &[&seconds]);
            y += centered(&mut scene, y, text, TextStyle::new(14.0), theme.muted, measure) + 14.0;
            let label = tr("Abbrechen");
            let button_width = measure.width(label, TextStyle::new(14.0)).ceil() + 16.0;
            let rect = Rect::new((width - button_width) / 2.0, y, button_width, 40.0);
            let lit = model.hover == Some(PowerTarget::Cancel) || model.focus == Some(PowerTarget::Cancel);
            button_shape(&mut scene, theme, rect, label, false, false, false, lit);
            scene.target(rect, PowerTarget::Cancel);
            y += 40.0 + SPACING;
        }
    }
    let note = match (&model.error, model.pending) {
        (Some(error), _) => Some(error.clone()),
        (None, None) => Some(tr("Esc zum Schließen").to_owned()),
        (None, Some(_)) => None,
    };
    if let Some(note) = note {
        centered(&mut scene, y, note, TextStyle::new(14.0), theme.muted, measure);
    }
    scene
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{scene::tests::Mono, theme::Palette};

    fn theme() -> Theme {
        let c = |v| Color::rgba(v, v, v, 255);
        Theme::new(Palette { background: c(10), surface: c(20), text: c(200), muted: c(150), accent: c(90), border: c(60) }, None)
    }

    fn model(pending: Option<PowerAction>) -> PowerModel {
        PowerModel {
            pending,
            progress: 1.0,
            error: None,
            size: card_size(pending.is_some(), (1280.0, 800.0)),
            hover: None,
            focus: None,
            language: Language::English,
        }
    }

    fn texts(scene: &Scene<PowerTarget>) -> Vec<String> {
        scene.nodes.iter().filter_map(|n| match n { Node::Label { text, .. } => Some(text.clone()), _ => None }).collect()
    }

    #[test]
    fn the_tiles_fill_the_card_in_a_row() {
        let scene = scene(&model(None), &theme(), &mut Mono);
        assert_eq!((scene.width, scene.height), (620.0, 260.0));
        let tiles: Vec<Rect> = PowerAction::TILES.iter().map(|a| scene.target_rect(&PowerTarget::Tile(*a)).unwrap()).collect();
        let width = (620.0 - 48.0 - 36.0) / 4.0;
        assert_eq!(tiles[0], Rect::new(24.0, 24.0 + 30.0 + 22.0, width, 112.0));
        assert_eq!(tiles[3].right(), 620.0 - 24.0);
        let texts = texts(&scene);
        assert!(texts.contains(&"Session and system".to_owned()) && texts.contains(&"Esc to close".to_owned()), "{texts:?}");
        // A narrow monitor narrows the card.
        assert_eq!(card_size(false, (500.0, 800.0)), (468.0, 260.0));
    }

    #[test]
    fn the_countdown_counts_whole_seconds_down_to_one() {
        let mut countdown = model(Some(PowerAction::Reboot));
        countdown.progress = 0.61;
        let scene = scene(&countdown, &theme(), &mut Mono);
        let texts = texts(&scene);
        assert!(texts.contains(&"Restart".to_owned()) && texts.contains(&"Automatically in 4 seconds".to_owned()), "{texts:?}");
        assert!(!texts.contains(&"Esc to close".to_owned()));
        assert!(scene.target_rect(&PowerTarget::Cancel).is_some());
        countdown.progress = 0.0;
        assert!(texts_of(&countdown).contains(&"Automatically in 1 seconds".to_owned()));
        countdown.error = Some("Action failed (1)".into());
        assert!(texts_of(&countdown).contains(&"Action failed (1)".to_owned()));
    }

    fn texts_of(model: &PowerModel) -> Vec<String> {
        texts(&scene(model, &theme(), &mut Mono))
    }

    #[test]
    fn focus_lights_a_tile_like_hovering() {
        let mut focused = model(None);
        focused.focus = Some(PowerTarget::Tile(PowerAction::Logout));
        let scene = scene(&focused, &theme(), &mut Mono);
        let rect = scene.target_rect(&PowerTarget::Tile(PowerAction::Logout)).unwrap();
        assert!(scene.nodes.iter().any(|n| matches!(n, Node::Rect { rect: r, fill, .. } if *r == rect && *fill == theme().accent)));
        assert_eq!(focus_chain(true), vec![PowerTarget::Cancel]);
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert!(ease_out_cubic(0.5) > 0.8);
    }
}
