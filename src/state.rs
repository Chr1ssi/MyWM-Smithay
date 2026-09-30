use std::{any::Any, ffi::OsString, sync::Arc, time::Instant};

use crate::{cursor::CursorAssets, desktop::Desktop, monitors::OutputEntry, udev::UdevData};
use mywm_config::{Binding, Config, Modifiers};
use smithay::{
    desktop::{PopupManager, Space, Window},
    backend::session::libseat::LibSeatSession,
    input::{keyboard::XkbConfig, pointer::CursorImageStatus, Seat, SeatState},
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction},
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
    utils::{Logical, Point},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        content_type::ContentTypeState,
        cursor_shape::CursorShapeManagerState,
        fractional_scale::FractionalScaleManagerState,
        pointer_constraints::PointerConstraintsState,
        relative_pointer::RelativePointerManagerState,
        shell::xdg::decoration::XdgDecorationState,
        viewporter::ViewporterState,
        dmabuf::{DmabufGlobal, DmabufState},
        drm_syncobj::DrmSyncobjState,
        presentation::PresentationState,
        selection::primary_selection::PrimarySelectionState,
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::{wlr_layer::WlrLayerShellState, xdg::XdgShellState},
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
    pub layer_shell_state: WlrLayerShellState,
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
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
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
                state
                    .display_handle
                    .insert_client(stream, Arc::new(ClientState::default()))
                    .expect("insert client");
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
            desktop,
            outputs: Vec::new(),
            next_output_id: 1,
            bindings: Vec::new(),
            cursor_status: CursorImageStatus::default_named(),
            cursor_assets: CursorAssets::new(),
            udev: None,
            session: None,
            dmabuf_state: DmabufState::new(),
            layer_shell_state: WlrLayerShellState::new::<State>(&dh),
            layer_focus: None,
            primary_selection_state: PrimarySelectionState::new::<State>(&dh),
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
        if with_theme {
            cmd.envs(self.config.theme_env());
            cmd.env("MYWM_TERMINAL_COUNT", self.config.terminal.len().to_string());
            for (index, argument) in self.config.terminal.iter().enumerate() {
                cmd.env(format!("MYWM_TERMINAL_{index}"), argument);
            }
            cmd.env("MYWM_LAUNCHER_X", (self.pointer_location.x as i32).to_string())
                .env("MYWM_LAUNCHER_Y", (self.pointer_location.y as i32).to_string());
        }
        match cmd.spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(error) => tracing::warn!("cannot start {program:?}: {error}"),
        }
    }

}
