use std::{any::Any, collections::HashSet, ffi::OsString, sync::Arc, time::Instant};

use crate::{screencopy::PendingCopy, cursor::CursorAssets, desktop::Desktop, monitors::OutputEntry, session::SessionLock, udev::UdevData};
use mywm_config::{Binding, Config, Modifiers};
use smithay::{
    desktop::{PopupManager, Space, Window},
    output::Output,
    backend::session::libseat::LibSeatSession,
    input::{keyboard::XkbConfig, pointer::CursorImageStatus, Seat, SeatState},
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction},
        wayland_protocols_wlr::output_power_management::v1::server::zwlr_output_power_v1::ZwlrOutputPowerV1,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
    utils::{Logical, Point},
    xwayland::{X11Surface, X11Wm},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        content_type::ContentTypeState,
        idle_inhibit::IdleInhibitManagerState,
        idle_notify::IdleNotifierState,
        session_lock::SessionLockManagerState,
        cursor_shape::CursorShapeManagerState,
        fractional_scale::FractionalScaleManagerState,
        pointer_constraints::PointerConstraintsState,
        relative_pointer::RelativePointerManagerState,
        shell::xdg::decoration::XdgDecorationState,
        viewporter::ViewporterState,
        dmabuf::{DmabufGlobal, DmabufState},
        drm_syncobj::DrmSyncobjState,
        presentation::PresentationState,
        selection::{primary_selection::PrimarySelectionState, wlr_data_control::DataControlState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::{wlr_layer::WlrLayerShellState, xdg::XdgShellState},
        xwayland_shell::XWaylandShellState,
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};

pub struct State {
    pub start_time: Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub loop_signal: LoopSignal,

    pub space: Space<Window>,
    pub popups: PopupManager,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<State>,
    pub data_device_state: DataDeviceState,

    pub seat: Seat<State>,
    pub pointer_location: Point<f64, Logical>,
    pub foreign_toplevels: smithay::wayland::foreign_toplevel_list::ForeignToplevelListState,
    pub image_capture: crate::image_capture::ImageCaptureState,
    pub output_management: crate::output_management::OutputManagement,
    /// The output the cursor was last drawn on, so it is erased there when it moves away.
    pub cursor_output: Option<Output>,

    pub desktop: Desktop,
    pub outputs: Vec<OutputEntry>,
    pub next_output_id: u32,

    pub config: Config,
    pub cursor_status: CursorImageStatus,
    pub cursor_assets: CursorAssets,
    /// Present with the hardware backend.
    pub udev: Option<UdevData>,
    pub session: Option<LibSeatSession>,
    pub dmabuf_state: DmabufState,
    pub data_control_state: DataControlState,
    pub layer_shell_state: WlrLayerShellState,
    /// Screen capture requests waiting for their output's next frame.
    pub pending_copies: Vec<PendingCopy>,
    pub xwayland_shell_state: XWaylandShellState,
    pub xwm: Option<X11Wm>,
    /// The X11 window that currently has the X11 input focus.
    pub x11_focus: Option<X11Surface>,
    /// Display number of Xwayland, once started.
    pub xdisplay: Option<u32>,
    /// Menus, tooltips and other X11 windows that position themselves.
    pub override_redirect: Vec<Window>,
    pub lock_manager_state: SessionLockManagerState,
    pub session_lock: SessionLock,
    pub idle_notifier_state: IdleNotifierState<State>,
    /// Surfaces that ask the desktop not to go idle (video players).
    pub idle_inhibitors: HashSet<WlSurface>,
    pub output_power_objects: Vec<(Output, ZwlrOutputPowerV1)>,
    /// Layer surface the user clicked and that accepts keyboard input.
    pub layer_focus: Option<WlSurface>,
    pub primary_selection_state: PrimarySelectionState,
    pub syncobj_state: Option<DrmSyncobjState>,
    pub presentation_state: Option<PresentationState>,
    pub loop_handle: LoopHandle<'static, State>,
    /// Protocol globals that only need to stay alive.
    _protocols: Vec<Box<dyn Any>>,
    /// Surface under the pointer, for pointer constraints.
    pub pointer_focus_surface: Option<WlSurface>,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub ipc: Option<crate::ipc::Ipc>,
    /// The desktop changed since the last broadcast to bar clients.
    pub ipc_dirty: bool,
    /// Key bindings after the nested-modifier remapping.
    pub bindings: Vec<Binding>,
    pub pointer_modifiers: Modifiers,
    /// While nested, Super belongs to the host, so bindings use Alt instead.
    pub remap_super: bool,
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
    /// Program name of the client (from `/proc/<pid>/comm`), for the log.
    pub name: std::sync::OnceLock<String>,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}

    fn disconnected(&self, _client_id: ClientId, reason: DisconnectReason) {
        let name = self.name.get().map_or("?", String::as_str);
        match reason {
            DisconnectReason::ConnectionClosed => tracing::debug!("client {name} disconnected"),
            // A protocol error means the compositor kicked the client out: that is worth a warning.
            DisconnectReason::ProtocolError(error) => tracing::warn!(
                "client {name} was disconnected for a protocol error: {} (object {}@{}, code {})",
                error.message,
                error.object_interface,
                error.object_id,
                error.code
            ),
        }
    }
}

