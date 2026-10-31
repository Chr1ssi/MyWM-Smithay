//! The built-in shell's lock screen (as splice-shell's `lock.qml`): the compositor locks the session itself
//! (`own_lock` in `session`), so no client draws, reaches or ends the lock, and only a password that PAM
//! accepts unlocks it.
//!
//! Each check runs `splice-shell-services --authenticate` in a process of its own, the password on its stdin:
//! a PAM module that crashes ends that attempt, never the lock (one that hangs keeps it waiting, as lock.qml
//! did). The session unlocks only when that process exits cleanly and answers `success` for the attempt still
//! being checked.
//!
//! The picture: the theme's image (`source` of `theme.json`), cut to each monitor's size on a worker thread
//! and dimmed, the theme's background until it is ready; on the primary monitor the clock and the card in
//! front (`splice_ui::lock_screen`), painted on the CPU when what they show changes.
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
};

use splice_shell_protocol::AuthOutcome;
use splice_ui::{
    clock::{self, LocalTime},
    lock_screen::{self, LockMessage, LockModel, LockTarget},
    text::Fonts,
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportMem, Renderer,
            element::{Id, Kind, solid::SolidColorRenderElement, texture::TextureRenderElement, utils::CropRenderElement},
            gles::{GlesRenderer, GlesTexture},
            utils::CommitCounter,
        },
    },
    input::keyboard::{Keysym, keysyms},
    output::Output,
    reexports::calloop::channel,
    utils::{Buffer, Logical, Physical, Point, Rectangle, Size, Transform},
};

use crate::{
    State,
    render::OutputElement,
    shell::{Painted, ShellHit, element, paint},
};

/// Room for a long password, so typing does not move it (leaving copies behind in freed memory).
const PASSWORD_CAPACITY: usize = 256;

#[derive(Default)]
pub struct LockScreen {
    /// What was typed; zeroed before it goes.
    pub(crate) password: String,
    /// The attempt being checked; keys wait meanwhile, as the disabled field did.
    pub(crate) checking: Option<u64>,
    pub(crate) attempts: u64,
    /// What the last check said; nothing before the first one and while checking.
    pub message: Option<LockMessage>,
    /// The message is an error (the password field turns red).
    pub failed: bool,
    /// The field or (on the login screen) button that has the keyboard.
    pub focus: LockTarget,
    /// The login screen (`--greeter`): what it asks besides the password.
    pub greeter: Option<crate::shell_greeter::Greeter>,
    /// Where the checks answer: the attempt and its outcome, `None` if the check itself failed.
    results: Option<channel::Sender<(u64, Option<AuthOutcome>)>>,
    /// The backdrop's id and color, and a count that changes with the color, for damage tracking.
    backdrop: Option<(Id, [f32; 4], CommitCounter)>,
    /// The dimming over the background.
    dim: Option<Id>,
    /// The clock and the card (and the login screen's islands) as painted.
    painted: Option<Shown>,
    /// The theme's image while locked; `None` shows the theme's background.
    source: Option<PathBuf>,
    /// The image cut to the sizes of the monitors, uploaded.
    backgrounds: Vec<Background>,
    /// The cyberdeck's scanlines in the sizes of the monitors, uploaded while the style asks for them.
    scanlines: Vec<Background>,
    /// Sizes asked of a worker, and pixels of them waiting for the renderer.
    requested: Vec<Size<i32, Physical>>,
    decoded: Vec<(Size<i32, Physical>, Vec<u8>)>,
    /// Where the workers answer: the lock they worked for (`locks`), the size, the pixels.
    decodes: Option<channel::Sender<Decoded>>,
    /// Counts the locks, so pictures of an earlier one are dropped.
    locks: u64,
}

struct Shown {
    output: Output,
    model: LockModel,
    scale: f64,
    /// Each scene and where its origin lies on the output.
    painted: Vec<(Painted<LockTarget>, (f32, f32))>,
}

struct Background {
    size: Size<i32, Physical>,
    texture: GlesTexture,
    id: Id,
}

type Decoded = (u64, Size<i32, Physical>, Result<Vec<u8>, String>);

