//! Outputs as monitors: hotplug, pointer/monitor focus and the workspace commands that span monitors.
use mywm_layout::{Direction, GAMING, Rect};
use smithay::{
    backend::renderer::element::{
        Kind,
        solid::{SolidColorBuffer, SolidColorRenderElement},
        utils::CropRenderElement,
    },
    desktop::{Window, layer_map_for_output, space::SpaceElement},
    output::Output,
    utils::{Logical, Physical, Point, Rectangle},
};

use crate::{State, desktop::WindowId, render::OutputElement};

/// Width of the strip at a monitor's edge that says windows are scrolled out beyond it (logical pixels).
const EDGE_MARKER: i32 = 5;

/// An output known to the compositor; its index equals its monitor's index in the `Desk`.
pub struct OutputEntry {
    pub output: Output,
    /// Stable id for the bar protocol.
    pub id: u32,
    /// The display is on (`false` while switched off for idle).
    pub powered: bool,
    /// Windows kept out of captures as last drawn here (see `State::capture_view`).
    pub capture_blanks: Vec<crate::render::Blank>,
    /// Tiled windows lie beyond the left and the right edge: a strip there says so and keeps its width
    /// free of windows. (Drawn here: as panels of the shell each one cost a window and a GL context.)
    pub edges: (bool, bool),
    edge_markers: [SolidColorBuffer; 2],
}

impl State {
    pub fn output_entry(&self, monitor: usize) -> Option<&OutputEntry> {
        self.outputs.get(monitor)
    }

    /// Copy the outputs' current geometry (mode, scale, transform, position) into the desk.
    pub fn sync_monitor_areas(&mut self) {
        for (entry, monitor) in self.outputs.iter().zip(&mut self.desktop.desk.monitors) {
            if let Some(geo) = self.space.output_geometry(&entry.output) {
                monitor.area = Rect { x: geo.loc.x, y: geo.loc.y, width: geo.size.w, height: geo.size.h };
                // Bars and panels reserve part of the output (exclusive zones).
                let zone = layer_map_for_output(&entry.output).non_exclusive_zone();
                monitor.usable = if zone.size.w > 0 && zone.size.h > 0 {
                    Rect { x: geo.loc.x + zone.loc.x, y: geo.loc.y + zone.loc.y, width: zone.size.w, height: zone.size.h }
                } else {
                    monitor.area
                };
                // The edge markers reserve their strips like panels.
                let (left, right) = entry.edges;
                if left {
                    monitor.usable.x += EDGE_MARKER;
                    monitor.usable.width -= EDGE_MARKER;
                }
                if right {
                    monitor.usable.width -= EDGE_MARKER;
                }
            }
        }
    }

    /// Add an output. Without a position it goes to the right of the existing ones.
    pub fn add_output(&mut self, output: Output, position: Option<Point<i32, Logical>>) -> usize {
        let position = position.unwrap_or_else(|| {
            let right = self
                .outputs
                .iter()
                .filter_map(|e| self.space.output_geometry(&e.output))
                .map(|g| g.loc.x + g.size.w)
                .max()
                .unwrap_or(0);
            (right, 0).into()
        });
        self.space.map_output(&output, position);
        output.change_current_state(None, None, None, Some(position));
        let geo = self.space.output_geometry(&output).unwrap_or_default();
        let area = Rect { x: geo.loc.x, y: geo.loc.y, width: geo.size.w, height: geo.size.h };
        let id = self.next_output_id;
        self.next_output_id += 1;
        tracing::info!(
            "output {} at {},{} size {}x{} scale {}",
            output.name(),
            area.x,
            area.y,
            area.width,
            area.height,
            output.current_scale().fractional_scale()
        );
        let monitor = self.desktop.desk.add_monitor(Some(output.name()), area);
        debug_assert_eq!(monitor, self.outputs.len());
        self.outputs.push(OutputEntry {
            output,
            id,
            powered: true,
            capture_blanks: Vec::new(),
            edges: (false, false),
            edge_markers: Default::default(),
        });
        self.place_all_pending();
        self.end_drag();
        self.refresh();
        self.output_management_changed();
        monitor
    }

