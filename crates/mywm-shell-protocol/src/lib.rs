//! The pipe between the compositor and `mywm-shell-services`, one JSON object per line: the services process
//! writes `Event`s to its stdout (the whole `Status` whenever any part of it changes, and every notification on
//! its own, so a volume change does not send their pictures again) and reads `Command`s from its stdin.
//! A line that does not parse is skipped, so either side may learn new fields first.
use serde::{Deserialize, Serialize};

/// The longest line either side accepts; anything longer is dropped with its connection. Notification pictures
/// (at most `IMAGE_SIZE` square) and covers (at most `COVER_SIZE`, base64) fit several times.
pub const MAX_LINE: usize = 4 << 20;
/// Notification pictures are scaled to fit this many pixels each way: the card shows them 42 logical pixels
/// large, so this is sharp up to a scale of three.
pub const IMAGE_SIZE: u32 = 128;
/// Tray and menu icons are scaled to fit this many pixels each way (they show 18 logical pixels large).
pub const ICON_SIZE: u32 = 36;
/// Covers are scaled down to just cover this many pixels (the media panel shows them 150 logical pixels high
/// across its width, cut to fit), as Quickshell's `sourceSize`.
pub const COVER_SIZE: (u32, u32) = (400, 200);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    Status(Status),
    /// A new notification, or new contents for one with this id (`replaces_id`).
    Notification(Notification),
    /// The notification is gone (closed by its app, or after an action of a non-resident one).
    Closed { id: u32 },
    /// The MPRIS players, in the order they appeared.
    Players { players: Vec<Player> },
    /// The tray's items (StatusNotifierItem), in the order they registered.
    Tray { items: Vec<TrayItem> },
    /// The menu of a tray item, after `TrayMenu` or `TrayMenuShow`, or when it changed while shown.
    TrayMenu { address: String, menu: MenuEntry },
}

/// What the bar and the panels show of the system.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    /// PipeWire's outputs, microphones and the streams of applications, in the order they appeared.
    pub nodes: Vec<AudioNode>,
    /// The ids of `default.audio.sink` and `default.audio.source`.
    pub default_sink: Option<u32>,
    pub default_source: Option<u32>,
    /// BlueZ's first adapter; `None` without BlueZ or adapter.
    pub bluetooth: Option<Bluetooth>,
}

