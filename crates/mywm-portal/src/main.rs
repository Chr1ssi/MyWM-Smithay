//! `mywm-portal`: the screen-cast backend of xdg-desktop-portal for the MyWM compositor.
//!
//! Instead of a generic dialog it asks the compositor what to share (click a window, or the
//! desktop for the monitor) and captures it with `ext-image-copy-capture` into a PipeWire
//! stream. Replaces `xdg-desktop-portal-wlr`.
mod chooser;
mod stream;

use std::{
    collections::HashMap,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use mywm_ipc::{Chosen, SourceKinds};
use stream::{Source, StreamHandle};
use zbus::{
    Connection, ObjectServer, interface,
    object_server::SignalEmitter,
    zvariant::{ObjectPath, OwnedValue, Value},
};

const NAME: &str = "org.freedesktop.impl.portal.desktop.mywm";
const PATH: &str = "/org/freedesktop/portal/desktop";

/// `SelectSources` choices of a session and its running stream.
#[derive(Default)]
struct SessionState {
    types: u32,
    cursor_mode: u32,
    stream: Option<StreamHandle>,
}

type Sessions = Arc<Mutex<HashMap<String, SessionState>>>;

struct ScreenCast {
    sessions: Sessions,
}

type Results = HashMap<String, OwnedValue>;

fn option_u32(options: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    options.get(key).and_then(|v| v.downcast_ref::<u32>().ok())
}

#[interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    async fn create_session(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: &str,
        _options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        tracing::info!("screen cast session for {app_id:?}");
        let path = session_handle.to_string();
        self.sessions.lock().unwrap().insert(path.clone(), SessionState::default());
        let session = Session { path, sessions: self.sessions.clone() };
        match server.at(session_handle, session).await {
            Ok(_) => (0, Results::new()),
            Err(error) => {
                tracing::warn!("cannot export the session: {error}");
                (2, Results::new())
            }
        }
    }

    async fn select_sources(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, Results) {
        let mut sessions = self.sessions.lock().unwrap();
        let Some(session) = sessions.get_mut(session_handle.as_str()) else { return (2, Results::new()) };
        session.types = option_u32(&options, "types").unwrap_or(1);
        session.cursor_mode = option_u32(&options, "cursor_mode").unwrap_or(1);
        (0, Results::new())
    }

    async fn start(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        _options: HashMap<String, OwnedValue>,
        #[zbus(connection)] connection: &Connection,
    ) -> (u32, Results) {
        let path = session_handle.to_string();
        let (types, cursor_mode) = {
            let sessions = self.sessions.lock().unwrap();
            let Some(session) = sessions.get(&path) else { return (2, Results::new()) };
            (session.types, session.cursor_mode)
        };
        let bus = zbus::blocking::Connection::from(connection.clone());
        let sessions = self.sessions.clone();
        let session_path = path.clone();
        let app_id = app_id.to_owned();
        let result = blocking::unblock(move || start_stream(&app_id, types, cursor_mode, &session_path, bus)).await;
        match result {
            Ok(Some((handle, node_id, size, source_type))) => {
                if let Some(session) = sessions.lock().unwrap().get_mut(&path) {
                    session.stream = Some(handle);
                }
                let mut properties: HashMap<&str, Value<'_>> = HashMap::new();
                properties.insert("size", Value::new((size.0 as i32, size.1 as i32)));
                properties.insert("source_type", Value::new(source_type));
                let streams = vec![(node_id, properties)];
                let mut results = Results::new();
                if let Ok(value) = OwnedValue::try_from(Value::new(streams)) {
                    results.insert("streams".into(), value);
                }
                (0, results)
            }
            Ok(None) => (1, Results::new()),
            Err(error) => {
                tracing::warn!("cannot start the screen cast: {error}");
                (2, Results::new())
            }
        }
    }

    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        // Hidden and embedded; the cursor is painted into the image.
        3
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }
}

/// A running stream: its handle, PipeWire node id, size and `source_type`.
type Shared = (StreamHandle, u32, (u32, u32), u32);

/// One question to the compositor that several sessions of an app can wait for at the same time.
#[derive(Default)]
struct Answer {
    value: Mutex<Option<Result<Chosen, String>>>,
    ready: Condvar,
}

