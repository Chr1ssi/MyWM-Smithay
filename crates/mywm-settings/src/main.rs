//! `mywm-settings`: edit the MyWM configuration (settings and keybindings) in a window.
//!
//! Changes go straight into the config file (comments are kept; a dotfiles symlink is followed),
//! are checked with the compositor's own parser, and the running compositor is told to reload.
use std::{collections::HashMap, path::PathBuf, sync::Arc};

use eframe::egui::{self, Color32, RichText};
use mywm_config::{Bindings, Config, parse_key};
use mywm_settings::{
    files,
    keys::{self, Mods},
    model::{self, Document, Kind, Page, Setting},
};
use toml_edit::{Table, Value};

/// Who uses which key: (keysym, (super, ctrl, alt, shift)) -> actions.
type Owners = HashMap<(u32, (bool, bool, bool, bool)), Vec<String>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Nav {
    Page(Page),
    Rules,
    Programs,
    File,
}

impl Nav {
    fn all() -> Vec<(Nav, &'static str)> {
        let mut all: Vec<(Nav, &'static str)> = Page::ALL.iter().map(|p| (Nav::Page(*p), p.title())).collect();
        all.insert(1, (Nav::Programs, "Programme"));
        all.insert(3, (Nav::Rules, "Fensterregeln"));
        all.push((Nav::File, "Datei"));
        all
    }
}

/// Where a captured key combination goes.
#[derive(Clone)]
struct Capture {
    /// Dotted path of the key list, e.g. `bindings.close`.
    path: String,
    title: String,
    mods: Mods,
}

struct App {
    path: PathBuf,
    path_edit: String,
    doc: Document,
    baseline: Document,
    problem: Option<String>,
    status: String,
    nav: Nav,
    capture: Option<Capture>,
    lists: HashMap<String, String>,
    new_program: String,
    frames: u32,
    screenshot: Option<PathBuf>,
}

fn action_label(name: &str) -> &'static str {
    match name {
        "reload" => "Konfiguration neu laden",
        "wallpaper" => "Wallpaper-Auswahl",
        "screenshot" => "Screenshot (Bereich oder Fenster)",
        "screenshot_screen" => "Screenshot (Monitor)",
        "screenshot_window" => "Screenshot (fokussiertes Fenster)",
        "overview" => "Workspace-Übersicht",
        "lock" => "Sperren",
        "terminal" => "Terminal",
        "launcher" => "Launcher",
        "close" => "Fenster schließen",
        "exit" => "Beenden",
        "focus_left" => "Fokus links",
        "focus_right" => "Fokus rechts",
        "move_left" => "Fenster nach links",
        "move_right" => "Fenster nach rechts",
        "workspace_previous" => "Workspace zurück",
        "workspace_next" => "Workspace vor",
        "move_to_workspace_previous" => "Fenster zum vorigen Workspace",
        "move_to_workspace_next" => "Fenster zum nächsten Workspace",
        "new_workspace" => "Neuer Workspace",
        "move_to_new_workspace" => "Fenster in neuen Workspace",
        "toggle_floating" => "Schweben umschalten",
        "toggle_fullscreen" => "Vollbild umschalten",
        "column_shrink" => "Spalte schmaler",
        "column_grow" => "Spalte breiter",
        "toggle_scratchpad" => "Scratchpad zeigen/verstecken",
        "move_to_scratchpad" => "Fenster ins Scratchpad",
        "release_shortcuts" => "Tastenkürzel von Spiel/VM zurückholen",
        "focus_output_left" => "Fokus Monitor links",
        "focus_output_right" => "Fokus Monitor rechts",
        "focus_output_up" => "Fokus Monitor oben",
        "focus_output_down" => "Fokus Monitor unten",
        "move_to_output_left" => "Fenster auf Monitor links",
        "move_to_output_right" => "Fenster auf Monitor rechts",
        "move_to_output_up" => "Fenster auf Monitor oben",
        "move_to_output_down" => "Fenster auf Monitor unten",
        _ => "",
    }
}

