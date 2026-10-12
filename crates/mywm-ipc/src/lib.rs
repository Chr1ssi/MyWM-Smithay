//! The `v1` bar protocol between the compositor and `mywm-shell`. Lines of text over a
//! Unix socket (`$MYWM_SOCKET`):
//!
//! * client → compositor: `v1 <command>\n`, answered by `v1 ok` or `v1 error invalid-command`
//! * compositor → client, whenever something changed:
//!   `v1 state <output>;<output>...`, `v1 scratchpad <visible> <occupied>`, `v1 locked <0|1>`,
//!   `v1 gaming <0|1>` (a game is shown fullscreen on some monitor), `v1 config ok` or
//!   `v1 config error <reason>` (the configuration file did not load)
//! * `v1 inhibit-idle` keeps the session from going idle until the sender's connection closes
//!   (mywm-portal, for apps that inhibit through the portal)
//! * compositor → the client that sent `handle-logout`, when the exit binding is pressed:
//!   `v1 confirm-logout <output id>` (it asks the user and answers with `logout`)
//!
//! An output is `id,x,y,width,height,active,left,right,number:occupied|number:occupied...`.
//! Workspace number 0 is the gaming workspace; `left`/`right` tell whether tiled
//! columns extend beyond the visible area.
use mywm_layout::Rect;

/// What a screen-sharing chooser may pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKinds {
    Monitor,
    Window,
    Both,
}

impl SourceKinds {
    pub fn monitors(self) -> bool {
        self != Self::Window
    }

    pub fn windows(self) -> bool {
        self != Self::Monitor
    }
}

/// The answer to `choose-source`, sent later as `v1 chosen ...`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chosen {
    Monitor(String),
    Window(String),
    Nothing,
}

impl Chosen {
    pub fn encode(&self) -> String {
        match self {
            Chosen::Monitor(name) => format!("v1 chosen monitor {name}\n"),
            Chosen::Window(id) => format!("v1 chosen window {id}\n"),
            Chosen::Nothing => "v1 chosen none\n".into(),
        }
    }