/// Chromium-based apps (Discord, Vesktop) open two sessions within a moment: ask once per app and
/// let the second session, which arrives while the first is being answered, take that answer
/// instead of being refused.
static ANSWERS: Mutex<Vec<(String, Arc<Answer>)>> = Mutex::new(Vec::new());

/// Vesktop asks once to list the sources and again, a moment later, to start: a choice made
/// within this window counts for the follow-up sessions of the same app.
const REUSE: Duration = Duration::from_secs(8);

static RECENT: Mutex<Vec<(String, std::time::Instant, Chosen)>> = Mutex::new(Vec::new());

fn choose_once(app_id: &str, kinds: SourceKinds) -> Result<Chosen, String> {
    let (answer, mine) = {
        let mut answers = ANSWERS.lock().unwrap();
        let now = std::time::Instant::now();
        let mut recent = RECENT.lock().unwrap();
        recent.retain(|(_, at, _)| now.duration_since(*at) < REUSE);
        if let Some((_, _, chosen)) = recent.iter().find(|(id, _, _)| id == app_id) {
            tracing::info!("{app_id}: reusing the choice made a moment ago");
            return Ok(chosen.clone());
        }
        drop(recent);
        match answers.iter().find(|(id, _)| id == app_id) {
            Some((_, answer)) => (answer.clone(), false),
            None => {
                let answer = Arc::new(Answer::default());
                answers.push((app_id.to_owned(), answer.clone()));
                (answer, true)
            }
        }
    };
    if mine {
        // Another question (the overview, a screenshot selection) may be open: wait for it.
        let mut result = chooser::choose(kinds);
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while matches!(&result, Err(e) if e.contains("busy")) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(500));
            result = chooser::choose(kinds);
        }
        if let Ok(chosen) = &result {
            if !matches!(chosen, Chosen::Nothing) {
                RECENT.lock().unwrap().push((app_id.to_owned(), std::time::Instant::now(), chosen.clone()));
            }
        }
        *answer.value.lock().unwrap() = Some(result.clone());
        answer.ready.notify_all();
        // Sessions that were waiting hold the answer; a later one asks again.
        ANSWERS.lock().unwrap().retain(|(id, _)| id != app_id);
        result
    } else {
        let mut value = answer.value.lock().unwrap();
        while value.is_none() {
            value = answer.ready.wait(value).unwrap();
        }
        tracing::info!("{app_id}: a second session shares the choice of the first");
        value.clone().unwrap()
    }
}

/// Ask the compositor what to share and start streaming it. `Ok(None)`: the user cancelled.
fn start_stream(
    app_id: &str,
    types: u32,
    cursor_mode: u32,
    session_path: &str,
    bus: zbus::blocking::Connection,
) -> Result<Option<Shared>, String> {
    let kinds = match types & 3 {
        1 => SourceKinds::Monitor,
        2 => SourceKinds::Window,
        _ => SourceKinds::Both,
    };
    let source = match choose_once(app_id, kinds)? {
        Chosen::Monitor(name) => Source::Monitor(name),
        Chosen::Window(id) => Source::Window(id),
        Chosen::Nothing => return Ok(None),
    };
    let source_type = source.portal_type();
    tracing::info!("sharing {source:?}");
    let path = session_path.to_owned();
    let (started, handle) = stream::start(source, cursor_mode == 2, move || {
        // The window or monitor went away: tell the app the session is over.
        let _ = bus.emit_signal(None::<&str>, path.as_str(), "org.freedesktop.impl.portal.Session", "Closed", &());
    })?;
    Ok(Some((handle, started.node_id, started.size, source_type)))
}

struct Session {
    path: String,
    sessions: Sessions,
}

#[interface(name = "org.freedesktop.impl.portal.Session")]
impl Session {
    async fn close(&self, #[zbus(object_server)] server: &ObjectServer) {
        let state = self.sessions.lock().unwrap().remove(&self.path);
        if let Some(handle) = state.and_then(|s| s.stream) {
            blocking::unblock(move || handle.stop()).await;
        }
        let _ = server.remove::<Session, _>(self.path.as_str()).await;
    }

    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

fn main() -> zbus::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    zbus::block_on(async {
        let sessions: Sessions = Arc::default();
        let _connection = zbus::connection::Builder::session()?
            .name(NAME)?
            .serve_at(PATH, ScreenCast { sessions })?
            .build()
            .await?;
        tracing::info!("screen-cast portal ready as {NAME}");
        std::future::pending::<()>().await;
        Ok(())
    })
}
