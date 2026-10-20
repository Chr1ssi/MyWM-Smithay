//! BlueZ's first adapter (as Quickshell's `Bluetooth.defaultAdapter`): whether it is on and how many of its devices
//! are connected. BlueZ may come and go; without it there is no adapter.
use std::sync::mpsc;

use zbus::{
    MatchRule,
    blocking::{Connection, fdo::ObjectManagerProxy},
    message::Type,
    zvariant::OwnedValue,
};
use mywm_shell_protocol::Bluetooth;

use crate::{
    RETRY, Update,
    media::{connect, watch},
};

pub fn run(updates: mpsc::Sender<Update>) {
    loop {
        if let Err(error) = follow(&updates) {
            tracing::info!("no system bus for Bluetooth: {error}");
        }
        if updates.send(Update::Bluetooth(None)).is_err() {
            return;
        }
        std::thread::sleep(RETRY);
    }
}

fn follow(updates: &mpsc::Sender<Update>) -> zbus::Result<()> {
    let connection = connect(true)?;
    let (ticks, tick) = mpsc::channel();
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
        watch(&connection, rule, &ticks)?;
    }
    drop(ticks);
    loop {
        if updates.send(Update::Bluetooth(adapter(&connection))).is_err() {
            return Ok(());
        }
        tick.recv().map_err(|_| zbus::Error::Failure("the signal threads ended".into()))?;
        while tick.try_recv().is_ok() {}
    }
}

fn flag(properties: &std::collections::HashMap<String, OwnedValue>, name: &str) -> bool {
    properties.get(name).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
}

/// The first adapter's state, `None` without BlueZ or adapter.
fn adapter(connection: &Connection) -> Option<Bluetooth> {
    let manager = ObjectManagerProxy::builder(connection).destination("org.bluez").ok()?.path("/").ok()?.build().ok()?;
    let objects = manager.get_managed_objects().ok()?;
    let mut adapters: Vec<_> = objects
        .iter()
        .filter_map(|(path, interfaces)| Some((path.as_str(), interfaces.get("org.bluez.Adapter1")?)))
        .collect();
    adapters.sort_by_key(|(path, _)| *path);
    let (path, adapter) = adapters.first()?;
    let connected = objects
        .values()
        .filter_map(|interfaces| interfaces.get("org.bluez.Device1"))
        .filter(|device| {
            let owner = device.get("Adapter").and_then(|v| zbus::zvariant::ObjectPath::try_from(v.clone()).ok());
            owner.is_some_and(|owner| owner.as_str() == *path) && flag(device, "Connected")
        })
        .count();
    Some(Bluetooth { powered: flag(adapter, "Powered"), connected })
}
