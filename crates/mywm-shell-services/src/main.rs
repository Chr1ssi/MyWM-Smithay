//! `mywm-shell-services`: what the compositor's built-in shell shows of the system, found out in a process of its
//! own so that a slow or broken service (D-Bus, PipeWire) never holds up the compositor. The compositor starts
//! it with pipes on stdin and stdout (see `mywm-shell-protocol`) and starts it again when it ends.
//!
//! Every source runs on its own thread and reconnects on its own: `audio` (PipeWire's default output), `media`
//! (MPRIS players on the session bus) and `bluetooth` (BlueZ on the system bus). Commands run `wpctl`, which
//! WirePlumber turns into the device's route volume as the Quickshell panel's commands did.
mod audio;
mod bluetooth;
mod media;

use std::{
    io::{BufRead, Write},
    process::Command as Process,
    sync::mpsc,
    time::Duration,
};

use mywm_shell_protocol::{Audio, Bluetooth, Command, MAX_LINE, Status, decode, encode};
use tracing_subscriber::EnvFilter;

/// A source's news.
pub enum Update {
    Audio(Option<Audio>),
    Media(Option<String>),
    Bluetooth(Option<Bluetooth>),
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
    // The compositor closes stdin when it ends (or drops the pipe): so does this process.
    let read = std::thread::Builder::new().name("stdin".into()).spawn(move || {
        let stdin = std::io::stdin().lock();
        for line in stdin.split(b'\n') {
            let Ok(line) = line else { break };
            if line.len() > MAX_LINE {
                continue;
            }
            match std::str::from_utf8(&line).ok().and_then(decode::<Command>) {
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
    loop {
        if sent.as_ref() != Some(&status) {
            if stdout.write_all(encode(&status).as_bytes()).and_then(|()| stdout.flush()).is_err() {
                // The compositor is gone.
                return;
            }
            sent = Some(status.clone());
        }
        let Ok(update) = news.recv() else { return };
        match update {
            Update::Audio(audio) => status.audio = audio,
            Update::Media(media) => status.media = media,
            Update::Bluetooth(bluetooth) => status.bluetooth = bluetooth,
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
            }
        }
    }
}

/// `commands` with every run of volume changes reduced to its last one.
fn collapse(commands: Vec<Command>) -> Vec<Command> {
    let mut kept: Vec<Command> = Vec::with_capacity(commands.len());
    for command in commands {
        match (kept.last_mut(), command) {
            (Some(last @ Command::SetVolume { .. }), Command::SetVolume { .. }) => *last = command,
            _ => kept.push(command),
        }
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
        assert_eq!(collapse(vec![Command::ToggleMute, Command::ToggleMute]), vec![Command::ToggleMute; 2]);
    }
}
