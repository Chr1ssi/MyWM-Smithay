//! Workspace overview: every workspace of every monitor as a live thumbnail, plus a cell for a new workspace (`+`)
//! and one for the scratchpad.
//!
//! Click a cell (or pick it with the arrow keys and Enter, or type its workspace number) to go there, or a window
//! in it to show that window. Drag a window onto another cell, also on another monitor, to move it there: onto a
//! workspace, onto `+` for a new one, onto the scratchpad. `q` or a middle click closes the window under the
//! pointer. Escape or a click beside the cells closes the overview. Windows are drawn scaled, straight from their
//! surfaces; opening and closing zoom out of and back into the workspace each monitor shows. When an app asks what
//! to share, the overview is how the user picks: a window's thumbnail shares the window, the rest of a cell (or
//! Enter) its monitor, `d` a dynamic cast; closing it in any way answers that nothing was picked.
use std::time::{Duration, Instant};

use splice_ipc::{Chosen, SourceKinds};
use splice_layout::{Rect, Workspace, arrange, place_floating};
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

use crate::{State, desktop::{Managed, WindowId, ease_out}, render::{Blank, OutputElement}, screenshot::ModalKey};

const BTN_LEFT: u32 = 0x110;
const BTN_MIDDLE: u32 = 0x112;
/// How far (logical pixels) a held window has to move before it is dragged rather than clicked.
const DRAG_THRESHOLD: f64 = 6.0;

pub struct Overview {
    /// One per monitor.
    parts: Vec<Part>,
    /// The app (bar-socket client) that asked what to share, and what it may share, while picking.
    share: Option<(u64, SourceKinds)>,
    opened: Instant,
    /// Since when it zooms back into the workspaces; it takes no input any more and is gone at the end.
    closing: Option<Instant>,
    /// A window held with the left button.
    held: Option<Held>,
}

/// The overview on one monitor.
struct Part {
    output: Output,
    /// Index into the part's cells; marked while the pointer is on this monitor.
    selected: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cell {
    Workspace(usize),
    /// Go to, or drop a window on, a new workspace of this monitor.
    New,
    Scratchpad,
}

/// A window pressed on: a click shows it, a drag (beyond `DRAG_THRESHOLD`) carries it to another cell.
struct Held {
    id: WindowId,
    press: Point<f64, Logical>,
    /// Where the pointer holds the thumbnail, from its top left corner.
    grab: Point<f64, Logical>,
    k: f64,
    moved: bool,
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

impl Overview {
    /// How far open, from 0 (the plain desktop) to 1, eased; closing plays opening backwards.
    fn openness(&self, ms: u32) -> f64 {
        if ms == 0 {
            return if self.closing.is_some() { 0.0 } else { 1.0 };
        }
        let progress = |since: Instant| (since.elapsed().as_secs_f64() * 1000.0 / f64::from(ms)).min(1.0);
        ease_out(match self.closing {
            Some(since) => 1.0 - progress(since),
            None => progress(self.opened),
        })
    }

    /// Opening or closing is still under way.
    pub fn animating(&self, ms: u32) -> bool {
        let length = Duration::from_millis(ms.into());
        ms > 0 && self.closing.unwrap_or(self.opened).elapsed() < length
    }
}

impl State {
    pub fn toggle_overview(&mut self) {
        if self.overview_open() {
            return self.close_overview();
        }
        self.open_overview(None);
    }

    /// The overview is up and takes the input (not when it is closing).
    pub fn overview_open(&self) -> bool {
        self.overview.as_ref().is_some_and(|o| o.closing.is_none())
    }

