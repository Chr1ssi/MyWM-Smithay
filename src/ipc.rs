//! Bar socket (`$MYWM_SOCKET`), speaking the `v1` protocol of `mywm-ipc`.
//! Bounded and nonblocking: a stuck or misbehaving client is dropped, never waited for.
use std::{
    io::{self, Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
};

use mywm_config::Action;
use mywm_ipc::{Chosen, Command, INVALID, OK, parse_command};
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction, generic::Generic};

use crate::State;

const MAX_CLIENTS: usize = 16;
const MAX_LINE: usize = 4096;
const MAX_PENDING_OUTPUT: usize = 65536;

struct Client {
    id: u64,
    socket: UnixStream,
    input: Vec<u8>,
    output: Vec<u8>,
    /// Last snapshot sent; empty until the first one.
    snapshot: String,
    /// A write failed. The client is only removed once its input is drained, so a
    /// one-shot sender that already hung up still gets its commands carried out.
    broken: bool,
}

impl Client {
    /// Write as much as the socket takes; false if the client is gone or hopelessly behind.
    fn write_pending(&mut self) -> bool {
        if self.output.len() > MAX_PENDING_OUTPUT {
            return false;
        }
        while !self.output.is_empty() {
            match self.socket.write(&self.output) {
                Ok(0) => return false,
                Ok(n) => {
                    self.output.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return false,
            }
        }
        true
    }
}

pub struct Ipc {
    path: PathBuf,
    handle: LoopHandle<'static, State>,
    clients: Vec<Client>,
    next_id: u64,
}

impl Drop for Ipc {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Bind the listener. A stale socket file from a crashed session is replaced;
/// a socket that still answers belongs to a live session and is left alone.
fn bind(path: &PathBuf) -> io::Result<UnixListener> {
    match UnixListener::bind(path) {
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).is_ok() {
                return Err(e);
            }
            std::fs::remove_file(path)?;
            UnixListener::bind(path)
        }
        other => other,
    }
}

