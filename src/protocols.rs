//! Smaller Wayland protocols: decoration policy, fractional scale, pointer constraints,
//! tearing control and the sync blockers that keep GPU buffers from being used too early.
use std::sync::atomic::{AtomicBool, Ordering};

use calloop::Interest;
use smithay::{
    backend::allocator::dmabuf::Dmabuf,
    delegate_content_type, delegate_cursor_shape, delegate_drm_syncobj, delegate_fractional_scale,
    delegate_pointer_constraints, delegate_presentation, delegate_primary_selection,
    delegate_relative_pointer, delegate_viewporter, delegate_xdg_decoration,
    input::pointer::PointerHandle,
    reexports::{
        wayland_protocols::{
            wp::tearing_control::v1::server::{
                wp_tearing_control_manager_v1::{self, WpTearingControlManagerV1},
                wp_tearing_control_v1::{self, PresentationHint, WpTearingControlV1},
            },
            xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Point},
    wayland::{
        compositor::{
            BufferAssignment, CompositorHandler, SurfaceAttributes, add_blocker, add_pre_commit_hook, get_parent, with_states,
        },
        dmabuf::get_dmabuf,
        drm_syncobj::{DrmSyncobjCachedState, DrmSyncobjHandler, DrmSyncobjState},
        fractional_scale::{FractionalScaleHandler, with_fractional_scale},
        pointer_constraints::{PointerConstraintsHandler, with_pointer_constraint},
        selection::primary_selection::{PrimarySelectionHandler, PrimarySelectionState},
        tablet_manager::TabletSeatHandler,
        shell::xdg::{ToplevelSurface, decoration::XdgDecorationHandler},
    },
};

use crate::State;

// --- Window decorations -------------------------------------------------------------------

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        self.force_server_side(&toplevel);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: Mode) {
        self.force_server_side(&toplevel);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.force_server_side(&toplevel);
    }
}

impl State {
    /// MyWM draws only focus borders, so clients should not draw their own title bars.
    fn force_server_side(&mut self, toplevel: &ToplevelSurface) {
        toplevel.with_pending_state(|state| state.decoration_mode = Some(Mode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
}

// --- Fractional scale ---------------------------------------------------------------------

impl FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        let scale = self.scale_for_surface(&root);
        with_states(&surface, |states| with_fractional_scale(states, |fs| fs.set_preferred_scale(scale)));
    }
}

impl State {
    /// Scale of the output a window's surface tree lives on (the focused monitor's for unknown ones).
    pub fn scale_for_surface(&self, root: &WlSurface) -> f64 {
        let monitor = self
            .desktop
            .by_surface(root)
            .and_then(|m| self.desktop.desk.locate(&m.id))
            .map_or(self.desktop.focused_monitor, |(monitor, _)| monitor);
        self.outputs
            .get(monitor)
            .or_else(|| self.outputs.first())
            .map_or(1.0, |entry| entry.output.current_scale().fractional_scale())
    }
}

// --- Selection ----------------------------------------------------------------------------

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

// --- Pointer constraints ------------------------------------------------------------------

impl PointerConstraintsHandler for State {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        // A client that already has pointer focus may constrain it right away.
        if pointer.current_focus().as_ref() == Some(surface) {
            self.activate_constraint(surface);
        }
    }

    fn cursor_position_hint(&mut self, _surface: &WlSurface, _pointer: &PointerHandle<Self>, _location: Point<f64, Logical>) {}
}

impl State {
    fn activate_constraint(&mut self, surface: &WlSurface) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        with_pointer_constraint(surface, &pointer, |constraint| {
            if let Some(constraint) = constraint
                && !constraint.is_active()
            {
                constraint.activate();
            }
        });
    }

    /// Called when the surface under the pointer changes: constraints follow pointer focus.
    pub fn pointer_focus_changed(&mut self, old: Option<&WlSurface>, new: Option<&WlSurface>) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        if let Some(old) = old {
            with_pointer_constraint(old, &pointer, |constraint| {
                if let Some(constraint) = constraint
                    && constraint.is_active()
                {
                    constraint.deactivate();
                }
            });
        }
        if let Some(new) = new {
            self.activate_constraint(new);
        }
    }
}

// --- Tearing control ----------------------------------------------------------------------

/// Per-surface tearing preference, stored in the surface's data map.
#[derive(Default)]
struct SurfaceTearing {
    /// Whether the client asked for immediate (tearing) presentation.
    wanted: AtomicBool,
    /// Whether a `wp_tearing_control_v1` object exists for the surface.
    constructed: AtomicBool,
}

/// The client allows tearing for this surface (`wp_tearing_control_v1.set_presentation_hint(async)`).
pub fn surface_allows_tearing(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        states.data_map.get::<SurfaceTearing>().is_some_and(|t| t.wanted.load(Ordering::Relaxed))
    })
}

