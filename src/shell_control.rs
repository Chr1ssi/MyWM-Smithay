//! The command center's own parts in the built-in shell: the night light (gammastep keeps the warm screen as long
//! as it runs, the choice is kept across logins in `$XDG_STATE_HOME/mywm/night-light`), the local IPv4 addresses
//! (asked of `ip` every five seconds while the command center is open, as the Quickshell panel did) and copying one
//! of them, what its clicks do, and the way to the settings app.
use std::time::Duration;

use mywm_shell_protocol::{Bluetooth, Command};
use mywm_ui::{
    bar::Panel,
    control::{AdapterView, ControlModel, ControlTarget, DeviceBusy, DeviceView},
};
use smithay::reexports::calloop::{
    channel,
    timer::{TimeoutAction, Timer},
};

use crate::State;

/// The settings editor (`mywm-settings`), found on `PATH` like the other helpers.
const SETTINGS: &str = "mywm-settings";
/// How long "Kopiert" stands instead of the address.
const COPIED: Duration = Duration::from_millis(1500);
const ADDRESSES_EVERY: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Control {
    pub night_light: bool,
    /// The running gammastep and which start it was (an exit of an earlier one is no failure).
    gammastep: Option<u32>,
    starts: u64,
    /// Device and address, as `ip` said last.
    addresses: Vec<(String, String)>,
    copied: Option<String>,
    copies: u64,
    /// Tells the address timer of this opening of the command center apart from earlier ones.
    asking: u64,
}

fn night_light_file() -> Option<std::path::PathBuf> {
    Some(mywm_config::state_dir()?.join("night-light"))
}

/// `ip -4 -o addr show scope global`: "2: eno1    inet 192.168.178.56/24 brd … scope global …".
pub fn parse_addresses(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            (fields.len() > 3 && fields[2] == "inet").then(|| (fields[1].to_owned(), fields[3].split('/').next().unwrap_or_default().to_owned()))
        })
        .collect()
}

impl State {
    /// The night light as chosen last.
    pub fn night_light_restore(&mut self) {
        let chosen = night_light_file().and_then(|file| std::fs::read_to_string(file).ok()).is_some_and(|text| text.trim() == "1");
        if chosen {
            self.shell.control.night_light = true;
            self.gammastep_start();
        }
    }