    /// Show the overview on every monitor; with `share`, to pick what to share.
    pub fn open_overview(&mut self, share: Option<(u64, SourceKinds)>) -> bool {
        if self.session_lock.is_active() || self.overview_open() || self.outputs.is_empty() {
            return false;
        }
        let parts = self.outputs.iter().map(|e| Part { output: e.output.clone(), selected: 0 }).collect();
        self.overview = Some(Overview { parts, share, opened: Instant::now(), closing: None, held: None });
        // Each monitor starts at the workspace it shows.
        for index in 0..self.outputs.len() {
            let Some(part) = self.overview.as_ref().and_then(|o| o.parts.get(index)) else { continue };
            let current = self.part_monitor(part).map(|m| self.desktop.desk.monitors[m].workspaces.current().number);
            let selected = self.overview_cell_kinds(part).iter().position(|c| Some(*c) == current.map(Cell::Workspace));
            if let Some(part) = self.overview.as_mut().and_then(|o| o.parts.get_mut(index)) {
                part.selected = selected.unwrap_or(0);
            }
        }
        // The bar says how to pick.
        self.ipc_dirty |= share.is_some();
        self.queue_redraw_all();
        true
    }

    /// Close the overview; one that was picking what to share answers that nothing was picked.
    pub fn close_overview(&mut self) {
        self.end_overview(Chosen::Nothing);
    }

    /// Close the overview, answering an app that asked what to share with `chosen`. It zooms back into the
    /// workspaces, which may change right after (the one picked is shown).
    fn end_overview(&mut self, chosen: Chosen) {
        let ms = self.config.effects.animation_ms;
        let Some(overview) = self.overview.as_mut().filter(|o| o.closing.is_none()) else { return };
        // Closing while still opening goes back from where it got to.
        let length = Duration::from_millis(ms.into());
        let shown = overview.opened.elapsed().min(length);
        overview.closing = Some(Instant::now() - (length - shown));
        overview.held = None;
        if let Some((client, _)) = overview.share {
            self.ipc_send_chosen(client, &chosen);
            self.ipc_dirty = true;
        }
        if ms == 0 {
            self.overview = None;
        }
        self.refresh_pointer_focus();
        self.queue_redraw_all();
    }

    /// Forget the overview once it has closed.
    pub fn overview_tick(&mut self) {
        let ms = self.config.effects.animation_ms;
        if self.overview.as_ref().is_some_and(|o| o.closing.is_some() && !o.animating(ms)) {
            self.overview = None;
        }
    }

    /// What the app picking a source may share, while the overview picks.
    pub fn share_kinds(&self) -> Option<SourceKinds> {
        self.overview.as_ref().filter(|o| o.closing.is_none())?.share.map(|(_, kinds)| kinds)
    }

    fn part_monitor(&self, part: &Part) -> Option<usize> {
        self.outputs.iter().position(|e| e.output == part.output)
    }

    /// The overview's part under the pointer.
    fn part_under_pointer(&self) -> Option<usize> {
        let output = self.space.output_under(self.pointer_location).next()?;
        self.overview.as_ref()?.parts.iter().position(|part| &part.output == output)
    }

    /// What a part's cells show, in display order: its workspaces, then `+` and the scratchpad (not when
    /// picking what to share).
    fn overview_cell_kinds(&self, part: &Part) -> Vec<Cell> {
        let Some(monitor) = self.part_monitor(part).and_then(|m| self.desktop.desk.monitors.get(m)) else { return Vec::new() };
        let mut cells: Vec<Cell> = monitor.workspaces.numbers().map(Cell::Workspace).collect();
        if self.overview.as_ref().is_some_and(|o| o.share.is_none()) {
            cells.extend([Cell::New, Cell::Scratchpad]);
        }
        cells
    }

