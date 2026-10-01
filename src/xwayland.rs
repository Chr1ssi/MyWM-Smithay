//! Xwayland for the legacy X11 apps that remain (Steam itself, older games). Everything else
//! runs as native Wayland clients; `xwayland = false` in the config switches this off.
use std::{os::unix::io::OwnedFd, process::Stdio};

use smithay::{
    delegate_xwayland_shell,
    desktop::Window,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle},
    wayland::{
        selection::{
            SelectionHandler, SelectionSource, SelectionTarget,
            data_device::{
                clear_data_device_selection, request_data_device_client_selection, set_data_device_selection,
            },
            primary_selection::{clear_primary_selection, request_primary_client_selection, set_primary_selection},
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
        xwm::{Reorder, ResizeEdge, XwmId},
    },
};

use crate::State;

/// Where Xwayland finds the cursor theme. libXcursor only looks in a few fixed directories unless
/// `XCURSOR_PATH` says otherwise; on NixOS the themes live elsewhere, so X11 windows would get its
/// plain built-in cursor. Offer the same directories the compositor itself searches.
fn cursor_environment() -> Vec<(String, String)> {
    let mut env = Vec::new();
    if std::env::var_os("XCURSOR_PATH").is_none() {
        let mut dirs: Vec<String> = Vec::new();
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() {
            dirs.push(format!("{home}/.icons"));
            dirs.push(format!("{home}/.local/share/icons"));
            dirs.push(format!("{home}/.nix-profile/share/icons"));
        }
        let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
        dirs.extend(data_dirs.split(':').filter(|d| !d.is_empty()).map(|d| format!("{d}/icons")));
        dirs.extend(["/run/current-system/sw/share/icons".into(), "/usr/share/icons".into(), "/usr/share/pixmaps".into()]);
        dirs.dedup();
        env.push(("XCURSOR_PATH".to_owned(), dirs.join(":")));
    }
    // The compositor draws with this theme and size; X11 clients should match.
    if std::env::var_os("XCURSOR_SIZE").is_none() {
        env.push(("XCURSOR_SIZE".to_owned(), "24".to_owned()));
    }
    env
}

/// Spawn Xwayland and, once it is up, become its window manager.
pub fn start(state: &mut State) {
    if !state.config.xwayland {
        return;
    }
    let spawned = XWayland::spawn(
        &state.display_handle,
        None,
        cursor_environment(),
        true,
        Stdio::null(),
        Stdio::null(),
        |_| {},
    );
    let (xwayland, client) = match spawned {
        Ok(spawned) => spawned,
        Err(error) => {
            tracing::warn!("cannot start Xwayland (is it installed?): {error}");
            return;
        }
    };
    // The X sockets already listen, so clients started from now on can connect and will wait.
    state.xdisplay = Some(xwayland.display_number());
    tracing::info!("Xwayland on DISPLAY=:{}", xwayland.display_number());
    let handle = state.loop_handle.clone();
    let registered = handle.insert_source(xwayland, move |event, _, state| match event {
        XWaylandEvent::Ready { x11_socket, display_number } => {
            match X11Wm::start_wm(state.loop_handle.clone(), x11_socket, client.clone()) {
                Ok(wm) => {
                    tracing::info!("X11 window manager ready on :{display_number}");
                    state.xwm = Some(wm);
                }
                Err(error) => tracing::error!("cannot manage Xwayland: {error}"),
            }
        }
        XWaylandEvent::Error => {
            tracing::error!("Xwayland exited during startup; X11 apps will not work");
            state.xdisplay = None;
        }
    });
    if let Err(error) = registered {
        tracing::error!("cannot watch Xwayland: {}", error.error);
    }
}

impl XWaylandShellHandler for State {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    /// The window's `wl_surface` exists only from here on. A window that was focused before
    /// (a game right after it appears) could not get keyboard focus without one, so redo it.
    fn surface_associated(&mut self, _xwm: XwmId, _surface: WlSurface, _window: X11Surface) {
        self.refresh();
    }
}

delegate_xwayland_shell!(State);

