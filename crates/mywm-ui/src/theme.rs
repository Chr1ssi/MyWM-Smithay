//! Colors and measures of the shell (mywm-shell's `Theme.qml`): the wallpaper's generated colors
//! (`theme.json`, Material roles) where there are some, else mywm's `[appearance]` palette.
use crate::Color;

/// The colors mywm's `[appearance]` section gives the shell (`MYWM_COLOR_*` for Quickshell).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub background: Color,
    pub surface: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub border: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub background: Color,
    pub surface: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    /// Recording and other things that should not go unnoticed.
    pub alert: Color,
    pub border: Color,
}

pub const FONT_FAMILY: &str = "JetBrainsMono Nerd Font";
/// Icons here get their real width; in `FONT_FAMILY` they overhang to the right of their cell.
pub const ICON_FONT_FAMILY: &str = "JetBrainsMono Nerd Font Propo";
pub const PANEL_PADDING: f32 = 16.0;
/// The bar is transparent; its groups float in it as rounded islands.
pub const BAR_HEIGHT: f32 = 40.0;
pub const BAR_MARGIN: f32 = 6.0;
pub const ISLAND_HEIGHT: f32 = 32.0;
pub const PANEL_RADIUS: f32 = 20.0;
pub const GLASS_OPACITY: f32 = 0.7;

/// The image `theme_json` (the contents of `theme.json`) was generated from: the lock screen's background. Like
/// the colors, only of a file of version 1 with colors.
pub fn generated_source(theme_json: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(theme_json).ok()?;
    if parsed["version"] != 1 || !parsed["colors"].is_object() {
        return None;
    }
    parsed["source"].as_str().filter(|source| !source.is_empty()).map(str::to_owned)
}

impl Theme {
    /// The generated colors of `theme_json` (the contents of `theme.json`, version 1) where it has them,
    /// `palette` for the rest. A file that does not parse counts as missing.
    pub fn new(palette: Palette, theme_json: Option<&str>) -> Self {
        let generated: Option<serde_json::Value> = theme_json
            .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
            .filter(|parsed| parsed["version"] == 1 && parsed["colors"].is_object());
        let token = |name: &str, fallback: Color| {
            generated
                .as_ref()
                .and_then(|parsed| parsed["colors"][name]["default"]["hex"].as_str())
                .and_then(Color::parse)
                .unwrap_or(fallback)
        };
        Self {
            background: token("surface", palette.background),
            surface: token("surface_container", palette.surface),
            text: token("on_surface", palette.text),
            muted: token("on_surface_variant", palette.muted),
            accent: token("primary", palette.accent),
            alert: token("error", Color::rgba(0xf3, 0x8b, 0xa8, 255)),
            border: token("outline_variant", palette.border),
        }
    }

    /// Layer surfaces get no compositor blur, so translucency is plain alpha.
    pub fn glass(&self) -> Color {
        self.background.with_alpha(GLASS_OPACITY)
    }

    pub fn glass_surface(&self) -> Color {
        self.surface.with_alpha(0.55)
    }

    pub fn glass_border(&self) -> Color {
        self.border.with_alpha(0.6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        let c = |v| Color::rgba(v, v, v, 255);
        Palette { background: c(1), surface: c(2), text: c(3), muted: c(4), accent: c(5), border: c(6) }
    }

    #[test]
    fn generated_colors_win_over_the_palette() {
        let json = r##"{"version":1,"colors":{"primary":{"default":{"hex":"#112233"}},"error":{"default":{"hex":"#ff0000"}}}}"##;
        let theme = Theme::new(palette(), Some(json));
        assert_eq!(theme.accent, Color::rgba(0x11, 0x22, 0x33, 255));
        assert_eq!(theme.alert, Color::rgba(255, 0, 0, 255));
        assert_eq!(theme.background, palette().background);
        assert_eq!(theme.glass(), palette().background.with_alpha(0.7));
    }

    #[test]
    fn a_broken_or_foreign_file_leaves_the_palette() {
        for json in ["not json", r#"{"version":2,"colors":{}}"#, r#"{"version":1}"#] {
            let theme = Theme::new(palette(), Some(json));
            assert_eq!(theme.accent, palette().accent, "{json}");
            assert_eq!(theme.alert, Color::rgba(0xf3, 0x8b, 0xa8, 255));
        }
    }
}