impl State {
    pub fn new(event_loop: &mut EventLoop<'static, State>, display: Display<State>, config: Config, remap_super: bool) -> Self {
        let dh = display.handle();
        let loop_handle = event_loop.handle();

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "seat0");
        let xkb = XkbConfig {
            layout: &config.keyboard.layout,
            variant: &config.keyboard.variant,
            options: (!config.keyboard.options.is_empty()).then(|| config.keyboard.options.clone()),
            ..Default::default()
        };
        seat.add_keyboard(xkb, 200, 25).expect("keyboard");
        seat.add_pointer();

        let socket = ListeningSocketSource::new_auto().expect("wayland socket");
        let socket_name = socket.socket_name().to_os_string();
        loop_handle
            .insert_source(socket, |stream, _, state: &mut State| {
                let data = Arc::new(ClientState::default());
                let client = state.display_handle.insert_client(stream, data.clone()).expect("insert client");
                let name = client
                    .get_credentials(&state.display_handle)
                    .ok()
                    .map(|c| {
                        let comm = std::fs::read_to_string(format!("/proc/{}/comm", c.pid)).unwrap_or_default();
                        format!("{} (pid {})", comm.trim(), c.pid)
                    })
                    .unwrap_or_else(|| "unknown".into());
                tracing::debug!("client {name} connected");
                let _ = data.name.set(name);
            })
            .expect("socket source");

        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // SAFETY: the display is never dropped while the source is alive.
                    unsafe { display.get_mut().dispatch_clients(state).unwrap() };
                    Ok(PostAction::Continue)
                },
            )
            .expect("display source");

        let desktop = Desktop::new(
            config.workspace_outputs.clone(),
            config.gaming_output.clone(),
            config.appearance.layout(),
        );
        let primary_selection_state = PrimarySelectionState::new::<State>(&dh);
        let mut state = Self {
            start_time: Instant::now(),
            socket_name,
            display_handle: dh.clone(),
            loop_signal: event_loop.get_signal(),
            space: Space::default(),
            popups: PopupManager::default(),
            compositor_state: CompositorState::new::<State>(&dh),
            xdg_shell_state: XdgShellState::new::<State>(&dh),
            shm_state: ShmState::new::<State>(&dh, vec![]),
            output_manager_state: OutputManagerState::new_with_xdg_output::<State>(&dh),
            seat_state,
            data_device_state: DataDeviceState::new::<State>(&dh),
            seat,
            pointer_location: (0.0, 0.0).into(),
            foreign_toplevels: smithay::wayland::foreign_toplevel_list::ForeignToplevelListState::new::<State>(&dh),
            image_capture: Default::default(),
            output_management: Default::default(),
            cursor_output: None,
            desktop,
            outputs: Vec::new(),
            next_output_id: 1,
            bindings: Vec::new(),
            cursor_status: CursorImageStatus::default_named(),
            cursor_assets: CursorAssets::new(),
            udev: None,
            session: None,
            dmabuf_state: DmabufState::new(),
            // Clipboard managers and wl-copy/wl-paste use this; MYWM_NO_DATA_CONTROL=1 hides it
            // so the plain focus-based clipboard path can be tested.
            data_control_state: DataControlState::new::<State, _>(
                &dh,
                Some(&primary_selection_state),
                |_| std::env::var_os("MYWM_NO_DATA_CONTROL").is_none(),
            ),
            layer_shell_state: WlrLayerShellState::new::<State>(&dh),
            pending_copies: Vec::new(),
            xwayland_shell_state: XWaylandShellState::new::<State>(&dh),
            xwm: None,
            x11_focus: None,
            xdisplay: None,
            override_redirect: Vec::new(),
            lock_manager_state: SessionLockManagerState::new::<State, _>(&dh, |_| true),
            session_lock: SessionLock::default(),
            idle_notifier_state: IdleNotifierState::new(&dh, loop_handle.clone()),
            idle_inhibitors: HashSet::new(),
            output_power_objects: Vec::new(),
            layer_focus: None,
            primary_selection_state,
            syncobj_state: None,
            presentation_state: None,
            loop_handle: loop_handle.clone(),
            _protocols: vec![
                Box::new(ViewporterState::new::<State>(&dh)),
                Box::new(FractionalScaleManagerState::new::<State>(&dh)),
                Box::new(CursorShapeManagerState::new::<State>(&dh)),
                Box::new(ContentTypeState::new::<State>(&dh)),
                Box::new(RelativePointerManagerState::new::<State>(&dh)),
                Box::new(PointerConstraintsState::new::<State>(&dh)),
                Box::new(XdgDecorationState::new::<State>(&dh)),
                Box::new(State::create_tearing_control_global(&dh)),
                Box::new(IdleInhibitManagerState::new::<State>(&dh)),
                Box::new(State::create_screencopy_global(&dh)),
                Box::new(State::create_output_power_global(&dh)),
                Box::new(State::create_output_management_global(&dh)),
                Box::new(State::create_image_capture_globals(&dh)),
            ],
            pointer_focus_surface: None,
            dmabuf_global: None,
            ipc: None,
            ipc_dirty: false,
            pointer_modifiers: Modifiers::default(),
            remap_super,
            config,
        };
        state.install_bindings();
        state
    }

    /// Derive key bindings from the config; nested, Super is mapped to Alt.
    pub fn install_bindings(&mut self) {
        let remap = |mut m: Modifiers| -> Option<Modifiers> {
            if self.remap_super && m.logo {
                if m.alt {
                    return None;
                }
                m.logo = false;
                m.alt = true;
            }
            Some(m)
        };
        self.bindings = self
            .config
            .keybindings()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|b| Some(Binding { modifiers: remap(b.modifiers)?, ..b }))
            .collect();
        self.pointer_modifiers = remap(self.config.pointer_modifiers().unwrap_or_default()).unwrap_or_default();
    }

    /// Re-read the config file; on error keep the running configuration.
    pub fn reload_config(&mut self) {
        let new = match Config::load() {
            Ok(new) => new,
            Err(error) => {
                tracing::error!("config reload failed: {error}");
                return;
            }
        };
        let xkb = XkbConfig {
            layout: &new.keyboard.layout,
            variant: &new.keyboard.variant,
            options: (!new.keyboard.options.is_empty()).then(|| new.keyboard.options.clone()),
            ..Default::default()
        };
        if let Some(keyboard) = self.seat.get_keyboard()
            && let Err(error) = keyboard.set_xkb_config(self, xkb)
        {
            tracing::warn!("keyboard layout not applied: {error:?}");
        }
        // Monitor and session settings need a restart; everything else applies live.
        let Config { workspace_outputs, gaming_output, async_outputs, idle, vrr, .. } =
            std::mem::take(&mut self.config);
        self.config = Config { workspace_outputs, gaming_output, async_outputs, idle, vrr, ..new };
        self.desktop.appearance = self.config.appearance.layout();
        if let Some(udev) = &mut self.udev {
            udev.late_margin = crate::udev::late_margin(&self.config.render);
        }
        self.install_bindings();
        self.refresh();
        tracing::info!("configuration reloaded");
    }

    /// Start a program without a shell; the child is reaped in the background.
    pub fn spawn_command(&self, command: &[String], with_theme: bool) {
        let Some((program, args)) = command.split_first() else { return };
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        cmd.env("WAYLAND_DISPLAY", &self.socket_name);
        if let Some(display) = self.x11_display_env() {
            cmd.env("DISPLAY", display);
        }
        if with_theme {
            cmd.envs(self.config.theme_env());
            cmd.envs(self.config.terminal_env());
            cmd.env("MYWM_LAUNCHER_X", (self.pointer_location.x as i32).to_string())
                .env("MYWM_LAUNCHER_Y", (self.pointer_location.y as i32).to_string());
        }
        Self::spawn_reaped(cmd, &format!("{program:?}"));
    }

    /// Open the wallpaper picker of the running wallpaper layer at the pointer.
    pub fn open_wallpaper_picker(&self) {
        let mut cmd = mywm_theme::picker_command(self.pointer_location.x as i32, self.pointer_location.y as i32);
        cmd.env("WAYLAND_DISPLAY", &self.socket_name);
        Self::spawn_reaped(cmd, "the wallpaper picker (is `mywm-compositor --wallpaper` running?)");
    }

    fn spawn_reaped(mut cmd: std::process::Command, what: &str) {
        match cmd.spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(error) => tracing::warn!("cannot start {what}: {error}"),
        }
    }

}
