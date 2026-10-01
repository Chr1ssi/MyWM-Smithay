//! Window and output capture for streaming: `ext-foreign-toplevel-list`,
//! `ext-image-capture-source` and `ext-image-copy-capture`.
//!
//! Newer portals and recorders ask for a window through these. A session names a source
//! (an output or a toplevel) and announces buffer constraints; each frame the client creates
//! is filled when the source has changed since the last one, using the renderer of the output
//! redraw in progress (like `screencopy.rs`, which this shares the rendering code with).
use std::sync::{Arc, Mutex};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            damage::OutputDamageTracker,
            element::{AsRenderElements, RenderElement, surface::WaylandSurfaceRenderElement},
            gles::{GlesRenderer, GlesTexture},
        },
    },
    output::Output,
    reexports::{
        wayland_protocols::ext::{
            image_capture_source::v1::server::{
                ext_foreign_toplevel_image_capture_source_manager_v1::{self, ExtForeignToplevelImageCaptureSourceManagerV1},
                ext_image_capture_source_v1::ExtImageCaptureSourceV1,
                ext_output_image_capture_source_manager_v1::{self, ExtOutputImageCaptureSourceManagerV1},
            },
            image_copy_capture::v1::server::{
                ext_image_copy_capture_cursor_session_v1::{self, ExtImageCopyCaptureCursorSessionV1},
                ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1, FailureReason},
                ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1, Options},
                ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
            },
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
            backend::{ClientId, GlobalId},
            protocol::{wl_buffer::WlBuffer, wl_output::Transform as WlTransform, wl_shm},
        },
    },
    utils::{Clock, Monotonic, Physical, Rectangle, Scale, Size, Transform},
    wayland::{
        dmabuf::get_dmabuf,
        foreign_toplevel_list::{ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState},
        shm::with_buffer_contents_mut,
    },
};
use smithay::backend::allocator::Buffer as _;

use crate::{
    State,
    render::OutputElement,
    screencopy::{Readback, offscreen_texture, render_to_buffer, start_readback},
};

smithay::delegate_foreign_toplevel_list!(State);

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevels
    }
}

/// What a session captures.
#[derive(Clone)]
pub enum Source {
    Output(Output),
    Toplevel(ForeignToplevelHandle),
}

/// GPU buffer constraints: the render node and, per format, the modifiers the renderer can draw to.
pub struct DmabufConstraints {
    pub device: u64,
    pub formats: Vec<(u32, Vec<u64>)>,
}

pub struct SessionShared {
    source: Source,
    paint_cursors: bool,
    inner: Mutex<SessionInner>,
}

struct SessionInner {
    /// The size last announced to the client.
    size: Size<i32, Physical>,
    has_frame: bool,
    stopped: bool,
    /// The window's commit count at the last capture (outputs: `Some(0)` after the first).
    captured: Option<u64>,
}

#[derive(Default)]
pub struct ImageCaptureState {
    sessions: Vec<(ExtImageCopyCaptureSessionV1, Arc<SessionShared>)>,
    pending: Vec<PendingFrame>,
    /// Outputs with a redraw timer for waiting frames.
    timers: std::collections::HashSet<String>,
    /// Shared-memory frames whose pixels the GPU is still copying.
    readbacks: Vec<InFlight>,
    /// Whether the timer that completes `readbacks` is armed.
    readback_timer: bool,
    /// Per session, the offscreen texture its frames are rendered into (reused while the size stays).
    textures: Vec<(std::sync::Weak<SessionShared>, Size<i32, Physical>, GlesTexture)>,
}

struct InFlight {
    frame: PendingFrame,
    readback: Readback,
    /// When the picture was rendered: its presentation time and the limit for waiting on the GPU.
    captured_at: std::time::Instant,
    timestamp: std::time::Duration,
}

/// How long a read-back may take before the event loop waits for it (a hung GPU, not a busy one).
const READBACK_LIMIT: std::time::Duration = std::time::Duration::from_millis(250);

