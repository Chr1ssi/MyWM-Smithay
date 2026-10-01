//! Layer shell (`wlr-layer-shell`): bars, launchers, wallpapers and notifications.
use std::time::Duration;

use mywm_layout::Rect;
use smithay::{
    delegate_layer_shell,
    backend::renderer::element::{RenderElementStates, default_primary_scanout_output_compare},
    desktop::{
        LayerSurface, PopupKind, WindowSurfaceType, layer_map_for_output,
        utils::{send_frames_surface_tree, surface_primary_scanout_output, update_surface_primary_scanout_output},
    },
    output::Output,
    reexports::wayland_server::protocol::{wl_output::WlOutput, wl_surface::WlSurface},
    utils::{Logical, Point},
    wayland::{
        compositor::with_states,
        shell::wlr_layer::{
            KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler,
            WlrLayerShellState,
        },
    },
};

use crate::State;

/// How often a surface nobody sees (covered by a fullscreen window, scrolled out of view) may draw.
/// Just under the fallback timer's second, so the timer always finds it due.
const HIDDEN_FRAME_THROTTLE: Duration = Duration::from_millis(995);
pub const HIDDEN_FRAME_TIMER: Duration = Duration::from_secs(1);

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(&mut self, surface: WlrLayerSurface, output: Option<WlOutput>, layer: Layer, namespace: String) {
        // Without a requested output, the focused monitor gets it.
        let output = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.outputs.get(self.desktop.focused_monitor).or(self.outputs.first()).map(|e| e.output.clone()));
        let Some(output) = output else {
            surface.send_close();
            return;
        };
        tracing::info!("layer surface {namespace:?} on {} ({:?})", output.name(), layer);
        let layer_surface = LayerSurface::new(surface, namespace);
        if let Err(error) = layer_map_for_output(&output).map_layer(&layer_surface) {
            tracing::warn!("cannot map layer surface {:?}: {error}", layer_surface.namespace());
        }
    }

    fn new_popup(&mut self, _parent: WlrLayerSurface, popup: smithay::wayland::shell::xdg::PopupSurface) {
        popup.with_pending_state(|state| state.geometry = state.positioner.get_geometry());
        let _ = self.popups.track_popup(PopupKind::Xdg(popup));
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        for entry in &self.outputs {
            let mut map = layer_map_for_output(&entry.output);
            let layer = map.layers().find(|l| l.layer_surface() == &surface).cloned();
            if let Some(layer) = layer {
                tracing::info!("layer surface {:?} closed", layer.namespace());
                map.unmap_layer(&layer);
            }
        }
        if self.layer_focus.as_ref() == Some(surface.wl_surface()) {
            self.layer_focus = None;
        }
        self.layers_changed(None);
        self.refresh();
    }
}

delegate_layer_shell!(State);

impl State {
    /// Handle a commit of a layer surface. Returns whether `surface` is one.
    pub fn layer_commit(&mut self, surface: &WlSurface) -> bool {
        let Some(output) = self
            .outputs
            .iter()
            .map(|e| &e.output)
            .find(|o| layer_map_for_output(o).layer_for_surface(surface, WindowSurfaceType::TOPLEVEL).is_some())
            .cloned()
        else {
            return false;
        };
        let initial_configure_sent = with_states(surface, |states| {
            states.data_map.get::<LayerSurfaceData>().is_none_or(|d| d.lock().unwrap().initial_configure_sent)
        });
        {
            let mut map = layer_map_for_output(&output);
            // Arrange first so the initial configure respects the size the client asked for.
            map.arrange();
            if !initial_configure_sent && let Some(layer) = map.layers().find(|l| l.wl_surface() == surface) {
                layer.layer_surface().send_configure();
            }
        }
        if !matches!(
            layer_map_for_output(&output).layer_for_surface(surface, WindowSurfaceType::TOPLEVEL).map(|l| l.layer()),
            Some(Layer::Top | Layer::Overlay)
        ) {
            self.blur_dirty(&output);
        }
        self.layers_changed(Some(&output));
        // A panel that now asks for the keyboard (a launcher, the wallpaper picker) gets it at once.
        self.sync_focus();
        true
    }

    /// The panels' reserved zones may have changed: re-tile if the usable area moved.
    /// `output`: the only one whose picture changes if the layout does not.
    pub fn layers_changed(&mut self, output: Option<&Output>) {
        let before: Vec<Rect> = self.desktop.desk.monitors.iter().map(|m| m.usable).collect();
        self.sync_monitor_areas();
        let after: Vec<Rect> = self.desktop.desk.monitors.iter().map(|m| m.usable).collect();
        if before != after {
            self.refresh();
        } else if let Some(output) = output {
            self.queue_redraw_output(output);
        } else {
            self.queue_redraw_all();
        }
    }

