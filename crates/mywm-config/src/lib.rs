//! Configuration for MyWM. The TOML schema matches the River-based MyWM, so an
//! existing `config.toml` keeps working; a few new keys are additions.
mod keys;
mod rules;
pub mod session;
mod sections;

use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
};

pub use keys::{Modifiers, parse_key};
use keys::Result;
use mywm_layout::MAX_NUMBER;
pub use rules::{Placement, Rule, opacity, resolve};
use serde::Deserialize;
pub use sections::{
    AppearanceConfig, HexColor, IdleConfig, KeyboardConfig, OutputConfig, OutputMode, OutputTransform, RenderConfig, VrrConfig, EffectsConfig,
};

/// Leaves at least one number of 1 to 9 free for dynamic workspaces.
const MAX_HOME_WORKSPACES: usize = MAX_NUMBER - 1;

pub use mywm_layout::Direction as OutputDirection;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Reload,
    Wallpaper,
    Lock,
    Terminal,
    Launcher,
    Close,
    Exit,
    ToggleFloating,
    ToggleFullscreen,
    ToggleScratchpad,
    MoveToScratchpad,
    FocusOutput(OutputDirection),
    MoveToOutput(OutputDirection),
    Focus(isize),
    Move(isize),
    ResizeColumn(i32),
    WorkspaceRelative(isize),
    NewWorkspace,
    MoveToNewWorkspace,
    MoveToWorkspaceRelative(isize),
    Workspace(usize),
    MoveToWorkspace(usize),
    /// Index into `Config::program_bindings` (sorted by name).
    Program(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub symbol: u32,
    pub modifiers: Modifiers,
    pub action: Action,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Monitors in workspace order: the first gets workspace 1, the next 2, ...
    pub workspace_outputs: Vec<String>,
    pub gaming_output: Option<String>,
    pub async_outputs: Vec<String>,
    pub wallpaper_directory: String,
    pub idle: IdleConfig,
    pub keyboard: KeyboardConfig,
    pub terminal: Vec<String>,
    pub launcher: Vec<String>,
    pub program_bindings: BTreeMap<String, ProgramBinding>,
    pub bindings: Bindings,
    pub appearance: AppearanceConfig,
    pub float_dialogs: bool,
    pub game_app_id_prefixes: Vec<String>,
    pub vrr: VrrConfig,
    pub render: RenderConfig,
    pub effects: EffectsConfig,
    pub rules: Vec<Rule>,
    /// Addition over the River-based MyWM: run Xwayland for legacy X11 apps (Steam, older games).
    pub xwayland: bool,
    /// Addition over the River-based MyWM (which used kanshi): native output setup.
    pub outputs: Vec<OutputConfig>,
}

/// Where the generated theme lives (`MYWM_THEME_STATE`, else `$XDG_STATE_HOME/mywm/theme.json`).
pub fn theme_state_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MYWM_THEME_STATE").filter(|p| !p.is_empty()) {
        return Some(path.into());
    }
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state")))?;
    Some(base.join("mywm/theme.json"))
}

