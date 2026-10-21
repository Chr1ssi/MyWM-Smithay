//! The built-in shell (`builtin_shell = true`): the bar on every monitor, the power menu and the notices at the
//! top right, drawn by the compositor itself (`mywm-ui`) instead of mywm-shell's Quickshell bar.
//!
//! Pictures are painted on the CPU only when what they show changes, and uploaded as textures; a frame only
//! places them. What they show comes from the same state the bar protocol sends (so `ipc_dirty` says when
//! to look again), plus pointer hover, the clock and the power menu's countdown.
//!
//! In front-to-back order the power menu covers everything, the notices sit behind the clients' `Overlay`
//! layer and the bar behind fullscreen windows and the clients' `Top` layer, as their Quickshell windows did.
use std::time::{Duration, Instant};

use mywm_config::Action;
use mywm_ui::{
    Scene, Theme,
    bar::{self, BarModel, BarTarget, Workspace},
    clock::{self, LocalTime},
    i18n::Language,
    launcher::LauncherTarget,
    notices::{self, NoticeTarget, NoticesModel},
    notifications::CardTarget,
    audio::AudioTarget,
    panel::PanelTarget,
    tray_menu::MenuTarget,
    power::{self, PowerAction, PowerModel, PowerTarget},
    text::Fonts,
    theme::{BAR_HEIGHT, Palette},
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{Kind, memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement}},
            gles::GlesRenderer,
        },
    },
    input::keyboard::{Keycode, Keysym, keysyms},
    output::Output,
    reexports::calloop::{channel, timer::{TimeoutAction, Timer}},
    utils::{Logical, Physical, Point, Rectangle, Size, Transform},
};

use crate::{State, render::OutputElement};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
/// How often the power menu redraws while its card resizes or counts down.
const FRAME: Duration = Duration::from_millis(16);

/// A scene and its pixels, ready to be shown.
pub struct Painted<T> {
    pub scene: Scene<T>,
    buffer: MemoryRenderBuffer,
    /// Pixels of the buffer, for its whole size.
    pixels: (i32, i32),
}

struct Bar {
    output: Output,
    /// What `painted` shows, at `scale`.
    model: Option<BarModel>,
    scale: f64,
    painted: Option<Painted<BarTarget>>,
}

/// The open power menu.
pub struct PowerMenu {
    output: Output,
    pending: Option<PowerAction>,
    /// When the countdown of `pending` began.
    started: Instant,
    error: Option<String>,
    /// The card changes its size from the first to the second over `power::RESIZE_MS` from the instant.
    resize: ((f32, f32), (f32, f32), Instant),
    hover: Option<PowerTarget>,
    focus: Option<PowerTarget>,
    /// `systemctl` runs; the menu waits for its outcome.
    running: bool,
    /// Tells this menu apart from a later one, for the timer that keeps it moving.
    id: u64,
    /// That timer runs (it ends by itself once nothing moves or the menu is gone).
    ticking: bool,
    model: Option<PowerModel>,
    painted: Option<Painted<PowerTarget>>,
    /// The dimming over the whole monitor (one pixel, stretched).
    backdrop: MemoryRenderBuffer,
}

impl PowerMenu {
    fn size(&self, now: Instant) -> (f32, f32) {
        let (from, to, at) = self.resize;
        let t = power::ease_out_cubic(now.duration_since(at).as_secs_f32() * 1000.0 / power::RESIZE_MS as f32);
        (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
    }

    fn progress(&self, now: Instant) -> f32 {
        if self.pending.is_none() {
            return 1.0;
        }
        if self.running {
            return 0.0;
        }
        1.0 - now.duration_since(self.started).as_secs_f32() * 1000.0 / power::COUNTDOWN_MS as f32
    }

    /// Whether the picture still changes by itself.
    fn animating(&self, now: Instant) -> bool {
        (self.pending.is_some() && !self.running) || now.duration_since(self.resize.2).as_millis() < u128::from(power::RESIZE_MS)
    }
}

/// What the pointer is over, of the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellHit {
    Bar(usize, BarTarget),
    /// The power menu's card (`Some`) or the dimmed monitor around it.
    Power(Option<PowerTarget>),
    Notices(NoticeTarget),
    /// The open panel.
    Panel(PanelTarget),
    TrayMenu(MenuTarget),
    Launcher(LauncherTarget),
}