impl App {
    fn new(path: PathBuf, page: Option<usize>, screenshot: Option<PathBuf>) -> Self {
        let (text, status) = match files::read(&path) {
            Ok(text) => (text, String::new()),
            Err(error) => (String::new(), error),
        };
        let doc = Document::parse(&text).unwrap_or_else(|error| {
            eprintln!("{}: {error}", path.display());
            Document::parse("").unwrap()
        });
        let mut app = Self {
            path_edit: path.display().to_string(),
            path,
            baseline: doc.clone(),
            doc,
            problem: None,
            status,
            nav: Nav::all().get(page.unwrap_or(0)).map_or(Nav::Page(Page::Keys), |n| n.0),
            capture: None,
            lists: HashMap::new(),
            new_program: String::new(),
            frames: 0,
            screenshot,
        };
        app.revalidate();
        app
    }

    fn revalidate(&mut self) {
        self.problem = self.doc.validate().err();
    }

    fn read_only(&self) -> bool {
        files::is_in_store(&self.path)
    }

    fn reload_from_disk(&mut self) {
        match files::read(&self.path).and_then(|text| Document::parse(&text)) {
            Ok(doc) => {
                self.baseline = doc.clone();
                self.doc = doc;
                self.lists.clear();
                self.status = "Von der Festplatte geladen.".into();
            }
            Err(error) => self.status = error,
        }
        self.revalidate();
    }

    fn save(&mut self) {
        if let Err(error) = self.doc.validate() {
            self.status = format!("Nicht gespeichert: {error}");
            return;
        }
        let changed_restart: Vec<&str> = model::SETTINGS
            .iter()
            .filter(|s| s.restart)
            .filter(|s| self.doc.get(s.path).map(|v| v.to_string()) != self.baseline.get(s.path).map(|v| v.to_string()))
            .map(|s| s.label)
            .collect();
        match files::write(&self.path, &self.doc.text()) {
            Ok(()) => {
                self.doc.mark_saved();
                self.baseline = self.doc.clone();
                let reloaded = files::reload_compositor();
                self.status = if reloaded {
                    "Gespeichert; der Compositor hat neu geladen.".to_owned()
                } else {
                    "Gespeichert. Der Compositor ist nicht erreichbar (läuft er? MYWM_SOCKET).".to_owned()
                };
                if !changed_restart.is_empty() {
                    self.status.push_str(&format!(" Neustart nötig für: {}.", changed_restart.join(", ")));
                }
            }
            Err(error) => self.status = format!("Nicht gespeichert: {error}"),
        }
    }

    // --- Setting rows -------------------------------------------------------------------------

    fn current(&self, setting: &Setting) -> Option<Value> {
        self.doc.get(setting.path).cloned().or_else(|| model::default_value(setting))
    }

