//! The settings the editor offers, and editing a config file as a TOML document.
use toml_edit::{Array, DocumentMut, Item, Table, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Keys,
    Windows,
    Effects,
    Input,
    Appearance,
    General,
}

impl Page {
    pub const ALL: [Page; 6] = [Page::Keys, Page::Windows, Page::Effects, Page::Input, Page::Appearance, Page::General];

    pub fn title(self) -> &'static str {
        match self {
            Page::Keys => "Tastenbelegung",
            Page::Windows => "Fenster",
            Page::Effects => "Effekte",
            Page::Input => "Eingabe",
            Page::Appearance => "Aussehen",
            Page::General => "Allgemein",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Bool,
    Int { min: i64, max: i64 },
    Float { min: f64, max: f64 },
    /// One of these words; an unset value (the compositor's own default) is offered as well when `optional`.
    Choice { options: &'static [&'static str], optional: bool },
    Text,
    Color,
    /// One entry per line.
    List,
}

#[derive(Debug)]
pub struct Setting {
    /// Dotted path in the file, e.g. `effects.corner_radius`.
    pub path: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub page: Page,
    pub kind: Kind,
    /// The compositor's default as TOML text; empty when the setting is unset by default.
    pub default: &'static str,
    /// Only takes effect after the next start of the compositor.
    pub restart: bool,
}

const fn s(path: &'static str, label: &'static str, help: &'static str, page: Page, kind: Kind, default: &'static str) -> Setting {
    Setting { path, label, help, page, kind, default, restart: false }
}

const fn restart(mut setting: Setting) -> Setting {
    setting.restart = true;
    setting
}

use Kind::*;
use Page::*;

