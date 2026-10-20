//! The tray (StatusNotifierItem), as Quickshell's `SystemTray` serves it: the shell is the watcher
//! (`org.kde.StatusNotifierWatcher`, taken over once another one leaves) and its host. Items come with their status,
//! icon (the icon theme's, the item's own `IconThemePath`, or its pixmaps; the attention icon while it needs
//! attention) and menu (DBusMenu), which is read when it is about to show.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
};

use mywm_shell_protocol::{ICON_SIZE, Image, MenuEntry, Toggle, TrayItem, TrayStatus};
use zbus::{
    MatchRule,
    blocking::{Connection, MessageIterator, Proxy, fdo::DBusProxy},
    fdo::RequestNameFlags,
    interface,
    message::{Header, Type},
    object_server::SignalEmitter,
    proxy::CacheProperties,
    zvariant::{OwnedValue, Structure, Value},
};

use crate::{RETRY, Update, icons, media::connect};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";

/// Where an item and its menu are, by the item's address.
pub type Paths = Arc<Mutex<HashMap<String, ItemPaths>>>;

#[derive(Clone)]
pub struct ItemPaths {
    service: String,
    path: String,
    menu: Option<String>,
}

/// The items registered with this process's watcher.
type Registered = Arc<Mutex<Vec<String>>>;

struct Watcher {
    items: Registered,
    ticks: mpsc::Sender<()>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// `service/path` of a registration: a path belongs to the caller, a name has the standard path.
fn address(service: &str, sender: &str) -> String {
    if service.starts_with('/') { format!("{sender}{service}") } else { format!("{service}/StatusNotifierItem") }
}

/// The bus name and object path of an address.
fn split(address: &str) -> (&str, &str) {
    match address.find('/') {
        Some(at) => (&address[..at], &address[at..]),
        None => (address, "/StatusNotifierItem"),
    }
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
        let address = address(&service, &sender);
        let added = {
            let mut items = lock(&self.items);
            let new = !items.contains(&address);
            if new {
                items.push(address.clone());
            }
            new
        };
        if added {
            tracing::debug!("tray item {address} registered");
            let _ = Self::status_notifier_item_registered(&emitter, &address).await;
            let _ = self.ticks.send(());
        }
    }

    async fn register_status_notifier_host(&self, _service: String, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let _ = Self::status_notifier_host_registered(&emitter).await;
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        lock(&self.items).clone()
    }

