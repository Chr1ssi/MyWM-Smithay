//! One screen-cast stream: captures a monitor or window from the compositor with
//! `ext-image-copy-capture` and publishes the frames as a PipeWire video source.
//!
//! Frames are captured straight into the PipeWire buffers (their shared memory backs the
//! `wl_buffer`), so there is no copy in this process. Everything runs on one thread whose
//! PipeWire loop also watches the Wayland connection.
use std::{
    cell::RefCell,
    collections::HashMap,
    os::fd::{AsFd, FromRawFd},
    rc::Rc,
    sync::mpsc,
    thread::JoinHandle,
};

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
    globals::{GlobalList, GlobalListContents, registry_queue_init},
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
}

impl Source {
    /// `source_type` of the portal: 1 monitor, 2 window.
    pub fn portal_type(&self) -> u32 {
        match self {
            Source::Monitor(_) => 1,
            Source::Window(_) => 2,
        }
    }
}

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
    constraints_changed: bool,
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
            Event::BufferSize { width, height } => {
                if state.size.is_some_and(|s| s != (width, height)) {
                    state.constraints_changed = true;
                }
                state.size = Some((width, height));
            }
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

// --- PipeWire side ------------------------------------------------------------------------

/// Everything the callbacks share (single-threaded).
struct Shared {
    conn: Connection,
    queue: EventQueue<Wl>,
    wl: Wl,
    qh: QueueHandle<Wl>,
    shm: WlShm,
    session: ExtImageCopyCaptureSessionV1,
    // Negotiated.
    format: Option<(VideoFormat, (u32, u32))>,
    streaming: bool,
    node_reported: bool,
    /// PipeWire buffer -> the wl_buffer over its memory.
    buffers: HashMap<usize, (WlBuffer, std::os::fd::OwnedFd)>,
    /// The buffer being captured into, and its frame object.
    in_flight: Option<(*mut pw::sys::pw_buffer, ExtImageCopyCaptureFrameV1)>,
    ready_tx: Option<mpsc::Sender<Result<Started, String>>>,
    /// The stream ended without being asked to.
    ended_itself: bool,
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
            self.ended_itself = true;
            return;
        }
        if self.wl.constraints_changed {
            // The window or monitor changed size: offer the new size.
            self.wl.constraints_changed = false;
            if let Some(size) = self.wl.size {
                let bytes = format_pod(size);
                if let Some(pod) = Pod::from_bytes(&bytes) {
                    let _ = stream.update_params(&mut [pod]);
                }
            }
        }
        if let Some(outcome) = self.wl.frame.take() {
            self.finish_frame(stream, outcome);
        }
        self.request_frame(stream);
        self.flush();
    }

    /// Start a capture into a free PipeWire buffer if the stream wants frames.
    fn request_frame(&mut self, stream: &StreamRef) {
        if !self.streaming || self.in_flight.is_some() || self.wl.stopped {
            return;
        }
        let Some((format, size)) = self.format else { return };
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
        let _ = (format, size);
        let frame = self.session.create_frame(&self.qh, ());
        frame.attach_buffer(&wl_buffer);
        frame.damage_buffer(0, 0, size.0 as i32, size.1 as i32);
        frame.capture();
        self.in_flight = Some((raw, frame));
    }

    fn finish_frame(&mut self, stream: &StreamRef, outcome: Result<(), FailureReason>) {
        let Some((raw, frame)) = self.in_flight.take() else { return };
        frame.destroy();
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
            let _ = stream.trigger_process();
        } else if matches!(outcome, Err(FailureReason::Stopped)) {
            self.wl.stopped = true;
            self.ended_itself = true;
        }
    }
}

/// The Wayland side once the compositor has announced what it can capture.
type WaylandParts = (Connection, EventQueue<Wl>, Wl, GlobalList, ExtImageCopyCaptureSessionV1, WlShm);