/// Decode `path` once and cut it to each of `sizes` like `PreserveAspectCrop`, for the lock counted `locks`.
fn decode(path: &std::path::Path, sizes: &[Size<i32, Physical>], locks: u64) -> Vec<Decoded> {
    let image = image::ImageReader::open(path)
        .map_err(|e| e.to_string())
        .and_then(|reader| reader.with_guessed_format().map_err(|e| e.to_string()))
        .and_then(|reader| reader.decode().map_err(|e| e.to_string()));
    sizes
        .iter()
        .map(|size| {
            let pixels = image.as_ref().map_err(Clone::clone).map(|image| {
                let (w, h) = (size.w.max(1) as u32, size.h.max(1) as u32);
                image.resize_to_fill(w, h, image::imageops::FilterType::CatmullRom).into_rgba8().into_raw()
            });
            (locks, *size, pixels)
        })
        .collect()
}

/// An output's size in pixels.
fn pixel_size(geo: Rectangle<i32, Logical>, output: &Output) -> Size<i32, Physical> {
    geo.size.to_f64().to_physical(output.current_scale().fractional_scale()).to_i32_ceil()
}

/// Overwrite `text` before it is emptied, so the password does not stay in memory.
pub fn wipe(text: &mut String) {
    // SAFETY: zero bytes are valid UTF-8.
    unsafe { text.as_mut_vec() }.fill(0);
    text.clear();
}

impl Drop for LockScreen {
    fn drop(&mut self) {
        wipe(&mut self.password);
    }
}

/// Check `password` in a helper process (see the module); its bytes are zeroed once it is written.
fn check(mut password: Vec<u8>) -> Option<AuthOutcome> {
    let child = Command::new(crate::shell_services::program())
        .arg("--authenticate")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            password.fill(0);
            tracing::error!("cannot check the password: {error}");
            return None;
        }
    };
    // Dropping stdin ends the password for the helper, which reads to the end.
    if let Some(mut stdin) = child.stdin.take()
        && let Err(error) = stdin.write_all(&password)
    {
        tracing::warn!("cannot hand the password to its check: {error}");
    }
    password.fill(0);
    let mut answer = String::new();
    if let Some(mut stdout) = child.stdout.take()
        && let Err(error) = stdout.read_to_string(&mut answer)
    {
        tracing::warn!("cannot read what the password check said: {error}");
    }
    match child.wait() {
        Ok(status) if status.success() => AuthOutcome::parse(&answer),
        Ok(status) => {
            tracing::error!("the password check failed ({status})");
            None
        }
        Err(error) => {
            tracing::error!("cannot wait for the password check: {error}");
            None
        }
    }
}

impl State {
    /// Set up where the password checks answer (the built-in shell only).
    pub fn shell_lock_init(&mut self) {
        let (sender, results) = channel::channel();
        match self.loop_handle.insert_source(results, |event, _, state: &mut State| {
            if let channel::Event::Msg((attempt, outcome)) = event {
                state.lock_checked(attempt, outcome);
            }
        }) {
            Ok(_) => self.shell.lock.results = Some(sender),
            Err(error) => tracing::error!("passwords cannot be checked; the lock screen will not unlock: {}", error.error),
        }
        let (sender, decoded) = channel::channel();
        match self.loop_handle.insert_source(decoded, |event, _, state: &mut State| {
            if let channel::Event::Msg(decoded) = event {
                state.lock_background_decoded(decoded);
            }
        }) {
            Ok(_) => self.shell.lock.decodes = Some(sender),
            Err(error) => tracing::warn!("the lock screen shows no image: {}", error.error),
        }
    }