pub struct Shell {
    pub enabled: bool,
    pub fonts: Option<Fonts>,
    pub theme: Theme,
    pub language: Language,
    pub time_language: Language,
    clock: String,
    bars: Vec<Bar>,
    pub power: Option<PowerMenu>,
    notices: Option<(Output, NoticesModel, Painted<NoticeTarget>)>,
    /// The configuration error the user closed; it shows again once the error changes.
    dismissed_config_error: Option<String>,
    pub hover: Option<ShellHit>,
    /// A button went down on the shell; its release belongs to the shell too.
    pressed: Option<(u32, Option<ShellHit>)>,
    /// The audio node whose slider the pointer holds.
    pub dragging: Option<u32>,
    /// Something only the shell knows of changed (hover, clock, countdown, theme).
    pub dirty: bool,
    /// The outcome of `systemctl reboot|poweroff`.
    systemctl: Option<channel::Sender<i32>>,
    menus_opened: u64,
    /// Audio, media and Bluetooth from `mywm-shell-services`.
    pub services: crate::shell_services::Services,
    /// Night light, addresses and the command center's other own parts.
    pub control: crate::shell_control::Control,
    /// Notifications and their toasts.
    pub notes: crate::shell_notifications::Notes,
    /// The open panel.
    pub panel: Option<crate::shell_panels::OpenPanel>,
    /// The open menu of a tray item.
    pub tray_menu: Option<crate::shell_tray::TrayMenuOpen>,
    /// The calendar panel's month, events and form.
    pub calendar: crate::shell_calendar::Calendar,
    /// The open launcher, and the applications it lists.
    pub launcher: Option<crate::shell_launcher::Launcher>,
    pub apps: crate::shell_launcher::Apps,
    /// The key the shell repeats while it is held, by its press (see `shell_key_pressed`).
    repeating: Option<(Keycode, u64)>,
    presses: u64,
}

fn palette(config: &mywm_config::Config) -> Palette {
    let a = &config.appearance;
    let color = |c: mywm_config::HexColor| mywm_ui::Color::parse(&c.css()).unwrap_or(mywm_ui::Color::TRANSPARENT);
    Palette {
        background: color(a.background),
        surface: color(a.surface),
        text: color(a.text),
        muted: color(a.muted_text),
        accent: color(a.active_border),
        border: color(a.inactive_border),
    }
}

fn load_theme(config: &mywm_config::Config) -> Theme {
    let json = mywm_config::theme_state_path().and_then(|path| std::fs::read_to_string(path).ok());
    Theme::new(palette(config), json.as_deref())
}

fn clock_text(language: Language) -> (String, Duration) {
    match LocalTime::now() {
        Some(now) => (clock::bar_text(&now, language), Duration::from_secs(now.seconds_to_next_minute().into())),
        None => (String::new(), Duration::from_secs(60)),
    }
}

pub fn paint<T: Clone>(scene: Scene<T>, fonts: &mut Fonts, scale: f64) -> Option<Painted<T>> {
    let pixmap = mywm_ui::paint::paint(&scene, fonts, scale as f32)?;
    let pixels = (pixmap.width() as i32, pixmap.height() as i32);
    let buffer = MemoryRenderBuffer::from_slice(pixmap.data(), Fourcc::Abgr8888, pixels, 1, Transform::Normal, None);
    Some(Painted { scene, buffer, pixels })
}

/// `painted` at `location` (logical, relative to the output) on an output at `scale`.
pub fn element<T>(renderer: &mut GlesRenderer, painted: &Painted<T>, location: (f64, f64), scale: f64) -> Option<OutputElement> {
    let size = Size::<i32, Logical>::from((painted.scene.width.round() as i32, painted.scene.height.round() as i32));
    stretched(renderer, &painted.buffer, painted.pixels, location, size, scale)
}

fn stretched(
    renderer: &mut GlesRenderer,
    buffer: &MemoryRenderBuffer,
    pixels: (i32, i32),
    location: (f64, f64),
    size: Size<i32, Logical>,
    scale: f64,
) -> Option<OutputElement> {
    let location = Point::<f64, Physical>::from((location.0 * scale, location.1 * scale));
    // The buffer has scale 1: its `src` is in pixels, and `size` says how large it shows.
    let src = Rectangle::<f64, Logical>::from_size((f64::from(pixels.0), f64::from(pixels.1)).into());
    match MemoryRenderBufferRenderElement::from_buffer(renderer, location, buffer, None, Some(src), Some(size), Kind::Unspecified) {
        Ok(element) => Some(OutputElement::from(element)),
        Err(error) => {
            tracing::warn!("cannot upload a picture of the shell: {error}");
            None
        }
    }
}

impl Shell {
    pub fn new(config: &mywm_config::Config) -> Self {
        let time_language = clock::time_language();
        Self {
            enabled: config.builtin_shell,
            fonts: None,
            theme: load_theme(config),
            language: Language::from_env(),
            time_language,
            clock: clock_text(time_language).0,
            bars: Vec::new(),
            power: None,
            notices: None,
            dismissed_config_error: None,
            hover: None,
            pressed: None,
            dragging: None,
            dirty: true,
            systemctl: None,
            menus_opened: 0,
            services: Default::default(),
            control: Default::default(),
            notes: Default::default(),
            panel: None,
            tray_menu: None,
            calendar: Default::default(),
            launcher: None,
            apps: Default::default(),
            repeating: None,
            presses: 0,
        }
    }
}

