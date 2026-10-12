//! The Inhibit portal: apps that keep the session awake through the portal (Firefox does, before it
//! would try Wayland's idle-inhibit) hold the compositor's `v1 inhibit-idle` while any of them does.
use std::{
    collections::HashSet,
    io::{Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::{Arc, Mutex},
};

use zbus::{ObjectServer, interface, zvariant::{ObjectPath, OwnedValue}};

/// `Inhibit` flag: keep the session from going idle (the others, logout, user switch and suspend, mywm
/// does not offer).
const IDLE: u32 = 8;

/// Request handles that keep the session awake, and the compositor connection that says so.
#[derive(Default)]
pub struct Inhibitions {
    handles: HashSet<String>,
    link: Option<UnixStream>,
}

impl Inhibitions {
    /// Connect and inhibit while there are handles, disconnect (which ends it) when there are none.
    fn sync(&mut self) {
        match (self.handles.is_empty(), self.link.is_some()) {
            (false, false) => match connect() {
                Ok(stream) => self.link = Some(stream),
                Err(error) => tracing::warn!("cannot keep the session awake: {error}"),
            },
            (true, true) => {
                if let Some(stream) = self.link.take() {
                    let _ = stream.shutdown(Shutdown::Both);
                }
            }
            _ => {}
        }
    }
}

/// A connection to the compositor that inhibits idle until it is shut down.
fn connect() -> Result<UnixStream, String> {
    let path = crate::chooser::socket_path().ok_or("no MYWM_SOCKET and no XDG_RUNTIME_DIR")?;
    let mut stream = UnixStream::connect(&path).map_err(|e| format!("cannot reach the compositor at {}: {e}", path.display()))?;
    stream.write_all(b"v1 inhibit-idle\n").map_err(|e| e.to_string())?;
    // The compositor sends every client its state; read it away, or it would drop the connection
    // once the backlog grows. Ends when the connection is shut down.
    let mut reader = stream.try_clone().map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let mut buf = [0; 4096];
        while matches!(reader.read(&mut buf), Ok(n) if n > 0) {}
    });
    Ok(stream)
}

pub type Shared = Arc<Mutex<Inhibitions>>;

pub struct Inhibit {
    pub inhibitions: Shared,
}

#[interface(name = "org.freedesktop.impl.portal.Inhibit")]
impl Inhibit {
    async fn inhibit(
        &self,
        handle: ObjectPath<'_>,
        app_id: &str,
        _window: &str,
        flags: u32,
        _options: std::collections::HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<()> {
        let path = handle.to_string();
        // The app ends the inhibition by closing this request (xdg-desktop-portal also does when the app goes away).
        server.at(&handle, Request { path: path.clone(), inhibitions: self.inhibitions.clone() }).await?;
        if flags & IDLE != 0 {
            tracing::info!("{app_id} keeps the session awake");
            let inhibitions = self.inhibitions.clone();
            blocking::unblock(move || {
                let mut inhibitions = inhibitions.lock().unwrap();
                inhibitions.handles.insert(path);
                inhibitions.sync();
            })
            .await;
        }
        Ok(())
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        3
    }
}

struct Request {
    path: String,
    inhibitions: Shared,
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    async fn close(&self, #[zbus(object_server)] server: &ObjectServer) {
        let (path, inhibitions) = (self.path.clone(), self.inhibitions.clone());
        blocking::unblock(move || {
            let mut inhibitions = inhibitions.lock().unwrap();
            if inhibitions.handles.remove(&path) {
                inhibitions.sync();
            }
        })
        .await;
        let _ = server.remove::<Request, _>(self.path.as_str()).await;
    }
}
