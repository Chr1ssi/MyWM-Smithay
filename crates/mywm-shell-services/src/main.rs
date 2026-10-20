//! `mywm-shell-services`: what the compositor's built-in shell shows of the system, found out in a process of its
//! own so that a slow or broken service (D-Bus, PipeWire) never holds up the compositor. The compositor starts
//! it with pipes on stdin and stdout (see `mywm-shell-protocol`) and starts it again when it ends.
//!
//! Every source runs on its own thread and reconnects on its own: `audio` (PipeWire's default output), `media`
//! (MPRIS players on the session bus), `bluetooth` (BlueZ on the system bus) and `notifications` (the
//! notification service). Volume commands run `wpctl`, which WirePlumber turns into the device's route volume as
//! the Quickshell panel's commands did.
mod audio;
mod bluetooth;
mod icons;
mod media;
mod notifications;

use std::{
    io::{BufRead, Write},
    process::Command as Process,
    sync::mpsc,
    time::Duration,
};

use mywm_shell_protocol::{Audio, Bluetooth, Command, Event, MAX_LINE, Notification, Status, decode, encode};
use tracing_subscriber::EnvFilter;

/// A source's news.
pub enum Update {
    Audio(Option<Audio>),
    Media(Option<String>),
    Bluetooth(Option<Bluetooth>),
    Notification(Box<Notification>),
    Closed(u32),
}

/// How long a source waits before it tries again after its service went away or never came.
pub const RETRY: Duration = Duration::from_secs(3);

fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let (updates, news) = mpsc::channel();
    for (name, run) in [("audio", audio::run as fn(mpsc::Sender<Update>)), ("media", media::run), ("bluetooth", bluetooth::run)] {
        let updates = updates.clone();
        if let Err(error) = std::thread::Builder::new().name(name.into()).spawn(move || run(updates)) {
            tracing::error!("cannot start the {name} source: {error}");
        }
    }
    let (commands, queue) = mpsc::channel();
    let started = std::thread::Builder::new().name("commands".into()).spawn(move || run_commands(queue));
    if let Err(error) = started {
        tracing::error!("cannot run commands: {error}");
    }
    let (notification_commands, notification_queue) = mpsc::channel();
    let notification_updates = updates.clone();
    let started = std::thread::Builder::new()
        .name("notifications".into())
        .spawn(move || notifications::run(notification_updates, notification_queue));
    if let Err(error) = started {
        tracing::error!("cannot start the notification service: {error}");
    }
    // The compositor closes stdin when it ends (or drops the pipe): so does this process.
    let read = std::thread::Builder::new().name("stdin".into()).spawn(move || {
        let stdin = std::io::stdin().lock();
        for line in stdin.split(b'\n') {
            let Ok(line) = line else { break };
            if line.len() > MAX_LINE {
                continue;
            }
            match std::str::from_utf8(&line).ok().and_then(decode::<Command>) {
                Some(command @ (Command::Dismiss { .. } | Command::Invoke { .. })) => {
                    let _ = notification_commands.send(command);
                }
                Some(command) => {
                    let _ = commands.send(command);
                }
                None => tracing::warn!("unknown command: {}", String::from_utf8_lossy(&line)),
            }
        }
        std::process::exit(0);
    });
    if let Err(error) = read {
        tracing::error!("cannot read commands: {error}");
        return;
    }

    let mut status = Status::default();
    let mut stdout = std::io::stdout().lock();
    let mut sent = None;
    let mut write = |event: &Event| stdout.write_all(encode(event).as_bytes()).and_then(|()| stdout.flush()).is_ok();
    loop {
        if sent.as_ref() != Some(&status) {
            if !write(&Event::Status(status.clone())) {
                // The compositor is gone.
                return;
            }
            sent = Some(status.clone());
        }
        let Ok(update) = news.recv() else { return };
        let event = match update {
            Update::Audio(audio) => {
                status.audio = audio;
                continue;
            }
            Update::Media(media) => {
                status.media = media;
                continue;
            }
            Update::Bluetooth(bluetooth) => {
                status.bluetooth = bluetooth;
                continue;
            }
            Update::Notification(notification) => Event::Notification(*notification),
            Update::Closed(id) => Event::Closed { id },
        };
        if !write(&event) {
            return;
        }
    }
}

/// Run the commands one after the other; of volume changes that queued up meanwhile only the last counts, so a
/// fast scroll does not start a `wpctl` per step.
fn run_commands(queue: mpsc::Receiver<Command>) {
    while let Ok(first) = queue.recv() {
        let mut batch = vec![first];
        batch.extend(queue.try_iter());
        for command in collapse(batch) {
            match command {
                Command::SetVolume { volume } => {
                    let volume = format!("{:.3}", volume.clamp(0.0, 1.0));
                    wpctl(&["set-volume", "@DEFAULT_AUDIO_SINK@", &volume]);
                    wpctl(&["set-mute", "@DEFAULT_AUDIO_SINK@", "0"]);
                }
                Command::ToggleMute => wpctl(&["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]),
                Command::Dismiss { .. } | Command::Invoke { .. } => {}
            }
        }
    }
}

/// `commands` with every run of volume changes reduced to its last one.
fn collapse(commands: Vec<Command>) -> Vec<Command> {
    let mut kept: Vec<Command> = Vec::with_capacity(commands.len());
    for command in commands {
        if matches!((kept.last(), &command), (Some(Command::SetVolume { .. }), Command::SetVolume { .. })) {
            kept.pop();
        }
        kept.push(command);
    }
    kept
}

fn wpctl(args: &[&str]) {
    match Process::new("wpctl").args(args).status() {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::warn!("wpctl {} failed ({status})", args.join(" ")),
        Err(error) => tracing::warn!("cannot run wpctl: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_last_of_several_volume_changes_runs() {
        let set = |volume| Command::SetVolume { volume };
        let batch = vec![set(0.1), set(0.2), Command::ToggleMute, set(0.3), set(0.4)];
        assert_eq!(collapse(batch), vec![set(0.2), Command::ToggleMute, set(0.4)]);
        assert_eq!(collapse(vec![Command::ToggleMute, Command::ToggleMute]), vec![Command::ToggleMute, Command::ToggleMute]);
    }
}
