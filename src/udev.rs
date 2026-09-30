//! The hardware backend: DRM/KMS output via GBM and EGL, input via libinput, session via libseat.
//!
//! One GPU (the primary one) renders every output. Outputs redraw on demand and are paced by
//! vblank; when nothing changed, an estimated vblank timer keeps frame callbacks flowing.
use std::{
    collections::HashMap,
    error::Error,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use mywm_config::{OutputConfig, OutputMode, OutputTransform};
use smithay::{
    backend::{
        allocator::{
            Fourcc,
            dmabuf::Dmabuf,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType, DrmEventMetadata as EventMetadata, DrmEventTime as DrmTime, VrrSupport,
            compositor::{DrmCompositor, FrameFlags},
            exporter::gbm::GbmFramebufferExporter,
        },
        egl::{EGLContext, EGLDisplay, context::ContextPriority},
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            ImportDma,
            element::{default_primary_scanout_output_compare, utils::select_dmabuf_feedback},
            gles::GlesRenderer,
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{UdevBackend, UdevEvent, primary_gpu},
    },
    desktop::{
        utils::{
            OutputPresentationFeedback, surface_presentation_feedback_flags_from_states,
            surface_primary_scanout_output, update_surface_primary_scanout_output,
        },
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        wayland_protocols::wp::{
            linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1::TrancheFlags,
            presentation_time::server::wp_presentation_feedback,
        },
        calloop::{
            EventLoop, LoopHandle, RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{ModeTypeFlags, connector, crtc},
        input::{DeviceCapability, Libinput},
        rustix::fs::OFlags,
    },
    utils::{Clock, ClockSource, DeviceFd, Monotonic, Time, Transform},
    wayland::{
        dmabuf::{DmabufFeedback, DmabufFeedbackBuilder, ImportNotifier},
        drm_syncobj::{DrmSyncobjState, supports_syncobj_eventfd},
        presentation::{PresentationState, Refresh},
    },
};
use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner};

use crate::{
    State,
    pacing::{RenderTimes, late_delay},
};

type GbmCompositor = DrmCompositor<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    Option<OutputPresentationFeedback>,
    DrmDeviceFd,
>;

/// Where an output is in its redraw cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Redraw {
    Idle,
    /// A redraw runs at the end of this loop iteration.
    Queued,
    /// A frame is on its way to the screen; `again` if something changed meanwhile.
    WaitingForVBlank { again: bool },
    /// The last frame was empty; a timer stands in for the vblank that will not come.
    WaitingForEstimatedVBlank { again: bool },
}

/// Whether VRR and tearing apply to an output right now, and the facts behind that.
#[derive(Default)]
struct PresentationPolicy {
    vrr: bool,
    tearing: bool,
    /// A recognized game is fullscreen and focused on the output.
    game: bool,
    tearing_configured: bool,
    tearing_requested: bool,
}

/// What clients are told about buffers: the default set, and one tuned for direct scanout.
struct SurfaceFeedback {
    render: DmabufFeedback,
    scanout: DmabufFeedback,
}

struct Surface {
    output: Output,
    compositor: GbmCompositor,
    connector: connector::Handle,
    redraw: Redraw,
    feedback: Option<SurfaceFeedback>,
    /// Adaptive sync is currently on.
    vrr: bool,
    /// Frames currently flip immediately (tearing).
    tearing: bool,
    /// The display is on; while off nothing is drawn.
    powered: bool,
    /// Last (game, tearing configured, tearing requested) that was logged.
    situation: (bool, bool, bool),
    /// Cleared when the driver rejected an immediate flip.
    tearing_works: bool,
    /// Adaptive sync was requested but cannot be enabled on this output.
    vrr_failed: bool,
    /// Monotonic time of the last vblank, the anchor for late scheduling.
    last_vblank: Option<Duration>,
    /// Recent CPU time spent rendering and queueing a frame.
    times: RenderTimes,
    stats: FrameStats,
    /// The last frame went to the screen without a compositing pass.
    scanout: bool,
}

/// Counters logged now and then (target `perf`, level debug).
struct FrameStats {
    since: Instant,
    frames: u32,
    missed: u32,
    worst: Duration,
}

impl FrameStats {
    fn new() -> Self {
        Self { since: Instant::now(), frames: 0, missed: 0, worst: Duration::ZERO }
    }
}

struct Gpu {
    dev_id: u64,
    drm: DrmDevice,
    gbm: GbmDevice<DrmDeviceFd>,
    renderer: GlesRenderer,
    render_node: DrmNode,
    scanner: DrmScanner,
    surfaces: HashMap<crtc::Handle, Surface>,
    notifier: RegistrationToken,
}

pub struct UdevData {
    session: LibSeatSession,
    libinput: Libinput,
    /// Path of the GPU that renders; devices are ignored until it shows up.
    primary: Option<PathBuf>,
    gpu: Option<Gpu>,
    handle: LoopHandle<'static, State>,
    /// `[render] late_scheduling`: the safety margin before the vblank, if enabled.
    pub late_margin: Option<Duration>,
}