/// Resolve the independently versioned shell at runtime.
pub fn shell_qml(file: &str) -> PathBuf {
    std::env::var_os("MYWM_SHELL_DIR")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // Sibling checkout of this repository: <parent>/mywm-shell/quickshell
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../mywm-shell/quickshell")
        })
        .join(file)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            workspace_outputs: Vec::new(),
            async_outputs: Vec::new(),
            wallpaper_directory: std::env::var("HOME")
                .map(|home| format!("{home}/Bilder/Wallpaper"))
                .unwrap_or_else(|_| "/usr/share/backgrounds".into()),
            idle: IdleConfig::default(),
            keyboard: KeyboardConfig::default(),
            terminal: vec!["kitty".into()],
            program_bindings: BTreeMap::new(),
            launcher: vec![
                "qs".into(),
                "--path".into(),
                shell_qml("shell.qml").to_string_lossy().into_owned(),
                "--no-duplicate".into(),
            ],
            bindings: Bindings::default(),
            appearance: AppearanceConfig::default(),
            float_dialogs: true,
            gaming_output: None,
            game_app_id_prefixes: Vec::new(),
            vrr: VrrConfig::default(),
            render: RenderConfig::default(),
            effects: EffectsConfig::default(),
            rules: Vec::new(),
            xwayland: true,
            outputs: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramBinding {
    pub keys: Vec<String>,
    pub command: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Bindings {
    reload: Vec<String>,
    wallpaper: Vec<String>,
    lock: Vec<String>,
    terminal: Vec<String>,
    launcher: Vec<String>,
    close: Vec<String>,
    exit: Vec<String>,
    focus_left: Vec<String>,
    focus_right: Vec<String>,
    move_left: Vec<String>,
    move_right: Vec<String>,
    workspace_previous: Vec<String>,
    workspace_next: Vec<String>,
    move_to_workspace_previous: Vec<String>,
    move_to_workspace_next: Vec<String>,
    new_workspace: Vec<String>,
    move_to_new_workspace: Vec<String>,
    toggle_floating: Vec<String>,
    // Additions over the River-based MyWM:
    toggle_fullscreen: Vec<String>,
    column_shrink: Vec<String>,
    column_grow: Vec<String>,
    toggle_scratchpad: Vec<String>,
    move_to_scratchpad: Vec<String>,
    focus_output_left: Vec<String>,
    focus_output_right: Vec<String>,
    focus_output_up: Vec<String>,
    focus_output_down: Vec<String>,
    move_to_output_left: Vec<String>,
    move_to_output_right: Vec<String>,
    move_to_output_up: Vec<String>,
    move_to_output_down: Vec<String>,
    pointer_modifiers: String,
    workspace_modifiers: String,
    move_to_workspace_modifiers: String,
}

impl Default for Bindings {
    fn default() -> Self {
        let keys = |values: &[&str]| values.iter().map(|v| (*v).into()).collect();
        Self {
            reload: keys(&["Super+Shift+r"]),
            wallpaper: keys(&["Super+Shift+w"]),
            lock: keys(&["Super+Escape"]),
            toggle_floating: keys(&["Super+v"]),
            toggle_fullscreen: keys(&["Super+f"]),
            column_shrink: keys(&["Super+minus"]),
            column_grow: keys(&["Super+equal"]),
            toggle_scratchpad: keys(&["Super+grave"]),
            move_to_scratchpad: keys(&["Super+Shift+grave"]),
            focus_output_left: keys(&["Super+Alt+Left"]),
            focus_output_right: keys(&["Super+Alt+Right"]),
            focus_output_up: keys(&["Super+Alt+Up"]),
            focus_output_down: keys(&["Super+Alt+Down"]),
            move_to_output_left: keys(&["Super+Shift+Left"]),
            move_to_output_right: keys(&["Super+Shift+Right"]),
            move_to_output_up: keys(&["Super+Shift+Up"]),
            move_to_output_down: keys(&["Super+Shift+Down"]),
            pointer_modifiers: "Super".into(),
            terminal: keys(&["Super+Return"]),
            launcher: keys(&["Super+Space"]),
            close: keys(&["Super+q"]),
            exit: keys(&["Super+m"]),
            focus_left: keys(&["Super+h", "Super+Left"]),
            focus_right: keys(&["Super+l", "Super+Right"]),
            move_left: keys(&["Super+Shift+h"]),
            move_right: keys(&["Super+Shift+l"]),
            workspace_previous: keys(&["Super+Ctrl+Left", "Super+Ctrl+Up"]),
            workspace_next: keys(&["Super+Ctrl+Right", "Super+Ctrl+Down"]),
            move_to_workspace_previous: keys(&["Super+Ctrl+Shift+Up"]),
            move_to_workspace_next: keys(&["Super+Ctrl+Shift+Down"]),
            new_workspace: keys(&["Super+n"]),
            move_to_new_workspace: keys(&["Super+Shift+n"]),
            workspace_modifiers: "Super".into(),
            move_to_workspace_modifiers: "Super+Shift".into(),
        }
    }
}

fn non_empty(list: &[String]) -> bool {
    list.first().is_some_and(|program| !program.trim().is_empty())
}

impl Config {
    pub fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text)?;
        let mut seen = HashSet::new();
        for output in &config.workspace_outputs {
            if output.trim().is_empty() || !seen.insert(output) {
                return Err("workspace_outputs must list distinct, nonempty monitor names".into());
            }
        }
        if config.workspace_outputs.len() > MAX_HOME_WORKSPACES {
            return Err(format!("workspace_outputs may list at most {MAX_HOME_WORKSPACES} monitors").into());
        }
        if config.gaming_output.as_ref().is_some_and(|output| output.trim().is_empty()) {
            return Err("gaming_output must not be empty".into());
        }
        if config.async_outputs.iter().any(|output| output.trim().is_empty()) {
            return Err("async_outputs must not contain empty output names".into());
        }
        if config.game_app_id_prefixes.iter().any(|prefix| prefix.trim().is_empty()) {
            return Err("game_app_id_prefixes must not contain empty values".into());
        }
        config.vrr.validate()?;
        config.render.validate()?;
        config.effects.validate()?;
        if !non_empty(&config.terminal) {
            return Err("terminal must contain a program, e.g. [\"kitty\"]".into());
        }
        if !non_empty(&config.launcher) {
            return Err("launcher must contain a program".into());
        }
        for (name, binding) in &config.program_bindings {
            if name.trim().is_empty() {
                return Err("program_bindings names must not be empty".into());
            }
            if binding.keys.is_empty() {
                return Err(format!("program_bindings.{name}.keys must not be empty").into());
            }
            if !non_empty(&binding.command) {
                return Err(format!("program_bindings.{name}.command must contain a program").into());
            }
        }
        for (index, rule) in config.rules.iter().enumerate() {
            rule.validate().map_err(|error| format!("rules[{}]: {error}", index + 1))?;
        }
        if !std::path::Path::new(&config.wallpaper_directory).is_absolute() {
            return Err("wallpaper_directory must be an absolute path".into());
        }
        let mut names = HashSet::new();
        for (index, output) in config.outputs.iter().enumerate() {
            output.validate().map_err(|error| format!("outputs[{}]: {error}", index + 1))?;
            if !names.insert(&output.name) {
                return Err(format!("outputs[{}]: duplicate output {}", index + 1, output.name).into());
            }
        }
        config.idle.validate()?;
        config.appearance.validate()?;
        config.keybindings()?;
        config.pointer_modifiers()?;
        Ok(config)
    }

    /// Path of the configuration file, if any location can be determined.
    pub fn path() -> Option<PathBuf> {
        std::env::var_os("MYWM_CONFIG").map(PathBuf::from).or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
                .map(|p| p.join("mywm/config.toml"))
        })
    }

    /// Load `$MYWM_CONFIG` or `$XDG_CONFIG_HOME/mywm/config.toml`; a missing default file means defaults.
    pub fn load() -> Result<Self> {
        let mut config = Self::load_file()?;
        config.apply_theme_state();
        Ok(config)
    }

    fn load_file() -> Result<Self> {
        let explicit = std::env::var_os("MYWM_CONFIG").is_some();
        let Some(path) = Self::path() else { return Ok(Self::default()) };
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()).into()),
            Err(e) if !explicit && e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display()).into()),
        }
    }

    pub fn pointer_modifiers(&self) -> Result<Modifiers> {
        Ok(parse_key(&format!("{}+a", self.bindings.pointer_modifiers))?.1)
    }

    pub fn keybindings(&self) -> Result<Vec<Binding>> {
        use mywm_layout::Direction::*;
        let b = &self.bindings;
        let mut bindings = Vec::new();
        let mut seen = HashSet::new();
        let mut add = |key: &str, action| -> Result<()> {
            let (symbol, modifiers) = parse_key(key)?;
            if !seen.insert((symbol, modifiers)) {
                return Err(format!("Duplicate keybinding: {key}").into());
            }
            bindings.push(Binding { symbol, modifiers, action });
            Ok(())
        };
        for (keys, action) in [
            (&b.reload, Action::Reload),
            (&b.wallpaper, Action::Wallpaper),
            (&b.lock, Action::Lock),
            (&b.terminal, Action::Terminal),
            (&b.launcher, Action::Launcher),
            (&b.close, Action::Close),
            (&b.exit, Action::Exit),
            (&b.toggle_floating, Action::ToggleFloating),
            (&b.toggle_fullscreen, Action::ToggleFullscreen),
            (&b.column_shrink, Action::ResizeColumn(-100)),
            (&b.column_grow, Action::ResizeColumn(100)),
            (&b.toggle_scratchpad, Action::ToggleScratchpad),
            (&b.move_to_scratchpad, Action::MoveToScratchpad),
            (&b.focus_output_left, Action::FocusOutput(Left)),
            (&b.focus_output_right, Action::FocusOutput(Right)),
            (&b.focus_output_up, Action::FocusOutput(Up)),
            (&b.focus_output_down, Action::FocusOutput(Down)),
            (&b.move_to_output_left, Action::MoveToOutput(Left)),
            (&b.move_to_output_right, Action::MoveToOutput(Right)),
            (&b.move_to_output_up, Action::MoveToOutput(Up)),
            (&b.move_to_output_down, Action::MoveToOutput(Down)),
            (&b.focus_left, Action::Focus(-1)),
            (&b.focus_right, Action::Focus(1)),
            (&b.move_left, Action::Move(-1)),
            (&b.move_right, Action::Move(1)),
            (&b.workspace_previous, Action::WorkspaceRelative(-1)),
            (&b.workspace_next, Action::WorkspaceRelative(1)),
            (&b.new_workspace, Action::NewWorkspace),
            (&b.move_to_new_workspace, Action::MoveToNewWorkspace),
            (&b.move_to_workspace_previous, Action::MoveToWorkspaceRelative(-1)),
            (&b.move_to_workspace_next, Action::MoveToWorkspaceRelative(1)),
        ] {
            for key in keys {
                add(key, action)?;
            }
        }
        for workspace in 1..=MAX_NUMBER {
            add(&format!("{}+{}", b.workspace_modifiers, workspace), Action::Workspace(workspace))?;
            add(
                &format!("{}+{}", b.move_to_workspace_modifiers, workspace),
                Action::MoveToWorkspace(workspace),
            )?;
        }
        for (index, binding) in self.program_bindings.values().enumerate() {
            for key in &binding.keys {
                add(key, Action::Program(index))?;
            }
        }
        Ok(bindings)
    }

    /// Environment telling the launcher which terminal to start.
    pub fn terminal_env(&self) -> Vec<(String, String)> {
        let mut env = vec![("MYWM_TERMINAL_COUNT".to_owned(), self.terminal.len().to_string())];
        env.extend(self.terminal.iter().enumerate().map(|(i, arg)| (format!("MYWM_TERMINAL_{i}"), arg.clone())));
        env
    }

    /// Environment describing the theme for helper programs (launcher, shell).
    pub fn theme_env(&self) -> Vec<(String, String)> {
        let a = &self.appearance;
        let state = theme_state_path().map(|p| ("MYWM_THEME_STATE".to_owned(), p.to_string_lossy().into_owned()));
        [
            ("BACKGROUND", a.background),
            ("SURFACE", a.surface),
            ("TEXT", a.text),
            ("MUTED", a.muted_text),
            ("ACCENT", a.active_border),
            ("BORDER", a.inactive_border),
        ]
        .into_iter()
        .map(|(name, color)| (format!("MYWM_COLOR_{name}"), color.css()))
        .chain(state)
        .collect()
    }

    /// Colors generated from the wallpaper (`mywm-theme`) replace the configured ones.
    pub fn apply_theme_state(&mut self) {
        if let Some(path) = theme_state_path()
            && let Ok(text) = std::fs::read_to_string(path)
        {
            self.appearance.apply_theme_json(&text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_outputs_list_distinct_monitors() {
        let config = Config::parse("workspace_outputs = ['DP-3', 'HDMI-A-1']").unwrap();
        assert_eq!(config.workspace_outputs, ["DP-3", "HDMI-A-1"]);
        Config::parse("gaming_output = 'DP-3'").unwrap();
        for invalid in [
            "workspace_outputs = ['DP-1', 'DP-1']",
            "workspace_outputs = ['']",
            "workspace_outputs = ['a','b','c','d','e','f','g','h','i']",
            "gaming_output = ''",
        ] {
            assert!(Config::parse(invalid).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn defaults_and_partial_configuration() {
        let defaults = Config::parse("").unwrap();
        // The River-based MyWM has 50 default bindings; three are additions here.
        assert_eq!(defaults.keybindings().unwrap().len(), 53);
        assert!(defaults.program_bindings.is_empty());
        let config = Config::parse("terminal = ['kitty', '--single-instance']").unwrap();
        assert_eq!(config.terminal[1], "--single-instance");
        // The shipped example has every setting commented out; it must parse both
        // as-is (defaults) and with all example settings enabled.
        let example = include_str!("../example.toml");
        Config::parse(example).unwrap();
        let enabled = example
            .lines()
            .map(|line| match line.strip_prefix('#') {
                Some(rest) if rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '[') => rest,
                _ => line,
            })
            .collect::<Vec<_>>()
            .join("\n");
        Config::parse(&enabled).unwrap();
    }

    #[test]
    fn invalid_config_is_rejected() {
        for text in [
            "workspaces = 3",
            "gaming_workspace = 3",
            "terminal = []",
            "launcher = []",
            "launcher = ['']",
            "terminal = ['']",
            "workpace = 3",
            "[bindings]\nunknown = []",
            "[bindings]\nexit = ['Super+q']",
            "[bindings]\nexit = ['Super+Bogus']",
            "[bindings]\nexit = ['Super+1']",
            "[bindings]\nexit = ['Super+Super+m']",
            "workspace_outputs =",
            "[bindings]\npointer_modifiers = 'Bogus'",
            "[program_bindings.browser]\nkeys = []\ncommand = ['firefox']",
            "[program_bindings.browser]\nkeys = ['Super+b']\ncommand = []",
            "[program_bindings.browser]\nkeys = ['Super+q']\ncommand = ['firefox']",
            "game_app_id_prefixes = ['']",
            "[effects]\ncorner_radius = 65",
            "[effects]\ninactive_opacity = 0",
            "[[rules]]\napp_id = 'a'\nopacity = 1.5",
            "[render]\nmargin_ms = 40",
            "[render]\nmargin_ms = -1",
            "[render]\nlate = true",
            "[vrr]\nenabled = true",
            "[vrr]\nenabled = true\noutput = ''",
            "[vrr]\nenabled = true\noutput = 'DP-3'\ncommand = []",
            "[appearance]\nactive_border = 'blue'",
            "[[outputs]]\nname = ''",
            "[[outputs]]\nname = 'DP-1'\nmode = '1920'",
            "[[outputs]]\nname = 'DP-1'\nmode = '1920x1080@'",
            "[[outputs]]\nname = 'DP-1'\nscale = 0",
            "[[outputs]]\nname = 'DP-1'\ntransform = 'sideways'",
            "[[outputs]]\nname = 'DP-1'\n[[outputs]]\nname = 'DP-1'",
            "[[outputs]]\nname = 'DP-1'\nposition = [1]",
            "[appearance]\ngaps_outer = 129",
        ] {
            assert!(Config::parse(text).is_err(), "accepted {text}");
        }
    }

    #[test]
    fn program_bindings_add_commands_and_detect_duplicates() {
        let config = Config::parse(
            "[program_bindings.browser]\nkeys = ['Super+b', 'Super+Shift+b']\ncommand = ['firefox', '--private-window']",
        )
        .unwrap();
        assert_eq!(config.keybindings().unwrap().len(), 55);
        let binding = config.program_bindings.get("browser").unwrap();
        assert_eq!(binding.command, ["firefox", "--private-window"]);
    }

    #[test]
    fn outputs_mirror_the_kanshi_profile() {
        let config = Config::parse(
            r#"
            [[outputs]]
            name = "HDMI-A-1"
            mode = "2560x1080@60Hz"
            position = [0, 0]
            [[outputs]]
            name = "DP-3"
            mode = "2560x1440@143.97"
            position = [0, 1080]
            [[outputs]]
            name = "DP-1"
            position = [2560, 0]
            transform = "270"
            [[outputs]]
            name = "DP-2"
            enable = false
        "#,
        )
        .unwrap();
        let dp3 = &config.outputs[1];
        assert_eq!(
            dp3.parsed_mode().unwrap(),
            Some(OutputMode { width: 2560, height: 1440, refresh_mhz: Some(143_970) })
        );
        assert_eq!(config.outputs[0].parsed_mode().unwrap().unwrap().refresh_mhz, Some(60_000));
        assert_eq!(config.outputs[2].parsed_mode().unwrap(), None);
        assert_eq!(config.outputs[2].transform, OutputTransform::Rotate270);
        assert_eq!(config.outputs[2].scale, 1.0);
        assert!(!config.outputs[3].enable);
    }

    #[test]
    fn theme_env_exposes_css_colors() {
        let env = Config::default().theme_env();
        assert!(env.contains(&("MYWM_COLOR_ACCENT".into(), "#89b4fa".into())));
    }
}