pub fn init(handle: &LoopHandle<'static, State>, state: &mut State) -> io::Result<()> {
    let Some(path) = std::env::var_os("MYWM_SOCKET").map(PathBuf::from) else {
        return Ok(());
    };
    let listener = bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    state.ipc = Some(Ipc { path, handle: handle.clone(), clients: Vec::new(), next_id: 1 });
    handle
        .insert_source(Generic::new(listener, Interest::READ, Mode::Level), |_, listener, state| {
            // SAFETY: the listener is only used to accept, never dropped from here.
            let listener = unsafe { listener.get_mut() };
            for _ in 0..MAX_CLIENTS {
                match listener.accept() {
                    Ok((socket, _)) => state.ipc_accept(socket),
                    Err(_) => break,
                }
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| io::Error::other(e.to_string()))?;
    tracing::info!("bar socket at {}", state.ipc.as_ref().unwrap().path.display());
    Ok(())
}

impl State {
    fn ipc_accept(&mut self, socket: UnixStream) {
        let Some(ipc) = &mut self.ipc else { return };
        if ipc.clients.len() >= MAX_CLIENTS || socket.set_nonblocking(true).is_err() {
            return;
        }
        let Ok(watched) = socket.try_clone() else { return };
        let id = ipc.next_id;
        ipc.next_id += 1;
        let registered = ipc.handle.insert_source(
            Generic::new(watched, Interest::READ, Mode::Level),
            move |_, _, state| Ok(state.ipc_readable(id)),
        );
        if registered.is_ok() {
            ipc.clients.push(Client {
                id,
                socket,
                input: Vec::new(),
                output: Vec::new(),
                snapshot: String::new(),
                broken: false,
            });
            // The new client gets the current state right away.
            self.ipc_dirty = true;
        }
    }

    fn ipc_readable(&mut self, id: u64) -> PostAction {
        // Take the client out so commands can borrow the whole state.
        let Some(mut ipc) = self.ipc.take() else { return PostAction::Remove };
        let Some(index) = ipc.clients.iter().position(|c| c.id == id) else {
            self.ipc = Some(ipc);
            return PostAction::Remove;
        };
        let mut client = ipc.clients.swap_remove(index);
        let mut alive = true;
        let mut buf = [0; 1024];
        // Bound the work per wakeup even if a peer keeps writing.
        for _ in 0..4 {
            match client.socket.read(&mut buf) {
                // One-shot senders close right after writing; their commands still count.
                Ok(0) => {
                    alive = false;
                    break;
                }
                Ok(n) => client.input.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => {
                    self.ipc = Some(ipc);
                    return PostAction::Remove;
                }
            }
        }
        if client.input.len() > MAX_LINE {
            self.ipc = Some(ipc);
            return PostAction::Remove;
        }
        while let Some(end) = client.input.iter().position(|c| *c == b'\n') {
            let line: Vec<u8> = client.input.drain(..=end).collect();
            let valid = std::str::from_utf8(&line)
                .ok()
                .and_then(parse_command)
                .is_some_and(|command| self.run_ipc_command(command, id));
            client.output.extend_from_slice(if valid { OK } else { INVALID }.as_bytes());
        }
        alive &= !client.broken && client.write_pending();
        if alive {
            ipc.clients.push(client);
        }
        self.ipc = Some(ipc);
        if alive { PostAction::Continue } else { PostAction::Remove }
    }

    /// Whether the command was valid; valid ones have been carried out.
    fn run_ipc_command(&mut self, command: Command, client: u64) -> bool {
        if self.session_lock.is_active() && command != Command::Lock {
            return false;
        }
        match command {
            Command::ChooseSource(kinds) => return self.start_share_chooser(client, kinds),
            Command::Lock => self.run_action(Action::Lock),
            Command::Logout => self.run_action(Action::Exit),
            Command::ThemeReload => self.run_action(Action::Reload),
            Command::Scratchpad => self.run_action(Action::ToggleScratchpad),
            Command::NewWorkspace { output } => {
                let Some(monitor) = self.monitor_for_output_id(output) else { return false };
                self.new_workspace_on(monitor);
            }
            Command::Workspace { output, number } => {
                let Some(monitor) = self.monitor_for_output_id(output) else { return false };
                if !self.desktop.desk.monitors[monitor].workspaces.contains(number) {
                    return false;
                }
                self.select_workspace_on(monitor, number);
            }
        }
        true
    }

    /// Tell a client the outcome of its `choose-source` (if it is still connected).
    pub fn ipc_send_chosen(&mut self, client: u64, chosen: &Chosen) {
        let Some(ipc) = &mut self.ipc else { return };
        if let Some(c) = ipc.clients.iter_mut().find(|c| c.id == client) {
            c.output.extend_from_slice(chosen.encode().as_bytes());
            c.broken = !c.write_pending();
        }
    }

    fn monitor_for_output_id(&self, id: u32) -> Option<usize> {
        self.outputs.iter().position(|e| e.id == id)
    }

    /// Send the current state to every client whose copy is outdated.
    pub fn ipc_flush(&mut self) {
        if !self.ipc_dirty {
            return;
        }
        self.ipc_dirty = false;
        if self.ipc.as_ref().is_none_or(|ipc| ipc.clients.is_empty()) {
            return;
        }
        let snapshot = self.ipc_snapshot().encode();
        let Some(ipc) = &mut self.ipc else { return };
        // A client that falls hopelessly behind is dropped; its event source removes
        // itself on the next wakeup.
        ipc.clients.retain_mut(|client| {
            if !client.broken {
                if client.snapshot != snapshot {
                    client.output.extend_from_slice(snapshot.as_bytes());
                    client.snapshot.clone_from(&snapshot);
                }
                client.broken = !client.write_pending();
            }
            client.output.len() <= MAX_PENDING_OUTPUT
        });
    }
}