pub fn init(event_loop: &mut EventLoop<'static, State>, state: &mut State) -> Result<(), Box<dyn Error>> {
    let (session, notifier) = LibSeatSession::new().map_err(|e| format!("cannot open a seat session: {e}"))?;
    let seat = session.seat();
    let handle = event_loop.handle();

    let mut libinput = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
    libinput.udev_assign_seat(&seat).map_err(|_| "libinput cannot use the seat")?;
    let primary = primary_gpu(&seat)?;
    tracing::info!("seat {seat}, primary GPU {primary:?}");

    state.session = Some(session.clone());
    state.udev = Some(UdevData { session, libinput: libinput.clone(), primary, gpu: None, handle: handle.clone(), late_margin: late_margin(&state.config.render) });

    handle
        .insert_source(notifier, |event, _, state| match event {
            SessionEvent::PauseSession => state.udev_pause(),
            SessionEvent::ActivateSession => state.udev_resume(),
        })
        .map_err(|e| e.error)?;

    handle
        .insert_source(LibinputInputBackend::new(libinput), |event, _, state| match event {
            InputEvent::DeviceAdded { mut device } => {
                if device.has_capability(DeviceCapability::Pointer) && device.config_tap_finger_count() > 0 {
                    let _ = device.config_tap_set_enabled(true);
                }
            }
            event => state.process_input_event(event, None),
        })
        .map_err(|e| e.error)?;

    let backend = UdevBackend::new(&seat)?;
    for (dev_id, path) in backend.device_list() {
        state.udev_device_added(dev_id, path);
    }
    handle
        .insert_source(backend, |event, _, state| match event {
            UdevEvent::Added { device_id, path } => state.udev_device_added(device_id, &path),
            UdevEvent::Changed { device_id } => state.udev_device_changed(device_id),
            UdevEvent::Removed { device_id } => state.udev_device_removed(device_id),
        })
        .map_err(|e| e.error)?;
    if state.udev.as_ref().is_some_and(|u| u.gpu.is_none()) {
        return Err("no usable GPU found".into());
    }
    Ok(())
}

pub fn late_margin(config: &mywm_config::RenderConfig) -> Option<Duration> {
    config.late_scheduling.then(|| Duration::from_secs_f64(config.margin_ms / 1000.0))
}

/// Every presented element went to a plane as is: the compositor drew nothing itself.
fn direct_scanout(states: &smithay::backend::renderer::element::RenderElementStates) -> bool {
    use smithay::backend::renderer::element::RenderElementPresentationState::{Rendering, ZeroCopy};
    let mut zero_copy = false;
    for state in states.states.values() {
        match state.presentation_state {
            Rendering { .. } => return false,
            ZeroCopy => zero_copy = true,
            _ => {}
        }
    }
    zero_copy
}

fn now() -> Duration {
    Clock::<Monotonic>::new().now().into()
}

/// The mode to use for a connector: the requested one if the display has it, else its preferred one.
fn pick_mode(modes: &[smithay::reexports::drm::control::Mode], wanted: Option<OutputMode>, name: &str) -> Option<smithay::reexports::drm::control::Mode> {
    if let Some(wanted) = wanted {
        let candidates = modes.iter().filter(|m| {
            let (w, h) = m.size();
            u32::from(w) == wanted.width && u32::from(h) == wanted.height
        });
        let best = match wanted.refresh_mhz {
            Some(hz) => candidates.min_by_key(|m| (Mode::from(**m).refresh - hz as i32).abs()),
            // Without a rate, the highest one.
            None => candidates.max_by_key(|m| Mode::from(**m).refresh),
        };
        match best {
            Some(mode) => return Some(*mode),
            None => tracing::warn!("{name}: mode {}x{} not offered by the display, using the preferred one", wanted.width, wanted.height),
        }
    }
    modes
        .iter()
        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .or_else(|| modes.first())
        .copied()
}

fn transform(config: OutputTransform) -> Transform {
    match config {
        OutputTransform::Normal => Transform::Normal,
        OutputTransform::Rotate90 => Transform::_90,
        OutputTransform::Rotate180 => Transform::_180,
        OutputTransform::Rotate270 => Transform::_270,
        OutputTransform::Flipped => Transform::Flipped,
        OutputTransform::Flipped90 => Transform::Flipped90,
        OutputTransform::Flipped180 => Transform::Flipped180,
        OutputTransform::Flipped270 => Transform::Flipped270,
    }
}

fn connector_name(info: &connector::Info) -> String {
    format!("{}-{}", info.interface().as_str(), info.interface_id())
}

