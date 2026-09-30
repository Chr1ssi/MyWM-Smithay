//! Window management model: windows, per-monitor workspaces and their mapping onto the smithay `Space`.
use mywm_layout::{
    Appearance, Desk, DragKind, Edges, GAMING, Placement, Rect, WindowInfo, Workspace,
    arrange, place_floating,
};
use smithay::{
    backend::renderer::{
        element::Kind,
        gles::element::PixelShaderElement,
    },
    backend::renderer::element::{
        solid::{SolidColorBuffer, SolidColorRenderElement},
        utils::CropRenderElement,
    },
    desktop::Window,
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Physical, Point, Rectangle},
    xwayland::{X11Surface, xwm::WmWindowType},
    wayland::{
        compositor::with_states, fractional_scale::with_fractional_scale, shell::xdg::XdgToplevelSurfaceData,
    },
};

use crate::State;
use smithay::wayland::seat::WaylandFocus;

pub type WindowId = u64;

pub type RingKey = (Rectangle<i32, Logical>, i32, i32, [f32; 4]);

pub struct Managed {
    pub id: WindowId,
    pub window: Window,
    pub floating: bool,
    pub fullscreen: bool,
    pub tiled_width: Option<i32>,
    pub floating_rect: Option<Rect>,
    pub parent: Option<WindowId>,
    pub app_id: Option<String>,
    /// The window in `ext-foreign-toplevel-list`, which capture sources refer to.
    pub foreign: Option<smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle>,
    /// Commits of the main surface, to tell capture sessions when the window changed.
    pub commits: u64,
    /// Border color for the current focus state.
    pub border_color: [f32; 4],
    /// The rounded border ring and what it was built for (frame, radius, width, color).
    /// Sliding to a new place: where it comes from, relative to its place, and since when.
    pub slide: Option<(Point<i32, Logical>, std::time::Instant)>,
    /// When the window (re)appeared, for fading in.
    pub shown: Option<std::time::Instant>,
    pub last_monitor: Option<usize>,
    pub shadow: Option<(smithay::backend::renderer::gles::element::PixelShaderElement, RingKey)>,
    pub ring: Option<(smithay::backend::renderer::gles::element::PixelShaderElement, RingKey)>,
    /// Floating state to restore when leaving the scratchpad.
    pub scratchpad_floating: Option<bool>,
    /// Rules and workspace are applied at the first commit, once the app id is known.
    pub placed: bool,
    /// Frame (content plus border) in global coordinates, while placed.
    pub frame: Option<(Rect, i32)>,
    /// Top, bottom, left, right border strips. Buffers persist so damage tracking stays exact.
    borders: [SolidColorBuffer; 4],
}

fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

impl Managed {
    /// How far the window is still from its place (logical pixels), `ms` being the animation length.
    pub fn slide_offset(&self, ms: u32) -> Point<i32, Logical> {
        match self.slide {
            Some((from, start)) if ms > 0 => {
                let left = 1.0 - ease_out(start.elapsed().as_secs_f64() * 1000.0 / f64::from(ms));
                ((f64::from(from.x) * left).round() as i32, (f64::from(from.y) * left).round() as i32).into()
            }
            _ => (0, 0).into(),
        }
    }

    /// Opacity factor while fading in (1.0 when done).
    pub fn fade(&self, ms: u32) -> f32 {
        match self.shown {
            Some(start) if ms > 0 => ease_out(start.elapsed().as_secs_f64() * 1000.0 / f64::from(ms)) as f32,
            _ => 1.0,
        }
    }

    /// An animation is running (or finished less than a frame ago and needs one last redraw).
    pub fn animating(&self, ms: u32) -> bool {
        let running = |start: std::time::Instant| start.elapsed().as_millis() <= u128::from(ms) + 40;
        ms > 0 && (self.slide.is_some_and(|(_, s)| running(s)) || self.shown.is_some_and(running))
    }

    /// The window's main surface (an X11 window has none until Xwayland associates one).
    pub fn surface(&self) -> Option<WlSurface> {
        self.window.wl_surface().map(|s| s.into_owned())
    }

    pub fn has_surface(&self, surface: &WlSurface) -> bool {
        self.window.wl_surface().is_some_and(|s| &*s == surface)
    }

    /// Ask the client to close the window.
    pub fn close(&self) {
        if let Some(toplevel) = self.window.toplevel() {
            toplevel.send_close();
        } else if let Some(x11) = self.window.x11_surface() {
            let _ = x11.close();
        }
    }

