//! Session state: the screen lock (`ext-session-lock`), idle detection and monitor power.
use std::{collections::HashSet, process::Child};

use smithay::{
    delegate_idle_inhibit, delegate_idle_notify, delegate_session_lock,
    desktop::utils::under_from_surface_tree,
    desktop::WindowSurfaceType,
    output::Output,
    reexports::{
        wayland_protocols_wlr::output_power_management::v1::server::{
            zwlr_output_power_manager_v1::{self, ZwlrOutputPowerManagerV1},
            zwlr_output_power_v1::{self, ZwlrOutputPowerV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
            protocol::{wl_output::WlOutput, wl_surface::WlSurface},
        },
    },
    utils::{Logical, Point},
    wayland::{
        idle_inhibit::IdleInhibitHandler,
        idle_notify::{IdleNotifierHandler, IdleNotifierState},
        session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker},
    },
};

use crate::State;

enum LockState {
    Unlocked,
    /// The locker asked; its confirmation is sent once every output showed the lock.
    Locking(SessionLocker),
    Locked,
}

pub struct SessionLock {
    state: LockState,
    surfaces: Vec<(Output, LockSurface)>,
    /// Outputs that still have to draw a locked frame before the lock is confirmed.
    unconfirmed: HashSet<String>,
    locker: Option<Child>,
}

impl Default for SessionLock {
    fn default() -> Self {
        Self { state: LockState::Unlocked, surfaces: Vec::new(), unconfirmed: HashSet::new(), locker: None }
    }
}

impl SessionLock {
    /// The session is locked or about to be: nothing but the lock may be shown or reached.
    pub fn is_active(&self) -> bool {
        !matches!(self.state, LockState::Unlocked)
    }

    /// The lock was confirmed to the locker (what the bar protocol calls `locked 1`).
    pub fn is_locked(&self) -> bool {
        matches!(self.state, LockState::Locked)
    }
}

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.lock_manager_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        if self.session_lock.is_active() {
            // Already locked: dropping the request tells the second locker `finished`.
            return;
        }
        self.end_drag();
        self.session_lock.unconfirmed = self.outputs.iter().map(|e| e.output.name()).collect();
        if self.session_lock.unconfirmed.is_empty() {
            confirmation.lock();
            self.session_lock.state = LockState::Locked;
        } else {
            self.session_lock.state = LockState::Locking(confirmation);
        }
        tracing::info!("session locking");
        self.refresh();
    }

    fn unlock(&mut self) {
        tracing::info!("session unlocked");
        self.session_lock.state = LockState::Unlocked;
        self.session_lock.surfaces.clear();
        self.session_lock.unconfirmed.clear();
        self.refresh();
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(output) = Output::from_resource(&output) else { return };
        if let Some(geo) = self.space.output_geometry(&output) {
            surface.with_pending_state(|state| state.size = Some((geo.size.w as u32, geo.size.h as u32).into()));
        }
        surface.send_configure();
        self.session_lock.surfaces.push((output, surface));
        self.refresh();
    }
}

delegate_session_lock!(State);

impl State {
    pub fn lock_surface_for(&self, output: &Output) -> Option<&LockSurface> {
        self.session_lock.surfaces.iter().find(|(o, _)| o == output).map(|(_, s)| s)
    }

    /// The lock surface of the output under the pointer (or the focused monitor's).
    pub fn active_lock_surface(&self) -> Option<&LockSurface> {
        let output = self
            .pointer_monitor()
            .or(Some(self.desktop.focused_monitor))
            .and_then(|monitor| self.outputs.get(monitor))
            .map(|entry| &entry.output)?;
        self.lock_surface_for(output).or_else(|| self.session_lock.surfaces.first().map(|(_, s)| s))
    }