impl State {
    /// Start the shell's helpers: the fonts load on a worker thread, the clock ticks every minute.
    pub fn shell_init(&mut self) {
        if !self.shell.enabled {
            return;
        }
        let (sender, fonts) = channel::channel();
        let watched = self.loop_handle.insert_source(fonts, |event, _, state: &mut State| {
            if let channel::Event::Msg(fonts) = event {
                state.shell.fonts = Some(fonts);
                state.shell.dirty = true;
            }
        });
        let spawned = watched.is_ok()
            && std::thread::Builder::new()
                .name("shell-fonts".into())
                .spawn(move || {
                    let _ = sender.send(Fonts::load());
                })
                .is_ok();
        if !spawned {
            tracing::error!("cannot load the fonts of the shell; the bar stays empty");
        }
        let (_, next) = clock_text(self.shell.time_language);
        let ticking = self.loop_handle.insert_source(Timer::from_duration(next), |_, _, state| {
            let (text, next) = clock_text(state.shell.time_language);
            if text != state.shell.clock {
                state.shell.clock = text;
                state.shell.dirty = true;
            }
            TimeoutAction::ToDuration(next)
        });
        if let Err(error) = ticking {
            tracing::warn!("the clock of the bar cannot tick: {}", error.error);
        }
        let (sender, results) = channel::channel();
        match self.loop_handle.insert_source(results, |event, _, state: &mut State| {
            if let channel::Event::Msg(code) = event {
                state.power_finished(code);
            }
        }) {
            Ok(_) => self.shell.systemctl = Some(sender),
            Err(error) => tracing::warn!("restart and shut down are not available: {}", error.error),
        }
        self.services_start();
        self.night_light_restore();
    }

    /// Something the shell shows changed outside the compositor's own state.
    pub fn shell_changed(&mut self) {
        self.shell.dirty = true;
    }

    /// Re-read the generated colors (after the wallpaper picker or a config reload).
    pub fn shell_reload_theme(&mut self) {
        if self.shell.enabled {
            self.shell.theme = load_theme(&self.config);
            for bar in &mut self.shell.bars {
                bar.model = None;
            }
            self.shell.notices = None;
            if let Some(menu) = &mut self.shell.power {
                menu.model = None;
            }
            self.panel_repaint();
            self.tray_menu_repaint();
            self.launcher_repaint();
            self.shell.dirty = true;
        }
    }

    /// How much of the top of every monitor the bar keeps free of windows.
    pub fn shell_reserved_top(&self) -> i32 {
        if self.shell.enabled { BAR_HEIGHT as i32 } else { 0 }
    }

    /// Bring the pictures up to date with what they show; called once per event loop iteration, before the
    /// bar protocol's clients get the same news.
    pub fn shell_refresh(&mut self) {
        if !self.shell.enabled || self.shell.fonts.is_none() || !(self.ipc_dirty || self.shell.dirty) {
            return;
        }
        self.toasts_for_gaming();
        self.shell.dirty = false;
        let Some(mut fonts) = self.shell.fonts.take() else { return };
        let now = Instant::now();
        let mut redraw: Vec<Output> = Vec::new();

        let outputs: Vec<Output> = self.outputs.iter().map(|e| e.output.clone()).collect();
        self.shell.bars.retain(|bar| outputs.contains(&bar.output));
        for (monitor, output) in outputs.iter().enumerate() {
            let Some(model) = self.bar_model(monitor, output) else { continue };
            let scale = output.current_scale().fractional_scale();
            let index = match self.shell.bars.iter().position(|b| &b.output == output) {
                Some(index) => index,
                None => {
                    self.shell.bars.push(Bar { output: output.clone(), model: None, scale, painted: None });
                    self.shell.bars.len() - 1
                }
            };
            let bar = &mut self.shell.bars[index];
            if bar.model.as_ref() == Some(&model) && bar.scale == scale {
                continue;
            }
            bar.painted = paint(bar::scene(&model, &self.shell.theme, &mut fonts), &mut fonts, scale);
            bar.model = Some(model);
            bar.scale = scale;
            redraw.push(output.clone());
        }

        let first = outputs.first().cloned();
        let model = first.as_ref().and_then(|output| Some((output.clone(), self.notices_model(output)?)));
        let current = self.shell.notices.as_ref().map(|(output, model, _)| (output.clone(), model.clone()));
        if model != current {
            if let Some((output, _)) = &current {
                redraw.push(output.clone());
            }
            self.shell.notices = model.and_then(|(output, model)| {
                let scene = notices::scene(&model, &self.shell.theme, &mut fonts)?;
                let painted = paint(scene, &mut fonts, output.current_scale().fractional_scale())?;
                redraw.push(output.clone());
                Some((output, model, painted))
            });
        }

        self.panel_refresh(&mut fonts, &mut redraw);
        self.tray_menu_refresh(&mut fonts, &mut redraw);
        self.launcher_refresh(&mut fonts, &mut redraw);

        if let Some(menu) = &mut self.shell.power {
            let model = PowerModel {
                pending: menu.pending,
                progress: menu.progress(now),
                error: menu.error.clone(),
                size: menu.size(now),
                hover: menu.hover,
                focus: menu.focus,
                language: self.shell.language,
            };
            if menu.model.as_ref() != Some(&model) {
                let scale = menu.output.current_scale().fractional_scale();
                menu.painted = paint(power::scene(&model, &self.shell.theme, &mut fonts), &mut fonts, scale);
                menu.model = Some(model);
                redraw.push(menu.output.clone());
            }
        }

        self.shell.fonts = Some(fonts);
        for output in redraw {
            self.queue_redraw_output(&output);
        }
    }