    /// There is always one: this process.
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

pub fn run(updates: mpsc::Sender<Update>, paths: Paths) {
    loop {
        if let Err(error) = follow(&updates, &paths) {
            tracing::info!("no tray: {error}");
        }
        if updates.send(Update::Tray(Vec::new())).is_err() {
            return;
        }
        std::thread::sleep(RETRY);
    }
}

/// Tick for every message matching `rule` that `relevant` accepts.
fn watch_where(
    connection: &Connection,
    rule: MatchRule<'static>,
    ticks: &mpsc::Sender<()>,
    relevant: impl Fn(&zbus::message::Message) -> bool + Send + 'static,
) -> zbus::Result<()> {
    let messages = MessageIterator::for_match_rule(rule, connection, Some(256))?;
    let ticks = ticks.clone();
    std::thread::Builder::new()
        .name("tray-signals".into())
        .spawn(move || {
            for message in messages {
                let Ok(message) = message else { break };
                if relevant(&message) && ticks.send(()).is_err() {
                    break;
                }
            }
        })
        .map(|_| ())
        .map_err(|error| zbus::Error::Failure(error.to_string()))
}

fn follow(updates: &mpsc::Sender<Update>, paths: &Paths) -> zbus::Result<()> {
    let registered = Registered::default();
    let (ticks, tick) = mpsc::channel();
    let watcher = Watcher { items: registered.clone(), ticks: ticks.clone() };
    let connection = zbus::blocking::connection::Builder::session()?
        .method_timeout(std::time::Duration::from_secs(2))
        .serve_at(WATCHER_PATH, watcher)?
        .build()?;
    // In the queue: while another watcher runs, its items show (this process is a host of it).
    connection.request_name_with_flags(WATCHER, RequestNameFlags::AllowReplacement.into())?;
    let host = format!("org.kde.StatusNotifierHost-{}-1", std::process::id());
    connection.request_name(host.as_str())?;

    let owners = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .build();
    // Only names that matter: the watcher's, and those of items that go away.
    let items_now = registered.clone();
    watch_where(&connection, owners, &ticks, move |message| {
        let Ok((name, _old, new)) = message.body().deserialize::<(String, String, String)>() else { return false };
        name == WATCHER || (new.is_empty() && lock(&items_now).iter().any(|a| split(a).0 == name))
    })?;
    let watcher_signals = MatchRule::builder().msg_type(Type::Signal).interface(WATCHER)?.build();
    watch_where(&connection, watcher_signals, &ticks, |_| true)?;
    let item_signals = MatchRule::builder().msg_type(Type::Signal).interface(ITEM)?.build();
    watch_where(&connection, item_signals, &ticks, |_| true)?;
    drop(ticks);

    let bus = DBusProxy::new(&connection)?;
    let mut icons: HashMap<String, Option<Image>> = HashMap::new();
    let mut sent: Option<Vec<TrayItem>> = None;
    let mut registered_with: Option<String> = None;
    loop {
        let own = bus.get_name_owner(WATCHER.try_into()?).ok().map(|o| o.to_string());
        let ours = own.is_some() && own.as_deref() == connection.unique_name().map(|n| n.as_str());
        let addresses: Vec<String> = if ours {
            // Items whose program ended go, and the hosts hear of it.
            let gone: Vec<String> = {
                let mut items = lock(&registered);
                let gone = items.iter().filter(|a| !has_owner(&bus, split(a).0)).cloned().collect::<Vec<_>>();
                items.retain(|a| !gone.contains(a));
                gone
            };
            for address in gone {
                tracing::debug!("tray item {address} went away");
                let _ = connection.emit_signal(None::<&str>, WATCHER_PATH, WATCHER, "StatusNotifierItemUnregistered", &(address.as_str(),));
            }
            lock(&registered).clone()
        } else {
            // Another watcher: register as its host once, and show what it has.
            if own.is_some() && registered_with != own {
                if let Ok(watcher) = proxy(&connection, WATCHER, WATCHER_PATH, WATCHER) {
                    let _ = watcher.call::<_, _, ()>("RegisterStatusNotifierHost", &(host.as_str(),));
                }
                registered_with = own.clone();
            }
            proxy(&connection, WATCHER, WATCHER_PATH, WATCHER)
                .and_then(|w| w.get_property::<Vec<String>>("RegisteredStatusNotifierItems"))
                .unwrap_or_default()
        };
        let mut found = HashMap::new();
        let items: Vec<TrayItem> = addresses.iter().filter_map(|address| item(&connection, address, &mut icons, &mut found)).collect();
        *lock(paths) = found;
        if sent.as_ref() != Some(&items) {
            if updates.send(Update::Tray(items.clone())).is_err() {
                return Ok(());
            }
            sent = Some(items);
        }
        tick.recv().map_err(|_| zbus::Error::Failure("the signal threads ended".into()))?;
        while tick.try_recv().is_ok() {}
    }
}

/// Whether a program holds the bus name `name` (a malformed name: no).
fn has_owner(bus: &DBusProxy, name: &str) -> bool {
    zbus::names::BusName::try_from(name).ok().and_then(|name| bus.name_has_owner(name).ok()).unwrap_or(false)
}

fn proxy<'a>(connection: &Connection, service: &'a str, path: &'a str, interface: &'a str) -> zbus::Result<Proxy<'a>> {
    zbus::blocking::proxy::Builder::new(connection)
        .destination(service)?
        .path(path)?
        .interface(interface)?
        .cache_properties(CacheProperties::No)
        .build()
}