    pub(crate) fn info(&self) -> WindowInfo<WindowId> {
        WindowInfo {
            id: self.id,
            floating: self.floating,
            fullscreen: self.fullscreen,
            tiled_width: self.tiled_width,
            floating_rect: self.floating_rect,
            parent: self.parent,
        }
    }
}

/// An interactive pointer drag started with modifier + button.
pub struct Drag {
    pub id: WindowId,
    pub kind: DragKind,
    pub start: Point<f64, Logical>,
    pub start_rect: Rect,
    pub start_width: i32,
}

pub struct Desktop {
    pub windows: Vec<Managed>,
    pub desk: Desk<WindowId>,
    /// The monitor keyboard commands act on; follows the pointer.
    pub focused_monitor: usize,
    pub appearance: Appearance,
    pub drag: Option<Drag>,
    /// Global floating stack shown above all workspaces, on the monitor under the pointer.
    pub scratchpad: Workspace<WindowId>,
    pub scratchpad_visible: bool,
    /// Whether keyboard focus is on the scratchpad rather than the workspace.
    pub focus_scratch: bool,
    next_id: WindowId,
}

impl Desktop {
    pub fn new(workspace_outputs: Vec<String>, gaming_output: Option<String>, appearance: Appearance) -> Self {
        Self {
            windows: Vec::new(),
            desk: Desk::new(workspace_outputs, gaming_output),
            focused_monitor: 0,
            appearance,
            drag: None,
            scratchpad: Workspace::default(),
            scratchpad_visible: false,
            focus_scratch: false,
            next_id: 1,
        }
    }

    pub fn by_x11(&self, window_id: u32) -> Option<&Managed> {
        self.windows.iter().find(|w| w.window.x11_surface().is_some_and(|x| x.window_id() == window_id))
    }

    pub fn get(&self, id: WindowId) -> Option<&Managed> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut Managed> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    pub fn by_surface(&self, surface: &WlSurface) -> Option<&Managed> {
        self.windows.iter().find(|w| w.has_surface(surface))
    }

    /// The workspace shown on the focused monitor.
    pub fn current(&self) -> Option<&Workspace<WindowId>> {
        self.desk.monitors.get(self.focused_monitor).map(|m| m.workspaces.current())
    }

    pub fn focused(&self) -> Option<WindowId> {
        if self.scratchpad_visible && self.focus_scratch && self.scratchpad.focused.is_some() {
            self.scratchpad.focused
        } else {
            self.current().and_then(|w| w.focused)
        }
    }

    pub fn in_scratchpad(&self, id: WindowId) -> bool {
        self.scratchpad.windows.contains(&id)
    }

    /// Area windows may use on the monitor a window lives on (below bars and panels).
    pub fn monitor_area_of(&self, id: WindowId) -> Option<Rect> {
        let monitor = self.desk.locate(&id).map(|(m, _)| m).unwrap_or(self.focused_monitor);
        self.desk.monitors.get(monitor).map(|m| m.usable)
    }
}

impl State {
    /// Register a new toplevel. It joins a workspace at its first commit (see `place_pending`).
    pub fn add_window(&mut self, window: Window) {
        let toplevel = window.toplevel().expect("wayland window").clone();
        self.push_window(window);
        // The client waits for a configure before it may attach a buffer.
        toplevel.send_configure();
    }

    /// Register a mapped X11 window. Its class is known already, so it is placed right away.
    pub fn add_x11_window(&mut self, x11: X11Surface) {
        let id = self.push_window(Window::new_x11_window(x11));
        self.place(id);
        self.refresh();
    }

    fn push_window(&mut self, window: Window) -> WindowId {
        let d = &mut self.desktop;
        let id = d.next_id;
        d.next_id += 1;
        d.windows.push(Managed {
            id,
            window,
            floating: false,
            fullscreen: false,
            tiled_width: None,
            floating_rect: None,
            parent: None,
            app_id: None,
            foreign: None,
            commits: 0,
            border_color: [0.0; 4],
            ring: None,
            slide: None,
            shown: None,
            last_monitor: None,
            shadow: None,
            scratchpad_floating: None,
            placed: false,
            frame: None,
            borders: Default::default(),
        });
        self.announce_foreign_toplevel(id);
        id
    }

    /// Apply window rules and pick the workspace once the app id is known.
    pub fn place_pending(&mut self, surface: &WlSurface) {
        let Some(id) = self.desktop.by_surface(surface).filter(|m| !m.placed).map(|m| m.id) else {
            return;
        };
        self.place(id);
        self.refresh();
    }

    /// Windows that arrived while no monitor existed get placed once one appears.
    pub fn place_all_pending(&mut self) {
        let pending: Vec<_> = self.desktop.windows.iter().filter(|m| !m.placed).map(|m| m.id).collect();
        for id in pending {
            self.place(id);
        }
    }

