//! Key binding syntax: `Super+Shift+h`.

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
    let name = parts.last().unwrap().to_ascii_lowercase();
    let symbol = match name.as_str() {
        "return" | "enter" => 0xff0d,
        "left" => 0xff51,
        "up" => 0xff52,
        "right" => 0xff53,
        "down" => 0xff54,
        "space" => 0x20,
        "tab" => 0xff09,
        "escape" | "esc" => 0xff1b,
        "print" => 0xff61,
        "grave" => 0x60,
        "minus" => 0x2d,
        "equal" => 0x3d,
        "plus" => 0x2b,
        "comma" => 0x2c,
        "period" => 0x2e,
        "semicolon" => 0x3b,
        "slash" => 0x2f,
        "backslash" => 0x5c,
        "bracketleft" => 0x5b,
        "bracketright" => 0x5d,
        "apostrophe" => 0x27,
        "backspace" => 0xff08,
        "delete" => 0xffff,
        "insert" => 0xff63,
        "home" => 0xff50,
        "end" => 0xff57,
        "pageup" => 0xff55,
        "pagedown" => 0xff56,
        "xf86audiolowervolume" => 0x1008ff11,
        "xf86audiomute" => 0x1008ff12,
        "xf86audioraisevolume" => 0x1008ff13,
        "xf86audioplay" => 0x1008ff14,
        "xf86audiostop" => 0x1008ff15,
        "xf86audioprev" => 0x1008ff16,
        "xf86audionext" => 0x1008ff17,
        "xf86audiopause" => 0x1008ff31,
        "xf86audiomicmute" => 0x1008ffb2,
        function if function.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()).is_some_and(|n| (1..=12).contains(&n)) => {
            0xffbd + function[1..].parse::<u32>().unwrap()
        }
        value if value.len() == 1 && value.as_bytes()[0].is_ascii_alphanumeric() => {
            value.as_bytes()[0] as u32
        }
        _ => return Err(format!("Unsupported key in keybinding: {key}").into()),
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
        assert!(parse_key("F13").is_err());
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
}
