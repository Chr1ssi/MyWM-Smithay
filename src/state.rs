use std::{ffi::OsString, sync::Arc, time::Instant};

use smithay::{
    desktop::{PopupManager, Space, Window},
    input::{Seat, SeatState},
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, LoopSignal, Mode, PostAction},
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            Display, DisplayHandle,
        },
    },
    utils::{Logical, Point},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::XdgShellState,
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
    pub fn new(event_loop: &mut EventLoop<State>, display: Display<State>) -> Self {
        let dh = display.handle();
        let loop_handle = event_loop.handle();

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "seat0");
        seat.add_keyboard(Default::default(), 200, 25).expect("keyboard");
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

        Self {
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
        }
    }

    pub fn spawn(&self, cmd: &str) {
        if let Err(err) = std::process::Command::new("sh").arg("-c").arg(cmd).spawn() {
            tracing::warn!("failed to spawn {cmd:?}: {err}");
        }
    }

    /// Placeholder layout for M0: equal-width columns over the first output.
    /// Replaced by the scrolling layout crate in M1.
    pub fn relayout(&mut self) {
        let Some(output) = self.space.outputs().next().cloned() else { return };
        let Some(geo) = self.space.output_geometry(&output) else { return };
        let windows: Vec<Window> = self.space.elements().cloned().collect();
        if windows.is_empty() {
            return;
        }
        let width = geo.size.w / windows.len() as i32;
        for (i, window) in windows.iter().enumerate() {
            if let Some(toplevel) = window.toplevel() {
                toplevel.with_pending_state(|s| s.size = Some((width, geo.size.h).into()));
                toplevel.send_pending_configure();
            }
            self.space
                .map_element(window.clone(), (geo.loc.x + i as i32 * width, geo.loc.y), false);
        }
    }
}
