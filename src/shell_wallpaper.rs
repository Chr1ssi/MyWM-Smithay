//! The wallpaper picker of the built-in shell (`wallpaper.qml`), for the wallpaper binding. Every opening reads
//! the wallpaper directory and the saved choice afresh and forgets all of it when it closes, as the picker that
//! Quickshell started on demand did. Pictures are decoded on worker threads, only for the tiles in view, the
//! selected one first; the tiles lean and slide on the GPU (the `tile` shader of `effects`), so a new selection
//! paints nothing again but the texts. A choice is saved only once its image decodes; the theme follows from the
//! own binary (`--theme-from-wallpaper`), as the picker ran it.
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Instant,
};

use mywm_ui::{
    edit::Input,
    power::ease_out_cubic,
    text::Fonts,
    wallpaper_picker::{self, Carousel, LEAN, LEAN_SHIFT, PickerError, PickerModel, PickerTarget, SLIDE_MS, TileView, VISIBLE},
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportMem, Renderer, Texture,
            element::{
                Element, Id, Kind, RenderElement, UnderlyingStorage, memory::MemoryRenderBuffer, texture::TextureRenderElement,
                utils::CropRenderElement,
            },
            gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform},
            utils::{CommitCounter, DamageSet, OpaqueRegions},
        },
    },
    input::keyboard::{Keysym, keysyms},
    output::Output,
    reexports::calloop::{
        channel,
        timer::{TimeoutAction, Timer},
    },
    utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
};

use crate::{
    State,
    render::OutputElement,
    shell::{FRAME, Painted, ShellHit, element, paint, stretched},
};

/// Pictures decoded at the same time.
const DECODING: usize = 2;

struct Entry {
    path: PathBuf,
    url: String,
    /// The file name, which the filter searches.
    name: String,
}

enum Thumb {
    /// Decoded (premultiplied RGBA), waiting for the renderer.
    Pixels(Vec<u8>, (u32, u32)),
    Texture(GlesTexture, (u32, u32)),
    Failed,
}

enum Message {
    Scanned(Result<Vec<PathBuf>, String>),
    Decoded(PathBuf, Result<(Vec<u8>, (u32, u32)), String>),
}

pub struct Picker {
    output: Output,
    /// Tells this opening apart from earlier ones, whose workers may still answer.
    opening: u64,
    /// The images of the directory; `None` while it is read.
    entries: Option<Vec<Entry>>,
    /// The saved choice (a `file://` URL).
    saved: Option<String>,
    query: Input,
    /// The entries that match the query, in order.
    results: Vec<usize>,
    selected: usize,
    /// Where every result's tile was when the selection last changed, and when: they slide from there.
    slide: Option<(Vec<(f32, f32)>, Instant)>,
    /// The timer that redraws while they slide runs.
    ticking: bool,
    thumbs: HashMap<PathBuf, Thumb>,
    decoding: HashSet<PathBuf>,
    /// Pictures are decoded to at most this size: covering the selected tile at the monitor's scale.
    thumb_size: (u32, u32),
    /// Chosen while its picture was still being decoded: whether it loads decides.
    choice: Option<PathBuf>,
    error: Option<PickerError>,
    sender: channel::Sender<Message>,
    model: Option<PickerModel>,
    painted: Option<Framed>,
    /// The dimming over the whole monitor (one pixel, stretched).
    backdrop: MemoryRenderBuffer,
    /// What the shader samples in tiles without a picture yet (it draws them without).
    blank: Option<GlesTexture>,
    /// Per result: the id of its tile's element, and what it last showed with a count of its changes.
    tile_ids: Vec<Id>,
    shown: HashMap<usize, (Vec<Uniform<'static>>, CommitCounter)>,
    /// The tiles in view, front to back, as worked out for the next frame.
    drawn: Vec<DrawnTile>,
}

impl Picker {
    /// Where the tile of every result is at `now`: its left edge in the carousel and its width.
    fn places(&self, carousel: &Carousel, now: Instant) -> Vec<(f32, f32)> {
        let t = self.slide.as_ref().map_or(1.0, |(_, start)| ease_out_cubic(now.duration_since(*start).as_secs_f32() * 1000.0 / SLIDE_MS));
        (0..self.results.len())
            .map(|index| {
                let (x, width) = carousel.place(index as isize - self.selected as isize);
                match self.slide.as_ref().and_then(|(from, _)| from.get(index)) {
                    Some(&(from_x, from_width)) if t < 1.0 => (from_x + (x - from_x) * t, from_width + (width - from_width) * t),
                    _ => (x, width),
                }
            })
            .collect()
    }

