//! What gets drawn on an output, independent of the backend that presents it.
use smithay::{
    backend::renderer::{
        element::{
            AsRenderElements, Element, Id, Kind, render_elements,
            solid::SolidColorRenderElement,
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            utils::CropRenderElement,
        },
        gles::{GlesRenderer, element::PixelShaderElement},
    },
    desktop::layer_map_for_output,
    output::Output,
    utils::{Physical, Rectangle, Scale},
    wayland::shell::wlr_layer::Layer,
};

use smithay::wayland::seat::WaylandFocus;

use crate::{State, cursor::CursorElement};

render_elements! {
    pub OutputElement<=GlesRenderer>;
    Cursor=CursorElement,
    Border=CropRenderElement<SolidColorRenderElement>,
    /// The rounded border ring of a window.
    Ring=CropRenderElement<PixelShaderElement>,
    /// A managed window with rounded corners.
    Rounded=crate::effects::Rounded<CropRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>>,
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
        let borders = self.border_elements(renderer, output);
        elements.extend(borders);
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
            let managed = self.desktop.windows.iter().find(|m| &m.window == window);
            let owner = managed.map(|m| self.monitor_of_window(m));
            // Managed windows belong to one monitor; unmanaged ones (X11 menus) show wherever they are.
            if owner.is_some_and(|monitor| monitor != Some(this_monitor)) {
                continue;
            }
            let Some(location) = self.space.element_location(window) else { continue };
            let render_location = (location - window.geometry().loc - geo.loc).to_physical_precise_round(scale);
            // Opacity: the window's rule, dimmed further while it has no focus. Not in fullscreen.
            let alpha = managed.filter(|m| !m.fullscreen).map_or(1.0, |m| {
                let rule = mywm_config::opacity(&self.config.rules, m.app_id.as_deref(), m.parent.is_some());
                let focus = if self.desktop.focused() == Some(m.id) { 1.0 } else { self.config.effects.inactive_opacity };
                rule.unwrap_or(1.0) * focus
            });
            let surfaces = AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                window,
                renderer,
                render_location,
                Scale::from(scale),
                alpha,
            );
            if owner.is_some() {
                let rounded = managed.is_some_and(|m| !m.fullscreen) && self.config.effects.corner_radius > 0;
                let main = window.wl_surface().map(|s| Id::from_wayland_resource(&*s));
                let geometry = window.geometry();
                for element in surfaces {
                    let shader = self.effect_shaders.as_ref().filter(|_| rounded && Some(element.id()) == main.as_ref());
                    // Pixels of the surface's buffer per physical pixel on screen.
                    let (buf, dst) = (element.src().size, element.geometry(Scale::from(scale)).size);
                    let factor = (buf.w / f64::from(dst.w.max(1)), buf.h / f64::from(dst.h.max(1)));
                    let Some(cropped) = CropRenderElement::from_element(element, scale, visible_area) else { continue };
                    match shader {
                        Some(shader) => {
                            let geo_px = [
                                (f64::from(geometry.loc.x) * scale * factor.0) as f32,
                                (f64::from(geometry.loc.y) * scale * factor.1) as f32,
                                (f64::from(geometry.size.w) * scale * factor.0) as f32,
                                (f64::from(geometry.size.h) * scale * factor.1) as f32,
                            ];
                            let radius = (f64::from(self.config.effects.corner_radius) * scale * factor.0) as f32;
                            elements.push(OutputElement::from(crate::effects::Rounded::new(
                                cropped,
                                shader.tex.clone(),
                                (buf.w as f32, buf.h as f32),
                                geo_px,
                                radius,
                            )));
                        }
                        None => elements.push(OutputElement::from(cropped)),
                    }
                }
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