pub struct TearingSurface(smithay::reexports::wayland_server::Weak<WlSurface>);

impl State {
    pub fn create_tearing_control_global(display: &DisplayHandle) -> smithay::reexports::wayland_server::backend::GlobalId {
        display.create_global::<State, WpTearingControlManagerV1, ()>(1, ())
    }
}

impl GlobalDispatch<WpTearingControlManagerV1, (), State> for State {
    fn bind(
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<WpTearingControlManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<WpTearingControlManagerV1, (), State> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        manager: &WpTearingControlManagerV1,
        request: wp_tearing_control_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        if let wp_tearing_control_manager_v1::Request::GetTearingControl { id, surface } = request {
            let taken = with_states(&surface, |states| {
                let data = states.data_map.get_or_insert_threadsafe(SurfaceTearing::default);
                data.constructed.swap(true, Ordering::Relaxed)
            });
            if taken {
                manager.post_error(
                    wp_tearing_control_manager_v1::Error::TearingControlExists,
                    "the surface already has a tearing control object",
                );
                return;
            }
            data_init.init(id, TearingSurface(surface.downgrade()));
        }
    }
}

impl Dispatch<WpTearingControlV1, TearingSurface, State> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        _resource: &WpTearingControlV1,
        request: wp_tearing_control_v1::Request,
        data: &TearingSurface,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        if let wp_tearing_control_v1::Request::SetPresentationHint { hint } = request
            && let Ok(surface) = data.0.upgrade()
        {
            let wanted = matches!(hint, WEnum::Value(PresentationHint::Async));
            with_states(&surface, |states| {
                states
                    .data_map
                    .get_or_insert_threadsafe(SurfaceTearing::default)
                    .wanted
                    .store(wanted, Ordering::Relaxed);
            });
        }
    }

    fn destroyed(_state: &mut State, _client: smithay::reexports::wayland_server::backend::ClientId, _resource: &WpTearingControlV1, data: &TearingSurface) {
        // Destroying the object returns the surface to normal presentation.
        if let Ok(surface) = data.0.upgrade() {
            with_states(&surface, |states| {
                let tearing = states.data_map.get_or_insert_threadsafe(SurfaceTearing::default);
                tearing.wanted.store(false, Ordering::Relaxed);
                tearing.constructed.store(false, Ordering::Relaxed);
            });
        }
    }
}

// --- Synchronization ----------------------------------------------------------------------

impl DrmSyncobjHandler for State {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.syncobj_state.as_mut()
    }
}

impl State {
    /// Make a surface's commits wait until the GPU is done writing the attached buffer:
    /// on the explicit acquire point if the client uses `linux-drm-syncobj`, else on the
    /// dmabuf's implicit fences. Without this a client's half-drawn frame can reach the screen.
    pub fn install_sync_blockers(&mut self, surface: &WlSurface) {
        add_pre_commit_hook::<State, _>(surface, |state, _display, surface| {
            let mut acquire_point = None;
            let dmabuf: Option<Dmabuf> = with_states(surface, |states| {
                acquire_point.clone_from(&states.cached_state.get::<DrmSyncobjCachedState>().pending().acquire_point);
                states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .pending()
                    .buffer
                    .as_ref()
                    .and_then(|assignment| match assignment {
                        BufferAssignment::NewBuffer(buffer) => get_dmabuf(buffer).cloned().ok(),
                        _ => None,
                    })
            });
            let Some(dmabuf) = dmabuf else { return };
            let Some(client) = surface.client() else { return };
            let handle = state.loop_handle.clone();
            if let Some(point) = acquire_point
                && let Ok((blocker, source)) = point.generate_blocker()
            {
                let client = client.clone();
                let registered = handle.insert_source(source, move |_, _, state| {
                    let display = state.display_handle.clone();
                    state.client_compositor_state(&client).blocker_cleared(state, &display);
                    Ok(())
                });
                if registered.is_ok() {
                    add_blocker(surface, blocker);
                    return;
                }
            }
            if let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) {
                let registered = handle.insert_source(source, move |_, _, state| {
                    let display = state.display_handle.clone();
                    state.client_compositor_state(&client).blocker_cleared(state, &display);
                    Ok(())
                });
                if registered.is_ok() {
                    add_blocker(surface, blocker);
                }
            }
        });
    }
}

impl TabletSeatHandler for State {}

// --- Plain delegates ----------------------------------------------------------------------

delegate_viewporter!(State);
delegate_fractional_scale!(State);
delegate_cursor_shape!(State);
delegate_content_type!(State);
delegate_xdg_decoration!(State);
delegate_primary_selection!(State);
delegate_relative_pointer!(State);
delegate_pointer_constraints!(State);
delegate_presentation!(State);
delegate_drm_syncobj!(State);