impl XwmHandler for State {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().expect("the X11 window manager runs while Xwayland clients exist")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if window.set_mapped(true).is_err() {
            return;
        }
        self.add_x11_window(window.clone());
        if let Some(xwm) = self.xwm.as_mut() {
            let _ = xwm.raise_window(&window);
        }
    }

    /// Menus, tooltips and drop-downs position themselves; they are not managed windows.
    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let location = window.geometry().loc;
        let element = Window::new_x11_window(window);
        self.space.map_element(element.clone(), location, false);
        self.override_redirect.push(element);
        self.queue_redraw_all();
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.forget_x11_window(&window);
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.forget_x11_window(&window);
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        match self.desktop.by_x11(window.window_id()).map(|m| (m.id, m.floating)) {
            // A managed window keeps the place the layout gave it; floating ones may change size.
            Some((id, floating)) => {
                if floating && (w.is_some() || h.is_some()) {
                    if let (Some(area), Some(m)) = (self.desktop.monitor_area_of(id), self.desktop.get_mut(id)) {
                        let mut rect = m.floating_rect.unwrap_or(mywm_layout::Rect::centered(area.width, area.height));
                        rect.width = w.map_or(rect.width, |w| w as i32);
                        rect.height = h.map_or(rect.height, |h| h as i32);
                        m.floating_rect = Some(rect);
                    }
                    self.refresh();
                }
                let _ = window.configure(None);
            }
            // Not managed (yet): grant what it asks for.
            None => {
                let mut geometry = window.geometry();
                geometry.loc.x = x.unwrap_or(geometry.loc.x);
                geometry.loc.y = y.unwrap_or(geometry.loc.y);
                geometry.size.w = w.map_or(geometry.size.w, |w| w as i32);
                geometry.size.h = h.map_or(geometry.size.h, |h| h as i32);
                let _ = window.configure(geometry);
            }
        }
    }

    fn configure_notify(&mut self, _xwm: XwmId, window: X11Surface, geometry: Rectangle<i32, Logical>, _above: Option<u32>) {
        if !window.is_override_redirect() {
            return;
        }
        if let Some(element) = self.override_redirect.iter().find(|w| w.x11_surface() == Some(&window)).cloned() {
            self.space.map_element(element, geometry.loc, false);
            self.queue_redraw_all();
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _ = window.set_maximized(false);
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(id) = self.desktop.by_x11(window.window_id()).map(|m| m.id) {
            self.set_fullscreen(id, true);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(id) = self.desktop.by_x11(window.window_id()).map(|m| m.id) {
            self.set_fullscreen(id, false);
        }
    }

    // Moving and resizing are done with the pointer modifier, not by the client.
    fn resize_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32, _edge: ResizeEdge) {}

    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}

    fn send_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
        let result = match selection {
            SelectionTarget::Clipboard => request_data_device_client_selection(&self.seat, mime_type, fd).map_err(|e| format!("{e:?}")),
            SelectionTarget::Primary => request_primary_client_selection(&self.seat, mime_type, fd).map_err(|e| format!("{e:?}")),
        };
        if let Err(error) = result {
            tracing::warn!("cannot hand the Wayland selection to X11: {error}");
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        match selection {
            SelectionTarget::Clipboard => set_data_device_selection(&self.display_handle, &self.seat, mime_types, ()),
            SelectionTarget::Primary => set_primary_selection(&self.display_handle, &self.seat, mime_types, ()),
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => clear_data_device_selection(&self.display_handle, &self.seat),
            SelectionTarget::Primary => clear_primary_selection(&self.display_handle, &self.seat),
        }
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();

    /// A Wayland client took the selection: let X11 clients know.
    fn new_selection(&mut self, ty: SelectionTarget, source: Option<SelectionSource>, _seat: smithay::input::Seat<Self>) {
        if let Some(xwm) = self.xwm.as_mut()
            && let Err(error) = xwm.new_selection(ty, source.map(|source| source.mime_types()))
        {
            tracing::warn!("cannot pass the selection to X11: {error}");
        }
    }

    /// An X11 client wants the selection an X11 client owns.
    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: smithay::input::Seat<Self>,
        _user_data: &(),
    ) {
        if let Some(xwm) = self.xwm.as_mut()
            && let Err(error) = xwm.send_selection(ty, mime_type, fd, self.loop_handle.clone())
        {
            tracing::warn!("cannot read the X11 selection: {error}");
        }
    }
}

impl State {
    /// An X11 window went away or was unmapped: drop it from the layout or the popup list.
    fn forget_x11_window(&mut self, window: &X11Surface) {
        if let Some(index) = self.override_redirect.iter().position(|w| w.x11_surface() == Some(window)) {
            let element = self.override_redirect.remove(index);
            self.space.unmap_elem(&element);
            self.queue_redraw_all();
        } else {
            self.remove_x11_window(window.window_id());
        }
    }

    /// Environment for programs we start: X11 apps find Xwayland through `DISPLAY`.
    pub fn x11_display_env(&self) -> Option<String> {
        self.xdisplay.map(|n| format!(":{n}"))
    }
}
