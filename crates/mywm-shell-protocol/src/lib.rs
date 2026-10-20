//! The pipe between the compositor and `mywm-shell-services`, one JSON object per line: the services process
//! writes `Event`s to its stdout (the whole `Status` whenever any part of it changes, and every notification on
//! its own, so a volume change does not send their pictures again) and reads `Command`s from its stdin.
//! A line that does not parse is skipped, so either side may learn new fields first.
use serde::{Deserialize, Serialize};

/// The longest line either side accepts; anything longer is dropped with its connection. A notification's
/// picture (at most `IMAGE_SIZE` square, base64) fits several times.
pub const MAX_LINE: usize = 1 << 20;
/// Notification pictures are scaled to fit this many pixels each way: the card shows them 42 logical pixels
/// large, so this is sharp up to a scale of three.
pub const IMAGE_SIZE: u32 = 128;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    Status(Status),
    /// A new notification, or new contents for one with this id (`replaces_id`).
    Notification(Notification),
    /// The notification is gone (closed by its app, or after an action of a non-resident one).
    Closed { id: u32 },
}

/// What the bar shows of the system.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    /// The default output (PipeWire's `default.audio.sink`); `None` without one.
    pub audio: Option<Audio>,
    /// The title (or, without one, the name) of the first MPRIS player that plays.
    pub media: Option<String>,
    /// BlueZ's first adapter; `None` without BlueZ or adapter.
    pub bluetooth: Option<Bluetooth>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Audio {
    /// 0.0 to 1.0 on the cubic scale `wpctl` and Quickshell show (the cube root of PipeWire's channel volume).
    pub volume: f32,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bluetooth {
    pub powered: bool,
    /// Devices of the adapter that are connected.
    pub connected: usize,
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
    /// Set the default output's volume (cubic scale, clamped to 0.0 to 1.0) and unmute it.
    SetVolume { volume: f32 },
    ToggleMute,
    /// The user closed the notification (`NotificationClosed` with reason 2).
    Dismiss { id: u32 },
    /// The user chose one of the notification's actions.
    Invoke { id: u32, action: String },
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
        let status = Status {
            audio: Some(Audio { volume: 0.8, muted: true }),
            media: Some("Netflix".into()),
            bluetooth: Some(Bluetooth { powered: true, connected: 1 }),
        };
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
        for event in [Event::Status(status), Event::Notification(notification), Event::Closed { id: 3 }] {
            let line = encode(&event);
            assert!(line.ends_with('\n') && !line[..line.len() - 1].contains('\n'), "{line}");
            assert_eq!(decode::<Event>(&line), Some(event));
        }
        let commands =
            [Command::SetVolume { volume: 0.25 }, Command::ToggleMute, Command::Dismiss { id: 2 }, Command::Invoke { id: 2, action: "x".into() }];
        for command in commands {
            assert_eq!(decode::<Command>(&encode(&command)), Some(command));
        }
        assert_eq!(encode(&Command::ToggleMute), "{\"command\":\"toggle-mute\"}\n");
    }

    #[test]
    fn unknown_or_missing_parts_do_not_break_a_line() {
        assert_eq!(decode::<Event>(r#"{"event":"status"}"#), Some(Event::Status(Status::default())));
        let newer = r#"{"event":"status","audio":null,"media":"x","tray":[1,2]}"#;
        assert_eq!(decode::<Event>(newer), Some(Event::Status(Status { media: Some("x".into()), ..Status::default() })));
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
