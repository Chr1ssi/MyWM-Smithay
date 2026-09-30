use mywm_layout::MAX_NUMBER;
use serde::Deserialize;

/// All supplied selectors must match; strings use exact, case-sensitive matching.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub app_id: Option<String>,
    pub dialog: Option<bool>,
    pub workspace: Option<usize>,
    pub floating: Option<bool>,
    /// Window opacity (0.1 to 1.0), see `[effects]`.
    pub opacity: Option<f32>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Placement {
    /// Workspace number as shown in the bar (1 to 9).
    pub workspace: Option<usize>,
    pub floating: bool,
}

impl Rule {
    pub fn validate(&self) -> Result<(), String> {
        if self.app_id.is_none() && self.dialog.is_none() {
            return Err("at least one selector (app_id or dialog) is required".into());
        }
        if self.app_id.as_ref().is_some_and(|id| id.trim().is_empty()) {
            return Err("app_id must not be empty".into());
        }
        if self.workspace.is_none() && self.floating.is_none() && self.opacity.is_none() {
            return Err("at least one action (workspace, floating or opacity) is required".into());
        }
        if self.opacity.is_some_and(|o| !(0.1..=1.0).contains(&o)) {
            return Err("opacity must be between 0.1 and 1.0".into());
        }
        if self.workspace.is_some_and(|number| !(1..=MAX_NUMBER).contains(&number)) {
            return Err(format!("workspace must be between 1 and {MAX_NUMBER}"));
        }
        Ok(())
    }

    fn matches(&self, app_id: Option<&str>, dialog: bool) -> bool {
        self.app_id.as_deref().is_none_or(|expected| app_id == Some(expected))
            && self.dialog.is_none_or(|expected| dialog == expected)
    }
}

/// The opacity the last matching rule asks for.
pub fn opacity(rules: &[Rule], app_id: Option<&str>, dialog: bool) -> Option<f32> {
    rules.iter().filter(|rule| rule.matches(app_id, dialog)).filter_map(|rule| rule.opacity).next_back()
}

pub fn resolve(rules: &[Rule], app_id: Option<&str>, dialog: bool, float_dialogs: bool) -> Placement {
    let mut placement = Placement { workspace: None, floating: dialog && float_dialogs };
    for rule in rules.iter().filter(|rule| rule.matches(app_id, dialog)) {
        if let Some(workspace) = rule.workspace {
            placement.workspace = Some(workspace);
        }
        if let Some(floating) = rule.floating {
            placement.floating = floating;
        }
    }
    placement
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    #[test]
    fn matching_is_exact_and_all_selectors_are_required() {
        let config = Config::parse(
            "[[rules]]\napp_id = 'test.App'\ndialog = true\nworkspace = 3\nfloating = false",
        )
        .unwrap();
        assert_eq!(
            resolve(&config.rules, Some("test.App"), true, true),
            Placement { workspace: Some(3), floating: false }
        );
        for (id, dialog) in [(None, true), (Some("test.app"), true), (Some("test.App"), false)] {
            assert_eq!(resolve(&config.rules, id, dialog, true).workspace, None);
        }
    }

    #[test]
    fn opacity_comes_from_the_last_matching_rule() {
        let config = Config::parse("[[rules]]\napp_id = 'kitty'\nopacity = 0.9\n[[rules]]\napp_id = 'kitty'\nopacity = 0.8").unwrap();
        assert_eq!(opacity(&config.rules, Some("kitty"), false), Some(0.8));
        assert_eq!(opacity(&config.rules, Some("other"), false), None);
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
            resolve(&config.rules, Some("test.App"), false, true),
            Placement { workspace: Some(2), floating: false }
        );
        assert_eq!(
            resolve(&config.rules, Some("test.App"), true, false),
            Placement { workspace: Some(2), floating: true }
        );
        assert!(!resolve(&[], None, true, false).floating);
        assert!(resolve(&[], None, true, true).floating);
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
        ] {
            assert!(Config::parse(&format!("[[rules]]\n{rule}")).is_err(), "accepted {rule}");
        }
    }
}
