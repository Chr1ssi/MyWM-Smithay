//! BlueZ's first adapter (as Quickshell's `Bluetooth.defaultAdapter`): whether it is on and searching, and its
//! devices (alias, pairing, connection, battery). Commands switch it, search, and connect, disconnect or pair a
//! device; while such a call runs the device counts as busy. BlueZ may come and go; without it there is no adapter.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use mywm_shell_protocol::{Bluetooth, Busy, Device};
use zbus::{
    MatchRule,
    blocking::{Connection, fdo::ObjectManagerProxy},
    message::Type,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

use crate::{
    RETRY, Update,
    media::{connect, watch},
};

const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const BATTERY: &str = "org.bluez.Battery1";

/// The devices a call runs for, by object path.
pub type BusyDevices = Arc<Mutex<HashMap<String, Busy>>>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// `pokes` wakes the report (a call began or ended); `poke` is its sender, for the signal threads.
pub fn run(updates: mpsc::Sender<Update>, busy: BusyDevices, poke: mpsc::Sender<()>, pokes: mpsc::Receiver<()>) {
    loop {
        if let Err(error) = follow(&updates, &busy, &poke, &pokes) {
            tracing::info!("no system bus for Bluetooth: {error}");
        }
        if updates.send(Update::Bluetooth(None)).is_err() {
            return;
        }
        std::thread::sleep(RETRY);
    }
}

fn follow(updates: &mpsc::Sender<Update>, busy: &BusyDevices, poke: &mpsc::Sender<()>, pokes: &mpsc::Receiver<()>) -> zbus::Result<()> {
    let connection = connect(true)?;
    let owner = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg(0, "org.bluez")?
        .build();
    let objects = MatchRule::builder().msg_type(Type::Signal).interface("org.freedesktop.DBus.ObjectManager")?.path("/")?.build();
    let changes = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path_namespace("/org/bluez")?
        .build();
    for rule in [owner, objects, changes] {
        watch(&connection, rule, poke)?;
    }
    loop {
        if updates.send(Update::Bluetooth(adapter(&connection, busy))).is_err() {
            return Ok(());
        }
        pokes.recv().map_err(|_| zbus::Error::Failure("nobody pokes any more".into()))?;
        while pokes.try_recv().is_ok() {}
    }
}

type Interfaces = HashMap<String, HashMap<String, OwnedValue>>;

fn flag(properties: &HashMap<String, OwnedValue>, name: &str) -> bool {
    properties.get(name).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
}

fn text(properties: &HashMap<String, OwnedValue>, name: &str) -> String {
    properties.get(name).and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default()
}

/// BlueZ's objects, `None` without BlueZ.
fn objects(connection: &Connection) -> Option<HashMap<OwnedObjectPath, Interfaces>> {
    let manager = ObjectManagerProxy::builder(connection).destination("org.bluez").ok()?.path("/").ok()?.build().ok()?;
    let objects = manager.get_managed_objects().ok()?;
    Some(objects.into_iter().map(|(path, interfaces)| (path, interfaces.into_iter().map(|(i, p)| (i.to_string(), p)).collect())).collect())
}

/// The first adapter's path.
fn first_adapter(objects: &HashMap<OwnedObjectPath, Interfaces>) -> Option<String> {
    let mut adapters: Vec<&str> = objects.iter().filter(|(_, i)| i.contains_key(ADAPTER)).map(|(p, _)| p.as_str()).collect();
    adapters.sort();
    adapters.first().map(|p| (*p).to_owned())
}

/// The first adapter's state, `None` without BlueZ or adapter.
fn adapter(connection: &Connection, busy: &BusyDevices) -> Option<Bluetooth> {
    let objects = objects(connection)?;
    let path = first_adapter(&objects)?;
    let adapter = objects.iter().find(|(p, _)| p.as_str() == path)?.1.get(ADAPTER)?;
    let busy = lock(busy).clone();
    let devices = objects
        .iter()
        .filter_map(|(device_path, interfaces)| {
            let device = interfaces.get(DEVICE)?;
            let owner = device.get("Adapter").and_then(|v| OwnedObjectPath::try_from(v.clone()).ok())?;
            if owner.as_str() != path {
                return None;
            }
            let address = text(device, "Address");
            let alias = text(device, "Alias");
            let battery = interfaces.get(BATTERY).and_then(|b| b.get("Percentage")).and_then(|v| u8::try_from(v).ok());
            Some(Device {
                path: device_path.to_string(),
                name: if alias.is_empty() { address.clone() } else { alias },
                address,
                paired: flag(device, "Paired"),
                connected: flag(device, "Connected"),
                battery,
                busy: busy.get(device_path.as_str()).copied(),
            })
        })
        .collect();
    Some(Bluetooth { powered: flag(adapter, "Powered"), discovering: flag(adapter, "Discovering"), devices })
}

pub enum BluetoothCommand {
    Power(bool),
    Discover(bool),
    Connect(String),
    Disconnect(String),
    Pair(String),
}

/// Carry out Bluetooth commands. Connecting and pairing can take long: each runs on a thread of its own, with
/// the device busy meanwhile.
pub fn run_commands(commands: mpsc::Receiver<BluetoothCommand>, busy: BusyDevices, poke: mpsc::Sender<()>) {
    let bus = || zbus::blocking::connection::Builder::system().and_then(|b| b.method_timeout(Duration::from_secs(60)).build());
    let mut connection: Option<Connection> = None;
    for command in commands {
        if connection.is_none() {
            connection = bus().map_err(|error| tracing::warn!("cannot reach BlueZ: {error}")).ok();
        }
        let Some(system) = connection.clone() else { continue };
        let result = match command {
            BluetoothCommand::Power(on) => with_adapter(&system, |adapter| set(&system, adapter, ADAPTER, "Powered", Value::from(on))),
            BluetoothCommand::Discover(on) => {
                with_adapter(&system, |adapter| call(&system, adapter, ADAPTER, if on { "StartDiscovery" } else { "StopDiscovery" }))
            }
            BluetoothCommand::Connect(path) => device_call(system, busy.clone(), poke.clone(), path, Busy::Connecting),
            BluetoothCommand::Disconnect(path) => device_call(system, busy.clone(), poke.clone(), path, Busy::Disconnecting),
            BluetoothCommand::Pair(path) => device_call(system, busy.clone(), poke.clone(), path, Busy::Pairing),
        };
        if let Err(error) = result {
            tracing::warn!("Bluetooth: {error}");
            if matches!(error, zbus::Error::InputOutput(_)) {
                connection = None;
            }
        }
    }
}

fn with_adapter(system: &Connection, then: impl FnOnce(&str) -> zbus::Result<()>) -> zbus::Result<()> {
    let objects = objects(system).ok_or_else(|| zbus::Error::Failure("BlueZ does not answer".into()))?;
    let adapter = first_adapter(&objects).ok_or_else(|| zbus::Error::Failure("no adapter".into()))?;
    then(&adapter)
}

fn call(system: &Connection, path: &str, interface: &str, method: &str) -> zbus::Result<()> {
    system.call_method(Some("org.bluez"), path, Some(interface), method, &()).map(|_| ())
}

fn set(system: &Connection, path: &str, interface: &str, property: &str, value: Value<'_>) -> zbus::Result<()> {
    system.call_method(Some("org.bluez"), path, Some("org.freedesktop.DBus.Properties"), "Set", &(interface, property, value)).map(|_| ())
}

/// Connect, disconnect or pair (after trusting, so the device connects by itself later) on a thread of its own.
fn device_call(system: Connection, busy: BusyDevices, poke: mpsc::Sender<()>, path: String, what: Busy) -> zbus::Result<()> {
    lock(&busy).insert(path.clone(), what);
    let _ = poke.send(());
    let spawned = std::thread::Builder::new().name("bluetooth-call".into()).spawn(move || {
        let result = match what {
            Busy::Connecting => call(&system, &path, DEVICE, "Connect"),
            Busy::Disconnecting => call(&system, &path, DEVICE, "Disconnect"),
            Busy::Pairing => set(&system, &path, DEVICE, "Trusted", Value::from(true)).and_then(|()| call(&system, &path, DEVICE, "Pair")),
        };
        if let Err(error) = result {
            tracing::warn!("Bluetooth {path}: {error}");
        }
        lock(&busy).remove(&path);
        let _ = poke.send(());
    });
    spawned.map(|_| ()).map_err(|error| zbus::Error::Failure(error.to_string()))
}