    fn place(&mut self, id: WindowId) {
        let Some(m) = self.desktop.get(id) else { return };
        if m.placed || self.desktop.desk.monitors.is_empty() {
            return;
        }
        let (app_id, parent, x11_float) = match m.window.x11_surface() {
            Some(x11) => {
                // Steam and friends: dialogs and popups float above their owner, fixed-size windows too.
                let parent = x11.is_transient_for().and_then(|w| self.desktop.by_x11(w).map(|p| p.id));
                let class = x11.class();
                let floats = x11.is_popup()
                    || x11.is_transient_for().is_some()
                    || matches!(
                        x11.window_type(),
                        Some(
                            WmWindowType::Dialog
                                | WmWindowType::Utility
                                | WmWindowType::Splash
                                | WmWindowType::Toolbar
                                | WmWindowType::Notification
                        )
                    )
                    || x11.min_size().is_some_and(|min| x11.max_size() == Some(min));
                ((!class.is_empty()).then_some(class), parent, floats)
            }
            None => {
                let app_id = m.surface().and_then(|surface| {
                    with_states(&surface, |states| {
                        states
                            .data_map
                            .get::<XdgToplevelSurfaceData>()
                            .and_then(|data| data.lock().unwrap().app_id.clone())
                    })
                });
                let parent = m
                    .window
                    .toplevel()
                    .and_then(|t| t.parent())
                    .and_then(|surface| self.desktop.by_surface(&surface).map(|p| p.id));
                (app_id, parent, false)
            }
        };
        // A child may be announced before its parent; place the parent first.
        if let Some(parent) = parent {
            self.place(parent);
        }
        tracing::info!("new window: app_id={app_id:?} dialog={}", parent.is_some() || x11_float);

        let placement = mywm_config::resolve(
            &self.config.rules,
            app_id.as_deref(),
            parent.is_some() || x11_float,
            self.config.float_dialogs,
        );
        let game = self.is_game(app_id.as_deref(), parent);
        let d = &mut self.desktop;
        // Dialogs join their parent's workspace, everything else the focused monitor's shown one.
        let focused = d.focused_monitor.min(d.desk.monitors.len() - 1);
        let (mut monitor, active) = parent
            .and_then(|p| d.desk.locate(&p))
            .unwrap_or((focused, d.desk.monitors[focused].workspaces.active));
        // Rules only apply while their workspace exists.
        let mut number = placement
            .workspace
            .filter(|n| d.desk.owner(*n).is_some())
            .unwrap_or(active);
        if let Some(owner) = d.desk.owner(number) {
            monitor = owner;
        }
        if game && let Some(gaming) = d.desk.ensure_gaming() {
            monitor = gaming;
            number = GAMING;
            d.desk.monitors[monitor].workspaces.select(GAMING);
        } else if number == GAMING {
            // Only games start on the gaming workspace; show the monitor's own instead.
            number = d.desk.monitors[monitor].workspaces.home;
            d.desk.monitors[monitor].workspaces.select(number);
        }
        let workspaces = &mut d.desk.monitors[monitor].workspaces;
        if number == workspaces.active {
            workspaces.add(id);
        } else {
            workspaces.add_to(number, id);
        }
        d.focus_scratch = false;
        if number == d.desk.monitors[monitor].workspaces.active {
            d.focused_monitor = monitor;
        }
        if let Some(m) = d.get_mut(id) {
            m.placed = true;
            m.floating = placement.floating || x11_float;
            m.parent = parent;
            m.app_id = app_id;
        }
    }

    /// The focused window of `monitor`'s shown workspace if it is a fullscreen game.
    pub fn fullscreen_game_on(&self, monitor: usize) -> Option<&Managed> {
        let focused = self.desktop.desk.monitors.get(monitor)?.workspaces.current().focused?;
        let m = self.desktop.get(focused)?;
        (m.fullscreen && m.placed && self.is_game(m.app_id.as_deref(), m.parent)).then_some(m)
    }

    /// Games (and their dialogs) are recognized by app id prefix.
    pub(crate) fn is_game(&self, app_id: Option<&str>, parent: Option<WindowId>) -> bool {
        let prefixes = &self.config.game_app_id_prefixes;
        let matches = |id: Option<&str>| id.is_some_and(|id| prefixes.iter().any(|p| id.starts_with(p)));
        if matches(app_id) {
            return true;
        }
        let mut current = parent;
        for _ in 0..=self.desktop.windows.len() {
            let Some(m) = current.and_then(|id| self.desktop.get(id)) else { return false };
            if matches(m.app_id.as_deref()) {
                return true;
            }
            current = m.parent;
        }
        false
    }

