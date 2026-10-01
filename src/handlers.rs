use std::os::unix::io::OwnedFd;

use smithay::{
    backend::{allocator::dmabuf::Dmabuf, renderer::utils::on_commit_buffer_handler},
    delegate_compositor, delegate_data_control, delegate_data_device, delegate_dmabuf, delegate_output, delegate_seat, delegate_shm,
    delegate_xdg_shell,
    desktop::{PopupKind, Window},
    input::{pointer::CursorImageStatus, Seat, SeatHandler, SeatState},
    reexports::wayland_server::{
        protocol::{wl_buffer, wl_output::WlOutput, wl_seat, wl_surface::WlSurface},
        Client,
    },
    utils::Serial,
    wayland::{
        buffer::BufferHandler,
        compositor::{get_parent, is_sync_subsurface, CompositorClientState, CompositorHandler, CompositorState},
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        output::OutputHandler,
        selection::{
            data_device::{
                set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
            },
            primary_selection::set_primary_focus,
            wlr_data_control::{DataControlHandler, DataControlState},
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
        shm::{ShmHandler, ShmState},
    },
};

use crate::state::{ClientState, State};
use smithay::{reexports::wayland_server::Resource, wayland::seat::WaylandFocus, xwayland::XWaylandClientData};

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // Xwayland connects as a client of its own kind.
        if let Some(data) = client.get_data::<XWaylandClientData>() {
            return &data.compositor_state;
        }
        &client.get_data::<ClientState>().expect("every client has compositor state").compositor_state
    }

    fn new_surface(&mut self, surface: &WlSurface) {
        self.install_sync_blockers(surface);
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if self.layer_commit(surface) {
            return;
        }
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            self.place_pending(&root);
            if let Some(window) = self
                .space
                .elements()
                .find(|w| w.wl_surface().is_some_and(|s| *s == root))
            {
                window.on_commit();
            }
        }
        self.popups.commit(surface);
        // A popup is mapped by its first configure, which the compositor has to send.
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface)
            && !popup.is_initial_configure_sent()
        {
            let _ = popup.send_configure();
        }
        self.count_commit(surface);
        self.queue_redraw_for(surface);
    }
}

impl State {
    /// Move a popup (menu, tooltip) so it stays on the output of its window: flipped or slid
    /// the way the client allowed in its positioner.
    ///
    /// xdg-shell gives the popup's position and the target rectangle relative to the parent's *window geometry*,
    /// whose top left corner is where the window sits on the screen (`element_location`).
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let kind = PopupKind::Xdg(popup.clone());
        let Ok(root) = smithay::desktop::find_popup_root_surface(&kind) else { return };
        let Some(window) = self.space.elements().find(|w| w.wl_surface().is_some_and(|s| *s == root)) else { return };
        let Some(location) = self.space.element_location(window) else { return };
        // The output of the window's workspace; the space's own idea (`outputs_for_element`) as a fallback.
        let by_layout = self.desktop.by_surface(&root).and_then(|m| self.monitor_of_window(m)).and_then(|monitor| self.outputs.get(monitor)).map(|e| e.output.clone());
        let Some(output) = by_layout.or_else(|| self.space.outputs_for_element(window).first().cloned()) else { return };
        let Some(output_geo) = self.space.output_geometry(&output) else { return };
        let mut target = output_geo;
        target.loc -= location;
        target.loc -= smithay::desktop::get_popup_toplevel_coords(&kind);
        popup.with_pending_state(|s| s.geometry = s.positioner.get_unconstrained_geometry(target));
    }

    /// A surface changed: redraw the outputs it shows on (all of them if it is not a plain window).
    fn queue_redraw_for(&mut self, surface: &WlSurface) {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        let window = self.space.elements().find(|w| w.wl_surface().is_some_and(|s| *s == root)).cloned();
        match window {
            Some(window) => {
                for output in self.space.outputs_for_element(&window) {
                    self.queue_redraw_output(&output);
                }
            }
            None => self.queue_redraw_all(),
        }
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        self.update_foreign_toplevel(surface.wl_surface());
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        self.update_foreign_toplevel(surface.wl_surface());
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        self.add_window(Window::new_wayland_window(surface));
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.remove_window(surface.wl_surface());
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<WlOutput>) {
        match self.desktop.by_surface(surface.wl_surface()).map(|w| w.id) {
            Some(id) => self.set_fullscreen(id, true),
            None => {
                surface.send_configure();
            }
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.desktop.by_surface(surface.wl_surface()).map(|w| w.id) {
            self.set_fullscreen(id, false);
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        // The layout decides sizes; acknowledge so the client does not wait.
        surface.send_pending_configure();
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        surface.with_pending_state(|s| s.geometry = positioner.get_geometry());
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|s| {
            s.geometry = positioner.get_geometry();
            s.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        // Clipboard and primary selection are offered to the client that has keyboard focus.
        let display = &self.display_handle;
        let client = focused.and_then(|surface| display.get_client(surface.id()).ok());
        set_data_device_focus(display, seat, client.clone());
        set_primary_focus(display, seat, client);
    }
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
        self.queue_redraw_all();
    }
}

impl DataControlHandler for State {
    fn data_control_state(&self) -> &DataControlState {
        &self.data_control_state
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}
impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {
    fn send(&mut self, _mime_type: String, _fd: OwnedFd, _seat: Seat<Self>) {}
}
impl OutputHandler for State {}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        self.udev_dmabuf_imported(&dmabuf, notifier);
    }
}

impl State {
    pub fn set_keyboard_focus(&mut self, surface: Option<WlSurface>) {
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, surface, serial);
        }
    }
}

delegate_compositor!(State);
delegate_shm!(State);
delegate_xdg_shell!(State);
delegate_seat!(State);
delegate_data_device!(State);
delegate_output!(State);
delegate_data_control!(State);
delegate_dmabuf!(State);
