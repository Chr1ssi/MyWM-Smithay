//! Workspace overview: every workspace of a monitor as a live thumbnail.
//!
//! Click one (or pick it with the arrow keys and Enter) to go there, or a window in it to show that window;
//! Escape or a click beside the thumbnails closes the overview. Windows are drawn scaled, straight from
//! their surfaces. When an app asks what to share, the overview opens on every monitor and is how the user
//! picks: a window's thumbnail shares the window, the rest of a cell (or Enter) its monitor, `d` a dynamic
//! cast; closing it in any way answers that nothing was picked.
use mywm_ipc::{Chosen, SourceKinds};
use mywm_layout::{Rect, Workspace, arrange};
use smithay::{
    backend::{
        input::ButtonState,
        renderer::{
            element::{AsRenderElements, Id, Kind, utils::{CropRenderElement, RescaleRenderElement}, solid::SolidColorRenderElement, surface::WaylandSurfaceRenderElement},
            gles::GlesRenderer,
            utils::CommitCounter,
        },
    },
    output::Output,
    utils::{Logical, Physical, Point, Rectangle, Scale},
};

use crate::{State, desktop::WindowId, render::OutputElement, screenshot::ModalKey};

const BTN_LEFT: u32 = 0x110;

pub struct Overview {
    /// One per monitor it is shown on: the one under the pointer, or every monitor when picking what to share.
    parts: Vec<Part>,
    /// The app (bar-socket client) that asked what to share, and what it may share, while picking.
    share: Option<(u64, SourceKinds)>,
}

/// The overview on one monitor.
struct Part {
    output: Output,
    monitor: usize,
    /// Workspace numbers in display order.
    numbers: Vec<usize>,
    selected: usize,
}

/// A window as the overview shows it, in output-local logical pixels: in which cell, where its picture
/// starts (unrounded), how much it is shrunk, and its slot.
struct Thumbnail {
    cell: usize,
    id: WindowId,
    origin: (f64, f64),
    k: f64,
    slot: Rectangle<i32, Logical>,
}

impl State {
    pub fn toggle_overview(&mut self) {
        if self.overview.is_some() {
            return self.close_overview();
        }
        self.open_overview(None);
    }

    /// Show the overview on the monitor under the pointer, or on every monitor to pick what to share.
    pub fn open_overview(&mut self, share: Option<(u64, SourceKinds)>) -> bool {
        if self.session_lock.is_active() || self.overview.is_some() {
            return false;
        }
        let monitors: Vec<usize> = match share {
            Some(_) => (0..self.desktop.desk.monitors.len()).collect(),
            None => vec![self.pointer_monitor().unwrap_or(self.desktop.focused_monitor)],
        };
        let parts: Vec<Part> = monitors.into_iter().filter_map(|monitor| self.overview_part(monitor)).collect();
        if parts.is_empty() {
            return false;
        }
        self.overview = Some(Overview { parts, share });
        // The bar says how to pick.
        self.ipc_dirty |= share.is_some();
        self.queue_redraw_all();
        true
    }

    fn overview_part(&self, monitor: usize) -> Option<Part> {
        let output = self.outputs.get(monitor)?.output.clone();
        let workspaces = &self.desktop.desk.monitors.get(monitor)?.workspaces;
        let numbers: Vec<usize> = workspaces.numbers().collect();
        let selected = numbers.iter().position(|n| *n == workspaces.current().number).unwrap_or(0);
        Some(Part { output, monitor, numbers, selected })
    }

    /// Close the overview; one that was picking what to share answers that nothing was picked.
    pub fn close_overview(&mut self) {
        self.end_overview(Chosen::Nothing);
    }

    /// Close the overview, answering an app that asked what to share with `chosen`.
    fn end_overview(&mut self, chosen: Chosen) {
        let Some(overview) = self.overview.take() else { return };
        if let Some((client, _)) = overview.share {
            self.ipc_send_chosen(client, &chosen);
            self.ipc_dirty = true;
        }
        self.queue_redraw_all();
    }

    /// What the app picking a source may share, while the overview picks.
    pub fn share_kinds(&self) -> Option<SourceKinds> {
        self.overview.as_ref()?.share.map(|(_, kinds)| kinds)
    }

