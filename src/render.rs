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
        utils::CommitCounter,
    },
    desktop::layer_map_for_output,
    output::Output,
    utils::{Physical, Rectangle, Scale},
    wayland::shell::wlr_layer::Layer,
};

use smithay::wayland::seat::WaylandFocus;

use crate::{State, cursor::CursorElement, desktop::Managed};

/// A window kept out of captures, as just drawn on an output: its elements, the area they cover (with
/// popups and menus), and the id of the black area that stands in for it.
pub struct Blank {
    ids: Vec<Id>,
    area: Rectangle<i32, Physical>,
    id: Id,
}

render_elements! {
    pub OutputElement<=GlesRenderer>;
    Cursor=CursorElement,
    Border=CropRenderElement<SolidColorRenderElement>,
    /// The rounded border ring of a window.
    Ring=CropRenderElement<PixelShaderElement>,
    /// Screenshot selection dimming; never captured.
    Overlay=SolidColorRenderElement,
    /// The part of the wallpaper on an output.
    Wallpaper=smithay::backend::renderer::element::texture::TextureRenderElement<smithay::backend::renderer::gles::GlesTexture>,
    /// The blurred wallpaper behind a translucent window.
    Backdrop=crate::effects::Rounded<CropRenderElement<smithay::backend::renderer::element::texture::TextureRenderElement<smithay::backend::renderer::gles::GlesTexture>>>,
    /// A managed window with rounded corners.
    Rounded=crate::effects::Rounded<CropRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>>,
    /// Panels, popups of unmanaged windows and lock screens.
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    /// A managed window, cut off at the edge of its own output.
    Window=CropRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    /// A window shrunk into a workspace thumbnail of the overview.
    Thumbnail=CropRenderElement<smithay::backend::renderer::element::utils::RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>>,
    /// A picture of the built-in shell (bar, power menu, notices); captures show it.
    Shell=smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement<GlesRenderer>,
    /// A leaning tile of the wallpaper picker, cut off at the sides of its carousel.
    Tile=CropRenderElement<crate::shell_wallpaper::TileElement>,
}

impl OutputElement {
    /// What the element is, for the log.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Cursor(_) => "cursor",
            Self::Border(_) => "border",
            Self::Ring(_) => "border ring",
            Self::Overlay(_) => "screenshot overlay",
            Self::Wallpaper(_) => "wallpaper",
            Self::Backdrop(_) => "blurred backdrop",
            Self::Rounded(_) => "rounded window",
            Self::Surface(_) => "surface",
            Self::Window(_) => "window",
            Self::Thumbnail(_) => "overview thumbnail",
            Self::Shell(_) => "shell",
            Self::Tile(_) => "wallpaper picker tile",
            _ => "element",
        }
    }
}

impl State {
    /// The elements to draw on `output`, front to back. Notes which of them captures must not show.
    pub fn output_elements(&mut self, renderer: &mut GlesRenderer, output: &Output) -> Vec<OutputElement> {
        self.overview_tick();
        let mut blanks = Vec::new();
        let elements = self.draw_output(renderer, output, &mut blanks);
        if let Some(entry) = self.outputs.iter_mut().find(|e| &e.output == output) {
            entry.capture_blanks = blanks;
        }
        elements
    }

    /// Note the elements just `drawn` for `m` if a rule keeps it out of captures.
    pub fn note_blank(&self, m: &Managed, drawn: &[OutputElement], scale: f64, blanks: &mut Vec<Blank>) {
        if !self.blocks_capture(m) {
            return;
        }
        let Some(area) = drawn.iter().map(|e| e.geometry(Scale::from(scale))).reduce(|a, b| a.merge(b)) else { return };
        blanks.push(Blank { ids: drawn.iter().map(|e| e.id().clone()).collect(), area, id: m.capture_blank.clone() });
    }

    /// The black areas for the windows kept out of captures of `output`, in the order `capture_view` takes.
    pub fn capture_blanks(&self, output: &Output) -> Vec<Option<OutputElement>> {
        let Some(entry) = self.outputs.iter().find(|e| &e.output == output) else { return Vec::new() };
        let Some(geometry) = self.capture_geometry(output) else { return Vec::new() };
        let full = Rectangle::<i32, Physical>::from_size(geometry.size);
        let scale = output.current_scale().fractional_scale();
        entry
            .capture_blanks
            .iter()
            .map(|blank| {
                let color = [0.0, 0.0, 0.0, 1.0];
                // A stable id: a capture's damage tracking sees the area change only when it moves.
                let element = SolidColorRenderElement::new(blank.id.clone(), blank.area, CommitCounter::default(), color, Kind::Unspecified);
                CropRenderElement::from_element(element, scale, full).map(OutputElement::from)
            })
            .collect()
    }

