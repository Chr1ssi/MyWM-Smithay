//! Workspace overview: every workspace of a monitor as a live thumbnail.
//!
//! Click one (or pick it with the arrow keys and Enter) to go there; Escape or a click beside
//! the thumbnails closes the overview. Windows are drawn scaled, straight from their surfaces.
use mywm_layout::{Rect, Workspace, arrange};
use smithay::{
    backend::{
        input::ButtonState,
        renderer::{
            element::{AsRenderElements, Id, Kind, solid::SolidColorRenderElement, surface::WaylandSurfaceRenderElement},
            gles::GlesRenderer,
            utils::CommitCounter,
        },
    },
    output::Output,
    utils::{Logical, Physical, Point, Rectangle, Scale},
};

use crate::{State, render::OutputElement, screenshot::ModalKey};

const BTN_LEFT: u32 = 0x110;

pub struct Overview {
    output: Output,
    monitor: usize,
    /// Workspace numbers in display order.
    numbers: Vec<usize>,
    selected: usize,
}

impl State {
    pub fn toggle_overview(&mut self) {
        if self.overview.is_some() {
            return self.close_overview();
        }
        if self.session_lock.is_active() {
            return;
        }
        let monitor = self.pointer_monitor().unwrap_or(self.desktop.focused_monitor);
        let Some(output) = self.outputs.get(monitor).map(|e| e.output.clone()) else { return };
        let workspaces = &self.desktop.desk.monitors[monitor].workspaces;
        let numbers: Vec<usize> = workspaces.numbers().collect();
        let selected = numbers.iter().position(|n| *n == workspaces.current().number).unwrap_or(0);
        self.overview = Some(Overview { output, monitor, numbers, selected });
        self.queue_redraw_all();
    }

    pub fn close_overview(&mut self) {
        if self.overview.take().is_some() {
            self.queue_redraw_all();
        }
    }