pub struct PendingFrame {
    frame: ExtImageCopyCaptureFrameV1,
    session: Arc<SessionShared>,
    buffer: WlBuffer,
    requested: std::time::Instant,
}

/// A frame request that has waited this long gets an answer even if nothing changed, so a still
/// picture does not make a stream go dead (players and Chromium treat silence as a broken source).
const KEEPALIVE: std::time::Duration = std::time::Duration::from_millis(400);

pub struct FrameData {
    session: Arc<SessionShared>,
    inner: Mutex<FrameInner>,
}

#[derive(Default)]
struct FrameInner {
    buffer: Option<WlBuffer>,
    captured: bool,
}

pub struct ImageCaptureGlobals(#[allow(dead_code)] Vec<GlobalId>);

impl State {
    pub fn create_image_capture_globals(display: &DisplayHandle) -> ImageCaptureGlobals {
        ImageCaptureGlobals(vec![
            display.create_global::<State, ExtOutputImageCaptureSourceManagerV1, ()>(1, ()),
            display.create_global::<State, ExtForeignToplevelImageCaptureSourceManagerV1, ()>(1, ()),
            display.create_global::<State, ExtImageCopyCaptureManagerV1, ()>(1, ()),
        ])
    }

    /// Whether a capture of `output` (or of a window showing there) is waiting for a frame.
    pub fn has_image_captures_for(&self, output: &Output) -> bool {
        self.image_capture.pending.iter().any(|p| self.frame_belongs_to(p, output))
    }

    /// Whether a capture session follows `window`.
    pub fn window_captured(&self, window: &smithay::desktop::Window) -> bool {
        self.image_capture.sessions.iter().any(|(_, shared)| match &shared.source {
            Source::Toplevel(handle) => self.managed_for_handle(handle).is_some_and(|m| &m.window == window),
            Source::Output(_) => false,
        })
    }

    /// The output whose frames drive the capture of `window`: the first it shows on, or the first output
    /// when it shows on none (the same choice as for its pending capture frames).
    pub fn capture_output(&self, window: &smithay::desktop::Window) -> Option<Output> {
        self.space.outputs_for_element(window).into_iter().next().or_else(|| self.outputs.first().map(|e| e.output.clone()))
    }

    fn managed_for_handle(&self, handle: &ForeignToplevelHandle) -> Option<&crate::desktop::Managed> {
        let id = handle.identifier();
        self.desktop.windows.iter().find(|m| m.foreign.as_ref().is_some_and(|h| h.identifier() == id))
    }

    /// Size in pixels of what a source currently shows.
    fn source_size(&self, source: &Source) -> Option<Size<i32, Physical>> {
        match source {
            Source::Output(output) => output.current_mode().map(|mode| {
                let transform = if self.udev.is_some() { output.current_transform() } else { Transform::Normal };
                transform.transform_size(mode.size)
            }),
            Source::Toplevel(handle) => {
                let managed = self.managed_for_handle(handle)?;
                let scale = managed.surface().map_or(1.0, |s| self.scale_for_surface(&s));
                let size = managed.window.geometry().size;
                let size: Size<i32, Physical> = size.to_f64().to_physical(scale).to_i32_round();
                (size.w > 0 && size.h > 0).then_some(size)
            }
        }
    }

    fn send_constraints(&self, session: &ExtImageCopyCaptureSessionV1, size: Size<i32, Physical>) {
        session.buffer_size(size.w as u32, size.h as u32);
        session.shm_format(wl_shm::Format::Argb8888);
        session.shm_format(wl_shm::Format::Xrgb8888);
        if let Some(dmabuf) = self.udev.as_ref().and_then(|u| u.capture_dmabuf_constraints()) {
            session.dmabuf_device(dmabuf.device.to_ne_bytes().to_vec());
            for (format, modifiers) in dmabuf.formats {
                let bytes = modifiers.iter().flat_map(|m| m.to_ne_bytes()).collect();
                session.dmabuf_format(format, bytes);
            }
        }
        session.done();
    }

    fn frame_belongs_to(&self, pending: &PendingFrame, output: &Output) -> bool {
        match &pending.session.source {
            Source::Output(o) => o == output,
            Source::Toplevel(handle) => match self.managed_for_handle(handle) {
                Some(managed) => {
                    let outputs = self.space.outputs_for_element(&managed.window);
                    outputs.contains(output) || (outputs.is_empty() && self.outputs.first().is_some_and(|e| e.output == *output))
                }
                None => false,
            },
        }
    }

    /// A window went away: its capture sessions end.
    pub fn image_capture_window_closed(&mut self, handle: &ForeignToplevelHandle) {
        let id = handle.identifier();
        self.stop_sessions(|source| matches!(source, Source::Toplevel(h) if h.identifier() == id));
    }

    /// An output went away: its capture sessions end.
    pub fn image_capture_output_removed(&mut self, output: &Output) {
        self.stop_sessions(|source| matches!(source, Source::Output(o) if o == output));
    }

    fn stop_sessions(&mut self, matches: impl Fn(&Source) -> bool) {
        let state = &mut self.image_capture;
        state.sessions.retain(|(resource, shared)| {
            if !matches(&shared.source) {
                return true;
            }
            shared.inner.lock().unwrap().stopped = true;
            tracing::info!("capture session ended: {}", describe(&shared.source));
            resource.stopped();
            false
        });
        state.pending.retain(|p| {
            if !matches(&p.session.source) {
                return true;
            }
            p.frame.failed(FailureReason::Stopped);
            false
        });
    }

    /// Answer the capture frames waiting on `output`, using the elements just drawn for outputs.
    pub fn fulfill_image_captures(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        elements: &[OutputElement],
        had_damage: bool,
    ) {
        self.image_capture.sessions.retain(|(resource, _)| resource.is_alive());
        self.complete_readbacks(renderer, false);
        if self.image_capture.pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.image_capture.pending);
        let mut waiting = Vec::new();
        for frame in pending {
            if !frame.frame.is_alive() {
                continue;
            }
            if !self.frame_belongs_to(&frame, output) {
                waiting.push(frame);
                continue;
            }
            let Some(size) = self.source_size(&frame.session.source) else {
                frame.frame.failed(FailureReason::Stopped);
                continue;
            };
            let mut inner = frame.session.inner.lock().unwrap();
            if inner.size != size {
                // The source resized: tell the client the new constraints and let it retry.
                inner.size = size;
                drop(inner);
                if let Some((resource, _)) = self.image_capture.sessions.iter().find(|(_, s)| Arc::ptr_eq(s, &frame.session)) {
                    self.send_constraints(resource, size);
                }
                frame.frame.failed(FailureReason::BufferConstraints);
                continue;
            }
            let (changed, mark) = match &frame.session.source {
                Source::Output(_) => (had_damage || inner.captured.is_none() || frame.requested.elapsed() >= KEEPALIVE, 0),
                Source::Toplevel(handle) => {
                    let commits = self.managed_for_handle(handle).map_or(0, |m| m.commits);
                    (inner.captured != Some(commits) || frame.requested.elapsed() >= KEEPALIVE, commits)
                }
            };
            if !changed {
                drop(inner);
                waiting.push(frame);
                continue;
            }
            if inner.captured.is_none() {
                tracing::info!("capture session: first frame of {} ({}x{})", describe(&frame.session.source), size.w, size.h);
            }
            inner.captured = Some(mark);
            drop(inner);
            let timestamp: std::time::Duration = Clock::<Monotonic>::new().now().into();
            match self.capture_frame(renderer, elements, &frame, size) {
                Ok(None) => send_ready(&frame.frame, size, timestamp),
                Ok(Some(readback)) => {
                    self.image_capture.readbacks.push(InFlight { frame, readback, captured_at: std::time::Instant::now(), timestamp });
                }
                Err(error) => {
                    tracing::warn!("image capture failed: {error}");
                    frame.frame.failed(FailureReason::Unknown);
                }
            }
        }
        if !self.image_capture.readbacks.is_empty() {
            if self.udev.is_some() {
                self.arm_readback_timer();
            } else {
                // The nested backend has no renderer outside its redraw: finish right away.
                self.complete_readbacks(renderer, true);
            }
        }
        waiting.append(&mut self.image_capture.pending);
        // Frames still waiting for a change on this output get a redraw when their keepalive is due.
        let due = waiting.iter().filter(|f| self.frame_belongs_to(f, output)).map(|f| KEEPALIVE.saturating_sub(f.requested.elapsed())).min();
        self.image_capture.pending = waiting;
        if let Some(due) = due {
            self.schedule_capture_redraw(output, due);
        }
    }

    fn schedule_capture_redraw(&mut self, output: &Output, after: std::time::Duration) {
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        if !self.image_capture.timers.insert(output.name()) {
            return;
        }
        let output = output.clone();
        let _ = self.loop_handle.insert_source(Timer::from_duration(after.max(std::time::Duration::from_millis(20))), move |_, _, state| {
            state.image_capture.timers.remove(&output.name());
            state.queue_redraw_output(&output);
            TimeoutAction::Drop
        });
    }

    fn capture_frame(
        &mut self,
        renderer: &mut GlesRenderer,
        elements: &[OutputElement],
        frame: &PendingFrame,
        size: Size<i32, Physical>,
    ) -> Result<Option<Readback>, String> {
        let region = Rectangle::from_size(size);
        // Shared memory goes through a read-back that completes later; GPU buffers are drawn into.
        let mut texture = if get_dmabuf(&frame.buffer).is_err() { Some(self.capture_texture(renderer, &frame.session, size)?) } else { None };
        match &frame.session.source {
            Source::Output(output) => {
                let mode_size = output.current_mode().ok_or("the output has no mode")?.size;
                let transform = if self.udev.is_some() { output.current_transform() } else { Transform::Normal };
                let scale = output.current_scale().fractional_scale();
                let mut tracker = OutputDamageTracker::new(mode_size, scale, transform);
                let visible: Vec<&OutputElement> = elements
                    .iter()
                    .filter(|e| !matches!(e, OutputElement::Overlay(_)))
                    .filter(|e| frame.session.paint_cursors || !matches!(e, OutputElement::Cursor(_)))
                    .collect();
                match texture.as_mut() {
                    Some(texture) => start_readback(renderer, &mut tracker, &visible, self.clear_color(), texture, region).map(Some),
                    None => render_to_buffer(renderer, &mut tracker, &visible, self.clear_color(), &frame.buffer, size, region).map(|()| None),
                }
            }
            Source::Toplevel(handle) => {
                let managed = self.managed_for_handle(handle).ok_or("the window is gone")?;
                let scale = managed.surface().map_or(1.0, |s| self.scale_for_surface(&s));
                let geometry = managed.window.geometry();
                // Put the window's geometry (without client-side shadows) at the origin.
                let origin = (-geometry.loc.x, -geometry.loc.y);
                let location = smithay::utils::Point::<i32, smithay::utils::Logical>::from(origin).to_physical_precise_round(scale);
                let elements = AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                    &managed.window,
                    renderer,
                    location,
                    Scale::from(scale),
                    1.0,
                );
                let mut tracker = OutputDamageTracker::new(size, scale, Transform::Normal);
                match texture.as_mut() {
                    Some(texture) => start_readback(renderer, &mut tracker, &elements, [0.0, 0.0, 0.0, 0.0], texture, region).map(Some),
                    None => render_elements_to_buffer(renderer, &mut tracker, &elements, &frame.buffer, size, region).map(|()| None),
                }
            }
        }
    }

    /// The session's offscreen texture for frames of `size`, created when missing or resized.
    fn capture_texture(&mut self, renderer: &mut GlesRenderer, session: &Arc<SessionShared>, size: Size<i32, Physical>) -> Result<GlesTexture, String> {
        let textures = &mut self.image_capture.textures;
        textures.retain(|(owner, _, _)| owner.strong_count() > 0);
        if let Some((_, _, texture)) = textures.iter().find(|(owner, s, _)| owner.as_ptr() == Arc::as_ptr(session) && *s == size) {
            return Ok(texture.clone());
        }
        let texture = offscreen_texture(renderer, size)?;
        textures.retain(|(owner, _, _)| owner.as_ptr() != Arc::as_ptr(session));
        textures.push((Arc::downgrade(session), size, texture.clone()));
        Ok(texture)
    }

    /// Hand finished read-backs to their clients; with `block`, wait for the unfinished ones too.
    fn complete_readbacks(&mut self, renderer: &mut GlesRenderer, block: bool) {
        if self.image_capture.readbacks.is_empty() {
            return;
        }
        let mut still = Vec::new();
        for flight in std::mem::take(&mut self.image_capture.readbacks) {
            if !flight.frame.frame.is_alive() {
                flight.readback.discard(renderer);
                continue;
            }
            if !block && flight.captured_at.elapsed() < READBACK_LIMIT && !flight.readback.is_ready(renderer) {
                still.push(flight);
                continue;
            }
            let size = flight.readback.size();
            match flight.readback.finish(renderer, &flight.frame.buffer) {
                Ok(()) => send_ready(&flight.frame.frame, size, flight.timestamp),
                Err(error) => {
                    tracing::warn!("image capture failed: {error}");
                    flight.frame.frame.failed(FailureReason::Unknown);
                }
            }
        }
        self.image_capture.readbacks = still;
    }

    /// Poll the GPU for finished read-backs every millisecond while some are in flight.
    fn arm_readback_timer(&mut self) {
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        if self.image_capture.readback_timer {
            return;
        }
        self.image_capture.readback_timer = true;
        const POLL: std::time::Duration = std::time::Duration::from_millis(1);
        let _ = self.loop_handle.insert_source(Timer::from_duration(POLL), |_, _, state| {
            let polled = state.with_capture_renderer(|state, renderer| state.complete_readbacks(renderer, false));
            if !polled {
                // The GPU went away: these frames will not be filled.
                for flight in std::mem::take(&mut state.image_capture.readbacks) {
                    flight.frame.frame.failed(FailureReason::Unknown);
                }
            }
            if state.image_capture.readbacks.is_empty() {
                state.image_capture.readback_timer = false;
                TimeoutAction::Drop
            } else {
                TimeoutAction::ToDuration(POLL)
            }
        });
    }
}