    fn setting_row(&mut self, ui: &mut egui::Ui, setting: &'static Setting) {
        let is_set = self.doc.is_set(setting.path);
        ui.horizontal(|ui| {
            let mut label = RichText::new(setting.label);
            if is_set {
                label = label.strong();
            }
            ui.label(label);
            if setting.restart {
                ui.label(RichText::new("(Neustart)").small().weak());
            }
            if is_set && ui.small_button("Standard").on_hover_text("Wert aus der Datei entfernen").clicked() {
                self.doc.unset(setting.path);
                self.lists.remove(setting.path);
                self.revalidate();
            }
        });
        let mut changed: Option<Option<Value>> = None;
        let current = self.current(setting);
        match setting.kind {
            Kind::Bool => {
                let mut on = current.as_ref().and_then(Value::as_bool).unwrap_or(false);
                if ui.checkbox(&mut on, "an").changed() {
                    changed = Some(Some(on.into()));
                }
            }
            Kind::Int { min, max } => {
                let mut value = current.as_ref().and_then(Value::as_integer).unwrap_or(min);
                if ui.add(egui::DragValue::new(&mut value).range(min..=max)).changed() {
                    changed = Some(Some(value.into()));
                }
            }
            Kind::Float { min, max } => {
                let mut value = current.as_ref().and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).unwrap_or(min.max(0.0));
                if ui.add(egui::Slider::new(&mut value, min..=max)).changed() {
                    changed = Some(Some(((value * 100.0).round() / 100.0).into()));
                }
            }
            Kind::Choice { options, optional } => {
                let shown = current.as_ref().and_then(Value::as_str).unwrap_or("(Standard)").to_owned();
                egui::ComboBox::from_id_salt(setting.path).selected_text(shown).show_ui(ui, |ui| {
                    if optional && ui.selectable_label(!is_set, "(Standard)").clicked() {
                        changed = Some(None);
                    }
                    for option in options {
                        if ui.selectable_label(current.as_ref().and_then(Value::as_str) == Some(option), *option).clicked() {
                            changed = Some(Some((*option).into()));
                        }
                    }
                });
            }
            Kind::Text => {
                let mut text = current.as_ref().and_then(Value::as_str).unwrap_or("").to_owned();
                if ui.add(egui::TextEdit::singleline(&mut text).desired_width(360.0)).changed() {
                    changed = Some(if text.is_empty() && setting.default.is_empty() { None } else { Some(text.into()) });
                }
            }
            Kind::Color => {
                let hex = current.as_ref().and_then(Value::as_str).unwrap_or("#000000").to_owned();
                let mut rgb = parse_hex(&hex).unwrap_or([0, 0, 0]);
                ui.horizontal(|ui| {
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        changed = Some(Some(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]).into()));
                    }
                    ui.label(RichText::new(hex).monospace());
                });
            }
            Kind::List => {
                let initial = self.doc.get_list(setting.path).or_else(|| model::default_value(setting).map(|v| list_of(&v))).unwrap_or_default();
                let buffer = self.lists.entry(setting.path.to_owned()).or_insert_with(|| initial.join("\n"));
                let lines = buffer.lines().count().clamp(2, 6);
                if ui.add(egui::TextEdit::multiline(buffer).desired_rows(lines).desired_width(360.0)).changed() {
                    let values: Vec<String> = buffer.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
                    self.doc.set_list(setting.path, &values);
                    self.revalidate();
                }
            }
        }
        if let Some(change) = changed {
            match change {
                Some(value) => self.doc.set(setting.path, value),
                None => self.doc.unset(setting.path),
            }
            self.revalidate();
        }
        if !setting.help.is_empty() {
            ui.label(RichText::new(setting.help).small().weak());
        }
        ui.add_space(8.0);
    }

    // --- Keybindings --------------------------------------------------------------------------

    /// All effective bindings as (normalized key, who), for spotting duplicates.
    fn binding_owners(&self) -> Owners {
        let mut owners: Owners = HashMap::new();
        let mut add = |key: &str, who: String| {
            if let Ok((symbol, m)) = parse_key(key) {
                owners.entry((symbol, (m.logo, m.ctrl, m.alt, m.shift))).or_default().push(who);
            }
        };
        let defaults = Bindings::default();
        for (name, default_keys) in defaults.entries() {
            let keys = self.doc.get_list(&format!("bindings.{name}")).unwrap_or_else(|| default_keys.clone());
            for key in keys {
                add(&key, action_label(name).to_owned());
            }
        }
        for name in self.doc.program_binding_names() {
            for key in self.doc.get_list(&format!("program_bindings.{name}.keys")).unwrap_or_default() {
                add(&key, format!("Programm {name}"));
            }
        }
        owners
    }

    fn key_chips(&mut self, ui: &mut egui::Ui, path: &str, title: &str, default_keys: Option<&Vec<String>>, owners: &Owners) {
        let is_set = self.doc.is_set(path);
        let keys = self.doc.get_list(path).or_else(|| default_keys.cloned()).unwrap_or_default();
        let mut new_keys = keys.clone();
        let mut remove = None;
        ui.horizontal_wrapped(|ui| {
            for (index, key) in keys.iter().enumerate() {
                let clash = parse_key(key).ok().and_then(|(s, m)| owners.get(&(s, (m.logo, m.ctrl, m.alt, m.shift)))).filter(|who| who.len() > 1);
                let text = RichText::new(key).monospace();
                ui.group(|ui| {
                    ui.label(if clash.is_some() { text.color(Color32::from_rgb(243, 139, 168)) } else { text });
                    if ui.small_button("x").on_hover_text("Taste entfernen").clicked() {
                        remove = Some(index);
                    }
                });
                if let Some(who) = clash {
                    ui.label(RichText::new(format!("doppelt: {}", who.join(", "))).small().color(Color32::from_rgb(243, 139, 168)));
                }
            }
            if ui.button("+ Taste").clicked() {
                self.capture = Some(Capture { path: path.to_owned(), title: title.to_owned(), mods: Mods { logo: true, ..Default::default() } });
            }
            if is_set && ui.small_button("Standard").on_hover_text("Belegung aus der Datei entfernen").clicked() {
                self.doc.unset(path);
                self.revalidate();
            }
        });
        if let Some(index) = remove {
            new_keys.remove(index);
            self.set_keys(path, new_keys, default_keys);
        }
    }

    fn set_keys(&mut self, path: &str, keys: Vec<String>, default_keys: Option<&Vec<String>>) {
        if default_keys.is_some_and(|d| *d == keys) {
            self.doc.unset(path);
        } else {
            self.doc.set_list(path, &keys);
        }
        self.revalidate();
    }

    fn keys_page(&mut self, ui: &mut egui::Ui) {
        let owners = self.binding_owners();
        let defaults = Bindings::default();
        ui.label(RichText::new("Belegungen mit doppelten Tasten sind rot markiert; die Datei wird dann nicht gespeichert.").small().weak());
        ui.add_space(6.0);
        for (name, default_keys) in defaults.entries() {
            ui.label(RichText::new(action_label(name)).strong());
            let path = format!("bindings.{name}");
            self.key_chips(ui, &path, action_label(name), Some(default_keys), &owners);
            ui.add_space(6.0);
        }
        ui.separator();
        ui.label(RichText::new("Modifikatoren der Zifferntasten und der Maus").strong());
        for (name, default) in defaults.modifier_entries() {
            let path = format!("bindings.{name}");
            let mut text = self.doc.get_str(&path).unwrap_or(default).to_owned();
            ui.horizontal(|ui| {
                ui.label(match name {
                    "pointer_modifiers" => "Fenster mit der Maus verschieben/skalieren",
                    "workspace_modifiers" => "Workspace 1-9 wählen",
                    _ => "Fenster zu Workspace 1-9",
                });
                if ui.add(egui::TextEdit::singleline(&mut text).desired_width(140.0)).changed() {
                    if text == *default {
                        self.doc.unset(&path);
                    } else {
                        self.doc.set(&path, text.into());
                    }
                    self.revalidate();
                }
            });
        }
    }

    fn capture_window(&mut self, ctx: &egui::Context) {
        let Some(capture) = self.capture.clone() else { return };
        let mut capture = capture;
        let mut close = false;
        egui::Window::new("Taste aufnehmen").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(format!("Für: {}", capture.title));
            ui.label("Modifikatoren (Super lässt sich nicht erkennen und wird hier gewählt):");
            ui.horizontal(|ui| {
                ui.checkbox(&mut capture.mods.logo, "Super");
                ui.checkbox(&mut capture.mods.ctrl, "Ctrl");
                ui.checkbox(&mut capture.mods.alt, "Alt");
                ui.checkbox(&mut capture.mods.shift, "Shift");
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Jetzt die Taste drücken …").strong());
            ui.label(RichText::new("Esc ohne Modifikator bricht ab.").small().weak());
            if ui.button("Abbrechen").clicked() {
                close = true;
            }
        });
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            if let egui::Event::Key { key, pressed: true, modifiers, repeat: false, .. } = event {
                let mods = Mods {
                    logo: capture.mods.logo,
                    ctrl: capture.mods.ctrl || modifiers.ctrl,
                    alt: capture.mods.alt || modifiers.alt,
                    shift: capture.mods.shift || modifiers.shift,
                };
                if key == egui::Key::Escape && mods == Mods::default() {
                    close = true;
                    break;
                }
                if let Some(binding) = keys::binding(mods, key) {
                    let default_keys = capture.path.strip_prefix("bindings.").and_then(|name| Bindings::default().entries().into_iter().find(|(n, _)| *n == name).map(|(_, k)| k.clone()));
                    let mut list = self.doc.get_list(&capture.path).or_else(|| default_keys.clone()).unwrap_or_default();
                    if !list.contains(&binding) {
                        list.push(binding);
                    }
                    self.set_keys(&capture.path, list, default_keys.as_ref());
                    close = true;
                    break;
                }
                self.status = "Diese Taste kann der Compositor nicht belegen.".into();
            }
        }
        self.capture = if close { None } else { Some(capture) };
    }

    // --- Rules and programs -------------------------------------------------------------------

    fn rules_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Regeln gelten für Fenster, auf die alle angegebenen Merkmale passen. Die app_id steht beim Öffnen eines Fensters im Log (new window: app_id=…).").small().weak());
        ui.add_space(6.0);
        let count = self.doc.table_array("rules").len();
        let mut remove = None;
        let mut dirty = false;
        for index in 0..count {
            ui.group(|ui| {
                let table = &mut self.doc.table_array_mut("rules").get_mut(index).expect("a rule");
                ui.horizontal(|ui| {
                    ui.label("app_id");
                    let mut text = table.get("app_id").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                    if ui.add(egui::TextEdit::singleline(&mut text).desired_width(260.0)).changed() {
                        if text.is_empty() {
                            table.remove("app_id");
                        } else {
                            table.insert("app_id", toml_edit::value(text));
                        }
                        dirty = true;
                    }
                    if ui.button("Regel löschen").clicked() {
                        remove = Some(index);
                    }
                });
                dirty |= tri_state(ui, table, "dialog", "Dialog");
                dirty |= tri_state(ui, table, "floating", "Schwebend");
                dirty |= optional_int(ui, table, "workspace", "Workspace", 1, 9);
                dirty |= optional_float(ui, table, "opacity", "Deckkraft", 0.1, 1.0);
            });
        }
        if let Some(index) = remove {
            self.doc.remove_table_array_entry("rules", index);
            dirty = true;
        }
        if ui.button("+ Regel hinzufügen").clicked() {
            let mut table = Table::new();
            table.insert("app_id", toml_edit::value("app.id"));
            table.insert("floating", toml_edit::value(true));
            self.doc.table_array_mut("rules").push(table);
            dirty = true;
        }
        if dirty {
            self.revalidate();
        }
    }

    fn programs_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Tasten, die ein Programm starten. Das Kommando: ein Argument pro Zeile.").small().weak());
        ui.add_space(6.0);
        let owners = self.binding_owners();
        let mut remove = None;
        for name in self.doc.program_binding_names() {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&name).strong());
                    if ui.button("Löschen").clicked() {
                        remove = Some(name.clone());
                    }
                });
                ui.label("Tasten");
                self.key_chips(ui, &format!("program_bindings.{name}.keys"), &name, None, &owners);
                ui.label("Kommando");
                let path = format!("program_bindings.{name}.command");
                let initial = self.doc.get_list(&path).unwrap_or_default();
                let buffer = self.lists.entry(path.clone()).or_insert_with(|| initial.join("\n"));
                let lines = buffer.lines().count().clamp(1, 5);
                if ui.add(egui::TextEdit::multiline(buffer).desired_rows(lines).desired_width(360.0)).changed() {
                    let values: Vec<String> = buffer.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
                    self.doc.set_list(&path, &values);
                    self.revalidate();
                }
            });
        }
        if let Some(name) = remove {
            self.doc.unset(&format!("program_bindings.{name}.keys"));
            self.doc.unset(&format!("program_bindings.{name}.command"));
            self.lists.retain(|k, _| !k.starts_with(&format!("program_bindings.{name}.")));
            self.revalidate();
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_program).hint_text("Name, z. B. browser").desired_width(200.0));
            let valid = !self.new_program.is_empty() && self.new_program.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if ui.add_enabled(valid, egui::Button::new("+ Programm hinzufügen")).clicked() {
                let name = std::mem::take(&mut self.new_program);
                self.doc.set_list(&format!("program_bindings.{name}.keys"), &["Super+b".to_owned()]);
                self.doc.set_list(&format!("program_bindings.{name}.command"), &["firefox".to_owned()]);
                self.revalidate();
            }
        });
    }

    fn file_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Konfigurationsdatei").strong());
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.path_edit).desired_width(460.0));
            if ui.button("Übernehmen").clicked() {
                let path = PathBuf::from(self.path_edit.trim());
                files::remember_path(&path);
                self.path = path;
                self.reload_from_disk();
            }
        });
        let real = files::real_path(&self.path);
        if real != self.path {
            ui.label(format!("Verweist auf: {}", real.display()));
        }
        if self.read_only() {
            ui.colored_label(Color32::from_rgb(249, 226, 175), "Die Datei liegt im Nix-Store und ist schreibgeschützt. Den Pfad in deinen Dotfiles-Ordner legen (z. B. …/dotfiles/mywm/config.toml) und hier übernehmen.");
        } else if !self.path.exists() {
            ui.label("Die Datei gibt es noch nicht; sie wird beim Speichern angelegt.");
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Von Festplatte neu laden").clicked() {
                self.reload_from_disk();
            }
            if ui.button("Compositor neu laden lassen").clicked() {
                self.status = if files::reload_compositor() { "Neu laden angefordert.".into() } else { "Der Compositor ist nicht erreichbar.".into() };
            }
        });
        ui.add_space(12.0);
        ui.label(RichText::new("Hinweis").strong());
        ui.label("Monitore (Modus, Position, Skalierung) stellt man mit kanshi oder wlr-randr ein; [[outputs]] in der Datei bleibt unberührt.");
    }
}

fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#')?;
    (hex.len() == 6).then_some(())?;
    Some([u8::from_str_radix(&hex[0..2], 16).ok()?, u8::from_str_radix(&hex[2..4], 16).ok()?, u8::from_str_radix(&hex[4..6], 16).ok()?])
}

fn list_of(value: &Value) -> Vec<String> {
    value.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

/// any / yes / no for an optional boolean key of a rule. Returns whether it changed.
fn tri_state(ui: &mut egui::Ui, table: &mut Table, key: &str, label: &str) -> bool {
    let current = table.get(key).and_then(|v| v.as_bool());
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt((key, table.get("app_id").map(|v| v.to_string()))).selected_text(match current {
            None => "egal",
            Some(true) => "ja",
            Some(false) => "nein",
        }).show_ui(ui, |ui| {
            for (value, text) in [(None, "egal"), (Some(true), "ja"), (Some(false), "nein")] {
                if ui.selectable_label(current == value, text).clicked() {
                    match value {
                        Some(v) => {
                            table.insert(key, toml_edit::value(v));
                        }
                        None => {
                            table.remove(key);
                        }
                    }
                    changed = true;
                }
            }
        });
    });
    changed
}

fn optional_int(ui: &mut egui::Ui, table: &mut Table, key: &str, label: &str, min: i64, max: i64) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        let mut on = table.contains_key(key);
        let mut value = table.get(key).and_then(|v| v.as_integer()).unwrap_or(min);
        if ui.checkbox(&mut on, "").changed() {
            if on {
                table.insert(key, toml_edit::value(value));
            } else {
                table.remove(key);
            }
            changed = true;
        }
        if on && ui.add(egui::DragValue::new(&mut value).range(min..=max)).changed() {
            table.insert(key, toml_edit::value(value));
            changed = true;
        }
    });
    changed
}