    /// What a capture of `output` shows: `elements` with every window kept out of captures replaced, where its
    /// frontmost element was, by its black area (so windows in front of it still cover it).
    pub fn capture_view<'a>(
        &self,
        output: &Output,
        elements: &'a [OutputElement],
        blanks: &'a [Option<OutputElement>],
    ) -> Vec<&'a OutputElement> {
        let noted = self.outputs.iter().find(|e| &e.output == output).map_or(&[][..], |e| &e.capture_blanks[..]);
        if noted.is_empty() {
            return elements.iter().collect();
        }
        let mut placed = vec![false; noted.len()];
        let mut view = Vec::with_capacity(elements.len());
        for element in elements {
            match noted.iter().position(|blank| blank.ids.contains(element.id())) {
                Some(index) => {
                    if !placed[index] {
                        placed[index] = true;
                        view.extend(blanks.get(index).and_then(Option::as_ref));
                    }
                }
                None => view.push(element),
            }
        }
        view
    }

    fn draw_output(&mut self, renderer: &mut GlesRenderer, output: &Output, blanks: &mut Vec<Blank>) -> Vec<OutputElement> {
        let mut elements: Vec<OutputElement> =
            self.cursor_elements(renderer, output).into_iter().map(OutputElement::from).collect();
        elements.extend(self.magnifier_elements(output).into_iter().map(OutputElement::from));
        elements.extend(self.overlay_elements(output).into_iter().map(OutputElement::from));
        self.wallpaper_prepare(renderer);
        self.wallpaper_picker_prepare(renderer);
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
        elements.extend(self.shell_power_elements(renderer, output));
        if let Some(overview) = self.overview_elements(renderer, output, blanks) {
            // The top layer stays on top: notifications, and what to do while picking what to share.
            elements.extend(layer_elements(renderer, output, &[Layer::Overlay]));
            elements.extend(self.shell_notice_elements(renderer, output));
            elements.extend(overview);
            let markers = self.edge_marker_elements(output);
            elements.extend(self.scene_elements(renderer, output, false, Vec::new(), markers, Vec::new(), blanks));
            return elements;
        }
        let markers = self.edge_marker_elements(output);
        let borders = self.border_elements(renderer, output);
        let mut shadows = self.blur_elements(renderer, output);
        shadows.extend(self.shadow_elements(renderer, output));
        elements.extend(self.scene_elements(renderer, output, true, shadows, markers, borders, blanks));
        elements
    }

    /// Layer surfaces and windows of `output`, front to back.
    ///
    /// Windows are drawn only on the output their workspace is on and are cut off at its edge.
    /// Tiles scrolled out of view lie beyond the edge in the shared coordinate space, where the
    /// neighbouring monitor would otherwise show them.
    /// Stacking, front to back: `Overlay` layer, the shell's notices, popups and menus of unmanaged windows,
    /// fullscreen windows, `Top` layer, the shell's bar, the edge `markers`, the other windows each behind its
    /// own border (so a dialog in front covers the border of the window behind it), `behind` (shadows,
    /// blur), `Bottom` and `Background` layers, the wallpaper. (The power menu is in front of all of it.)
    #[allow(clippy::too_many_arguments)]
    fn scene_elements(
        &self,
        renderer: &mut GlesRenderer,
        output: &Output,
        with_windows: bool,
        behind: Vec<OutputElement>,
        markers: Vec<OutputElement>,
        mut borders: Vec<(crate::desktop::WindowId, OutputElement)>,
        blanks: &mut Vec<Blank>,
    ) -> Vec<OutputElement> {
        let Some(geo) = self.space.output_geometry(output) else { return Vec::new() };
        let Some(this_monitor) = self.outputs.iter().position(|e| &e.output == output) else { return Vec::new() };
        let scale = output.current_scale().fractional_scale();
        let visible_area = Rectangle::<i32, Physical>::from_size(geo.size.to_physical_precise_round(scale));
        let layer_elements = |renderer: &mut GlesRenderer, layers: &[Layer]| layer_elements(renderer, output, layers);

        // With the overview the `Overlay` layer is drawn in front of it (see `draw_output`).
        let mut elements = if with_windows { layer_elements(renderer, &[Layer::Overlay]) } else { Vec::new() };
        if with_windows {
            elements.extend(self.shell_notice_elements(renderer, output));
        }
        let (mut unmanaged, mut fullscreen, mut regular) = (Vec::new(), Vec::new(), Vec::new());
        for window in self.space.elements().rev().filter(|_| with_windows) {
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
            let anim_ms = self.config.effects.animation_ms;
            let slide = managed.map_or((0, 0).into(), |m| m.slide_offset(anim_ms));
            let render_location = (location + slide - window.geometry().loc - geo.loc).to_physical_precise_round(scale);
            // Opacity: the window's rule, dimmed further while it has no focus. Not in fullscreen.
            let alpha = managed.map_or(1.0, |m| self.window_opacity(m) * m.fade(anim_ms));
            let surfaces = AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                window,
                renderer,
                render_location,
                Scale::from(scale),
                alpha,
            );
            let bucket: &mut Vec<OutputElement> = match managed {
                None => &mut unmanaged,
                Some(m) if m.shown_fullscreen => &mut fullscreen,
                Some(_) => &mut regular,
            };
            if let Some(m) = managed {
                // Before `start`: a window kept out of captures is blanked, its border still shows (as before).
                bucket.extend(borders.extract_if(.., |(id, _)| *id == m.id).map(|(_, border)| border));
            }
            let start = bucket.len();
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
                            // The window geometry in output pixels: where the shader keeps the surface's pixels.
                            let shape = Rectangle::new(
                                render_location.to_f64() + geometry.loc.to_f64().to_physical(scale),
                                geometry.size.to_f64().to_physical(scale),
                            );
                            let corner = f64::from(self.config.effects.corner_radius) * scale;
                            bucket.push(OutputElement::from(
                                crate::effects::Rounded::new(cropped, shader.tex.clone(), (buf.w as f32, buf.h as f32), geo_px, radius)
                                    .with_opaque_shape(shape, corner),
                            ));
                        }
                        None => bucket.push(OutputElement::from(cropped)),
                    }
                }
            } else {
                bucket.extend(surfaces.into_iter().map(OutputElement::from));
            }
            if let Some(m) = managed {
                self.note_blank(m, &bucket[start..], scale, blanks);
            }
        }
        elements.extend(unmanaged);
        elements.extend(fullscreen);
        elements.extend(layer_elements(renderer, &[Layer::Top]));
        elements.extend(self.shell_bar_elements(renderer, output));
        elements.extend(markers);
        elements.extend(regular);
        elements.extend(behind);
        elements.extend(layer_elements(renderer, &[Layer::Bottom, Layer::Background]));
        elements.extend(self.wallpaper_element(renderer, output));
        elements
    }

    /// Opacity of a window from its rule and focus (1.0 in fullscreen).
    pub fn window_opacity(&self, m: &crate::desktop::Managed) -> f32 {
        if m.fullscreen {
            return 1.0;
        }
        let rule = mywm_config::opacity(&self.config.rules, m.subject());
        let focus = if self.desktop.focused() == Some(m.id) { 1.0 } else { self.config.effects.inactive_opacity };
        rule.unwrap_or(1.0) * focus
    }

    /// The layers below the windows and the wallpaper, front to back.
    pub fn background_elements(&self, renderer: &mut GlesRenderer, output: &Output) -> Vec<OutputElement> {
        let scale = output.current_scale().fractional_scale();
        let map = layer_map_for_output(output);
        let mut elements: Vec<OutputElement> = map
            .layers()
            .rev()
            .filter(|surface| matches!(surface.layer(), Layer::Bottom | Layer::Background))
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
            .collect();
        elements.extend(self.wallpaper_element(renderer, output));
        elements
    }

    /// Background color: the palette's, or black while locked.
    pub fn clear_color(&self) -> [f32; 4] {
        if self.session_lock.is_active() { [0.0, 0.0, 0.0, 1.0] } else { self.desktop.appearance.background.0 }
    }
}

/// The surfaces of `output`'s layer shell `layers`, front to back.
fn layer_elements(renderer: &mut GlesRenderer, output: &Output, layers: &[Layer]) -> Vec<OutputElement> {
    let scale = output.current_scale().fractional_scale();
    let map = layer_map_for_output(output);
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
}
