//! Desktop entries as Quickshell's `DesktopEntries` reads them: the applications of the XDG `applications`
//! directories that the launcher lists. The parsing follows Quickshell's `desktopentry.cpp` closely, quirks included,
//! so an application gets the same name, command and icon as there.
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::mpsc,
};

use splice_shell_protocol::{APP_ICON_SIZE, Application};

use crate::{Update, icons};

/// Read the applications whenever asked (requests that queued up meanwhile count once), the icon each one shows
/// in the launcher in place of its own, then the pictures of icons not sent before.
pub fn run(requests: mpsc::Receiver<()>, updates: mpsc::Sender<Update>) {
    let mut sent = HashSet::new();
    while requests.recv().is_ok() {
        while requests.try_recv().is_ok() {}
        let apps: Vec<Application> = scan(&desktop_dirs(), &Locale::system())
            .into_iter()
            .map(|app| Application { icon: launcher_icon(&app), ..app })
            .collect();
        let icons: Vec<String> = apps.iter().map(|app| app.icon.clone()).filter(|icon| !sent.contains(icon)).collect();
        if updates.send(Update::Applications(apps)).is_err() {
            return;
        }
        for name in icons {
            if !sent.insert(name.clone()) {
                continue;
            }
            if let Some(image) = icons::load_icon(&name, &[], APP_ICON_SIZE)
                && updates.send(Update::AppIcon(name, image)).is_err()
            {
                return;
            }
        }
    }
}

/// A locale such as `de_DE.UTF-8@euro`, split into the parts that localized keys are matched by.
#[derive(Debug, Default, PartialEq)]
pub struct Locale {
    language: String,
    territory: String,
    modifier: String,
}

impl Locale {
    /// Splits `text` as Quickshell does: the modifier is what follows `@`, the codeset after `.` is dropped and the
    /// territory is what follows `_`.
    pub fn parse(text: &str) -> Locale {
        let territory_idx = text.find('_');
        let codeset_idx = text.find('.');
        let modifier_idx = text.find('@');
        let mut parse_end = text.len();
        let mut modifier = "";
        if let Some(idx) = modifier_idx {
            modifier = &text[idx + 1..];
            parse_end = idx;
        }
        // A codeset cuts the language even when it comes after the modifier, as in Quickshell ("de@x.y" is "de@x").
        if let Some(idx) = codeset_idx {
            parse_end = idx;
        }
        let mut territory = "";
        if let Some(idx) = territory_idx {
            // Quickshell slices with a negative length when the territory starts after the codeset or modifier; there
            // it is empty.
            if idx < parse_end {
                territory = &text[idx + 1..parse_end];
            }
            parse_end = idx;
        }
        Locale {
            language: text[..parse_end].to_owned(),
            territory: territory.to_owned(),
            modifier: modifier.to_owned(),
        }
    }

    /// Whether there is a language to match with (Quickshell's `isValid`).
    fn is_valid(&self) -> bool {
        !self.language.is_empty()
    }

    /// How well this system locale fits `other`, the locale of a localized value: 0 if it does not fit at all, else 1,
    /// plus 2 for a territory and 1 for a modifier that `other` names.
    fn match_score(&self, other: &Locale) -> u32 {
        if self.language != other.language {
            return 0;
        }
        if !other.modifier.is_empty() && self.modifier != other.modifier {
            return 0;
        }
        if !other.territory.is_empty() && self.territory != other.territory {
            return 0;
        }
        let mut score = 1;
        if !other.territory.is_empty() {
            score += 2;
        }
        if !other.modifier.is_empty() {
            score += 1;
        }
        score
    }

    /// The locale of the messages: `LC_MESSAGES`, else `LANG` (not `LC_ALL`, as in Quickshell).
    pub fn system() -> Locale {
        let text = ["LC_MESSAGES", "LANG"]
            .into_iter()
            .map(|var| std::env::var_os(var).map(|v| v.to_string_lossy().into_owned()).unwrap_or_default())
            .find(|value| !value.is_empty())
            .unwrap_or_default();
        Locale::parse(&text)
    }
}