    /// The tiles in view at `now`.
    fn tiles(&self, carousel: &Carousel, now: Instant) -> Vec<TileView> {
        self.places(carousel, now)
            .into_iter()
            .enumerate()
            .filter(|(index, _)| index.abs_diff(self.selected) <= VISIBLE)
            .map(|(index, (x, width))| TileView { index, x, width })
            .collect()
    }

    fn sliding(&self, now: Instant) -> bool {
        self.slide.as_ref().is_some_and(|(_, start)| now.duration_since(*start).as_secs_f32() * 1000.0 < SLIDE_MS)
    }

    fn entry(&self, index: usize) -> Option<&Entry> {
        self.entries.as_ref()?.get(*self.results.get(index)?)
    }
}

/// `path` decoded and scaled down to cover `size` (never up, as Qt's `sourceSize`), premultiplied.
fn thumbnail(path: &Path, size: (u32, u32)) -> Result<(Vec<u8>, (u32, u32)), String> {
    let image = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;
    let (width, height) = (image.width().max(1), image.height().max(1));
    let cover = (size.0 as f32 / width as f32).max(size.1 as f32 / height as f32);
    let image = if cover < 1.0 {
        let scaled = |v: u32| ((v as f32 * cover).ceil() as u32).max(1);
        image.resize_exact(scaled(width), scaled(height), image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let image = image.into_rgba8();
    let size = image.dimensions();
    let mut rgba = image.into_raw();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    Ok((rgba, size))
}

/// Premultiplied, for the shader.
fn premultiplied(color: mywm_ui::Color) -> (f32, f32, f32, f32) {
    let a = f32::from(color.a) / 255.0;
    (f32::from(color.r) / 255.0 * a, f32::from(color.g) / 255.0 * a, f32::from(color.b) / 255.0 * a, a)
}

impl State {
    /// Open the picker on the monitor under the pointer; asked again while it is open, it starts over there.
    pub fn wallpaper_picker_open(&mut self) {
        self.wallpaper_picker_close();
        let under = self.space.output_under(self.pointer_location).next().cloned();
        let Some(output) = under.or_else(|| self.outputs.first().map(|e| e.output.clone())) else { return };
        let Some(geo) = self.space.output_geometry(&output) else { return };
        // They lost the keyboard to it, which closed their windows.
        self.launcher_close();
        self.panel_close();
        self.tray_menu_close();
        self.shell.pickers_opened += 1;
        let opening = self.shell.pickers_opened;
        let (sender, messages) = channel::channel();
        let watched = self.loop_handle.insert_source(messages, move |event, _, state: &mut State| {
            if let channel::Event::Msg(message) = event {
                state.wallpaper_picker_message(opening, message);
            }
        });
        if let Err(error) = watched {
            return tracing::warn!("the wallpaper picker cannot load anything: {}", error.error);
        }
        let directory = self.config.wallpaper_directory.clone();
        let scanner = sender.clone();
        let spawned = std::thread::Builder::new().name("wallpapers".into()).spawn(move || {
            let paths = mywm_theme::wallpapers(&directory).map_err(|e| e.to_string());
            // As `--wallpaper-list`: only paths a URL can name.
            let _ = scanner.send(Message::Scanned(paths.map(|paths| paths.into_iter().filter(|p| p.to_str().is_some()).collect())));
        });
        if let Err(error) = spawned {
            return tracing::warn!("cannot read the wallpaper directory: {error}");
        }
        let (saved, error) = match mywm_theme::wallpaper_state_path().map(|state| mywm_theme::saved_wallpaper(&state)) {
            Some(Ok(saved)) => (saved, None),
            Some(Err(_)) => (None, Some(PickerError::State)),
            None => (None, None),
        };
        let scale = output.current_scale().fractional_scale();
        let (width, height) = Carousel::new((geo.size.w as f32, geo.size.h as f32)).picture_size();
        let thumb_size = ((f64::from(width) * scale).ceil() as u32, (f64::from(height) * scale).ceil() as u32);
        let shade = wallpaper_picker::BACKDROP;
        // Premultiplied: the color is black, so only the alpha stays.
        let pixel = [shade.r, shade.g, shade.b, shade.a];
        let backdrop = MemoryRenderBuffer::from_slice(&pixel, Fourcc::Abgr8888, (1, 1), 1, Transform::Normal, None);
        self.shell.picker = Some(Picker {
            output,
            opening,
            entries: None,
            saved,
            query: Input::default(),
            results: Vec::new(),
            selected: 0,
            slide: None,
            ticking: false,
            thumbs: HashMap::new(),
            decoding: HashSet::new(),
            thumb_size,
            choice: None,
            error,
            sender,
            model: None,
            painted: None,
            backdrop,
            blank: None,
            tile_ids: Vec::new(),
            shown: HashMap::new(),
            drawn: Vec::new(),
        });
        self.sync_focus();
        self.shell_changed();
    }

    pub fn wallpaper_picker_close(&mut self) {
        if let Some(picker) = self.shell.picker.take() {
            self.queue_redraw_output(&picker.output);
            self.sync_focus();
            self.shell_changed();
        }
    }

    pub fn wallpaper_picker_takes_keyboard(&self) -> bool {
        self.shell.picker.is_some()
    }

    fn wallpaper_picker_message(&mut self, opening: u64, message: Message) {
        let directory = self.config.wallpaper_directory.clone();
        let Some(picker) = self.shell.picker.as_mut().filter(|p| p.opening == opening) else { return };
        match message {
            Message::Scanned(Ok(paths)) => {
                let entries = paths.into_iter().map(|path| {
                    let url = path.to_str().map(mywm_theme::file_url).unwrap_or_default();
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    Entry { path, url, name }
                });
                picker.entries = Some(entries.collect());
                self.wallpaper_picker_filter();
                // On the image in use (the saved one, else the first).
                let Some(picker) = self.shell.picker.as_mut() else { return };
                if picker.query.text.is_empty() {
                    let current = picker.saved.clone().or_else(|| picker.entry(0).map(|e| e.url.clone()));
                    picker.selected = (0..picker.results.len()).find(|&i| picker.entry(i).map(|e| &e.url) == current.as_ref()).unwrap_or(0);
                }
            }
            Message::Scanned(Err(error)) => {
                tracing::warn!("cannot read the wallpaper directory {directory}: {error}");
                picker.entries = Some(Vec::new());
                picker.error = Some(PickerError::Directory(directory));
            }
            Message::Decoded(path, result) => {
                picker.decoding.remove(&path);
                let loaded = match result {
                    Ok((rgba, size)) => Thumb::Pixels(rgba, size),
                    Err(error) => {
                        tracing::warn!("the wallpaper {} cannot be shown: {error}", path.display());
                        Thumb::Failed
                    }
                };
                let failed = matches!(loaded, Thumb::Failed);
                picker.thumbs.insert(path.clone(), loaded);
                let output = picker.output.clone();
                if picker.choice.as_ref() == Some(&path) {
                    picker.choice = None;
                    if failed {
                        picker.error = Some(PickerError::Image);
                    } else {
                        return self.wallpaper_picker_commit(&path);
                    }
                }
                self.queue_redraw_output(&output);
            }
        }
        self.wallpaper_picker_request();
        self.shell_changed();
    }

    /// The results for the query; a new search starts at its first image, its tiles in place at once (the QML
    /// `Repeater` made new ones). The pictures are asked for once the selection is settled.
    fn wallpaper_picker_filter(&mut self) {
        let Some(picker) = &mut self.shell.picker else { return };
        let names: Vec<String> = picker.entries.iter().flatten().map(|e| e.name.clone()).collect();
        picker.results = wallpaper_picker::results(&names, &picker.query.text);
        picker.tile_ids = picker.results.iter().map(|_| Id::new()).collect();
        picker.shown.clear();
        picker.selected = 0;
        picker.slide = None;
    }

    /// Decode the pictures still missing for the tiles in view (the choice first, then from the selected one
    /// outwards) and forget the others.
    fn wallpaper_picker_request(&mut self) {
        let Some(picker) = &mut self.shell.picker else { return };
        let mut near: Vec<usize> = (0..picker.results.len()).filter(|i| i.abs_diff(picker.selected) <= VISIBLE).collect();
        near.sort_by_key(|i| i.abs_diff(picker.selected));
        let wanted: Vec<PathBuf> = picker.choice.iter().cloned().chain(near.into_iter().filter_map(|i| picker.entry(i).map(|e| e.path.clone()))).collect();
        picker.thumbs.retain(|path, _| wanted.contains(path));
        for path in wanted {
            if picker.decoding.len() >= DECODING {
                break;
            }
            if picker.thumbs.contains_key(&path) || picker.decoding.contains(&path) {
                continue;
            }
            let (sender, size, decoded) = (picker.sender.clone(), picker.thumb_size, path.clone());
            let spawned = std::thread::Builder::new().name("wallpaper-thumb".into()).spawn(move || {
                let result = thumbnail(&decoded, size);
                let _ = sender.send(Message::Decoded(decoded, result));
            });
            match spawned {
                Ok(_) => {
                    picker.decoding.insert(path);
                }
                Err(error) => {
                    tracing::warn!("cannot decode {}: {error}", path.display());
                    picker.thumbs.insert(path, Thumb::Failed);
                }
            }
        }
    }

    /// Select the result at `index`; the tiles slide to their new places.
    fn wallpaper_picker_select(&mut self, index: usize) {
        let Some(picker) = &mut self.shell.picker else { return };
        if index == picker.selected || index >= picker.results.len() {
            return;
        }
        let Some(geo) = self.space.output_geometry(&picker.output) else { return };
        let now = Instant::now();
        let from = picker.places(&Carousel::new((geo.size.w as f32, geo.size.h as f32)), now);
        picker.selected = index;
        picker.slide = Some((from, now));
        self.wallpaper_picker_tick();
        self.wallpaper_picker_request();
        self.shell_changed();
    }

    /// One step through the results, stopping at their ends.
    fn wallpaper_picker_move(&mut self, delta: isize) {
        let Some(picker) = &self.shell.picker else { return };
        let Some(last) = picker.results.len().checked_sub(1) else { return };
        let index = (picker.selected as isize + delta).clamp(0, last as isize) as usize;
        self.wallpaper_picker_select(index);
    }

    /// Keep redrawing while the tiles slide.
    fn wallpaper_picker_tick(&mut self) {
        let Some(picker) = &mut self.shell.picker else { return };
        if picker.ticking {
            return;
        }
        let opening = picker.opening;
        let ticker = self.loop_handle.insert_source(Timer::from_duration(FRAME), move |_, _, state: &mut State| {
            let Some(picker) = state.shell.picker.as_mut().filter(|p| p.opening == opening) else { return TimeoutAction::Drop };
            let sliding = picker.sliding(Instant::now());
            if !sliding {
                picker.ticking = false;
                picker.slide = None;
            }
            let output = picker.output.clone();
            state.queue_redraw_output(&output);
            if sliding { TimeoutAction::ToDuration(FRAME) } else { TimeoutAction::Drop }
        });
        match ticker {
            Ok(_) => picker.ticking = true,
            Err(error) => tracing::warn!("the wallpaper picker cannot slide: {}", error.error),
        }
    }

    /// Choose the result at `index`: saved once its picture has loaded.
    fn wallpaper_picker_choose(&mut self, index: usize) {
        let Some(picker) = &mut self.shell.picker else { return };
        let Some(path) = picker.entry(index).map(|e| e.path.clone()) else { return };
        picker.error = None;
        match picker.thumbs.get(&path) {
            Some(Thumb::Pixels(..) | Thumb::Texture(..)) => return self.wallpaper_picker_commit(&path),
            Some(Thumb::Failed) => picker.error = Some(PickerError::Image),
            None => {
                picker.choice = Some(path);
                self.wallpaper_picker_request();
            }
        }
        self.shell_changed();
    }

    /// Save `path` as the wallpaper, close, and generate the theme from it.
    fn wallpaper_picker_commit(&mut self, path: &Path) {
        let saved = mywm_theme::wallpaper_state_path()
            .ok_or_else(|| "HOME or XDG_STATE_HOME is required".to_owned())
            .and_then(|state| mywm_theme::save_wallpaper(&state, path).map_err(|e| e.to_string()));
        if let Err(error) = saved {
            tracing::warn!("the wallpaper choice cannot be saved: {error}");
            if let Some(picker) = &mut self.shell.picker {
                picker.error = Some(PickerError::Save);
            }
            return self.shell_changed();
        }
        self.wallpaper_picker_close();
        self.wallpaper_reload();
        self.wallpaper_picker_apply_theme(path);
    }

    /// `--theme-from-wallpaper` of the own binary in the background. It asks the compositor over the bar socket to
    /// reload the theme; without a socket nobody would, so then this does once it is done.
    fn wallpaper_picker_apply_theme(&mut self, path: &Path) {
        let exe = match mywm_config::current_exe() {
            Ok(exe) => exe,
            Err(error) => return tracing::warn!("cannot find the own binary for the theme: {error}"),
        };
        let mut command = std::process::Command::new(exe);
        command
            .arg("--theme-from-wallpaper")
            .arg(path)
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        let (sender, done) = channel::channel();
        let watched = self.loop_handle.insert_source(done, |event, _, state: &mut State| {
            if let channel::Event::Msg(outcome) = event {
                match outcome {
                    Ok(()) if state.ipc.is_none() => state.reload_config(),
                    Ok(()) => {}
                    Err(error) => tracing::warn!("the theme cannot be generated from the wallpaper: {error}"),
                }
            }
        });
        if let Err(error) = watched {
            tracing::warn!("cannot wait for the theme: {}", error.error);
        }
        match command.spawn() {
            Ok(child) => {
                std::thread::spawn(move || {
                    let outcome = match child.wait_with_output() {
                        Ok(output) if output.status.success() => Ok(()),
                        Ok(output) => Err(format!("{}: {}", output.status, String::from_utf8_lossy(&output.stderr).trim())),
                        Err(error) => Err(error.to_string()),
                    };
                    let _ = sender.send(outcome);
                });
            }
            Err(error) => tracing::warn!("cannot generate the theme: {error}"),
        }
    }

    /// A key while the picker holds the keyboard; `modified`: Ctrl, Alt or Super is held. The arrows and Tab move
    /// through the images, not through the filter's text.
    pub fn wallpaper_picker_key(&mut self, sym: Keysym, modified: bool) {
        let Some(picker) = &mut self.shell.picker else { return };
        let query = &mut picker.query;
        let before = query.text.clone();
        match sym.raw() {
            keysyms::KEY_Escape => return self.wallpaper_picker_close(),
            keysyms::KEY_Right | keysyms::KEY_KP_Right | keysyms::KEY_Down | keysyms::KEY_KP_Down | keysyms::KEY_Tab => {
                return self.wallpaper_picker_move(1);
            }
            keysyms::KEY_Left | keysyms::KEY_KP_Left | keysyms::KEY_Up | keysyms::KEY_KP_Up | keysyms::KEY_ISO_Left_Tab => {
                return self.wallpaper_picker_move(-1);
            }
            keysyms::KEY_Return | keysyms::KEY_KP_Enter => {
                let selected = picker.selected;
                return self.wallpaper_picker_choose(selected);
            }
            keysyms::KEY_BackSpace => query.backspace(),
            keysyms::KEY_Delete | keysyms::KEY_KP_Delete => query.delete(),
            keysyms::KEY_Home | keysyms::KEY_KP_Home => query.home(),
            keysyms::KEY_End | keysyms::KEY_KP_End => query.end(),
            _ => match sym.key_char().filter(|c| !c.is_control() && !modified) {
                Some(c) => query.insert(c.encode_utf8(&mut [0; 4])),
                None => return,
            },
        }
        if query.text != before {
            self.wallpaper_picker_filter();
            self.wallpaper_picker_request();
        }
        self.shell_changed();
    }

    /// A click: on a neighbour it selects it, on the selected tile it chooses it, in the field it places the
    /// cursor, anywhere else on the monitor it closes the picker.
    pub fn wallpaper_picker_clicked(&mut self, target: Option<PickerTarget>) {
        match target {
            None => self.wallpaper_picker_close(),
            Some(PickerTarget::Tile(index)) if self.shell.picker.as_ref().is_some_and(|p| p.selected == index) => self.wallpaper_picker_choose(index),
            Some(PickerTarget::Tile(index)) => self.wallpaper_picker_select(index),
            Some(PickerTarget::Search) => {
                let Some(picker) = self.shell.picker.as_ref() else { return };
                let (Some(geo), Some(frame)) = (self.space.output_geometry(&picker.output), picker.painted.as_ref().map(|p| p.frame)) else { return };
                let output = (geo.size.w as f32, geo.size.h as f32);
                let Some(field) = picker.painted.as_ref().and_then(|p| p.painted.scene.target_rect(&PickerTarget::Search)) else { return };
                let x = (self.pointer_location.x - f64::from(geo.loc.x)) as f32 - frame.x - field.x;
                let query = picker.query.clone();
                let Some(fonts) = self.shell.fonts.as_mut() else { return };
                let cursor = wallpaper_picker::search_cursor_at(fonts, &query, output, x);
                if let Some(picker) = &mut self.shell.picker {
                    picker.query.cursor = cursor;
                }
                self.shell_changed();
            }
        }
    }

    /// The wheel over a tile moves through the images, a notch a step.
    pub fn wallpaper_picker_scroll(&mut self, target: Option<PickerTarget>, steps: f32) {
        if matches!(target, Some(PickerTarget::Tile(_))) && steps != 0.0 {
            self.wallpaper_picker_move(if steps < 0.0 { 1 } else { -1 });
        }
    }

    /// Paint the picker's texts if what they show changed; its output goes to `redraw`.
    pub fn wallpaper_picker_refresh(&mut self, fonts: &mut Fonts, redraw: &mut Vec<Output>) {
        let Some(picker) = self.shell.picker.as_ref() else { return };
        let Some(geo) = self.space.output_geometry(&picker.output) else {
            // Its monitor is gone: nothing may hold the keyboard unseen.
            return self.wallpaper_picker_close();
        };
        let output = (geo.size.w as f32, geo.size.h as f32);
        let model = PickerModel {
            output,
            selected: picker.selected,
            count: picker.results.len(),
            name: picker.entry(picker.selected).map(|e| e.name.clone()).unwrap_or_default(),
            query: picker.query.clone(),
            loading: picker.entries.is_none(),
            error: picker.error.clone(),
            language: self.shell.language,
        };
        if picker.model.as_ref() == Some(&model) {
            return;
        }
        let scene = wallpaper_picker::scene(&model, &self.shell.theme, fonts);
        let frame = wallpaper_picker::frame(fonts, output);
        let area = wallpaper_picker::carousel_area(fonts, output);
        let scale = picker.output.current_scale().fractional_scale();
        let painted = paint(scene, fonts, scale).map(|painted| Framed { painted, frame, area });
        let Some(picker) = &mut self.shell.picker else { return };
        picker.painted = painted;
        picker.model = Some(model);
        redraw.push(picker.output.clone());
    }

    /// Forget the picker's picture (the theme changed).
    pub fn wallpaper_picker_repaint(&mut self) {
        if let Some(picker) = &mut self.shell.picker {
            picker.model = None;
        }
    }

    /// Upload decoded pictures, make sure the tile shader is there and work out how the tiles look now; runs
    /// before the elements are built.
    pub fn wallpaper_picker_prepare(&mut self, renderer: &mut GlesRenderer) {
        if self.shell.picker.is_none() || !self.ensure_effect_shaders(renderer) {
            return;
        }
        let Some(picker) = &mut self.shell.picker else { return };
        if picker.blank.is_none() {
            picker.blank = renderer.import_memory(&[0, 0, 0, 0], Fourcc::Abgr8888, Size::from((1, 1)), false).ok();
        }
        for thumb in picker.thumbs.values_mut() {
            let Thumb::Pixels(rgba, size) = thumb else { continue };
            let pixels = Size::from((size.0 as i32, size.1 as i32));
            *thumb = match renderer.import_memory(rgba, Fourcc::Abgr8888, pixels, false) {
                Ok(texture) => Thumb::Texture(texture, *size),
                Err(error) => {
                    tracing::warn!("a wallpaper picture cannot be uploaded: {error}");
                    Thumb::Failed
                }
            };
        }
        picker.drawn.clear();
        let (Some(geo), Some(framed), Some(blank)) = (self.space.output_geometry(&picker.output), &picker.painted, &picker.blank) else { return };
        let scale = picker.output.current_scale().fractional_scale();
        let s = scale as f32;
        let carousel = Carousel::new((geo.size.w as f32, geo.size.h as f32));
        let tiles = picker.tiles(&carousel, Instant::now());
        let area = framed.area.translated(framed.frame.x, framed.frame.y);
        let mut drawn = Vec::new();
        for tile in wallpaper_picker::stacked(&tiles, picker.selected).into_iter().rev() {
            let look = wallpaper_picker::tile_look(&area, tile, tile.index == picker.selected, &self.shell.theme);
            let rect = look.rect;
            // Its bounding box once it leans, a pixel larger for the smoothed edges.
            let shifts = [LEAN_SHIFT, LEAN_SHIFT + LEAN * rect.h];
            let left = rect.x + shifts[0].min(shifts[1]) - 1.0;
            let right = rect.right() + shifts[0].max(shifts[1]) + 1.0;
            let location = Point::<f64, Physical>::from((f64::from((left * s).floor()), f64::from(((rect.y - 1.0) * s).floor())));
            let size = Size::from(((right - left + 1.0).ceil() as i32, (rect.h + 3.0).ceil() as i32));
            let thumb = picker.entry(tile.index).and_then(|e| picker.thumbs.get(&e.path));
            let (texture, picture) = match thumb {
                Some(Thumb::Texture(texture, size)) => (texture.clone(), Some(*size)),
                _ => (blank.clone(), None),
            };
            let Some(id) = picker.tile_ids.get(tile.index).cloned() else { continue };
            let geometry = tile_texture(id.clone(), renderer, location, &texture, size).geometry(Scale::from(scale));
            let origin = (geometry.loc.x as f32, geometry.loc.y as f32);
            let local = |r: &mywm_ui::Rect| (r.x * s - origin.0, r.y * s - origin.1, r.w * s, r.h * s);
            let (tile_px, inner_px) = (local(&rect), local(&look.image));
            // The picture fills the space inside the margin, cut to its proportions around the middle.
            let image = picture.map_or((0.0, 0.0, 1.0, 1.0), |(w, h)| {
                let cover = (inner_px.2 / w as f32).max(inner_px.3 / h as f32);
                let (shown_w, shown_h) = (w as f32 * cover, h as f32 * cover);
                (inner_px.0 + (inner_px.2 - shown_w) / 2.0, inner_px.1 + (inner_px.3 - shown_h) / 2.0, shown_w, shown_h)
            });
            let uniforms = vec![
                Uniform::new("size", (geometry.size.w as f32, geometry.size.h as f32)),
                Uniform::new("tile", tile_px),
                Uniform::new("lean", (LEAN, LEAN_SHIFT * s)),
                Uniform::new("image", image),
                Uniform::new("inner", inner_px),
                Uniform::new("has_image", if picture.is_some() { 1.0f32 } else { 0.0 }),
                Uniform::new("border", look.border * s),
                Uniform::new("surface", premultiplied(look.surface)),
                Uniform::new("dim", premultiplied(look.dim)),
                Uniform::new("border_color", premultiplied(look.border_color)),
            ];
            // The same element as before unless it shows something else: a moved one the damage tracker notices
            // by itself.
            let shown = picker.shown.entry(tile.index).or_insert_with(|| (Vec::new(), CommitCounter::default()));
            if shown.0 != uniforms {
                shown.0 = uniforms.clone();
                shown.1.increment();
            }
            drawn.push(DrawnTile { id, location, size, texture, uniforms, commit: shown.1 });
        }
        picker.drawn = drawn;
    }

    /// The picker on `output`, front to back: its texts, the tiles (the selected one first), the dimming.
    pub fn wallpaper_picker_elements(&self, renderer: &mut GlesRenderer, output: &Output) -> Vec<OutputElement> {
        let Some(picker) = self.shell.picker.as_ref().filter(|p| &p.output == output) else { return Vec::new() };
        let Some(geo) = self.space.output_geometry(output) else { return Vec::new() };
        let scale = output.current_scale().fractional_scale();
        let mut elements = Vec::new();
        if let Some(framed) = &picker.painted {
            elements.extend(element(renderer, &framed.painted, (f64::from(framed.frame.x), f64::from(framed.frame.y)), scale));
            if let Some(shaders) = &self.effect_shaders {
                // The carousel cuts off what reaches past its sides.
                let area = framed.area.translated(framed.frame.x, framed.frame.y);
                let s = scale as f32;
                let crop = Rectangle::<i32, Physical>::new(
                    ((area.x * s).floor() as i32, (area.y * s).floor() as i32).into(),
                    ((area.w * s).ceil() as i32, (area.h * s).ceil() as i32).into(),
                );
                for tile in &picker.drawn {
                    let inner = tile_texture(tile.id.clone(), renderer, tile.location, &tile.texture, tile.size);
                    let shaded = TileElement { inner, program: shaders.tile.clone(), uniforms: tile.uniforms.clone(), commit: tile.commit };
                    elements.extend(CropRenderElement::from_element(shaded, Scale::from(scale), crop).map(OutputElement::from));
                }
            }
        }
        elements.extend(stretched(renderer, &picker.backdrop, (1, 1), (0.0, 0.0), geo.size, scale));
        elements
    }

    /// What of the picker is at the pointer: on its monitor everything is (`None`: nothing but the dimming).
    pub fn wallpaper_picker_at_pointer(&self) -> Option<ShellHit> {
        let picker = self.shell.picker.as_ref()?;
        let geo = self.space.output_geometry(&picker.output)?;
        if !geo.to_f64().contains(self.pointer_location) {
            return None;
        }
        let Some(framed) = &picker.painted else { return Some(ShellHit::Picker(None)) };
        let local = self.pointer_location - geo.loc.to_f64();
        let (x, y) = ((local.x as f32) - framed.frame.x, (local.y as f32) - framed.frame.y);
        let tiles = picker.tiles(&Carousel::new((geo.size.w as f32, geo.size.h as f32)), Instant::now());
        Some(ShellHit::Picker(wallpaper_picker::target_at(&framed.painted.scene, &tiles, picker.selected, &framed.area, x, y)))
    }
}

/// `texture` stretched over `size` (logical) at `location`: the tile shader only takes its coordinates (they run
/// from 0 to 1 over the whole box) and samples the picture where it wants.
fn tile_texture(id: Id, renderer: &GlesRenderer, location: Point<f64, Physical>, texture: &GlesTexture, size: Size<i32, Logical>) -> TextureRenderElement<GlesTexture> {
    // Without `src` the element would take `size` for the texture's own.
    let src = Rectangle::from_size(texture.size().to_f64().to_logical(1.0, Transform::Normal));
    TextureRenderElement::from_static_texture(id, renderer.context_id(), location, texture.clone(), 1, Transform::Normal, None, Some(src), Some(size), None, Kind::Unspecified)
}

/// A tile as worked out before the frame (`wallpaper_picker_prepare`).
struct DrawnTile {
    id: Id,
    location: Point<f64, Physical>,
    size: Size<i32, Logical>,
    texture: GlesTexture,
    uniforms: Vec<Uniform<'static>>,
    commit: CommitCounter,
}

/// A tile drawn through the `tile` shader; `commit` counts what it showed, so an unchanged one is not drawn again.
pub struct TileElement {
    inner: TextureRenderElement<GlesTexture>,
    program: GlesTexProgram,
    uniforms: Vec<Uniform<'static>>,
    commit: CommitCounter,
}

impl Element for TileElement {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
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
        if self.commit.distance(commit) == Some(0) {
            DamageSet::default()
        } else {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
        }
    }

    /// Its edges lean and are smoothed: nothing of it is opaque.
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

impl RenderElement<GlesRenderer> for TileElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        frame.override_default_tex_program(self.program.clone(), self.uniforms.clone());
        let result = RenderElement::<GlesRenderer>::draw(&self.inner, frame, src, dst, damage, opaque_regions);
        frame.clear_tex_program_override();
        result
    }

    /// Scanning out the texture directly would skip the shader.
    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

/// The picker's texts and where they and the carousel are (`wallpaper_picker::frame`, `carousel_area`).
pub struct Framed {
    painted: Painted<PickerTarget>,
    frame: mywm_ui::Rect,
    area: mywm_ui::Rect,
}
