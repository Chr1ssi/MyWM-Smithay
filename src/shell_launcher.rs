//! The launcher of the built-in shell (`shell.qml`), for the launcher binding while `launcher` is splice's own: the
//! applications come from the desktop entries, read by `splice-shell-services` (at its start and at every opening, so
//! new ones show up), their icons after them. Open, it holds the keyboard; clicks beside it go to the windows, as
//! they went past the Quickshell window. Starting an application, Escape or the binding of a lock closes it.
use std::collections::HashMap;

use splice_shell_protocol::{Application, Command, Image};
use splice_ui::{
    edit::Input,
    launcher::{self, AppView, LauncherModel, LauncherTarget, Searchable},
    panel::SCROLL_STEP,
    scene::Picture,
    text::Fonts,
};
use smithay::{
    backend::renderer::gles::GlesRenderer,
    input::keyboard::{Keysym, keysyms},
    output::Output,
};

use crate::{
    State,
    render::OutputElement,
    shell::{Painted, ShellHit, element, paint},
};

/// The applications as the services process read them, and the icons it sent.
#[derive(Default)]
pub struct Apps {
    list: Vec<Application>,
    icons: HashMap<String, Picture>,
}

pub struct Launcher {
    output: Output,
    query: Input,
    selected: usize,
    scroll: f32,
    /// What `results` was computed for (the query and the list), so a new list keeps the selection only when it
    /// matches the same.
    results: Vec<usize>,
    model: Option<LauncherModel>,
    painted: Option<Painted<LauncherTarget>>,
}

impl State {
    /// Whether the launcher binding opens the built-in launcher rather than a program of the configuration.
    pub fn builtin_launcher(&self) -> bool {
        self.shell.enabled && self.config.launcher.is_none()
    }

    /// Open the launcher on the monitor under the pointer; an open one stays as it is (as `--no-duplicate` did).
    pub fn launcher_open(&mut self) {
        if self.shell.launcher.is_some() {
            return;
        }
        // The monitor under the pointer (`SPLICE_LAUNCHER_X`/`_Y`), else the first.
        let under = self.space.output_under(self.pointer_location).next().cloned();
        let Some(output) = under.or_else(|| self.outputs.first().map(|e| e.output.clone())) else { return };
        // The panel, the tray menu and the wallpaper picker lost the keyboard to it, which closed their windows.
        self.panel_close();
        self.tray_menu_close();
        self.wallpaper_picker_close();
        self.shell.launcher = Some(Launcher { output, query: Input::default(), selected: 0, scroll: 0.0, results: Vec::new(), model: None, painted: None });
        self.services_send(Command::Applications);
        self.sync_focus();
        self.shell_changed();
    }

    pub fn launcher_close(&mut self) {
        if let Some(launcher) = self.shell.launcher.take() {
            self.queue_redraw_output(&launcher.output);
            self.sync_focus();
            self.shell_changed();
        }
    }

    pub fn launcher_takes_keyboard(&self) -> bool {
        self.shell.launcher.is_some()
    }

    /// New applications from the services process.
    pub fn shell_apps(&mut self, apps: Vec<Application>) {
        self.shell.apps.list = apps;
        self.shell_changed();
    }

    pub fn shell_app_icon(&mut self, name: String, image: Image) {
        if let Some(picture) = Picture::from_rgba(image.width, image.height, &image.rgba) {
            self.shell.apps.icons.insert(name, picture);
            self.shell_changed();
        }
    }

    /// The services process ended: the next one sends the icons again.
    pub fn shell_apps_reset(&mut self) {
        self.shell.apps.icons.clear();
    }

    fn launcher_results(&self, query: &str) -> Vec<usize> {
        let searchable: Vec<Searchable> = self
            .shell
            .apps
            .list
            .iter()
            .map(|app| Searchable {
                id: app.id.clone(),
                name: app.name.clone(),
                generic_name: app.generic_name.clone(),
                comment: app.comment.clone(),
                keywords: app.keywords.clone(),
                launchable: !app.command.is_empty(),
            })
            .collect();
        launcher::results(&searchable, query)
    }

