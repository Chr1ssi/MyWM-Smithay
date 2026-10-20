//! The pipe between the compositor and `mywm-shell-services`, one JSON object per line: the services process
//! writes the whole `Status` to its stdout whenever any part of it changes, and reads `Command`s from its stdin.
//! A line that does not parse is skipped, so either side may learn new fields first.
use serde::{Deserialize, Serialize};

/// The longest line either side accepts; anything longer is dropped with its connection.
pub const MAX_LINE: usize = 65536;

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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum Command {
    /// Set the default output's volume (cubic scale, clamped to 0.0 to 1.0) and unmute it.
    SetVolume { volume: f32 },
    ToggleMute,
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
        let line = encode(&status);
        assert!(line.ends_with('\n') && !line[..line.len() - 1].contains('\n'));
        assert_eq!(decode::<Status>(&line), Some(status));
        for command in [Command::SetVolume { volume: 0.25 }, Command::ToggleMute] {
            assert_eq!(decode::<Command>(&encode(&command)), Some(command));
        }
        assert_eq!(encode(&Command::ToggleMute), "{\"command\":\"toggle-mute\"}\n");
    }

    #[test]
    fn unknown_or_missing_parts_do_not_break_a_line() {
        assert_eq!(decode::<Status>("{}"), Some(Status::default()));
        let newer = r#"{"audio":null,"media":"x","tray":[1,2]}"#;
        assert_eq!(decode::<Status>(newer), Some(Status { media: Some("x".into()), ..Status::default() }));
        assert_eq!(decode::<Command>(r#"{"command":"launch-rockets"}"#), None);
        assert_eq!(decode::<Status>("not json"), None);
    }
}