fn send_ready(frame: &ExtImageCopyCaptureFrameV1, size: Size<i32, Physical>, timestamp: std::time::Duration) {
    frame.transform(WlTransform::Normal);
    frame.damage(0, 0, size.w, size.h);
    frame.presentation_time((timestamp.as_secs() >> 32) as u32, timestamp.as_secs() as u32, timestamp.subsec_nanos());
    frame.ready();
}

fn render_elements_to_buffer<E: RenderElement<GlesRenderer>>(
    renderer: &mut GlesRenderer,
    tracker: &mut OutputDamageTracker,
    elements: &[E],
    buffer: &WlBuffer,
    size: Size<i32, Physical>,
    region: Rectangle<i32, Physical>,
) -> Result<(), String> {
    render_to_buffer(renderer, tracker, elements, [0.0, 0.0, 0.0, 0.0], buffer, size, region)
}

// --- Sources ------------------------------------------------------------------------------

impl GlobalDispatch<ExtOutputImageCaptureSourceManagerV1, (), State> for State {
    fn bind(_: &mut State, _: &DisplayHandle, _: &Client, resource: New<ExtOutputImageCaptureSourceManagerV1>, _: &(), init: &mut DataInit<'_, State>) {
        init.init(resource, ());
    }
}

impl Dispatch<ExtOutputImageCaptureSourceManagerV1, (), State> for State {
    fn request(
        _: &mut State,
        _: &Client,
        _: &ExtOutputImageCaptureSourceManagerV1,
        request: ext_output_image_capture_source_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, State>,
    ) {
        if let ext_output_image_capture_source_manager_v1::Request::CreateSource { source, output } = request {
            // An output that does not exist (any more) yields a source that fails its sessions.
            let output = Output::from_resource(&output);
            init.init(source, output.map(Source::Output));
        }
    }
}