    fn bar_model(&self, monitor: usize, output: &Output) -> Option<BarModel> {
        let geo = self.space.output_geometry(output)?;
        let desk = self.desktop.desk.monitors.get(monitor)?;
        let workspaces = desk
            .workspaces
            .entries
            .iter()
            .map(|w| Workspace { number: w.number as u32, occupied: !w.windows.is_empty() })
            .collect();
        let hover = match self.shell.hover {
            Some(ShellHit::Bar(m, target)) if m == monitor => Some(target),
            _ => None,
        };
        Some(BarModel {
            width: geo.size.w as f32,
            workspaces,
            active: Some(desk.workspaces.active as u32),
            scratchpad_visible: self.desktop.scratchpad_visible,
            scratchpad_occupied: !self.desktop.scratchpad.windows.is_empty(),
            media: self.playing_title(),
            clock: self.shell.clock.clone(),
            capturing: self.capturing(),
            tray: self.tray_icons(),
            control_icon: bar::control_icon(
                self.shell.services.status.bluetooth.as_ref().map(|b| (b.powered, b.connected())),
                self.shell.control.night_light,
            ),
            notifications: self.notification_count(),
            audio: self.shell.services.status.default_audio().map(|a| bar::Audio { volume: a.volume, muted: a.muted }),
            // The other panels come with their own steps.
            open_panel: self.panel_on(output),
            power_open: self.shell.power.as_ref().is_some_and(|m| &m.output == output),
            hover,
            language: self.shell.language,
        })
    }

    fn notices_model(&self, output: &Output) -> Option<NoticesModel> {
        // The panels on the right and a tray menu would show them through their glass.
        if matches!(self.panel_on(output), Some(bar::Panel::Notifications | bar::Panel::Audio)) || self.tray_menu_on(output) {
            return None;
        }
        let geo = self.space.output_geometry(output)?;
        let config_error = self.config_error.clone().filter(|error| Some(error) != self.shell.dismissed_config_error.as_ref());
        let hover = match self.shell.hover {
            Some(ShellHit::Notices(target)) => Some(target),
            _ => None,
        };
        let model = NoticesModel {
            output_width: geo.size.w as f32,
            choosing: self.share_kinds().is_some(),
            picking: self.picked_color(),
            config_error,
            toasts: self.toast_views(),
            hover,
            language: self.shell.language,
        };
        (model.choosing || model.picking.is_some() || model.config_error.is_some() || !model.toasts.is_empty()).then_some(model)
    }

    /// Where the power menu's card is on its output (logical, relative to the output).
    fn power_card_origin(&self, menu: &PowerMenu) -> Option<(f64, f64)> {
        let geo = self.space.output_geometry(&menu.output)?;
        let painted = menu.painted.as_ref()?;
        let x = ((f64::from(geo.size.w) - f64::from(painted.scene.width)) / 2.0).round();
        let y = ((f64::from(geo.size.h) - f64::from(painted.scene.height)) / 2.0).round();
        Some((x, y))
    }

    fn notices_origin(&self, output: &Output, painted: &Painted<NoticeTarget>) -> Option<(f64, f64)> {
        let geo = self.space.output_geometry(output)?;
        Some((f64::from(geo.size.w) - f64::from(notices::RIGHT) - f64::from(painted.scene.width), f64::from(notices::TOP)))
    }

    /// The bar of `output`, front to back.
    pub fn shell_bar_elements(&self, renderer: &mut GlesRenderer, output: &Output) -> Vec<OutputElement> {
        let scale = output.current_scale().fractional_scale();
        let bar = self.shell.bars.iter().find(|b| &b.output == output).and_then(|b| b.painted.as_ref());
        bar.and_then(|painted| element(renderer, painted, (0.0, 0.0), scale)).into_iter().collect()
    }

