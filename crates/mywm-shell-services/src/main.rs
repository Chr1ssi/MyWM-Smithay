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
mod tray;

use std::{
    io::{BufRead, Write},
    process::Command as Process,
    sync::mpsc,
    time::Duration,
};

use mywm_shell_protocol::{Bluetooth, Command, Event, MAX_LINE, MenuEntry, Notification, Player, Status, TrayItem, decode, encode};
use tracing_subscriber::EnvFilter;

/// A source's news.
pub enum Update {
    Audio(audio::Graph),
    Players(Vec<Player>),
    Tray(Vec<TrayItem>),
    TrayMenu(String, Box<MenuEntry>),
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
    for (name, run) in [("audio", audio::run as fn(mpsc::Sender<Update>)), ("media", media::run)] {
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
    let busy = bluetooth::BusyDevices::default();
    let (poke, pokes) = mpsc::channel();
    let (bluetooth_updates, bluetooth_busy, bluetooth_poke) = (updates.clone(), busy.clone(), poke.clone());
    let started = std::thread::Builder::new()
        .name("bluetooth".into())
        .spawn(move || bluetooth::run(bluetooth_updates, bluetooth_busy, bluetooth_poke, pokes));
    if let Err(error) = started {
        tracing::error!("cannot start the bluetooth source: {error}");
    }
    let (bluetooth_commands, bluetooth_queue) = mpsc::channel();
    let started = std::thread::Builder::new().name("bluetooth-commands".into()).spawn(move || bluetooth::run_commands(bluetooth_queue, busy, poke));
    if let Err(error) = started {
        tracing::error!("cannot control Bluetooth: {error}");
    }
    let (player_commands, player_queue) = mpsc::channel();
    let started = std::thread::Builder::new().name("player-commands".into()).spawn(move || media::run_commands(player_queue));
    if let Err(error) = started {
        tracing::error!("cannot control media players: {error}");
    }
    let paths = tray::Paths::default();
    let (tray_updates, tray_paths) = (updates.clone(), paths.clone());
    let started = std::thread::Builder::new().name("tray".into()).spawn(move || tray::run(tray_updates, tray_paths));
    if let Err(error) = started {
        tracing::error!("cannot start the tray: {error}");
    }
    let (tray_commands, tray_queue) = mpsc::channel();
    let menu_updates = updates.clone();
    let started = std::thread::Builder::new().name("tray-commands".into()).spawn(move || tray::run_commands(tray_queue, paths, menu_updates));
    if let Err(error) = started {
        tracing::error!("cannot control the tray: {error}");
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
                Some(Command::Player { bus, action }) => {
                    let _ = player_commands.send((bus, action));
                }
                Some(Command::BluetoothPower { on }) => {
                    let _ = bluetooth_commands.send(bluetooth::BluetoothCommand::Power(on));
                }
                Some(Command::BluetoothDiscover { on }) => {
                    let _ = bluetooth_commands.send(bluetooth::BluetoothCommand::Discover(on));
                }
                Some(Command::DeviceConnect { path }) => {
                    let _ = bluetooth_commands.send(bluetooth::BluetoothCommand::Connect(path));
                }
                Some(Command::DeviceDisconnect { path }) => {
                    let _ = bluetooth_commands.send(bluetooth::BluetoothCommand::Disconnect(path));
                }
                Some(Command::DevicePair { path }) => {
                    let _ = bluetooth_commands.send(bluetooth::BluetoothCommand::Pair(path));
                }
                Some(Command::TrayActivate { address }) => {
                    let _ = tray_commands.send(tray::TrayCommand::Activate(address));
                }
                Some(Command::TraySecondaryActivate { address }) => {
                    let _ = tray_commands.send(tray::TrayCommand::SecondaryActivate(address));
                }
                Some(Command::TrayScroll { address, delta, horizontal }) => {
                    let _ = tray_commands.send(tray::TrayCommand::Scroll(address, delta, horizontal));
                }
                Some(Command::TrayMenu { address }) => {
                    let _ = tray_commands.send(tray::TrayCommand::Menu(address));
                }
                Some(Command::TrayMenuShow { address, id }) => {
                    let _ = tray_commands.send(tray::TrayCommand::MenuShow(address, id));
                }
                Some(Command::TrayMenuClick { address, id }) => {
                    let _ = tray_commands.send(tray::TrayCommand::MenuClick(address, id));
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
            Update::Audio(graph) => {
                status.nodes = graph.nodes;
                status.default_sink = graph.default_sink;
                status.default_source = graph.default_source;
                continue;
            }
            Update::Players(players) => Event::Players { players },
            Update::Tray(items) => Event::Tray { items },
            Update::TrayMenu(address, menu) => Event::TrayMenu { address, menu: *menu },
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
                Command::SetVolume { id, volume } => {
                    let (id, volume) = (id.to_string(), format!("{:.3}", volume.clamp(0.0, 1.0)));
                    wpctl(&["set-volume", &id, &volume]);
                    wpctl(&["set-mute", &id, "0"]);
                }
                Command::ToggleMute { id } => wpctl(&["set-mute", &id.to_string(), "toggle"]),
                Command::SetDefault { id } => wpctl(&["set-default", &id.to_string()]),
                _ => {}
            }
        }
    }
}

/// `commands` with every run of volume changes of one node reduced to its last one.
fn collapse(commands: Vec<Command>) -> Vec<Command> {
    let mut kept: Vec<Command> = Vec::with_capacity(commands.len());
    for command in commands {
        if let (Some(Command::SetVolume { id: last, .. }), Command::SetVolume { id, .. }) = (kept.last(), &command)
            && last == id
        {
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
        let set = |id, volume| Command::SetVolume { id, volume };
        let mute = Command::ToggleMute { id: 1 };
        let batch = vec![set(1, 0.1), set(1, 0.2), mute.clone(), set(1, 0.3), set(2, 0.5), set(2, 0.4)];
        assert_eq!(collapse(batch), vec![set(1, 0.2), mute.clone(), set(1, 0.3), set(2, 0.4)]);
        assert_eq!(collapse(vec![mute.clone(), mute.clone()]), vec![mute.clone(), mute]);
    }
}