impl GlobalDispatch<ExtForeignToplevelImageCaptureSourceManagerV1, (), State> for State {
    fn bind(_: &mut State, _: &DisplayHandle, _: &Client, resource: New<ExtForeignToplevelImageCaptureSourceManagerV1>, _: &(), init: &mut DataInit<'_, State>) {
        init.init(resource, ());
    }
}

impl Dispatch<ExtForeignToplevelImageCaptureSourceManagerV1, (), State> for State {
    fn request(
        _: &mut State,
        _: &Client,
        _: &ExtForeignToplevelImageCaptureSourceManagerV1,
        request: ext_foreign_toplevel_image_capture_source_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, State>,
    ) {
        if let ext_foreign_toplevel_image_capture_source_manager_v1::Request::CreateSource { source, toplevel_handle } = request {
            let handle = ForeignToplevelHandle::from_resource(&toplevel_handle);
            init.init(source, handle.map(Source::Toplevel));
        }
    }
}

impl Dispatch<ExtImageCaptureSourceV1, Option<Source>, State> for State {
    fn request(
        _: &mut State,
        _: &Client,
        _: &ExtImageCaptureSourceV1,
        _: smithay::reexports::wayland_protocols::ext::image_capture_source::v1::server::ext_image_capture_source_v1::Request,
        _: &Option<Source>,
        _: &DisplayHandle,
        _: &mut DataInit<'_, State>,
    ) {
    }
}