    /// The launcher, the open panel and the notices on `output`, front to back.
    pub fn shell_notice_elements(&self, renderer: &mut GlesRenderer, output: &Output) -> Vec<OutputElement> {
        let mut elements: Vec<OutputElement> = self.launcher_elements(renderer, output).into_iter().collect();
        elements.extend(self.panel_elements(renderer, output));
        elements.extend(self.tray_menu_elements(renderer, output));
        let Some((on, _, painted)) = self.shell.notices.as_ref().filter(|(on, _, _)| on == output) else { return elements };
        let scale = on.current_scale().fractional_scale();
        let origin = self.notices_origin(on, painted);
        elements.extend(origin.and_then(|origin| element(renderer, painted, origin, scale)));
        elements
    }

    /// The power menu on `output` and the dimming behind it, front to back.
    pub fn shell_power_elements(&self, renderer: &mut GlesRenderer, output: &Output) -> Vec<OutputElement> {
        let Some(menu) = self.shell.power.as_ref().filter(|m| &m.output == output) else { return Vec::new() };
        let Some(geo) = self.space.output_geometry(output) else { return Vec::new() };
        let scale = output.current_scale().fractional_scale();
        let card = menu
            .painted
            .as_ref()
            .zip(self.power_card_origin(menu))
            .and_then(|(painted, origin)| element(renderer, painted, origin, scale));
        let backdrop = stretched(renderer, &menu.backdrop, (1, 1), (0.0, 0.0), geo.size, scale);
        card.into_iter().chain(backdrop).collect()
    }

    /// What of the shell is at the pointer, among the parts in front of the clients' `Overlay` layer.
    pub fn shell_power_at_pointer(&self) -> Option<ShellHit> {
        let menu = self.shell.power.as_ref()?;
        let geo = self.space.output_geometry(&menu.output)?;
        if !geo.to_f64().contains(self.pointer_location) {
            return None;
        }
        let local = self.pointer_location - geo.loc.to_f64();
        let card = self.power_card_origin(menu).zip(menu.painted.as_ref());
        let target = card.and_then(|((x, y), painted)| painted.scene.target_at((local.x - x) as f32, (local.y - y) as f32));
        Some(ShellHit::Power(target))
    }

    /// The launcher, the open panel or the notices at the pointer (they lie behind the clients' `Overlay` layer).
    pub fn shell_notices_at_pointer(&self) -> Option<ShellHit> {
        if let Some(hit) = self.launcher_at_pointer().or_else(|| self.panel_at_pointer()).or_else(|| self.tray_menu_at_pointer()) {
            return Some(hit);
        }
        let (output, _, painted) = self.shell.notices.as_ref()?;
        let geo = self.space.output_geometry(output)?;
        let (x, y) = self.notices_origin(output, painted)?;
        let local = self.pointer_location - geo.loc.to_f64();
        painted.scene.target_at((local.x - x) as f32, (local.y - y) as f32).map(ShellHit::Notices)
    }

    /// The bar at the pointer (it lies behind fullscreen windows and the clients' `Top` layer).
    pub fn shell_bar_at_pointer(&self) -> Option<ShellHit> {
        let monitor = self.outputs.iter().position(|e| {
            self.space.output_geometry(&e.output).is_some_and(|geo| geo.to_f64().contains(self.pointer_location))
        })?;
        let output = &self.outputs[monitor].output;
        let geo = self.space.output_geometry(output)?;
        let painted = self.shell.bars.iter().find(|b| &b.output == output)?.painted.as_ref()?;
        let local = self.pointer_location - geo.loc.to_f64();
        painted.scene.target_at(local.x as f32, local.y as f32).map(|target| ShellHit::Bar(monitor, target))
    }

    /// Whether a button pressed on the shell is still down (its release belongs to the shell).
    pub fn shell_holds_pointer(&self) -> bool {
        self.shell.pressed.is_some()
    }

    /// The pointer moved; `hit` is what of the shell is under it now.
    pub fn shell_hover(&mut self, hit: Option<ShellHit>) {
        if hit == self.shell.hover {
            return;
        }
        if self.shell.hover.is_none() && hit.is_some() {
            // The client the pointer left may have set another cursor (a text cursor).
            self.cursor_status = smithay::input::pointer::CursorImageStatus::default_named();
            self.queue_redraw_cursor();
        }
        self.shell.hover = hit;
        self.toasts_hovered(hit);
        if let Some(menu) = &mut self.shell.power {
            menu.hover = match hit {
                Some(ShellHit::Power(target)) => target.filter(|t| *t != PowerTarget::Card),
                _ => None,
            };
        }
        self.shell.dirty = true;
    }

