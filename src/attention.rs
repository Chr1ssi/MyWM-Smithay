//! Keyboard shortcut inhibiting (games, VMs) and focus requests (`xdg-activation`).
use smithay::{
    delegate_keyboard_shortcuts_inhibit, delegate_xdg_activation,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::{
        compositor::get_parent,
        keyboard_shortcuts_inhibit::{
            KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
            KeyboardShortcutsInhibitorSeat,
        },
        xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData},
    },
};

use crate::State;

/// A focus request from a user action is honoured this long.
const TOKEN_LIFETIME: std::time::Duration = std::time::Duration::from_secs(10);

impl KeyboardShortcutsInhibitHandler for State {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.shortcuts_inhibit
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        // Asked for by the client, taken back with `release_shortcuts`.
        inhibitor.activate();
    }
}

delegate_keyboard_shortcuts_inhibit!(State);

impl State {
    /// The focused client asked for the keyboard shortcuts and has them.
    pub fn shortcuts_inhibited(&self) -> bool {
        let Some(surface) = self.seat.get_keyboard().and_then(|k| k.current_focus()) else { return false };
        self.seat.keyboard_shortcuts_inhibitor_for_surface(&surface).is_some_and(|i| i.is_active())
    }

    /// `release_shortcuts`: the focused client loses its hold on the shortcuts.
    pub fn release_shortcuts(&mut self) {
        let Some(surface) = self.seat.get_keyboard().and_then(|k| k.current_focus()) else { return };
        if let Some(inhibitor) = self.seat.keyboard_shortcuts_inhibitor_for_surface(&surface) {
            inhibitor.inactivate();
            tracing::info!("keyboard shortcuts released from the focused client");
        }
    }
}

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation_state
    }

    fn token_created(&mut self, _token: XdgActivationToken, _data: XdgActivationTokenData) -> bool {
        true
    }

    fn request_activation(&mut self, token: XdgActivationToken, data: XdgActivationTokenData, surface: WlSurface) {
        self.activation_state.remove_token(&token);
        let mut root = surface;
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        let Some((id, visible)) = self.desktop.by_surface(&root).map(|m| (m.id, m.frame.is_some())) else { return };
        if self.desktop.focused() == Some(id) {
            return;
        }
        // A request caused by a user action in the last moments, for a window that is on
        // screen, takes the focus. Anything else only makes the border stand out.
        if visible && data.serial.is_some() && data.timestamp.elapsed() < TOKEN_LIFETIME {
            self.focus_window(id);
        } else if let Some(m) = self.desktop.get_mut(id) {
            m.urgent = true;
            self.refresh();
        }
    }
}

delegate_xdg_activation!(State);
