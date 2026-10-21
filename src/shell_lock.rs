//! The built-in shell's lock screen (as mywm-shell's `lock.qml`): the compositor locks the session itself
//! (`own_lock` in `session`), so no client draws, reaches or ends the lock, and only a password that PAM
//! accepts unlocks it.
//!
//! Each check runs `mywm-shell-services --authenticate` in a process of its own, the password on its stdin:
//! a PAM module that hangs or crashes ends that attempt, never the lock. The session unlocks only when that
//! process exits cleanly and answers `success` for the attempt still being checked.
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
};

use mywm_shell_protocol::AuthOutcome;
use mywm_ui::lock_screen::LockMessage;
use smithay::{
    backend::renderer::{
        element::{Id, Kind, solid::SolidColorRenderElement, utils::CropRenderElement},
        utils::CommitCounter,
    },
    input::keyboard::{Keysym, keysyms},
    output::Output,
    reexports::calloop::channel,
    utils::{Physical, Rectangle},
};

use crate::{State, render::OutputElement};

/// Room for a long password, so typing does not move it (leaving copies behind in freed memory).
const PASSWORD_CAPACITY: usize = 256;

#[derive(Default)]
pub struct LockScreen {
    /// What was typed; zeroed before it goes.
    password: String,
    /// The attempt being checked; keys wait meanwhile, as the disabled field did.
    checking: Option<u64>,
    attempts: u64,
    /// What the last check said; nothing before the first one and while checking.
    pub message: Option<LockMessage>,
    /// Where the checks answer: the attempt and its outcome, `None` if the check itself failed.
    results: Option<channel::Sender<(u64, Option<AuthOutcome>)>>,
    /// The backdrop's id and color, and a count that changes with the color, for damage tracking.
    backdrop: Option<(Id, [f32; 4], CommitCounter)>,
}

/// Overwrite `text` before it is emptied, so the password does not stay in memory.
fn wipe(text: &mut String) {
    // SAFETY: zero bytes are valid UTF-8.
    unsafe { text.as_mut_vec() }.fill(0);
    text.clear();
}

impl Drop for LockScreen {
    fn drop(&mut self) {
        wipe(&mut self.password);
    }
}

/// Check `password` in a helper process (see the module); its bytes are zeroed once it is written.
fn check(mut password: Vec<u8>) -> Option<AuthOutcome> {
    let child = Command::new(crate::shell_services::program())
        .arg("--authenticate")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            password.fill(0);
            tracing::error!("cannot check the password: {error}");
            return None;
        }
    };
    // Dropping stdin ends the password for the helper, which reads to the end.
    if let Some(mut stdin) = child.stdin.take()
        && let Err(error) = stdin.write_all(&password)
    {
        tracing::warn!("cannot hand the password to its check: {error}");
    }
    password.fill(0);
    let mut answer = String::new();
    if let Some(mut stdout) = child.stdout.take()
        && let Err(error) = stdout.read_to_string(&mut answer)
    {
        tracing::warn!("cannot read what the password check said: {error}");
    }
    match child.wait() {
        Ok(status) if status.success() => AuthOutcome::parse(&answer),
        Ok(status) => {
            tracing::error!("the password check failed ({status})");
            None
        }
        Err(error) => {
            tracing::error!("cannot wait for the password check: {error}");
            None
        }
    }
}

impl State {
    /// Set up where the password checks answer (the built-in shell only).
    pub fn shell_lock_init(&mut self) {
        let (sender, results) = channel::channel();
        match self.loop_handle.insert_source(results, |event, _, state: &mut State| {
            if let channel::Event::Msg((attempt, outcome)) = event {
                state.lock_checked(attempt, outcome);
            }
        }) {
            Ok(_) => self.shell.lock.results = Some(sender),
            Err(error) => tracing::error!("passwords cannot be checked; the lock screen will not unlock: {}", error.error),
        }
    }