fn text(properties: &HashMap<String, OwnedValue>, name: &str) -> String {
    properties.get(name).and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default()
}

/// The item at `address`, `None` if it does not answer.
fn item(connection: &Connection, address: &str, icons: &mut HashMap<String, Option<Image>>, paths: &mut HashMap<String, ItemPaths>) -> Option<TrayItem> {
    let (service, path) = split(address);
    let properties = zbus::blocking::fdo::PropertiesProxy::builder(connection)
        .destination(service)
        .ok()?
        .path(path)
        .ok()?
        .build()
        .ok()?
        .get_all(ITEM.try_into().ok()?)
        .ok()?;
    let status = match text(&properties, "Status").as_str() {
        "Passive" => TrayStatus::Passive,
        "NeedsAttention" => TrayStatus::NeedsAttention,
        _ => TrayStatus::Active,
    };
    let menu = properties
        .get("Menu")
        .and_then(|v| zbus::zvariant::OwnedObjectPath::try_from(v.clone()).ok())
        .map(|p| p.to_string())
        .filter(|p| !p.is_empty() && p != "/" && p != "/NO_DBUSMENU");
    let only_menu = properties.get("ItemIsMenu").and_then(|v| bool::try_from(v).ok()).unwrap_or(false);
    let theme_path = text(&properties, "IconThemePath");
    let extra: Vec<std::path::PathBuf> = Some(theme_path.clone()).filter(|p| !p.is_empty()).map(Into::into).into_iter().collect();
    let pixmaps = |name: &str| -> Vec<(i32, i32, Vec<u8>)> {
        properties.get(name).and_then(|v| Vec::<(i32, i32, Vec<u8>)>::try_from(v.clone()).ok()).unwrap_or_default()
    };
    let attention = status == TrayStatus::NeedsAttention;
    let name = if attention && !text(&properties, "AttentionIconName").is_empty() {
        text(&properties, "AttentionIconName")
    } else {
        text(&properties, "IconName")
    };
    let icon = if !name.is_empty() {
        let key = format!("{name}|{theme_path}");
        icons.entry(key).or_insert_with(|| icons::load_icon(&name, &extra, ICON_SIZE)).clone()
    } else {
        let mut chosen = if attention { pixmaps("AttentionIconPixmap") } else { Vec::new() };
        if chosen.is_empty() {
            chosen = pixmaps("IconPixmap");
        }
        icons::from_argb_pixmaps(&chosen, ICON_SIZE)
    };
    paths.insert(address.to_owned(), ItemPaths { service: service.to_owned(), path: path.to_owned(), menu: menu.clone() });
    Some(TrayItem {
        address: address.to_owned(),
        id: text(&properties, "Id"),
        title: text(&properties, "Title"),
        status,
        icon,
        has_menu: menu.is_some(),
        only_menu,
    })
}

/// What the compositor asks of the tray.
pub enum TrayCommand {
    Activate(String),
    SecondaryActivate(String),
    Scroll(String, i32, bool),
    Menu(String),
    MenuShow(String, i32),
    MenuClick(String, i32),
}