impl UdevData {
    fn queue_redraw(&mut self, crtc: crtc::Handle) {
        let Some(gpu) = &mut self.gpu else { return };
        let Some(surface) = gpu.surfaces.get_mut(&crtc).filter(|s| s.powered) else { return };
        surface.redraw = match surface.redraw {
            Redraw::Idle => {
                let dev_id = gpu.dev_id;
                match self.late_margin.and_then(|margin| surface.late_delay(margin)) {
                    // Wait until the frame just fits before the vblank; commits meanwhile
                    // land in the same frame (the state stays Queued).
                    Some(delay) => {
                        let _ = self.handle.insert_source(Timer::from_duration(delay), move |_, _, state| {
                            state.udev_redraw(dev_id, crtc);
                            TimeoutAction::Drop
                        });
                    }
                    None => {
                        self.handle.insert_idle(move |state| state.udev_redraw(dev_id, crtc));
                    }
                }
                Redraw::Queued
            }
            Redraw::WaitingForVBlank { .. } => Redraw::WaitingForVBlank { again: true },
            Redraw::WaitingForEstimatedVBlank { .. } => Redraw::WaitingForEstimatedVBlank { again: true },
            Redraw::Queued => Redraw::Queued,
        };
    }

    /// Redraw the output only (a change that cannot show on the others).
    pub fn queue_redraw_output(&mut self, output: &Output) {
        let crtc = self.gpu.iter().flat_map(|g| &g.surfaces).find(|(_, s)| s.output == *output).map(|(crtc, _)| *crtc);
        if let Some(crtc) = crtc {
            self.queue_redraw(crtc);
        }
    }

    pub fn queue_redraw_all(&mut self) {
        let crtcs: Vec<_> = self.gpu.iter().flat_map(|g| g.surfaces.keys().copied()).collect();
        for crtc in crtcs {
            self.queue_redraw(crtc);
        }
    }

    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        self.gpu.as_mut().is_some_and(|gpu| gpu.renderer.import_dmabuf(dmabuf, None).is_ok())
    }
}

impl Surface {
    fn interval(&self) -> Duration {
        let refresh = self.output.current_mode().map_or(60_000, |m| m.refresh.max(1000)) as u64;
        Duration::from_nanos(1_000_000_000_000 / refresh)
    }

    /// How long to hold back the next redraw for late scheduling; `None` to render right away
    /// (no vblank seen yet, or VRR/tearing where frames are not tied to a fixed refresh).
    fn late_delay(&self, margin: Duration) -> Option<Duration> {
        if self.vrr || self.tearing {
            return None;
        }
        let last = self.last_vblank?;
        Some(late_delay(now(), last, self.interval(), self.times.worst(), margin))
    }
}

impl State {
    pub fn queue_redraw_output(&mut self, output: &Output) {
        if let Some(udev) = &mut self.udev {
            udev.queue_redraw_output(output);
        }
    }

    pub fn queue_redraw_all(&mut self) {
        if let Some(udev) = &mut self.udev {
            udev.queue_redraw_all();
        }
    }

    pub fn udev_dmabuf_imported(&mut self, dmabuf: &Dmabuf, notifier: ImportNotifier) {
        if self.udev.as_mut().is_some_and(|u| u.import_dmabuf(dmabuf)) {
            let _ = notifier.successful::<State>();
        } else {
            notifier.failed();
        }
    }

    fn udev_device_added(&mut self, dev_id: u64, path: &Path) {
        let Some(udev) = &mut self.udev else { return };
        if udev.gpu.is_some() || udev.primary.as_deref().is_some_and(|p| p != path) {
            tracing::info!("ignoring {}: only the primary GPU is used", path.display());
            return;
        }
        match self.add_gpu(dev_id, path) {
            Ok(()) => self.udev_scan(dev_id),
            Err(error) => tracing::error!("cannot use {}: {error}", path.display()),
        }
    }

    fn add_gpu(&mut self, dev_id: u64, path: &Path) -> Result<(), Box<dyn Error>> {
        let handle;
        let fd = {
            let udev = self.udev.as_mut().unwrap();
            handle = udev.handle.clone();
            let fd = udev.session.open(path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)?;
            DrmDeviceFd::new(DeviceFd::from(fd))
        };
        let (drm, notifier) = DrmDevice::new(fd.clone(), true)?;
        let gbm = GbmDevice::new(fd.clone())?;
        // SAFETY: the GBM device outlives the display and everything created from it (all live in `Gpu`).
        let display = unsafe { EGLDisplay::new(gbm.clone())? };
        let context = EGLContext::new_with_priority(&display, ContextPriority::High)?;
        // SAFETY: the context was just created for this display.
        let renderer = unsafe { GlesRenderer::new(context)? };

        let node = DrmNode::from_path(path)?;
        let render_node = node.node_with_type(NodeType::Render).and_then(Result::ok).unwrap_or(node);

        // Clients may hand us GPU buffers: advertise what the renderer can import.
        let feedback = DmabufFeedbackBuilder::new(render_node.dev_id(), renderer.dmabuf_formats()).build()?;
        self.dmabuf_global = Some(
            self.dmabuf_state
                .create_global_with_default_feedback::<State>(&self.display_handle, &feedback),
        );

        // Explicit sync (needed by NVIDIA's Vulkan/Xwayland paths) and presentation timing.
        if supports_syncobj_eventfd(&fd) {
            self.syncobj_state = Some(DrmSyncobjState::new::<State>(&self.display_handle, fd));
            tracing::info!("explicit sync (linux-drm-syncobj) available");
        } else {
            tracing::info!("explicit sync unavailable: the kernel lacks syncobj eventfd support");
        }
        self.presentation_state = Some(PresentationState::new::<State>(&self.display_handle, Monotonic::ID as u32));

        let token = handle
            .insert_source(notifier, move |event, metadata, state| state.on_drm_event(dev_id, event, metadata))
            .map_err(|e| e.error)?;
        tracing::info!("GPU {} ready (render node {:?})", path.display(), render_node);
        self.udev.as_mut().unwrap().gpu = Some(Gpu {
            dev_id,
            drm,
            gbm,
            renderer,
            render_node,
            scanner: DrmScanner::new(),
            surfaces: HashMap::new(),
            notifier: token,
        });
        Ok(())
    }