    pub fn remove_window(&mut self, surface: &WlSurface) {
        let Some(id) = self.desktop.by_surface(surface).map(|w| w.id) else { return };
        self.remove_managed(id);
    }

    pub fn remove_x11_window(&mut self, window_id: u32) {
        let Some(id) = self.desktop.by_x11(window_id).map(|w| w.id) else { return };
        self.remove_managed(id);
    }

    fn remove_managed(&mut self, id: WindowId) {
        if let Some(managed) = self.desktop.get(id) {
            self.space.unmap_elem(&managed.window);
        }
        self.desktop.desk.remove_window(&id);
        self.desktop.scratchpad.remove(&id);
        if self.desktop.scratchpad.windows.is_empty() {
            self.desktop.scratchpad_visible = false;
        }
        if let Some(handle) = self.desktop.get(id).and_then(|m| m.foreign.clone()) {
            self.image_capture_window_closed(&handle);
            handle.send_closed();
            self.foreign_toplevels.remove_toplevel(&handle);
        }
        self.desktop.windows.retain(|w| w.id != id);
        // Dialogs of a closed window lose their parent but stay where they are.
        for w in &mut self.desktop.windows {
            if w.parent == Some(id) {
                w.parent = None;
            }
        }
        if self.desktop.drag.as_ref().is_some_and(|d| d.id == id) {
            self.desktop.drag = None;
        }
        self.desktop.desk.prune();
        self.refresh();
    }

    /// Recompute the layout and push it to the clients, then sync keyboard focus.
    pub fn refresh(&mut self) {
        self.sync_monitor_areas();
        self.apply_layout();
        self.sync_focus();
        self.update_fractional_scales();
        self.ipc_dirty = true;
        self.queue_redraw_all();
    }

    fn apply_layout(&mut self) {
        let scratch_monitor = self.scratchpad_monitor();
        let anim_ms = self.config.effects.animation_ms;
        let monitors: Vec<(WindowId, Option<usize>)> =
            self.desktop.windows.iter().map(|m| (m.id, self.monitor_of_window(m))).collect();
        let d = &mut self.desktop;
        if d.desk.monitors.is_empty() {
            return;
        }
        let infos: Vec<_> = d.windows.iter().filter(|m| m.placed).map(Managed::info).collect();
        let mut placements: Vec<Placement<WindowId>> = Vec::new();
        for monitor in &mut d.desk.monitors {
            let (usable, area) = (monitor.usable, monitor.area);
            placements.extend(arrange(monitor.workspaces.current_mut(), &infos, usable, area, &d.appearance));
        }
        if d.scratchpad_visible
            && let Some(monitor) = d.desk.monitors.get(scratch_monitor)
        {
            placements.extend(
                d.scratchpad
                    .windows
                    .iter()
                    .filter_map(|id| infos.iter().find(|i| i.id == *id))
                    .map(|info| place_floating(info, monitor.usable, &d.appearance)),
            );
        }

        let visible: Vec<_> = placements.iter().map(|p| p.id).collect();
        for m in &mut d.windows {
            if !visible.contains(&m.id) {
                self.space.unmap_elem(&m.window);
                m.frame = None;
            }
        }
        let active = d.appearance.active_border.0;
        let inactive = d.appearance.inactive_border.0;
        let focused = d.focused();
        for p in placements {
            let Some(m) = d.windows.iter_mut().find(|w| w.id == p.id) else { continue };
            if let Some(rect) = p.floating_rect {
                m.floating_rect = Some(rect);
            }
            if let Some(toplevel) = m.window.toplevel() {
                toplevel.with_pending_state(|s| {
                    s.size = Some((p.content.width, p.content.height).into());
                    if p.fullscreen {
                        s.states.set(xdg_toplevel::State::Fullscreen);
                    } else {
                        s.states.unset(xdg_toplevel::State::Fullscreen);
                    }
                });
                toplevel.send_pending_configure();
            } else if let Some(x11) = m.window.x11_surface() {
                // X11 windows know their position on the (global) screen.
                let target = Rectangle::new(
                    (p.content.x, p.content.y).into(),
                    (p.content.width, p.content.height).into(),
                );
                if x11.geometry() != target {
                    let _ = x11.configure(target);
                }
                if x11.is_fullscreen() != p.fullscreen {
                    let _ = x11.set_fullscreen(p.fullscreen);
                }
            }
            // Mapping in paint order keeps floating windows above tiled ones.
            self.space
                .map_element(m.window.clone(), (p.content.x, p.content.y), false);
            let old_place = m.frame.map(|(f, _)| (f.x, f.y));
            let monitor = monitors.iter().find(|(id, _)| *id == m.id).and_then(|(_, monitor)| *monitor);
            m.frame = Some((
                Rect {
                    x: p.content.x - p.border,
                    y: p.content.y - p.border,
                    width: p.content.width + 2 * p.border,
                    height: p.content.height + 2 * p.border,
                },
                p.border,
            ));
            // Animations: slide to the new place on the same monitor, fade in when (re)appearing.
            let new_place = (p.content.x - p.border, p.content.y - p.border);
            if anim_ms == 0 || p.fullscreen {
                m.slide = None;
                m.shown = None;
            } else {
                match old_place {
                    Some(old) if old != new_place && m.last_monitor == monitor => {
                        let current = m.slide_offset(anim_ms);
                        m.slide = Some((
                            (old.0 - new_place.0 + current.x, old.1 - new_place.1 + current.y).into(),
                            std::time::Instant::now(),
                        ));
                    }
                    Some(_) => {}
                    None => m.shown = Some(std::time::Instant::now()),
                }
            }
            m.last_monitor = monitor;
            let color = if focused == Some(m.id) { active } else { inactive };
            m.border_color = color;
            if let Some((frame, b)) = m.frame.filter(|(_, b)| *b > 0) {
                let strips = [
                    (frame.width, b),
                    (frame.width, b),
                    (b, frame.height - 2 * b),
                    (b, frame.height - 2 * b),
                ];
                for (buffer, size) in m.borders.iter_mut().zip(strips) {
                    buffer.update(size, color);
                }
            }
        }
    }