fn optional_float(ui: &mut egui::Ui, table: &mut Table, key: &str, label: &str, min: f64, max: f64) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        let mut on = table.contains_key(key);
        let mut value = table.get(key).and_then(|v| v.as_float()).unwrap_or(max);
        if ui.checkbox(&mut on, "").changed() {
            if on {
                table.insert(key, toml_edit::value(value));
            } else {
                table.remove(key);
            }
            changed = true;
        }
        if on && ui.add(egui::Slider::new(&mut value, min..=max)).changed() {
            table.insert(key, toml_edit::value((value * 100.0).round() / 100.0));
            changed = true;
        }
    });
    changed
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frames += 1;
        // Screenshot mode (for the smoke test and documentation): a few frames, then one image.
        if let Some(target) = self.screenshot.clone() {
            if self.frames == 4 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            let image = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(Arc::clone(image)),
                    _ => None,
                })
            });
            if let Some(image) = image {
                save_png(&target, &image);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }

        let dirty = self.doc.is_dirty();
        let read_only = self.read_only();
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) && dirty && self.problem.is_none() && !read_only {
            self.save();
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("mywm Einstellungen");
                ui.separator();
                let can_save = dirty && self.problem.is_none() && !read_only;
                if ui.add_enabled(can_save, egui::Button::new("Speichern")).on_hover_text("Strg+S").clicked() {
                    self.save();
                }
                if ui.add_enabled(dirty, egui::Button::new("Verwerfen")).clicked() {
                    self.reload_from_disk();
                }
                if dirty {
                    ui.label(RichText::new("ungespeicherte Änderungen").color(Color32::from_rgb(249, 226, 175)));
                }
            });
            if let Some(problem) = &self.problem {
                ui.colored_label(Color32::from_rgb(243, 139, 168), format!("Die Datei wäre ungültig: {problem}"));
            } else if !self.status.is_empty() {
                ui.label(RichText::new(&self.status).weak());
            }
            ui.add_space(2.0);
        });

        egui::SidePanel::left("nav").resizable(false).default_width(170.0).show(ctx, |ui| {
            ui.add_space(6.0);
            for (nav, title) in Nav::all() {
                if ui.selectable_label(self.nav == nav, title).clicked() {
                    self.nav = nav;
                }
            }
            ui.add_space(12.0);
            ui.label(RichText::new(self.path.display().to_string()).small().weak());
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.nav {
                Nav::Page(Page::Keys) => self.keys_page(ui),
                Nav::Page(page) => {
                    for setting in model::settings_of(page) {
                        self.setting_row(ui, setting);
                    }
                }
                Nav::Rules => self.rules_page(ui),
                Nav::Programs => self.programs_page(ui),
                Nav::File => self.file_page(ui),
            });
        });

        self.capture_window(ctx);
    }
}