    /// The overview's part under the pointer.
    fn part_under_pointer(&self) -> Option<usize> {
        let output = self.space.output_under(self.pointer_location).next()?;
        self.overview.as_ref()?.parts.iter().position(|part| &part.output == output)
    }

    /// Every window a part shows, front to back within each cell.
    fn overview_thumbnails(&self, part: &Part) -> Vec<Thumbnail> {
        let Some(monitor) = self.desktop.desk.monitors.get(part.monitor) else { return Vec::new() };
        let (area, usable) = (monitor.area, monitor.usable);
        let infos: Vec<_> = self.desktop.windows.iter().filter(|m| m.placed && !m.awaiting_size).map(|m| m.info()).collect();
        let mut thumbnails = Vec::new();
        for (cell_index, (number, cell)) in part.numbers.iter().zip(self.overview_cells(part)).enumerate() {
            let Some(workspace) = monitor.workspaces.get(*number) else { continue };
            // Lay the workspace out as it would be shown (on a copy: arranging scrolls it).
            let mut copy = Workspace { number: workspace.number, kind: workspace.kind, windows: workspace.windows.clone(), focused: workspace.focused, scroll: workspace.scroll };
            let placements = arrange(&mut copy, &infos, usable, area, &self.desktop.appearance);
            // A scrolling workspace reaches beyond the screen: shrink the picture until every window fits.
            let left = placements.iter().map(|p| p.content.x).min().unwrap_or(area.x).min(area.x);
            let right = placements.iter().map(|p| p.content.x + p.content.width).max().unwrap_or(area.x + area.width).max(area.x + area.width);
            let span = f64::from((right - left).max(1));
            let k = f64::from(cell.size.w) / span;
            // Placements come back in paint order (back to front); we list front to back.
            for placement in placements.iter().rev() {
                let Rect { x, y, width, height } = placement.content;
                let origin = (
                    f64::from(cell.loc.x) + f64::from(x - left) * k,
                    f64::from(cell.loc.y) + f64::from(y - area.y) * k,
                );
                let slot = Rectangle::new(
                    (origin.0.round() as i32, origin.1.round() as i32).into(),
                    ((f64::from(width) * k).round() as i32, (f64::from(height) * k).round() as i32).into(),
                );
                thumbnails.push(Thumbnail { cell: cell_index, id: placement.id, origin, k, slot });
            }
        }
        thumbnails
    }

    /// The window whose thumbnail is at `point`, the frontmost where they overlap.
    fn overview_window_at(&self, part: &Part, point: Point<f64, Logical>) -> Option<WindowId> {
        let geo = self.space.output_geometry(&part.output)?;
        let local = point - geo.loc.to_f64();
        let cells = self.overview_cells(part);
        self.overview_thumbnails(part)
            .into_iter()
            .find(|t| cells.get(t.cell).and_then(|cell| cell.intersection(t.slot)).is_some_and(|shown| shown.to_f64().contains(local)))
            .map(|t| t.id)
    }

    /// Thumbnail cells in output-local logical pixels, in the order of `numbers`.
    fn overview_cells(&self, part: &Part) -> Vec<Rectangle<i32, Logical>> {
        let Some(monitor) = self.desktop.desk.monitors.get(part.monitor) else { return Vec::new() };
        let (w, h) = (f64::from(monitor.area.width), f64::from(monitor.area.height));
        let n = part.numbers.len().max(1);
        let columns = n.min(3);
        let rows = n.div_ceil(columns);
        let margin = 0.08 * w.min(h);
        let gap = 0.03 * w.min(h);
        // The largest cell with the monitor's aspect that fits the grid.
        let fit_w = (w - 2.0 * margin - gap * (columns as f64 - 1.0)) / columns as f64;
        let fit_h = (h - 2.0 * margin - gap * (rows as f64 - 1.0)) / rows as f64;
        let scale = (fit_w / w).min(fit_h / h);
        let (cell_w, cell_h) = (w * scale, h * scale);
        let grid_w = cell_w * columns as f64 + gap * (columns as f64 - 1.0);
        let grid_h = cell_h * rows as f64 + gap * (rows as f64 - 1.0);
        let (x0, y0) = ((w - grid_w) / 2.0, (h - grid_h) / 2.0);
        (0..part.numbers.len())
            .map(|i| {
                let (column, row) = (i % columns, i / columns);
                Rectangle::new(
                    (
                        (x0 + column as f64 * (cell_w + gap)).round() as i32,
                        (y0 + row as f64 * (cell_h + gap)).round() as i32,
                    )
                        .into(),
                    (cell_w.round() as i32, cell_h.round() as i32).into(),
                )
            })
            .collect()
    }