    /// Tell each window the scale of the output it is on (`wp_fractional_scale_v1`).
    fn update_fractional_scales(&self) {
        for m in self.desktop.windows.iter().filter(|m| m.placed) {
            let monitor = self.desktop.desk.locate(&m.id).map_or_else(|| self.scratchpad_monitor(), |(i, _)| i);
            let scale = self.outputs.get(monitor).map_or(1.0, |e| e.output.current_scale().fractional_scale());
            m.window.with_surfaces(|_, states| {
                with_fractional_scale(states, |fs| fs.set_preferred_scale(scale));
            });
        }
    }

    fn sync_focus(&mut self) {
        if self.session_lock.is_active() {
            let surface = self.active_lock_surface().map(|l| l.wl_surface().clone());
            self.set_keyboard_focus(surface);
            return;
        }
        // A launcher or lock prompt holds the keyboard; windows look unfocused meanwhile.
        let layer = self.keyboard_layer();
        let focused = self.desktop.focused().filter(|_| layer.is_none());
        for m in &self.desktop.windows {
            let changed = m.window.set_activated(Some(m.id) == focused);
            if changed && let Some(toplevel) = m.window.toplevel() {
                toplevel.send_pending_configure();
            }
        }
        let surface = layer.clone().or_else(|| focused.and_then(|id| self.desktop.get(id)).and_then(|m| m.surface()));
        // X11 clients also need the X11 input focus; a wl_surface focus alone leaves Wine and
        // Proton games without keyboard input.
        let x11 = focused
            .filter(|_| layer.is_none())
            .and_then(|id| self.desktop.get(id))
            .and_then(|m| m.window.x11_surface().cloned());
        if x11 != self.x11_focus {
            if let Some(old) = self.x11_focus.take()
                && x11.is_none()
            {
                old.set_x11_input_focus(false);
            }
            if let Some(new) = &x11 {
                tracing::info!("X11 input focus: {} ({})", new.title(), new.class());
                new.set_x11_input_focus(true);
            }
            self.x11_focus = x11;
        }
        self.set_keyboard_focus(surface);
    }

    /// The monitor a window is shown on: the scratchpad's follows the pointer, others their workspace's.
    pub fn monitor_of_window(&self, m: &Managed) -> Option<usize> {
        if self.desktop.in_scratchpad(m.id) {
            self.desktop.scratchpad_visible.then(|| self.scratchpad_monitor())
        } else {
            self.desktop.desk.locate(&m.id).map(|(monitor, _)| monitor)
        }
    }

    /// Compile the effect shaders on first use; `false` if they are unavailable.
    pub fn ensure_effect_shaders(&mut self, renderer: &mut smithay::backend::renderer::gles::GlesRenderer) -> bool {
        if self.effect_shaders.is_none() && !self.effect_shaders_failed {
            match crate::effects::compile(renderer) {
                Ok(shaders) => self.effect_shaders = Some(shaders),
                Err(error) => {
                    tracing::warn!("effect shaders failed to compile, corners stay square: {error}");
                    self.effect_shaders_failed = true;
                }
            }
        }
        self.effect_shaders.is_some()
    }

