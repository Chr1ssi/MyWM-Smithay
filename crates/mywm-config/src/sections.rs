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

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VrrConfig {
    pub enabled: bool,
    pub output: String,
}

impl VrrConfig {
    pub fn validate(&self) -> Result<()> {
        if self.enabled && self.output.trim().is_empty() {
            return Err("vrr.output must name an output when VRR is enabled".into());
        }
        Ok(())
    }
}

/// Pointer and keyboard behaviour; unset values leave the device's own setting alone.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InputConfig {
    /// `flat` (raw, no acceleration: best for games) or `adaptive`.
    pub accel_profile: Option<AccelProfile>,
    /// -1.0 (slow) to 1.0 (fast).
    pub accel_speed: Option<f64>,
    pub natural_scroll: Option<bool>,
    /// Tap to click on touchpads.
    pub tap: Option<bool>,
    pub left_handed: Option<bool>,
    /// Key repeats per second and the delay before repeating starts (milliseconds).
    pub repeat_rate: i32,
    pub repeat_delay: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AccelProfile {
    Flat,
    Adaptive,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            accel_profile: None,
            accel_speed: None,
            natural_scroll: None,
            tap: Some(true),
            left_handed: None,
            repeat_rate: 25,
            repeat_delay: 200,
        }
    }
}

impl InputConfig {
    pub fn validate(&self) -> Result<()> {
        if self.accel_speed.is_some_and(|s| !(-1.0..=1.0).contains(&s)) {
            return Err("input.accel_speed must be between -1.0 and 1.0".into());
        }
        if !(1..=200).contains(&self.repeat_rate) || !(50..=2000).contains(&self.repeat_delay) {
            return Err("input.repeat_rate must be 1-200 and input.repeat_delay 50-2000".into());
        }
        Ok(())
    }
}

/// Visual effects outside of fullscreen windows (games never get them).
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EffectsConfig {
    /// Corner radius of windows and their borders in logical pixels; 0 keeps square corners.
    pub corner_radius: i32,
    /// Opacity of windows that do not have the focus (1.0 = opaque).
    pub inactive_opacity: f32,
    /// Milliseconds windows take to slide to a new place and to fade in; 0 turns animations off.
    pub animation_ms: u32,
    /// Soft drop shadow around windows, extent in logical pixels; 0 for none.
    pub shadow: i32,
    /// Blur of the wallpaper behind translucent windows, 1 (light) to 16 (strong); 0 for none.
    pub blur: u32,
}

impl Default for EffectsConfig {
    fn default() -> Self {
        Self { corner_radius: 0, inactive_opacity: 1.0, animation_ms: 0, shadow: 0, blur: 0 }
    }
}

impl EffectsConfig {
    pub fn validate(&self) -> Result<()> {
        if !(0..=64).contains(&self.corner_radius) {
            return Err("effects.corner_radius must be between 0 and 64".into());
        }
        if !(0.1..=1.0).contains(&self.inactive_opacity) {
            return Err("effects.inactive_opacity must be between 0.1 and 1.0".into());
        }
        if self.animation_ms > 1000 {
            return Err("effects.animation_ms must be between 0 and 1000".into());
        }
        if self.blur > 16 {
            return Err("effects.blur must be between 0 and 16".into());
        }
        if !(0..=64).contains(&self.shadow) {
            return Err("effects.shadow must be between 0 and 64".into());
        }
        Ok(())
    }
}

/// Frame pacing of the hardware backend.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RenderConfig {
    /// Start each frame as late as possible before the vblank so it shows the newest
    /// client content (lower latency); off while VRR is active.
    pub late_scheduling: bool,
    /// Safety margin before the vblank, on top of the measured render time.
    pub margin_ms: f64,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self { late_scheduling: false, margin_ms: 2.0 }
    }
}

impl RenderConfig {
    pub fn validate(&self) -> Result<()> {
        if !(0.0..=16.0).contains(&self.margin_ms) {
            return Err("render.margin_ms must be between 0 and 16".into());
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
    /// Border of a window that asks for attention (`xdg-activation`) without being focused.
    pub urgent_border: HexColor,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            gaps_inner: 8,
            gaps_outer: 8,
            border_width: 2,
            active_border: hex("#89b4fa"),
            inactive_border: hex("#45475a"),
            urgent_border: hex("#f38ba8"),
            background: hex("#1e1e2e"),
            surface: hex("#313244"),
            text: hex("#cdd6f4"),
            muted_text: hex("#a6adc8"),
        }
    }
}

