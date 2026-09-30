//! What gets drawn on an output, independent of the backend that presents it.
use smithay::{
    backend::renderer::{
        element::{
            AsRenderElements, Kind, render_elements,
            solid::SolidColorRenderElement,
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            utils::CropRenderElement,
        },
        gles::GlesRenderer,
    },
    desktop::layer_map_for_output,
    output::Output,
    utils::{Physical, Rectangle, Scale},
    wayland::shell::wlr_layer::Layer,
};

use crate::{State, cursor::CursorElement};

render_elements! {
    pub OutputElement<=GlesRenderer>;
    Cursor=CursorElement,
    Border=CropRenderElement<SolidColorRenderElement>,
    /// Panels, popups of unmanaged windows and lock screens.
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    /// A managed window, cut off at the edge of its own output.
    Window=CropRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
}

impl State {
    /// The elements to draw on `output`, front to back.
    pub fn output_elements(&mut self, renderer: &mut GlesRenderer, output: &Output) -> Vec<OutputElement> {
        let mut elements: Vec<OutputElement> =
            self.cursor_elements(renderer, output).into_iter().map(OutputElement::from).collect();
        if self.session_lock.is_active() {
            // A locked session shows the locker's surface (or plain black) and nothing else.
            if let Some(surface) = self.lock_surface_for(output) {
                let scale = output.current_scale().fractional_scale();
                elements.extend(render_elements_from_surface_tree(
                    renderer,
                    surface.wl_surface(),
                    (0, 0),
                    scale,
                    1.0,
                    Kind::Unspecified,
                ));
            }
            return elements;
        }
        elements.extend(self.border_elements(output).into_iter().map(OutputElement::from));
        elements.extend(self.scene_elements(renderer, output));
        elements
    }

    /// Layer surfaces and windows of `output`, front to back.
    ///
    /// Windows are drawn only on the output their workspace is on and are cut off at its edge.
    /// Tiles scrolled out of view lie beyond the edge in the shared coordinate space, where the
    /// neighbouring monitor would otherwise show them.
    fn scene_elements(&self, renderer: &mut GlesRenderer, output: &Output) -> Vec<OutputElement> {
        let Some(geo) = self.space.output_geometry(output) else { return Vec::new() };
        let Some(this_monitor) = self.outputs.iter().position(|e| &e.output == output) else { return Vec::new() };
        let scale = output.current_scale().fractional_scale();
        let visible_area = Rectangle::<i32, Physical>::from_size(geo.size.to_physical_precise_round(scale));
        let map = layer_map_for_output(output);
        let layer_elements = |renderer: &mut GlesRenderer, layers: &[Layer]| -> Vec<OutputElement> {
            map.layers()
                .rev()
                .filter(|surface| layers.contains(&surface.layer()))
                .filter_map(|surface| map.layer_geometry(surface).map(|geo| (geo.loc, surface)))
                .flat_map(|(loc, surface)| {
                    AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                        surface,
                        renderer,
                        loc.to_physical_precise_round(scale),
                        Scale::from(scale),
                        1.0,
                    )
                })
                .map(OutputElement::from)
                .collect()
        };

        let mut elements = layer_elements(renderer, &[Layer::Overlay, Layer::Top]);
        for window in self.space.elements().rev() {
            if !self.space.element_bbox(window).is_some_and(|bbox| bbox.overlaps(geo)) {
                continue;
            }
            let owner = self
                .desktop
                .windows
                .iter()
                .find(|m| &m.window == window)
                .map(|m| self.monitor_of_window(m));
            // Managed windows belong to one monitor; unmanaged ones (X11 menus) show wherever they are.
            if owner.is_some_and(|monitor| monitor != Some(this_monitor)) {
                continue;
            }
            let Some(location) = self.space.element_location(window) else { continue };
            let render_location = (location - window.geometry().loc - geo.loc).to_physical_precise_round(scale);
            let surfaces = AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                window,
                renderer,
                render_location,
                Scale::from(scale),
                1.0,
            );
            if owner.is_some() {
                elements.extend(
                    surfaces
                        .into_iter()
                        .filter_map(|e| CropRenderElement::from_element(e, scale, visible_area))
                        .map(OutputElement::from),
                );
            } else {
                elements.extend(surfaces.into_iter().map(OutputElement::from));
            }
        }
        elements.extend(layer_elements(renderer, &[Layer::Bottom, Layer::Background]));
        elements
    }

    /// Background color: the palette's, or black while locked.
    pub fn clear_color(&self) -> [f32; 4] {
        if self.session_lock.is_active() { [0.0, 0.0, 0.0, 1.0] } else { self.desktop.appearance.background.0 }
    }
}