    pub fn remove_output(&mut self, output: &Output) {
        let Some(index) = self.outputs.iter().position(|e| &e.output == output) else { return };
        self.end_drag();
        self.fail_screencopies_of(output);
        self.image_capture_output_removed(output);
        // Floating rects belong to a monitor's work area; recenter windows that change monitor.
        let before: Vec<_> = self.desktop.windows.iter().map(|m| (m.id, self.desktop.desk.locate(&m.id))).collect();
        self.space.unmap_output(output);
        self.outputs.remove(index);
        self.desktop.desk.remove_monitor(index);
        for (id, old) in before {
            let new = self.desktop.desk.locate(&id).map(|(m, _)| m);
            let old = old.and_then(|(m, _)| mywm_layout::scrolling::index_after_remove(m, index));
            if old != new
                && let Some(m) = self.desktop.get_mut(id)
            {
                m.floating_rect = None;
            }
        }
        let focused = self.desktop.focused_monitor;
        self.desktop.focused_monitor = mywm_layout::scrolling::index_after_remove(focused, index).unwrap_or(0);
        self.desktop.desk.prune();
        self.refresh();
        self.output_management_changed();
        // Keep the pointer somewhere visible.
        let p = self.desktop.desk.clamp_to_monitors((self.pointer_location.x, self.pointer_location.y));
        if (p.0, p.1) != (self.pointer_location.x, self.pointer_location.y) {
            self.warp_pointer(p.into());
        }
    }

    /// Monitor under the pointer.
    pub fn pointer_monitor(&self) -> Option<usize> {
        self.desktop
            .desk
            .monitor_at(self.pointer_location.x.floor() as i32, self.pointer_location.y.floor() as i32)
    }

    /// Name of the monitor with workspace 1 (the first of `workspace_outputs`): where the login and lock
    /// screens ask for the password (`MYWM_PRIMARY_OUTPUT`).
    pub fn primary_output_name(&self) -> Option<String> {
        let desk = &self.desktop.desk;
        let monitor = (0..desk.monitors.len()).find(|index| desk.home_number(*index) == 1)?;
        self.outputs.get(monitor).map(|entry| entry.output.name())
    }

    /// The scratchpad appears on the monitor under the pointer.
    pub fn scratchpad_monitor(&self) -> usize {
        self.pointer_monitor().unwrap_or(self.desktop.focused_monitor)
    }

    /// Make `monitor` the one keyboard commands act on.
    pub fn focus_monitor(&mut self, monitor: usize) {
        if monitor >= self.desktop.desk.monitors.len() {
            return;
        }
        self.desktop.focused_monitor = monitor;
        self.desktop.focus_scratch = false;
        self.refresh();
    }

    /// Follow the pointer across monitors. Returns whether the focused monitor changed.
    pub fn update_pointer_monitor(&mut self) -> bool {
        match self.pointer_monitor() {
            Some(monitor) if monitor != self.desktop.focused_monitor && self.desktop.drag.is_none() => {
                self.focus_monitor(monitor);
                true
            }
            _ => false,
        }
    }

    pub fn warp_to_monitor(&mut self, monitor: usize) {
        if let Some(m) = self.desktop.desk.monitors.get(monitor) {
            let center = (m.area.x + m.area.width / 2, m.area.y + m.area.height / 2);
            self.warp_pointer((f64::from(center.0), f64::from(center.1)).into());
        }
    }

    pub fn focus_output_direction(&mut self, direction: Direction) {
        let source = self.desktop.focused_monitor;
        if let Some(target) = self.desktop.desk.adjacent(source, direction) {
            self.desktop.focused_monitor = target;
            self.warp_to_monitor(target);
            self.focus_monitor(target);
        }
    }

    /// Move the focused window one column that way, or to the neighbouring monitor at the end.
    pub fn move_to_output_direction(&mut self, direction: Direction) {
        let Some(id) = self.desktop.focused().filter(|id| !self.desktop.in_scratchpad(*id)) else { return };
        let Some((source, _)) = self.desktop.desk.locate(&id) else { return };
        let floating = self.desktop.get(id).is_some_and(|m| m.floating);
        let horizontal = match direction {
            Direction::Left => Some(-1),
            Direction::Right => Some(1),
            Direction::Up | Direction::Down => None,
        };
        if !floating && let Some(step) = horizontal {
            let floating_ids: Vec<_> = self.desktop.windows.iter().filter(|w| w.floating).map(|w| w.id).collect();
            let workspaces = &mut self.desktop.desk.monitors[source].workspaces;
            if workspaces.can_navigate_matching(step, |c| !floating_ids.contains(c)) {
                workspaces.navigate_matching(step, true, |c| !floating_ids.contains(c));
                self.refresh();
                return;
            }
        }
        let Some(target) = self.desktop.desk.adjacent(source, direction) else { return };
        let number = self.desktop.desk.monitors[target].workspaces.active;
        if number == GAMING {
            return; // only games belong there
        }
        self.end_drag();
        self.desktop.desk.move_window(&id, number);
        if floating && let Some(m) = self.desktop.get_mut(id) {
            m.floating_rect = None;
        }
        self.desktop.desk.prune();
        self.desktop.focused_monitor = target;
        self.warp_to_monitor(target);
        self.refresh();
    }