    fn udev_device_changed(&mut self, dev_id: u64) {
        self.udev_scan(dev_id);
    }

    fn udev_device_removed(&mut self, dev_id: u64) {
        let Some(udev) = &mut self.udev else { return };
        let Some(gpu) = udev.gpu.take_if(|g| g.dev_id == dev_id) else { return };
        tracing::warn!("GPU removed");
        udev.handle.remove(gpu.notifier);
        let outputs: Vec<_> = gpu.surfaces.values().map(|s| s.output.clone()).collect();
        drop(gpu);
        for output in outputs {
            self.remove_output(&output);
        }
    }

    /// Find out which connectors came or went and set outputs up accordingly.
    fn udev_scan(&mut self, dev_id: u64) {
        let events: Vec<_> = {
            let Some(gpu) = self.udev.as_mut().and_then(|u| u.gpu.as_mut()).filter(|g| g.dev_id == dev_id) else { return };
            match gpu.scanner.scan_connectors(&gpu.drm) {
                Ok(result) => result.into_iter().collect(),
                Err(error) => {
                    tracing::warn!("connector scan failed: {error}");
                    return;
                }
            }
        };
        for event in events {
            match event {
                DrmScanEvent::Connected { connector, crtc: Some(crtc) } => self.connector_connected(connector, crtc),
                DrmScanEvent::Connected { connector, crtc: None } => {
                    tracing::warn!("{}: no free CRTC", connector_name(&connector));
                }
                DrmScanEvent::Disconnected { connector, crtc: Some(crtc) } => {
                    tracing::info!("{} unplugged", connector_name(&connector));
                    self.connector_disconnected(crtc);
                }
                DrmScanEvent::Disconnected { .. } => {}
            }
        }
    }