impl AppearanceConfig {
    /// Take the Material-You roles of a generated theme (`mywm-theme`) as window colors.
    pub fn apply_theme_json(&mut self, json: &str) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else { return };
        let color = |name: &str| {
            value["colors"][name]["default"]["hex"].as_str().and_then(|hex| HexColor::try_from(hex.to_owned()).ok())
        };
        for (role, target) in [
            ("surface", &mut self.background),
            ("surface_container", &mut self.surface),
            ("on_surface", &mut self.text),
            ("on_surface_variant", &mut self.muted_text),
            ("primary", &mut self.active_border),
            ("outline_variant", &mut self.inactive_border),
        ] {
            if let Some(color) = color(role) {
                *target = color;
            }
        }
    }

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

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OutputTransform {
    #[default]
    Normal,
    #[serde(rename = "90")]
    Rotate90,
    #[serde(rename = "180")]
    Rotate180,
    #[serde(rename = "270")]
    Rotate270,
    Flipped,
    #[serde(rename = "flipped-90")]
    Flipped90,
    #[serde(rename = "flipped-180")]
    Flipped180,
    #[serde(rename = "flipped-270")]
    Flipped270,
}

/// A video mode request; the refresh rate is optional and in millihertz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputMode {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: Option<u32>,
}

/// One `[[outputs]]` entry, equivalent to a line of the kanshi profile.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    /// Connector name, e.g. `DP-3`.
    pub name: String,
    #[serde(default = "yes")]
    pub enable: bool,
    /// `WIDTHxHEIGHT` or `WIDTHxHEIGHT@HZ` (a trailing `Hz` is allowed); default: preferred mode.
    pub mode: Option<String>,
    /// Top-left corner in logical pixels; default: to the right of the previous output.
    pub position: Option<[i32; 2]>,
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default)]
    pub transform: OutputTransform,
}

fn yes() -> bool {
    true
}

fn one() -> f64 {
    1.0
}

impl OutputConfig {
    pub fn parsed_mode(&self) -> std::result::Result<Option<OutputMode>, String> {
        let Some(mode) = &self.mode else { return Ok(None) };
        let invalid = || format!("mode {mode:?} must look like 2560x1440 or 2560x1440@144");
        let (size, refresh) = match mode.split_once('@') {
            Some((size, refresh)) => (size, Some(refresh)),
            None => (mode.as_str(), None),
        };
        let (width, height) = size.split_once('x').ok_or_else(invalid)?;
        let refresh_mhz = refresh
            .map(|hz| {
                let hz: f64 = hz.trim_end_matches("Hz").parse().map_err(|_| invalid())?;
                (hz > 0.0 && hz < 1000.0).then(|| (hz * 1000.0).round() as u32).ok_or_else(invalid)
            })
            .transpose()?;
        let (width, height): (u32, u32) =
            (width.parse().map_err(|_| invalid())?, height.parse().map_err(|_| invalid())?);
        if width == 0 || height == 0 {
            return Err(invalid());
        }
        Ok(Some(OutputMode { width, height, refresh_mhz }))
    }

    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("name must not be empty".into());
        }
        self.parsed_mode()?;
        if !(0.1..=10.0).contains(&self.scale) {
            return Err("scale must be between 0.1 and 10".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod theme_tests {
    use super::*;

    #[test]
    fn generated_theme_overrides_only_the_roles_it_has() {
        let mut appearance = AppearanceConfig::default();
        let before = appearance.text;
        appearance.apply_theme_json(r##"{"colors":{"primary":{"default":{"hex":"#112233"}},"surface":{"default":{"hex":"nope"}}}}"##);
        assert_eq!(appearance.active_border, hex("#112233"));
        assert_eq!(appearance.text, before);
        assert_eq!(appearance.background, AppearanceConfig::default().background);
        appearance.apply_theme_json("not json");
    }
}
