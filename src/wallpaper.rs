//! The wallpaper, drawn by the compositor behind every layer and window.
//!
//! The picker (`wallpaper.qml` of mywm-shell) saves the choice and asks for a theme reload, which
//! re-reads it here. The image is decoded and scaled on a worker thread, once for the whole desktop
//! (the bounding box of all outputs, filled and cropped in the middle like Qt's `PreserveAspectCrop`),
//! and uploaded as one texture; every output shows its part of it. Nothing of this runs per frame
//! except building one texture element per output.
use std::{
    path::PathBuf,
    time::SystemTime,
};

use calloop::{LoopHandle, channel};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportMem, Renderer,
            element::{Id, Kind, texture::TextureRenderElement},
            gles::{GlesRenderer, GlesTexture},
        },
    },
    output::Output,
    utils::{Buffer, Logical, Physical, Point, Rectangle, Size, Transform},
};

use crate::{State, render::OutputElement};

/// What a picture is made from; a new one is made when any of it changes.
#[derive(Clone, Debug, PartialEq)]
struct Source {
    path: PathBuf,
    /// Replacing the file under the same name counts as a new wallpaper.
    modified: Option<SystemTime>,
    /// The desktop in pixels at the largest output scale.
    size: Size<i32, Physical>,
}

struct Decoded {
    source: Source,
    rgba: Vec<u8>,
    opaque: bool,
}

struct Picture {
    texture: GlesTexture,
    size: Size<i32, Buffer>,
    id: Id,
    opaque: bool,
}

pub struct Wallpaper {
    /// The chosen image and its modification time; `None` shows the palette background.
    file: Option<(PathBuf, Option<SystemTime>)>,
    /// The picture last asked of a worker.
    requested: Option<Source>,
    /// Pixels of `requested`, waiting for the renderer.
    decoded: Option<Decoded>,
    picture: Option<Picture>,
    sender: channel::Sender<Result<Decoded, (Source, String)>>,
}

impl Wallpaper {
    pub fn new(loop_handle: &LoopHandle<'static, State>, config: &mywm_config::Config) -> Self {
        let (sender, receiver) = channel::channel();
        if let Err(error) = loop_handle.insert_source(receiver, |event, _, state| {
            if let channel::Event::Msg(result) = event {
                state.wallpaper_decoded(result);
            }
        }) {
            tracing::warn!("the wallpaper cannot be loaded: {}", error.error);
        }
        Self { file: chosen_file(config), requested: None, decoded: None, picture: None, sender }
    }
}

/// The picker's choice (or the first image of the wallpaper directory).
fn chosen_file(config: &mywm_config::Config) -> Option<(PathBuf, Option<SystemTime>)> {
    let state = mywm_theme::wallpaper_state_path()?;
    match mywm_theme::current_wallpaper(&state, config.wallpaper_directory.as_ref()) {
        Ok(path) => {
            let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            Some((path, modified))
        }
        Err(error) => {
            tracing::info!("no wallpaper: {error}");
            None
        }
    }
}

/// Decode `source.path` and scale it to fill `source.size`.
fn decode(source: Source) -> Result<Decoded, (Source, String)> {
    let image = image::ImageReader::open(&source.path)
        .map_err(|e| e.to_string())
        .and_then(|reader| reader.with_guessed_format().map_err(|e| e.to_string()))
        .and_then(|reader| reader.decode().map_err(|e| e.to_string()));
    let image = match image {
        Ok(image) => image,
        Err(error) => return Err((source, error)),
    };
    let opaque = !image.color().has_alpha();
    let (w, h) = (source.size.w.max(1) as u32, source.size.h.max(1) as u32);
    let rgba = image.resize_to_fill(w, h, image::imageops::FilterType::CatmullRom).into_rgba8().into_raw();
    Ok(Decoded { source, rgba, opaque })
}

impl State {
    /// Re-read the chosen wallpaper (after the picker saved one; a theme reload).
    pub fn wallpaper_reload(&mut self) {
        let file = chosen_file(&self.config);
        if file != self.wallpaper.file {
            self.wallpaper.file = file;
            self.queue_redraw_all();
        }
    }