// --- Sessions and frames ------------------------------------------------------------------

impl GlobalDispatch<ExtImageCopyCaptureManagerV1, (), State> for State {
    fn bind(_: &mut State, _: &DisplayHandle, _: &Client, resource: New<ExtImageCopyCaptureManagerV1>, _: &(), init: &mut DataInit<'_, State>) {
        init.init(resource, ());
    }
}

impl Dispatch<ExtImageCopyCaptureManagerV1, (), State> for State {
    fn request(
        state: &mut State,
        _: &Client,
        manager: &ExtImageCopyCaptureManagerV1,
        request: ext_image_copy_capture_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, State>,
    ) {
        match request {
            ext_image_copy_capture_manager_v1::Request::CreateSession { session, source, options } => {
                let paint_cursors = options.into_result().is_ok_and(|o| o.contains(Options::PaintCursors));
                let Some(source) = source.data::<Option<Source>>().cloned().flatten() else {
                    // The source's output or window is already gone.
                    let shared = Arc::new(SessionShared { source: dead_source(), paint_cursors, inner: stopped_inner() });
                    init.init(session, shared).stopped();
                    return;
                };
                let Some(size) = state.source_size(&source) else {
                    let shared = Arc::new(SessionShared { source, paint_cursors, inner: stopped_inner() });
                    init.init(session, shared).stopped();
                    return;
                };
                let shared = Arc::new(SessionShared {
                    source,
                    paint_cursors,
                    inner: Mutex::new(SessionInner { size, has_frame: false, stopped: false, captured: None }),
                });
                tracing::info!(
                    "capture session: {} {}x{}{}",
                    describe(&shared.source),
                    size.w,
                    size.h,
                    if shared.paint_cursors { ", with cursor" } else { "" }
                );
                let resource = init.init(session, shared.clone());
                state.send_constraints(&resource, size);
                state.image_capture.sessions.push((resource, shared));
            }
            ext_image_copy_capture_manager_v1::Request::CreatePointerCursorSession { session, .. } => {
                // Cursors are painted into the image (`paint_cursors`); there is no separate stream.
                init.init(session, ());
            }
            _ => {
                let _ = manager;
            }
        }
    }
}

