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
        "grave" => 0x60,
        "minus" => 0x2d,
        "equal" => 0x3d,
        "plus" => 0x2b,
        "comma" => 0x2c,
        "period" => 0x2e,
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
    fn key_aliases_and_modifiers() {
        assert_eq!(parse_key("Super+Return").unwrap(), parse_key("super+enter").unwrap());
        let (_, modifiers) = parse_key("Super+Shift+2").unwrap();
        assert_eq!(modifiers, Modifiers { logo: true, shift: true, ..Default::default() });
        assert_eq!(parse_key("Ctrl+Alt+h").unwrap().0, 0x68);
        assert!(parse_key("Super+Super+m").is_err());
        assert!(parse_key("Super+Bogus").is_err());
    }
}
