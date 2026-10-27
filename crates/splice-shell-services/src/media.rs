//! The MPRIS players on the session bus, as Quickshell's `Mpris.players` lists them: in the order they appeared, with
//! title, artists, cover and what they allow. The bar shows the first one that plays; the media panel all of them.
use std::{collections::HashMap, sync::mpsc, time::Duration};

use splice_shell_protocol::{Image, Player, PlayerAction};
use zbus::{
    MatchRule,
    blocking::{Connection, MessageIterator, Proxy, connection, fdo::DBusProxy},
    message::Type,
    proxy::CacheProperties,
    zvariant::OwnedValue,
};

use crate::{RETRY, Update, icons};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

pub fn run(updates: mpsc::Sender<Update>) {
    loop {
        if let Err(error) = follow(&updates) {
            tracing::info!("no media players: {error}");
        }
        if updates.send(Update::Players(Vec::new())).is_err() {
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
    let mut order: Vec<String> = Vec::new();
    let mut covers: HashMap<String, Option<Image>> = HashMap::new();
    let mut sent: Option<Vec<Player>> = None;
    loop {
        let present: Vec<String> =
            bus.list_names()?.into_iter().map(|name| name.to_string()).filter(|name| name.starts_with(PREFIX)).collect();
        order.retain(|name| present.contains(name));
        for name in present {
            if !order.contains(&name) {
                order.push(name);
            }
        }
        let found: Vec<(Player, String)> = order.iter().filter_map(|name| player(&connection, name, &mut covers)).collect();
        // Covers nobody shows any more are forgotten.
        covers.retain(|url, _| found.iter().any(|(_, used)| used == url));
        let players: Vec<Player> = found.into_iter().map(|(player, _)| player).collect();
        if sent.as_ref() != Some(&players) {
            if updates.send(Update::Players(players.clone())).is_err() {
                return Ok(());
            }
            sent = Some(players);
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

/// The player at `name` with its cover's URL, `None` if it does not answer.
fn player(connection: &Connection, name: &str, covers: &mut HashMap<String, Option<Image>>) -> Option<(Player, String)> {
    let player = proxy(connection, name, PLAYER).ok()?;
    let flag = |property: &str| player.get_property::<bool>(property).unwrap_or(false);
    let identity = proxy(connection, name, "org.mpris.MediaPlayer2").ok().and_then(|app| app.get_property::<String>("Identity").ok());
    let metadata = player.get_property::<HashMap<String, OwnedValue>>("Metadata").unwrap_or_default();
    let text = |key: &str| metadata.get(key).and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default();
    let artist = metadata
        .get("xesam:artist")
        .and_then(|v| Vec::<String>::try_from(v.clone()).ok())
        .map(|artists| artists.join(", "))
        .unwrap_or_default();
    let playing = player.get_property::<String>("PlaybackStatus").is_ok_and(|status| status == "Playing");
    // As Quickshell: nothing is allowed without CanControl; play/pause asks for what the toggle would do.
    let control = flag("CanControl");
    let url = text("mpris:artUrl");
    let cover = if url.is_empty() { None } else { covers.entry(url.clone()).or_insert_with(|| icons::load_cover(&url)).clone() };
    let player = Player {
        bus: name.to_owned(),
        identity: identity.unwrap_or_default(),
        title: text("xesam:title"),
        artist,
        cover,
        playing,
        can_previous: control && flag("CanGoPrevious"),
        can_toggle: control && if playing { flag("CanPause") } else { flag("CanPlay") },
        can_next: control && flag("CanGoNext"),
    };
    Some((player, url))
}

/// Carry out a command for a player, on a connection of its own (a player that does not answer delays only this).
pub fn run_commands(commands: mpsc::Receiver<(String, PlayerAction)>) {
    let mut connection: Option<Connection> = None;
    for (bus, action) in commands {
        if connection.is_none() {
            connection = connect(false).map_err(|error| tracing::warn!("cannot reach the media players: {error}")).ok();
        }
        let Some(bus_connection) = &connection else { continue };
        let method = match action {
            PlayerAction::Previous => "Previous",
            PlayerAction::Play => "Play",
            PlayerAction::Pause => "Pause",
            PlayerAction::Next => "Next",
        };
        let result = proxy(bus_connection, &bus, PLAYER).and_then(|player| player.call::<_, _, ()>(method, &()));
        if let Err(error) = result {
            tracing::warn!("{bus}: {method} failed: {error}");
            if matches!(error, zbus::Error::InputOutput(_)) {
                connection = None;
            }
        }
    }
}