    fn launcher_move(&mut self, delta: isize) {
        let Some(launcher) = &mut self.shell.launcher else { return };
        let count = launcher.results.len();
        if count == 0 {
            return;
        }
        launcher.selected = (launcher.selected as isize + delta).rem_euclid(count as isize) as usize;
        if let Some(geo) = self.space.output_geometry(&launcher.output) {
            let height = launcher::size((geo.size.w as f32, geo.size.h as f32)).1;
            launcher.scroll = launcher::scroll_to(launcher.scroll, launcher.selected, count, height);
        }
        self.shell_changed();
    }

    /// Start the application at `index` of the results and close.
    fn launcher_launch(&mut self, index: usize) {
        let Some(app) = self.shell.launcher.as_ref().and_then(|l| l.results.get(index)).and_then(|i| self.shell.apps.list.get(*i)) else {
            return;
        };
        let command = if app.terminal {
            let mut command = self.config.terminal.clone();
            command.push("-e".into());
            command.extend(app.command.iter().cloned());
            command
        } else {
            app.command.clone()
        };
        let directory = Some(app.working_directory.as_str()).filter(|d| !d.is_empty()).map(std::path::PathBuf::from);
        self.spawn_program(&command, true, directory.as_deref());
        self.launcher_close();
    }

    /// A key while the launcher holds the keyboard; `modified`: Ctrl, Alt or Super is held.
    pub fn launcher_key(&mut self, sym: Keysym, modified: bool) {
        let Some(launcher) = &mut self.shell.launcher else { return };
        let query = &mut launcher.query;
        let before = query.text.clone();
        match sym.raw() {
            keysyms::KEY_Escape => return self.launcher_close(),
            keysyms::KEY_Down | keysyms::KEY_KP_Down | keysyms::KEY_Tab => return self.launcher_move(1),
            keysyms::KEY_Up | keysyms::KEY_KP_Up | keysyms::KEY_ISO_Left_Tab => return self.launcher_move(-1),
            keysyms::KEY_Return | keysyms::KEY_KP_Enter => {
                let selected = launcher.selected;
                return self.launcher_launch(selected);
            }
            keysyms::KEY_BackSpace => query.backspace(),
            keysyms::KEY_Delete | keysyms::KEY_KP_Delete => query.delete(),
            keysyms::KEY_Left | keysyms::KEY_KP_Left => query.left(),
            keysyms::KEY_Right | keysyms::KEY_KP_Right => query.right(),
            keysyms::KEY_Home | keysyms::KEY_KP_Home => query.home(),
            keysyms::KEY_End | keysyms::KEY_KP_End => query.end(),
            _ => match sym.key_char().filter(|c| !c.is_control() && !modified) {
                Some(c) => query.insert(c.encode_utf8(&mut [0; 4])),
                None => return,
            },
        }
        if query.text != before {
            // A new search starts at its first result.
            launcher.selected = 0;
            launcher.scroll = 0.0;
        }
        self.shell_changed();
    }

    pub fn launcher_clicked(&mut self, target: LauncherTarget) {
        match target {
            LauncherTarget::App(index) => self.launcher_launch(index),
            LauncherTarget::Search => {
                let Some(launcher) = self.shell.launcher.as_ref() else { return };
                let (Some(geo), Some((x, _))) = (self.space.output_geometry(&launcher.output), self.launcher_origin()) else { return };
                let field_x = (self.pointer_location.x - f64::from(geo.loc.x) - x) as f32 - splice_ui::theme::PANEL_PADDING;
                let output = (geo.size.w as f32, geo.size.h as f32);
                let query = launcher.query.clone();
                let Some(fonts) = self.shell.fonts.as_mut() else { return };
                let cursor = launcher::search_cursor_at(fonts, &query, output, field_x);
                if let Some(launcher) = &mut self.shell.launcher {
                    launcher.query.cursor = cursor;
                }
                self.shell_changed();
            }
            LauncherTarget::Background | LauncherTarget::List => {}
        }
    }