    fn connector_connected(&mut self, info: connector::Info, crtc: crtc::Handle) {
        let name = connector_name(&info);
        let config: Option<&OutputConfig> = self.config.outputs.iter().find(|o| o.name == name);
        if config.is_some_and(|c| !c.enable) {
            tracing::info!("{name}: disabled by configuration");
            return;
        }
        let wanted = config.and_then(|c| c.parsed_mode().ok().flatten());
        let Some(drm_mode) = pick_mode(info.modes(), wanted, &name) else {
            tracing::warn!("{name}: the display offers no modes");
            return;
        };
        let (scale, output_transform, position) = config
            .map(|c| (c.scale, transform(c.transform), c.position.map(|[x, y]| (x, y).into())))
            .unwrap_or((1.0, Transform::Normal, None));
        let wl_mode = Mode::from(drm_mode);
        tracing::info!("{name}: {}x{}@{:.2} scale {scale}", wl_mode.size.w, wl_mode.size.h, f64::from(wl_mode.refresh) / 1000.0);

        let (mm_w, mm_h) = info.size().unwrap_or((0, 0));
        let output = Output::new(
            name.clone(),
            PhysicalProperties {
                size: (mm_w as i32, mm_h as i32).into(),
                subpixel: Subpixel::Unknown,
                make: "Unknown".into(),
                model: name.clone(),
            },
        );
        output.create_global::<State>(&self.display_handle);
        output.change_current_state(Some(wl_mode), Some(output_transform), Some(Scale::Fractional(scale)), position);
        output.set_preferred(wl_mode);

        let Some(gpu) = self.udev.as_mut().and_then(|u| u.gpu.as_mut()) else { return };
        let surface = match gpu.drm.create_surface(crtc, drm_mode, &[info.handle()]) {
            Ok(surface) => surface,
            Err(error) => {
                tracing::error!("{name}: cannot create a surface: {error}");
                return;
            }
        };
        let allocator = GbmAllocator::new(gbm_clone(&gpu.gbm), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
        let exporter = GbmFramebufferExporter::new(gbm_clone(&gpu.gbm), Some(gpu.render_node));
        let formats = [Fourcc::Xrgb8888, Fourcc::Argb8888, Fourcc::Xbgr8888, Fourcc::Abgr8888];
        let render_formats = gpu.renderer.egl_context().dmabuf_render_formats().iter().copied().collect::<Vec<_>>();
        let compositor = match DrmCompositor::new(
            &output,
            surface,
            None,
            allocator,
            exporter,
            formats,
            render_formats,
            gpu.drm.cursor_size(),
            Some(gbm_clone(&gpu.gbm)),
        ) {
            Ok(compositor) => compositor,
            Err(error) => {
                tracing::error!("{name}: cannot create the compositor: {error}");
                return;
            }
        };
        let feedback = surface_feedback(&gpu.renderer, &compositor, gpu.render_node);
        gpu.surfaces.insert(
            crtc,
            Surface {
                output: output.clone(),
                compositor,
                connector: info.handle(),
                redraw: Redraw::Idle,
                feedback,
                vrr: false,
                tearing: false,
                powered: true,
                situation: (false, false, false),
                tearing_works: true,
                vrr_failed: false,
                last_vblank: None,
                times: RenderTimes::default(),
                stats: FrameStats::new(),
                scanout: false,
            },
        );
        self.add_output(output, position);
        if let Some(udev) = &mut self.udev {
            udev.queue_redraw(crtc);
        }
    }

    fn connector_disconnected(&mut self, crtc: crtc::Handle) {
        let Some(gpu) = self.udev.as_mut().and_then(|u| u.gpu.as_mut()) else { return };
        if let Some(surface) = gpu.surfaces.remove(&crtc) {
            self.remove_output(&surface.output);
        }
    }

    fn on_drm_event(&mut self, dev_id: u64, event: DrmEvent, metadata: &mut Option<EventMetadata>) {
        match event {
            DrmEvent::VBlank(crtc) => self.on_vblank(dev_id, crtc, metadata.take()),
            DrmEvent::Error(error) => tracing::error!("DRM error: {error}"),
        }
    }

    fn on_vblank(&mut self, dev_id: u64, crtc: crtc::Handle, metadata: Option<EventMetadata>) {
        let Some(udev) = &mut self.udev else { return };
        let Some(surface) = udev.gpu.as_mut().filter(|g| g.dev_id == dev_id).and_then(|g| g.surfaces.get_mut(&crtc)) else { return };
        let time: Time<Monotonic> = match metadata {
            Some(EventMetadata { time: DrmTime::Monotonic(time), .. }) => time.into(),
            _ => Clock::<Monotonic>::new().now(),
        };
        surface.last_vblank = Some(time.into());
        match surface.compositor.frame_submitted() {
            Ok(Some(Some(mut feedback))) => {
                // Tell clients when their frame reached the screen.
                let sequence = metadata.map_or(0, |m| u64::from(m.sequence));
                let refresh = surface.output.current_mode().map_or(Refresh::Unknown, |mode| {
                    let interval = Duration::from_nanos(1_000_000_000_000 / mode.refresh.max(1) as u64);
                    if surface.vrr { Refresh::variable(interval) } else { Refresh::fixed(interval) }
                });
                let flags = wp_presentation_feedback::Kind::Vsync
                    | wp_presentation_feedback::Kind::HwClock
                    | wp_presentation_feedback::Kind::HwCompletion;
                feedback.presented::<_, Monotonic>(time, refresh, sequence, flags);
            }
            Ok(_) => {}
            Err(error) => tracing::warn!("frame_submitted: {error}"),
        }
        let again = matches!(surface.redraw, Redraw::WaitingForVBlank { again: true });
        surface.redraw = Redraw::Idle;
        let output = surface.output.clone();
        let late = udev.late_margin.is_some();
        if again {
            udev.queue_redraw(crtc);
        }
        // With late scheduling clients start their next frame at the vblank, so it is ready
        // when the delayed redraw runs.
        if late {
            self.send_frames(&output);
        }
    }

    /// Render and queue one frame of the output on `crtc`.
    fn udev_redraw(&mut self, dev_id: u64, crtc: crtc::Handle) {
        // Take the backend out so rendering can borrow the rest of the state freely.
        let Some(mut udev) = self.udev.take() else { return };
        if let Some(gpu) = udev.gpu.as_mut().filter(|g| g.dev_id == dev_id) {
            self.redraw_surface(gpu, &udev.handle, udev.late_margin.is_some(), crtc);
        }
        self.udev = Some(udev);
    }

    fn redraw_surface(&mut self, gpu: &mut Gpu, handle: &LoopHandle<'static, State>, late: bool, crtc: crtc::Handle) {
        let Gpu { renderer, surfaces, dev_id, .. } = gpu;
        let Some(surface) = surfaces.get_mut(&crtc) else { return };
        if surface.redraw != Redraw::Queued {
            return;
        }
        let output = surface.output.clone();

        let elements = self.output_elements(renderer, &output);
        let background = self.clear_color();

        // Adaptive sync and tearing only while a game owns the output.
        let policy = self.presentation_policy(&output);
        let (vrr_wanted, tearing_wanted) = (policy.vrr, policy.tearing);
        // Say why VRR and tearing are (not) used whenever the situation changes: a game owning
        // the output, whether the config allows tearing here and whether the game asks for it.
        let situation = (policy.game, policy.tearing_configured, policy.tearing_requested);
        if surface.situation != situation {
            surface.situation = situation;
            if policy.game || surface.tearing {
                tracing::info!(
                    "{}: fullscreen game: {}; tearing allowed by config: {}; requested by the game: {}",
                    output.name(),
                    policy.game,
                    policy.tearing_configured,
                    policy.tearing_requested
                );
            }
        }
        if vrr_wanted != surface.vrr && !surface.vrr_failed {
            surface.vrr = set_vrr(&mut surface.compositor, surface.connector, vrr_wanted, &output.name());
            // Not supported (or refused): do not ask again every frame.
            surface.vrr_failed = vrr_wanted && !surface.vrr;
        }
        let tearing = tearing_wanted && surface.tearing_works;
        if tearing != surface.tearing {
            tracing::info!("{}: tearing {}", output.name(), if tearing { "on (immediate page flips)" } else { "off" });
        }
        surface.tearing = tearing;
        surface.compositor.set_async_flip(surface.tearing);

        let started = Instant::now();
        let mut had_damage = false;
        let queued = match surface.compositor.render_frame(renderer, &elements, background, FrameFlags::DEFAULT) {
            Ok(result) => {
                let states = result.states.clone();
                let is_empty = result.is_empty;
                had_damage = !is_empty;
                self.update_scanout_feedback(&output, &states, surface.feedback.as_ref());
                if !is_empty {
                    let scanout = direct_scanout(&states);
                    if scanout != surface.scanout {
                        surface.scanout = scanout;
                        let what = if scanout { "direct scanout (no compositing)" } else { "composited" };
                        if policy.game {
                            tracing::info!("{}: {what}", output.name());
                        } else {
                            tracing::debug!(target: "perf", "{}: {what}", output.name());
                        }
                    }
                }
                if is_empty {
                    false
                } else {
                    let mut feedback = OutputPresentationFeedback::new(&output);
                    for window in self.space.elements_for_output(&output) {
                        window.take_presentation_feedback(&mut feedback, surface_primary_scanout_output, |surface, _| {
                            surface_presentation_feedback_flags_from_states(surface, &states)
                        });
                    }
                    match surface.compositor.queue_frame(Some(feedback)) {
                        Ok(()) => true,
                        Err(error) if surface.tearing => {
                            // The driver refused an immediate flip; do not try again on this output.
                            tracing::warn!("{}: tearing flip rejected ({error}); falling back to vsync", output.name());
                            surface.tearing_works = false;
                            surface.tearing = false;
                            surface.compositor.set_async_flip(false);
                            false
                        }
                        Err(error) => {
                            tracing::warn!("{}: queue_frame: {error}", output.name());
                            false
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!("{}: render failed: {error}", output.name());
                false
            }
        };
        let took = started.elapsed();
        if queued {
            surface.times.record(took);
            surface.stats.frames += 1;
            surface.stats.worst = surface.stats.worst.max(took);
            if took > surface.interval() {
                surface.stats.missed += 1;
            }
            if surface.stats.since.elapsed() >= Duration::from_secs(5) {
                let stats = std::mem::replace(&mut surface.stats, FrameStats::new());
                tracing::debug!(
                    target: "perf",
                    "{}: {} frames in {:.1}s, cpu render avg {:.2} ms, worst {:.2} ms, {} slower than the refresh interval",
                    output.name(),
                    stats.frames,
                    stats.since.elapsed().as_secs_f64(),
                    surface.times.average().as_secs_f64() * 1000.0,
                    stats.worst.as_secs_f64() * 1000.0,
                    stats.missed
                );
            }
        }
        self.fulfill_screencopy(renderer, &output, &elements, had_damage);
        // With late scheduling frame callbacks go out at the vblank instead (see `on_vblank`).
        if !late {
            self.send_frames(&output);
        }
        self.note_locked_frame(&output);

        if queued {
            surface.redraw = Redraw::WaitingForVBlank { again: false };
        } else {
            // No vblank will arrive for an unchanged frame: pretend one does.
            let refresh = output.current_mode().map_or(60_000, |m| m.refresh.max(1000)) as u64;
            let interval = Duration::from_nanos(1_000_000_000_000 / refresh);
            let dev_id = *dev_id;
            surface.redraw = Redraw::WaitingForEstimatedVBlank { again: false };
            let _ = handle.insert_source(Timer::from_duration(interval), move |_, _, state| {
                state.on_estimated_vblank(dev_id, crtc);
                TimeoutAction::Drop
            });
        }
    }

    fn on_estimated_vblank(&mut self, dev_id: u64, crtc: crtc::Handle) {
        let Some(udev) = &mut self.udev else { return };
        let Some(surface) = udev.gpu.as_mut().filter(|g| g.dev_id == dev_id).and_then(|g| g.surfaces.get_mut(&crtc)) else { return };
        let again = matches!(surface.redraw, Redraw::WaitingForEstimatedVBlank { again: true });
        surface.redraw = Redraw::Idle;
        let output = surface.output.clone();
        let late = udev.late_margin.is_some();
        if again {
            udev.queue_redraw(crtc);
        }
        if late {
            self.send_frames(&output);
        }
    }

    /// Whether VRR and tearing should be active on `output` right now.
    ///
    /// VRR needs `[vrr] enabled` with this output named, tearing needs the output in
    /// `async_outputs`; both only while a recognized game is fullscreen and focused there,
    /// and tearing additionally needs the game to allow it (`wp_tearing_control_v1`).
    fn presentation_policy(&self, output: &Output) -> PresentationPolicy {
        let name = output.name();
        let vrr_allowed = self.config.vrr.enabled && self.config.vrr.output == name;
        let tearing_configured = self.config.async_outputs.contains(&name);
        let mut policy = PresentationPolicy { tearing_configured, ..Default::default() };
        if !vrr_allowed && !tearing_configured {
            return policy;
        }
        let Some(monitor) = self.outputs.iter().position(|e| &e.output == output) else { return policy };
        let Some(game) = self.fullscreen_game_on(monitor) else { return policy };
        policy.game = true;
        // X11 games (Xwayland) have their surface only through the window, Wayland ones through the toplevel.
        policy.tearing_requested = game.surface().is_some_and(|s| crate::protocols::surface_allows_tearing(&s));
        policy.vrr = vrr_allowed;
        policy.tearing = tearing_configured && policy.tearing_requested;
        policy
    }

    /// Point the surface tree's primary scanout output at `output` and hand clients the
    /// buffer feedback that matches whether they are being scanned out directly.
    fn update_scanout_feedback(
        &self,
        output: &Output,
        states: &smithay::backend::renderer::element::RenderElementStates,
        feedback: Option<&SurfaceFeedback>,
    ) {
        for window in self.space.elements_for_output(output) {
            window.with_surfaces(|surface, data| {
                update_surface_primary_scanout_output(surface, output, data, states, default_primary_scanout_output_compare);
            });
            if let Some(feedback) = feedback {
                window.send_dmabuf_feedback(output, surface_primary_scanout_output, |surface, _| {
                    select_dmabuf_feedback(surface, states, &feedback.render, &feedback.scanout)
                });
            }
        }
    }

    /// DPMS: switch the display of `output` on or off.
    pub fn udev_set_power(&mut self, output: &Output, on: bool) {
        let Some(udev) = &mut self.udev else { return };
        let Some(gpu) = &mut udev.gpu else { return };
        let Some((&crtc, surface)) = gpu.surfaces.iter_mut().find(|(_, s)| &s.output == output) else { return };
        surface.powered = on;
        if on {
            let _ = surface.compositor.reset_state();
            surface.redraw = Redraw::Idle;
            udev.queue_redraw(crtc);
        } else {
            // Dropping the pending frame and disabling the planes puts the display to sleep.
            if let Err(error) = surface.compositor.clear() {
                tracing::warn!("{}: cannot power the display off: {error}", output.name());
            }
            surface.redraw = Redraw::Idle;
        }
    }

    /// The seat was taken away (VT switch): stop using the devices.
    fn udev_pause(&mut self) {
        let Some(udev) = &mut self.udev else { return };
        tracing::info!("session paused");
        udev.libinput.suspend();
        if let Some(gpu) = &mut udev.gpu {
            gpu.drm.pause();
        }
    }

    fn udev_resume(&mut self) {
        let Some(udev) = &mut self.udev else { return };
        tracing::info!("session resumed");
        if udev.libinput.resume().is_err() {
            tracing::error!("libinput could not resume");
        }
        if let Some(gpu) = &mut udev.gpu {
            if let Err(error) = gpu.drm.activate(true) {
                tracing::error!("cannot re-activate the GPU: {error}");
            }
            for surface in gpu.surfaces.values_mut() {
                let _ = surface.compositor.reset_state();
                surface.compositor.reset_buffers();
                surface.redraw = Redraw::Idle;
            }
        }
        // Monitors may have changed while we were away.
        if let Some(dev_id) = self.udev.as_ref().and_then(|u| u.gpu.as_ref()).map(|g| g.dev_id) {
            self.udev_scan(dev_id);
        }
        self.queue_redraw_all();
    }
}

/// Turn adaptive sync on or off; returns whether it is on afterwards.
fn set_vrr(compositor: &mut GbmCompositor, connector: connector::Handle, on: bool, name: &str) -> bool {
    if on {
        match compositor.vrr_supported(connector) {
            Ok(VrrSupport::Supported | VrrSupport::RequiresModeset) => {}
            Ok(VrrSupport::NotSupported) | Err(_) => {
                tracing::info!("{name}: the display or driver does not support adaptive sync");
                return false;
            }
        }
    }
    match compositor.use_vrr(on) {
        Ok(()) => {
            tracing::info!("{name}: adaptive sync {}", if on { "on" } else { "off" });
            on
        }
        Err(error) => {
            tracing::warn!("{name}: cannot switch adaptive sync: {error}");
            !on
        }
    }
}

/// Buffer feedback for one output: what the renderer takes, plus a tranche that steers
/// clients towards buffers the display planes can scan out directly.
fn surface_feedback(renderer: &GlesRenderer, compositor: &GbmCompositor, render_node: DrmNode) -> Option<SurfaceFeedback> {
    let render_formats = renderer.dmabuf_formats();
    let scanout_formats: Vec<_> = compositor
        .surface()
        .plane_info()
        .formats
        .iter()
        .filter(|format| render_formats.contains(format))
        .copied()
        .collect();
    let scanout_device = compositor.surface().device_fd().dev_id().ok()?;
    let builder = DmabufFeedbackBuilder::new(render_node.dev_id(), render_formats.iter().copied());
    let render = builder
        .clone()
        .add_preference_tranche(render_node.dev_id(), None, render_formats.iter().copied())
        .build()
        .ok()?;
    let scanout = builder
        .add_preference_tranche(scanout_device, Some(TrancheFlags::Scanout), scanout_formats)
        .add_preference_tranche(render_node.dev_id(), None, render_formats.iter().copied())
        .build()
        .ok()?;
    Some(SurfaceFeedback { render, scanout })
}

fn gbm_clone(gbm: &GbmDevice<DrmDeviceFd>) -> GbmDevice<DrmDeviceFd> {
    gbm.clone()
}

#[cfg(test)]
mod tests {
    use drm_ffi::drm_mode_modeinfo;
    use smithay::reexports::drm::control::Mode as DrmMode;

    use super::*;

    fn mode(width: u16, height: u16, hz: u32, preferred: bool) -> DrmMode {
        // Totals chosen so that clock / (htotal * vtotal) is exactly `hz`.
        let (htotal, vtotal) = (u32::from(width) + 100, u32::from(height) + 50);
        DrmMode::from(drm_mode_modeinfo {
            clock: htotal * vtotal * hz / 1000,
            hdisplay: width,
            hsync_start: width + 10,
            hsync_end: width + 20,
            htotal: htotal as u16,
            hskew: 0,
            vdisplay: height,
            vsync_start: height + 5,
            vsync_end: height + 10,
            vtotal: vtotal as u16,
            vscan: 0,
            vrefresh: hz,
            flags: 0,
            type_: if preferred { 8 } else { 0 },
            name: [0; 32],
        })
    }

    fn refresh(m: DrmMode) -> i32 {
        Mode::from(m).refresh
    }

    #[test]
    fn preferred_mode_without_configuration() {
        let modes = [mode(2560, 1440, 60, false), mode(1920, 1080, 60, true), mode(1280, 720, 60, false)];
        assert_eq!(pick_mode(&modes, None, "t"), Some(modes[1]));
        assert_eq!(pick_mode(&modes[..1], None, "t"), Some(modes[0]), "first mode when none is preferred");
        assert_eq!(pick_mode(&[], None, "t"), None);
    }

    #[test]
    fn configured_size_and_refresh_pick_the_nearest_rate() {
        let modes = [mode(2560, 1440, 60, true), mode(2560, 1440, 144, false), mode(2560, 1440, 120, false)];
        let want = |refresh_mhz| Some(OutputMode { width: 2560, height: 1440, refresh_mhz });
        assert_eq!(refresh(pick_mode(&modes, want(Some(143_970)), "t").unwrap()), 144_000);
        assert_eq!(refresh(pick_mode(&modes, want(Some(59_950)), "t").unwrap()), 60_000);
        assert_eq!(refresh(pick_mode(&modes, want(None), "t").unwrap()), 144_000, "no rate asked: the highest");
    }

    #[test]
    fn unavailable_size_falls_back_to_the_preferred_mode() {
        let modes = [mode(1920, 1080, 60, false), mode(1280, 720, 60, true)];
        let want = Some(OutputMode { width: 3840, height: 2160, refresh_mhz: None });
        assert_eq!(pick_mode(&modes, want, "t"), Some(modes[1]));
    }

    #[test]
    fn transforms_map_one_to_one() {
        assert_eq!(transform(OutputTransform::Rotate270), Transform::_270);
        assert_eq!(transform(OutputTransform::Flipped90), Transform::Flipped90);
        assert_eq!(transform(OutputTransform::Normal), Transform::Normal);
    }
}