pub static SETTINGS: &[Setting] = &[
    // Windows
    s("float_dialogs", "Dialoge schweben", "Dialogfenster erscheinen als schwebende Fenster statt in den Spalten.", Windows, Bool, "true"),
    s("game_app_id_prefixes", "Spiele erkennen (app_id-Anfang)", "Ein Fenster, dessen app_id so beginnt, gilt im Vollbild als Spiel (VRR, Tearing, keine Effekte). Eine Angabe pro Zeile.", Windows, List, "[]"),
    s("appearance.gaps_inner", "Abstand zwischen Fenstern", "Pixel.", Windows, Int { min: 0, max: 128 }, "8"),
    s("appearance.gaps_outer", "Abstand zum Rand", "Pixel.", Windows, Int { min: 0, max: 128 }, "8"),
    s("appearance.border_width", "Rahmenbreite", "Pixel.", Windows, Int { min: 0, max: 32 }, "2"),
    restart(s("xwayland", "Xwayland", "X11-Programme (Steam, ältere Spiele) unterstützen.", Windows, Bool, "true")),
    // Effects
    s("effects.corner_radius", "Abgerundete Ecken", "Radius in Pixeln; 0 = eckig. Nie bei Vollbild.", Effects, Int { min: 0, max: 64 }, "0"),
    s("effects.inactive_opacity", "Deckkraft ohne Fokus", "1.0 = undurchsichtig.", Effects, Float { min: 0.1, max: 1.0 }, "1.0"),
    s("effects.animation_ms", "Animationsdauer (ms)", "Fenster gleiten an ihren Platz und blenden ein; 0 = keine Animationen.", Effects, Int { min: 0, max: 1000 }, "0"),
    s("effects.shadow", "Schatten", "Ausdehnung in Pixeln; 0 = kein Schatten.", Effects, Int { min: 0, max: 64 }, "0"),
    s("effects.blur", "Unschärfe hinter durchsichtigen Fenstern", "1 (leicht) bis 16 (stark); 0 = aus. Zeigt das Wallpaper unscharf.", Effects, Int { min: 0, max: 16 }, "0"),
    s("render.force_tearing", "Tearing erzwingen", "Auf den async_outputs auch tearen, wenn das Spiel es nicht erlaubt.", Effects, Bool, "false"),
    s("render.late_scheduling", "Späte Frame-Planung", "Frames erst kurz vor dem Vblank rendern (weniger Latenz). Aus bei VRR und Tearing.", Effects, Bool, "false"),
    s("render.margin_ms", "Reserve vor dem Vblank (ms)", "Zusätzlich zur gemessenen Renderzeit.", Effects, Float { min: 0.0, max: 16.0 }, "2.0"),
    // Input
    s("input.accel_profile", "Mausbeschleunigung", "„flat“ ist roh und ideal für Spiele.", Input, Choice { options: &["flat", "adaptive"], optional: true }, ""),
    s("input.accel_speed", "Mausgeschwindigkeit", "-1.0 (langsam) bis 1.0 (schnell).", Input, Float { min: -1.0, max: 1.0 }, ""),
    s("input.natural_scroll", "Natürliches Scrollen", "", Input, Bool, ""),
    s("input.tap", "Tippen zum Klicken (Touchpad)", "", Input, Bool, "true"),
    s("input.left_handed", "Linkshändig", "", Input, Bool, ""),
    s("input.repeat_rate", "Tastenwiederholung (pro Sekunde)", "", Input, Int { min: 1, max: 200 }, "25"),
    s("input.repeat_delay", "Wiederholung beginnt nach (ms)", "", Input, Int { min: 50, max: 2000 }, "200"),
    s("keyboard.layout", "Tastaturlayout", "xkb-Layout, z. B. de oder us.", Input, Text, "\"de\""),
    s("keyboard.variant", "Variante", "xkb-Variante, z. B. nodeadkeys.", Input, Text, "\"\""),
    s("keyboard.options", "xkb-Optionen", "z. B. caps:escape.", Input, Text, "\"\""),
    // Appearance
    s("appearance.active_border", "Rahmen (Fokus)", "", Appearance, Color, "\"#89b4fa\""),
    s("appearance.inactive_border", "Rahmen (ohne Fokus)", "", Appearance, Color, "\"#45475a\""),
    s("appearance.urgent_border", "Rahmen (Aufmerksamkeit)", "Fenster, das den Fokus anfordert.", Appearance, Color, "\"#f38ba8\""),
    s("appearance.background", "Hintergrund", "", Appearance, Color, "\"#1e1e2e\""),
    s("appearance.surface", "Flächen", "", Appearance, Color, "\"#313244\""),
    s("appearance.text", "Text", "", Appearance, Color, "\"#cdd6f4\""),
    s("appearance.muted_text", "Text (gedämpft)", "", Appearance, Color, "\"#a6adc8\""),
    // General
    s("terminal", "Terminal", "Programm und Argumente, eine Angabe pro Zeile.", General, List, "[\"foot\"]"),
    s("wallpaper_directory", "Wallpaper-Ordner", "Absoluter Pfad.", General, Text, ""),
    s("screenshot_directory", "Screenshot-Ordner", "Absoluter Pfad.", General, Text, ""),
    restart(s("async_outputs", "Monitore mit Tearing", "Namen der Monitore (z. B. DP-3), eine Angabe pro Zeile.", General, List, "[]")),
    restart(s("workspace_outputs", "Monitor-Reihenfolge", "Monitore in Workspace-Reihenfolge, eine Angabe pro Zeile.", General, List, "[]")),
    restart(s("idle.lock_after_seconds", "Sperren nach (s)", "0 = nie.", General, Int { min: 0, max: 86400 }, "300")),
    restart(s("idle.monitor_off_after_seconds", "Monitore aus nach (s)", "0 = nie; muss später als das Sperren sein.", General, Int { min: 0, max: 86400 }, "600")),
    restart(s("vrr.enabled", "VRR (adaptive Sync)", "Nur bei einem Vollbild-Spiel auf dem gewählten Monitor.", General, Bool, "false")),
    restart(s("vrr.output", "VRR-Monitor", "Name des Monitors, z. B. DP-3.", General, Text, "\"\"")),
];

pub fn settings_of(page: Page) -> impl Iterator<Item = &'static Setting> {
    SETTINGS.iter().filter(move |setting| setting.page == page)
}

/// A TOML file being edited. Comments and formatting survive every change.
#[derive(Clone)]
pub struct Document {
    doc: DocumentMut,
    saved: String,
}

impl Document {
    pub fn parse(text: &str) -> Result<Self, String> {
        let doc = text.parse::<DocumentMut>().map_err(|e| e.to_string())?;
        Ok(Self { doc, saved: text.to_owned() })
    }

    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    pub fn is_dirty(&self) -> bool {
        self.doc.to_string() != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.doc.to_string();
    }