/// The source in words, for the log.
fn describe(source: &Source) -> String {
    match source {
        Source::Output(output) => format!("output {}", output.name()),
        Source::Toplevel(handle) => format!("window {:?} ({})", handle.title(), handle.app_id()),
    }
}

fn stopped_inner() -> Mutex<SessionInner> {
    Mutex::new(SessionInner { size: Size::default(), has_frame: false, stopped: true, captured: None })
}

/// Placeholder source of sessions that stopped before they began.
fn dead_source() -> Source {
    Source::Output(Output::new(
        String::new(),
        smithay::output::PhysicalProperties {
            size: (0, 0).into(),
            subpixel: smithay::output::Subpixel::Unknown,
            make: String::new(),
            model: String::new(),
        },
    ))
}

impl Dispatch<ExtImageCopyCaptureCursorSessionV1, (), State> for State {
    fn request(
        _: &mut State,
        _: &Client,
        _: &ExtImageCopyCaptureCursorSessionV1,
        request: ext_image_copy_capture_cursor_session_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, State>,
    ) {
        if let ext_image_copy_capture_cursor_session_v1::Request::GetCaptureSession { session } = request {
            let shared = Arc::new(SessionShared { source: dead_source(), paint_cursors: false, inner: stopped_inner() });
            init.init(session, shared).stopped();
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, Arc<SessionShared>, State> for State {
    fn request(
        _: &mut State,
        _: &Client,
        session: &ExtImageCopyCaptureSessionV1,
        request: ext_image_copy_capture_session_v1::Request,
        data: &Arc<SessionShared>,
        _: &DisplayHandle,
        init: &mut DataInit<'_, State>,
    ) {
        if let ext_image_copy_capture_session_v1::Request::CreateFrame { frame } = request {
            let mut inner = data.inner.lock().unwrap();
            if inner.has_frame {
                session.post_error(ext_image_copy_capture_session_v1::Error::DuplicateFrame, "a frame is still alive");
                return;
            }
            inner.has_frame = true;
            let stopped = inner.stopped;
            drop(inner);
            let resource = init.init(frame, FrameData { session: data.clone(), inner: Mutex::new(FrameInner::default()) });
            if stopped {
                resource.failed(FailureReason::Stopped);
            }
        }
    }

    fn destroyed(state: &mut State, _: ClientId, session: &ExtImageCopyCaptureSessionV1, data: &Arc<SessionShared>) {
        if state.image_capture.sessions.iter().any(|(s, _)| s == session) {
            tracing::info!("capture session closed: {}", describe(&data.source));
        }
        state.image_capture.sessions.retain(|(s, _)| s != session);
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, FrameData, State> for State {
    fn request(
        state: &mut State,
        _: &Client,
        frame: &ExtImageCopyCaptureFrameV1,
        request: ext_image_copy_capture_frame_v1::Request,
        data: &FrameData,
        _: &DisplayHandle,
        _: &mut DataInit<'_, State>,
    ) {
        use ext_image_copy_capture_frame_v1::{Error, Request};
        let mut inner = data.inner.lock().unwrap();
        match request {
            Request::AttachBuffer { buffer } => {
                if inner.captured {
                    frame.post_error(Error::AlreadyCaptured, "the frame was already captured");
                    return;
                }
                inner.buffer = Some(buffer);
            }
            Request::DamageBuffer { x, y, width, height } => {
                if inner.captured {
                    frame.post_error(Error::AlreadyCaptured, "the frame was already captured");
                } else if x < 0 || y < 0 || width <= 0 || height <= 0 {
                    frame.post_error(Error::InvalidBufferDamage, "damage must be a positive rectangle");
                }
            }
            Request::Capture => {
                if inner.captured {
                    frame.post_error(Error::AlreadyCaptured, "the frame was already captured");
                    return;
                }
                let Some(buffer) = inner.buffer.clone() else {
                    frame.post_error(Error::NoBuffer, "capture without a buffer");
                    return;
                };
                inner.captured = true;
                drop(inner);
                let (size, stopped) = {
                    let session = data.session.inner.lock().unwrap();
                    (session.size, session.stopped)
                };
                if stopped {
                    frame.failed(FailureReason::Stopped);
                    return;
                }
                if !buffer_fits(&buffer, size) {
                    frame.failed(FailureReason::BufferConstraints);
                    return;
                }
                state.image_capture.pending.push(PendingFrame { frame: frame.clone(), session: data.session.clone(), buffer, requested: std::time::Instant::now() });
                // Only the output that answers the frame: a redraw of the others (a game's output
                // when a different monitor is shared) would cost them a frame for nothing.
                let output = match &data.session.source {
                    Source::Output(output) => Some(output.clone()),
                    Source::Toplevel(handle) => state.managed_for_handle(handle).and_then(|m| state.capture_output(&m.window)),
                };
                match output {
                    Some(output) => state.queue_redraw_output(&output),
                    None => state.queue_redraw_all(),
                }
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut State, _: ClientId, frame: &ExtImageCopyCaptureFrameV1, data: &FrameData) {
        data.session.inner.lock().unwrap().has_frame = false;
        state.image_capture.pending.retain(|p| &p.frame != frame);
    }
}

fn buffer_fits(buffer: &WlBuffer, size: Size<i32, Physical>) -> bool {
    let acceptable = |code: u32| code == Fourcc::Argb8888 as u32 || code == Fourcc::Xrgb8888 as u32;
    if let Ok(dmabuf) = get_dmabuf(buffer) {
        return acceptable(dmabuf.format().code as u32) && dmabuf.size().w == size.w && dmabuf.size().h == size.h;
    }
    with_buffer_contents_mut(buffer, |_, _, d| {
        d.width == size.w
            && d.height == size.h
            && d.stride >= size.w * 4
            && matches!(d.format, wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888)
    })
    .unwrap_or(false)
}

impl State {
    /// Register a window in `ext-foreign-toplevel-list` so capture clients can name it.
    pub fn announce_foreign_toplevel(&mut self, id: crate::desktop::WindowId) {
        let Some(m) = self.desktop.get(id) else { return };
        let (title, app_id) = match m.window.x11_surface() {
            Some(x11) => (x11.title(), x11.class()),
            None => m.surface().map(|s| toplevel_names(&s)).unwrap_or_default(),
        };
        let handle = self.foreign_toplevels.new_toplevel::<State>(title, app_id);
        if let Some(m) = self.desktop.get_mut(id) {
            m.foreign = Some(handle);
        }
    }

    /// Title or app id of a toplevel changed.
    pub fn update_foreign_toplevel(&mut self, surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) {
        let Some(handle) = self.desktop.by_surface(surface).and_then(|m| m.foreign.clone()) else { return };
        let (title, app_id) = toplevel_names(surface);
        if handle.title() != title {
            handle.send_title(&title);
        }
        if handle.app_id() != app_id {
            handle.send_app_id(&app_id);
        }
        handle.send_done();
    }

    /// The surface committed: capture sessions of its window have something new to show.
    pub fn count_commit(&mut self, surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) {
        let mut root = surface.clone();
        while let Some(parent) = smithay::wayland::compositor::get_parent(&root) {
            root = parent;
        }
        if let Some(id) = self.desktop.by_surface(&root).map(|m| m.id)
            && let Some(m) = self.desktop.get_mut(id)
        {
            m.commits += 1;
        }
    }
}

fn toplevel_names(surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) -> (String, String) {
    use smithay::wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData};
    with_states(surface, |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .map(|data| {
                let data = data.lock().unwrap();
                (data.title.clone().unwrap_or_default(), data.app_id.clone().unwrap_or_default())
            })
            .unwrap_or_default()
    })
}