    /// The session was just locked with the built-in lock screen: it starts empty.
    pub fn shell_lock_started(&mut self) {
        let lock = &mut self.shell.lock;
        wipe(&mut lock.password);
        lock.password.reserve(PASSWORD_CAPACITY);
        // An attempt of an earlier lock answers too late to count.
        lock.checking = None;
        lock.message = None;
        lock.failed = false;
        lock.locks += 1;
        lock.focus = match &lock.greeter {
            Some(greeter) if greeter.user.text.is_empty() => LockTarget::User,
            _ => LockTarget::Password,
        };
        // lock.qml read the theme when it started: its colors and its image. The login screen's image is its own.
        lock.source = match &lock.greeter {
            Some(greeter) => greeter.background.clone(),
            None => {
                let json = splice_config::theme_state_path().and_then(|path| std::fs::read_to_string(path).ok());
                json.as_deref().and_then(splice_ui::theme::generated_source).map(PathBuf::from)
            }
        };
        self.shell_reload_theme();
        self.shell_release_pointer();
        self.shell.hover = None;
        self.shell.dirty = true;
    }

    /// The session was unlocked: the pictures go.
    pub fn shell_lock_ended(&mut self) {
        let lock = &mut self.shell.lock;
        lock.painted = None;
        lock.backgrounds.clear();
        lock.scanlines.clear();
        lock.requested.clear();
        lock.decoded.clear();
        lock.source = None;
        self.shell.hover = None;
    }

    /// What Tab moves through.
    fn lock_focus_chain(&self) -> Vec<LockTarget> {
        let greeter = self.shell.lock.greeter.as_ref();
        lock_screen::focus_chain(greeter.is_some(), greeter.map_or(0, |g| g.sessions.len()), greeter.is_some())
    }

    /// A key pressed on the lock or login screen: Tab moves the focus; a field takes typing, Backspace and Return
    /// (the user field also Delete, the arrows, Home and End); a button Return and Space. Nothing while a check
    /// runs.
    pub fn lock_key(&mut self, sym: Keysym, modified: bool) {
        if self.shell.lock.checking.is_some() {
            return;
        }
        let raw = sym.raw();
        if matches!(raw, keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab) {
            let chain = self.lock_focus_chain();
            let lock = &mut self.shell.lock;
            let position = chain.iter().position(|t| *t == lock.focus).unwrap_or(0);
            let next = if raw == keysyms::KEY_Tab { position + 1 } else { position + chain.len() - 1 };
            lock.focus = chain[next % chain.len()];
            self.shell.dirty = true;
            return;
        }
        let typed = sym.key_char().filter(|c| !c.is_control() && !modified);
        let lock = &mut self.shell.lock;
        match lock.focus {
            LockTarget::Password => match raw {
                keysyms::KEY_Return | keysyms::KEY_KP_Enter => return self.lock_submit(),
                keysyms::KEY_BackSpace => {
                    lock.password.pop();
                }
                _ => match typed {
                    Some(c) => lock.password.push(c),
                    None => return,
                },
            },
            LockTarget::User => {
                let Some(user) = lock.greeter.as_mut().map(|g| &mut g.user) else { return };
                match raw {
                    keysyms::KEY_Return | keysyms::KEY_KP_Enter => lock.focus = LockTarget::Password,
                    keysyms::KEY_BackSpace => user.backspace(),
                    keysyms::KEY_Delete | keysyms::KEY_KP_Delete => user.delete(),
                    keysyms::KEY_Left | keysyms::KEY_KP_Left => user.left(),
                    keysyms::KEY_Right | keysyms::KEY_KP_Right => user.right(),
                    keysyms::KEY_Home | keysyms::KEY_KP_Home => user.home(),
                    keysyms::KEY_End | keysyms::KEY_KP_End => user.end(),
                    _ => match typed {
                        Some(c) => user.insert(c.encode_utf8(&mut [0; 4])),
                        None => return,
                    },
                }
            }
            button => {
                if matches!(raw, keysyms::KEY_Return | keysyms::KEY_KP_Enter | keysyms::KEY_space) {
                    self.lock_clicked(button);
                }
                return;
            }
        }
        self.shell.dirty = true;
    }