    /// Borders of the windows on `output`, top-most first: four strips, or one rounded ring
    /// when `[effects] corner_radius` is set.
    pub fn border_elements(
        &mut self,
        renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
        output: &Output,
    ) -> Vec<crate::render::OutputElement> {
        use crate::render::OutputElement;
        let Some(geo) = self.space.output_geometry(output) else { return Vec::new() };
        let scale = output.current_scale().fractional_scale();
        let Some(this_monitor) = self.outputs.iter().position(|e| &e.output == output) else { return Vec::new() };
        let radius = self.config.effects.corner_radius;
        let anim_ms = self.config.effects.animation_ms;
        let rounded = radius > 0 && self.ensure_effect_shaders(renderer);
        let ring_shader = self.effect_shaders.as_ref().map(|s| s.ring.clone());
        let monitors: Vec<Option<usize>> = self.desktop.windows.iter().map(|m| self.monitor_of_window(m)).collect();
        // Nothing may show beyond the edge of the output it belongs to.
        let visible_area = Rectangle::<i32, Physical>::from_size(geo.size.to_physical_precise_round(scale));
        let mut elements = Vec::new();
        for (m, monitor) in self.desktop.windows.iter_mut().zip(monitors).rev() {
            if monitor != Some(this_monitor) {
                continue;
            }
            let Some((frame, b)) = m.frame.filter(|(_, b)| *b > 0) else { continue };
            let (slide, fade) = (m.slide_offset(anim_ms), m.fade(anim_ms));
            let frame = Rect { x: frame.x + slide.x, y: frame.y + slide.y, ..frame };
            let visible = frame.x < geo.loc.x + geo.size.w
                && frame.x + frame.width > geo.loc.x
                && frame.y < geo.loc.y + geo.size.h
                && frame.y + frame.height > geo.loc.y;
            if !visible {
                continue;
            }
            if let (true, false, Some(shader)) = (rounded, m.fullscreen, &ring_shader) {
                let area = Rectangle::<i32, Logical>::new(
                    (frame.x - geo.loc.x, frame.y - geo.loc.y).into(),
                    (frame.width, frame.height).into(),
                );
                let (border, radius) = ((f64::from(b) * scale) as f32, (f64::from(radius) * scale) as f32);
                let [r, g, bl, a] = m.border_color;
                let color = [r, g, bl, a * fade];
                let key = (area, (border * 64.0) as i32, (radius * 64.0) as i32, color);
                let uniforms = || crate::effects::ring_uniforms(color, border, radius);
                match &mut m.ring {
                    Some((_, old)) if *old == key => {}
                    Some((element, old)) => {
                        element.resize(area, None);
                        element.update_uniforms(crate::effects::ring_uniforms(key.3, border, radius));
                        *old = key;
                    }
                    None => {
                        let element = PixelShaderElement::new(shader.clone(), area, None, 1.0, uniforms(), Kind::Unspecified);
                        m.ring = Some((element, key));
                    }
                }
                if let Some((element, _)) = &m.ring
                    && let Some(cropped) = CropRenderElement::from_element(element.clone(), scale, visible_area)
                {
                    elements.push(OutputElement::from(cropped));
                }
                continue;
            }
            let origins = [
                (frame.x, frame.y),
                (frame.x, frame.y + frame.height - b),
                (frame.x, frame.y + b),
                (frame.x + frame.width - b, frame.y + b),
            ];
            for (buffer, (x, y)) in m.borders.iter().zip(origins) {
                let local = (
                    ((x - geo.loc.x) as f64 * scale).round() as i32,
                    ((y - geo.loc.y) as f64 * scale).round() as i32,
                );
                let element = SolidColorRenderElement::from_buffer(buffer, local, scale, fade, Kind::Unspecified);
                if let Some(cropped) = CropRenderElement::from_element(element, scale, visible_area) {
                    elements.push(OutputElement::from(cropped));
                }
            }
        }
        elements
    }

