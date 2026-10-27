//! The freedesktop notification service (`org.freedesktop.Notifications`, as splice-shell's Quickshell bar served it):
//! notifications go to the compositor, which shows them; its users' choices come back as `Dismiss` and `Invoke`.
//!
//! The name is requested in the queue: while another daemon (dunst, the Quickshell bar) holds it, this one waits
//! and takes over when it leaves. Pictures are decoded on a worker thread, so a large one never holds up the bus;
//! closings go through the same thread and never overtake their notification.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
};

use splice_shell_protocol::{Action, Command, Notification, Urgency};
use zbus::{
    blocking::Connection,
    fdo::{RequestNameFlags, RequestNameReply},
    interface,
    object_server::SignalEmitter,
    zvariant::{OwnedValue, Structure},
};

use crate::{RETRY, Update, icons};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// `NotificationClosed` reasons.
const DISMISSED: u32 = 2;
const CLOSED_BY_CALL: u32 = 3;

/// Work for the thread that hands notifications to the compositor, in order.
enum Job {
    Notify(Box<Incoming>),
    Closed(u32),
}

/// A `Notify` call as it came.
struct Incoming {
    id: u32,
    app_name: String,
    app_icon: String,
    summary: String,
    body: String,
    actions: Vec<String>,
    hints: HashMap<String, OwnedValue>,
    expire_timeout: i32,
}

/// The live notifications and whether each is resident (stays after one of its actions).
type Known = Arc<Mutex<HashMap<u32, bool>>>;

struct Server {
    known: Known,
    next_id: Mutex<u32>,
    jobs: mpsc::Sender<Job>,
}

fn hint<T: TryFrom<OwnedValue>>(hints: &HashMap<String, OwnedValue>, name: &str) -> Option<T> {
    hints.get(name).and_then(|value| value.try_clone().ok()).and_then(|value| T::try_from(value).ok())
}

#[interface(name = "org.freedesktop.Notifications")]
impl Server {
    fn get_capabilities(&self) -> Vec<&'static str> {
        vec!["actions", "body", "body-markup", "icon-static", "persistence"]
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let resident = hint::<bool>(&hints, "resident").unwrap_or(false);
        let mut known = self.known.lock().unwrap_or_else(|e| e.into_inner());
        // A notification that is gone cannot be replaced: it comes again as a new one.
        let id = if replaces_id != 0 && known.contains_key(&replaces_id) {
            replaces_id
        } else {
            let mut next = self.next_id.lock().unwrap_or_else(|e| e.into_inner());
            let id = *next;
            *next = next.checked_add(1).unwrap_or(1);
            id
        };
        known.insert(id, resident);
        let incoming = Incoming { id, app_name, app_icon, summary, body, actions, hints, expire_timeout };
        let _ = self.jobs.send(Job::Notify(Box::new(incoming)));
        id
    }

    async fn close_notification(&self, id: u32, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let known = self.known.lock().unwrap_or_else(|e| e.into_inner()).remove(&id).is_some();
        if known {
            let _ = self.jobs.send(Job::Closed(id));
            if let Err(error) = Self::notification_closed(&emitter, id, CLOSED_BY_CALL).await {
                tracing::warn!("cannot say that notification {id} closed: {error}");
            }
        }
    }

    fn get_server_information(&self) -> (&'static str, &'static str, &'static str, &'static str) {
        ("splice", "splice", env!("CARGO_PKG_VERSION"), "1.2")
    }

    #[zbus(signal)]
    async fn notification_closed(emitter: &SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;
}

pub fn run(updates: mpsc::Sender<Update>, commands: mpsc::Receiver<Command>) {
    loop {
        match serve(&updates, &commands) {
            Ok(()) => return,
            Err(error) => tracing::info!("no notification service: {error}"),
        }
        std::thread::sleep(RETRY);
    }
}