    fn overview_cell_at(&self, part: &Part, point: Point<f64, Logical>) -> Option<usize> {
        let geo = self.space.output_geometry(&part.output)?;
        let local = point - geo.loc.to_f64();
        self.overview_cells(part).iter().position(|cell| cell.to_f64().contains(local))
    }

    pub fn overview_moved(&mut self) {
        let Some(index) = self.part_under_pointer() else { return };
        let Some(overview) = &self.overview else { return };
        if let Some(cell) = self.overview_cell_at(&overview.parts[index], self.pointer_location)
            && let Some(part) = self.overview.as_mut().map(|o| &mut o.parts[index])
            && part.selected != cell
        {
            part.selected = cell;
            self.queue_redraw_all();
        }
    }

    /// Keys act on the part under the pointer (the only one outside sharing).
    pub fn overview_key(&mut self, key: ModalKey) {
        let index = self.part_under_pointer().unwrap_or(0);
        let Some(overview) = &mut self.overview else { return };
        let sharing = overview.share.is_some();
        let Some(part) = overview.parts.get_mut(index) else { return };
        let n = part.numbers.len();
        let columns = n.clamp(1, 3);
        let selected = part.selected;
        part.selected = match key {
            ModalKey::Left => selected.saturating_sub(1),
            ModalKey::Right => (selected + 1).min(n - 1),
            ModalKey::Up => selected.saturating_sub(columns),
            ModalKey::Down => (selected + columns).min(n - 1),
            ModalKey::Cancel => return self.close_overview(),
            ModalKey::Confirm => return self.activate_overview_selection(index),
            // Picking what to share: a stream whose content the cast keys switch.
            ModalKey::Dynamic if sharing => return self.end_overview(Chosen::Dynamic),
            ModalKey::Dynamic | ModalKey::Ignore => selected,
        };
        self.queue_redraw_all();
    }

    pub fn overview_button(&mut self, button: u32, state: ButtonState) {
        if state != ButtonState::Pressed || self.overview.is_none() {
            return;
        }
        // A click on a monitor the overview is not on closes it.
        let Some(index) = self.part_under_pointer() else { return self.close_overview() };
        let Some(overview) = &self.overview else { return };
        let part = &overview.parts[index];
        if button == BTN_LEFT
            && let Some(id) = self.overview_window_at(part, self.pointer_location)
        {
            if overview.share.is_some() {
                return self.share_from_overview(index, Some(id));
            }
            self.overview = None;
            self.show_window(id);
            return self.queue_redraw_all();
        }
        match self.overview_cell_at(part, self.pointer_location) {
            Some(cell) if button == BTN_LEFT => {
                if let Some(part) = self.overview.as_mut().map(|o| &mut o.parts[index]) {
                    part.selected = cell;
                }
                self.activate_overview_selection(index);
            }
            _ => self.close_overview(),
        }
    }

    fn activate_overview_selection(&mut self, index: usize) {
        if self.share_kinds().is_some() {
            return self.share_from_overview(index, None);
        }
        let Some(overview) = self.overview.take() else { return };
        if let Some(part) = overview.parts.get(index)
            && let Some(number) = part.numbers.get(part.selected).copied()
        {
            self.select_workspace_on(part.monitor, number);
        }
        self.queue_redraw_all();
    }

    /// Share `window`, or without one the part's monitor, as far as the request allows.
    fn share_from_overview(&mut self, index: usize, window: Option<WindowId>) {
        let Some(overview) = &self.overview else { return };
        let Some(part) = overview.parts.get(index) else { return };
        let kinds = self.share_kinds().unwrap_or(SourceKinds::Both);
        let window = window
            .filter(|_| kinds.windows())
            .and_then(|id| self.desktop.get(id))
            .and_then(|m| m.foreign.as_ref())
            .map(|handle| Chosen::Window(handle.identifier()));
        let chosen = window.or_else(|| kinds.monitors().then(|| Chosen::Monitor(part.output.name())));
        if let Some(chosen) = chosen {
            self.end_overview(chosen);
        }
    }

