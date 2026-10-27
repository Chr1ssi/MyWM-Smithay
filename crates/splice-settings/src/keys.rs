//! Turning a pressed key into the config syntax (`Super+Shift+h`): the keysym that the configured keyboard layout
//! puts on that key without modifiers, which is what the compositor matches. On a German layout the key right of
//! `0` is `ssharp`, the one left of `1` `dead_circumflex`, whatever egui calls them.
use eframe::egui::Key;
use xkbcommon::xkb;

/// The keyboard layout of the configuration being edited (`[keyboard]`).
pub struct Layout {
    keymap: Option<xkb::Keymap>,
}

impl Layout {
    pub fn new(layout: &str, variant: &str, options: &str) -> Self {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let options = (!options.is_empty()).then(|| options.to_owned());
        let keymap = xkb::Keymap::new_from_names(&context, "", "", layout, variant, options, xkb::KEYMAP_COMPILE_NO_FLAGS);
        Self { keymap }
    }

    /// The keysym name of a physical key (egui names it by its place on a US keyboard) in this layout; `None` for
    /// keys the compositor cannot bind or a layout xkb does not know.
    pub fn key_name(&self, key: Key) -> Option<String> {
        let keycode = xkb::Keycode::new(evdev_code(key)? + 8);
        let symbol = *self.keymap.as_ref()?.key_get_syms_by_level(keycode, 0, 0).first()?;
        let name = xkb::keysym_get_name(symbol);
        splice_config::parse_key(&name).is_ok().then_some(name)
    }

    /// `Super+Ctrl+Alt+Shift+key`, in this order.
    pub fn binding(&self, mods: Mods, key: Key) -> Option<String> {
        let name = self.key_name(key)?;
        let mut parts = Vec::new();
        for (on, word) in [(mods.logo, "Super"), (mods.ctrl, "Ctrl"), (mods.alt, "Alt"), (mods.shift, "Shift")] {
            if on {
                parts.push(word.to_owned());
            }
        }
        parts.push(name);
        Some(parts.join("+"))
    }
}

/// The Linux input event code of a physical key (`input-event-codes.h`).
fn evdev_code(key: Key) -> Option<u32> {
    Some(match key {
        Key::Escape => 1,
        Key::Num1 => 2,
        Key::Num2 => 3,
        Key::Num3 => 4,
        Key::Num4 => 5,
        Key::Num5 => 6,
        Key::Num6 => 7,
        Key::Num7 => 8,
        Key::Num8 => 9,
        Key::Num9 => 10,
        Key::Num0 => 11,
        Key::Minus => 12,
        Key::Equals => 13,
        Key::Backspace => 14,
        Key::Tab => 15,
        Key::Q => 16,
        Key::W => 17,
        Key::E => 18,
        Key::R => 19,
        Key::T => 20,
        Key::Y => 21,
        Key::U => 22,
        Key::I => 23,
        Key::O => 24,
        Key::P => 25,
        Key::OpenBracket => 26,
        Key::CloseBracket => 27,
        Key::Enter => 28,
        Key::A => 30,
        Key::S => 31,
        Key::D => 32,
        Key::F => 33,
        Key::G => 34,
        Key::H => 35,
        Key::J => 36,
        Key::K => 37,
        Key::L => 38,
        Key::Semicolon => 39,
        Key::Quote => 40,
        Key::Backtick => 41,
        Key::Backslash => 43,
        Key::Z => 44,
        Key::X => 45,
        Key::C => 46,
        Key::V => 47,
        Key::B => 48,
        Key::N => 49,
        Key::M => 50,
        Key::Comma => 51,
        Key::Period => 52,
        Key::Slash => 53,
        Key::Space => 57,
        Key::F1 => 59,
        Key::F2 => 60,
        Key::F3 => 61,
        Key::F4 => 62,
        Key::F5 => 63,
        Key::F6 => 64,
        Key::F7 => 65,
        Key::F8 => 66,
        Key::F9 => 67,
        Key::F10 => 68,
        Key::F11 => 87,
        Key::F12 => 88,
        Key::Home => 102,
        Key::ArrowUp => 103,
        Key::PageUp => 104,
        Key::ArrowLeft => 105,
        Key::ArrowRight => 106,
        Key::End => 107,
        Key::ArrowDown => 108,
        Key::PageDown => 109,
        Key::Insert => 110,
        Key::Delete => 111,
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub logo: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_is_understood_by_the_compositor() {
        for layout in [Layout::new("us", "", ""), Layout::new("de", "", ""), Layout::new("de", "nodeadkeys", "")] {
            for key in Key::ALL {
                if let Some(name) = layout.key_name(*key) {
                    splice_config::parse_key(&name).unwrap_or_else(|e| panic!("{key:?} -> {name}: {e}"));
                }
            }
        }
    }

    #[test]
    fn keys_are_named_by_the_layout() {
        let us = Layout::new("us", "", "");
        let de = Layout::new("de", "", "");
        // Where a US keyboard has `=`, `-`, `` ` ``, `[`, `'`, `\` and `y`, a German one has these.
        for (key, us_name, de_name) in [
            (Key::Equals, "equal", "dead_acute"),
            (Key::Minus, "minus", "ssharp"),
            (Key::Backtick, "grave", "dead_circumflex"),
            (Key::OpenBracket, "bracketleft", "udiaeresis"),
            (Key::Quote, "apostrophe", "adiaeresis"),
            (Key::Backslash, "backslash", "numbersign"),
            (Key::Y, "y", "z"),
        ] {
            assert_eq!(us.key_name(key).as_deref(), Some(us_name), "{key:?}");
            assert_eq!(de.key_name(key).as_deref(), Some(de_name), "{key:?}");
        }
        assert_eq!(Layout::new("de", "nodeadkeys", "").key_name(Key::Equals).as_deref(), Some("acute"));
        assert_eq!(Layout::new("no-such-layout", "", "").key_name(Key::A), None);
    }

    #[test]
    fn bindings_are_written_with_ordered_modifiers() {
        let de = Layout::new("de", "", "");
        let mods = Mods { logo: true, shift: true, ..Default::default() };
        assert_eq!(de.binding(mods, Key::H).as_deref(), Some("Super+Shift+h"));
        assert_eq!(de.binding(Mods::default(), Key::F5).as_deref(), Some("F5"));
        let ctrl_alt = Mods { ctrl: true, alt: true, ..Default::default() };
        assert_eq!(de.binding(ctrl_alt, Key::ArrowLeft).as_deref(), Some("Ctrl+Alt+Left"));
        assert_eq!(de.binding(Mods { logo: true, ..Default::default() }, Key::Backtick).as_deref(), Some("Super+dead_circumflex"));
    }
}