    /// A button changed on the shell (`hit`: what is under the pointer). Like a QML `MouseArea`, a click
    /// happens when the button is released over what it was pressed on.
    pub fn shell_button(&mut self, button: u32, pressed: bool, hit: Option<ShellHit>) {
        if pressed {
            if self.shell.pressed.is_none() {
                self.shell.pressed = Some((button, hit));
                if let (BTN_LEFT, Some(ShellHit::Panel(PanelTarget::Audio(AudioTarget::Slider(id))))) = (button, hit) {
                    self.shell.dragging = Some(id);
                    self.slider_dragged();
                }
                // Like a click on a panel that takes no keyboard: the clients get it back.
                self.note_click(None);
                if !matches!(hit, Some(ShellHit::Panel(_) | ShellHit::TrayMenu(_) | ShellHit::Bar(..) | ShellHit::Launcher(_))) {
                    self.panel_pressed_elsewhere();
                }
            }
            return;
        }
        let Some((down, on)) = self.shell.pressed else { return };
        if down != button {
            return;
        }
        self.shell.pressed = None;
        if self.shell.dragging.take().is_some() {
            return;
        }
        if on != hit {
            return;
        }
        if let Some(ShellHit::Bar(monitor, BarTarget::Tray(index))) = hit {
            return self.tray_clicked(monitor, index, button);
        }
        if button == BTN_RIGHT {
            // A `BarButton`'s secondary click: only the audio button has one (mute).
            if matches!(hit, Some(ShellHit::Bar(_, BarTarget::Panel(bar::Panel::Audio)))) {
                self.audio_toggle_mute();
            }
            return;
        }
        if button != BTN_LEFT {
            return;
        }
        match hit {
            Some(ShellHit::Bar(monitor, target)) => self.bar_clicked(monitor, target),
            Some(ShellHit::Power(None)) => self.power_cancel(),
            Some(ShellHit::Power(Some(target))) => self.power_activate(target),
            Some(ShellHit::Notices(NoticeTarget::DismissConfigError)) => {
                self.shell.dismissed_config_error = self.config_error.clone();
                self.shell.dirty = true;
            }
            Some(ShellHit::Notices(NoticeTarget::Toast(CardTarget::Dismiss(id)))) => self.notification_dismiss(id),
            Some(ShellHit::Notices(NoticeTarget::Toast(CardTarget::Action(id, index)))) => self.notification_invoke(id, index),
            Some(ShellHit::Panel(target)) => self.panel_clicked(target),
            Some(ShellHit::TrayMenu(target)) => self.tray_menu_clicked(target),
            Some(ShellHit::Launcher(target)) => self.launcher_clicked(target),
            Some(ShellHit::Notices(NoticeTarget::Background | NoticeTarget::Toast(CardTarget::Card(_)))) | None => {}
        }
    }

    /// The wheel turned `steps` notches (positive: away from the user) over `hit`: over the audio button that
    /// changes the volume; elsewhere on the shell it does nothing, and no window gets it either.
    pub fn shell_scroll(&mut self, hit: ShellHit, steps: f32) {
        match hit {
            ShellHit::Panel(target) => return self.panel_scroll(&target, steps),
            ShellHit::TrayMenu(_) => return self.tray_menu_scroll(steps),
            ShellHit::Launcher(_) => return self.launcher_scroll(steps),
            ShellHit::Bar(_, BarTarget::Tray(index)) => return self.tray_scroll(index, steps),
            _ => {}
        }
        if !matches!(hit, ShellHit::Bar(_, BarTarget::Panel(bar::Panel::Audio))) {
            return;
        }
        let status = &self.shell.services.status;
        let (Some(id), Some(audio)) = (status.default_sink, status.default_audio()) else { return };
        self.node_set_volume(id, bar::scrolled_volume(audio.volume, steps));
    }

    fn audio_toggle_mute(&mut self) {
        if let Some(id) = self.shell.services.status.default_sink {
            self.node_toggle_mute(id);
        }
    }

    fn bar_clicked(&mut self, monitor: usize, target: BarTarget) {
        match target {
            BarTarget::Workspace(number) => self.select_workspace_on(monitor, number as usize),
            BarTarget::NewWorkspace => self.new_workspace_on(monitor),
            BarTarget::Scratchpad => self.run_action(Action::ToggleScratchpad),
            BarTarget::Power => {
                let output = self.outputs.get(monitor).map(|e| e.output.clone());
                match (&self.shell.power, output) {
                    (Some(menu), Some(output)) if menu.output == output => self.power_cancel(),
                    (_, Some(output)) => self.power_open(output, None),
                    _ => {}
                }
            }
            BarTarget::Panel(kind) => {
                if let Some(output) = self.outputs.get(monitor).map(|e| e.output.clone()) {
                    self.panel_toggle(kind, output);
                }
            }
            // Tray icons have their own clicks (`tray_clicked`).
            BarTarget::Tray(_) | BarTarget::Background => {}
        }
    }