    pub fn select_workspace(&mut self, number: usize) {
        let Some(monitor) = self.desktop.desk.owner(number) else { return };
        self.select_workspace_on(monitor, number);
    }

    pub fn select_workspace_on(&mut self, monitor: usize, number: usize) {
        self.end_drag();
        let desk = &mut self.desktop.desk;
        if monitor >= desk.monitors.len() || !desk.monitors[monitor].workspaces.contains(number) {
            return;
        }
        desk.monitors[monitor].workspaces.select(number);
        desk.prune();
        if self.pointer_monitor() != Some(monitor) {
            self.warp_to_monitor(monitor);
        }
        self.focus_monitor(monitor);
    }

    pub fn cycle_workspace(&mut self, direction: isize) {
        let monitor = self.desktop.focused_monitor;
        if let Some(target) = self.desktop.desk.relative_workspace(monitor, direction) {
            self.select_workspace_on(monitor, target);
        }
    }

    pub fn new_workspace(&mut self) {
        self.new_workspace_on(self.desktop.focused_monitor);
    }

    pub fn new_workspace_on(&mut self, monitor: usize) {
        if monitor < self.desktop.desk.monitors.len()
            && let Some(number) = self.desktop.desk.create(monitor)
        {
            self.select_workspace_on(monitor, number);
        }
    }

    /// Move the focused window to workspace `number` (wherever it lives); focus stays put.
    pub fn move_to_workspace(&mut self, number: usize) {
        let Some(id) = self.desktop.focused().filter(|id| !self.desktop.in_scratchpad(*id)) else { return };
        self.move_window_to_workspace(id, number);
    }

    /// Move `id` to workspace `number` (wherever it lives), also out of the scratchpad.
    pub fn move_window_to_workspace(&mut self, id: WindowId, number: usize) {
        // The gaming workspace is reserved for games.
        if number == GAMING {
            return;
        }
        if self.desktop.in_scratchpad(id) {
            let Some(target) = self.desktop.desk.owner(number) else { return };
            self.take_from_scratchpad(id);
            self.desktop.desk.monitors[target].workspaces.add_to(number, id);
            return self.refresh();
        }
        let Some((source, _)) = self.desktop.desk.locate(&id) else { return };
        self.end_drag();
        if let Some(target) = self.desktop.desk.move_window(&id, number) {
            if target != source && let Some(m) = self.desktop.get_mut(id) {
                // Floating coordinates belong to the source monitor; recenter on the target.
                m.floating_rect = None;
            }
            self.desktop.desk.prune();
            self.refresh();
        }
    }

    pub fn move_to_workspace_relative(&mut self, direction: isize) {
        let monitor = self.desktop.focused_monitor;
        if let Some(number) = self.desktop.desk.relative_workspace(monitor, direction) {
            self.move_to_workspace(number);
        }
    }

    pub fn move_to_new_workspace(&mut self) {
        let monitor = self.desktop.focused_monitor;
        if monitor < self.desktop.desk.monitors.len()
            && let Some(number) = self.desktop.desk.create(monitor)
        {
            self.move_to_workspace(number);
        }
    }
}

impl State {
    /// State for the bar (see `mywm-ipc`).
    pub fn ipc_snapshot(&self) -> mywm_ipc::Snapshot {
        let d = &self.desktop;
        let mut snapshot = mywm_ipc::Snapshot {
            scratchpad_visible: d.scratchpad_visible,
            scratchpad_occupied: !d.scratchpad.windows.is_empty(),
            locked: self.session_lock.is_locked(),
            gaming: self.game_shown(),
            capturing: self.capturing(),
            choosing: self.share_kinds().is_some(),
            picking: self.picked_color(),
            config_error: self.config_error.clone(),
            ..Default::default()
        };
        for (index, (entry, monitor)) in self.outputs.iter().zip(&d.desk.monitors).enumerate() {
            let (overflow_left, overflow_right) = self.overflow(index);
            snapshot.outputs.push(mywm_ipc::OutputState {
                id: entry.id,
                x: monitor.area.x,
                y: monitor.area.y,
                width: monitor.area.width,
                height: monitor.area.height,
                active: monitor.workspaces.active,
                overflow_left,
                overflow_right,
                workspaces: monitor.workspaces.entries.iter().map(|w| (w.number, !w.windows.is_empty())).collect(),
            });
        }
        snapshot
    }

