//! Visual effects outside of fullscreen: rounded window corners and borders, window opacity.
//!
//! Windows are drawn through a texture shader that fades out everything beyond the rounded
//! rectangle of the window's geometry; the borders become one shader-drawn ring. Both cost a
//! blended draw instead of direct scanout, so none of this applies to fullscreen windows.
use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{
            GlesError, GlesFrame, GlesPixelProgram, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
        },
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{Buffer, Physical, Point, Rectangle, Scale, Transform},
};

pub struct Shaders {
    pub tex: GlesTexProgram,
    pub ring: GlesPixelProgram,
}

const ROUNDED_SDF: &str = "
float rounded_box(vec2 p, vec2 hs, float r) {
    vec2 q = abs(p) - (hs - vec2(r));
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
";

const PRECISION: &str = "
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif
";

fn tex_shader() -> String {
    format!(
        "#version 100
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
{PRECISION}
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
uniform vec2 buf_size;
uniform vec4 geo;
uniform float radius;
{ROUNDED_SDF}
void main() {{
    vec4 color = texture2D(tex, v_coords);
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif
    vec2 p = v_coords * buf_size - (geo.xy + geo.zw * 0.5);
    float d = rounded_box(p, geo.zw * 0.5, radius);
    color *= alpha * clamp(0.5 - d, 0.0, 1.0);
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif
    gl_FragColor = color;
}}
"
    )
}

fn ring_shader() -> String {
    format!(
        "{PRECISION}
uniform vec2 size;
uniform float alpha;
varying vec2 v_coords;
uniform vec4 color;
uniform float border;
uniform float radius;
{ROUNDED_SDF}
void main() {{
    vec2 p = (v_coords - 0.5) * size;
    float outer = rounded_box(p, size * 0.5, radius + border);
    float inner = rounded_box(p, size * 0.5 - vec2(border), radius);
    float m = clamp(0.5 - outer, 0.0, 1.0) * clamp(0.5 + inner, 0.0, 1.0);
    gl_FragColor = color * (m * alpha);
}}
"
    )
}

pub fn compile(renderer: &mut GlesRenderer) -> Result<Shaders, GlesError> {
    let tex = renderer.compile_custom_texture_shader(
        tex_shader(),
        &[
            UniformName::new("buf_size", UniformType::_2f),
            UniformName::new("geo", UniformType::_4f),
            UniformName::new("radius", UniformType::_1f),
        ],
    )?;
    let ring = renderer.compile_custom_pixel_shader(
        ring_shader(),
        &[
            UniformName::new("color", UniformType::_4f),
            UniformName::new("border", UniformType::_1f),
            UniformName::new("radius", UniformType::_1f),
        ],
    )?;
    Ok(Shaders { tex, ring })
}

/// Uniforms of the ring around a window frame (physical pixels; premultiplied color).
pub fn ring_uniforms(color: [f32; 4], border: f32, radius: f32) -> Vec<Uniform<'static>> {
    let [r, g, b, a] = color;
    vec![
        Uniform::new("color", (r * a, g * a, b * a, a)),
        Uniform::new("border", border),
        Uniform::new("radius", radius),
    ]
}

/// A surface element drawn with the rounded-corner shader.
pub struct Rounded<E> {
    inner: E,
    program: GlesTexProgram,
    uniforms: Vec<Uniform<'static>>,
}

impl<E> Rounded<E> {
    /// `buf_size`: size of the surface's buffer in pixels; `geo`: the window geometry and
    /// `radius` in the same pixels.
    pub fn new(inner: E, program: GlesTexProgram, buf_size: (f32, f32), geo: [f32; 4], radius: f32) -> Self {
        let uniforms = vec![
            Uniform::new("buf_size", buf_size),
            Uniform::new("geo", (geo[0], geo[1], geo[2], geo[3])),
            Uniform::new("radius", radius),
        ];
        Self { inner, program, uniforms }
    }
}

impl<E: Element> Element for Rounded<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    /// The corners are see-through, so nothing behind the element may be skipped.
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl<E: RenderElement<GlesRenderer>> RenderElement<GlesRenderer> for Rounded<E> {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        frame.override_default_tex_program(self.program.clone(), self.uniforms.clone());
        let result = self.inner.draw(frame, src, dst, damage, opaque_regions);
        frame.clear_tex_program_override();
        result
    }

    /// Scanning out the buffer directly would skip the shader.
    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
