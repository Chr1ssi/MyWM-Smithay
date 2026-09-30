//! Smaller configuration sections.
use mywm_layout::{Appearance, Color};
use serde::Deserialize;

use crate::keys::Result;

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeyboardConfig {
    pub layout: String,
    pub variant: String,
    pub options: String,
}

impl Default for KeyboardConfig {
    fn default() -> Self {
        Self { layout: "de".into(), variant: String::new(), options: String::new() }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IdleConfig {
    pub lock_after_seconds: u32,
    pub monitor_off_after_seconds: u32,
}

impl Default for IdleConfig {
    fn default() -> Self {
        Self { lock_after_seconds: 300, monitor_off_after_seconds: 600 }
    }
}

impl IdleConfig {
    pub fn validate(&self) -> Result<()> {
        if self.lock_after_seconds > 86400 || self.monitor_off_after_seconds > 86400 {
            return Err("idle timeouts must be between 0 and 86400 seconds".into());
        }
        if self.monitor_off_after_seconds > 0
            && (self.lock_after_seconds == 0 || self.monitor_off_after_seconds <= self.lock_after_seconds)
        {
            return Err("monitor-off timeout must be later than the enabled lock timeout".into());
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VrrConfig {
    pub enabled: bool,
    pub output: String,
    pub command: Vec<String>,
}

impl Default for VrrConfig {
    fn default() -> Self {
        Self { enabled: false, output: String::new(), command: vec!["wlr-randr".into()] }
    }
}

impl VrrConfig {
    pub fn validate(&self) -> Result<()> {
        if self.enabled && self.output.trim().is_empty() {
            return Err("vrr.output must name an output when VRR is enabled".into());
        }
        if self.enabled && self.command.first().is_none_or(|program| program.trim().is_empty()) {
            return Err("vrr.command must contain a program when VRR is enabled".into());
        }
        Ok(())
    }
}

/// `#RRGGBB` in the config file.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(try_from = "String")]
pub struct HexColor(pub Color);

impl TryFrom<String> for HexColor {
    type Error = String;
    fn try_from(value: String) -> std::result::Result<Self, String> {
        Color::parse(&value).map(Self).map_err(|_| "border colors must use #RRGGBB".to_string())
    }
}

impl HexColor {
    pub fn css(self) -> String {
        let [r, g, b, _] = self.0.0;
        format!("#{:02x}{:02x}{:02x}", (r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8)
    }
}

fn hex(value: &str) -> HexColor {
    HexColor::try_from(value.to_string()).unwrap()
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppearanceConfig {
    pub gaps_inner: i32,
    pub gaps_outer: i32,
    pub border_width: i32,
    pub active_border: HexColor,
    pub inactive_border: HexColor,
    pub background: HexColor,
    pub surface: HexColor,
    pub text: HexColor,
    pub muted_text: HexColor,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            gaps_inner: 8,
            gaps_outer: 8,
            border_width: 2,
            active_border: hex("#89b4fa"),
            inactive_border: hex("#45475a"),
            background: hex("#1e1e2e"),
            surface: hex("#313244"),
            text: hex("#cdd6f4"),
            muted_text: hex("#a6adc8"),
        }
    }
}

impl AppearanceConfig {
    /// The subset the layout and renderer need.
    pub fn layout(&self) -> Appearance {
        Appearance {
            gaps_inner: self.gaps_inner,
            gaps_outer: self.gaps_outer,
            border_width: self.border_width,
            active_border: self.active_border.0,
            inactive_border: self.inactive_border.0,
            background: self.background.0,
        }
    }

    pub fn validate(&self) -> std::result::Result<(), String> {
        self.layout().validate()
    }
}