fn save_png(path: &PathBuf, image: &egui::ColorImage) {
    let [width, height] = image.size;
    let Ok(file) = std::fs::File::create(path) else { return };
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    if let Ok(mut writer) = encoder.write_header() {
        let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
        let _ = writer.write_image_data(&bytes);
    }
}

fn main() -> eframe::Result {
    let mut args = std::env::args().skip(1);
    let (mut config, mut page, mut screenshot, mut size) = (None, None, None, (980.0, 720.0));
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config = args.next().map(PathBuf::from),
            "--page" => page = args.next().and_then(|p| p.parse().ok()),
            "--screenshot" => screenshot = args.next().map(PathBuf::from),
            "--size" => {
                if let Some((w, h)) = args.next().and_then(|s| s.split_once('x').map(|(w, h)| (w.to_owned(), h.to_owned()))) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--help" | "-h" => {
                println!("mywm-settings [--config DATEI]");
                return Ok(());
            }
            other => eprintln!("unknown argument {other}"),
        }
    }
    let path = files::resolve(config);
    // Check the file loads before opening a window, so a broken config is reported plainly.
    if let Ok(text) = files::read(&path)
        && let Err(error) = Config::parse(&text)
    {
        eprintln!("Hinweis: {} ist derzeit ungültig: {error}", path.display());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([size.0, size.1]).with_title("mywm Einstellungen").with_app_id("mywm-settings"),
        ..Default::default()
    };
    eframe::run_native("mywm-settings", options, Box::new(move |_cc| Ok(Box::new(App::new(path, page, screenshot)))))
}