    /// The session was just locked with the built-in lock screen: it starts empty.
    pub fn shell_lock_started(&mut self) {
        let lock = &mut self.shell.lock;
        wipe(&mut lock.password);
        lock.password.reserve(PASSWORD_CAPACITY);
        // An attempt of an earlier lock answers too late to count.
        lock.checking = None;
        lock.message = None;
        self.shell_release_pointer();
        self.shell.dirty = true;
    }

    /// A key pressed on the lock screen: typing, Backspace and Return; nothing while a check runs.
    pub fn lock_key(&mut self, sym: Keysym, modified: bool) {
        let lock = &mut self.shell.lock;
        if lock.checking.is_some() {
            return;
        }
        match sym.raw() {
            keysyms::KEY_Return | keysyms::KEY_KP_Enter => return self.lock_submit(),
            keysyms::KEY_BackSpace => {
                lock.password.pop();
            }
            _ => match sym.key_char().filter(|c| !c.is_control() && !modified) {
                Some(c) => lock.password.push(c),
                None => return,
            },
        }
        self.shell.dirty = true;
    }

    /// Check the typed password, unless there is none or a check runs.
    fn lock_submit(&mut self) {
        let lock = &mut self.shell.lock;
        if lock.checking.is_some() || lock.password.is_empty() {
            return;
        }
        lock.message = None;
        lock.attempts += 1;
        let attempt = lock.attempts;
        lock.checking = Some(attempt);
        let Some(results) = lock.results.clone() else {
            return self.lock_checked(attempt, None);
        };
        let password = lock.password.as_bytes().to_vec();
        let started = std::thread::Builder::new().name("password-check".into()).spawn(move || {
            let _ = results.send((attempt, check(password)));
        });
        if let Err(error) = started {
            tracing::error!("cannot check the password: {error}");
            self.lock_checked(attempt, None);
        }
        self.shell.dirty = true;
    }

    /// A check answered: only `success` for the current attempt, while the lock screen holds the session,
    /// unlocks. Like lock.qml shows it, every other outcome but too many tries reads as a wrong password.
    fn lock_checked(&mut self, attempt: u64, outcome: Option<AuthOutcome>) {
        let lock = &mut self.shell.lock;
        // An answer for an earlier attempt or an earlier lock counts for nothing.
        if lock.checking != Some(attempt) || !self.session_lock.is_own() {
            return;
        }
        lock.checking = None;
        wipe(&mut lock.password);
        if outcome == Some(AuthOutcome::Success) {
            return self.own_unlock();
        }
        lock.message = Some(match outcome {
            Some(AuthOutcome::MaxTries) => LockMessage::TooManyTries,
            _ => LockMessage::WrongPassword,
        });
        tracing::info!("the password was not accepted ({})", outcome.map_or("check failed", AuthOutcome::as_str));
        self.shell.dirty = true;
    }

    /// What the lock screen shows on `output`: for now the theme's background over all of it, so nothing
    /// behind the lock shows through.
    pub fn shell_lock_elements(&mut self, output: &Output) -> Vec<OutputElement> {
        let Some(geo) = self.space.output_geometry(output) else { return Vec::new() };
        let scale = output.current_scale().fractional_scale();
        let area: Rectangle<i32, Physical> = geo.to_physical_precise_round(scale);
        let area = Rectangle::from_size(area.size);
        let theme = self.shell.theme.background;
        // Opaque whatever the theme says.
        let color = [f32::from(theme.r) / 255.0, f32::from(theme.g) / 255.0, f32::from(theme.b) / 255.0, 1.0];
        // A stable id: the backdrop is damaged only when its color changes.
        let (id, shown, commit) = self.shell.lock.backdrop.get_or_insert_with(|| (Id::new(), color, CommitCounter::default()));
        if *shown != color {
            *shown = color;
            commit.increment();
        }
        let element = SolidColorRenderElement::new(id.clone(), area, *commit, color, Kind::Unspecified);
        CropRenderElement::from_element(element, scale, area).map(OutputElement::from).into_iter().collect()
    }
}