    fn wallpaper_decoded(&mut self, result: Result<Decoded, (Source, String)>) {
        match result {
            Ok(decoded) if self.wallpaper.requested.as_ref() == Some(&decoded.source) => {
                self.wallpaper.decoded = Some(decoded);
            }
            Err((source, error)) if self.wallpaper.requested.as_ref() == Some(&source) => {
                tracing::warn!("the wallpaper {} cannot be shown: {error}", source.path.display());
                self.wallpaper.picture = None;
                self.blur_reset();
            }
            // Outdated: something changed while the worker ran.
            _ => return,
        }
        self.queue_redraw_all();
    }

    /// The bounding box of all outputs and the largest scale among them.
    fn desktop_area(&self) -> Option<(Rectangle<i32, Logical>, f64)> {
        self.outputs
            .iter()
            .filter_map(|e| Some((self.space.output_geometry(&e.output)?, e.output.current_scale().fractional_scale())))
            .reduce(|(a, sa), (b, sb)| (a.merge(b), sa.max(sb)))
    }

    /// Ask for a new picture when the file or the desktop changed, and upload one that is ready.
    /// Runs before the elements of an output are built.
    pub fn wallpaper_prepare(&mut self, renderer: &mut GlesRenderer) {
        let wanted = match (&self.wallpaper.file, self.desktop_area()) {
            (Some((path, modified)), Some((desktop, scale))) => Some(Source {
                path: path.clone(),
                modified: *modified,
                size: desktop.size.to_f64().to_physical(scale).to_i32_ceil(),
            }),
            _ => None,
        };
        if wanted != self.wallpaper.requested {
            self.wallpaper.requested = wanted.clone();
            self.wallpaper.decoded = None;
            match wanted {
                Some(source) => {
                    let sender = self.wallpaper.sender.clone();
                    let spawned = std::thread::Builder::new().name("wallpaper".into()).spawn(move || {
                        let _ = sender.send(decode(source));
                    });
                    if let Err(error) = spawned {
                        tracing::warn!("cannot start loading the wallpaper: {error}");
                    }
                }
                None if self.wallpaper.picture.take().is_some() => self.blur_reset(),
                None => {}
            }
        }
        // The previous picture stays until the new one is ready, so a change never flashes the background.
        let Some(decoded) = self.wallpaper.decoded.take() else { return };
        let size = Size::<i32, Buffer>::from((decoded.source.size.w.max(1), decoded.source.size.h.max(1)));
        self.wallpaper.picture = match renderer.import_memory(&decoded.rgba, Fourcc::Abgr8888, size, false) {
            Ok(texture) => Some(Picture { texture, size, id: Id::new(), opaque: decoded.opaque }),
            Err(error) => {
                tracing::warn!("the wallpaper {} cannot be uploaded: {error}", decoded.source.path.display());
                None
            }
        };
        self.blur_reset();
    }

    /// The part of the wallpaper on `output`, filling it.
    pub fn wallpaper_element(&self, renderer: &GlesRenderer, output: &Output) -> Option<OutputElement> {
        let picture = self.wallpaper.picture.as_ref()?;
        let (desktop, _) = self.desktop_area()?;
        let geo = self.space.output_geometry(output)?;
        // Texture pixels per logical pixel of the desktop (also right while a newer picture is on its way).
        let fx = f64::from(picture.size.w) / f64::from(desktop.size.w.max(1));
        let fy = f64::from(picture.size.h) / f64::from(desktop.size.h.max(1));
        let src = Rectangle::<f64, Logical>::new(
            (f64::from(geo.loc.x - desktop.loc.x) * fx, f64::from(geo.loc.y - desktop.loc.y) * fy).into(),
            (f64::from(geo.size.w) * fx, f64::from(geo.size.h) * fy).into(),
        );
        Some(OutputElement::from(TextureRenderElement::from_static_texture(
            picture.id.clone(),
            renderer.context_id(),
            Point::<f64, Physical>::from((0.0, 0.0)),
            picture.texture.clone(),
            1,
            Transform::Normal,
            None,
            Some(src),
            Some(geo.size),
            picture.opaque.then(|| vec![Rectangle::from_size(picture.size)]),
            Kind::Unspecified,
        )))
    }
}