    /// Check the typed password, unless there is none or a check runs; on the login screen, log in.
    pub fn lock_submit(&mut self) {
        if self.shell.lock.greeter.is_some() {
            return self.greeter_submit();
        }
        let lock = &mut self.shell.lock;
        if lock.checking.is_some() || lock.password.is_empty() {
            return;
        }
        lock.message = None;
        lock.failed = false;
        lock.attempts += 1;
        let attempt = lock.attempts;
        lock.checking = Some(attempt);
        let Some(results) = lock.results.clone() else {
            return self.lock_checked(attempt, None);
        };
        let password = lock.password.as_bytes().to_vec();
        let started = std::thread::Builder::new().name("password-check".into()).spawn(move || {
            let _ = results.send((attempt, check(password)));
        });
        if let Err(error) = started {
            tracing::error!("cannot check the password: {error}");
            self.lock_checked(attempt, None);
        }
        self.shell.dirty = true;
    }

    /// A check answered: only `success` for the current attempt, while the lock screen holds the session,
    /// unlocks. Like lock.qml shows it, every other outcome but too many tries reads as a wrong password.
    fn lock_checked(&mut self, attempt: u64, outcome: Option<AuthOutcome>) {
        let lock = &mut self.shell.lock;
        // An answer for an earlier attempt or an earlier lock counts for nothing; the login screen never unlocks.
        if lock.checking != Some(attempt) || !self.session_lock.is_own() || lock.greeter.is_some() {
            return;
        }
        lock.checking = None;
        wipe(&mut lock.password);
        if outcome == Some(AuthOutcome::Success) {
            return self.own_unlock();
        }
        lock.failed = true;
        lock.message = Some(match outcome {
            Some(AuthOutcome::MaxTries) => LockMessage::TooManyTries,
            _ => LockMessage::WrongPassword,
        });
        tracing::info!("the password was not accepted ({})", outcome.map_or("check failed", AuthOutcome::as_str));
        self.shell.dirty = true;
    }

    /// Paint the clock and the card again where what they show changed (once per event loop iteration).
    pub fn lock_refresh(&mut self, fonts: &mut Fonts, redraw: &mut Vec<Output>) {
        if !self.session_lock.is_own() {
            return;
        }
        let primary = self.primary_output_name();
        let output = self
            .outputs
            .iter()
            .find(|e| Some(e.output.name()) == primary)
            .or(self.outputs.first())
            .map(|e| e.output.clone());
        let Some((output, geo)) = output.and_then(|o| Some((o.clone(), self.space.output_geometry(&o)?))) else {
            if let Some(shown) = self.shell.lock.painted.take() {
                redraw.push(shown.output);
            }
            return;
        };
        let now = LocalTime::now();
        let lock = &self.shell.lock;
        let greeter = lock.greeter.as_ref();
        let model = LockModel {
            output: (geo.size.w as f32, geo.size.h as f32),
            time: now.as_ref().map(clock::lock_time).unwrap_or_default(),
            date: now.as_ref().map(|now| clock::lock_date(now, self.shell.time_language)).unwrap_or_default(),
            // lock.qml showed `$USER`.
            user: greeter.map_or_else(|| std::env::var("USER").unwrap_or_default(), |g| g.user.text.clone()),
            ask_user: greeter.map(|g| g.user.cursor),
            password_len: lock.password.chars().count(),
            busy: lock.checking.is_some(),
            message: lock.message.clone(),
            failed: lock.failed,
            focus: lock.focus,
            hover: match self.shell.hover {
                Some(ShellHit::Lock(target)) => Some(target),
                _ => None,
            },
            sessions: greeter.map(|g| g.sessions.iter().map(|s| s.name.clone()).collect()).unwrap_or_default(),
            session: greeter.map_or(0, |g| g.session),
            show_power: greeter.is_some(),
            language: self.shell.language,
        };
        let scale = output.current_scale().fractional_scale();
        if let Some(shown) = &lock.painted
            && shown.output == output
            && shown.scale == scale
            && shown.model == model
        {
            return;
        }
        if let Some(shown) = self.shell.lock.painted.take()
            && shown.output != output
        {
            redraw.push(shown.output);
        }
        let painted = lock_screen::scenes(&model, &self.shell.theme, fonts)
            .into_iter()
            .filter_map(|(scene, origin)| Some((paint(scene, fonts, scale)?, origin)))
            .collect();
        self.shell.lock.painted = Some(Shown { output: output.clone(), model, scale, painted });
        redraw.push(output);
    }