    pub fn parse(line: &str) -> Option<Self> {
        let args: Vec<_> = line.split_whitespace().collect();
        match args.as_slice() {
            ["v1", "chosen", "monitor", name] => Some(Chosen::Monitor((*name).into())),
            ["v1", "chosen", "window", id] => Some(Chosen::Window((*id).into())),
            ["v1", "chosen", "none"] => Some(Chosen::Nothing),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Ask the user what to share; answered by `v1 ok` and later `v1 chosen ...`.
    ChooseSource(SourceKinds),
    Lock,
    /// End the session now.
    Logout,
    /// The sender confirms logouts: the exit binding asks it with `v1 confirm-logout <output>`
    /// instead of ending the session (which it still does while no such client is connected).
    HandleLogout,
    /// Keep the session from going idle while the sender stays connected.
    InhibitIdle,
    ThemeReload,
    Scratchpad,
    NewWorkspace { output: u32 },
    Workspace { output: u32, number: usize },
}

/// Parse the syntax of a command line; whether the output or workspace exists is up to the caller.
pub fn parse_command(line: &str) -> Option<Command> {
    let args: Vec<_> = line.split_whitespace().collect();
    match args.as_slice() {
        ["v1", "lock"] => Some(Command::Lock),
        ["v1", "choose-source", kinds] => Some(Command::ChooseSource(match *kinds {
            "monitor" => SourceKinds::Monitor,
            "window" => SourceKinds::Window,
            "both" => SourceKinds::Both,
            _ => return None,
        })),
        ["v1", "logout"] => Some(Command::Logout),
        ["v1", "handle-logout"] => Some(Command::HandleLogout),
        ["v1", "inhibit-idle"] => Some(Command::InhibitIdle),
        ["v1", "theme-reload"] => Some(Command::ThemeReload),
        ["v1", "scratchpad"] => Some(Command::Scratchpad),
        ["v1", "new-workspace", output] => Some(Command::NewWorkspace { output: output.parse().ok()? }),
        ["v1", "workspace", output, number] => Some(Command::Workspace {
            output: output.parse().ok()?,
            number: number.parse().ok()?,
        }),
        _ => None,
    }
}

/// Ask the `handle-logout` client to confirm a logout on the output with this id.
pub fn confirm_logout(output: u32) -> String {
    format!("v1 confirm-logout {output}\n")
}

pub const OK: &str = "v1 ok\n";
pub const INVALID: &str = "v1 error invalid-command\n";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputState {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub active: usize,
    pub overflow_left: bool,
    pub overflow_right: bool,
    /// `(number, occupied)` in display order.
    pub workspaces: Vec<(usize, bool)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub outputs: Vec<OutputState>,
    pub scratchpad_visible: bool,
    pub scratchpad_occupied: bool,
    pub locked: bool,
    pub gaming: bool,
    /// Why the configuration file did not load (one line), if it did not.
    pub config_error: Option<String>,
}

impl Snapshot {
    pub fn encode(&self) -> String {
        let outputs: Vec<_> = self
            .outputs
            .iter()
            .map(|o| {
                let workspaces: Vec<_> =
                    o.workspaces.iter().map(|(n, occupied)| format!("{n}:{}", u8::from(*occupied))).collect();
                format!(
                    "{},{},{},{},{},{},{},{},{}",
                    o.id,
                    o.x,
                    o.y,
                    o.width,
                    o.height,
                    o.active,
                    u8::from(o.overflow_left),
                    u8::from(o.overflow_right),
                    workspaces.join("|")
                )
            })
            .collect();
        format!(
            "v1 state {}\nv1 scratchpad {} {}\nv1 locked {}\nv1 gaming {}\nv1 config {}\n",
            outputs.join(";"),
            u8::from(self.scratchpad_visible),
            u8::from(self.scratchpad_occupied),
            u8::from(self.locked),
            u8::from(self.gaming),
            // A reason with a line break would break the protocol's lines.
            self.config_error.as_deref().map_or("ok".into(), |reason| format!("error {}", reason.replace('\n', " ")))
        )
    }

    /// Inverse of `encode`, as a client (the bar, tests) reads it.
    pub fn decode(text: &str) -> Option<Self> {
        let mut snapshot = Self::default();
        for line in text.lines() {
            let parts: Vec<_> = line.split(' ').collect();
            match parts.as_slice() {
                ["v1", "state", rest @ ..] => {
                    for entry in rest.first().copied().unwrap_or("").split(';').filter(|s| !s.is_empty()) {
                        let f: Vec<_> = entry.split(',').collect();
                        if f.len() != 9 {
                            return None;
                        }
                        let workspaces = f[8]
                            .split('|')
                            .filter(|s| !s.is_empty())
                            .map(|w| {
                                let (n, o) = w.split_once(':')?;
                                Some((n.parse().ok()?, o == "1"))
                            })
                            .collect::<Option<_>>()?;
                        snapshot.outputs.push(OutputState {
                            id: f[0].parse().ok()?,
                            x: f[1].parse().ok()?,
                            y: f[2].parse().ok()?,
                            width: f[3].parse().ok()?,
                            height: f[4].parse().ok()?,
                            active: f[5].parse().ok()?,
                            overflow_left: f[6] == "1",
                            overflow_right: f[7] == "1",
                            workspaces,
                        });
                    }
                }
                ["v1", "scratchpad", visible, occupied] => {
                    snapshot.scratchpad_visible = *visible == "1";
                    snapshot.scratchpad_occupied = *occupied == "1";
                }
                ["v1", "locked", locked] => snapshot.locked = *locked == "1",
                ["v1", "gaming", gaming] => snapshot.gaming = *gaming == "1",
                ["v1", "config", "ok"] => snapshot.config_error = None,
                ["v1", "config", "error", reason @ ..] => snapshot.config_error = Some(reason.join(" ")),
                _ => return None,
            }
        }
        Some(snapshot)
    }
}

/// Whether any window lies beyond the left or right edge of `area`.
pub fn overflow_directions(area: Rect, geometries: impl IntoIterator<Item = Rect>) -> (bool, bool) {
    let (mut left, mut right) = (false, false);
    for g in geometries {
        left |= g.x < area.x;
        right |= g.x + g.width > area.x + area.width;
    }
    (left, right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, width: i32) -> Rect {
        Rect { x, y: 6, width, height: 1068 }
    }

    #[test]
    fn markers_only_report_windows_outside_the_visible_area() {
        let area = Rect { x: 4, y: 4, width: 1912, height: 1072 };
        assert_eq!(overflow_directions(area, [rect(6, 952), rect(962, 952)]), (false, false));
        assert_eq!(overflow_directions(area, [rect(-950, 952), rect(6, 952)]), (true, false));
        assert_eq!(overflow_directions(area, [rect(962, 952), rect(1918, 952)]), (false, true));
    }

    #[test]
    fn commands_parse() {
        assert_eq!(parse_command("v1 lock\n"), Some(Command::Lock));
        assert_eq!(parse_command("v1 logout"), Some(Command::Logout));
        assert_eq!(parse_command("v1 handle-logout\n"), Some(Command::HandleLogout));
        assert_eq!(parse_command("v1 inhibit-idle\n"), Some(Command::InhibitIdle));
        assert_eq!(confirm_logout(7), "v1 confirm-logout 7\n");
        assert_eq!(parse_command("v1 theme-reload"), Some(Command::ThemeReload));
        assert_eq!(parse_command("v1 choose-source both"), Some(Command::ChooseSource(SourceKinds::Both)));
        assert_eq!(parse_command("v1 choose-source tab"), None);
        for chosen in [Chosen::Monitor("DP-3".into()), Chosen::Window("abc123".into()), Chosen::Nothing] {
            assert_eq!(Chosen::parse(&chosen.encode()), Some(chosen));
        }
        assert_eq!(parse_command("v1 scratchpad"), Some(Command::Scratchpad));
        assert_eq!(parse_command("v1 new-workspace 7"), Some(Command::NewWorkspace { output: 7 }));
        assert_eq!(
            parse_command("v1  workspace 7 3"),
            Some(Command::Workspace { output: 7, number: 3 })
        );
        for bad in ["", "lock", "v2 lock", "v1 workspace 7", "v1 workspace x 3", "v1 workspace 7 -1", "v1 lock now"] {
            assert_eq!(parse_command(bad), None, "accepted {bad:?}");
        }
    }

    #[test]
    fn snapshot_matches_the_wire_format_the_bar_parses() {
        let snapshot = Snapshot {
            outputs: vec![OutputState {
                id: 1,
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                active: 1,
                overflow_left: false,
                overflow_right: true,
                workspaces: vec![(1, true), (3, false), (0, true)],
            }],
            scratchpad_visible: true,
            scratchpad_occupied: true,
            locked: false,
            gaming: true,
            config_error: None,
        };
        let text = snapshot.encode();
        assert_eq!(
            text,
            "v1 state 1,0,0,1920,1080,1,0,1,1:1|3:0|0:1\nv1 scratchpad 1 1\nv1 locked 0\nv1 gaming 1\nv1 config ok\n"
        );
        assert_eq!(Snapshot::decode(&text), Some(snapshot));
        let broken = Snapshot { config_error: Some("line 3 · expected a number".into()), ..Snapshot::default() };
        assert!(broken.encode().ends_with("v1 config error line 3 · expected a number\n"));
        assert_eq!(Snapshot::decode(&broken.encode()), Some(broken));
        assert_eq!(Snapshot::decode("v1 state \nv1 scratchpad 0 0\nv1 locked 0\n"), Some(Snapshot::default()));
        assert_eq!(Snapshot::decode("v1 state 1,2\n"), None);
    }
}
