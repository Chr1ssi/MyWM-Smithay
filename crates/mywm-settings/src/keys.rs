//! Turning a pressed key into the config syntax (`Super+Shift+h`).
use eframe::egui::Key;

/// The name `mywm-config` knows for an egui key; `None` for keys it does not support.
pub fn key_name(key: Key) -> Option<String> {
    let name = match key {
        Key::Enter => "Return",
        Key::Space => "space",
        Key::Tab => "Tab",
        Key::Escape => "Escape",
        Key::Backspace => "BackSpace",
        Key::Delete => "Delete",
        Key::Insert => "Insert",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::Minus => "minus",
        Key::Equals => "equal",
        Key::Plus => "plus",
        Key::Comma => "comma",
        Key::Period => "period",
        Key::Semicolon => "semicolon",
        Key::Slash => "slash",
        Key::Backslash => "backslash",
        Key::OpenBracket => "bracketleft",
        Key::CloseBracket => "bracketright",
        Key::Quote => "apostrophe",
        Key::Backtick => "grave",
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        other => {
            let name = other.name();
            // Letters and digits: "A" to "Z" and "0" to "9".
            if name.len() == 1 && name.as_bytes()[0].is_ascii_alphanumeric() {
                return Some(name.to_ascii_lowercase());
            }
            return None;
        }
    };
    Some(name.to_owned())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub logo: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// `Super+Ctrl+Alt+Shift+key`, in this order.
pub fn binding(mods: Mods, key: Key) -> Option<String> {
    let name = key_name(key)?;
    let mut parts = Vec::new();
    for (on, word) in [(mods.logo, "Super"), (mods.ctrl, "Ctrl"), (mods.alt, "Alt"), (mods.shift, "Shift")] {
        if on {
            parts.push(word.to_owned());
        }
    }
    parts.push(name);
    Some(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_is_understood_by_the_compositor() {
        for key in Key::ALL {
            if let Some(name) = key_name(*key) {
                mywm_config::parse_key(&name).unwrap_or_else(|e| panic!("{key:?} -> {name}: {e}"));
            }
        }
    }

    #[test]
    fn bindings_are_written_with_ordered_modifiers() {
        let mods = Mods { logo: true, shift: true, ..Default::default() };
        assert_eq!(binding(mods, Key::H).as_deref(), Some("Super+Shift+h"));
        assert_eq!(binding(Mods::default(), Key::F5).as_deref(), Some("F5"));
        assert_eq!(binding(Mods { ctrl: true, alt: true, ..Default::default() }, Key::ArrowLeft).as_deref(), Some("Ctrl+Alt+Left"));
        assert_eq!(binding(Mods::default(), Key::Num7).as_deref(), Some("7"));
    }
}
