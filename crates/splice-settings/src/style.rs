//! The look of the settings window: the colors of the shell's theme (`[appearance]` of the config, replaced by the
//! wallpaper's generated colors in `theme.json` where there are some, as in the shell), the system's sans-serif font,
//! and rounded cards that group the settings.
use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Response, RichText, Sense, Stroke, TextStyle, Ui, Vec2,
};
use splice_config::Config;

/// The shell's colors (the same roles as `splice_ui::Theme`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colors {
    pub background: Color32,
    pub surface: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub border: Color32,
    /// Problems and duplicate keys.
    pub alert: Color32,
}

/// Cards and inputs round their corners by these.
const CARD_RADIUS: u8 = 16;
const CONTROL_RADIUS: u8 = 10;

fn parse(text: &str) -> Option<Color32> {
    let hex = text.trim().strip_prefix('#')?;
    (hex.len() == 6).then_some(())?;
    let byte = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some(Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?))
}

impl Colors {
    /// The colors of `config_text` (the defaults where it does not parse), then those `theme_json` (the contents
    /// of `theme.json`, version 1) has: the roles and their names are the shell's (`splice_ui::Theme::new`).
    pub fn load(config_text: &str, theme_json: Option<&str>) -> Self {
        let config = Config::parse(config_text).unwrap_or_default();
        let a = &config.appearance;
        let color = |c: splice_config::HexColor| parse(&c.css()).unwrap_or(Color32::GRAY);
        let generated: Option<serde_json::Value> = theme_json
            .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
            .filter(|parsed| parsed["version"] == 1 && parsed["colors"].is_object());
        let token = |name: &str, fallback: Color32| {
            generated.as_ref().and_then(|parsed| parsed["colors"][name]["default"]["hex"].as_str()).and_then(parse).unwrap_or(fallback)
        };
        Self {
            background: token("surface", color(a.background)),
            surface: token("surface_container", color(a.surface)),
            text: token("on_surface", color(a.text)),
            muted: token("on_surface_variant", color(a.muted_text)),
            accent: token("primary", color(a.active_border)),
            border: token("outline_variant", color(a.inactive_border)),
            alert: token("error", Color32::from_rgb(0xf3, 0x8b, 0xa8)),
        }
    }

    /// The colors the shell shows now: the config text being edited and the saved `theme.json`.
    pub fn current(config_text: &str) -> Self {
        let json = splice_config::theme_state_path().and_then(|path| std::fs::read_to_string(path).ok());
        Self::load(config_text, json.as_deref())
    }

    fn dark(&self) -> bool {
        let [r, g, b, _] = self.background.to_array();
        0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b) < 140.0
    }

    fn sunken(&self) -> Color32 {
        mix(self.background, if self.dark() { Color32::BLACK } else { Color32::WHITE }, 0.18)
    }

    fn control(&self) -> Color32 {
        mix(self.surface, self.text, 0.07)
    }

    fn control_hovered(&self) -> Color32 {
        mix(self.surface, self.text, 0.15)
    }
}

/// The background of the content area: a little away from the sidebar's color, so the cards stand out.
pub fn sunken(colors: &Colors) -> Color32 {
    colors.sunken()
}

/// Fill of a button or input that lies on a card.
pub fn control(colors: &Colors) -> Color32 {
    colors.control()
}

fn mix(from: Color32, to: Color32, by: f32) -> Color32 {
    let channel = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * by).round() as u8;
    Color32::from_rgb(channel(from.r(), to.r()), channel(from.g(), to.g()), channel(from.b(), to.b()))
}

fn round(radius: u8) -> CornerRadius {
    CornerRadius::same(radius)
}

/// Fonts, text sizes, spacing and colors for the whole window.
pub fn install(ctx: &egui::Context, colors: &Colors) {
    if let Some(fonts) = system_fonts() {
        ctx.set_fonts(fonts);
    }
    ctx.set_visuals(visuals(colors));
    ctx.style_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::new(24.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(14.5, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.5, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(12.5, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(13.5, FontFamily::Monospace)),
        ]
        .into();
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.spacing.button_padding = Vec2::new(14.0, 6.0);
        style.spacing.interact_size = Vec2::new(40.0, 32.0);
        style.spacing.slider_rail_height = 6.0;
        style.spacing.combo_width = 140.0;
        style.spacing.menu_margin = Margin::same(8);
    });
}

fn visuals(colors: &Colors) -> egui::Visuals {
    let mut visuals = if colors.dark() { egui::Visuals::dark() } else { egui::Visuals::light() };
    visuals.panel_fill = colors.sunken();
    visuals.window_fill = colors.surface;
    visuals.window_corner_radius = round(CARD_RADIUS);
    visuals.window_stroke = Stroke::new(1.0_f32, colors.border);
    visuals.window_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(90) };
    visuals.menu_corner_radius = round(12);
    visuals.popup_shadow = visuals.window_shadow;
    visuals.extreme_bg_color = mix(colors.surface, if colors.dark() { Color32::BLACK } else { Color32::WHITE }, 0.16);
    visuals.faint_bg_color = colors.control();
    visuals.hyperlink_color = colors.accent;
    visuals.selection.bg_fill = mix(colors.surface, colors.accent, 0.45);
    visuals.selection.stroke = Stroke::new(1.0_f32, colors.accent);
    visuals.slider_trailing_fill = true;
    visuals.override_text_color = None;

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = colors.surface;
    widgets.noninteractive.weak_bg_fill = colors.surface;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, colors.border.gamma_multiply(0.45));
    widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, colors.text);
    widgets.noninteractive.corner_radius = round(CONTROL_RADIUS);
    for (state, fill) in [(&mut widgets.inactive, colors.control()), (&mut widgets.hovered, colors.control_hovered())] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.0_f32, colors.text);
        state.corner_radius = round(CONTROL_RADIUS);
        state.expansion = 0.0;
    }
    widgets.hovered.fg_stroke = Stroke::new(1.5_f32, colors.text);
    let pressed = mix(colors.surface, colors.accent, 0.35);
    for state in [&mut widgets.active, &mut widgets.open] {
        state.bg_fill = pressed;
        state.weak_bg_fill = pressed;
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.5_f32, colors.text);
        state.corner_radius = round(CONTROL_RADIUS);
        state.expansion = 0.0;
    }
    visuals
}