    /// The night light tile: switch, and keep the choice for the next login.
    fn night_light_set(&mut self, on: bool) {
        self.shell.control.night_light = on;
        if let Some(file) = night_light_file() {
            let written = file.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&file, if on { "1\n" } else { "0\n" }));
            if let Err(error) = written {
                tracing::warn!("night light: cannot keep the choice in {}: {error}", file.display());
            }
        }
        if on {
            self.gammastep_start();
        } else {
            self.gammastep_stop();
        }
        self.shell_changed();
    }

    fn gammastep_start(&mut self) {
        self.gammastep_stop();
        let control = &mut self.shell.control;
        control.starts += 1;
        let start = control.starts;
        // One-shot mode keeps running on Wayland: the warm ramp lasts as long as the process.
        let mut command = std::process::Command::new("gammastep");
        command.args(["-m", "wayland", "-O", "4000"]).env("WAYLAND_DISPLAY", &self.socket_name);
        crate::shell_services::end_with_parent(&mut command);
        let (sender, ended) = channel::channel();
        let watched = self.loop_handle.insert_source(ended, move |event, _, state: &mut State| {
            let control = &mut state.shell.control;
            if let channel::Event::Msg(code) = event
                && control.starts == start
                && control.night_light
            {
                // Not switched off: it failed (no gamma control). The saved choice stays for the next login.
                tracing::warn!("night light: gammastep exited with {code}");
                control.night_light = false;
                control.gammastep = None;
                state.shell_changed();
            }
        });
        let started = watched.is_ok().then(|| command.spawn());
        match started {
            Some(Ok(mut child)) => {
                self.shell.control.gammastep = Some(child.id());
                std::thread::spawn(move || {
                    let code = child.wait().map_or(-1, |status| status.code().unwrap_or(-1));
                    let _ = sender.send(code);
                });
            }
            Some(Err(error)) => {
                tracing::warn!("night light: cannot start gammastep: {error}");
                self.shell.control.night_light = false;
            }
            None => self.shell.control.night_light = false,
        }
    }

    fn gammastep_stop(&mut self) {
        let control = &mut self.shell.control;
        // A later start (or none) makes this one's end expected.
        control.starts += 1;
        if let Some(pid) = control.gammastep.take().and_then(|pid| i32::try_from(pid).ok()) {
            // SAFETY: a plain signal to the child this process started.
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
    }

    /// The command center opened: ask for the addresses now and every few seconds while it is open.
    pub fn addresses_follow(&mut self) {
        let control = &mut self.shell.control;
        control.asking += 1;
        let asking = control.asking;
        let (sender, answers) = channel::channel();
        let answered = self.loop_handle.insert_source(answers, |event, _, state: &mut State| {
            if let channel::Event::Msg(addresses) = event
                && addresses != state.shell.control.addresses
            {
                // Unchanged addresses keep their rows.
                state.shell.control.addresses = addresses;
                state.shell_changed();
            }
        });
        if let Err(error) = answered {
            return tracing::warn!("cannot follow the addresses: {}", error.error);
        }
        let ask = move || {
            let sender = sender.clone();
            std::thread::spawn(move || {
                let output = std::process::Command::new("ip").args(["-4", "-o", "addr", "show", "scope", "global"]).output();
                match output {
                    Ok(output) => {
                        let _ = sender.send(parse_addresses(&String::from_utf8_lossy(&output.stdout)));
                    }
                    Err(error) => tracing::warn!("cannot ask ip for the addresses: {error}"),
                }
            });
        };
        ask();
        let timer = self.loop_handle.insert_source(Timer::from_duration(ADDRESSES_EVERY), move |_, _, state: &mut State| {
            let open = state.shell.panel.as_ref().is_some_and(|p| p.kind == Panel::Control);
            if state.shell.control.asking != asking || !open {
                return TimeoutAction::Drop;
            }
            ask();
            TimeoutAction::ToDuration(ADDRESSES_EVERY)
        });
        if let Err(error) = timer {
            tracing::warn!("cannot follow the addresses: {}", error.error);
        }
    }

    fn address_copy(&mut self, address: String) {
        let mut command = std::process::Command::new("wl-copy");
        command.arg(&address).env("WAYLAND_DISPLAY", &self.socket_name);
        match command.spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(error) => return tracing::warn!("cannot copy the address: {error}"),
        }
        let control = &mut self.shell.control;
        control.copied = Some(address);
        control.copies += 1;
        let copy = control.copies;
        let reset = self.loop_handle.insert_source(Timer::from_duration(COPIED), move |_, _, state: &mut State| {
            if state.shell.control.copies == copy {
                state.shell.control.copied = None;
                state.shell_changed();
            }
            TimeoutAction::Drop
        });
        if let Err(error) = reset {
            tracing::warn!("cannot time the copy note: {}", error.error);
        }
        self.shell_changed();
    }

    pub fn control_clicked(&mut self, target: ControlTarget) {
        let bluetooth = self.shell.services.status.bluetooth.clone();
        match target {
            ControlTarget::Bluetooth => {
                if let Some(adapter) = bluetooth {
                    self.services_send(Command::BluetoothPower { on: !adapter.powered });
                }
            }
            ControlTarget::NightLight => self.night_light_set(!self.shell.control.night_light),
            ControlTarget::Address(index) => {
                if let Some((_, address)) = self.shell.control.addresses.get(index).cloned() {
                    self.address_copy(address);
                }
            }
            ControlTarget::Search => {
                if let Some(adapter) = bluetooth {
                    self.services_send(Command::BluetoothDiscover { on: !adapter.discovering });
                }
            }
            ControlTarget::Device(index) => {
                let Some(device) = bluetooth.as_ref().and_then(|b| b.listed().get(index).map(|d| (*d).clone())) else { return };
                let path = device.path;
                // A device paired from here should reconnect by itself later: the services trust it first.
                let command = if device.connected {
                    Command::DeviceDisconnect { path }
                } else if device.paired {
                    Command::DeviceConnect { path }
                } else {
                    Command::DevicePair { path }
                };
                self.services_send(command);
            }
            ControlTarget::DeviceRow(_) => {}
            ControlTarget::Settings => {
                self.spawn_command(&[SETTINGS.to_owned()], false);
                // The app's window takes the keyboard, which closes the panel anyway.
                self.panel_close();
            }
        }
    }

    pub fn control_model(&self) -> ControlModel {
        let adapter = self.shell.services.status.bluetooth.as_ref().map(adapter_view);
        let control = &self.shell.control;
        ControlModel { adapter, night_light: control.night_light, addresses: control.addresses.clone(), copied: control.copied.clone() }
    }
}

fn adapter_view(bluetooth: &Bluetooth) -> AdapterView {
    let devices = bluetooth
        .listed()
        .into_iter()
        .map(|d| DeviceView {
            name: d.name.clone(),
            paired: d.paired,
            connected: d.connected,
            battery: d.battery,
            busy: d.busy.map(|busy| match busy {
                mywm_shell_protocol::Busy::Pairing => DeviceBusy::Pairing,
                mywm_shell_protocol::Busy::Connecting => DeviceBusy::Connecting,
                mywm_shell_protocol::Busy::Disconnecting => DeviceBusy::Disconnecting,
            }),
        })
        .collect();
    AdapterView { powered: bluetooth.powered, discovering: bluetooth.discovering, devices }
}

#[cfg(test)]
mod tests {
    #[test]
    fn addresses_come_from_ip() {
        let text = "2: eno1    inet 192.168.178.56/24 brd 192.168.178.255 scope global dynamic eno1\\       valid_lft 1sec\n\
                    3: wlan0    inet 10.0.0.2/8 scope global wlan0\n";
        assert_eq!(super::parse_addresses(text), [("eno1".into(), "192.168.178.56".into()), ("wlan0".into(), "10.0.0.2".into())]);
        assert!(super::parse_addresses("").is_empty());
    }
}
