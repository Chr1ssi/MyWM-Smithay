//! The media panel's content (`MediaPanel.qml`): every MPRIS player with its cover, title, artist and the buttons
//! for the previous track, play/pause and the next track; a button the player does not allow is disabled. Each
//! player sits in a card.
use crate::{
    Align, Measure, Node, Rect, TextStyle, Theme,
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

const PLAYER_SPACING: f32 = 10.0;
const PADDING: f32 = 14.0;
const SPACING: f32 = 8.0;
const COVER_HEIGHT: f32 = 150.0;
const SIDE_BUTTON: f32 = 38.0;
const PLAY_BUTTON: f32 = 46.0;
const BUTTON_SPACING: f32 = 14.0;
const ICON: TextStyle = TextStyle::new(18.0).icons(true);

pub fn content(players: &[PlayerView], hover: Option<&PanelTarget>, theme: &Theme, measure: &mut impl Measure, width: f32, language: Language) -> Content {
    let mut content = Content::default();
    if players.is_empty() {
        let style = TextStyle::new(14.0);
        let rect = Rect::new(0.0, 0.0, width, measure.line_height(style));
        content.push(Node::Label { rect, text: language.tr("Keine aktive Wiedergabe").into(), style, color: theme.muted, align: Align::Left });
        content.height = rect.h;
        return content;
    }
    let inner = width - 2.0 * PADDING;
    let (title_style, artist_style) = (TextStyle::new(16.0).bold(true), TextStyle::new(13.0));
    let (title_h, artist_h) = (measure.line_height(title_style), measure.line_height(artist_style));
    let cover = |player: &PlayerView| if player.art.is_some() { COVER_HEIGHT + SPACING } else { 0.0 };
    let mut y = 0.0;
    for (index, player) in players.iter().enumerate() {
        if index > 0 {
            y += PLAYER_SPACING;
        }
        let height = PADDING + cover(player) + title_h + 2.0 + artist_h + SPACING + PLAY_BUTTON + PADDING;
        content.card(Rect::new(0.0, y, width, height), theme.glass_surface());
        let mut top = y + PADDING;
        if let Some(art) = &player.art {
            content.push(Node::Image { rect: Rect::new(PADDING, top, inner, COVER_HEIGHT), picture: art.clone() });
            top += COVER_HEIGHT + SPACING;
        }
        let fallback = |text: &str| if text.is_empty() { player.identity.clone() } else { text.to_owned() };
        content.push(Node::Label { rect: Rect::new(PADDING, top, inner, title_h), text: fallback(&player.title), style: title_style, color: theme.text, align: Align::Left });
        top += title_h + 2.0;
        content.push(Node::Label { rect: Rect::new(PADDING, top, inner, artist_h), text: fallback(&player.artist), style: artist_style, color: theme.muted, align: Align::Left });
        top += artist_h + SPACING;
        let toggle = if player.playing { "󰏤" } else { "󰐊" };
        let buttons = [
            ("󰒮", player.can_previous, MediaTarget::Previous(index), SIDE_BUTTON),
            (toggle, player.can_toggle, MediaTarget::Toggle(index), PLAY_BUTTON),
            ("󰒭", player.can_next, MediaTarget::Next(index), SIDE_BUTTON),
        ];
        let row = buttons.iter().map(|b| b.3).sum::<f32>() + BUTTON_SPACING * (buttons.len() - 1) as f32;
        let mut x = (width - row) / 2.0;
        for (glyph, enabled, target, size) in buttons {
            let rect = Rect::new(x, top + (PLAY_BUTTON - size) / 2.0, size, size);
            let panel_target = PanelTarget::Media(target);
            let hovered = enabled && hover == Some(&panel_target);
            let main = matches!(target, MediaTarget::Toggle(_));
            let (fill, color) = match (main, hovered) {
                (true, false) => (Some(theme.accent), theme.background),
                (true, true) => (Some(theme.text), theme.background),
                (false, true) => (Some(theme.surface), theme.text),
                (false, false) => (None, theme.text),
            };
            let mut nodes = Vec::new();
            if let Some(fill) = fill {
                nodes.push(Node::Rect { rect, radius: size / 2.0, fill, border: None });
            }
            nodes.push(Node::Label { rect, text: glyph.into(), style: ICON, color, align: Align::Center });
            let fade = if enabled { 1.0 } else { 0.45 };
            content.nodes.extend(nodes.into_iter().map(|node| node.faded(fade)));
            if enabled {
                content.target(rect, panel_target);
            }
            x += size + BUTTON_SPACING;
        }
        y += height;
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
        // Padding, cover, title and artist (2 apart), the buttons (8 below) and padding again.
        assert_eq!(content.height, 14.0 + 150.0 + 8.0 + 22.0 + 2.0 + 18.0 + 8.0 + 46.0 + 14.0);
        // The buttons are centered: the first starts as far from the left as the last ends from the right.
        let toggle = content.targets[0].0;
        assert!((toggle.x + toggle.w / 2.0 - 180.0).abs() < 0.01);
    }
}