    /// The lock surface under the pointer and the global position of its origin.
    pub fn lock_surface_at_pointer(&self) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output = self.space.output_under(self.pointer_location).next()?;
        let geo = self.space.output_geometry(output)?;
        let surface = self.lock_surface_for(output)?;
        let relative = self.pointer_location - geo.loc.to_f64();
        under_from_surface_tree(surface.wl_surface(), relative, (0, 0), WindowSurfaceType::ALL)
            .map(|(surface, origin)| (surface, (origin + geo.loc).to_f64()))
    }

    /// An output showed a frame: once all did while locking, the lock counts.
    pub fn note_locked_frame(&mut self, output: &Output) {
        if !matches!(self.session_lock.state, LockState::Locking(_)) {
            return;
        }
        self.session_lock.unconfirmed.remove(&output.name());
        if self.session_lock.unconfirmed.is_empty()
            && let LockState::Locking(confirmation) = std::mem::replace(&mut self.session_lock.state, LockState::Locked)
        {
            confirmation.lock();
            tracing::info!("session locked");
            self.ipc_dirty = true;
        }
    }

    /// Start the screen locker (`swaylock` with the palette's colors) unless the session is locked.
    pub fn start_locker(&mut self) {
        if self.session_lock.is_active() {
            return;
        }
        if let Some(child) = &mut self.session_lock.locker {
            match child.try_wait() {
                Ok(None) => return,
                _ => self.session_lock.locker = None,
            }
        }
        let mut command = mywm_config::session::locker(&self.config);
        command.env("WAYLAND_DISPLAY", &self.socket_name);
        match command.spawn() {
            Ok(child) => self.session_lock.locker = Some(child),
            Err(error) => tracing::warn!("cannot start the screen locker: {error}"),
        }
    }
}

// --- Idle ---------------------------------------------------------------------------------

impl IdleNotifierHandler for State {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.idle_notifier_state
    }
}

impl IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.insert(surface);
        self.update_idle_inhibition();
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.remove(&surface);
        self.update_idle_inhibition();
    }
}

impl State {
    /// Idle timers stand still while a video player (or anything else) holds an inhibitor.
    pub fn update_idle_inhibition(&mut self) {
        self.idle_inhibitors.retain(|surface| surface.is_alive());
        let inhibited = !self.idle_inhibitors.is_empty();
        self.idle_notifier_state.set_is_inhibited(inhibited);
    }
}

delegate_idle_notify!(State);
delegate_idle_inhibit!(State);

// --- Monitor power (wlr-output-power-management) --------------------------------------------

impl State {
    pub fn create_output_power_global(display: &DisplayHandle) -> smithay::reexports::wayland_server::backend::GlobalId {
        display.create_global::<State, ZwlrOutputPowerManagerV1, ()>(1, ())
    }

    /// Switch an output's display on or off (DPMS) and tell everyone watching.
    pub fn set_output_power(&mut self, output: &Output, on: bool) {
        let Some(entry) = self.outputs.iter_mut().find(|e| &e.output == output) else { return };
        if entry.powered == on {
            return;
        }
        entry.powered = on;
        tracing::info!("{}: display {}", output.name(), if on { "on" } else { "off" });
        self.udev_set_power(output, on);
        let mode = if on { zwlr_output_power_v1::Mode::On } else { zwlr_output_power_v1::Mode::Off };
        self.output_power_objects.retain(|(o, resource)| {
            if o == output && resource.is_alive() {
                resource.mode(mode);
            }
            resource.is_alive()
        });
    }
}

impl GlobalDispatch<ZwlrOutputPowerManagerV1, (), State> for State {
    fn bind(
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrOutputPowerManagerV1>,
        _data: &(),
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrOutputPowerManagerV1, (), State> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        _manager: &ZwlrOutputPowerManagerV1,
        request: zwlr_output_power_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        if let zwlr_output_power_manager_v1::Request::GetOutputPower { id, output } = request {
            let resource = data_init.init(id, OutputPowerData(Output::from_resource(&output)));
            match state.outputs.iter().find(|e| Some(&e.output) == Output::from_resource(&output).as_ref()) {
                Some(entry) => {
                    resource.mode(if entry.powered {
                        zwlr_output_power_v1::Mode::On
                    } else {
                        zwlr_output_power_v1::Mode::Off
                    });
                    state.output_power_objects.push((entry.output.clone(), resource));
                }
                None => resource.failed(),
            }
        }
    }
}

pub struct OutputPowerData(Option<Output>);

impl Dispatch<ZwlrOutputPowerV1, OutputPowerData, State> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        _resource: &ZwlrOutputPowerV1,
        request: zwlr_output_power_v1::Request,
        data: &OutputPowerData,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        if let zwlr_output_power_v1::Request::SetMode { mode } = request
            && let Some(output) = &data.0
        {
            match mode {
                WEnum::Value(zwlr_output_power_v1::Mode::On) => state.set_output_power(output, true),
                WEnum::Value(zwlr_output_power_v1::Mode::Off) => state.set_output_power(output, false),
                _ => {}
            }
        }
    }
}
