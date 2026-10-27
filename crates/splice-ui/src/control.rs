//! The command center's content (`CommandCenter.qml`): tiles that switch Bluetooth and the night light, the local
//! IPv4 addresses (a click copies one), the Bluetooth devices to connect, disconnect or pair, with a search for
//! new ones, and the way to the settings app.
use crate::{
    Align, Color, Measure, Node, Rect, Scene, TextStyle, Theme,
    bar::{BUTTON_HEIGHT, button_shape},
    i18n::Language,
    panel::{Content, PanelTarget},
    theme::{CARD_RADIUS, ROW_RADIUS},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceBusy {
    Pairing,
    Connecting,
    Disconnecting,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceView {
    pub name: String,
    pub paired: bool,
    pub connected: bool,
    pub battery: Option<u8>,
    pub busy: Option<DeviceBusy>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdapterView {
    pub powered: bool,
    pub discovering: bool,
    /// As the command center lists them (see `splice_shell_protocol::Bluetooth::listed`).
    pub devices: Vec<DeviceView>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControlModel {
    pub adapter: Option<AdapterView>,
    pub night_light: bool,
    /// Device and address.
    pub addresses: Vec<(String, String)>,
    /// The address just copied (it says so for a moment).
    pub copied: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlTarget {
    Bluetooth,
    NightLight,
    /// Copy the address at this index.
    Address(usize),
    Search,
    /// The row of the device at this index (it lights up under the pointer).
    DeviceRow(usize),
    /// Its button: disconnect, connect or pair.
    Device(usize),
    /// Open the settings app.
    Settings,
}

const SECTION_SPACING: f32 = 16.0;
const SPACING: f32 = 6.0;
const TEXT: TextStyle = TextStyle::new(14.0);
const TILE_HEIGHT: f32 = 68.0;
/// Rows sit inside their card, this far from its edges.
const CARD_PADDING: f32 = 6.0;
const ADDRESS_ROW: f32 = 38.0;
const DEVICE_ROW: f32 = 54.0;
const SETTINGS_ROW: f32 = 44.0;
/// Space left of a row's text.
const ROW_INSET: f32 = 12.0;

/// What a device's second line says.
fn device_status(device: &DeviceView, language: Language) -> String {
    let tr = |text| language.tr(text).to_owned();
    match device.busy {
        Some(DeviceBusy::Pairing) => tr("Koppeln …"),
        Some(DeviceBusy::Connecting) => tr("Verbinden …"),
        Some(DeviceBusy::Disconnecting) => tr("Trennen …"),
        None if device.connected => match device.battery {
            Some(battery) => language.trf("Verbunden · %1 %", &[&battery.to_string()]),
            None => tr("Verbunden"),
        },
        None if device.paired => tr("Gekoppelt"),
        None => tr("Neu"),
    }
}

fn label(content: &mut Content, rect: Rect, text: String, style: TextStyle, color: Color, align: Align) {
    content.push(Node::Label { rect, text, style, color, align });
}

fn button(content: &mut Content, theme: &Theme, rect: Rect, text: &str, enabled: bool, target: PanelTarget, hover: Option<&PanelTarget>) {
    let mut scene: Scene<()> = Scene::new(rect.w, 0.0);
    button_shape(&mut scene, theme, rect, text, false, false, false, enabled && hover == Some(&target));
    let fade = if enabled { 1.0 } else { 0.45 };
    content.nodes.extend(scene.nodes.into_iter().map(|node| node.faded(fade)));
    if enabled {
        content.target(rect, target);
    }
}

pub fn content(model: &ControlModel, hover: Option<&PanelTarget>, theme: &Theme, measure: &mut impl Measure, width: f32, language: Language) -> Content {
    let tr = |text| language.tr(text);
    let mut content = Content::default();
    let hovered = |target: ControlTarget| hover == Some(&PanelTarget::Control(target));

    // The tiles.
    let bluetooth_on = model.adapter.as_ref().is_some_and(|a| a.powered);
    let connected = model.adapter.as_ref().map_or(0, |a| a.devices.iter().filter(|d| d.connected).count());
    let bluetooth_icon = match (bluetooth_on, connected) {
        (false, _) => "󰂲",
        (true, 0) => "󰂯",
        (true, _) => "󰂱",
    };
    let bluetooth_detail = match &model.adapter {
        None => tr("Kein Adapter").to_owned(),
        Some(a) if !a.powered => tr("Aus").to_owned(),
        Some(_) if connected > 0 => language.trf("%1 verbunden", &[&connected.to_string()]),
        Some(_) => tr("An").to_owned(),
    };
    let night = if model.night_light { tr("An") } else { tr("Aus") };
    let tiles = [
        (bluetooth_icon, "Bluetooth", bluetooth_detail, bluetooth_on, model.adapter.is_some(), ControlTarget::Bluetooth),
        ("󰖔", tr("Nachtlicht"), night.to_owned(), model.night_light, true, ControlTarget::NightLight),
    ];
    let tile_width = (width - 10.0) / 2.0;
    for (index, (icon, title, detail, active, available, target)) in tiles.into_iter().enumerate() {
        let rect = Rect::new(index as f32 * (tile_width + 10.0), 0.0, tile_width, TILE_HEIGHT);
        let fill = if active { theme.accent } else if hovered(target) && available { theme.surface } else { theme.glass_surface() };
        let mut nodes = vec![Node::Rect { rect, radius: CARD_RADIUS, fill, border: None }];
        let (icon_style, title_style, detail_style) = (TextStyle::new(24.0).icons(true), TextStyle::new(14.0).bold(true), TextStyle::new(12.0));
        let (icon_h, title_h, detail_h) = (measure.line_height(icon_style), measure.line_height(title_style), measure.line_height(detail_style));
        let column_h = title_h + 2.0 + detail_h;
        let row_h = icon_h.max(column_h);
        let top = rect.y + (rect.h - row_h) / 2.0;
        let icon_w = measure.width(icon, icon_style).ceil();
        let on = |base: Color| if active { theme.background } else { base };
        let icon_rect = Rect::new(rect.x + 16.0, top + (row_h - icon_h) / 2.0, icon_w, icon_h);
        nodes.push(Node::Label { rect: icon_rect, text: icon.into(), style: icon_style, color: on(theme.accent), align: Align::Left });
        let left = icon_rect.right() + 12.0;
        let column_top = top + (row_h - column_h) / 2.0;
        let text_w = (rect.right() - left - 8.0).max(0.0);
        nodes.push(Node::Label { rect: Rect::new(left, column_top, text_w, title_h), text: title.into(), style: title_style, color: on(theme.text), align: Align::Left });
        let detail_rect = Rect::new(left, column_top + title_h + 2.0, text_w, detail_h);
        nodes.push(Node::Label { rect: detail_rect, text: detail, style: detail_style, color: if active { theme.background.faded(0.75) } else { theme.muted }, align: Align::Left });
        let fade = if available { 1.0 } else { 0.45 };
        content.nodes.extend(nodes.into_iter().map(|node| node.faded(fade)));
        if available {
            content.target(rect, PanelTarget::Control(target));
        }
    }
    let mut y = TILE_HEIGHT + SECTION_SPACING;

    // The network: one card, a row per address.
    y += content.heading(theme, measure, y, width, tr("Netzwerk")) + SPACING;
    if model.addresses.is_empty() {
        let height = measure.line_height(TEXT);
        label(&mut content, Rect::new(0.0, y, width, height), tr("Keine Verbindung").into(), TEXT, theme.muted, Align::Left);
        y += height;
    } else {
        let card = Rect::new(0.0, y, width, 2.0 * CARD_PADDING + model.addresses.len() as f32 * ADDRESS_ROW);
        content.card(card, theme.glass_surface());
        for (index, (device, address)) in model.addresses.iter().enumerate() {
            let rect = Rect::new(CARD_PADDING, y + CARD_PADDING + index as f32 * ADDRESS_ROW, width - 2.0 * CARD_PADDING, ADDRESS_ROW);
            let target = ControlTarget::Address(index);
            if hovered(target) {
                content.push(Node::Rect { rect, radius: ROW_RADIUS, fill: theme.surface, border: None });
            }
            label(&mut content, Rect::new(rect.x + ROW_INSET, rect.y, rect.w / 2.0, rect.h), device.clone(), TEXT, theme.muted, Align::Left);
            let shown = if model.copied.as_ref() == Some(address) { tr("Kopiert").to_owned() } else { address.clone() };
            let shown_w = measure.width(&shown, TEXT).ceil();
            let color = if model.copied.as_ref() == Some(address) { theme.accent } else { theme.text };
            label(&mut content, Rect::new(rect.right() - ROW_INSET - shown_w, rect.y, shown_w, rect.h), shown, TEXT, color, Align::Left);
            content.target(rect, PanelTarget::Control(target));
        }
        y = card.bottom();
    }
    y += SECTION_SPACING;

    // The devices: the same, with the search beside the title.
    if let Some(adapter) = &model.adapter {
        let heading_top = y + (BUTTON_HEIGHT - measure.line_height(crate::panel::HEADING)) / 2.0;
        let heading_h = content.heading(theme, measure, heading_top, width, tr("Geräte"));
        if adapter.powered {
            let text = if adapter.discovering { format!("󰓛 {}", tr("Suche beenden")) } else { format!("󰍉 {}", tr("Suchen")) };
            let search_w = measure.width(&text, TEXT).ceil() + 16.0;
            button(&mut content, theme, Rect::new(width - search_w, y, search_w, BUTTON_HEIGHT), &text, true, PanelTarget::Control(ControlTarget::Search), hover);
        }
        y += BUTTON_HEIGHT.max(heading_h) + SPACING;
        let note = if !adapter.powered {
            Some(tr("Bluetooth ist aus"))
        } else if adapter.devices.is_empty() {
            Some(if adapter.discovering { tr("Suche läuft …") } else { tr("Keine gekoppelten Geräte") })
        } else {
            None
        };
        if let Some(note) = note {
            let height = measure.line_height(TEXT);
            label(&mut content, Rect::new(0.0, y, width, height), note.into(), TEXT, theme.muted, Align::Left);
            y += height + SECTION_SPACING;
        }
        let listed = if adapter.powered { adapter.devices.as_slice() } else { &[] };
        if !listed.is_empty() {
            let card = Rect::new(0.0, y, width, 2.0 * CARD_PADDING + listed.len() as f32 * DEVICE_ROW);
            content.card(card, theme.glass_surface());
            for (index, device) in listed.iter().enumerate() {
                let rect = Rect::new(CARD_PADDING, y + CARD_PADDING + index as f32 * DEVICE_ROW, width - 2.0 * CARD_PADDING, DEVICE_ROW);
                let row = ControlTarget::DeviceRow(index);
                let action = ControlTarget::Device(index);
                let fill = if hovered(row) || hovered(action) {
                    Some(theme.surface)
                } else if device.connected {
                    Some(theme.accent.with_alpha(0.16))
                } else {
                    None
                };
                if let Some(fill) = fill {
                    content.push(Node::Rect { rect, radius: ROW_RADIUS, fill, border: None });
                }
                content.target(rect, PanelTarget::Control(row));
                let text = if device.connected { tr("Trennen") } else if device.paired { tr("Verbinden") } else { tr("Koppeln") };
                let action_w = measure.width(text, TEXT).ceil() + 16.0;
                let column_w = (rect.w - action_w - 2.0 * ROW_INSET).max(0.0);
                let (name_style, status_style) = (TEXT.bold(device.connected), TextStyle::new(12.0));
                let (name_h, status_h) = (measure.line_height(name_style), measure.line_height(status_style));
                let top = rect.y + (rect.h - name_h - 2.0 - status_h) / 2.0;
                label(&mut content, Rect::new(rect.x + ROW_INSET, top, column_w, name_h), device.name.clone(), name_style, theme.text, Align::Left);
                let status = device_status(device, language);
                label(&mut content, Rect::new(rect.x + ROW_INSET, top + name_h + 2.0, column_w, status_h), status, status_style, theme.muted, Align::Left);
                let action_rect = Rect::new(rect.right() - ROW_INSET + 4.0 - action_w, rect.y + (rect.h - BUTTON_HEIGHT) / 2.0, action_w, BUTTON_HEIGHT);
                button(&mut content, theme, action_rect, text, device.busy != Some(DeviceBusy::Pairing), PanelTarget::Control(action), hover);
            }
            y = card.bottom() + SECTION_SPACING;
        }
    }

    // The way to the settings app.
    let card = Rect::new(0.0, y, width, 2.0 * CARD_PADDING + SETTINGS_ROW);
    content.card(card, theme.glass_surface());
    let rect = card.inset(CARD_PADDING);
    if hovered(ControlTarget::Settings) {
        content.push(Node::Rect { rect, radius: ROW_RADIUS, fill: theme.surface, border: None });
    }
    let icon_style = TextStyle::new(18.0).icons(true);
    let icon = "󰒓";
    let icon_w = measure.width(icon, icon_style).ceil();
    label(&mut content, Rect::new(rect.x + ROW_INSET, rect.y, icon_w, rect.h), icon.into(), icon_style, theme.accent, Align::Left);
    let text_x = rect.x + ROW_INSET + icon_w + 12.0;
    label(&mut content, Rect::new(text_x, rect.y, (rect.right() - text_x - 36.0).max(0.0), rect.h), tr("Einstellungen").into(), TEXT, theme.text, Align::Left);
    label(&mut content, Rect::new(rect.right() - ROW_INSET - 18.0, rect.y, 18.0, rect.h), "󰅂".into(), icon_style, theme.muted, Align::Center);
    content.target(rect, PanelTarget::Control(ControlTarget::Settings));
    content.height = card.bottom();
    content
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{scene::tests::Mono, theme::Palette};

    fn theme() -> Theme {
        let c = |v| Color::rgba(v, v, v, 255);
        Theme::new(Palette { background: c(10), surface: c(20), text: c(200), muted: c(150), accent: c(90), border: c(60) }, None)
    }

    fn texts(content: &Content) -> Vec<String> {
        content.nodes.iter().filter_map(|n| match n { Node::Label { text, .. } => Some(text.clone()), _ => None }).collect()
    }

    fn has(content: &Content, target: ControlTarget) -> bool {
        content.targets.iter().any(|(_, t)| *t == PanelTarget::Control(target))
    }

    #[test]
    fn without_bluetooth_the_tile_is_disabled() {
        let content = content(&ControlModel::default(), None, &theme(), &mut Mono, 360.0, Language::German);
        let texts = texts(&content);
        assert!(texts.contains(&"Kein Adapter".into()) && texts.contains(&"Keine Verbindung".into()), "{texts:?}");
        assert!(!has(&content, ControlTarget::Bluetooth) && has(&content, ControlTarget::NightLight));
        assert!(!texts.contains(&"Geräte".into()));
        assert_eq!(content.targets[0].0, Rect::new(175.0 + 10.0, 0.0, 175.0, TILE_HEIGHT));
    }

    #[test]
    fn devices_say_what_they_do_and_pairing_waits() {
        let device = |name: &str, connected, paired, busy| DeviceView { name: name.into(), paired, connected, battery: Some(80), busy };
        let model = ControlModel {
            adapter: Some(AdapterView {
                powered: true,
                discovering: true,
                devices: vec![device("Kopfhörer", true, true, None), device("Lautsprecher", false, false, Some(DeviceBusy::Pairing))],
            }),
            night_light: true,
            addresses: vec![("eno1".into(), "192.168.1.23".into())],
            copied: Some("192.168.1.23".into()),
        };
        let content = content(&model, None, &theme(), &mut Mono, 360.0, Language::German);
        let texts = texts(&content);
        for text in ["󰂱", "1 verbunden", "Kopiert", "󰓛 Suche beenden", "Verbunden · 80 %", "Trennen", "Koppeln …", "Koppeln"] {
            assert!(texts.contains(&text.to_owned()), "{text} missing in {texts:?}");
        }
        assert!(has(&content, ControlTarget::Device(0)) && !has(&content, ControlTarget::Device(1)), "pairing disables the button");
        assert!(has(&content, ControlTarget::Address(0)) && has(&content, ControlTarget::Search));
    }

    #[test]
    fn the_way_to_the_settings_is_the_last_row_with_or_without_bluetooth() {
        let with = ControlModel { adapter: Some(AdapterView { powered: true, discovering: false, devices: Vec::new() }), ..ControlModel::default() };
        for model in [ControlModel::default(), with] {
            let content = content(&model, None, &theme(), &mut Mono, 360.0, Language::English);
            assert!(texts(&content).contains(&"Settings".into()));
            let (rect, target) = content.targets.last().unwrap();
            assert_eq!(*target, PanelTarget::Control(ControlTarget::Settings));
            assert!(rect.bottom() < content.height && rect.w > 300.0, "{rect:?} in {}", content.height);
        }
    }

    #[test]
    fn switched_off_bluetooth_hides_the_devices_and_the_search() {
        let model = ControlModel {
            adapter: Some(AdapterView { powered: false, discovering: false, devices: vec![DeviceView { name: "x".into(), paired: true, connected: false, battery: None, busy: None }] }),
            ..ControlModel::default()
        };
        let content = content(&model, None, &theme(), &mut Mono, 360.0, Language::English);
        let texts = texts(&content);
        assert!(texts.contains(&"Bluetooth is off".into()) && texts.contains(&"Off".into()), "{texts:?}");
        assert!(!has(&content, ControlTarget::Search) && !has(&content, ControlTarget::Device(0)));
    }
}