/// Serve the bus until the compositor's commands end.
fn serve(updates: &mpsc::Sender<Update>, commands: &mpsc::Receiver<Command>) -> zbus::Result<()> {
    let known = Known::default();
    let (jobs, queue) = mpsc::channel();
    let forward = updates.clone();
    std::thread::Builder::new()
        .name("notification-pictures".into())
        .spawn(move || {
            for job in queue {
                let update = match job {
                    Job::Notify(incoming) => Update::Notification(Box::new(prepare(*incoming))),
                    Job::Closed(id) => Update::Closed(id),
                };
                if forward.send(update).is_err() {
                    return;
                }
            }
        })
        .map_err(|error| zbus::Error::Failure(error.to_string()))?;
    let server = Server { known: known.clone(), next_id: Mutex::new(1), jobs: jobs.clone() };
    let connection = zbus::blocking::connection::Builder::session()?.serve_at(PATH, server)?.build()?;
    match connection.request_name_with_flags(NAME, RequestNameFlags::AllowReplacement.into())? {
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => tracing::info!("serving notifications"),
        RequestNameReply::InQueue | RequestNameReply::Exists => {
            tracing::warn!("another notification daemon serves {NAME}; the shell takes over once it ends")
        }
    }
    for command in commands {
        match command {
            Command::Dismiss { id } => {
                if known.lock().unwrap_or_else(|e| e.into_inner()).remove(&id).is_some() {
                    emit(&connection, "NotificationClosed", &(id, DISMISSED));
                }
            }
            Command::Invoke { id, action } => {
                let resident = known.lock().unwrap_or_else(|e| e.into_inner()).get(&id).copied();
                let Some(resident) = resident else { continue };
                emit(&connection, "ActionInvoked", &(id, action.as_str()));
                // Like Quickshell: an action ends a notification unless it asked to stay.
                if !resident {
                    known.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                    emit(&connection, "NotificationClosed", &(id, DISMISSED));
                    let _ = jobs.send(Job::Closed(id));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn emit<B: serde::Serialize + zbus::zvariant::DynamicType>(connection: &Connection, member: &str, body: &B) {
    if let Err(error) = connection.emit_signal(None::<&str>, PATH, NAME, member, body) {
        tracing::warn!("cannot send {member}: {error}");
    }
}

/// The notification for the compositor, with its picture: `image-data`, else `image-path`, else the app's icon
/// (its own or its desktop entry's).
fn prepare(incoming: Incoming) -> Notification {
    let Incoming { id, app_name, app_icon, summary, body, actions, hints, expire_timeout } = incoming;
    let urgency = match hint::<u8>(&hints, "urgency") {
        Some(0) => Urgency::Low,
        Some(2) => Urgency::Critical,
        _ => Urgency::Normal,
    };
    let raw = ["image-data", "image_data", "icon_data"].iter().find_map(|name| hint::<Structure>(&hints, name));
    let image = raw
        .and_then(|structure| {
            type Data = (i32, i32, i32, bool, i32, i32, Vec<u8>);
            let (width, height, rowstride, has_alpha, bits_per_sample, channels, data) = Data::try_from(structure).ok()?;
            icons::from_raw(&icons::Raw { width, height, rowstride, has_alpha, bits_per_sample, channels, data: &data })
        })
        .or_else(|| {
            let path = ["image-path", "image_path"].iter().find_map(|name| hint::<String>(&hints, name))?;
            icons::load(&path)
        })
        .or_else(|| {
            let icon = Some(app_icon)
                .filter(|icon| !icon.is_empty())
                .or_else(|| icons::desktop_entry_icon(&hint::<String>(&hints, "desktop-entry").unwrap_or_default()))
                .unwrap_or_else(|| "dialog-information".into());
            icons::load(&icon)
        });
    // Actions come in pairs of identifier and label; an odd list is broken and has none.
    let actions = if actions.len() % 2 == 0 {
        actions.chunks(2).map(|pair| Action { identifier: pair[0].clone(), text: pair[1].clone() }).collect()
    } else {
        Vec::new()
    };
    Notification { id, app_name, summary, body, urgency, expire_timeout, actions, image }
}