/// Carry out tray commands on a connection of their own (an item that does not answer delays only this).
pub fn run_commands(commands: mpsc::Receiver<TrayCommand>, paths: Paths, updates: mpsc::Sender<Update>) {
    let mut connection: Option<Connection> = None;
    for command in commands {
        if connection.is_none() {
            connection = connect(false).map_err(|error| tracing::warn!("cannot reach the tray: {error}")).ok();
        }
        let Some(bus) = &connection else { continue };
        let address = match &command {
            TrayCommand::Activate(a) | TrayCommand::SecondaryActivate(a) | TrayCommand::Scroll(a, ..) => a,
            TrayCommand::Menu(a) | TrayCommand::MenuShow(a, _) | TrayCommand::MenuClick(a, _) => a,
        };
        let Some(item) = lock(&paths).get(address).cloned() else { continue };
        let result = match &command {
            TrayCommand::Activate(_) => call_item(bus, &item, "Activate", &(0i32, 0i32)),
            TrayCommand::SecondaryActivate(_) => call_item(bus, &item, "SecondaryActivate", &(0i32, 0i32)),
            TrayCommand::Scroll(_, delta, horizontal) => {
                call_item(bus, &item, "Scroll", &(*delta, if *horizontal { "horizontal" } else { "vertical" }))
            }
            TrayCommand::Menu(_) | TrayCommand::MenuShow(..) => {
                let id = if let TrayCommand::MenuShow(_, id) = command { id } else { 0 };
                match read_menu(bus, &item, id) {
                    Some(menu) => {
                        let _ = updates.send(Update::TrayMenu(address.clone(), Box::new(menu)));
                        Ok(())
                    }
                    None => Err(zbus::Error::Failure("no menu".into())),
                }
            }
            TrayCommand::MenuClick(_, id) => {
                let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as u32);
                match &item.menu {
                    Some(menu) => proxy(bus, &item.service, menu, MENU)
                        .and_then(|menu| menu.call::<_, _, ()>("Event", &(*id, "clicked", Value::from(0i32), timestamp))),
                    None => Ok(()),
                }
            }
        };
        if let Err(error) = result {
            tracing::warn!("tray item {address}: {error}");
        }
    }
}

fn call_item<B: serde::Serialize + zbus::zvariant::DynamicType>(bus: &Connection, item: &ItemPaths, method: &str, body: &B) -> zbus::Result<()> {
    proxy(bus, &item.service, &item.path, ITEM)?.call::<_, _, ()>(method, body)
}

/// The item's menu after `AboutToShow(id)`: the whole tree from the root.
fn read_menu(bus: &Connection, item: &ItemPaths, id: i32) -> Option<MenuEntry> {
    let menu = proxy(bus, &item.service, item.menu.as_deref()?, MENU).ok()?;
    if let Err(error) = menu.call::<_, _, bool>("AboutToShow", &(id,)) {
        // As Quickshell: the menu shows anyway.
        tracing::debug!("AboutToShow failed: {error}");
    }
    type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);
    let (_revision, layout): (u32, Layout) = menu.call("GetLayout", &(0i32, -1i32, Vec::<String>::new())).ok()?;
    Some(entry(layout).0)
}

/// An entry of a menu layout and whether it is visible.
fn entry((id, properties, children): (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>)) -> (MenuEntry, bool) {
    let flag = |name: &str| properties.get(name).and_then(|v| bool::try_from(v).ok());
    let icon_name = text(&properties, "icon-name");
    let icon = properties
        .get("icon-data")
        .and_then(|v| Vec::<u8>::try_from(v.clone()).ok())
        .and_then(|data| icons::from_bytes(&data, ICON_SIZE))
        .or_else(|| (!icon_name.is_empty() && !icon_name.contains("symbolic")).then(|| icons::load_icon(&icon_name, &[], ICON_SIZE)).flatten());
    let toggle = match text(&properties, "toggle-type").as_str() {
        "checkmark" => Toggle::Check,
        "radio" => Toggle::Radio,
        _ => Toggle::None,
    };
    let checked = properties.get("toggle-state").and_then(|v| i32::try_from(v).ok()) == Some(1);
    let children = children
        .into_iter()
        .filter_map(|child| {
            let structure = Structure::try_from(child).ok()?;
            let layout = <(i32, HashMap<String, OwnedValue>, Vec<OwnedValue>)>::try_from(structure).ok()?;
            Some(entry(layout))
        })
        // Hidden entries are left out, as Quickshell lists only the visible ones.
        .filter(|(_, visible)| *visible)
        .map(|(child, _)| child)
        .collect();
    let entry = MenuEntry {
        id,
        label: text(&properties, "label"),
        separator: text(&properties, "type") == "separator",
        enabled: flag("enabled").unwrap_or(true),
        icon_name,
        icon,
        toggle,
        checked,
        submenu: text(&properties, "children-display") == "submenu",
        children,
    };
    (entry, flag("visible").unwrap_or(true))
}
