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

/// The cursor settings for Xwayland. Smithay starts it with an otherwise empty environment, so whatever
/// is not passed here is lost: without `XCURSOR_THEME` and `XCURSOR_PATH` libXcursor finds no theme (on
/// NixOS they live outside its fixed search directories) and every X11 window gets the built-in 10x16
/// X cursor. The compositor draws with the same theme and size (see `cursor::export_theme`).
fn cursor_environment() -> Vec<(String, String)> {
    let current = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let path = current("XCURSOR_PATH").unwrap_or_else(|| {
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
        dirs.join(":")
    });
    let mut env = vec![("XCURSOR_PATH".to_owned(), path), ("XCURSOR_SIZE".to_owned(), current("XCURSOR_SIZE").unwrap_or_else(|| "24".to_owned()))];
    if let Some(theme) = current("XCURSOR_THEME") {
        env.push(("XCURSOR_THEME".to_owned(), theme));
    }
    // libXcursor also looks below $HOME, so give it the home directory too.
    if let Some(home) = current("HOME") {
        env.push(("HOME".to_owned(), home));
    }
    env
}

/// Spawn Xwayland and, once it is up, become its window manager.
pub fn start(state: &mut State) {
    if !state.config.xwayland {
        return;
    }
    let environment = cursor_environment();
    tracing::info!(
        "Xwayland cursor: theme {:?}, size {:?}, {} search directories",
        environment.iter().find(|(k, _)| k == "XCURSOR_THEME").map(|(_, v)| v.as_str()),
        environment.iter().find(|(k, _)| k == "XCURSOR_SIZE").map(|(_, v)| v.as_str()),
        environment.iter().find(|(k, _)| k == "XCURSOR_PATH").map_or(0, |(_, v)| v.split(':').count())
    );
    let spawned = XWayland::spawn(
        &state.display_handle,
        None,
        environment,
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
        tracing::info!("X11 window unmapped: {:?} ({})", window.title(), window.class());
        self.forget_x11_window(&window);
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::info!("X11 window destroyed: {:?} ({})", window.title(), window.class());
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
        tracing::info!("X11 window asks for fullscreen: {:?} ({})", window.title(), window.class());
        if let Some(id) = self.desktop.by_x11(window.window_id()).map(|m| m.id) {
            self.set_fullscreen(id, true);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::info!("X11 window leaves fullscreen: {:?} ({})", window.title(), window.class());
        if let Some(id) = self.desktop.by_x11(window.window_id()).map(|m| m.id) {
            self.set_fullscreen(id, false);
        }
    }

    // Wine minimizes an exclusive-fullscreen window when it loses the focus. Nothing is done about it yet; the
    // requests are logged to find out whether that is what blacks the game out.
    fn minimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::info!("X11 window asks to be minimized (ignored): {:?} ({})", window.title(), window.class());
    }

    fn unminimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::info!("X11 window asks to be restored (ignored): {:?} ({})", window.title(), window.class());
    }

    // Moving and resizing are done with the pointer modifier, not by the client.
    fn resize_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32, _edge: ResizeEdge) {}

    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}

    /// X11 clients may read the Wayland clipboard while an X11 window has the keyboard focus (they
    /// could otherwise spy on it in the background).
    fn allow_selection_access(&mut self, _xwm: XwmId, _selection: SelectionTarget) -> bool {
        use smithay::reexports::wayland_server::Resource;
        let Some(surface) = self.seat.get_keyboard().and_then(|keyboard| keyboard.current_focus()) else { return false };
        let allowed = self
            .display_handle
            .get_client(surface.id())
            .is_ok_and(|client| client.get_data::<smithay::xwayland::XWaylandClientData>().is_some());
        tracing::debug!("an X11 client wants the selection: {}", if allowed { "allowed" } else { "refused" });
        allowed
    }

    fn send_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
        tracing::debug!("X11 asks for the Wayland {selection:?} as {mime_type}");
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
        tracing::debug!("selection {ty:?} by a Wayland client: {:?}", source.as_ref().map(|s| s.mime_types()));
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

#[cfg(test)]
mod tests {
    use super::cursor_environment;

    fn value(env: &[(String, String)], key: &str) -> Option<String> {
        env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    /// Xwayland starts with an empty environment: the compositor's own cursor settings must be passed
    /// on, also when they are set (they used to be passed only when unset, so X11 windows lost the theme).
    #[test]
    fn passes_the_compositors_cursor_settings_on() {
        // SAFETY: the only test that touches these variables.
        unsafe {
            std::env::set_var("XCURSOR_THEME", "Test-Theme");
            std::env::set_var("XCURSOR_SIZE", "32");
            std::env::set_var("XCURSOR_PATH", "/one:/two");
            std::env::set_var("HOME", "/home/test");
        }
        let env = cursor_environment();
        assert_eq!(value(&env, "XCURSOR_THEME").as_deref(), Some("Test-Theme"));
        assert_eq!(value(&env, "XCURSOR_SIZE").as_deref(), Some("32"));
        assert_eq!(value(&env, "XCURSOR_PATH").as_deref(), Some("/one:/two"));
        assert_eq!(value(&env, "HOME").as_deref(), Some("/home/test"));

        // SAFETY: as above.
        unsafe {
            std::env::remove_var("XCURSOR_THEME");
            std::env::remove_var("XCURSOR_SIZE");
            std::env::remove_var("XCURSOR_PATH");
        }
        let env = cursor_environment();
        assert_eq!(value(&env, "XCURSOR_THEME"), None);
        assert_eq!(value(&env, "XCURSOR_SIZE").as_deref(), Some("24"));
        assert!(value(&env, "XCURSOR_PATH").unwrap().contains("/home/test/.icons"));
    }
}