    /// Soft shadows around the windows on `output` (behind all windows), when `[effects] shadow` is set.
    pub fn shadow_elements(
        &mut self,
        renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
        output: &Output,
    ) -> Vec<crate::render::OutputElement> {
        use crate::render::OutputElement;
        let spread_logical = self.config.effects.shadow;
        if spread_logical <= 0 || !self.ensure_effect_shaders(renderer) {
            return Vec::new();
        }
        let Some(shader) = self.effect_shaders.as_ref().map(|s| s.shadow.clone()) else { return Vec::new() };
        let Some(geo) = self.space.output_geometry(output) else { return Vec::new() };
        let Some(this_monitor) = self.outputs.iter().position(|e| &e.output == output) else { return Vec::new() };
        let scale = output.current_scale().fractional_scale();
        let (radius_logical, anim_ms) = (self.config.effects.corner_radius, self.config.effects.animation_ms);
        let monitors: Vec<Option<usize>> = self.desktop.windows.iter().map(|m| self.monitor_of_window(m)).collect();
        let visible_area = Rectangle::<i32, Physical>::from_size(geo.size.to_physical_precise_round(scale));
        let mut elements = Vec::new();
        for (m, monitor) in self.desktop.windows.iter_mut().zip(monitors).rev() {
            if monitor != Some(this_monitor) || m.fullscreen {
                continue;
            }
            let Some((frame, b)) = m.frame else { continue };
            let slide = m.slide_offset(anim_ms);
            let area = Rectangle::<i32, Logical>::new(
                (frame.x + slide.x - geo.loc.x - spread_logical, frame.y + slide.y - geo.loc.y - spread_logical).into(),
                (frame.width + 2 * spread_logical, frame.height + 2 * spread_logical).into(),
            );
            let spread = (f64::from(spread_logical) * scale) as f32;
            let radius = (f64::from(radius_logical + if radius_logical > 0 { b } else { 0 }) * scale) as f32;
            let strength = 0.5 * m.fade(anim_ms);
            let key = (area, (spread * 64.0) as i32, (radius * 64.0) as i32, [strength, 0.0, 0.0, 0.0]);
            match &mut m.shadow {
                Some((_, old)) if *old == key => {}
                Some((element, old)) => {
                    element.resize(area, None);
                    element.update_uniforms(crate::effects::shadow_uniforms(spread, radius, strength));
                    *old = key;
                }
                None => {
                    let element = PixelShaderElement::new(
                        shader.clone(),
                        area,
                        None,
                        1.0,
                        crate::effects::shadow_uniforms(spread, radius, strength),
                        Kind::Unspecified,
                    );
                    m.shadow = Some((element, key));
                }
            }
            if let Some((element, _)) = &m.shadow
                && let Some(cropped) = CropRenderElement::from_element(element.clone(), scale, visible_area)
            {
                elements.push(OutputElement::from(cropped));
            }
        }
        elements
    }

    /// Some window is still animating: keep redrawing.
    pub fn animations_active(&self) -> bool {
        let ms = self.config.effects.animation_ms;
        ms > 0 && self.desktop.windows.iter().any(|m| m.animating(ms))
    }

    pub fn focus_window(&mut self, id: WindowId) {
        let d = &mut self.desktop;
        if d.focused() == Some(id) {
            return;
        }
        if d.scratchpad_visible && d.in_scratchpad(id) {
            d.scratchpad.focused = Some(id);
            d.focus_scratch = true;
        } else if let Some((monitor, _)) = d.desk.locate(&id)
            && d.desk.monitors[monitor].workspaces.focus(&id)
        {
            d.focused_monitor = monitor;
            d.focus_scratch = false;
        } else {
            return;
        }
        self.refresh();
    }

    pub fn toggle_floating(&mut self) {
        let Some(id) = self.desktop.focused() else { return };
        let area = self.desktop.monitor_area_of(id);
        if let Some(m) = self.desktop.get_mut(id) {
            if !m.floating
                && let (Some((frame, _)), Some(area)) = (m.frame, area)
            {
                // Start floating where the window currently is (relative to its monitor).
                m.floating_rect = Some(Rect { x: frame.x - area.x, y: frame.y - area.y, ..frame });
            }
            m.floating = !m.floating;
            m.fullscreen = false;
        }
        self.refresh();
    }

    pub fn set_fullscreen(&mut self, id: WindowId, fullscreen: bool) {
        if let Some(m) = self.desktop.get_mut(id) {
            m.fullscreen = fullscreen;
        }
        if fullscreen && let Some((monitor, _)) = self.desktop.desk.locate(&id) {
            self.desktop.desk.monitors[monitor].workspaces.focus(&id);
        }
        self.refresh();
    }

    pub fn toggle_fullscreen(&mut self) {
        if let Some(id) = self.desktop.focused() {
            let now = self.desktop.get(id).is_some_and(|m| m.fullscreen);
            self.set_fullscreen(id, !now);
        }
    }