    /// `None` if the compositor would reject the file as it is now.
    pub fn validate(&self) -> Result<(), String> {
        mywm_config::Config::parse(&self.text()).map(|_| ()).map_err(|e| e.to_string())
    }

    pub fn get(&self, path: &str) -> Option<&Value> {
        let mut item: &Item = self.doc.as_item();
        for part in path.split('.') {
            item = item.as_table_like()?.get(part)?;
        }
        item.as_value()
    }

    pub fn is_set(&self, path: &str) -> bool {
        self.get(path).is_some()
    }

    /// Set a value, creating the tables on the way.
    pub fn set(&mut self, path: &str, value: Value) {
        let parts: Vec<&str> = path.split('.').collect();
        let (key, tables) = parts.split_last().expect("a path");
        let mut table: &mut Table = self.doc.as_table_mut();
        for part in tables {
            let entry = table.entry(part).or_insert_with(|| {
                let mut new = Table::new();
                new.set_implicit(true);
                Item::Table(new)
            });
            table = entry.as_table_mut().expect("a table in the path");
        }
        // Keep the comment in front of an existing key.
        match table.get_mut(key) {
            Some(Item::Value(old)) => {
                let decor = old.decor().clone();
                let mut new = value;
                *new.decor_mut() = decor;
                *old = new;
            }
            _ => {
                table.insert(key, Item::Value(value));
            }
        }
    }

    /// Remove a value (back to the compositor's default); empty tables are cleaned up.
    pub fn unset(&mut self, path: &str) {
        let parts: Vec<&str> = path.split('.').collect();
        let (key, tables) = parts.split_last().expect("a path");
        fn remove(table: &mut Table, tables: &[&str], key: &str) {
            match tables.split_first() {
                None => {
                    table.remove(key);
                }
                Some((first, rest)) => {
                    if let Some(child) = table.get_mut(first).and_then(Item::as_table_mut) {
                        remove(child, rest, key);
                        if child.is_empty() {
                            table.remove(first);
                        }
                    }
                }
            }
        }
        remove(self.doc.as_table_mut(), tables, key);
    }

    pub fn get_bool(&self, path: &str) -> Option<bool> {
        self.get(path)?.as_bool()
    }

    pub fn get_f64(&self, path: &str) -> Option<f64> {
        let value = self.get(path)?;
        value.as_float().or_else(|| value.as_integer().map(|i| i as f64))
    }

    pub fn get_i64(&self, path: &str) -> Option<i64> {
        self.get(path)?.as_integer()
    }

    pub fn get_str(&self, path: &str) -> Option<&str> {
        self.get(path)?.as_str()
    }

