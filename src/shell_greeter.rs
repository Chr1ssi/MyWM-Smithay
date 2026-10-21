//! The built-in login screen (`mywm-compositor --greeter` without a command; as mywm-shell's `greeter.qml`): the
//! lock screen's card with a field for the user, the sessions to choose from and restart and shut down, shown
//! behind the compositor's own lock from the start, so nothing but it can be reached.
//!
//! A login runs `mywm-shell-services --login` in a process of its own (greetd's IPC, see `login.rs` there) and
//! follows what it tells line by line. Once greetd starts the session the compositor ends, as greetd waits for
//! the greeter to end before it starts the session.
//!
//! Environment (set by the NixOS module): `MYWM_GREETER_USER` (the user it starts with),
//! `MYWM_GREETER_BACKGROUND` (its image), `MYWM_GREETER_SESSIONS` (directory of the sessions' `.desktop` files).
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use mywm_shell_protocol::{LoginEvent, LoginRequest};
use mywm_ui::{edit::Input, lock_screen::LockMessage};
use smithay::reexports::calloop::channel;

use crate::State;

const DEFAULT_SESSIONS: &str = "/run/current-system/sw/share/wayland-sessions";

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub name: String,
    pub command: Vec<String>,
}

pub struct Greeter {
    pub user: Input,
    pub sessions: Vec<Session>,
    /// The session chosen.
    pub session: usize,
    pub background: Option<PathBuf>,
    /// greetd starts the session: the login screen only waits to end.
    started: bool,
    /// Where the logins tell how they go: the attempt and what happened (`None`: the login process ended).
    events: Option<channel::Sender<(u64, Option<LoginEvent>)>>,
}

/// The sessions of `.desktop` files `(file name, contents)`, sorted by file name: each one's first `Name=` and
/// `Exec=` line, without the field codes (`%U`) of the command, as greeter.qml read them.
fn sessions(mut files: Vec<(String, String)>) -> Vec<Session> {
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
        .into_iter()
        .filter_map(|(_, contents)| {
            let value = |key: &str| contents.lines().find_map(|line| line.strip_prefix(key)).map(str::trim);
            let name = value("Name=").filter(|n| !n.is_empty())?;
            let command: Vec<String> =
                value("Exec=")?.split_whitespace().filter(|part| !part.starts_with('%')).map(str::to_owned).collect();
            (!command.is_empty()).then(|| Session { name: name.to_owned(), command })
        })
        .collect()
}

fn read_sessions(directory: &Path) -> Vec<Session> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::error!("cannot read the sessions in {}: {error}", directory.display());
            return Vec::new();
        }
    };
    let files = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|e| e == "desktop"))
        .filter_map(|entry| Some((entry.file_name().to_string_lossy().into_owned(), std::fs::read_to_string(entry.path()).ok()?)))
        .collect();
    sessions(files)
}

/// Log in as `request` says in a helper process; tell each event, then `None` when it ended.
fn login(request: &LoginRequest, attempt: u64, events: &channel::Sender<(u64, Option<LoginEvent>)>) {
    let child = Command::new(crate::shell_services::program()).arg("--login").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            tracing::error!("cannot log in: {error}");
            return;
        }
    };
    let mut line = mywm_shell_protocol::encode(request);
    // Dropping stdin ends the request for the helper, which reads to the end.
    if let Some(mut stdin) = child.stdin.take()
        && let Err(error) = stdin.write_all(line.as_bytes())
    {
        tracing::warn!("cannot hand the login to its process: {error}");
    }
    crate::shell_lock::wipe(&mut line);
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            match mywm_shell_protocol::decode::<LoginEvent>(&line) {
                Some(event) => {
                    let _ = events.send((attempt, Some(event)));
                }
                None => tracing::warn!("the login process said something unknown: {line}"),
            }
        }
    }
    if let Err(error) = child.wait() {
        tracing::warn!("cannot wait for the login process: {error}");
    }
}

impl State {
    /// `--greeter` without a command: the shell draws the login screen (and nothing of a session).
    pub fn greeter_setup(&mut self) {
        let variable = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let directory = variable("MYWM_GREETER_SESSIONS").unwrap_or_else(|| DEFAULT_SESSIONS.to_owned());
        let sessions = read_sessions(Path::new(&directory));
        if sessions.is_empty() {
            tracing::error!("no sessions to start in {directory}");
        }
        self.shell.enabled = true;
        self.shell.lock.greeter = Some(Greeter {
            user: Input::new(&variable("MYWM_GREETER_USER").unwrap_or_default()),
            sessions,
            session: 0,
            background: variable("MYWM_GREETER_BACKGROUND").map(PathBuf::from),
            started: false,
            events: None,
        });
    }

