//! "Xray" blur: translucent windows show a blurred copy of the wallpaper behind them.
//!
//! Only the layers below the windows (wallpaper) are blurred, once per change of those
//! layers, with a dual-Kawase downsample/upsample chain. Drawing it per window is one textured
//! quad, so the cost stays close to nothing; blurring windows behind windows is not attempted.
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Color32F, Frame, Offscreen, Renderer,
            damage::OutputDamageTracker,
            element::{Id, Kind, texture::TextureRenderElement, utils::CropRenderElement},
            gles::{GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType},
        },
    },
    output::Output,
    utils::{Buffer, Logical, Physical, Point, Rectangle, Size, Transform},
};

use crate::{State, effects::Rounded, render::OutputElement};

const LEVELS: usize = 3;

pub struct Programs {
    down: GlesTexProgram,
    up: GlesTexProgram,
}

/// The blurred wallpaper of one output.
pub struct BlurCache {
    /// Half the output's size in pixels.
    texture: GlesTexture,
    size: Size<i32, Physical>,
    strength: u32,
    id: Id,
    dirty: bool,
}

fn shader(body: &str) -> String {
    format!(
        "#version 100
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
precision mediump float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
uniform vec2 half_pixel;
void main() {{
    vec2 uv = v_coords;
    vec2 h = half_pixel;
    vec4 sum;
{body}
    gl_FragColor = sum * alpha;
}}
"
    )
}

const DOWN: &str = "
    sum = texture2D(tex, uv) * 4.0;
    sum += texture2D(tex, uv - h);
    sum += texture2D(tex, uv + h);
    sum += texture2D(tex, uv + vec2(h.x, -h.y));
    sum += texture2D(tex, uv - vec2(h.x, -h.y));
    sum /= 8.0;
";

const UP: &str = "
    sum = texture2D(tex, uv + vec2(-h.x * 2.0, 0.0));
    sum += texture2D(tex, uv + vec2(-h.x, h.y)) * 2.0;
    sum += texture2D(tex, uv + vec2(0.0, h.y * 2.0));
    sum += texture2D(tex, uv + vec2(h.x, h.y)) * 2.0;
    sum += texture2D(tex, uv + vec2(h.x * 2.0, 0.0));
    sum += texture2D(tex, uv + vec2(h.x, -h.y)) * 2.0;
    sum += texture2D(tex, uv + vec2(0.0, -h.y * 2.0));
    sum += texture2D(tex, uv + vec2(-h.x, -h.y)) * 2.0;
    sum /= 12.0;
";

fn compile(renderer: &mut GlesRenderer) -> Result<Programs, smithay::backend::renderer::gles::GlesError> {
    let uniforms = [UniformName::new("half_pixel", UniformType::_2f)];
    Ok(Programs {
        down: renderer.compile_custom_texture_shader(shader(DOWN), &uniforms)?,
        up: renderer.compile_custom_texture_shader(shader(UP), &uniforms)?,
    })
}

/// One pass: draw all of `from` into the (smaller or larger) `to`.
fn pass(
    renderer: &mut GlesRenderer,
    from: &GlesTexture,
    from_size: Size<i32, Physical>,
    to: &mut GlesTexture,
    to_size: Size<i32, Physical>,
    program: &GlesTexProgram,
    offset: f32,
) -> Result<(), String> {
    let mut target = renderer.bind(to).map_err(|e| format!("bind: {e}"))?;
    let mut frame = renderer.render(&mut target, to_size, Transform::Normal).map_err(|e| format!("frame: {e}"))?;
    let dst = Rectangle::<i32, Physical>::from_size(to_size);
    frame.clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[dst]).map_err(|e| format!("clear: {e}"))?;
    let src = Rectangle::<f64, Buffer>::from_size((f64::from(from_size.w), f64::from(from_size.h)).into());
    let half_pixel = (0.5 / from_size.w as f32 * offset, 0.5 / from_size.h as f32 * offset);
    frame
        .render_texture_from_to(from, src, dst, &[dst], &[], Transform::Normal, 1.0, Some(program), &[Uniform::new("half_pixel", half_pixel)])
        .map_err(|e| format!("draw: {e}"))?;
    let sync = frame.finish().map_err(|e| format!("finish: {e}"))?;
    let _ = sync.wait();
    Ok(())
}

impl State {
    /// Something the blurred backdrop is made of changed.
    pub fn blur_dirty(&mut self, output: &Output) {
        if let Some(cache) = self.blur.get_mut(&output.name()) {
            cache.dirty = true;
        }
    }

    pub fn blur_reset(&mut self) {
        self.blur.clear();
    }

    fn blur_render(&mut self, renderer: &mut GlesRenderer, output: &Output, size: Size<i32, Physical>) -> Result<GlesTexture, String> {
        let scale = output.current_scale().fractional_scale();
        let strength = self.config.effects.blur;
        let offset = 1.0 + strength as f32 * 0.25;
        let programs = self.blur_programs.as_ref().ok_or("blur shaders missing")?;
        let (down, up) = (programs.down.clone(), programs.up.clone());

        // 1. The wallpaper (layers below the windows) at full size.
        let background = self.background_elements(renderer, output);
        let mut full = Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Argb8888, (size.w, size.h).into())
            .map_err(|e| format!("texture: {e}"))?;
        {
            let mut target = renderer.bind(&mut full).map_err(|e| format!("bind: {e}"))?;
            let mut tracker = OutputDamageTracker::new(size, scale, Transform::Normal);
            tracker
                .render_output(renderer, &mut target, 0, &background, self.clear_color())
                .map_err(|e| format!("render: {e:?}"))?;
        }

