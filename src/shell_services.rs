//! The built-in shell's services process (`mywm-shell-services`): started with the shell, read without ever
//! blocking, and started again (after a growing pause) whenever it ends. Until it reports, and while it is down,
//! the bar shows what the Quickshell bar showed without the services.
use std::{
    io::{self, Read, Write},
    os::{fd::AsRawFd, unix::process::CommandExt},
    path::PathBuf,
    process::{ChildStdin, ChildStdout, Command, Stdio},
    time::{Duration, Instant},
};

use mywm_shell_protocol::{MAX_LINE, Status, decode, encode};
use smithay::reexports::calloop::{
    Interest, Mode, PostAction,
    generic::Generic,
    timer::{TimeoutAction, Timer},
};

use crate::State;

pub const PROGRAM: &str = "mywm-shell-services";
/// Pauses before the next start after the process ended (the last one repeats).
const BACKOFF: [u64; 6] = [1, 2, 4, 8, 16, 30];
/// A process that ran this long counts as healthy: the next restart waits the shortest pause again.
const HEALTHY: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct Services {
    pub status: Status,
    stdin: Option<ChildStdin>,
    input: Vec<u8>,
    started: Option<Instant>,
    failures: usize,
    /// Tells this process apart from later ones, for its event source.
    generation: u64,
}

/// The program next to the compositor's own binary (installed side by side), else from `PATH`.
fn program() -> PathBuf {
    mywm_config::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(PROGRAM)))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from(PROGRAM))
}

fn nonblocking(fd: &impl AsRawFd) -> io::Result<()> {
    // SAFETY: plain fcntl calls on a descriptor the caller owns.
    unsafe {
        let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Let `command`'s process end with the compositor, even if the compositor crashes.
pub fn end_with_parent(command: &mut Command) {
    // SAFETY: prctl is async-signal-safe and only touches the new process.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
}

impl State {
    pub fn services_start(&mut self) {
        let services = &mut self.shell.services;
        services.generation += 1;
        services.input.clear();
        let generation = services.generation;
        let mut command = Command::new(program());
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        end_with_parent(&mut command);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                tracing::warn!("cannot start {PROGRAM}: {error}");
                return self.services_ended();
            }
        };
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else { return self.services_ended() };
        if let Err(error) = nonblocking(&stdin).and_then(|()| nonblocking(&stdout)) {
            tracing::warn!("cannot read {PROGRAM}: {error}");
            return self.services_ended();
        }
        // The process ends when its pipes close; only its exit status is left to collect.
        let reaped = std::thread::Builder::new().name("shell-services-reaper".into()).spawn(move || {
            if let Ok(status) = child.wait() {
                tracing::info!("{PROGRAM} ended ({status})");
            }
        });
        if let Err(error) = reaped {
            tracing::warn!("cannot watch {PROGRAM}: {error}");
        }
        let source = Generic::new(stdout, Interest::READ, Mode::Level);
        let registered = self.loop_handle.insert_source(source, move |_, stdout, state: &mut State| {
            if state.shell.services.generation != generation {
                return Ok(PostAction::Remove);
            }
            // SAFETY: the pipe is only read here and never dropped while the source lives.
            Ok(state.services_readable(unsafe { stdout.get_mut() }))
        });
        if let Err(error) = registered {
            tracing::warn!("cannot read {PROGRAM}: {}", error.error);
            return self.services_ended();
        }
        let services = &mut self.shell.services;
        services.stdin = Some(stdin);
        services.started = Some(Instant::now());
    }

    fn services_readable(&mut self, stdout: &mut ChildStdout) -> PostAction {
        let mut buffer = [0; 4096];
        // Bounded per wakeup; the level-triggered source comes back for the rest.
        for _ in 0..16 {
            match stdout.read(&mut buffer) {
                Ok(0) => {
                    self.services_ended();
                    return PostAction::Remove;
                }
                Ok(n) => self.shell.services.input.extend_from_slice(&buffer[..n]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    tracing::warn!("cannot read {PROGRAM}: {error}");
                    self.services_ended();
                    return PostAction::Remove;
                }
            }
        }
        let services = &mut self.shell.services;
        // Only the newest complete line matters: every line is the whole status.
        let Some(end) = services.input.iter().rposition(|b| *b == b'\n') else {
            if services.input.len() > MAX_LINE {
                tracing::warn!("{PROGRAM} wrote an overlong line; starting it again");
                self.services_ended();
                return PostAction::Remove;
            }
            return PostAction::Continue;
        };
        let lines: Vec<u8> = services.input.drain(..=end).collect();
        let newest = lines[..lines.len() - 1].rsplit(|b| *b == b'\n').next().unwrap_or_default();
        match std::str::from_utf8(newest).ok().and_then(decode::<Status>) {
            Some(status) if status != services.status => {
                tracing::debug!("shell services: {status:?}");
                services.status = status;
                self.shell_changed();
            }
            Some(_) => {}
            None => tracing::warn!("{PROGRAM} wrote what is no status: {}", String::from_utf8_lossy(newest)),
        }
        PostAction::Continue
    }

    /// The process ended or cannot be used: forget what it said and start it again after a pause.
    fn services_ended(&mut self) {
        let services = &mut self.shell.services;
        services.stdin = None;
        services.generation += 1;
        if services.started.take().is_some_and(|at| at.elapsed() >= HEALTHY) {
            services.failures = 0;
        }
        let pause = Duration::from_secs(BACKOFF[services.failures.min(BACKOFF.len() - 1)]);
        services.failures += 1;
        if services.status != Status::default() {
            services.status = Status::default();
            self.shell_changed();
        }
        let restart = self.loop_handle.insert_source(Timer::from_duration(pause), |_, _, state| {
            state.services_start();
            TimeoutAction::Drop
        });
        if let Err(error) = restart {
            tracing::error!("cannot start {PROGRAM} again: {}", error.error);
        }
    }

    /// Send `command`; dropped if the process is down or does not keep up (it never blocks the compositor).
    pub fn services_send(&mut self, command: mywm_shell_protocol::Command) {
        let Some(stdin) = &mut self.shell.services.stdin else { return };
        match stdin.write(encode(&command).as_bytes()) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => tracing::warn!("{PROGRAM} does not keep up; dropped {command:?}"),
            Err(error) => {
                tracing::warn!("cannot reach {PROGRAM}: {error}");
                self.shell.services.stdin = None;
            }
        }
    }
}
