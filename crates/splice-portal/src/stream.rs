//! One screen-cast stream: captures a monitor or window from the compositor with
//! `ext-image-copy-capture` and publishes the frames as a PipeWire video source.
//!
//! A dynamic stream follows the compositor's cast target (`v1 cast-target`, switched with keys):
//! it replaces its capture session when the target changes and sends black frames while there is
//! none, so a consumer never keeps showing the last picture of something that was taken away.
//!
//! Frames are captured straight into the PipeWire buffers (their shared memory backs the
//! `wl_buffer`), so there is no copy in this process. Everything runs on one thread whose
//! PipeWire loop also watches the Wayland connection.
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    io::{Read, Write},
    os::fd::{AsFd, AsRawFd, FromRawFd},
    os::unix::net::UnixStream,
    rc::Rc,
    sync::mpsc,
    thread::JoinHandle,
};

use splice_ipc::CastTarget;

use pipewire as pw;
use pw::{
    properties::properties,
    spa::{
        self,
        param::video::{VideoFormat, VideoInfoRaw},
        pod::{self, Pod, Property, Value, serialize::PodSerializer},
        sys as spa_sys,
        utils::{Choice, ChoiceEnum, ChoiceFlags, Fraction, Id, Rectangle},
    },
    stream::{Stream as StreamRef, StreamFlags, StreamState},
};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, WEnum,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer::WlBuffer,
        wl_output::{self, WlOutput},
        wl_registry::WlRegistry,
        wl_shm::{self, WlShm},
        wl_shm_pool::WlShmPool,
    },
};
use wayland_protocols::ext::{
    foreign_toplevel_list::v1::client::{
        ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
        ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
    },
    image_capture_source::v1::client::{
        ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1, FailureReason},
        ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    },
};

#[derive(Clone, Debug)]
pub enum Source {
    Monitor(String),
    Window(String),
    /// Whatever the compositor's cast target is (see the module documentation).
    Dynamic,
}

impl Source {
    /// `source_type` of the portal: 1 monitor, 2 window. A dynamic stream counts as a monitor.
    pub fn portal_type(&self) -> u32 {
        match self {
            Source::Monitor(_) | Source::Dynamic => 1,
            Source::Window(_) => 2,
        }
    }
}

/// The size a dynamic stream starts with, black, before it is given a target.
const DYNAMIC_START_SIZE: (u32, u32) = (1280, 720);

pub struct Started {
    pub node_id: u32,
    pub size: (u32, u32),
}