        // 2. Down to 1/2, 1/4, 1/8 of the size, then back up to 1/2.
        let sizes: Vec<Size<i32, Physical>> =
            (1..=LEVELS).map(|level| ((size.w >> level).max(1), (size.h >> level).max(1)).into()).collect();
        let mut levels = Vec::new();
        for level in &sizes {
            levels.push(
                Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Argb8888, (level.w, level.h).into())
                    .map_err(|e| format!("texture: {e}"))?,
            );
        }
        let mut from = full.clone();
        let mut from_size = size;
        for (texture, level_size) in levels.iter_mut().zip(&sizes) {
            pass(renderer, &from, from_size, texture, *level_size, &down, offset)?;
            from = texture.clone();
            from_size = *level_size;
        }
        for index in (0..LEVELS - 1).rev() {
            let source = levels[index + 1].clone();
            pass(renderer, &source, sizes[index + 1], &mut levels[index], sizes[index], &up, offset)?;
        }
        Ok(levels.swap_remove(0))
    }

    /// Backdrops of the translucent windows on `output`, front to back, behind the windows.
    pub fn blur_elements(&mut self, renderer: &mut GlesRenderer, output: &Output) -> Vec<OutputElement> {
        let strength = self.config.effects.blur;
        if strength == 0 {
            return Vec::new();
        }
        let Some(size) = self.capture_geometry(output).map(|g| g.size) else { return Vec::new() };
        let Some(this_monitor) = self.outputs.iter().position(|e| &e.output == output) else { return Vec::new() };
        let Some(geo) = self.space.output_geometry(output) else { return Vec::new() };
        let scale = output.current_scale().fractional_scale();
        let half: Size<i32, Physical> = ((size.w >> 1).max(1), (size.h >> 1).max(1)).into();

        // Windows that let the backdrop show through.
        let monitors: Vec<Option<usize>> = self.desktop.windows.iter().map(|m| self.monitor_of_window(m)).collect();
        let anim_ms = self.config.effects.animation_ms;
        let translucent: Vec<(Rectangle<i32, Logical>, i32)> = self
            .desktop
            .windows
            .iter()
            .zip(monitors)
            .rev()
            .filter(|(m, monitor)| *monitor == Some(this_monitor) && !m.fullscreen && self.window_opacity(m) < 0.99)
            .filter_map(|(m, _)| {
                let (frame, b) = m.frame?;
                let slide = m.slide_offset(anim_ms);
                Some((
                    Rectangle::new(
                        (frame.x + slide.x - geo.loc.x, frame.y + slide.y - geo.loc.y).into(),
                        (frame.width, frame.height).into(),
                    ),
                    b,
                ))
            })
            .collect();
        if translucent.is_empty() {
            return Vec::new();
        }

        if self.blur_programs.is_none() && !self.blur_failed {
            match compile(renderer) {
                Ok(programs) => self.blur_programs = Some(programs),
                Err(error) => {
                    tracing::warn!("blur shaders failed to compile, blur is off: {error}");
                    self.blur_failed = true;
                }
            }
        }
        if self.blur_programs.is_none() || !self.ensure_effect_shaders(renderer) {
            return Vec::new();
        }
        let name = output.name();
        let stale = self.blur.get(&name).is_none_or(|c| c.dirty || c.size != size || c.strength != strength);
        if stale {
            match self.blur_render(renderer, output, size) {
                Ok(texture) => {
                    self.blur.insert(name.clone(), BlurCache { texture, size, strength, id: Id::new(), dirty: false });
                }
                Err(error) => {
                    tracing::warn!("blur failed: {error}");
                    self.blur_failed = true;
                    return Vec::new();
                }
            }
        }
        let (Some(cache), Some(shaders)) = (self.blur.get(&name), self.effect_shaders.as_ref()) else { return Vec::new() };
        let visible_area = Rectangle::<i32, Physical>::from_size(geo.size.to_physical_precise_round(scale));
        let radius_logical = self.config.effects.corner_radius;
        let mut elements = Vec::new();
        for (area, border) in translucent {
            let phys: Rectangle<i32, Physical> = area.to_f64().to_physical(scale).to_i32_round();
            // The texture shows the output at half size: its "logical" space is output pixels / 2.
            let src = Rectangle::<f64, Logical>::new(
                (f64::from(phys.loc.x) / 2.0, f64::from(phys.loc.y) / 2.0).into(),
                (f64::from(phys.size.w) / 2.0, f64::from(phys.size.h) / 2.0).into(),
            );
            let element = TextureRenderElement::from_static_texture(
                cache.id.clone(),
                renderer.context_id(),
                Point::<f64, Physical>::from((f64::from(phys.loc.x), f64::from(phys.loc.y))),
                cache.texture.clone(),
                1,
                Transform::Normal,
                Some(1.0),
                Some(src),
                Some(area.size),
                None,
                Kind::Unspecified,
            );
            let Some(cropped) = CropRenderElement::from_element(element, scale, visible_area) else { continue };
            let radius = if radius_logical > 0 { f64::from(radius_logical + border) * scale / 2.0 } else { 0.0 };
            let geo_px = [
                (f64::from(phys.loc.x) / 2.0) as f32,
                (f64::from(phys.loc.y) / 2.0) as f32,
                (f64::from(phys.size.w) / 2.0) as f32,
                (f64::from(phys.size.h) / 2.0) as f32,
            ];
            elements.push(OutputElement::from(Rounded::new(
                cropped,
                shaders.tex.clone(),
                (half.w as f32, half.h as f32),
                geo_px,
                radius as f32,
            )));
        }
        elements
    }
}