    /// The overview of `output`, front to back: frames, windows, thumbnail backgrounds, dimming.
    pub fn overview_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        blanks: &mut Vec<crate::render::Blank>,
    ) -> Option<Vec<OutputElement>> {
        let overview = self.overview.as_ref()?;
        let sharing = overview.share.is_some();
        let part = overview.parts.iter().find(|part| &part.output == output)?;
        let monitor = self.desktop.desk.monitors.get(part.monitor)?;
        let scale = output.current_scale().fractional_scale();
        let cells = self.overview_cells(part);
        let size = self.capture_geometry(output)?.size;
        let full = Rectangle::<i32, Physical>::from_size(size);
        let to_phys = |r: Rectangle<i32, Logical>| {
            Rectangle::<i32, Physical>::new(
                ((f64::from(r.loc.x) * scale).round() as i32, (f64::from(r.loc.y) * scale).round() as i32).into(),
                ((f64::from(r.size.w) * scale).round() as i32, (f64::from(r.size.h) * scale).round() as i32).into(),
            )
        };
        // Cropped solids (the `Border` variant) rather than plain ones, which are the capture-exempt overlay.
        let solid = |area: Rectangle<i32, Physical>, color: [f32; 4]| {
            let element = SolidColorRenderElement::new(Id::new(), area, CommitCounter::default(), color, Kind::Unspecified);
            CropRenderElement::from_element(element, scale, full)
                .map(OutputElement::from)
        };
        // Picking what to share, the frames take the capture color.
        let accent = if sharing { self.config.appearance.capture_border.0.0 } else { self.desktop.appearance.active_border.0 };
        let surface = self.config.appearance.surface.0.0;
        let current = monitor.workspaces.current().number;

        let mut frames = Vec::new();
        let mut windows = Vec::new();
        let mut backgrounds = Vec::new();
        for (index, (number, cell)) in part.numbers.iter().zip(&cells).enumerate() {
            if monitor.workspaces.get(*number).is_none() {
                continue;
            }
            let cell_phys = to_phys(*cell);
            backgrounds.extend(solid(cell_phys, surface));
            let t = (3.0 * scale).round().max(1.0) as i32;
            let color = if index == part.selected {
                Some(([1.0, 1.0, 1.0, 1.0], t))
            } else if *number == current {
                Some((accent, t))
            } else {
                None
            };
            if let Some((color, t)) = color {
                let (x, y, w, h) = (cell_phys.loc.x, cell_phys.loc.y, cell_phys.size.w, cell_phys.size.h);
                frames.extend(solid(Rectangle::new((x - t, y - t).into(), (w + 2 * t, t).into()), color));
                frames.extend(solid(Rectangle::new((x - t, y + h).into(), (w + 2 * t, t).into()), color));
                frames.extend(solid(Rectangle::new((x - t, y).into(), (t, h).into()), color));
                frames.extend(solid(Rectangle::new((x + w, y).into(), (t, h).into()), color));
            }
        }
        for thumbnail in self.overview_thumbnails(part) {
            let Some(managed) = self.desktop.get(thumbnail.id) else { continue };
            let Some(cell) = cells.get(thumbnail.cell) else { continue };
            let (origin, k) = (thumbnail.origin, thumbnail.k);
            let geometry_offset = managed.window.geometry().loc;
            // The window is laid out at full size around its top left corner and then shrunk about that corner.
            let corner = Point::<i32, Physical>::from(((origin.0 * scale).round() as i32, (origin.1 * scale).round() as i32));
            let location = corner - geometry_offset.to_physical_precise_round(scale);
            // A window on a hidden workspace may still hold a buffer of an older size: keep it in its slot.
            let Some(clip) = to_phys(*cell).intersection(to_phys(thumbnail.slot)) else { continue };
            let start = windows.len();
            for element in AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                &managed.window,
                renderer,
                location,
                Scale::from(scale),
                1.0,
            ) {
                let shrunk = RescaleRenderElement::from_element(element, corner, k);
                if let Some(cropped) = CropRenderElement::from_element(shrunk, scale, clip) {
                    windows.push(OutputElement::from(cropped));
                }
            }
            self.note_blank(managed, &windows[start..], scale, blanks);
        }
        let mut elements = frames;
        elements.extend(windows);
        elements.extend(backgrounds);
        elements.extend(solid(full, [0.0, 0.0, 0.0, 0.6]));
        Some(elements)
    }
}
