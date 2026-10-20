//! The shell's texts in the system language (as mywm-shell's `I18n.qml`). The German text in the code is the
//! key; English stands in for every language without a table of its own. The language is the first of the
//! system's UI languages (`LANGUAGE`, then `LC_ALL`, `LC_MESSAGES`, `LANG`) that the shell knows; dates and
//! times follow `LC_TIME` instead (see `clock`).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    German,
    English,
}

impl Language {
    pub fn from_env() -> Self {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    /// The language for the locale variables that `var` returns.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        let var = |name: &str| var(name).filter(|value| !value.is_empty());
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].into_iter().find_map(var);
        let preferences = var("LANGUAGE").map(|list| list.split(':').map(str::to_owned).collect::<Vec<_>>());
        preferences
            .unwrap_or_default()
            .into_iter()
            .chain(locale)
            .find_map(|name| match language_code(&name) {
                "de" => Some(Language::German),
                "en" => Some(Language::English),
                _ => None,
            })
            .unwrap_or(Language::English)
    }

    /// `text` (German) in this language.
    pub fn tr(self, text: &'static str) -> &'static str {
        match self {
            Language::German => text,
            Language::English => ENGLISH.iter().find(|(german, _)| *german == text).map_or(text, |(_, english)| english),
        }
    }

    /// `tr` with `%1`, `%2`, … replaced by `args`.
    pub fn trf(self, text: &'static str, args: &[&str]) -> String {
        args.iter()
            .enumerate()
            .fold(self.tr(text).to_owned(), |result, (index, value)| result.replace(&format!("%{}", index + 1), value))
    }
}

/// `de` of `de_DE.UTF-8`, `de-AT` or `de@euro`.
pub fn language_code(locale: &str) -> &str {
    locale.split(['_', '-', '.', '@']).next().unwrap_or("")
}

const ENGLISH: &[(&str, &str)] = &[
    ("Abbrechen", "Cancel"),
    ("Abmelden", "Log out"),
    ("Aktion fehlgeschlagen (%1)", "Action failed (%1)"),
    ("Aufnahme", "Recording"),
    ("Ausschalten", "Shut down"),
    ("Automatisch in %1 Sekunden", "Automatically in %1 seconds"),
    (
        "Bis zur Korrektur gilt die zuletzt geladene Konfiguration, beim Start die Standardwerte.",
        "Until it is fixed, the last loaded configuration applies, or the defaults at startup.",
    ),
    ("Esc zum Schließen", "Esc to close"),
    ("Farbpipette", "Color picker"),
    (
        "Fenster anklicken · Kachel daneben oder Enter: ganzer Monitor · D: dynamisch, mit den Cast-Tasten umschaltbar · Esc: abbrechen",
        "Click a window · tile beside it or Enter: whole monitor · D: dynamic, switchable with the cast keys · Esc: cancel",
    ),
    ("Klick kopiert die Farbe · Esc: abbrechen", "Click copies the color · Esc: cancel"),
    ("Konfiguration fehlerhaft", "Configuration error"),
    ("Neu starten", "Restart"),
    ("Neustart", "Restart"),
    ("Sitzung und System", "Session and system"),
    ("Sperren", "Lock"),
    ("Stumm", "Muted"),
    ("Was soll geteilt werden?", "What should be shared?"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn language(vars: &[(&str, &str)]) -> Language {
        Language::from_vars(|name| vars.iter().find(|(n, _)| *n == name).map(|(_, v)| (*v).to_owned()))
    }

    #[test]
    fn the_language_follows_the_locale_variables() {
        assert_eq!(language(&[("LANG", "de_DE.UTF-8")]), Language::German);
        // Dates in German do not make the texts German: LC_TIME is not the language.
        assert_eq!(language(&[("LANG", "en_US.UTF-8"), ("LC_TIME", "de_DE.UTF-8")]), Language::English);
        assert_eq!(language(&[("LANG", "en_US.UTF-8"), ("LC_MESSAGES", "de_DE.UTF-8")]), Language::German);
        assert_eq!(language(&[("LANG", "en_US.UTF-8"), ("LC_ALL", "de_AT.UTF-8")]), Language::German);
        // LANGUAGE lists the preferences; the first one the shell knows wins.
        assert_eq!(language(&[("LANG", "en_US.UTF-8"), ("LANGUAGE", "fr:de:en")]), Language::German);
        assert_eq!(language(&[("LANG", "fr_FR.UTF-8")]), Language::English);
        assert_eq!(language(&[("LANG", "C")]), Language::English);
        assert_eq!(language(&[]), Language::English);
    }

    #[test]
    fn texts_are_translated_with_their_arguments() {
        assert_eq!(Language::English.trf("Automatisch in %1 Sekunden", &["5"]), "Automatically in 5 seconds");
        assert_eq!(Language::German.trf("Automatisch in %1 Sekunden", &["5"]), "Automatisch in 5 Sekunden");
        assert_eq!(Language::English.tr("Unbekannt"), "Unbekannt");
    }

    /// Every text the shell asks for has its English entry, and the table has nothing nobody asks for.
    #[test]
    fn every_text_has_english() {
        let sources = [include_str!("bar.rs"), include_str!("power.rs"), include_str!("notices.rs")];
        let mut used = std::collections::BTreeSet::new();
        for source in sources {
            for call in ["tr(\"", "trf(\""] {
                for part in source.split(call).skip(1) {
                    used.insert(part.split('"').next().unwrap_or_default().to_owned());
                }
            }
        }
        let known: std::collections::BTreeSet<_> = ENGLISH.iter().map(|(german, _)| (*german).to_owned()).collect();
        let missing: Vec<_> = used.difference(&known).collect();
        let unused: Vec<_> = known.difference(&used).collect();
        assert!(missing.is_empty(), "without English text: {missing:?}");
        assert!(unused.is_empty(), "English text nobody uses: {unused:?}");
    }
}
