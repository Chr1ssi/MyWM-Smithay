//! Finding, reading and writing the config file, and telling the compositor about it.
use std::{
    io::Write,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
};

fn state_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("mywm-settings/config-path"))
}

/// The remembered config path (chosen in this app), if any.
pub fn remembered_path() -> Option<PathBuf> {
    let text = std::fs::read_to_string(state_file()?).ok()?;
    let path = PathBuf::from(text.trim());
    (!text.trim().is_empty()).then_some(path)
}

pub fn remember_path(path: &Path) {
    if let Some(file) = state_file() {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, path.to_string_lossy().as_bytes());
    }
}

/// Which file to edit: the command line, then the remembered choice, then what the
/// compositor itself would load (`MYWM_CONFIG`, `~/.config/mywm/config.toml`).
pub fn resolve(cli: Option<PathBuf>) -> PathBuf {
    cli.or_else(remembered_path)
        .or_else(mywm_config::Config::path)
        .unwrap_or_else(|| PathBuf::from("config.toml"))
}

/// The file behind symlinks (a dotfiles link ends up in the dotfiles repository).
pub fn real_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// A file in the Nix store cannot be edited; its owner has to change the Nix code.
pub fn is_in_store(path: &Path) -> bool {
    real_path(path).starts_with("/nix/store")
}

/// The text of the file; a file that does not exist yet is an empty config.
pub fn read(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// Write `text` to the real file behind `path`, replacing it in one step. A symlink is kept.
pub fn write(path: &Path, text: &str) -> Result<(), String> {
    if is_in_store(path) {
        return Err("Die Datei liegt im Nix-Store und ist schreibgeschützt. Bitte die Datei im Dotfiles-Ordner wählen.".into());
    }
    let target = real_path(path);
    let dir = target.parent().ok_or("the path has no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = dir.join(format!(".{name}.mywm-settings.tmp"));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        // Same permissions as the file it replaces.
        if let Ok(metadata) = std::fs::metadata(&target) {
            std::fs::set_permissions(&temporary, metadata.permissions())?;
        }
        std::fs::rename(&temporary, &target)
    })();
    result.map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        format!("{}: {e}", target.display())
    })
}

/// Make the running compositor reload its configuration. Returns whether one answered.
pub fn reload_compositor() -> bool {
    let socket = std::env::var_os("MYWM_SOCKET")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("mywm.sock")));
    let Some(socket) = socket else { return false };
    UnixStream::connect(socket).and_then(|mut stream| stream.write_all(b"v1 theme-reload\n")).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mywm-settings-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writing_through_a_symlink_changes_the_file_it_points_to() {
        let dir = temp_dir("symlink");
        let dotfiles = dir.join("dotfiles");
        std::fs::create_dir_all(&dotfiles).unwrap();
        let real = dotfiles.join("config.toml");
        std::fs::write(&real, "old").unwrap();
        let link = dir.join("config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write(&link, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link must stay a link");
        assert!(std::fs::read_dir(&dotfiles).unwrap().count() == 1, "no temporary file left behind");
    }

    #[test]
    fn a_missing_file_reads_as_empty_and_is_created_with_its_folders() {
        let dir = temp_dir("missing");
        let path = dir.join("a/b/config.toml");
        assert_eq!(read(&path).unwrap(), "");
        write(&path, "x = 1\n").unwrap();
        assert_eq!(read(&path).unwrap(), "x = 1\n");
    }

    #[test]
    fn the_nix_store_is_refused() {
        assert!(is_in_store(Path::new("/nix/store/abc-config.toml")));
        assert!(!is_in_store(Path::new("/home/user/dotfiles/config.toml")));
        assert!(write(Path::new("/nix/store/abc-config.toml"), "x").is_err());
    }
}