    /// The layer surface under the pointer among `layers` (front to back), with the
    /// global position of the hit surface's origin.
    pub fn layer_surface_at(&self, layers: &[Layer]) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output = self.space.output_under(self.pointer_location).next()?;
        let geo = self.space.output_geometry(output)?;
        let relative = self.pointer_location - geo.loc.to_f64();
        let map = layer_map_for_output(output);
        layers.iter().find_map(|layer| {
            let hit = map.layer_under(*layer, relative)?;
            let layer_geo = map.layer_geometry(hit)?;
            hit.surface_under(relative - layer_geo.loc.to_f64(), WindowSurfaceType::ALL)
                .map(|(surface, origin)| (surface, (origin + layer_geo.loc + geo.loc).to_f64()))
        })
    }

    /// The layer surface (of any of its popups or subsurfaces) `surface` belongs to, if any.
    pub fn layer_of_surface(&self, surface: &WlSurface) -> Option<LayerSurface> {
        self.outputs.iter().find_map(|e| {
            layer_map_for_output(&e.output).layer_for_surface(surface, WindowSurfaceType::ALL).cloned()
        })
    }

    /// The layer surface that holds keyboard focus: one that demands it exclusively on
    /// top of the windows (a launcher, a lock prompt), else the last one the user clicked.
    pub fn keyboard_layer(&self) -> Option<WlSurface> {
        for entry in &self.outputs {
            let map = layer_map_for_output(&entry.output);
            let exclusive = map
                .layers_on(Layer::Overlay)
                .chain(map.layers_on(Layer::Top))
                .find(|l| l.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive);
            if let Some(layer) = exclusive {
                return Some(layer.wl_surface().clone());
            }
        }
        self.layer_focus.clone().filter(|surface| self.layer_of_surface(surface).is_some())
    }

    /// The pointer was pressed on `surface` (or on a window if `None`).
    pub fn note_click(&mut self, surface: Option<&WlSurface>) {
        self.layer_focus = surface
            .and_then(|s| self.layer_of_surface(s))
            .filter(|layer| layer.can_receive_keyboard_focus())
            .map(|layer| layer.wl_surface().clone());
        self.refresh();
    }

    /// Tell the surfaces `output` just showed that they may draw their next frame.
    ///
    /// Only surfaces the last frame of `output` actually showed (see `update_primary_outputs`) get one with
    /// every frame; hidden ones get one per `HIDDEN_FRAME_THROTTLE`, so a browser or a video behind a game
    /// or scrolled out of view does not keep drawing at the full refresh rate. A window that is being
    /// captured draws for the capture even where nobody sees it.
    pub fn send_frames(&self, output: &Output) {
        let elapsed = self.start_time.elapsed();
        for window in self.space.elements() {
            if self.window_captured(window) {
                if self.capture_output(window).as_ref() == Some(output) {
                    window.send_frame(output, elapsed, None, |_, _| Some(output.clone()));
                }
                continue;
            }
            window.send_frame(output, elapsed, Some(HIDDEN_FRAME_THROTTLE), surface_primary_scanout_output);
        }
        for layer in layer_map_for_output(output).layers() {
            layer.send_frame(output, elapsed, Some(HIDDEN_FRAME_THROTTLE), surface_primary_scanout_output);
        }
        self.send_lock_frame(output, elapsed);
    }

    /// Like `send_frames`, for an output that is never presented (the nested backend's virtual outputs):
    /// without a frame there is nothing to tell what is visible, so everything on it may draw.
    pub fn send_frames_unpresented(&self, output: &Output) {
        let elapsed = self.start_time.elapsed();
        for window in self.space.elements_for_output(output) {
            window.send_frame(output, elapsed, None, |_, _| Some(output.clone()));
        }
        for layer in layer_map_for_output(output).layers() {
            layer.send_frame(output, elapsed, None, |_, _| Some(output.clone()));
        }
        self.send_lock_frame(output, elapsed);
    }

    /// The locker draws its input feedback (swaylock's ring) only when its frame callback comes.
    fn send_lock_frame(&self, output: &Output, elapsed: Duration) {
        if let Some(lock) = self.lock_surface_for(output) {
            send_frames_surface_tree(lock.wl_surface(), output, elapsed, None, |_, _| Some(output.clone()));
        }
    }

    /// The fallback timer: hidden surfaces whose throttled frame is due get it even while no output redraws.
    pub fn send_overdue_frames(&self) {
        let elapsed = self.start_time.elapsed();
        if let Some(entry) = self.outputs.first() {
            for window in self.space.elements() {
                window.send_frame(&entry.output, elapsed, Some(HIDDEN_FRAME_THROTTLE), |_, _| None);
            }
        }
        for entry in &self.outputs {
            for layer in layer_map_for_output(&entry.output).layers() {
                layer.send_frame(&entry.output, elapsed, Some(HIDDEN_FRAME_THROTTLE), |_, _| None);
            }
        }
    }

    /// Remember for every surface whether the frame just rendered for `output` showed it (its primary scanout
    /// output). Covered or cropped-away surfaces lose `output`, which throttles their frame callbacks.
    pub fn update_primary_outputs(&self, output: &Output, states: &RenderElementStates) {
        let update = |surface: &WlSurface, data: &smithay::wayland::compositor::SurfaceData| {
            update_surface_primary_scanout_output(surface, output, data, states, default_primary_scanout_output_compare);
        };
        for window in self.space.elements() {
            window.with_surfaces(update);
        }
        for layer in layer_map_for_output(output).layers() {
            layer.with_surfaces(update);
        }
    }
}
