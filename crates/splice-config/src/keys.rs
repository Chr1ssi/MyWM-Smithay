//! Key binding syntax: `Super+Shift+h`, the key by its xkb keysym name.
use xkbcommon::xkb;

/// Modifier set of a binding or of the current keyboard state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub logo: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Returns the (lowercase) X keysym and the modifiers of a binding string.
pub fn parse_key(key: &str) -> Result<(u32, Modifiers)> {
    let parts: Vec<_> = key.split('+').map(str::trim).collect();
    let mut modifiers = Modifiers::default();
    for modifier in &parts[..parts.len() - 1] {
        let flag = match modifier.to_ascii_lowercase().as_str() {
            "super" => &mut modifiers.logo,
            "shift" => &mut modifiers.shift,
            "ctrl" | "control" => &mut modifiers.ctrl,
            "alt" => &mut modifiers.alt,
            _ => return Err(format!("Unknown modifier in keybinding: {key}").into()),
        };
        if *flag {
            return Err(format!("Repeated modifier in keybinding: {key}").into());
        }
        *flag = true;
    }
    let name = parts.last().unwrap();
    let symbol = match name.to_ascii_lowercase().as_str() {
        // Short forms that are not keysym names.
        "enter" => xkb::keysyms::KEY_Return,
        "esc" => xkb::keysyms::KEY_Escape,
        "pageup" => xkb::keysyms::KEY_Page_Up,
        "pagedown" => xkb::keysyms::KEY_Page_Down,
        // Any keysym of the layout by its xkb name (`dead_circumflex`, `ssharp`, `udiaeresis`, `XF86AudioMute`).
        // Bindings match the unshifted symbol of a key, so letters are lowercase.
        _ => match xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE).raw() {
            xkb::keysyms::KEY_NoSymbol => return Err(format!("Unsupported key in keybinding: {key}").into()),
            symbol => symbol,
        },
    };
    Ok((symbol, modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_and_navigation_keys() {
        assert_eq!(parse_key("Super+F1").unwrap().0, 0xffbe);
        assert_eq!(parse_key("F12").unwrap().0, 0xffc9);
        assert_eq!(parse_key("F13").unwrap().0, 0xffca);
        assert_eq!(parse_key("Super+PageUp").unwrap().0, 0xff55);
        assert_eq!(parse_key("Shift+Delete").unwrap().0, 0xffff);
    }

    #[test]
    fn media_keys() {
        assert_eq!(parse_key("XF86AudioRaiseVolume").unwrap(), (0x1008ff13, Modifiers::default()));
        assert_eq!(parse_key("Shift+XF86AudioMute").unwrap().0, 0x1008ff12);
        assert_eq!(parse_key("XF86AudioMicMute").unwrap().0, 0x1008ffb2);
    }

    #[test]
    fn key_aliases_and_modifiers() {
        assert_eq!(parse_key("Super+Return").unwrap(), parse_key("super+enter").unwrap());
        let (_, modifiers) = parse_key("Super+Shift+2").unwrap();
        assert_eq!(modifiers, Modifiers { logo: true, shift: true, ..Default::default() });
        assert_eq!(parse_key("Ctrl+Alt+h").unwrap().0, 0x68);
        assert!(parse_key("Super+Super+m").is_err());
        assert!(parse_key("Super+Bogus").is_err());
    }

    #[test]
    fn keys_of_other_layouts() {
        // German: the keys right of 0 and left of 1, the umlauts, # and <.
        assert_eq!(parse_key("Super+ssharp").unwrap().0, 0xdf);
        assert_eq!(parse_key("Super+dead_acute").unwrap().0, 0xfe51);
        assert_eq!(parse_key("Super+dead_circumflex").unwrap().0, 0xfe52);
        assert_eq!(parse_key("Super+Udiaeresis").unwrap().0, parse_key("Super+udiaeresis").unwrap().0);
        assert_eq!(parse_key("Super+udiaeresis").unwrap().0, 0xfc);
        assert_eq!(parse_key("Super+numbersign").unwrap().0, 0x23);
        assert_eq!(parse_key("Super+less").unwrap().0, 0x3c);
        // Letters by either case: the binding matches the unshifted one.
        assert_eq!(parse_key("Super+H").unwrap().0, 0x68);
    }
}