    /// Show the login screen (from `shell_init`, once the shell is set up).
    pub fn greeter_start(&mut self) {
        let (sender, events) = channel::channel();
        match self.loop_handle.insert_source(events, |event, _, state: &mut State| {
            if let channel::Event::Msg((attempt, event)) = event {
                state.greeter_event(attempt, event);
            }
        }) {
            Ok(_) => {
                if let Some(greeter) = &mut self.shell.lock.greeter {
                    greeter.events = Some(sender);
                }
            }
            Err(error) => tracing::error!("nobody can log in: {}", error.error),
        }
        self.own_lock();
    }

    /// Log in with the user, password and session chosen, unless one is missing or a login runs.
    pub fn greeter_submit(&mut self) {
        let lock = &mut self.shell.lock;
        let Some(greeter) = &lock.greeter else { return };
        let Some(session) = greeter.sessions.get(greeter.session) else { return };
        if lock.checking.is_some() || greeter.user.text.is_empty() {
            return;
        }
        let request = LoginRequest { user: greeter.user.text.clone(), password: lock.password.clone(), command: session.command.clone() };
        let events = greeter.events.clone();
        lock.message = None;
        lock.failed = false;
        lock.attempts += 1;
        let attempt = lock.attempts;
        lock.checking = Some(attempt);
        self.shell.dirty = true;
        let Some(events) = events else {
            return self.greeter_failed(attempt, LockMessage::CannotSignIn("greetd cannot be asked".into()));
        };
        let started = std::thread::Builder::new().name("login".into()).spawn(move || {
            let mut request = request;
            login(&request, attempt, &events);
            crate::shell_lock::wipe(&mut request.password);
            let _ = events.send((attempt, None));
        });
        if let Err(error) = started {
            tracing::error!("cannot log in: {error}");
            self.greeter_failed(attempt, LockMessage::CannotSignIn(error.to_string()));
        }
    }

    /// What the login of `attempt` told (`None`: it ended), as greeter.qml followed greetd.
    fn greeter_event(&mut self, attempt: u64, event: Option<LoginEvent>) {
        let lock = &mut self.shell.lock;
        let Some(greeter) = lock.greeter.as_mut() else { return };
        if lock.checking != Some(attempt) || greeter.started {
            return;
        }
        match event {
            Some(LoginEvent::Message { text }) => lock.message = Some(LockMessage::Said(text)),
            Some(LoginEvent::Wrong) => return self.greeter_failed(attempt, LockMessage::WrongPassword),
            Some(LoginEvent::Error { description }) => return self.greeter_failed(attempt, LockMessage::CannotSignIn(description)),
            Some(LoginEvent::Launching) => {
                lock.message = Some(LockMessage::Starting);
                lock.failed = false;
            }
            Some(LoginEvent::Started) => {
                greeter.started = true;
                tracing::info!("greetd starts the session; the login screen ends");
                self.loop_signal.stop();
            }
            None => return self.greeter_failed(attempt, LockMessage::CannotSignIn("the login ended without an answer".into())),
        }
        self.shell.dirty = true;
    }

    fn greeter_failed(&mut self, attempt: u64, message: LockMessage) {
        let lock = &mut self.shell.lock;
        if lock.checking != Some(attempt) {
            return;
        }
        tracing::info!("login refused: {message:?}");
        lock.checking = None;
        lock.failed = true;
        crate::shell_lock::wipe(&mut lock.password);
        lock.message = Some(message);
        self.shell.dirty = true;
    }

    pub fn greeter_pick_session(&mut self, index: usize) {
        if let Some(greeter) = &mut self.shell.lock.greeter
            && index < greeter.sessions.len()
        {
            greeter.session = index;
            self.shell.dirty = true;
        }
    }

    /// Restart or shut down (`systemctl reboot|poweroff`), as the login screen's buttons did.
    pub fn greeter_power(&mut self, action: &str) {
        if self.shell.lock.greeter.is_none() {
            return;
        }
        let mut command = Command::new("systemctl");
        command.arg(action);
        Self::spawn_reaped(command, "systemctl");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_read_like_greeter_qml() {
        let files = vec![
            ("other.desktop".into(), "[Desktop Entry]\nName=Andere\nExec=/run/other-launch --flag %U\n".into()),
            ("mywm.desktop".into(), "[Desktop Entry]\nName=mywm\nName[de]=MyWM\nExec=/run/mywm-launch\nType=Application\n".into()),
            ("broken.desktop".into(), "[Desktop Entry]\nName=Ohne Befehl\n".into()),
        ];
        assert_eq!(sessions(files), vec![
            Session { name: "mywm".into(), command: vec!["/run/mywm-launch".into()] },
            Session { name: "Andere".into(), command: vec!["/run/other-launch".into(), "--flag".into()] },
        ]);
    }
}
