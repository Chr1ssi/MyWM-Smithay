//! Window management model: windows, workspaces and their mapping onto the smithay `Space`.
use mywm_layout::{
    Appearance, DragKind, Edges, GAMING, Kind as WorkspaceKind, Placement, Rect, WindowInfo, Workspace, Workspaces, arrange,
    place_floating,
};
use smithay::{
    backend::renderer::element::{Kind, solid::{SolidColorBuffer, SolidColorRenderElement}},
    desktop::Window,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point},
    wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData},
};

use crate::State;

pub type WindowId = u64;

pub struct Managed {
    pub id: WindowId,
    pub window: Window,
    pub floating: bool,
    pub fullscreen: bool,
    pub tiled_width: Option<i32>,
    pub floating_rect: Option<Rect>,
    pub parent: Option<WindowId>,
    pub app_id: Option<String>,
    /// Floating state to restore when leaving the scratchpad.
    pub scratchpad_floating: Option<bool>,
    /// Rules and workspace are applied at the first commit, once the app id is known.
    pub placed: bool,
    /// Frame (content plus border) in output coordinates, while placed.
    pub frame: Option<(Rect, i32)>,
    /// Top, bottom, left, right border strips. Buffers persist so damage tracking stays exact.
    borders: [SolidColorBuffer; 4],
}

impl Managed {
    fn surface(&self) -> Option<&WlSurface> {
        self.window.toplevel().map(|t| t.wl_surface())
    }

    fn info(&self) -> WindowInfo<WindowId> {
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
    pub workspaces: Workspaces<WindowId>,
    pub appearance: Appearance,
    pub drag: Option<Drag>,
    /// Global floating stack shown above all workspaces.
    pub scratchpad: Workspace<WindowId>,
    pub scratchpad_visible: bool,
    /// Whether keyboard focus is on the scratchpad rather than the workspace.
    pub focus_scratch: bool,
    next_id: WindowId,
}

impl Default for Desktop {
    fn default() -> Self {
        Self {
            windows: Vec::new(),
            workspaces: Workspaces::new(1),
            appearance: Appearance::default(),
            drag: None,
            scratchpad: Workspace::default(),
            scratchpad_visible: false,
            focus_scratch: false,
            next_id: 1,
        }
    }
}

impl Desktop {
    pub fn get(&self, id: WindowId) -> Option<&Managed> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut Managed> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    pub fn by_surface(&self, surface: &WlSurface) -> Option<&Managed> {
        self.windows.iter().find(|w| w.surface() == Some(surface))
    }

    pub fn focused(&self) -> Option<WindowId> {
        if self.scratchpad_visible && self.focus_scratch && self.scratchpad.focused.is_some() {
            self.scratchpad.focused
        } else {
            self.workspaces.current().focused
        }
    }

    pub fn in_scratchpad(&self, id: WindowId) -> bool {
        self.scratchpad.windows.contains(&id)
    }
}

impl State {
    pub fn work_area(&self) -> Option<Rect> {
        let output = self.output.as_ref()?;
        let geo = self.space.output_geometry(output)?;
        Some(Rect { x: geo.loc.x, y: geo.loc.y, width: geo.size.w, height: geo.size.h })
    }