    /// Thumbnail cells in output-local logical pixels, in the order of `numbers`.
    fn overview_cells(&self, overview: &Overview) -> Vec<Rectangle<i32, Logical>> {
        let Some(monitor) = self.desktop.desk.monitors.get(overview.monitor) else { return Vec::new() };
        let (w, h) = (f64::from(monitor.area.width), f64::from(monitor.area.height));
        let n = overview.numbers.len().max(1);
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
        (0..overview.numbers.len())
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

    fn overview_cell_at(&self, overview: &Overview, point: Point<f64, Logical>) -> Option<usize> {
        let geo = self.space.output_geometry(&overview.output)?;
        let local = point - geo.loc.to_f64();
        self.overview_cells(overview).iter().position(|cell| cell.to_f64().contains(local))
    }

    pub fn overview_moved(&mut self) {
        let Some(overview) = &self.overview else { return };
        if overview.output != *self.space.output_under(self.pointer_location).next().unwrap_or(&overview.output) {
            return;
        }
        if let Some(cell) = self.overview_cell_at(overview, self.pointer_location)
            && let Some(overview) = &mut self.overview
            && overview.selected != cell
        {
            overview.selected = cell;
            self.queue_redraw_all();
        }
    }

    pub fn overview_key(&mut self, key: ModalKey) {
        let Some(overview) = &mut self.overview else { return };
        let n = overview.numbers.len();
        let columns = n.clamp(1, 3);
        let selected = overview.selected;
        overview.selected = match key {
            ModalKey::Left => selected.saturating_sub(1),
            ModalKey::Right => (selected + 1).min(n - 1),
            ModalKey::Up => selected.saturating_sub(columns),
            ModalKey::Down => (selected + columns).min(n - 1),
            ModalKey::Cancel => return self.close_overview(),
            ModalKey::Confirm => return self.activate_overview_selection(),
            ModalKey::Ignore => selected,
        };
        self.queue_redraw_all();
    }

    pub fn overview_button(&mut self, button: u32, state: ButtonState) {
        if state != ButtonState::Pressed {
            return;
        }
        let Some(overview) = &self.overview else { return };
        match self.overview_cell_at(overview, self.pointer_location) {
            Some(cell) if button == BTN_LEFT => {
                if let Some(overview) = &mut self.overview {
                    overview.selected = cell;
                }
                self.activate_overview_selection();
            }
            _ => self.close_overview(),
        }
    }

    fn activate_overview_selection(&mut self) {
        let Some(overview) = self.overview.take() else { return };
        if let Some(number) = overview.numbers.get(overview.selected).copied() {
            self.select_workspace_on(overview.monitor, number);
        }
        self.queue_redraw_all();
    }

    /// The overview of `output`, front to back: frames, windows, thumbnail backgrounds, dimming.
    pub fn overview_elements(&mut self, renderer: &mut GlesRenderer, output: &Output) -> Option<Vec<OutputElement>> {
        let overview = self.overview.as_ref().filter(|o| &o.output == output)?;
        let monitor = self.desktop.desk.monitors.get(overview.monitor)?;
        let area = monitor.area;
        let usable = monitor.usable;
        let scale = output.current_scale().fractional_scale();
        let cells = self.overview_cells(overview);
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
            smithay::backend::renderer::element::utils::CropRenderElement::from_element(element, scale, full)
                .map(OutputElement::from)
        };
        let accent = self.desktop.appearance.active_border.0;
        let surface = self.config.appearance.surface.0.0;
        let current = monitor.workspaces.current().number;
        let appearance = &self.desktop.appearance;

        let infos: Vec<_> = self.desktop.windows.iter().filter(|m| m.placed).map(|m| m.info()).collect();
        let mut frames = Vec::new();
        let mut windows = Vec::new();
        let mut backgrounds = Vec::new();
        for (index, (number, cell)) in overview.numbers.iter().zip(&cells).enumerate() {
            let Some(workspace) = monitor.workspaces.get(*number) else { continue };
            let cell_phys = to_phys(*cell);
            backgrounds.extend(solid(cell_phys, surface));
            let t = (3.0 * scale).round().max(1.0) as i32;
            let color = if index == overview.selected {
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
            // Lay the workspace out as it would be shown (on a copy: arranging scrolls it).
            let mut copy = Workspace { number: workspace.number, kind: workspace.kind, windows: workspace.windows.clone(), focused: workspace.focused, scroll: workspace.scroll };
            let placements = arrange(&mut copy, &infos, usable, area, appearance);
            let k = f64::from(cell.size.w) / f64::from(area.width.max(1));
            // Placements come back in paint order (back to front); we list front to back.
            for placement in placements.iter().rev() {
                let Some(managed) = self.desktop.get(placement.id) else { continue };
                let Rect { x, y, .. } = placement.content;
                let origin = (
                    f64::from(cell.loc.x) + f64::from(x - area.x) * k,
                    f64::from(cell.loc.y) + f64::from(y - area.y) * k,
                );
                let geometry_offset = managed.window.geometry().loc;
                let location = Point::<i32, Physical>::from((
                    ((origin.0 - f64::from(geometry_offset.x) * k) * scale).round() as i32,
                    ((origin.1 - f64::from(geometry_offset.y) * k) * scale).round() as i32,
                ));
                let clip = cell_phys;
                for element in AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                    &managed.window,
                    renderer,
                    location,
                    Scale::from(scale * k),
                    1.0,
                ) {
                    if let Some(cropped) = smithay::backend::renderer::element::utils::CropRenderElement::from_element(element, scale, clip) {
                        windows.push(OutputElement::from(cropped));
                    }
                }
            }
        }
        let mut elements = frames;
        elements.extend(windows);
        elements.extend(backgrounds);
        elements.extend(solid(full, [0.0, 0.0, 0.0, 0.6]));
        Some(elements)
    }
}
