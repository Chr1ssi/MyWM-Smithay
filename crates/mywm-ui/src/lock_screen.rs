//! The lock screen (as mywm-shell's `lock.qml` with `LoginScreen.qml`).
use crate::i18n::Language;

/// What the lock screen says after a password was not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockMessage {
    WrongPassword,
    TooManyTries,
}

impl LockMessage {
    pub fn text(self, language: Language) -> &'static str {
        match self {
            Self::WrongPassword => language.tr("Falsches Passwort"),
            Self::TooManyTries => language.tr("Zu viele Versuche"),
        }
    }
}