    pub fn launcher_scroll(&mut self, steps: f32) {
        let Some(launcher) = &mut self.shell.launcher else { return };
        let Some(geo) = self.space.output_geometry(&launcher.output) else { return };
        let height = launcher::size((geo.size.w as f32, geo.size.h as f32)).1;
        let max = launcher::max_scroll(launcher.results.len(), height);
        let moved = (launcher.scroll - steps * SCROLL_STEP).clamp(0.0, max);
        if moved != launcher.scroll {
            launcher.scroll = moved;
            self.shell_changed();
        }
    }

    /// Paint the launcher if what it shows changed; its output goes to `redraw`.
    pub fn launcher_refresh(&mut self, fonts: &mut Fonts, redraw: &mut Vec<Output>) {
        let Some(launcher) = self.shell.launcher.as_ref() else { return };
        let Some(geo) = self.space.output_geometry(&launcher.output) else {
            // Its monitor is gone: nothing may hold the keyboard unseen.
            return self.launcher_close();
        };
        let results = self.launcher_results(&launcher.query.text);
        let apps: Vec<AppView> = results
            .iter()
            .filter_map(|i| self.shell.apps.list.get(*i))
            .map(|app| {
                let detail = [&app.generic_name, &app.comment, &app.id].into_iter().find(|t| !t.is_empty()).cloned().unwrap_or_default();
                AppView { name: app.name.clone(), detail, icon: self.shell.apps.icons.get(&app.icon).cloned() }
            })
            .collect();
        let hover = match self.shell.hover {
            Some(ShellHit::Launcher(target)) => Some(target),
            _ => None,
        };
        let (language, theme) = (self.shell.language, self.shell.theme.clone());
        let Some(launcher) = &mut self.shell.launcher else { return };
        if results != launcher.results {
            launcher.selected = launcher.selected.min(results.len().saturating_sub(1));
            launcher.results = results;
        }
        let output = (geo.size.w as f32, geo.size.h as f32);
        let scroll = launcher.scroll.min(launcher::max_scroll(apps.len(), launcher::size(output).1));
        launcher.scroll = scroll;
        let model = LauncherModel { output, query: launcher.query.clone(), apps, selected: launcher.selected, scroll, hover, language };
        if launcher.model.as_ref() == Some(&model) {
            return;
        }
        launcher.painted = paint(launcher::scene(&model, &theme, fonts), fonts, launcher.output.current_scale().fractional_scale());
        launcher.model = Some(model);
        redraw.push(launcher.output.clone());
    }

    /// Forget the launcher's picture (the theme changed).
    pub fn launcher_repaint(&mut self) {
        if let Some(launcher) = &mut self.shell.launcher {
            launcher.model = None;
        }
    }

    fn launcher_origin(&self) -> Option<(f64, f64)> {
        let launcher = self.shell.launcher.as_ref()?;
        let geo = self.space.output_geometry(&launcher.output)?;
        let (x, y) = launcher::origin((geo.size.w as f32, geo.size.h as f32));
        Some((f64::from(x), f64::from(y)))
    }

    pub fn launcher_elements(&self, renderer: &mut GlesRenderer, output: &Output) -> Option<OutputElement> {
        let launcher = self.shell.launcher.as_ref().filter(|l| &l.output == output)?;
        element(renderer, launcher.painted.as_ref()?, self.launcher_origin()?, output.current_scale().fractional_scale())
    }

    pub fn launcher_at_pointer(&self) -> Option<ShellHit> {
        let launcher = self.shell.launcher.as_ref()?;
        let geo = self.space.output_geometry(&launcher.output)?;
        let (x, y) = self.launcher_origin()?;
        let local = self.pointer_location - geo.loc.to_f64();
        launcher.painted.as_ref()?.scene.target_at((local.x - x) as f32, (local.y - y) as f32).map(ShellHit::Launcher)
    }
}