    /// Paint the clock and the card anew (the theme changed).
    pub fn lock_repaint(&mut self) {
        // `lock_refresh` paints it again before the next frame.
        self.shell.lock.painted = None;
    }

    /// What of the lock screen is under the pointer.
    pub fn lock_at_pointer(&self) -> Option<ShellHit> {
        let shown = self.shell.lock.painted.as_ref()?;
        let geo = self.space.output_geometry(&shown.output)?;
        if !geo.to_f64().contains(self.pointer_location) {
            return None;
        }
        let local = self.pointer_location - geo.loc.to_f64();
        shown
            .painted
            .iter()
            .rev()
            .find_map(|(painted, (x, y))| painted.scene.target_at((local.x as f32) - x, (local.y as f32) - y))
            .map(ShellHit::Lock)
    }

    /// A click (or Return on a button) on the lock or login screen: a field takes the keyboard, unless a check
    /// runs.
    pub fn lock_clicked(&mut self, target: LockTarget) {
        match target {
            LockTarget::Submit => self.lock_submit(),
            LockTarget::User | LockTarget::Password => {
                let lock = &mut self.shell.lock;
                if lock.checking.is_none() && (target == LockTarget::Password || lock.greeter.is_some()) {
                    lock.focus = target;
                    self.shell.dirty = true;
                }
            }
            LockTarget::Session(index) => self.greeter_pick_session(index),
            LockTarget::Reboot => self.greeter_power("reboot"),
            LockTarget::Poweroff => self.greeter_power("poweroff"),
            LockTarget::Card => {}
        }
    }

    fn lock_background_decoded(&mut self, (locks, size, pixels): Decoded) {
        let lock = &mut self.shell.lock;
        if locks != lock.locks || !self.session_lock.is_own() {
            return;
        }
        match pixels {
            Ok(pixels) => lock.decoded.push((size, pixels)),
            // The background color stays.
            Err(error) => {
                let path = lock.source.as_ref().map_or_else(String::new, |p| p.display().to_string());
                tracing::warn!("the lock screen cannot show {path}: {error}");
            }
        }
        self.queue_redraw_all();
    }

    /// Ask for the image in the sizes of the monitors that lack it, and upload what is ready. Runs before the
    /// elements of an output are built.
    pub fn lock_prepare(&mut self, renderer: &mut GlesRenderer) {
        if !self.session_lock.is_own() {
            return;
        }
        let lock = &mut self.shell.lock;
        if let (Some(source), Some(sender)) = (&lock.source, &lock.decodes) {
            let mut missing: Vec<Size<i32, Physical>> = Vec::new();
            for entry in &self.outputs {
                let Some(geo) = self.space.output_geometry(&entry.output) else { continue };
                let size = pixel_size(geo, &entry.output);
                if !lock.requested.contains(&size) && !missing.contains(&size) {
                    missing.push(size);
                }
            }
            if !missing.is_empty() {
                lock.requested.extend(&missing);
                let (path, sender, locks) = (source.clone(), sender.clone(), lock.locks);
                let spawned = std::thread::Builder::new().name("lock-background".into()).spawn(move || {
                    for decoded in decode(&path, &missing, locks) {
                        let _ = sender.send(decoded);
                    }
                });
                if let Err(error) = spawned {
                    tracing::warn!("the lock screen shows no image: {error}");
                }
            }
        }
        for (size, pixels) in std::mem::take(&mut lock.decoded) {
            let buffer = Size::<i32, Buffer>::from((size.w.max(1), size.h.max(1)));
            match renderer.import_memory(&pixels, Fourcc::Abgr8888, buffer, false) {
                Ok(texture) => {
                    lock.backgrounds.retain(|b| b.size != size);
                    lock.backgrounds.push(Background { size, texture, id: Id::new() });
                }
                Err(error) => tracing::warn!("cannot upload the lock screen's image: {error}"),
            }
        }
        if self.shell.theme.shape == splice_ui::Shape::Chamfer {
            for entry in &self.outputs {
                let Some(geo) = self.space.output_geometry(&entry.output) else { continue };
                let size = pixel_size(geo, &entry.output);
                if lock.scanlines.iter().any(|b| b.size == size) {
                    continue;
                }
                let pixels = lock_screen::scanline_pixels(size.w.max(1) as usize, size.h.max(1) as usize);
                let buffer = Size::<i32, Buffer>::from((size.w.max(1), size.h.max(1)));
                match renderer.import_memory(&pixels, Fourcc::Abgr8888, buffer, false) {
                    Ok(texture) => lock.scanlines.push(Background { size, texture, id: Id::new() }),
                    Err(error) => tracing::warn!("cannot upload the lock screen's scanlines: {error}"),
                }
            }
        }
    }