    /// Register a new toplevel. It joins a workspace at its first commit (see `place_pending`).
    pub fn add_window(&mut self, window: Window) {
        let toplevel = window.toplevel().expect("wayland window").clone();
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
            scratchpad_floating: None,
            placed: false,
            frame: None,
            borders: Default::default(),
        });
        // The client waits for a configure before it may attach a buffer.
        toplevel.send_configure();
    }

    /// Apply window rules and pick the workspace once the app id is known.
    pub fn place_pending(&mut self, surface: &WlSurface) {
        let Some(id) = self.desktop.by_surface(surface).filter(|m| !m.placed).map(|m| m.id) else {
            return;
        };
        self.place(id);
        self.refresh();
    }

    fn place(&mut self, id: WindowId) {
        let Some(m) = self.desktop.get(id) else { return };
        if m.placed {
            return;
        }
        let surface = m.surface().cloned();
        let app_id = surface.as_ref().and_then(|surface| {
            with_states(surface, |states| {
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
        // A child may be announced before its parent; place the parent first.
        if let Some(parent) = parent {
            self.place(parent);
        }
        tracing::info!("new window: app_id={app_id:?} dialog={}", parent.is_some());

        let placement = mywm_config::resolve(
            &self.config.rules,
            app_id.as_deref(),
            parent.is_some(),
            self.config.float_dialogs,
        );
        let game = self.is_game(app_id.as_deref(), parent);
        let d = &mut self.desktop;
        let ws = &mut d.workspaces;
        let active = parent.and_then(|p| ws.location(&p)).unwrap_or(ws.active);
        // Rules only apply while their workspace exists.
        let mut number = placement.workspace.filter(|n| ws.contains(*n)).unwrap_or(active);
        if game {
            ws.ensure(GAMING, WorkspaceKind::Gaming);
            number = GAMING;
            ws.select(GAMING);
        } else if number == GAMING {
            // Only games start on the gaming workspace.
            number = ws.home;
            ws.select(number);
        }
        if number == ws.active {
            ws.add(id);
        } else {
            ws.add_to(number, id);
        }
        d.focus_scratch = false;
        if let Some(m) = d.get_mut(id) {
            m.placed = true;
            m.floating = placement.floating;
            m.parent = parent;
            m.app_id = app_id;
        }
    }

    /// Games (and their dialogs) are recognized by app id prefix.
    fn is_game(&self, app_id: Option<&str>, parent: Option<WindowId>) -> bool {
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
        if let Some(managed) = self.desktop.get(id) {
            self.space.unmap_elem(&managed.window);
        }
        self.desktop.workspaces.remove(&id);
        self.desktop.scratchpad.remove(&id);
        if self.desktop.scratchpad.windows.is_empty() {
            self.desktop.scratchpad_visible = false;
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
        self.desktop.workspaces.prune();
        self.refresh();
    }

    /// Recompute the layout and push it to the clients, then sync keyboard focus.
    pub fn refresh(&mut self) {
        self.apply_layout();
        self.sync_focus();
    }

    fn apply_layout(&mut self) {
        let Some(area) = self.work_area() else { return };
        let d = &mut self.desktop;
        let infos: Vec<_> = d.windows.iter().filter(|m| m.placed).map(Managed::info).collect();
        let mut placements: Vec<Placement<WindowId>> =
            arrange(d.workspaces.current_mut(), &infos, area, &d.appearance);
        if d.scratchpad_visible {
            placements.extend(
                d.scratchpad
                    .windows
                    .iter()
                    .filter_map(|id| infos.iter().find(|i| i.id == *id))
                    .map(|info| place_floating(info, area, &d.appearance)),
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
        let focused = d.workspaces.current().focused;
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
            }
            // Mapping in paint order keeps floating windows above tiled ones.
            self.space
                .map_element(m.window.clone(), (p.content.x, p.content.y), false);
            m.frame = (p.border > 0).then_some((
                Rect {
                    x: p.content.x - p.border,
                    y: p.content.y - p.border,
                    width: p.content.width + 2 * p.border,
                    height: p.content.height + 2 * p.border,
                },
                p.border,
            ));
            let color = if focused == Some(m.id) { active } else { inactive };
            if let Some((frame, b)) = m.frame {
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

    fn sync_focus(&mut self) {
        let focused = self.desktop.focused();
        for m in &self.desktop.windows {
            if let Some(toplevel) = m.window.toplevel()
                && m.window.set_activated(Some(m.id) == focused)
            {
                toplevel.send_pending_configure();
            }
        }
        let surface = focused
            .and_then(|id| self.desktop.get(id))
            .and_then(|m| m.surface().cloned());
        self.set_keyboard_focus(surface);
    }

    /// Border rectangles for the active workspace, top-most first.
    pub fn border_elements(&self) -> Vec<SolidColorRenderElement> {
        let mut elements = Vec::new();
        for m in self.desktop.windows.iter().rev() {
            let Some((frame, b)) = m.frame else { continue };
            let origins = [
                (frame.x, frame.y),
                (frame.x, frame.y + frame.height - b),
                (frame.x, frame.y + b),
                (frame.x + frame.width - b, frame.y + b),
            ];
            for (buffer, origin) in m.borders.iter().zip(origins) {
                elements.push(SolidColorRenderElement::from_buffer(
                    buffer, origin, 1.0, 1.0, Kind::Unspecified,
                ));
            }
        }
        elements
    }

    pub fn focus_window(&mut self, id: WindowId) {
        let d = &mut self.desktop;
        if d.focused() == Some(id) {
            return;
        }
        if d.scratchpad_visible && d.in_scratchpad(id) {
            d.scratchpad.focused = Some(id);
            d.focus_scratch = true;
        } else if d.workspaces.focus(&id) {
            d.focus_scratch = false;
        } else {
            return;
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
            d.workspaces.add(id);
            if let Some(m) = d.get_mut(id) {
                m.floating = m.scratchpad_floating.take().unwrap_or(true);
            }
        } else {
            d.workspaces.remove(&id);
            d.workspaces.prune();
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

    pub fn move_to_workspace_relative(&mut self, direction: isize) {
        if let Some(number) = self.desktop.workspaces.relative(direction) {
            self.move_to_workspace(number);
        }
    }

    pub fn focus_step(&mut self, direction: isize) {
        self.desktop.workspaces.navigate(direction, false);
        self.refresh();
    }

    pub fn move_column(&mut self, direction: isize) {
        let floating: Vec<_> = self.desktop.windows.iter().filter(|w| w.floating).map(|w| w.id).collect();
        self.desktop
            .workspaces
            .navigate_matching(direction, true, |id| !floating.contains(id));
        self.refresh();
    }

    pub fn select_workspace(&mut self, number: usize) {
        self.desktop.workspaces.select(number);
        self.desktop.workspaces.prune();
        self.refresh();
    }

    pub fn cycle_workspace(&mut self, direction: isize) {
        self.desktop.workspaces.cycle(direction);
        self.desktop.workspaces.prune();
        self.refresh();
    }

    pub fn new_workspace(&mut self) {
        if let Some(number) = self.desktop.workspaces.free_number() {
            self.desktop.workspaces.ensure(number, mywm_layout::Kind::Extra);
            self.select_workspace(number);
        }
    }

    pub fn move_to_workspace(&mut self, number: usize) {
        let d = &mut self.desktop;
        let ws = &mut d.workspaces;
        let Some(id) = ws.current().focused.filter(|id| !d.scratchpad.windows.contains(id)) else {
            return;
        };
        if number == ws.active || number == mywm_layout::GAMING || number > mywm_layout::MAX_NUMBER {
            return;
        }
        ws.remove(&id);
        ws.add_to(number, id);
        ws.prune();
        self.refresh();
    }

    pub fn move_to_new_workspace(&mut self) {
        if let Some(number) = self.desktop.workspaces.free_number() {
            self.move_to_workspace(number);
        }
    }

    pub fn toggle_floating(&mut self) {
        if let Some(m) = self.desktop.focused().and_then(|id| self.desktop.get_mut(id)) {
            if !m.floating {
                // Start floating where the window currently is.
                m.floating_rect = m.frame.map(|(f, _)| f).or(m.floating_rect);
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
        if fullscreen {
            self.desktop.workspaces.focus(&id);
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
        let Some(area) = self.work_area() else { return };
        let viewport = self.desktop.appearance.viewport(area);
        let Some(m) = self.desktop.focused().and_then(|id| self.desktop.get_mut(id)) else { return };
        if m.floating {
            return;
        }
        let current = m.frame.map(|(f, _)| f.width).unwrap_or(viewport.width / 2);
        m.tiled_width = Some((current + viewport.width * permille / 1000).clamp(100, viewport.width));
        self.refresh();
    }

    pub fn begin_drag(&mut self, kind: DragKind) {
        let under = self.space.element_under(self.pointer_location).map(|(w, _)| w.clone());
        let Some(m) = self
            .desktop
            .windows
            .iter()
            .find(|m| under.as_ref() == Some(&m.window))
        else {
            return;
        };
        let (id, floating) = (m.id, m.floating);
        let Some((frame, _)) = m.frame else { return };
        // Dragging moves floating windows; resizing also works on tiled columns.
        if !floating && matches!(kind, DragKind::Move) {
            return;
        }
        let start_rect = m.floating_rect.unwrap_or(frame);
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
        let Some(area) = self.work_area() else { return };
        let Some(drag) = &self.desktop.drag else { return };
        let dx = (self.pointer_location.x - drag.start.x) as i32;
        let dy = (self.pointer_location.y - drag.start.y) as i32;
        let (id, kind, start_rect, start_width) = (drag.id, drag.kind, drag.start_rect, drag.start_width);
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
}
