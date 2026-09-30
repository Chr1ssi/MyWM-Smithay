//! What gets drawn on an output, independent of the backend that presents it.
use smithay::{
    backend::renderer::{
        element::{
            Kind, render_elements,
            solid::SolidColorRenderElement,
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
        },
        gles::GlesRenderer,
    },
    desktop::space::{SpaceRenderElements, space_render_elements},
    output::Output,
};

use crate::{State, cursor::CursorElement};

render_elements! {
    pub OutputElement<=GlesRenderer>;
    Cursor=CursorElement,
    Border=SolidColorRenderElement,
    Lock=WaylandSurfaceRenderElement<GlesRenderer>,
    Space=SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
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
        match space_render_elements(renderer, [&self.space], output, 1.0) {
            Ok(space) => elements.extend(space.into_iter().map(OutputElement::from)),
            Err(error) => tracing::warn!("{}: {error:?}", output.name()),
        }
        elements
    }

    /// Background color: the palette's, or black while locked.
    pub fn clear_color(&self) -> [f32; 4] {
        if self.session_lock.is_active() { [0.0, 0.0, 0.0, 1.0] } else { self.desktop.appearance.background.0 }
    }
}