/// The family fontconfig matches for "Sans Serif" (the font the shell uses), first in the proportional family;
/// `None` (egui keeps its own fonts) when fontconfig is not at hand or the file does not load.
fn system_fonts() -> Option<FontDefinitions> {
    let output = std::process::Command::new("fc-match").args(["--format", "%{file}\n%{index}", "Sans Serif"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let (file, index) = text.split_once('\n')?;
    let bytes = std::fs::read(file.trim()).ok()?;
    let mut data = FontData::from_owned(bytes);
    data.index = index.trim().parse().unwrap_or(0);
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("system".to_owned(), std::sync::Arc::new(data));
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "system".to_owned());
    Some(fonts)
}

/// Settings that belong together on a rounded card.
pub fn card<R>(ui: &mut Ui, colors: &Colors, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(colors.surface)
        .stroke(Stroke::new(1.0_f32, colors.border.gamma_multiply(0.35)))
        .corner_radius(round(CARD_RADIUS))
        .inner_margin(Margin::symmetric(20, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui)
        })
        .inner
}

/// A switch for a yes/no setting.
pub fn toggle(ui: &mut Ui, colors: &Colors, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(44.0, 24.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let position = ui.ctx().animate_bool_responsive(response.id, *on);
    let off = if response.hovered() { colors.control_hovered() } else { mix(colors.surface, colors.text, 0.12) };
    ui.painter().rect_filled(rect, round(12), mix(off, colors.accent, position));
    let knob = egui::pos2(rect.left() + 12.0 + position * (rect.width() - 24.0), rect.center().y);
    let knob_color = mix(colors.muted, colors.background, position);
    ui.painter().circle_filled(knob, 8.5, knob_color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A thin line between two rows of a card.
pub fn divider(ui: &mut Ui) {
    ui.add(egui::Separator::default().spacing(12.0));
}

/// A rounded note in `color`, for the file's problems and the like.
pub fn banner(ui: &mut Ui, color: Color32, text: &str) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.14))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.5)))
        .corner_radius(round(12))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(color));
        });
}

/// A small tag (a key, say) with a rounded background.
pub fn chip<R>(ui: &mut Ui, colors: &Colors, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(colors.control())
        .corner_radius(round(8))
        .inner_margin(Margin { left: 10, right: 4, top: 2, bottom: 2 })
        .show(ui, add)
        .inner
}

/// An entry of the sidebar: a rounded bar, lit while it is the page shown.
pub fn nav_item(ui: &mut Ui, colors: &Colors, text: &str, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());
    if selected || response.hovered() {
        let fill = if selected { colors.accent.gamma_multiply(0.2) } else { colors.control() };
        ui.painter().rect_filled(rect, round(12), fill);
    }
    let color = if selected { colors.accent } else { colors.text };
    let font = FontId::new(15.0, FontFamily::Proportional);
    ui.painter().text(egui::pos2(rect.left() + 14.0, rect.center().y), egui::Align2::LEFT_CENTER, text, font, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_colors_replace_the_configured_ones() {
        let config = "[appearance]\nbackground = \"#101010\"\ntext = \"#eeeeee\"\n";
        let plain = Colors::load(config, None);
        assert_eq!(plain.background, Color32::from_rgb(0x10, 0x10, 0x10));
        assert_eq!(plain.text, Color32::from_rgb(0xee, 0xee, 0xee));
        let json = r##"{"version":1,"colors":{"primary":{"default":{"hex":"#112233"}},"surface":{"default":{"hex":"#202020"}}}}"##;
        let themed = Colors::load(config, Some(json));
        assert_eq!(themed.accent, Color32::from_rgb(0x11, 0x22, 0x33));
        assert_eq!(themed.background, Color32::from_rgb(0x20, 0x20, 0x20));
        assert_eq!(themed.text, plain.text, "roles the file lacks keep the configured color");
    }

    #[test]
    fn a_broken_theme_file_or_config_leaves_the_defaults() {
        let defaults = Colors::load("", None);
        assert_eq!(Colors::load("not = [toml", Some("not json")), defaults);
        assert_eq!(Colors::load("", Some(r#"{"version":2,"colors":{}}"#)), defaults);
    }

    #[test]
    fn dark_and_light_backgrounds_are_told_apart() {
        let dark = Colors::load("", None);
        assert!(dark.dark(), "the default palette is dark");
        let light = Colors { background: Color32::from_rgb(0xf0, 0xf0, 0xf0), ..dark };
        assert!(!light.dark());
    }
}
