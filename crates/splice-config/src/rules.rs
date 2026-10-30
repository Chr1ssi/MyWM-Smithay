use splice_layout::MAX_NUMBER;
use serde::Deserialize;

/// All supplied selectors must match. Strings match case-sensitively, as a whole; `*` stands for
/// any run of characters (`steam_app_*`, `*Bitwarden*`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub dialog: Option<bool>,
    pub workspace: Option<usize>,
    /// Monitor (connector name, `DP-3`) to open on, unless `workspace` names one.
    pub output: Option<String>,
    pub floating: Option<bool>,
    /// Open fullscreen (`true`) or not even if the client asks to (`false`).
    pub fullscreen: Option<bool>,
    /// Float on every workspace of its monitor (picture in picture).
    pub sticky: Option<bool>,
    /// Window opacity (0.1 to 1.0) in place of `effects.window_opacity`, see `[effects]`.
    pub opacity: Option<f32>,
    /// Treat the window as video (`true`: no transparency) or not even if it looks like one (`false`).
    pub video: Option<bool>,
    /// Shown as a black area in screenshots, recordings and streams.
    pub block_from_capture: Option<bool>,
}

/// What rules select a window by.
#[derive(Clone, Copy, Debug, Default)]
pub struct Subject<'a> {
    pub app_id: Option<&'a str>,
    pub title: Option<&'a str>,
    pub dialog: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Placement {
    /// Workspace number as shown in the bar (1 to 9).
    pub workspace: Option<usize>,
    pub output: Option<String>,
    pub floating: bool,
    pub fullscreen: Option<bool>,
    pub sticky: bool,
}

impl Rule {
    pub fn validate(&self) -> Result<(), String> {
        if self.app_id.is_none() && self.title.is_none() && self.dialog.is_none() {
            return Err("at least one selector (app_id, title or dialog) is required".into());
        }
        for (name, value) in [("app_id", &self.app_id), ("title", &self.title), ("output", &self.output)] {
            if value.as_ref().is_some_and(|v| v.trim().is_empty()) {
                return Err(format!("{name} must not be empty"));
            }
        }
        if self.workspace.is_none()
            && self.output.is_none()
            && self.floating.is_none()
            && self.fullscreen.is_none()
            && self.sticky.is_none()
            && self.opacity.is_none()
            && self.video.is_none()
            && self.block_from_capture.is_none()
        {
            return Err(
                "at least one action (workspace, output, floating, fullscreen, sticky, opacity, video or block_from_capture) is required"
                    .into(),
            );
        }
        if self.opacity.is_some_and(|o| !(0.1..=1.0).contains(&o)) {
            return Err("opacity must be between 0.1 and 1.0".into());
        }
        if self.workspace.is_some_and(|number| !(1..=MAX_NUMBER).contains(&number)) {
            return Err(format!("workspace must be between 1 and {MAX_NUMBER}"));
        }
        Ok(())
    }

    fn matches(&self, window: Subject) -> bool {
        let text = |pattern: &Option<String>, value: Option<&str>| {
            pattern.as_deref().is_none_or(|pattern| value.is_some_and(|value| glob(pattern, value)))
        };
        text(&self.app_id, window.app_id)
            && text(&self.title, window.title)
            && self.dialog.is_none_or(|expected| window.dialog == expected)
    }
}