fn setup_wayland(source: &Source, cursor: bool) -> Result<WaylandParts, String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("cannot connect to the compositor: {e}"))?;
    let (globals, mut queue) = registry_queue_init::<Wl>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let mut wl = Wl::default();
    let need = |what: &str| format!("the compositor does not offer {what}");
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).map_err(|_| need("wl_shm"))?;
    let copy: ExtImageCopyCaptureManagerV1 = globals.bind(&qh, 1..=1, ()).map_err(|_| need("ext-image-copy-capture"))?;
    let source_object = match source {
        Source::Monitor(name) => {
            let manager: ExtOutputImageCaptureSourceManagerV1 = globals.bind(&qh, 1..=1, ()).map_err(|_| need("ext-image-capture-source"))?;
            // Bind every output to learn its name.
            let outputs: Vec<(u32, u32)> = globals.contents().with_list(|list| {
                list.iter().filter(|g| g.interface == "wl_output").map(|g| (g.name, g.version)).collect()
            });
            for (global, version) in outputs {
                let output: WlOutput = globals.registry().bind(global, version.min(4), &qh, ());
                wl.outputs.push((output, None));
            }
            queue.roundtrip(&mut wl).map_err(|e| e.to_string())?;
            let output = wl
                .outputs
                .iter()
                .find(|(_, n)| n.as_deref() == Some(name.as_str()))
                .map(|(o, _)| o.clone())
                .ok_or_else(|| format!("no monitor called {name}"))?;
            manager.create_source(&output, &qh, ())
        }
        Source::Window(identifier) => {
            let manager: ExtForeignToplevelImageCaptureSourceManagerV1 = globals.bind(&qh, 1..=1, ()).map_err(|_| need("ext-image-capture-source"))?;
            let _list: ExtForeignToplevelListV1 = globals.bind(&qh, 1..=1, ()).map_err(|_| need("ext-foreign-toplevel-list"))?;
            queue.roundtrip(&mut wl).map_err(|e| e.to_string())?;
            queue.roundtrip(&mut wl).map_err(|e| e.to_string())?;
            let handle = wl
                .toplevels
                .iter()
                .find(|(_, id)| id.as_deref() == Some(identifier.as_str()))
                .map(|(h, _)| h.clone())
                .ok_or("the window is gone")?;
            manager.create_source(&handle, &qh, ())
        }
    };
    let options = if cursor { Options::PaintCursors } else { Options::empty() };
    let session = copy.create_session(&source_object, options, &qh, ());
    while !wl.constraints_done {
        queue.blocking_dispatch(&mut wl).map_err(|e| e.to_string())?;
        if wl.stopped {
            return Err("the source is not available for capture".into());
        }
    }
    Ok((conn, queue, wl, globals, session, shm))
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
    let (conn, queue, wl, _globals, session, shm) = match setup_wayland(&source, cursor) {
        Ok(parts) => parts,
        Err(message) => return fail(&ready_tx, message),
    };
    let Some(size) = wl.size else { return fail(&ready_tx, "the compositor announced no size".into()) };
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
        session,
        format: None,
        streaming: false,
        node_reported: false,
        buffers: HashMap::new(),
        in_flight: None,
        ready_tx: Some(ready_tx.clone()),
        ended_itself: false,
    }));

    let stream = match pw::stream::StreamRc::new(
        core.clone(),
        "mywm-screencast",
        properties! {
            *pw::keys::MEDIA_CLASS => "Video/Source",
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
            *pw::keys::NODE_NAME => "mywm-screencast",
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
                        s.streaming = false;
                        if !s.node_reported {
                            s.node_reported = true;
                            if let Some(tx) = s.ready_tx.take() {
                                let size = s.wl.size.unwrap_or((0, 0));
                                let _ = tx.send(Ok(Started { node_id: stream.node_id(), size }));
                            }
                        }
                    }
                    StreamState::Streaming => {
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
            if let Ok(mut s) = shared.try_borrow_mut() {
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
                let raw_fd = libc::memfd_create(c"mywm-screencast".as_ptr(), libc::MFD_CLOEXEC);
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
                s.flush();
            }
        })
        .remove_buffer(|_, shared, raw| {
            if let Ok(mut s) = shared.try_borrow_mut()
                && let Some((buffer, _fd)) = s.buffers.remove(&(raw as usize))
            {
                // A capture into this buffer is pointless now; the buffer is gone.
                if s.in_flight.as_ref().is_some_and(|(p, _)| *p == raw)
                    && let Some((_, frame)) = s.in_flight.take()
                {
                    frame.destroy();
                }
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
    // A buffer may be missing right when a frame is wanted; look again now and then.
    let timer = mainloop.loop_().add_timer({
        let (shared, stream) = (shared.clone(), stream.clone());
        move |_| {
            if let Ok(mut s) = shared.try_borrow_mut() {
                s.request_frame(&stream);
                s.flush();
            }
        }
    });
    let interval = std::time::Duration::from_millis(8);
    let _ = timer.update_timer(Some(interval), Some(interval));
    let _stop = stop_rx.attach(mainloop.loop_(), {
        let quit = quit.clone();
        move |()| quit.quit()
    });

    mainloop.run();
    let _ = stream.disconnect();
    shared.try_borrow().map(|s| s.ended_itself).unwrap_or(false)
}