    /// Every window a part shows, front to back within each cell.
    fn overview_thumbnails(&self, part: &Part) -> Vec<Thumbnail> {
        let Some(monitor) = self.part_monitor(part).and_then(|m| self.desktop.desk.monitors.get(m)) else { return Vec::new() };
        let (area, usable) = (monitor.area, monitor.usable);
        let d = &self.desktop;
        let infos: Vec<_> = d.windows.iter().filter(|m| m.placed && !m.awaiting_size).map(|m| m.info()).collect();
        let mut thumbnails = Vec::new();
        for (cell_index, (kind, cell)) in self.overview_cell_kinds(part).into_iter().zip(self.overview_cells(part)).enumerate() {
            let placements = match kind {
                Cell::Workspace(number) => {
                    let Some(workspace) = monitor.workspaces.get(number) else { continue };
                    // Lay the workspace out as it would be shown (on a copy: arranging scrolls it).
                    let mut copy = Workspace { number: workspace.number, kind: workspace.kind, windows: workspace.windows.clone(), focused: workspace.focused, scroll: workspace.scroll };
                    arrange(&mut copy, &infos, usable, area, &d.appearance)
                }
                // The scratchpad as it would appear on this monitor.
                Cell::Scratchpad => d
                    .scratchpad
                    .windows
                    .iter()
                    .filter_map(|id| infos.iter().find(|i| i.id == *id))
                    .map(|info| place_floating(info, usable, &d.appearance))
                    .collect(),
                Cell::New => continue,
            };
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
    fn overview_window_at(&self, part: &Part, point: Point<f64, Logical>) -> Option<Thumbnail> {
        let geo = self.space.output_geometry(&part.output)?;
        let local = point - geo.loc.to_f64();
        let cells = self.overview_cells(part);
        self.overview_thumbnails(part)
            .into_iter()
            .find(|t| cells.get(t.cell).and_then(|cell| cell.intersection(t.slot)).is_some_and(|shown| shown.to_f64().contains(local)))
    }

    /// Cells in output-local logical pixels, in the order of `overview_cell_kinds`. While the overview opens or
    /// closes they are zoomed about the monitor's shown workspace, so that its cell covers the monitor at first.
    fn overview_cells(&self, part: &Part) -> Vec<Rectangle<i32, Logical>> {
        let Some(monitor) = self.part_monitor(part).and_then(|m| self.desktop.desk.monitors.get(m)) else { return Vec::new() };
        let kinds = self.overview_cell_kinds(part);
        let (w, h) = (f64::from(monitor.area.width), f64::from(monitor.area.height));
        let n = kinds.len().max(1);
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
        let corner = |i: usize| (x0 + (i % columns) as f64 * (cell_w + gap), y0 + (i / columns) as f64 * (cell_h + gap));
        let open = self.overview.as_ref().map_or(1.0, |o| o.openness(self.config.effects.animation_ms));
        let current = Cell::Workspace(monitor.workspaces.current().number);
        // Scaled by `zoom` about the origin, then moved by `shift`: at first the shown workspace's cell is the monitor.
        let (zoom, shift) = match kinds.iter().position(|c| *c == current) {
            Some(anchor) if open < 1.0 => {
                let full = 1.0 / scale;
                let (ax, ay) = corner(anchor);
                (full + (1.0 - full) * open, (-ax * full * (1.0 - open), -ay * full * (1.0 - open)))
            }
            _ => (1.0, (0.0, 0.0)),
        };
        (0..kinds.len())
            .map(|i| {
                let (x, y) = corner(i);
                Rectangle::new(
                    ((x * zoom + shift.0).round() as i32, (y * zoom + shift.1).round() as i32).into(),
                    ((cell_w * zoom).round() as i32, (cell_h * zoom).round() as i32).into(),
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
        let pointer = self.pointer_location;
        if let Some(held) = self.overview.as_mut().and_then(|o| o.held.as_mut()) {
            let distance = pointer - held.press;
            held.moved |= distance.x.hypot(distance.y) >= DRAG_THRESHOLD;
        }
        let Some(index) = self.part_under_pointer() else { return };
        let Some(overview) = &self.overview else { return };
        if let Some(cell) = self.overview_cell_at(&overview.parts[index], pointer)
            && let Some(part) = self.overview.as_mut().map(|o| &mut o.parts[index])
            && part.selected != cell
        {
            part.selected = cell;
            self.queue_redraw_all();
        }
    }

    /// Keys act on the part under the pointer.
    pub fn overview_key(&mut self, key: ModalKey) {
        let index = self.part_under_pointer().unwrap_or(0);
        let sharing = self.share_kinds().is_some();
        match key {
            ModalKey::Cancel => return self.close_overview(),
            ModalKey::Confirm => return self.activate_overview_selection(index),
            // Picking what to share: a stream whose content the cast keys switch.
            ModalKey::Dynamic if sharing => return self.end_overview(Chosen::Dynamic),
            ModalKey::Digit(number) if !sharing => {
                if self.desktop.desk.owner(number).is_some() {
                    self.end_overview(Chosen::Nothing);
                    self.select_workspace(number);
                }
                return;
            }
            ModalKey::Close if !sharing => {
                if let Some(id) = self.overview_window_under_pointer()
                    && let Some(m) = self.desktop.get(id)
                {
                    m.close();
                }
                return;
            }
            _ => {}
        }
        let Some(part) = self.overview.as_ref().and_then(|o| o.parts.get(index)) else { return };
        let n = self.overview_cell_kinds(part).len();
        let Some(part) = self.overview.as_mut().and_then(|o| o.parts.get_mut(index)) else { return };
        if n == 0 {
            return;
        }
        let columns = n.min(3);
        let selected = part.selected.min(n - 1);
        part.selected = match key {
            ModalKey::Left => selected.saturating_sub(1),
            ModalKey::Right => (selected + 1).min(n - 1),
            ModalKey::Up => selected.saturating_sub(columns),
            ModalKey::Down => (selected + columns).min(n - 1),
            _ => selected,
        };
        self.queue_redraw_all();
    }

    fn overview_window_under_pointer(&self) -> Option<WindowId> {
        let part = self.overview.as_ref()?.parts.get(self.part_under_pointer()?)?;
        self.overview_window_at(part, self.pointer_location).map(|t| t.id)
    }

    pub fn overview_button(&mut self, button: u32, state: ButtonState) {
        if !self.overview_open() {
            return;
        }
        if state == ButtonState::Released {
            if button == BTN_LEFT
                && let Some(held) = self.overview.as_mut().and_then(|o| o.held.take())
            {
                if held.moved {
                    self.drop_on_overview(held.id);
                } else {
                    self.end_overview(Chosen::Nothing);
                    self.show_window(held.id);
                }
            }
            return;
        }
        // A click on a monitor the overview is not on (plugged in meanwhile) closes it.
        let Some(index) = self.part_under_pointer() else { return self.close_overview() };
        let sharing = self.share_kinds().is_some();
        let Some(part) = self.overview.as_ref().map(|o| &o.parts[index]) else { return };
        let geo = self.space.output_geometry(&part.output).unwrap_or_default();
        match (button, self.overview_window_at(part, self.pointer_location)) {
            (BTN_LEFT, Some(t)) if sharing => return self.share_from_overview(index, Some(t.id)),
            (BTN_LEFT, Some(t)) => {
                let corner = geo.loc.to_f64() + Point::from(t.origin);
                let press = self.pointer_location;
                if let Some(overview) = &mut self.overview {
                    overview.held = Some(Held { id: t.id, press, grab: press - corner, k: t.k, moved: false });
                }
                return;
            }
            (BTN_MIDDLE, Some(t)) if !sharing => {
                if let Some(m) = self.desktop.get(t.id) {
                    m.close();
                }
                return;
            }
            _ => {}
        }
        match self.overview_cell_at(part, self.pointer_location) {
            Some(cell) if button == BTN_LEFT => {
                if let Some(part) = self.overview.as_mut().map(|o| &mut o.parts[index]) {
                    part.selected = cell;
                }
                self.activate_overview_selection(index);
            }
            // A middle click that missed the windows does nothing.
            Some(_) if button == BTN_MIDDLE => {}
            _ => self.close_overview(),
        }
    }

    fn activate_overview_selection(&mut self, index: usize) {
        if self.share_kinds().is_some() {
            return self.share_from_overview(index, None);
        }
        let Some(part) = self.overview.as_ref().and_then(|o| o.parts.get(index)) else { return };
        let (Some(monitor), Some(cell)) = (self.part_monitor(part), self.overview_cell_kinds(part).get(part.selected).copied()) else { return };
        self.end_overview(Chosen::Nothing);
        match cell {
            Cell::Workspace(number) => self.select_workspace_on(monitor, number),
            Cell::New => self.new_workspace_on(monitor),
            // It appears on the monitor under the pointer: this one, when clicked.
            Cell::Scratchpad if !self.desktop.scratchpad.windows.is_empty() => {
                self.desktop.scratchpad_visible = true;
                self.desktop.focus_scratch = true;
                self.refresh();
            }
            Cell::Scratchpad => {}
        }
    }

    /// Put a dragged window where it was let go: onto another workspace (on any monitor), a new one, or into the
    /// scratchpad. The overview stays open.
    fn drop_on_overview(&mut self, id: WindowId) {
        self.queue_redraw_all();
        // It may have closed while it was held.
        if self.desktop.get(id).is_none() {
            return;
        }
        let Some(part) = self.part_under_pointer().and_then(|index| self.overview.as_ref()?.parts.get(index)) else { return };
        let Some(monitor) = self.part_monitor(part) else { return };
        let Some(cell) = self.overview_cell_at(part, self.pointer_location).and_then(|i| self.overview_cell_kinds(part).get(i).copied()) else { return };
        match cell {
            Cell::Workspace(number) if self.desktop.desk.locate(&id).map(|(_, n)| n) != Some(number) => {
                self.move_window_to_workspace(id, number);
            }
            Cell::New => {
                if let Some(number) = self.desktop.desk.create(monitor) {
                    self.move_window_to_workspace(id, number);
                }
            }
            Cell::Scratchpad if !self.desktop.in_scratchpad(id) => {
                self.put_in_scratchpad(id, false);
                self.refresh();
            }
            Cell::Workspace(_) | Cell::Scratchpad => {}
        }
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

    /// A window's picture with its top left corner at `origin` (output-local logical), shrunk by `k` and cut to `clip`.
    fn thumbnail_elements(
        &self,
        renderer: &mut GlesRenderer,
        managed: &Managed,
        (origin, k): ((f64, f64), f64),
        clip: Rectangle<i32, Physical>,
        scale: f64,
    ) -> Vec<OutputElement> {
        // The window is laid out at full size around its top left corner and then shrunk about that corner.
        let corner = Point::<i32, Physical>::from(((origin.0 * scale).round() as i32, (origin.1 * scale).round() as i32));
        let location = corner - managed.window.geometry().loc.to_physical_precise_round(scale);
        AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
            &managed.window,
            renderer,
            location,
            Scale::from(scale),
            1.0,
        )
        .into_iter()
        .filter_map(|element| CropRenderElement::from_element(RescaleRenderElement::from_element(element, corner, k), scale, clip))
        .map(OutputElement::from)
        .collect()
    }

    /// The overview of `output`, front to back: a dragged window, frames, `+`, windows, cell backgrounds, dimming.
    pub fn overview_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        blanks: &mut Vec<Blank>,
    ) -> Option<Vec<OutputElement>> {
        let overview = self.overview.as_ref()?;
        let index = overview.parts.iter().position(|part| &part.output == output)?;
        let part = &overview.parts[index];
        let monitor = self.desktop.desk.monitors.get(self.part_monitor(part)?)?;
        let open = overview.openness(self.config.effects.animation_ms) as f32;
        // Only the monitor under the pointer marks a cell: the one keys and clicks act on.
        let marked = (overview.closing.is_none() && self.part_under_pointer() == Some(index)).then_some(part.selected);
        let dragged = overview.held.as_ref().filter(|held| held.moved);
        let scale = output.current_scale().fractional_scale();
        let kinds = self.overview_cell_kinds(part);
        let cells = self.overview_cells(part);
        let geo = self.space.output_geometry(output)?;
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
        // Colors are premultiplied: fading scales every channel.
        let faded = |color: [f32; 4], alpha: f32| color.map(|channel| channel * alpha);
        let appearance = &self.config.appearance;
        // Picking what to share, the frames take the capture color.
        let accent = if overview.share.is_some() { appearance.capture_border.0.0 } else { self.desktop.appearance.active_border.0 };
        let current = Cell::Workspace(monitor.workspaces.current().number);
        let thumbnails = self.overview_thumbnails(part);

        let mut frames = Vec::new();
        let mut marks = Vec::new();
        let mut windows = Vec::new();
        let mut backgrounds = Vec::new();
        for (cell_index, (kind, cell)) in kinds.iter().zip(&cells).enumerate() {
            let cell_phys = to_phys(*cell);
            let (x, y, w, h) = (cell_phys.loc.x, cell_phys.loc.y, cell_phys.size.w, cell_phys.size.h);
            // The cells grow out of the desktop: at first only the windows show.
            let background = if *kind == Cell::New { 0.5 * open } else { open };
            backgrounds.extend(solid(cell_phys, faded(appearance.surface.0.0, background)));
            if *kind == Cell::New {
                let (arm, t) = ((w.min(h) / 8).max(4), (w.min(h) / 40).max(2));
                let (cx, cy) = (x + w / 2, y + h / 2);
                let color = faded(appearance.muted_text.0.0, open);
                marks.extend(solid(Rectangle::new((cx - arm, cy - t / 2).into(), (2 * arm, t).into()), color));
                marks.extend(solid(Rectangle::new((cx - t / 2, cy - arm).into(), (t, 2 * arm).into()), color));
            }
            let urgent = thumbnails.iter().any(|t| t.cell == cell_index && self.desktop.get(t.id).is_some_and(|m| m.urgent));
            let color = if marked == Some(cell_index) {
                Some([1.0, 1.0, 1.0, 1.0])
            } else if urgent {
                Some(appearance.urgent_border.0.0)
            } else if *kind == current {
                Some(accent)
            } else if *kind == Cell::Scratchpad {
                Some(self.desktop.appearance.inactive_border.0)
            } else {
                None
            };
            if let Some(color) = color {
                let t = (3.0 * scale).round().max(1.0) as i32;
                frames.extend(solid(Rectangle::new((x - t, y - t).into(), (w + 2 * t, t).into()), color));
                frames.extend(solid(Rectangle::new((x - t, y + h).into(), (w + 2 * t, t).into()), color));
                frames.extend(solid(Rectangle::new((x - t, y).into(), (t, h).into()), color));
                frames.extend(solid(Rectangle::new((x + w, y).into(), (t, h).into()), color));
            }
        }
        for thumbnail in &thumbnails {
            if dragged.is_some_and(|held| held.id == thumbnail.id) {
                continue;
            }
            let Some(managed) = self.desktop.get(thumbnail.id) else { continue };
            let Some(cell) = cells.get(thumbnail.cell) else { continue };
            // A window on a hidden workspace may still hold a buffer of an older size: keep it in its slot.
            let Some(clip) = to_phys(*cell).intersection(to_phys(thumbnail.slot)) else { continue };
            let start = windows.len();
            windows.extend(self.thumbnail_elements(renderer, managed, (thumbnail.origin, thumbnail.k), clip, scale));
            self.note_blank(managed, &windows[start..], scale, blanks);
        }
        // A dragged window follows the pointer over every monitor.
        let mut elements = Vec::new();
        if let Some(held) = dragged
            && let Some(managed) = self.desktop.get(held.id)
        {
            let corner = self.pointer_location - held.grab - geo.loc.to_f64();
            elements = self.thumbnail_elements(renderer, managed, ((corner.x, corner.y), held.k), full, scale);
            self.note_blank(managed, &elements, scale, blanks);
        }
        elements.extend(frames);
        elements.extend(marks);
        elements.extend(windows);
        elements.extend(backgrounds);
        elements.extend(solid(full, [0.0, 0.0, 0.0, 0.6 * open]));
        Some(elements)
    }
}