pub struct StreamHandle {
    stop: pw::channel::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl StreamHandle {
    pub fn stop(mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Start capturing `source`; returns once the PipeWire node exists. `on_end` runs when the
/// source goes away (window closed, output removed) and the stream ends by itself.
pub fn start(
    source: Source,
    cursor: bool,
    on_end: impl FnOnce() + Send + 'static,
) -> Result<(Started, StreamHandle), String> {
    let (ready_tx, ready_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = pw::channel::channel::<()>();
    let thread = std::thread::Builder::new()
        .name("screencast".into())
        .spawn(move || {
            let ended_by_itself = run(source, cursor, ready_tx, stop_rx);
            if ended_by_itself {
                on_end();
            }
        })
        .map_err(|e| e.to_string())?;
    match ready_rx.recv() {
        Ok(Ok(started)) => Ok((started, StreamHandle { stop: stop_tx, thread: Some(thread) })),
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(error)
        }
        Err(_) => Err("the capture thread died".into()),
    }
}

// --- Wayland side -------------------------------------------------------------------------

#[derive(Default)]
struct Wl {
    outputs: Vec<(WlOutput, Option<String>)>,
    toplevels: Vec<(ExtForeignToplevelHandleV1, Option<String>)>,
    // Session constraints.
    size: Option<(u32, u32)>,
    constraints_done: bool,
    stopped: bool,
    // Frame outcome, set by events.
    frame: Option<Result<(), FailureReason>>,
}

impl Dispatch<WlRegistry, GlobalListContents> for Wl {
    fn event(_: &mut Self, _: &WlRegistry, _: wayland_client::protocol::wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlOutput, ()> for Wl {
    fn event(state: &mut Self, output: &WlOutput, event: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Name { name } = event
            && let Some(entry) = state.outputs.iter_mut().find(|(o, _)| o == output)
        {
            entry.1 = Some(name);
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for Wl {
    fn event(state: &mut Self, _: &ExtForeignToplevelListV1, event: ext_foreign_toplevel_list_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state.toplevels.push((toplevel, None));
        }
    }
    wayland_client::event_created_child!(Wl, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Wl {
    fn event(state: &mut Self, handle: &ExtForeignToplevelHandleV1, event: ext_foreign_toplevel_handle_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } = event
            && let Some(entry) = state.toplevels.iter_mut().find(|(h, _)| h == handle)
        {
            entry.1 = Some(identifier);
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for Wl {
    fn event(state: &mut Self, _: &ExtImageCopyCaptureSessionV1, event: ext_image_copy_capture_session_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use ext_image_copy_capture_session_v1::Event;
        match event {
            Event::BufferSize { width, height } => state.size = Some((width, height)),
            Event::Done => state.constraints_done = true,
            Event::Stopped => state.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for Wl {
    fn event(state: &mut Self, _: &ExtImageCopyCaptureFrameV1, event: ext_image_copy_capture_frame_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => state.frame = Some(Ok(())),
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                state.frame = Some(Err(match reason {
                    WEnum::Value(reason) => reason,
                    WEnum::Unknown(_) => FailureReason::Unknown,
                }))
            }
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(Wl: ignore WlShm);
wayland_client::delegate_noop!(Wl: ignore WlShmPool);
wayland_client::delegate_noop!(Wl: ignore WlBuffer);
wayland_client::delegate_noop!(Wl: ignore ExtImageCaptureSourceV1);
wayland_client::delegate_noop!(Wl: ignore ExtOutputImageCaptureSourceManagerV1);
wayland_client::delegate_noop!(Wl: ignore ExtForeignToplevelImageCaptureSourceManagerV1);
wayland_client::delegate_noop!(Wl: ignore ExtImageCopyCaptureManagerV1);

/// What a stream needs to open capture sessions, also later ones (a dynamic stream's).
struct Capture {
    copy: ExtImageCopyCaptureManagerV1,
    outputs: ExtOutputImageCaptureSourceManagerV1,
    toplevels: ExtForeignToplevelImageCaptureSourceManagerV1,
    /// Kept so the toplevels' identifiers stay known.
    _list: ExtForeignToplevelListV1,
    cursor: bool,
}

impl Capture {
    /// A session for a monitor or window by name; `None` if it is not there (any more).
    fn open(&self, wl: &Wl, qh: &QueueHandle<Wl>, target: &CastTarget) -> Option<ExtImageCopyCaptureSessionV1> {
        let source = match target {
            CastTarget::Nothing => return None,
            CastTarget::Monitor(name) => {
                let output = wl.outputs.iter().find(|(_, n)| n.as_deref() == Some(name.as_str()))?.0.clone();
                self.outputs.create_source(&output, qh, ())
            }
            CastTarget::Window(identifier) => {
                let handle = wl.toplevels.iter().find(|(_, id)| id.as_deref() == Some(identifier.as_str()))?.0.clone();
                self.toplevels.create_source(&handle, qh, ())
            }
        };
        let options = if self.cursor { Options::PaintCursors } else { Options::empty() };
        let session = self.copy.create_session(&source, options, qh, ());
        source.destroy();
        Some(session)
    }
}

// --- PipeWire side ------------------------------------------------------------------------

/// Everything the callbacks share (single-threaded).
struct Shared {
    conn: Connection,
    queue: EventQueue<Wl>,
    wl: Wl,
    qh: QueueHandle<Wl>,
    shm: WlShm,
    capture: Capture,
    /// None for a dynamic stream without a target: it sends black frames.
    session: Option<ExtImageCopyCaptureSessionV1>,
    /// The compositor connection a dynamic stream hears its targets on, and what arrived of a line.
    follow: Option<(UnixStream, Vec<u8>)>,
    /// The size last offered to the consumer.
    offered: (u32, u32),
    /// Buffers already black (a dynamic stream without a target fills each once).
    blackened: HashSet<usize>,
    // Negotiated.
    format: Option<(VideoFormat, (u32, u32))>,
    streaming: bool,
    node_reported: bool,
    /// PipeWire buffer -> the wl_buffer over its memory.
    buffers: HashMap<usize, (WlBuffer, std::os::fd::OwnedFd)>,
    /// The buffer being captured into, and its frame object (none for a black frame).
    in_flight: Option<(*mut pw::sys::pw_buffer, Option<ExtImageCopyCaptureFrameV1>)>,
    ready_tx: Option<mpsc::Sender<Result<Started, String>>>,
    /// The stream ended without being asked to.
    ended_itself: bool,
    /// Frames handed to PipeWire since the last statistics line, and when that was.
    delivered: u32,
    /// Shortest time between two captures: the frame rate the consumer negotiated.
    frame_interval: std::time::Duration,
    /// Earliest start of the next capture.
    next_capture: std::time::Instant,
    last_stats: std::time::Instant,
}

fn pod_bytes(value: Value) -> Vec<u8> {
    PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &value).expect("serialize pod").0.into_inner()
}

fn id_choice(default: u32, alternatives: &[u32]) -> Value {
    Value::Choice(pod::ChoiceValue::Id(Choice(
        ChoiceFlags::empty(),
        ChoiceEnum::Enum { default: Id(default), alternatives: alternatives.iter().map(|a| Id(*a)).collect() },
    )))
}

/// The formats offered for a stream of `size`.
fn format_pod(size: (u32, u32)) -> Vec<u8> {
    let object = pod::Object {
        type_: spa_sys::SPA_TYPE_OBJECT_Format,
        id: spa_sys::SPA_PARAM_EnumFormat,
        properties: vec![
            Property::new(spa_sys::SPA_FORMAT_mediaType, Value::Id(Id(spa_sys::SPA_MEDIA_TYPE_video))),
            Property::new(spa_sys::SPA_FORMAT_mediaSubtype, Value::Id(Id(spa_sys::SPA_MEDIA_SUBTYPE_raw))),
            Property::new(
                spa_sys::SPA_FORMAT_VIDEO_format,
                id_choice(spa_sys::SPA_VIDEO_FORMAT_BGRx, &[spa_sys::SPA_VIDEO_FORMAT_BGRx, spa_sys::SPA_VIDEO_FORMAT_BGRA]),
            ),
            Property::new(spa_sys::SPA_FORMAT_VIDEO_size, Value::Rectangle(Rectangle { width: size.0, height: size.1 })),
            Property::new(
                spa_sys::SPA_FORMAT_VIDEO_framerate,
                Value::Choice(pod::ChoiceValue::Fraction(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: Fraction { num: 60, denom: 1 },
                        min: Fraction { num: 0, denom: 1 },
                        max: Fraction { num: 1000, denom: 1 },
                    },
                ))),
            ),
        ],
    };
    pod_bytes(Value::Object(object))
}

/// Buffer requirements once the format is settled: shared memory (memfd), one plane.
fn buffers_pod(size: (u32, u32)) -> Vec<u8> {
    let stride = size.0 * 4;
    let int = |key, value: i32| Property::new(key, Value::Int(value));
    let object = pod::Object {
        type_: spa_sys::SPA_TYPE_OBJECT_ParamBuffers,
        id: spa_sys::SPA_PARAM_Buffers,
        properties: vec![
            Property::new(
                spa_sys::SPA_PARAM_BUFFERS_buffers,
                Value::Choice(pod::ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range { default: 4, min: 2, max: 16 },
                ))),
            ),
            int(spa_sys::SPA_PARAM_BUFFERS_blocks, 1),
            int(spa_sys::SPA_PARAM_BUFFERS_size, (stride * size.1) as i32),
            int(spa_sys::SPA_PARAM_BUFFERS_stride, stride as i32),
            int(spa_sys::SPA_PARAM_BUFFERS_align, 16),
            int(spa_sys::SPA_PARAM_BUFFERS_dataType, 1 << spa_sys::SPA_DATA_MemFd),
        ],
    };
    pod_bytes(Value::Object(object))
}

fn header_meta_pod() -> Vec<u8> {
    let object = pod::Object {
        type_: spa_sys::SPA_TYPE_OBJECT_ParamMeta,
        id: spa_sys::SPA_PARAM_Meta,
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_META_type, Value::Id(Id(spa_sys::SPA_META_Header))),
            Property::new(spa_sys::SPA_PARAM_META_size, Value::Int(std::mem::size_of::<spa_sys::spa_meta_header>() as i32)),
        ],
    };
    pod_bytes(Value::Object(object))
}

impl Shared {
    fn flush(&self) {
        let _ = self.conn.flush();
    }

    /// Read what the compositor sent and act on it.
    fn pump_wayland(&mut self, stream: &StreamRef) {
        if let Some(guard) = self.queue.prepare_read() {
            let _ = guard.read();
        }
        let _ = self.queue.dispatch_pending(&mut self.wl);
        self.after_events(stream);
    }

    fn after_events(&mut self, stream: &StreamRef) {
        if self.wl.stopped {
            // A dynamic stream outlives what it showed: it goes black until the next target.
            if self.follow.is_some() {
                self.retarget(stream, &CastTarget::Nothing);
            } else {
                self.ended_itself = true;
                return;
            }
        }
        // The window or monitor changed size, or a new target has another: offer the new size.
        if let Some(size) = self.wl.size.filter(|_| self.session.is_some() && self.wl.constraints_done)
            && size != self.offered
        {
            self.offer(stream, size);
        }
        if let Some(outcome) = self.wl.frame.take() {
            self.finish_frame(stream, outcome);
        }
        self.request_frame(stream);
        self.flush();
    }

    fn offer(&mut self, stream: &StreamRef, size: (u32, u32)) {
        self.offered = size;
        let bytes = format_pod(size);
        if let Some(pod) = Pod::from_bytes(&bytes) {
            let _ = stream.update_params(&mut [pod]);
        }
    }

    /// Show `target` from now on: the capture in flight is dropped, the old session closed, a new one
    /// opened; without a target (or when it is gone) the stream goes black.
    fn retarget(&mut self, stream: &StreamRef, target: &CastTarget) {
        if self.in_flight.is_some() {
            self.finish_frame(stream, Err(FailureReason::Unknown));
        }
        if let Some(session) = self.session.take() {
            session.destroy();
        }
        self.wl.size = None;
        self.wl.constraints_done = false;
        self.wl.stopped = false;
        self.wl.frame = None;
        self.session = self.capture.open(&self.wl, &self.qh, target);
        if self.session.is_none() {
            // The buffers hold the last picture: black them again.
            self.blackened.clear();
        }
        tracing::info!("screen cast: now showing {target:?}");
        self.flush();
    }

    /// Lines from the compositor connection of a dynamic stream: the targets.
    fn read_targets(&mut self, stream: &StreamRef) {
        let Some((connection, pending)) = &mut self.follow else { return };
        let mut buf = [0; 1024];
        let mut gone = false;
        loop {
            match connection.read(&mut buf) {
                Ok(0) => {
                    gone = true;
                    break;
                }
                Ok(n) => pending.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        let mut targets = Vec::new();
        while let Some(end) = pending.iter().position(|c| *c == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            if let Some(target) = std::str::from_utf8(&line).ok().and_then(CastTarget::parse) {
                targets.push(target);
            }
        }
        if gone {
            // The compositor went away: nothing will be shown any more.
            self.follow = None;
            targets.push(CastTarget::Nothing);
        }
        for target in targets {
            self.retarget(stream, &target);
        }
    }

    /// A black frame for a dynamic stream without a target: every buffer is filled once.
    fn black_frame(&mut self, stream: &StreamRef, size: (u32, u32)) {
        // SAFETY: a buffer we get here is handed back with `queue_raw_buffer` exactly once.
        let raw = unsafe { stream.dequeue_raw_buffer() };
        if raw.is_null() {
            return;
        }
        if !self.blackened.contains(&(raw as usize))
            && let Some((_, fd)) = self.buffers.get(&(raw as usize))
        {
            let zeros = vec![0u8; (size.0 * size.1 * 4) as usize];
            // SAFETY: writes into our own memfd, at most its length.
            let written = unsafe { libc::pwrite(fd.as_raw_fd(), zeros.as_ptr().cast(), zeros.len(), 0) };
            if written == zeros.len() as isize {
                self.blackened.insert(raw as usize);
            }
        }
        self.in_flight = Some((raw, None));
        self.finish_frame(stream, Ok(()));
    }

    /// Start a capture into a free PipeWire buffer if the stream wants frames.
    fn request_frame(&mut self, stream: &StreamRef) {
        if !self.streaming || self.in_flight.is_some() || self.wl.stopped {
            return;
        }
        let Some((format, size)) = self.format else { return };
        // The consumer asked for at most so many frames a second: a game at 144 fps must not
        // cost 144 full-size copies a second.
        let now = std::time::Instant::now();
        if now < self.next_capture {
            return;
        }
        self.next_capture = if now.duration_since(self.next_capture) > self.frame_interval { now + self.frame_interval } else { self.next_capture + self.frame_interval };
        let Some(session) = self.session.clone() else {
            return self.black_frame(stream, size);
        };
        // A new session first agrees on its size with the consumer.
        if !self.wl.constraints_done || self.wl.size != Some(size) {
            return;
        }
        // SAFETY: a buffer we get here is handed back with `queue_raw_buffer` exactly once.
        let raw = unsafe { stream.dequeue_raw_buffer() };
        if raw.is_null() {
            return;
        }
        let Some(wl_buffer) = self.buffers.get(&(raw as usize)).map(|(b, _)| b.clone()) else {
            // Not one of ours (should not happen): give it back untouched.
            unsafe { stream.queue_raw_buffer(raw) };
            return;
        };
        let _ = format;
        let frame = session.create_frame(&self.qh, ());
        frame.attach_buffer(&wl_buffer);
        frame.damage_buffer(0, 0, size.0 as i32, size.1 as i32);
        frame.capture();
        self.in_flight = Some((raw, Some(frame)));
    }

    fn finish_frame(&mut self, stream: &StreamRef, outcome: Result<(), FailureReason>) {
        let Some((raw, frame)) = self.in_flight.take() else { return };
        if let Some(frame) = frame {
            frame.destroy();
        }
        let Some((_, size)) = self.format else { return };
        // SAFETY: `raw` came from `dequeue_raw_buffer` and is valid until queued.
        unsafe {
            let spa_buffer = (*raw).buffer;
            if !spa_buffer.is_null() && (*spa_buffer).n_datas > 0 {
                let data = &mut *(*spa_buffer).datas;
                if !data.chunk.is_null() {
                    let chunk = &mut *data.chunk;
                    chunk.offset = 0;
                    chunk.stride = (size.0 * 4) as i32;
                    match outcome {
                        Ok(()) => {
                            chunk.size = size.0 * size.1 * 4;
                            chunk.flags = 0;
                        }
                        Err(_) => {
                            chunk.size = 0;
                            chunk.flags = spa_sys::SPA_CHUNK_FLAG_CORRUPTED as i32;
                        }
                    }
                }
            }
            if !(*raw).buffer.is_null() {
                let metas = (*(*raw).buffer).metas;
                let n_metas = (*(*raw).buffer).n_metas;
                for i in 0..n_metas as usize {
                    let meta = &*metas.add(i);
                    if meta.type_ == spa_sys::SPA_META_Header && !meta.data.is_null() {
                        let header = &mut *(meta.data as *mut spa_sys::spa_meta_header);
                        let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
                        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now);
                        header.pts = now.tv_sec * 1_000_000_000 + now.tv_nsec;
                        header.flags = 0;
                    }
                }
            }
            stream.queue_raw_buffer(raw);
        }
        if outcome.is_ok() {
            self.delivered += 1;
            let _ = stream.trigger_process();
        } else if matches!(outcome, Err(FailureReason::Stopped)) {
            self.wl.stopped = true;
        }
    }
}

/// The Wayland side once the compositor has announced what it can capture.
type WaylandParts = (Connection, EventQueue<Wl>, Wl, Capture, Option<ExtImageCopyCaptureSessionV1>, WlShm);

fn setup_wayland(source: &Source, cursor: bool) -> Result<WaylandParts, String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("cannot connect to the compositor: {e}"))?;
    let (globals, mut queue) = registry_queue_init::<Wl>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let mut wl = Wl::default();
    let need = |what: &str| format!("the compositor does not offer {what}");
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).map_err(|_| need("wl_shm"))?;
    let capture = Capture {
        copy: globals.bind(&qh, 1..=1, ()).map_err(|_| need("ext-image-copy-capture"))?,
        outputs: globals.bind(&qh, 1..=1, ()).map_err(|_| need("ext-image-capture-source"))?,
        toplevels: globals.bind(&qh, 1..=1, ()).map_err(|_| need("ext-image-capture-source"))?,
        _list: globals.bind(&qh, 1..=1, ()).map_err(|_| need("ext-foreign-toplevel-list"))?,
        cursor,
    };
    // Bind every output to learn its name.
    let outputs: Vec<(u32, u32)> =
        globals.contents().with_list(|list| list.iter().filter(|g| g.interface == "wl_output").map(|g| (g.name, g.version)).collect());
    for (global, version) in outputs {
        let output: WlOutput = globals.registry().bind(global, version.min(4), &qh, ());
        wl.outputs.push((output, None));
    }
    // Names, then the toplevels and their identifiers.
    queue.roundtrip(&mut wl).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut wl).map_err(|e| e.to_string())?;
    let target = match source {
        Source::Dynamic => return Ok((conn, queue, wl, capture, None, shm)),
        Source::Monitor(name) => CastTarget::Monitor(name.clone()),
        Source::Window(identifier) => CastTarget::Window(identifier.clone()),
    };
    let session = capture.open(&wl, &qh, &target).ok_or_else(|| match source {
        Source::Monitor(name) => format!("no monitor called {name}"),
        _ => "the window is gone".to_owned(),
    })?;
    while !wl.constraints_done {
        queue.blocking_dispatch(&mut wl).map_err(|e| e.to_string())?;
        if wl.stopped {
            return Err("the source is not available for capture".into());
        }
    }
    Ok((conn, queue, wl, capture, Some(session), shm))
}

/// Follow the compositor's cast target: `v1 cast-follow` on a connection of the stream's own.
fn follow_targets() -> Result<UnixStream, String> {
    let path = crate::chooser::socket_path().ok_or("no SPLICE_SOCKET and no XDG_RUNTIME_DIR")?;
    let mut connection = UnixStream::connect(&path).map_err(|e| format!("cannot reach the compositor at {}: {e}", path.display()))?;
    connection.write_all(b"v1 cast-follow\n").map_err(|e| e.to_string())?;
    connection.set_nonblocking(true).map_err(|e| e.to_string())?;
    Ok(connection)
}

/// Run the stream until asked to stop (or the source ends). Returns whether it ended by itself.
fn run(
    source: Source,
    cursor: bool,
    ready_tx: mpsc::Sender<Result<Started, String>>,
    stop_rx: pw::channel::Receiver<()>,
) -> bool {
    let fail = |tx: &mpsc::Sender<Result<Started, String>>, message: String| {
        tracing::warn!("screen cast: {message}");
        let _ = tx.send(Err(message));
        false
    };
    let (conn, queue, wl, capture, session, shm) = match setup_wayland(&source, cursor) {
        Ok(parts) => parts,
        Err(message) => return fail(&ready_tx, message),
    };
    let follow = match source {
        Source::Dynamic => match follow_targets() {
            Ok(connection) => Some((connection, Vec::new())),
            Err(message) => return fail(&ready_tx, message),
        },
        _ => None,
    };
    let size = if follow.is_some() { DYNAMIC_START_SIZE } else {
        let Some(size) = wl.size else { return fail(&ready_tx, "the compositor announced no size".into()) };
        size
    };
    let qh = queue.handle();

    pw::init();
    let setup = || -> Result<_, pw::Error> {
        let mainloop = pw::main_loop::MainLoopRc::new(None)?;
        let context = pw::context::ContextRc::new(&mainloop, None)?;
        let core = context.connect_rc(None)?;
        Ok((mainloop, context, core))
    };
    let (mainloop, _context, core) = match setup() {
        Ok(parts) => parts,
        Err(error) => return fail(&ready_tx, format!("cannot reach PipeWire: {error}")),
    };

    let shared = Rc::new(RefCell::new(Shared {
        conn: conn.clone(),
        queue,
        wl,
        qh,
        shm,
        capture,
        session,
        follow,
        offered: size,
        blackened: HashSet::new(),
        format: None,
        streaming: false,
        node_reported: false,
        buffers: HashMap::new(),
        in_flight: None,
        ready_tx: Some(ready_tx.clone()),
        ended_itself: false,
        delivered: 0,
        frame_interval: std::time::Duration::from_micros(16_667),
        next_capture: std::time::Instant::now(),
        last_stats: std::time::Instant::now(),
    }));

    let stream = match pw::stream::StreamRc::new(
        core.clone(),
        "splice-screencast",
        properties! {
            *pw::keys::MEDIA_CLASS => "Video/Source",
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
            *pw::keys::NODE_NAME => "splice-screencast",
        },
    ) {
        Ok(stream) => stream,
        Err(error) => return fail(&ready_tx, format!("cannot create the stream: {error}")),
    };

    let quit = mainloop.clone();
    let listener = stream
        .add_local_listener_with_user_data(shared.clone())
        .state_changed({
            let quit = quit.clone();
            move |stream, shared, _old, new| {
                let Ok(mut s) = shared.try_borrow_mut() else { return };
                match new {
                    StreamState::Paused => {
                        tracing::debug!("screen cast: stream paused");
                        s.streaming = false;
                        if !s.node_reported {
                            s.node_reported = true;
                            if let Some(tx) = s.ready_tx.take() {
                                let size = s.offered;
                                let _ = tx.send(Ok(Started { node_id: stream.node_id(), size }));
                            }
                        }
                    }
                    StreamState::Streaming => {
                        tracing::info!("screen cast: a consumer is reading the stream");
                        s.streaming = true;
                        s.request_frame(stream);
                        s.flush();
                    }
                    StreamState::Error(message) => {
                        tracing::warn!("PipeWire stream error: {message}");
                        if let Some(tx) = s.ready_tx.take() {
                            let _ = tx.send(Err(format!("PipeWire: {message}")));
                        }
                        s.ended_itself = true;
                        quit.quit();
                    }
                    _ => {}
                }
            }
        })
        .param_changed(|stream, shared, id, param| {
            if id != spa_sys::SPA_PARAM_Format {
                return;
            }
            let Some(param) = param else {
                // The consumer left: the format is cleared.
                if let Ok(mut s) = shared.try_borrow_mut() {
                    s.format = None;
                }
                return;
            };
            let mut info = VideoInfoRaw::new();
            if info.parse(param).is_err() {
                return;
            }
            let size = (info.size().width, info.size().height);
            tracing::info!("screen cast: negotiated {:?} {}x{} at up to {}/{} fps", info.format(), size.0, size.1, info.max_framerate().num, info.max_framerate().denom.max(1));
            if let Ok(mut s) = shared.try_borrow_mut() {
                let rate = info.max_framerate();
                if rate.num > 0 {
                    s.frame_interval = std::time::Duration::from_secs_f64(f64::from(rate.denom.max(1)) / f64::from(rate.num));
                }
                s.format = Some((info.format(), size));
            }
            let (buffers, meta) = (buffers_pod(size), header_meta_pod());
            if let (Some(buffers), Some(meta)) = (Pod::from_bytes(&buffers), Pod::from_bytes(&meta)) {
                let _ = stream.update_params(&mut [buffers, meta]);
            }
        })
        .add_buffer(|_, shared, raw| {
            let Ok(mut s) = shared.try_borrow_mut() else { return };
            let Some((format, size)) = s.format else { return };
            // SAFETY: PipeWire hands us a valid buffer; we give it memory of our own (a memfd)
            // that the wl_buffer and PipeWire both use, and keep the fd until the buffer goes.
            unsafe {
                let spa_buffer = (*raw).buffer;
                if spa_buffer.is_null() || (*spa_buffer).n_datas == 0 {
                    return;
                }
                let data = &mut *(*spa_buffer).datas;
                // With ALLOC_BUFFERS `type_` holds the set of memory types the consumers accept.
                if data.type_ & (1 << spa_sys::SPA_DATA_MemFd) == 0 {
                    tracing::warn!("the consumer does not accept shared-memory buffers (types {:#x})", data.type_);
                    return;
                }
                let len = (size.0 * size.1 * 4) as usize;
                let raw_fd = libc::memfd_create(c"splice-screencast".as_ptr(), libc::MFD_CLOEXEC);
                if raw_fd < 0 || libc::ftruncate(raw_fd, len as libc::off_t) != 0 {
                    tracing::warn!("cannot allocate a frame buffer: {}", std::io::Error::last_os_error());
                    return;
                }
                let fd = std::os::fd::OwnedFd::from_raw_fd(raw_fd);
                data.type_ = spa_sys::SPA_DATA_MemFd;
                data.flags = spa_sys::SPA_DATA_FLAG_READWRITE;
                data.fd = i64::from(raw_fd);
                data.mapoffset = 0;
                data.maxsize = len as u32;
                data.data = std::ptr::null_mut();
                let stride = (size.0 * 4) as i32;
                let pool = s.shm.create_pool(fd.as_fd(), len as i32, &s.qh, ());
                let wl_format = if format == VideoFormat::BGRA { wl_shm::Format::Argb8888 } else { wl_shm::Format::Xrgb8888 };
                let buffer = pool.create_buffer(0, size.0 as i32, size.1 as i32, stride, wl_format, &s.qh, ());
                pool.destroy();
                s.buffers.insert(raw as usize, (buffer, fd));
                tracing::debug!("screen cast: buffer {} added ({} bytes, shared memory)", s.buffers.len(), len);
                s.flush();
            }
        })
        .remove_buffer(|_, shared, raw| {
            if let Ok(mut s) = shared.try_borrow_mut()
                && let Some((buffer, _fd)) = s.buffers.remove(&(raw as usize))
            {
                // A capture into this buffer is pointless now; the buffer is gone.
                if s.in_flight.as_ref().is_some_and(|(p, _)| *p == raw)
                    && let Some((_, Some(frame))) = s.in_flight.take()
                {
                    frame.destroy();
                }
                s.blackened.remove(&(raw as usize));
                buffer.destroy();
            }
        })
        .register();
    let _listener = match listener {
        Ok(listener) => listener,
        Err(error) => return fail(&ready_tx, format!("cannot register the stream listener: {error}")),
    };

    let format = format_pod(size);
    let Some(format_pod) = Pod::from_bytes(&format) else { return fail(&ready_tx, "bad format pod".into()) };
    if let Err(error) = stream.connect(
        spa::utils::Direction::Output,
        None,
        StreamFlags::DRIVER | StreamFlags::ALLOC_BUFFERS,
        &mut [format_pod],
    ) {
        return fail(&ready_tx, format!("cannot connect the stream: {error}"));
    }

    // Watch the Wayland connection from the PipeWire loop.
    let wl_fd = match conn.backend().poll_fd().try_clone_to_owned() {
        Ok(fd) => fd,
        Err(error) => return fail(&ready_tx, format!("cannot watch the Wayland socket: {error}")),
    };
    let _io = mainloop.loop_().add_io(wl_fd, spa::support::system::IoFlags::IN, {
        let (shared, stream, quit) = (shared.clone(), stream.clone(), quit.clone());
        move |_| {
            let Ok(mut s) = shared.try_borrow_mut() else { return };
            s.pump_wayland(&stream);
            if s.ended_itself {
                quit.quit();
            }
        }
    });
    // A dynamic stream hears its targets on its compositor connection.
    let follow_fd = shared.borrow().follow.as_ref().and_then(|(c, _)| c.as_fd().try_clone_to_owned().ok());
    let _follow_io = follow_fd.map(|fd| {
        mainloop.loop_().add_io(fd, spa::support::system::IoFlags::IN, {
            let (shared, stream) = (shared.clone(), stream.clone());
            move |_| {
                if let Ok(mut s) = shared.try_borrow_mut() {
                    s.read_targets(&stream);
                }
            }
        })
    });
    // A buffer may be missing right when a frame is wanted; look again now and then.
    let timer = mainloop.loop_().add_timer({
        let (shared, stream) = (shared.clone(), stream.clone());
        move |_| {
            if let Ok(mut s) = shared.try_borrow_mut() {
                s.request_frame(&stream);
                s.flush();
                if s.last_stats.elapsed() >= std::time::Duration::from_secs(5) {
                    if s.streaming || s.delivered > 0 {
                        tracing::info!("screen cast: {} frames in {:.0} s", s.delivered, s.last_stats.elapsed().as_secs_f64());
                    }
                    s.delivered = 0;
                    s.last_stats = std::time::Instant::now();
                }
            }
        }
    });
    let interval = std::time::Duration::from_millis(4);
    let _ = timer.update_timer(Some(interval), Some(interval));
    let _stop = stop_rx.attach(mainloop.loop_(), {
        let quit = quit.clone();
        move |()| quit.quit()
    });

    mainloop.run();
    let _ = stream.disconnect();
    shared.try_borrow().map(|s| s.ended_itself).unwrap_or(false)
}