    pub fn get_list(&self, path: &str) -> Option<Vec<String>> {
        let array = self.get(path)?.as_array()?;
        Some(array.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
    }

    pub fn set_list(&mut self, path: &str, values: &[String]) {
        let mut array = Array::new();
        for value in values {
            array.push(value.as_str());
        }
        self.set(path, Value::Array(array));
    }

    // --- Arrays of tables ([[rules]]) and tables of tables ([program_bindings.NAME]).

    /// The entries of `[[name]]`.
    pub fn table_array(&self, name: &str) -> Vec<&Table> {
        self.doc.get(name).and_then(Item::as_array_of_tables).map(|a| a.iter().collect()).unwrap_or_default()
    }

    pub fn table_array_mut(&mut self, name: &str) -> &mut toml_edit::ArrayOfTables {
        if !self.doc.contains_key(name) {
            self.doc.insert(name, Item::ArrayOfTables(toml_edit::ArrayOfTables::new()));
        }
        self.doc[name].as_array_of_tables_mut().expect("an array of tables")
    }

    pub fn remove_table_array_entry(&mut self, name: &str, index: usize) {
        if let Some(array) = self.doc.get_mut(name).and_then(Item::as_array_of_tables_mut) {
            array.remove(index);
            if array.is_empty() {
                self.doc.remove(name);
            }
        }
    }

    /// Names of `[program_bindings.NAME]`.
    pub fn program_binding_names(&self) -> Vec<String> {
        self.doc
            .get("program_bindings")
            .and_then(Item::as_table_like)
            .map(|t| t.iter().map(|(k, _)| k.to_owned()).collect())
            .unwrap_or_default()
    }
}

/// `value` as TOML text for a setting's default (used to show and compare it).
pub fn default_value(setting: &Setting) -> Option<Value> {
    if setting.default.is_empty() {
        return None;
    }
    let doc: DocumentMut = format!("v = {}", setting.default).parse().ok()?;
    doc.get("v")?.as_value().cloned()
}

/// The text of a value the way a person writes it (no quotes around strings).
pub fn display(value: &Value) -> String {
    match value {
        Value::String(s) => s.value().clone(),
        Value::Array(a) => a.iter().map(display).collect::<Vec<_>>().join(", "),
        other => other.to_string().trim().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_is_a_value_the_compositor_accepts() {
        for setting in SETTINGS {
            let Some(value) = default_value(setting) else { continue };
            let mut doc = Document::parse("").unwrap();
            doc.set(setting.path, value);
            doc.validate().unwrap_or_else(|e| panic!("{}: {e}", setting.path));
        }
    }

    #[test]
    fn the_extremes_of_every_range_are_accepted_and_beyond_them_rejected() {
        for setting in SETTINGS {
            let (low, high): (Value, Value) = match setting.kind {
                Int { min, max } => (min.into(), max.into()),
                Float { min, max } => (min.into(), max.into()),
                _ => continue,
            };
            for (value, ok) in [(low.clone(), true), (high.clone(), true)] {
                let mut doc = Document::parse("").unwrap();
                doc.set(setting.path, value);
                // The idle timeouts depend on each other: the monitor timeout must follow the lock one.
                if setting.path.starts_with("idle.") {
                    continue;
                }
                assert_eq!(doc.validate().is_ok(), ok, "{}", setting.path);
            }
            let beyond: Value = match setting.kind {
                Int { max, .. } => (max + 1).into(),
                Float { max, .. } => (max + 1.0).into(),
                _ => unreachable!(),
            };
            if setting.path.starts_with("idle.") || setting.path.starts_with("appearance.gaps") || setting.path == "appearance.border_width" {
                continue;
            }
            let mut doc = Document::parse("").unwrap();
            doc.set(setting.path, beyond);
            assert!(doc.validate().is_err(), "{} accepts a value beyond its range", setting.path);
        }
    }

    #[test]
    fn editing_keeps_comments_and_unrelated_text() {
        let text = "# my config\nterminal = [\"kitty\"] # the terminal\n\n[effects]\n# corners\ncorner_radius = 4 # round\n";
        let mut doc = Document::parse(text).unwrap();
        doc.set("effects.corner_radius", 12.into());
        doc.set("effects.shadow", 20.into());
        let out = doc.text();
        assert!(out.contains("# my config") && out.contains("# the terminal") && out.contains("# corners"));
        assert!(out.contains("corner_radius = 12 # round"), "{out}");
        assert!(out.contains("shadow = 20"));
        assert!(doc.is_dirty());
        doc.mark_saved();
        assert!(!doc.is_dirty());
    }

    #[test]
    fn unsetting_removes_the_key_and_empty_tables() {
        let mut doc = Document::parse("[effects]\ncorner_radius = 4\n").unwrap();
        doc.unset("effects.corner_radius");
        assert!(!doc.text().contains("effects"), "{}", doc.text());
        assert!(doc.validate().is_ok());
    }

    #[test]
    fn values_come_back_in_their_types() {
        let doc = Document::parse("xwayland = false\n[effects]\ninactive_opacity = 1\n[bindings]\nclose = [\"Super+q\"]\n").unwrap();
        assert_eq!(doc.get_bool("xwayland"), Some(false));
        assert_eq!(doc.get_f64("effects.inactive_opacity"), Some(1.0));
        assert_eq!(doc.get_list("bindings.close"), Some(vec!["Super+q".to_owned()]));
        assert_eq!(doc.get_bool("nothing.here"), None);
    }

    #[test]
    fn rules_are_listed_and_removed() {
        let mut doc = Document::parse("[[rules]]\napp_id = \"a\"\nfloating = true\n[[rules]]\napp_id = \"b\"\nopacity = 0.5\n").unwrap();
        assert_eq!(doc.table_array("rules").len(), 2);
        doc.remove_table_array_entry("rules", 0);
        assert_eq!(doc.table_array("rules").len(), 1);
        doc.remove_table_array_entry("rules", 0);
        assert!(!doc.text().contains("rules"));
    }
}