/// `Exec` split into arguments as Quickshell's `parseExecString` does it: double or single quotes keep a word
/// together, a backslash escapes inside them, field codes such as `%U` are dropped and `%%` stays a percent sign.
pub fn parse_exec(exec: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    // Backslashes read so far. Inside quotes four of them make one, as the desktop entry spec requires for `\\`
    // written into the file.
    let mut escape = 0;
    let mut percent = false;
    for c in exec.chars() {
        if escape == 0 && c == '\\' {
            escape = 1;
        } else if in_quotes {
            if c == '\\' {
                escape += 1;
                if escape == 4 {
                    current.push('\\');
                    escape = 0;
                }
            } else if escape == 2 {
                current.push(c);
                escape = 0;
            } else if escape != 0 {
                current.push(match c {
                    's' => ' ',
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '\\' => '\\',
                    // Quickshell warns about other escapes and keeps the character.
                    _ => c,
                });
                escape = 0;
            } else if c == '"' || c == '\'' {
                in_quotes = false;
            } else {
                current.push(c);
            }
        } else if escape != 0 {
            current.push(c);
            escape = 0;
        } else if percent {
            if c == '%' {
                current.push('%');
            }
            percent = false;
        } else if c == '%' {
            percent = true;
        } else if c == '"' || c == '\'' {
            in_quotes = true;
        } else if c == ' ' {
            if !current.is_empty() {
                arguments.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        arguments.push(current);
    }
    arguments
}

/// An entry as the scan reads it: its application and the flags that keep it out of the launcher.
#[derive(Debug, PartialEq)]
pub struct Entry {
    pub app: Application,
    /// `Hidden=true`: the entry masks the file, so an earlier entry of the same id is dropped too.
    pub hidden: bool,
    /// `NoDisplay=true`: not listed, but it still replaces an earlier entry of the same id.
    pub no_display: bool,
}

/// The values of a group by key, each with the locale it came with.
type Values = HashMap<String, (Locale, String)>;

/// The `[Desktop Entry]` of the file `text` of `id`, with the localized values that fit `system` best. Other groups,
/// the actions included, are not read.
pub fn parse_entry(id: &str, text: &str, system: &Locale) -> Entry {
    let mut entry = Entry {
        app: Application { id: id.to_owned(), ..Application::default() },
        hidden: false,
        no_display: false,
    };
    let mut group = String::new();
    let mut values = Values::new();
    // Only `\n` splits lines, so a `\r` of a CRLF file stays at the end of the line. Then `[Desktop Entry]\r` is not a
    // group header, which is what Quickshell does too.
    for line in text.split('\n').filter(|line| !line.is_empty()) {
        if line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            finish_group(&mut entry, &group, &mut values);
            group = name.to_owned();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, locale) = match key.find('[') {
            // `Name[de]`: the locale is what is inside the brackets. A `[` at the very end is part of the key.
            Some(idx) if idx + 1 != key.len() => {
                let rest = &key[idx + 1..];
                let inside = rest.chars().next_back().and_then(|last| rest.strip_suffix(last)).unwrap_or_default();
                (&key[..idx], Locale::parse(inside))
            }
            _ => (key, Locale::default()),
        };
        // A value of a later line replaces the kept one only when its locale fits the system better, or when nothing
        // fitting was kept yet and it is not localized at all.
        let replace = values.get(key).is_none_or(|(old, _)| {
            let old_score = system.match_score(old);
            let new_score = system.match_score(&locale);
            new_score > old_score || (old_score == 0 && !locale.is_valid())
        });
        if replace {
            values.insert(key.to_owned(), (locale, value.to_owned()));
        }
    }
    finish_group(&mut entry, &group, &mut values);
    entry
}

/// Applies the values of a finished group the way Quickshell's `finishCategory` does. Only `[Desktop Entry]` with
/// `Type=Application` counts. Quickshell returns early for the other `[Desktop Entry]` groups without clearing the
/// values, so they carry over into the next group; that is kept.
fn finish_group(entry: &mut Entry, group: &str, values: &mut Values) {
    if group == "Desktop Entry" {
        let is_application = values.get("Type").is_some_and(|(_, kind)| kind == "Application");
        if !is_application {
            return;
        }
        let app = &mut entry.app;
        for (key, (_, value)) in values.iter() {
            match key.as_str() {
                "Name" => app.name = value.clone(),
                "GenericName" => app.generic_name = value.clone(),
                "Comment" => app.comment = value.clone(),
                "Icon" => app.icon = value.clone(),
                "Exec" => app.command = parse_exec(value),
                "Path" => app.working_directory = value.clone(),
                "Terminal" => app.terminal = value == "true",
                "Keywords" => app.keywords = value.split(';').filter(|k| !k.is_empty()).map(str::to_owned).collect(),
                "NoDisplay" => entry.no_display = value == "true",
                "Hidden" => entry.hidden = value == "true",
                _ => {}
            }
        }
    }
    values.clear();
}

/// The `applications` directories of the XDG data directories, the most important first.
pub fn desktop_dirs() -> Vec<PathBuf> {
    let var = |name: &str| std::env::var_os(name).map(|v| v.to_string_lossy().into_owned());
    desktop_dirs_from(var("XDG_DATA_HOME"), var("HOME"), var("XDG_DATA_DIRS"))
}

/// `desktop_dirs` from the values of `$XDG_DATA_HOME`, `$HOME` and `$XDG_DATA_DIRS` (`None` when unset), without
/// reading the environment.
fn desktop_dirs_from(data_home: Option<String>, home: Option<String>, data_dirs: Option<String>) -> Vec<PathBuf> {
    // Quickshell takes `$HOME` as given when it is set at all, so an empty one yields `/.local/share`.
    let data_home = data_home.filter(|d| !d.is_empty()).or_else(|| home.map(|h| format!("{h}/.local/share")));
    let data_dirs = data_dirs
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    data_home
        .into_iter()
        .chain(data_dirs.split(':').filter(|d| !d.is_empty()).map(str::to_owned))
        .map(|dir| PathBuf::from(format!("{dir}/applications")))
        .collect()
}

/// The applications of `dirs`, given most important first, sorted by id. A directory is read recursively; an id
/// of a file in a subdirectory is prefixed with the subdirectory names joined by `-`.
///
/// As Quickshell does, the directories are read least important first, and each entry then replaces the one of the
/// same id before it. A hidden entry removes that one and is not listed, and an entry without a name is ignored
/// (it neither replaces nor removes anything).
pub fn scan(dirs: &[PathBuf], system: &Locale) -> Vec<Application> {
    let mut results = Vec::new();
    for dir in dirs.iter().rev() {
        if dir.is_dir() {
            scan_directory(dir, "", system, &mut results);
        }
    }
    let mut merged: HashMap<String, Entry> = HashMap::new();
    for entry in results {
        if entry.hidden {
            merged.remove(&entry.app.id);
            continue;
        }
        if entry.app.name.is_empty() {
            continue;
        }
        merged.insert(entry.app.id.clone(), entry);
    }
    let mut apps: Vec<Application> = merged.into_values().filter(|e| !e.no_display).map(|e| e.app).collect();
    apps.sort_by(|a, b| a.id.cmp(&b.id));
    apps
}

/// The entries of `dir` and its subdirectories, appended to `results`. Hidden files and directories (names starting
/// with a dot) are left out, as Quickshell's directory listing leaves them out.
fn scan_directory(dir: &Path, prefix: &str, system: &Locale, results: &mut Vec<Entry>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<_> = read
        .filter_map(Result::ok)
        .filter(|child| !child.file_name().to_string_lossy().starts_with('.'))
        .collect();
    // The order decides which of two files of one id wins within a directory, so it is fixed by name.
    children.sort_by_key(|a| a.file_name());
    for child in children {
        let name = child.file_name().to_string_lossy().into_owned();
        let path = child.path();
        if path.is_dir() {
            let sub = if prefix.is_empty() { name } else { format!("{prefix}-{name}") };
            scan_directory(&path, &sub, system, results);
        } else if path.is_file() {
            // Only the last `.desktop` is cut off, so `a.b.desktop` has the id `a.b`.
            let Some(base) = name.strip_suffix(".desktop") else {
                continue;
            };
            let id = if prefix.is_empty() { base.to_owned() } else { format!("{prefix}-{base}") };
            // Quickshell decodes the file as UTF-8 and replaces what is not, which `from_utf8_lossy` does too.
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            results.push(parse_entry(&id, &String::from_utf8_lossy(&bytes), system));
        }
    }
}

/// The icon the launcher shows for `app`: for two applications a fixed one, else the entry's icon, else a generic
/// executable. The check is on the lowercased id and command, so it also catches the other spellings.
pub fn launcher_icon(app: &Application) -> String {
    let id = app.id.to_lowercase();
    let command = app.command.join(" ").to_lowercase();
    let mentions = |text: &str| id.contains(text) || command.contains(text);
    if mentions("nemo") {
        "nemo".to_owned()
    } else if mentions("nm-connection-editor") {
        "nm-device-wired".to_owned()
    } else if !app.icon.is_empty() {
        app.icon.clone()
    } else {
        "application-x-executable".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes `text` to `root/path`, creating the directories on the way.
    fn write(root: &Path, path: &str, text: &str) -> std::io::Result<()> {
        let file = root.join(path);
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(file, text)
    }

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(|p| PathBuf::from(*p)).collect()
    }

    #[test]
    fn locale_parts_split_like_quickshell() {
        assert_eq!(
            Locale::parse("de_DE.UTF-8@euro"),
            Locale { language: "de".into(), territory: "DE".into(), modifier: "euro".into() }
        );
        assert_eq!(Locale::parse("sr@latin"), Locale { language: "sr".into(), territory: String::new(), modifier: "latin".into() });
        assert_eq!(Locale::parse(""), Locale::default());
    }

    #[test]
    fn localized_name_follows_the_system_locale() {
        let de = Locale::parse("de_DE.UTF-8");
        let en = Locale::parse("en_US.UTF-8");
        // `de_AT` does not fit `de_DE`, so `de` wins, whichever of the two comes first.
        let first = "[Desktop Entry]\nType=Application\nName[de_AT]=Austrian\nName[de]=German\nName=Plain\nName[fr]=French\n";
        let last = "[Desktop Entry]\nType=Application\nName=Plain\nName[de]=German\nName[de_AT]=Austrian\n";
        assert_eq!(parse_entry("app", first, &de).app.name, "German");
        assert_eq!(parse_entry("app", last, &de).app.name, "German");
        // The `de_AT` value does not fit, so the unlocalized name is taken, even though it comes after it.
        assert_eq!(parse_entry("app", "[Desktop Entry]\nType=Application\nName[de_AT]=Austrian\nName=Plain\n", &de).app.name, "Plain");
        // Nothing localized fits `en_US`, so the unlocalized name is taken.
        assert_eq!(parse_entry("app", first, &en).app.name, "Plain");
    }

    #[test]
    fn exec_strings_split_like_quickshell() {
        // Quoted words stay one argument and the field code `%U` goes.
        assert_eq!(parse_exec(r#"foo "two words" %U"#), ["foo", "two words"]);
        // `%%` is a percent sign, a field code is not an argument of its own.
        assert_eq!(parse_exec("a %% b"), ["a", "%", "b"]);
        // Inside quotes `\s` is a space and `\"` a quote.
        assert_eq!(parse_exec(r#""a\sb" c"#), ["a b", "c"]);
        assert_eq!(parse_exec(r#""a\"b""#), [r#"a"b"#]);
        // Four backslashes inside quotes make one: the file holds `\\\\$HOME`, the command gets `\$HOME`.
        assert_eq!(parse_exec(r#"sh -c "echo \\\\$HOME""#), ["sh", "-c", r"echo \$HOME"]);
        // Outside quotes a backslash only keeps the next character, `\s` included.
        assert_eq!(parse_exec(r"a\sb"), ["asb"]);
    }

    #[test]
    fn only_application_entries_are_applied() {
        let system = Locale::default();
        let directory = parse_entry("dir", "[Desktop Entry]\nType=Directory\nName=Dir\nHidden=true\n", &system);
        assert_eq!(directory.app.name, "");
        assert!(!directory.hidden);
        let flags = parse_entry(
            "flags",
            "[Desktop Entry]\nType=Application\nName=X\nHidden=true\nNoDisplay=true\nTerminal=true\nPath=/tmp\nKeywords=a;;b;\nExec=x\n\
             [Desktop Action new]\nName=New\nExec=y\n",
            &system,
        );
        assert_eq!(flags.app.name, "X");
        assert!(flags.hidden && flags.no_display && flags.app.terminal);
        assert_eq!(flags.app.working_directory, "/tmp");
        assert_eq!(flags.app.keywords, ["a", "b"]);
        assert_eq!(flags.app.command, ["x"]);
    }

    #[test]
    fn scan_lets_the_data_home_win_and_masks_and_hides_as_quickshell() -> std::io::Result<()> {
        let root = std::env::temp_dir().join(format!("splice-apps-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (home, sys) = (root.join("home/applications"), root.join("sys/applications"));
        let app = |name: &str| format!("[Desktop Entry]\nType=Application\nName={name}\nExec=run-{name}\n");
        write(&sys, "one.desktop", &app("SysOne"))?;
        write(&sys, "two.desktop", &app("Two"))?;
        write(&sys, "three.desktop", &app("Three"))?;
        write(&sys, "dotted.name.desktop", &app("Dotted"))?;
        write(&sys, "quiet.desktop", "[Desktop Entry]\nType=Application\nName=Quiet\nNoDisplay=true\n")?;
        write(&sys, "kde/foo.desktop", &app("KdeFoo"))?;
        write(&sys, "notes.txt", "not an entry")?;
        write(&sys, ".hidden.desktop", &app("Dotfile"))?;
        write(&home, "one.desktop", &app("HomeOne"))?;
        // Hidden in the data home: the system's `two` disappears.
        write(&home, "two.desktop", "[Desktop Entry]\nType=Application\nName=Two\nHidden=true\n")?;
        // No name: the system's `three` stays.
        write(&home, "three.desktop", "[Desktop Entry]\nType=Application\nExec=none\n")?;

        let apps = scan(&[home, sys, root.join("missing")], &Locale::default());
        let listed: Vec<(&str, &str)> = apps.iter().map(|a| (a.id.as_str(), a.name.as_str())).collect();
        assert_eq!(
            listed,
            [("dotted.name", "Dotted"), ("kde-foo", "KdeFoo"), ("one", "HomeOne"), ("three", "Three")]
        );
        std::fs::remove_dir_all(&root)
    }

    #[test]
    fn desktop_dirs_follow_xdg() {
        let defaults = ["/home/u/.local/share/applications", "/usr/local/share/applications", "/usr/share/applications"];
        assert_eq!(desktop_dirs_from(None, Some("/home/u".into()), None), paths(&defaults));
        // Empty counts as unset, and an empty `$HOME` that is set gives `/.local/share`, as in Quickshell.
        assert_eq!(desktop_dirs_from(Some(String::new()), Some("/home/u".into()), Some(String::new())), paths(&defaults));
        assert_eq!(
            desktop_dirs_from(None, Some(String::new()), None),
            paths(&["/.local/share/applications", "/usr/local/share/applications", "/usr/share/applications"])
        );
        assert_eq!(
            desktop_dirs_from(Some("/data".into()), None, Some("/a:/b::".into())),
            paths(&["/data/applications", "/a/applications", "/b/applications"])
        );
        assert_eq!(desktop_dirs_from(None, None, Some("/x".into())), paths(&["/x/applications"]));
    }

    #[test]
    fn launcher_icons_override_a_few_applications() {
        let app = |id: &str, command: &[&str], icon: &str| Application {
            id: id.into(),
            command: command.iter().map(|c| c.to_string()).collect(),
            icon: icon.into(),
            ..Application::default()
        };
        assert_eq!(launcher_icon(&app("Nemo", &["nemo"], "folder")), "nemo");
        assert_eq!(launcher_icon(&app("org.gnome.Files", &["/usr/bin/NEMO", "--new-window"], "")), "nemo");
        assert_eq!(launcher_icon(&app("nm-connection-editor", &[], "")), "nm-device-wired");
        assert_eq!(launcher_icon(&app("firefox", &["firefox"], "firefox-icon")), "firefox-icon");
        assert_eq!(launcher_icon(&app("tool", &["tool"], "")), "application-x-executable");
    }
}