    /// Whether the shell holds the keyboard (the power menu is open, or the notification center was clicked).
    pub fn shell_takes_keyboard(&self) -> bool {
        self.shell.power.is_some() || self.launcher_takes_keyboard() || self.panel_takes_keyboard() || self.tray_menu_takes_keyboard()
    }

    /// A key pressed for the shell (`shell_key`), repeated while it is held if the keymap repeats it, at the
    /// configured delay and rate: clients repeat keys themselves, and the Quickshell windows did.
    pub fn shell_key_pressed(&mut self, keycode: Keycode, sym: Keysym, modified: bool, repeats: bool) {
        self.shell.presses += 1;
        self.shell.repeating = None;
        self.shell_key(sym, modified);
        if !repeats || !self.shell_takes_keyboard() {
            return;
        }
        let press = self.shell.presses;
        self.shell.repeating = Some((keycode, press));
        let delay = Duration::from_millis(self.config.input.repeat_delay.max(1) as u64);
        let interval = Duration::from_millis((1000 / self.config.input.repeat_rate.max(1)).max(1) as u64);
        let timer = self.loop_handle.insert_source(Timer::from_duration(delay), move |_, _, state| {
            if state.shell.repeating != Some((keycode, press)) {
                return TimeoutAction::Drop;
            }
            if !state.shell_takes_keyboard() {
                state.shell.repeating = None;
                return TimeoutAction::Drop;
            }
            state.shell_key(sym, modified);
            TimeoutAction::ToDuration(interval)
        });
        if let Err(error) = timer {
            tracing::warn!("cannot repeat the key: {}", error.error);
        }
    }

    /// A key went up: a repeat of it ends.
    pub fn shell_key_released(&mut self, keycode: Keycode) {
        if self.shell.repeating.is_some_and(|(held, _)| held == keycode) {
            self.shell.repeating = None;
        }
    }

    /// A key pressed while the shell holds the keyboard; `modified`: with Ctrl, Alt or Super.
    pub fn shell_key(&mut self, sym: Keysym, modified: bool) {
        let Some(menu) = &mut self.shell.power else {
            if self.shell.launcher.is_some() {
                return self.launcher_key(sym, modified);
            }
            if self.calendar_key(sym, modified) {
                return;
            }
            if sym.raw() == keysyms::KEY_Escape {
                self.panel_close();
                self.tray_menu_close();
            }
            return;
        };
        match sym.raw() {
            keysyms::KEY_Escape => self.power_cancel(),
            keysyms::KEY_Return | keysyms::KEY_KP_Enter => match menu.focus {
                Some(target) => self.power_activate(target),
                None if menu.pending.is_some() => self.power_execute(),
                None => {}
            },
            keysyms::KEY_space => {
                if let Some(target) = menu.focus {
                    self.power_activate(target);
                }
            }
            raw @ (keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab) => {
                let chain = power::focus_chain(menu.pending.is_some());
                let position = menu.focus.and_then(|f| chain.iter().position(|t| *t == f));
                let next = match (raw == keysyms::KEY_Tab, position) {
                    (true, Some(i)) => (i + 1) % chain.len(),
                    (false, Some(i)) => (i + chain.len() - 1) % chain.len(),
                    (true, None) => 0,
                    (false, None) => chain.len() - 1,
                };
                menu.focus = chain.get(next).copied();
                self.shell.dirty = true;
            }
            _ => {}
        }
    }

    /// The exit binding: ask on the monitor under the pointer, counting down to the logout.
    pub fn shell_confirm_logout(&mut self) {
        let monitor = self.pointer_monitor().unwrap_or(self.desktop.focused_monitor);
        if let Some(output) = self.outputs.get(monitor).or(self.outputs.first()).map(|e| e.output.clone()) {
            self.power_open(output, Some(PowerAction::Logout));
        }
    }

    /// Open the power menu on `output`, counting down to `pending` if given.
    fn power_open(&mut self, output: Output, pending: Option<PowerAction>) {
        let Some(geo) = self.space.output_geometry(&output) else { return };
        let monitor = (geo.size.w as f32, geo.size.h as f32);
        let size = power::card_size(false, monitor);
        let now = Instant::now();
        self.power_close();
        // As the QML bar: the power menu closes the panel and the tray menu.
        self.panel_close();
        self.tray_menu_close();
        let pixel = [power::BACKDROP.r, power::BACKDROP.g, power::BACKDROP.b, power::BACKDROP.a];
        // Premultiplied: the color is black, so only the alpha stays.
        let premultiplied = [0, 0, 0, pixel[3]];
        self.shell.menus_opened += 1;
        self.shell.power = Some(PowerMenu {
            output,
            pending: None,
            started: now,
            error: None,
            // Opening shows the card at its size at once.
            resize: (size, size, now - Duration::from_millis(power::RESIZE_MS.into())),
            hover: None,
            focus: None,
            running: false,
            id: self.shell.menus_opened,
            ticking: false,
            model: None,
            painted: None,
            backdrop: MemoryRenderBuffer::from_slice(&premultiplied, Fourcc::Abgr8888, (1, 1), 1, Transform::Normal, None),
        });
        if let Some(action) = pending {
            self.power_choose(action);
        }
        self.shell.dirty = true;
        self.sync_focus();
    }