    /// Grow or shrink the focused column by a fraction of the viewport width.
    pub fn resize_column(&mut self, permille: i32) {
        let Some(id) = self.desktop.focused() else { return };
        let Some(area) = self.desktop.monitor_area_of(id) else { return };
        let viewport = self.desktop.appearance.viewport(area);
        let Some(m) = self.desktop.get_mut(id) else { return };
        if m.floating {
            return;
        }
        let current = m.frame.map(|(f, _)| f.width).unwrap_or(viewport.width / 2);
        m.tiled_width = Some((current + viewport.width * permille / 1000).clamp(100, viewport.width));
        self.refresh();
    }

    pub fn begin_drag(&mut self, kind: DragKind) {
        let under = self.window_at(self.pointer_location).map(|(w, _)| w);
        let Some(m) = self.desktop.windows.iter().find(|m| under.as_ref() == Some(&m.window)) else {
            return;
        };
        let (id, floating) = (m.id, m.floating);
        let Some((frame, _)) = m.frame else { return };
        // Dragging moves floating windows; resizing also works on tiled columns.
        if !floating && matches!(kind, DragKind::Move) {
            return;
        }
        let area = self.desktop.monitor_area_of(id).unwrap_or(frame);
        // Floating rects are relative to the monitor's work area.
        let relative = Rect { x: frame.x - area.x, y: frame.y - area.y, ..frame };
        let start_rect = m.floating_rect.unwrap_or(relative);
        self.desktop.drag = Some(Drag {
            id,
            kind,
            start: self.pointer_location,
            start_rect,
            start_width: frame.width,
        });
        self.focus_window(id);
    }

    pub fn update_drag(&mut self) {
        let Some(drag) = &self.desktop.drag else { return };
        let dx = (self.pointer_location.x - drag.start.x) as i32;
        let dy = (self.pointer_location.y - drag.start.y) as i32;
        let (id, kind, start_rect, start_width) = (drag.id, drag.kind, drag.start_rect, drag.start_width);
        let Some(area) = self.desktop.monitor_area_of(id) else { return };
        let viewport_width = self.desktop.appearance.viewport(area).width;
        let Some(m) = self.desktop.get_mut(id) else { return };
        if m.floating {
            m.floating_rect = Some(start_rect.dragged(kind, dx, dy, area.width, area.height));
        } else if let DragKind::Resize(Edges { right: true, .. }) = kind {
            m.tiled_width = Some((start_width + dx).clamp(100, viewport_width));
        }
        self.refresh();
    }

    pub fn end_drag(&mut self) {
        self.desktop.drag = None;
    }

    pub fn focus_step(&mut self, direction: isize) {
        if let Some(monitor) = self.desktop.desk.monitors.get_mut(self.desktop.focused_monitor) {
            monitor.workspaces.navigate(direction, false);
        }
        self.desktop.focus_scratch = false;
        self.refresh();
    }

    pub fn move_column(&mut self, direction: isize) {
        let floating: Vec<_> = self.desktop.windows.iter().filter(|w| w.floating).map(|w| w.id).collect();
        if let Some(monitor) = self.desktop.desk.monitors.get_mut(self.desktop.focused_monitor) {
            monitor.workspaces.navigate_matching(direction, true, |id| !floating.contains(id));
        }
        self.refresh();
    }

    pub fn toggle_scratchpad(&mut self) {
        let d = &mut self.desktop;
        if d.scratchpad.windows.is_empty() {
            return;
        }
        d.scratchpad_visible = !d.scratchpad_visible;
        d.focus_scratch = d.scratchpad_visible;
        self.refresh();
    }

    pub fn move_to_scratchpad(&mut self) {
        let Some(id) = self.desktop.focused() else { return };
        let d = &mut self.desktop;
        if d.in_scratchpad(id) {
            d.scratchpad.remove(&id);
            d.scratchpad_visible = !d.scratchpad.windows.is_empty();
            d.focus_scratch = d.scratchpad_visible;
            if let Some(monitor) = d.desk.monitors.get_mut(d.focused_monitor) {
                monitor.workspaces.add(id);
            }
            if let Some(m) = d.get_mut(id) {
                m.floating = m.scratchpad_floating.take().unwrap_or(true);
                m.floating_rect = None;
            }
        } else {
            d.desk.remove_window(&id);
            d.desk.prune();
            d.scratchpad.windows.push(id);
            d.scratchpad.focused = Some(id);
            d.scratchpad_visible = true;
            d.focus_scratch = true;
            if let Some(m) = d.get_mut(id) {
                m.scratchpad_floating = Some(m.floating);
                m.floating = true;
                m.fullscreen = false;
            }
        }
        self.refresh();
    }
}