impl Status {
    /// The default output's volume, `None` without one (or before it reported its volume).
    pub fn default_audio(&self) -> Option<Audio> {
        let id = self.default_sink?;
        self.nodes.iter().find(|n| n.id == id)?.audio
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    /// `Audio/Sink`.
    #[default]
    Sink,
    /// `Audio/Source…`, but WirePlumber's smart filters.
    Source,
    /// `Stream/Output/Audio` of an application.
    Stream,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioNode {
    pub id: u32,
    pub kind: NodeKind,
    pub name: String,
    pub description: String,
    pub application: String,
    /// `None` until the node reported its volume.
    pub audio: Option<Audio>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Audio {
    /// 0.0 to 1.0 on the cubic scale `wpctl` and Quickshell show (the cube root of PipeWire's channel volume).
    pub volume: f32,
    pub muted: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bluetooth {
    pub powered: bool,
    pub discovering: bool,
    /// The adapter's devices.
    pub devices: Vec<Device>,
}

impl Bluetooth {
    /// The devices the command center lists: paired ones, and while searching the others too; connected ones
    /// first, then paired ones, then by name.
    pub fn listed(&self) -> Vec<&Device> {
        let mut devices: Vec<&Device> = self.devices.iter().filter(|d| d.paired || self.discovering).collect();
        devices.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.paired.cmp(&a.paired)).then_with(|| a.name.cmp(&b.name)));
        devices
    }

    /// Listed devices that are connected.
    pub fn connected(&self) -> usize {
        self.listed().iter().filter(|d| d.connected).count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Busy {
    Pairing,
    Connecting,
    Disconnecting,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Device {
    /// Its object path, which commands name.
    pub path: String,
    /// Its alias (what the user called it), else its address.
    pub name: String,
    pub address: String,
    pub paired: bool,
    pub connected: bool,
    /// Percent.
    pub battery: Option<u8>,
    /// A call for it runs.
    pub busy: Option<Busy>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrayStatus {
    /// Hidden: nothing to say right now.
    Passive,
    #[default]
    Active,
    NeedsAttention,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrayItem {
    /// `bus name/object path`, which commands name.
    pub address: String,
    pub id: String,
    pub title: String,
    pub status: TrayStatus,
    pub icon: Option<Image>,
    pub has_menu: bool,
    /// It has only a menu: a click opens that instead of activating the item.
    pub only_menu: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Toggle {
    #[default]
    None,
    Check,
    Radio,
}

/// An entry of a tray item's menu (DBusMenu), with its visible children.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MenuEntry {
    pub id: i32,
    /// As sent, with `_` before the mnemonic.
    pub label: String,
    pub separator: bool,
    pub enabled: bool,
    pub icon_name: String,
    pub icon: Option<Image>,
    pub toggle: Toggle,
    pub checked: bool,
    /// It opens a submenu (`children-display: submenu`).
    pub submenu: bool,
    pub children: Vec<MenuEntry>,
}

/// An MPRIS player.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Player {
    /// Its bus name (`org.mpris.MediaPlayer2.…`), which commands name.
    pub bus: String,
    pub identity: String,
    pub title: String,
    /// The artists, joined with ", ".
    pub artist: String,
    pub cover: Option<Image>,
    pub playing: bool,
    pub can_previous: bool,
    /// Pause while playing, else play.
    pub can_toggle: bool,
    pub can_next: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlayerAction {
    Previous,
    Play,
    Pause,
    Next,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

/// A notification as `org.freedesktop.Notifications.Notify` gave it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub summary: String,
    /// With the markup the spec allows (`<b>`, `<i>`, `<u>`, `<a>`), as sent.
    pub body: String,
    pub urgency: Urgency,
    /// Milliseconds the toast shows; 0 keeps it, -1 leaves it to the shell.
    pub expire_timeout: i32,
    pub actions: Vec<Action>,
    /// The notification's own picture, else its app's icon.
    pub image: Option<Image>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub identifier: String,
    pub text: String,
}

/// Straight-alpha RGBA pixels, row by row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    #[serde(with = "base64")]
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum Command {
    /// Set a node's volume (cubic scale, clamped to 0.0 to 1.0) and unmute it.
    SetVolume { id: u32, volume: f32 },
    ToggleMute { id: u32 },
    /// Make the node the default output or microphone.
    SetDefault { id: u32 },
    BluetoothPower { on: bool },
    BluetoothDiscover { on: bool },
    DeviceConnect { path: String },
    DeviceDisconnect { path: String },
    /// Trust the device (so it connects by itself later) and pair it.
    DevicePair { path: String },
    /// The user closed the notification (`NotificationClosed` with reason 2).
    Dismiss { id: u32 },
    /// The user chose one of the notification's actions.
    Invoke { id: u32, action: String },
    Player { bus: String, action: PlayerAction },
    TrayActivate { address: String },
    TraySecondaryActivate { address: String },
    /// `delta` in eighths of a degree, as Qt's `angleDelta` (120 a notch, positive away from the user).
    TrayScroll { address: String, delta: i32, horizontal: bool },
    /// The item's menu is about to show: send it.
    TrayMenu { address: String },
    /// A submenu of it is about to show.
    TrayMenuShow { address: String, id: i32 },
    TrayMenuClick { address: String, id: i32 },
}

/// `message` as one line, with its newline.
pub fn encode<T: Serialize>(message: &T) -> String {
    // Serializing these plain types cannot fail.
    let mut line = serde_json::to_string(message).unwrap_or_default();
    line.push('\n');
    line
}

/// The message on `line` (with or without its newline), `None` if it is not one.
pub fn decode<'a, T: Deserialize<'a>>(line: &'a str) -> Option<T> {
    serde_json::from_str(line.trim_end()).ok()
}

/// Bytes as standard base64 text, so pictures take a third more room on the line instead of four times as much.
mod base64 {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(bytes: &[u8]) -> String {
        let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let value = u32::from(chunk[0]) << 16 | u32::from(*chunk.get(1).unwrap_or(&0)) << 8 | u32::from(*chunk.get(2).unwrap_or(&0));
            for index in 0..4 {
                if index <= chunk.len() {
                    text.push(char::from(ALPHABET[(value >> (18 - 6 * index) & 63) as usize]));
                } else {
                    text.push('=');
                }
            }
        }
        text
    }

    pub fn decode(text: &str) -> Option<Vec<u8>> {
        let digits: Vec<u32> = text
            .trim_end_matches('=')
            .bytes()
            .map(|b| ALPHABET.iter().position(|a| *a == b).map(|p| p as u32))
            .collect::<Option<_>>()?;
        let mut bytes = Vec::with_capacity(digits.len() * 3 / 4);
        for chunk in digits.chunks(4) {
            if chunk.len() == 1 {
                return None;
            }
            let value = chunk.iter().enumerate().fold(0, |value, (index, digit)| value | digit << (18 - 6 * index));
            bytes.extend((0..chunk.len() - 1).map(|index| (value >> (16 - 8 * index) & 255) as u8));
        }
        Some(bytes)
    }

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        decode(&text).ok_or_else(|| D::Error::custom("not base64"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_survive_the_pipe() {
        let node = AudioNode { id: 7, kind: NodeKind::Stream, application: "Firefox".into(), audio: Some(Audio { volume: 0.8, muted: true }), ..AudioNode::default() };
        let status = Status { nodes: vec![node], default_sink: Some(7), bluetooth: Some(Bluetooth::default()), ..Status::default() };
        assert_eq!(status.default_audio(), Some(Audio { volume: 0.8, muted: true }));
        let notification = Notification {
            id: 7,
            app_name: "Firefox".into(),
            summary: "Download".into(),
            body: "<b>fertig</b>\nzweite Zeile".into(),
            urgency: Urgency::Critical,
            expire_timeout: -1,
            actions: vec![Action { identifier: "default".into(), text: "Öffnen".into() }],
            image: Some(Image { width: 1, height: 2, rgba: vec![1, 2, 3, 4, 5, 6, 7, 255] }),
        };
        let players = vec![Player { bus: "org.mpris.MediaPlayer2.x".into(), title: "Lied".into(), playing: true, ..Player::default() }];
        let items = vec![TrayItem { address: ":1.5/StatusNotifierItem".into(), status: TrayStatus::NeedsAttention, ..TrayItem::default() }];
        let menu = MenuEntry {
            id: 0,
            children: vec![MenuEntry { id: 1, label: "_Quit".into(), enabled: true, toggle: Toggle::Check, ..MenuEntry::default() }],
            ..MenuEntry::default()
        };
        let events = [
            Event::Status(status),
            Event::Notification(notification),
            Event::Closed { id: 3 },
            Event::Players { players },
            Event::Tray { items },
            Event::TrayMenu { address: ":1.5/StatusNotifierItem".into(), menu },
        ];
        for event in events {
            let line = encode(&event);
            assert!(line.ends_with('\n') && !line[..line.len() - 1].contains('\n'), "{line}");
            assert_eq!(decode::<Event>(&line), Some(event));
        }
        let commands = [
            Command::SetVolume { id: 7, volume: 0.25 },
            Command::ToggleMute { id: 7 },
            Command::SetDefault { id: 8 },
            Command::Dismiss { id: 2 },
            Command::Invoke { id: 2, action: "x".into() },
            Command::Player { bus: "org.mpris.MediaPlayer2.x".into(), action: PlayerAction::Pause },
            Command::TrayScroll { address: ":1.5/StatusNotifierItem".into(), delta: -120, horizontal: false },
            Command::TrayMenuClick { address: ":1.5/StatusNotifierItem".into(), id: 3 },
        ];
        for command in commands {
            assert_eq!(decode::<Command>(&encode(&command)), Some(command));
        }
        assert_eq!(encode(&Command::ToggleMute { id: 3 }), "{\"command\":\"toggle-mute\",\"id\":3}\n");
    }

    #[test]
    fn listed_devices_are_paired_ones_connected_first() {
        let device = |name: &str, paired, connected| Device { name: name.into(), paired, connected, ..Device::default() };
        let bluetooth = Bluetooth {
            powered: true,
            discovering: false,
            devices: vec![device("b", true, false), device("new", false, false), device("a", true, false), device("z", true, true)],
        };
        let names: Vec<_> = bluetooth.listed().iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["z", "a", "b"]);
        assert_eq!(bluetooth.connected(), 1);
        let searching = Bluetooth { discovering: true, ..bluetooth };
        let names: Vec<_> = searching.listed().iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["z", "a", "b", "new"]);
    }

    #[test]
    fn unknown_or_missing_parts_do_not_break_a_line() {
        assert_eq!(decode::<Event>(r#"{"event":"status"}"#), Some(Event::Status(Status::default())));
        let newer = r#"{"event":"status","default_sink":4,"tray":[1,2]}"#;
        assert_eq!(decode::<Event>(newer), Some(Event::Status(Status { default_sink: Some(4), ..Status::default() })));
        assert_eq!(decode::<Command>(r#"{"command":"launch-rockets"}"#), None);
        assert_eq!(decode::<Event>("not json"), None);
    }

    #[test]
    fn base64_round_trips_every_length() {
        assert_eq!(base64::encode(b"Man"), "TWFu");
        assert_eq!(base64::encode(b"Ma"), "TWE=");
        assert_eq!(base64::encode(b"M"), "TQ==");
        for length in 0..20u8 {
            let bytes: Vec<u8> = (0..length).map(|b| b.wrapping_mul(37)).collect();
            assert_eq!(base64::decode(&base64::encode(&bytes)), Some(bytes));
        }
        assert_eq!(base64::decode("T"), None);
        assert_eq!(base64::decode("T!=="), None);
    }
}