    /// A tile or the menu's button was activated (clicked, or Enter/Space on the focused one).
    fn power_activate(&mut self, target: PowerTarget) {
        match target {
            PowerTarget::Tile(PowerAction::Lock) => {
                self.power_close();
                self.run_action(Action::Lock);
            }
            PowerTarget::Tile(action) => self.power_choose(action),
            PowerTarget::Cancel => self.power_cancel(),
            PowerTarget::Card => {}
        }
    }

    /// Count down to `action`.
    fn power_choose(&mut self, action: PowerAction) {
        let Some(geo) = self.shell.power.as_ref().and_then(|m| self.space.output_geometry(&m.output)) else { return };
        let Some(menu) = &mut self.shell.power else { return };
        let now = Instant::now();
        let from = menu.size(now);
        menu.pending = Some(action);
        menu.error = None;
        menu.started = now;
        menu.focus = None;
        menu.resize = (from, power::card_size(true, (geo.size.w as f32, geo.size.h as f32)), now);
        self.shell.dirty = true;
        self.power_tick();
    }

    /// Keep the menu's picture moving while it resizes or counts down, and act when the countdown ends.
    fn power_tick(&mut self) {
        let Some(menu) = &mut self.shell.power else { return };
        if menu.ticking {
            return;
        }
        let id = menu.id;
        let ticker = self.loop_handle.insert_source(Timer::from_duration(FRAME), move |_, _, state: &mut State| {
            let now = Instant::now();
            let Some(menu) = state.shell.power.as_mut().filter(|m| m.id == id) else { return TimeoutAction::Drop };
            state.shell.dirty = true;
            if menu.pending.is_some() && !menu.running && menu.progress(now) <= 0.0 {
                menu.ticking = false;
                state.power_execute();
                return TimeoutAction::Drop;
            }
            if menu.animating(now) {
                TimeoutAction::ToDuration(FRAME)
            } else {
                menu.ticking = false;
                TimeoutAction::Drop
            }
        });
        match ticker {
            Ok(_) => menu.ticking = true,
            Err(error) => tracing::warn!("the power menu cannot count down: {}", error.error),
        }
    }

    /// Carry out the pending action now.
    fn power_execute(&mut self) {
        let Some(menu) = &mut self.shell.power else { return };
        let Some(action) = menu.pending.filter(|_| !menu.running) else { return };
        self.shell.dirty = true;
        match action.systemctl() {
            None => {
                self.power_close();
                if action == PowerAction::Logout {
                    tracing::info!("logging out from the power menu");
                    self.loop_signal.stop();
                }
            }
            Some(verb) => {
                menu.running = true;
                let Some(sender) = self.shell.systemctl.clone() else { return self.power_finished(-1) };
                let mut command = std::process::Command::new("systemctl");
                command.arg(verb);
                match command.spawn() {
                    Ok(mut child) => {
                        std::thread::spawn(move || {
                            let code = child.wait().map_or(-1, |status| status.code().unwrap_or(-1));
                            let _ = sender.send(code);
                        });
                    }
                    Err(error) => {
                        tracing::warn!("cannot run systemctl {verb}: {error}");
                        self.power_finished(-1);
                    }
                }
            }
        }
    }

    /// `systemctl` ended with `code`.
    fn power_finished(&mut self, code: i32) {
        let language = self.shell.language;
        let Some(menu) = &mut self.shell.power else { return };
        menu.running = false;
        if code == 0 {
            self.power_close();
            return;
        }
        tracing::warn!("systemctl {:?} failed with {code}", menu.pending.and_then(PowerAction::systemctl));
        menu.pending = None;
        menu.error = Some(power::failure(language, code));
        self.shell.dirty = true;
    }

    /// Escape, a click beside the card or "Cancel": stop the countdown and close.
    fn power_cancel(&mut self) {
        if self.shell.power.as_ref().is_some_and(|m| m.running) {
            return;
        }
        self.power_close();
    }

    fn power_close(&mut self) {
        let Some(menu) = self.shell.power.take() else { return };
        if matches!(self.shell.hover, Some(ShellHit::Power(_))) {
            self.shell.hover = None;
        }
        self.queue_redraw_output(&menu.output);
        self.shell.dirty = true;
        self.sync_focus();
        self.refresh_pointer_focus();
    }
}