    /// What the lock screen shows on `output`, front to back: the clock and the card (on the primary monitor),
    /// the dimming, the image and, opaque whatever the theme says, the theme's background over all of it, so
    /// nothing behind the lock shows through.
    pub fn shell_lock_elements(&mut self, renderer: &mut GlesRenderer, output: &Output) -> Vec<OutputElement> {
        let Some(geo) = self.space.output_geometry(output) else { return Vec::new() };
        let scale = output.current_scale().fractional_scale();
        let area: Rectangle<i32, Physical> = geo.to_physical_precise_round(scale);
        let area = Rectangle::from_size(area.size);
        let mut elements = Vec::new();
        let chamfer = self.shell.theme.shape == splice_ui::Shape::Chamfer;
        let lock = &mut self.shell.lock;
        if let Some(shown) = lock.painted.as_ref().filter(|s| &s.output == output) {
            for (painted, (x, y)) in shown.painted.iter().rev() {
                elements.extend(element(renderer, painted, (f64::from(*x), f64::from(*y)), scale));
            }
        }
        let size = pixel_size(geo, output);
        // The cyberdeck's scanlines lie over the dimmed image, behind the card.
        if chamfer && let Some(lines) = lock.scanlines.iter().find(|b| b.size == size) {
            let src = Rectangle::<f64, Logical>::from_size((f64::from(size.w), f64::from(size.h)).into());
            elements.push(OutputElement::from(TextureRenderElement::from_static_texture(
                lines.id.clone(),
                renderer.context_id(),
                Point::<f64, Physical>::from((0.0, 0.0)),
                lines.texture.clone(),
                1,
                Transform::Normal,
                None,
                Some(src),
                Some(geo.size),
                None,
                Kind::Unspecified,
            )));
        }
        let solid = |id: &Id, commit: CommitCounter, color: [f32; 4]| {
            let element = SolidColorRenderElement::new(id.clone(), area, commit, color, Kind::Unspecified);
            CropRenderElement::from_element(element, scale, area).map(OutputElement::from)
        };
        let dim = lock.dim.get_or_insert_with(Id::new);
        elements.extend(solid(dim, CommitCounter::default(), [0.0, 0.0, 0.0, lock_screen::DIM]));
        if let Some(background) = lock.backgrounds.iter().find(|b| b.size == size) {
            let src = Rectangle::<f64, Logical>::from_size((f64::from(size.w), f64::from(size.h)).into());
            elements.push(OutputElement::from(TextureRenderElement::from_static_texture(
                background.id.clone(),
                renderer.context_id(),
                Point::<f64, Physical>::from((0.0, 0.0)),
                background.texture.clone(),
                1,
                Transform::Normal,
                None,
                Some(src),
                Some(geo.size),
                None,
                Kind::Unspecified,
            )));
        }
        let theme = self.shell.theme.background;
        let color = [f32::from(theme.r) / 255.0, f32::from(theme.g) / 255.0, f32::from(theme.b) / 255.0, 1.0];
        // A stable id: the backdrop is damaged only when its color changes.
        let (id, shown, commit) = lock.backdrop.get_or_insert_with(|| (Id::new(), color, CommitCounter::default()));
        if *shown != color {
            *shown = color;
            commit.increment();
        }
        elements.extend(solid(id, *commit, color));
        elements
    }
}