    /// Whether tiled windows of `monitor`'s workspace lie beyond the left and the right edge of its usable area.
    fn overflow(&self, monitor: usize) -> (bool, bool) {
        let d = &self.desktop;
        let Some(monitor) = d.desk.monitors.get(monitor) else { return (false, false) };
        let tiled = monitor
            .workspaces
            .current()
            .windows
            .iter()
            .filter_map(|id| d.get(*id))
            .filter(|m| !m.floating)
            .filter_map(|m| m.frame)
            .map(|(f, b)| Rect { x: f.x + b, y: f.y + b, width: f.width - 2 * b, height: f.height - 2 * b });
        mywm_ipc::overflow_directions(monitor.usable, tiled)
    }

    /// Show or hide the edge markers after a layout change; true if one changed (the usable areas then
    /// changed with them).
    pub fn update_edges(&mut self) -> bool {
        let edges: Vec<_> = (0..self.outputs.len()).map(|index| self.overflow(index)).collect();
        let mut changed = false;
        for (entry, edges) in self.outputs.iter_mut().zip(edges) {
            changed |= entry.edges != edges;
            entry.edges = edges;
        }
        if changed {
            self.sync_monitor_areas();
        }
        changed
    }

    /// The edge markers of `output` (see `OutputEntry::edges`) in the active border's color.
    pub fn edge_marker_elements(&mut self, output: &Output) -> Vec<OutputElement> {
        let Some(index) = self.outputs.iter().position(|e| &e.output == output) else { return Vec::new() };
        let (Some(geo), Some(usable)) = (self.space.output_geometry(output), self.desktop.desk.monitors.get(index).map(|m| m.usable))
        else {
            return Vec::new();
        };
        let scale = output.current_scale().fractional_scale();
        let visible_area = Rectangle::<i32, Physical>::from_size(geo.size.to_physical_precise_round(scale));
        let color = self.desktop.appearance.active_border.0;
        let entry = &mut self.outputs[index];
        let sides = [(entry.edges.0, usable.x - EDGE_MARKER), (entry.edges.1, usable.x + usable.width)];
        let mut elements = Vec::new();
        for ((shown, x), buffer) in sides.into_iter().zip(&mut entry.edge_markers) {
            if !shown {
                continue;
            }
            buffer.update((EDGE_MARKER, usable.height), color);
            let local = (((x - geo.loc.x) as f64 * scale).round() as i32, ((usable.y - geo.loc.y) as f64 * scale).round() as i32);
            let element = SolidColorRenderElement::from_buffer(buffer, local, scale, 1.0, Kind::Unspecified);
            if let Some(cropped) = CropRenderElement::from_element(element, scale, visible_area) {
                elements.push(OutputElement::from(cropped));
            }
        }
        elements
    }
}

impl State {
    /// The window under `point` and its render location, front to back.
    ///
    /// Like the drawing, this only considers windows of the monitor the point is on, so tiles
    /// scrolled off one monitor's edge cannot catch the pointer on its neighbour.
    pub fn window_at(&self, point: Point<f64, Logical>) -> Option<(Window, Point<i32, Logical>)> {
        let monitor = self.desktop.desk.monitor_at(point.x.floor() as i32, point.y.floor() as i32);
        self.space.elements().rev().find_map(|window| {
            let owner = self.desktop.windows.iter().find(|m| &m.window == window).map(|m| self.monitor_of_window(m));
            if owner.is_some_and(|owner| owner != monitor) {
                return None;
            }
            let render_location = self.space.element_location(window)? - window.geometry().loc;
            // Popups (menus) may reach beyond the window itself.
            let mut area = window.bbox_with_popups();
            area.loc += render_location;
            if !area.to_f64().contains(point) {
                return None;
            }
            window
                .is_in_input_region(&(point - render_location.to_f64()))
                .then(|| (window.clone(), render_location))
        })
    }
}
