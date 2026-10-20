//! The first MPRIS player that plays (as the Quickshell bar's `Mpris.players.values.find(p => p.isPlaying)`):
//! its title, or its name without one. Players count in the order they appeared on the session bus.
use std::{collections::HashMap, sync::mpsc, time::Duration};

use zbus::{
    MatchRule,
    blocking::{Connection, MessageIterator, Proxy, connection, fdo::DBusProxy},
    message::Type,
    proxy::CacheProperties,
    zvariant::OwnedValue,
};

use crate::{RETRY, Update};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";

pub fn run(updates: mpsc::Sender<Update>) {
    loop {
        if let Err(error) = follow(&updates) {
            tracing::info!("no media players: {error}");
        }
        if updates.send(Update::Media(None)).is_err() {
            return;
        }
        std::thread::sleep(RETRY);
    }
}

/// Start a thread that ticks `ticks` for every message matching `rule`.
pub fn watch(connection: &Connection, rule: MatchRule<'static>, ticks: &mpsc::Sender<()>) -> zbus::Result<()> {
    let messages = MessageIterator::for_match_rule(rule, connection, Some(64))?;
    let ticks = ticks.clone();
    std::thread::Builder::new()
        .name("dbus-signals".into())
        .spawn(move || {
            for message in messages {
                if message.is_err() || ticks.send(()).is_err() {
                    break;
                }
            }
        })
        .map(|_| ())
        .map_err(|error| zbus::Error::Failure(error.to_string()))
}

/// A connection whose calls give up after a while: a player that does not answer must not stop the others.
pub fn connect(system: bool) -> zbus::Result<Connection> {
    let builder = if system { connection::Builder::system()? } else { connection::Builder::session()? };
    builder.method_timeout(Duration::from_secs(2)).build()
}

fn follow(updates: &mpsc::Sender<Update>) -> zbus::Result<()> {
    let connection = connect(false)?;
    let (ticks, tick) = mpsc::channel();
    let names = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg0ns("org.mpris.MediaPlayer2")?
        .build();
    let changes = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path(PATH)?
        .build();
    watch(&connection, names, &ticks)?;
    watch(&connection, changes, &ticks)?;
    drop(ticks);
    let bus = DBusProxy::new(&connection)?;
    let mut players: Vec<String> = Vec::new();
    loop {
        let present: Vec<String> =
            bus.list_names()?.into_iter().map(|name| name.to_string()).filter(|name| name.starts_with(PREFIX)).collect();
        players.retain(|name| present.contains(name));
        for name in present {
            if !players.contains(&name) {
                players.push(name);
            }
        }
        let playing = players.iter().find_map(|name| playing_title(&connection, name));
        if updates.send(Update::Media(playing)).is_err() {
            return Ok(());
        }
        // Wait for news, then take all that came meanwhile at once.
        tick.recv().map_err(|_| zbus::Error::Failure("the signal threads ended".into()))?;
        while tick.try_recv().is_ok() {}
    }
}

fn proxy<'a>(connection: &Connection, name: &'a str, interface: &'a str) -> zbus::Result<Proxy<'a>> {
    zbus::blocking::proxy::Builder::new(connection)
        .destination(name)?
        .path(PATH)?
        .interface(interface)?
        .cache_properties(CacheProperties::No)
        .build()
}

/// What the bar shows for `name` if it plays: the track's title, else the player's name.
fn playing_title(connection: &Connection, name: &str) -> Option<String> {
    let player = proxy(connection, name, "org.mpris.MediaPlayer2.Player").ok()?;
    if player.get_property::<String>("PlaybackStatus").ok()? != "Playing" {
        return None;
    }
    let title = player
        .get_property::<HashMap<String, OwnedValue>>("Metadata")
        .ok()
        .and_then(|metadata| metadata.get("xesam:title").and_then(|v| String::try_from(v.clone()).ok()))
        .filter(|title| !title.is_empty());
    title.or_else(|| proxy(connection, name, "org.mpris.MediaPlayer2").ok()?.get_property::<String>("Identity").ok())
}