/// Whether `text` matches `pattern` as a whole, `*` standing for any run of characters.
pub fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    // Without a `*` there is a single part, which must be the whole text.
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else { return false };
    let mut parts: Vec<&str> = parts.collect();
    let Some(last) = parts.pop() else { return rest.is_empty() };
    for part in parts {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// The opacity the last matching rule asks for.
pub fn opacity(rules: &[Rule], window: Subject) -> Option<f32> {
    rules.iter().filter(|rule| rule.matches(window)).filter_map(|rule| rule.opacity).next_back()
}

/// What the last matching rule says about the window being video (`None`: it does not say).
pub fn video(rules: &[Rule], window: Subject) -> Option<bool> {
    rules.iter().filter(|rule| rule.matches(window)).filter_map(|rule| rule.video).next_back()
}

/// Whether the last matching rule keeps the window out of captures.
pub fn blocks_capture(rules: &[Rule], window: Subject) -> bool {
    rules.iter().filter(|rule| rule.matches(window)).filter_map(|rule| rule.block_from_capture).next_back() == Some(true)
}

/// Whether any rule keeps windows out of captures (otherwise captures need no second look).
pub fn any_blocks_capture(rules: &[Rule]) -> bool {
    rules.iter().any(|rule| rule.block_from_capture == Some(true))
}

pub fn resolve(rules: &[Rule], window: Subject, float_dialogs: bool) -> Placement {
    let mut placement = Placement { floating: window.dialog && float_dialogs, ..Placement::default() };
    for rule in rules.iter().filter(|rule| rule.matches(window)) {
        if let Some(workspace) = rule.workspace {
            placement.workspace = Some(workspace);
        }
        if let Some(output) = &rule.output {
            placement.output = Some(output.clone());
        }
        if let Some(floating) = rule.floating {
            placement.floating = floating;
        }
        if let Some(fullscreen) = rule.fullscreen {
            placement.fullscreen = Some(fullscreen);
        }
        if let Some(sticky) = rule.sticky {
            placement.sticky = sticky;
        }
    }
    placement
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    fn app(app_id: Option<&str>, dialog: bool) -> Subject<'_> {
        Subject { app_id, title: None, dialog }
    }

    #[test]
    fn matching_is_exact_and_all_selectors_are_required() {
        let config = Config::parse(
            "[[rules]]\napp_id = 'test.App'\ndialog = true\nworkspace = 3\nfloating = false",
        )
        .unwrap();
        assert_eq!(
            resolve(&config.rules, app(Some("test.App"), true), true),
            Placement { workspace: Some(3), floating: false, ..Placement::default() }
        );
        for (id, dialog) in [(None, true), (Some("test.app"), true), (Some("test.App"), false)] {
            assert_eq!(resolve(&config.rules, app(id, dialog), true).workspace, None);
        }
    }

    #[test]
    fn video_comes_from_the_last_matching_rule() {
        let config = Config::parse("[[rules]]\napp_id = 'mpv'\nvideo = true\n[[rules]]\napp_id = 'mpv'\nvideo = false").unwrap();
        assert_eq!(video(&config.rules, app(Some("mpv"), false)), Some(false));
        assert_eq!(video(&config.rules, app(Some("other"), false)), None);
        assert!(Config::parse("[[rules]]\napp_id = 'mpv'\nvideo = true").is_ok());
    }

    #[test]
    fn opacity_comes_from_the_last_matching_rule() {
        let config = Config::parse("[[rules]]\napp_id = 'kitty'\nopacity = 0.9\n[[rules]]\napp_id = 'kitty'\nopacity = 0.8").unwrap();
        assert_eq!(opacity(&config.rules, app(Some("kitty"), false)), Some(0.8));
        assert_eq!(opacity(&config.rules, app(Some("other"), false)), None);
    }

    #[test]
    fn last_matching_value_wins_without_erasing_other_fields() {
        let config = Config::parse(
            r#"
            [[rules]]
            app_id = "test.App"
            workspace = 2
            floating = true
            [[rules]]
            app_id = "test.App"
            floating = false
            [[rules]]
            dialog = true
            floating = true
        "#,
        )
        .unwrap();
        assert_eq!(
            resolve(&config.rules, app(Some("test.App"), false), true),
            Placement { workspace: Some(2), floating: false, ..Placement::default() }
        );
        assert_eq!(
            resolve(&config.rules, app(Some("test.App"), true), false),
            Placement { workspace: Some(2), floating: true, ..Placement::default() }
        );
        assert!(!resolve(&[], app(None, true), false).floating);
        assert!(resolve(&[], app(None, true), true).floating);
    }

    #[test]
    fn invalid_rules_are_rejected() {
        for rule in [
            "floating = true",
            "app_id = ''\nfloating = true",
            "app_id = 'app'",
            "dialog = true\nworkspace = 0",
            "dialog = false\nworkspace = 10",
            "app_id = 'app'\nfloatng = true",
            "title = ''\nfloating = true",
            "app_id = 'app'\noutput = ''",
            "title = 'x'",
        ] {
            assert!(Config::parse(&format!("[[rules]]\n{rule}")).is_err(), "accepted {rule}");
        }
    }

    #[test]
    fn stars_match_any_run_of_characters() {
        for (pattern, text) in [
            ("steam_app_*", "steam_app_1234"),
            ("steam_app_*", "steam_app_"),
            ("*", ""),
            ("*Bitwarden*", "Tresor - Bitwarden - Firefox"),
            ("a*b*c", "a-b-b-c"),
            ("exact", "exact"),
        ] {
            assert!(glob(pattern, text), "{pattern} should match {text}");
        }
        for (pattern, text) in [("steam_app_*", "steam_ap"), ("exact", "exactly"), ("a*a", "a"), ("*.exe", "game.exe.bak")] {
            assert!(!glob(pattern, text), "{pattern} should not match {text}");
        }
    }

    #[test]
    fn block_from_capture_follows_the_last_matching_rule() {
        let config = Config::parse(
            "[[rules]]\ntitle = '*Bitwarden*'\nblock_from_capture = true\n[[rules]]\ntitle = 'Bitwarden Hilfe*'\nblock_from_capture = false",
        )
        .unwrap();
        let titled = |title| Subject { app_id: Some("firefox"), title: Some(title), dialog: false };
        assert!(any_blocks_capture(&config.rules));
        assert!(blocks_capture(&config.rules, titled("Tresor - Bitwarden")));
        assert!(!blocks_capture(&config.rules, titled("Bitwarden Hilfe - Bitwarden")));
        assert!(!blocks_capture(&config.rules, titled("Nachrichten")));
        assert!(!any_blocks_capture(&[]));
    }

    #[test]
    fn title_output_and_fullscreen() {
        let config = Config::parse(
            r#"
            [[rules]]
            title = "*Bitwarden*"
            floating = true
            [[rules]]
            app_id = "steam_app_*"
            output = "DP-3"
            fullscreen = true
        "#,
        )
        .unwrap();
        let vault = Subject { app_id: Some("firefox"), title: Some("Tresor - Bitwarden"), dialog: false };
        assert!(resolve(&config.rules, vault, false).floating);
        // A window without a title never matches a title selector.
        assert!(!resolve(&config.rules, app(Some("firefox"), false), false).floating);
        assert_eq!(
            resolve(&config.rules, app(Some("steam_app_570"), false), false),
            Placement { output: Some("DP-3".into()), fullscreen: Some(true), ..Placement::default() }
        );
    }
}
