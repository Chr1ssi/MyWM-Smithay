//! The media panel's content (`MediaPanel.qml`): every MPRIS player with its cover, title, artist and the buttons
//! for the previous track, play/pause and the next track; a button the player does not allow is disabled.
use crate::{
    Align, Measure, Node, Rect, Scene, TextStyle, Theme,
    bar::{BUTTON_HEIGHT, button_shape},
    i18n::Language,
    panel::{Content, PanelTarget},
    scene::Picture,
};

/// A player as the panel shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerView {
    pub identity: String,
    pub title: String,
    pub artist: String,
    pub art: Option<Picture>,
    pub playing: bool,
    pub can_previous: bool,
    pub can_toggle: bool,
    pub can_next: bool,
}

/// A button of the player at this index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaTarget {
    Previous(usize),
    Toggle(usize),
    Next(usize),
}

const PLAYER_SPACING: f32 = 14.0;
const SPACING: f32 = 8.0;
const BUTTON_SPACING: f32 = 16.0;
const BUTTON_TEXT: TextStyle = TextStyle::new(14.0);

pub fn content(players: &[PlayerView], hover: Option<&PanelTarget>, theme: &Theme, measure: &mut impl Measure, width: f32, language: Language) -> Content {
    let mut content = Content::default();
    if players.is_empty() {
        let style = TextStyle::new(14.0);
        let rect = Rect::new(0.0, 0.0, width, measure.line_height(style));
        content.push(Node::Label { rect, text: language.tr("Keine aktive Wiedergabe").into(), style, color: theme.muted, align: Align::Left });
        content.height = rect.h;
        return content;
    }
    let mut y = 0.0;
    for (index, player) in players.iter().enumerate() {
        if index > 0 {
            y += PLAYER_SPACING;
        }
        if let Some(art) = &player.art {
            content.push(Node::Image { rect: Rect::new(0.0, y, width, 150.0), picture: art.clone() });
            y += 150.0 + SPACING;
        }
        let fallback = |text: &str| if text.is_empty() { player.identity.clone() } else { text.to_owned() };
        for (text, size, color) in [(fallback(&player.title), 17.0, theme.text), (fallback(&player.artist), 14.0, theme.muted)] {
            let style = TextStyle::new(size);
            let rect = Rect::new(0.0, y, width, measure.line_height(style));
            content.push(Node::Label { rect, text, style, color, align: Align::Left });
            y += rect.h + SPACING;
        }
        let toggle = if player.playing { "󰏤" } else { "󰐊" };
        let buttons = [
            ("󰒮", player.can_previous, MediaTarget::Previous(index)),
            (toggle, player.can_toggle, MediaTarget::Toggle(index)),
            ("󰒭", player.can_next, MediaTarget::Next(index)),
        ];
        let widths: Vec<f32> = buttons.iter().map(|(text, _, _)| measure.width(text, BUTTON_TEXT).ceil() + 16.0).collect();
        let row = widths.iter().sum::<f32>() + BUTTON_SPACING * (buttons.len() - 1) as f32;
        let mut x = (width - row) / 2.0;
        for ((text, enabled, target), button_width) in buttons.into_iter().zip(widths) {
            let rect = Rect::new(x, y, button_width, BUTTON_HEIGHT);
            let panel_target = PanelTarget::Media(target);
            let hovered = enabled && hover == Some(&panel_target);
            let mut button: Scene<()> = Scene::new(width, 0.0);
            button_shape(&mut button, theme, rect, text, false, false, false, hovered);
            let fade = if enabled { 1.0 } else { 0.45 };
            content.nodes.extend(button.nodes.into_iter().map(|node| node.faded(fade)));
            if enabled {
                content.target(rect, panel_target);
            }
            x += button_width + BUTTON_SPACING;
        }
        y += BUTTON_HEIGHT;
    }
    content.height = y;
    content
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
    fn nothing_playing_says_so() {
        let content = content(&[], None, &theme(), &mut Mono, 360.0, Language::German);
        assert!(content.nodes.iter().any(|n| matches!(n, Node::Label { text, .. } if text == "Keine aktive Wiedergabe")));
        assert_eq!(content.height, 19.0);
    }

    #[test]
    fn a_player_shows_cover_texts_and_its_allowed_buttons() {
        let art = Picture::from_rgba(1, 1, &[1, 2, 3, 255]);
        let player = PlayerView {
            identity: "Firefox".into(),
            title: "Lied".into(),
            playing: true,
            can_toggle: true,
            can_next: true,
            art,
            ..PlayerView::default()
        };
        let content = content(&[player], None, &theme(), &mut Mono, 360.0, Language::English);
        let texts: Vec<_> = content.nodes.iter().filter_map(|n| match n { Node::Label { text, .. } => Some(text.as_str()), _ => None }).collect();
        // No artist: the player's name stands in.
        assert_eq!(texts, ["Lied", "Firefox", "󰒮", "󰏤", "󰒭"]);
        let targets: Vec<_> = content.targets.iter().map(|(_, t)| *t).collect();
        assert_eq!(targets, [PanelTarget::Media(MediaTarget::Toggle(0)), PanelTarget::Media(MediaTarget::Next(0))]);
        // Cover, title, artist and buttons, 8 apart.
        assert_eq!(content.height, 150.0 + 8.0 + 23.0 + 8.0 + 19.0 + 8.0 + 26.0);
        // The buttons are centered: the first starts as far from the left as the last ends from the right.
        let toggle = content.targets[0].0;
        assert!((toggle.x + toggle.w / 2.0 - 180.0).abs() < 0.01);
    }
}
