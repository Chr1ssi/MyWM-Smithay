use material_colors::{
    color::Argb,
    dynamic_color::Variant,
    image::{FilterType, ImageReader},
    scheme::Scheme,
    theme::ThemeBuilder,
};
use minijinja::Environment;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    io::Write,
    os::unix::ffi::OsStringExt,
    os::unix::net::UnixStream,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub use mywm_config::theme_state_path as state_path;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Debug, Serialize)]
struct HexValue {
    hex: String,
}

#[derive(Clone, Debug, Serialize)]
struct Token {
    default: HexValue,
}

#[derive(Debug, Serialize)]
struct ThemeState {
    version: u8,
    mode: &'static str,
    scheme: &'static str,
    source: String,
    source_color: String,
    colors: BTreeMap<String, Token>,
}

const KITTY: &str = include_str!("../assets/kitty.conf");
const GTK3: &str = include_str!("../assets/gtk3.css");
const GTK4: &str = include_str!("../assets/gtk4.css");
const NVIM: &str = include_str!("../assets/nvim.lua");
const VESKTOP: &str = include_str!("../assets/vesktop.css");
const GREETER: &str = include_str!("../assets/greeter.css");

pub fn apply(wallpaper: &Path) -> Result<()> {
    let mut image = ImageReader::open(wallpaper)?;
    image.resize(112, 112, FilterType::Triangle);
    let source = ImageReader::extract_color(&image);
    let generated = ThemeBuilder::with_source(source)
        .variant(Variant::Content)
        .build();
    let state = state_from_scheme(wallpaper, source, &generated.schemes.dark);
    let state_path = state_path().ok_or("HOME or XDG_STATE_HOME is required for theme persistence")?;
    let directory = state_path
        .parent()
        .ok_or("theme state path has no parent directory")?;
    fs::create_dir_all(directory)?;

    let serialized = serde_json::to_string_pretty(&state)? + "\n";
    write_atomic(&state_path, &serialized)?;
    render(directory.join("kitty.conf"), KITTY, &state)?;
    render(directory.join("gtk-3.css"), GTK3, &state)?;
    render(directory.join("gtk-4.css"), GTK4, &state)?;
    render(directory.join("nvim.lua"), NVIM, &state)?;
    render(directory.join("vesktop.css"), VESKTOP, &state)?;
    sync_greeter(wallpaper, &state);
    notify_consumers(directory);
    println!("Theme generated from {}", wallpaper.display());
    Ok(())
}

pub fn apply_from_wallpaper_state(state: &Path, wallpaper_directory: &Path) -> Result<()> {
    let saved = fs::read_to_string(state)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value["wallpaper"].as_str().map(str::to_owned))
        .and_then(|url| decode_file_url(&url));
    let wallpaper = match saved.filter(|path| path.is_file()) {
        Some(path) => path,
        None => first_wallpaper(wallpaper_directory)?,
    };
    apply(&wallpaper)
}

fn first_wallpaper(directory: &Path) -> Result<PathBuf> {
    let mut candidates = fs::read_dir(directory)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|extension| {
                        matches!(
                            extension.to_ascii_lowercase().as_str(),
                            "jpg" | "jpeg" | "png" | "webp" | "bmp"
                        )
                    })
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .ok_or_else(|| format!("no wallpapers found in {}", directory.display()).into())
}

fn decode_file_url(url: &str) -> Option<PathBuf> {
    let encoded = url.strip_prefix("file://")?.as_bytes();
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut index = 0;
    while index < encoded.len() {
        if encoded[index] == b'%' {
            let hex = encoded.get(index + 1..index + 3)?;
            let value = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
            bytes.push(value);
            index += 3;
        } else {
            bytes.push(encoded[index]);
            index += 1;
        }
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

fn render(path: PathBuf, source: &str, state: &ThemeState) -> Result<()> {
    let mut env = Environment::new();
    env.add_template("theme", source)?;
    let output = env.get_template("theme")?.render(state)?;
    write_atomic(&path, &output)
}

fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, contents)?;
    fs::rename(temporary, path)?;
    Ok(())
}

/// Mirrors wallpaper and colors into a directory the greeter user can read.
fn sync_greeter(wallpaper: &Path, state: &ThemeState) {
    let Some(directory) = std::env::var_os("MYWM_GREETER_DIR")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
    else {
        return;
    };
    let sync = || -> Result<()> {
        render(directory.join("theme.css"), GREETER, state)?;
        let temporary = directory.join("background.tmp");
        fs::copy(wallpaper, &temporary)?;
        fs::rename(temporary, directory.join("background"))?;
        Ok(())
    };
    if let Err(error) = sync() {
        eprintln!("Greeter theme not updated in {}: {error}", directory.display());
    }
}

fn notify_consumers(directory: &Path) {
    if let Some(socket) = std::env::var_os("MYWM_SOCKET")
        && let Ok(mut stream) = UnixStream::connect(socket)
    {
        let _ = stream.write_all(b"v1 theme-reload\n");
    }
    // Without a socket, `kitty @` talks to the controlling terminal and waits ten seconds for an
    // answer that never comes (the console of the greeter at session start); detached from it, the
    // call fails at once.
    let mut kitty = Command::new("kitty");
    kitty
        .args(["@", "set-colors", "--all"])
        .arg(directory.join("kitty.conf"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: `setsid` is async-signal-safe and touches nothing but the child's own session.
    unsafe {
        kitty.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let _ = kitty.status();
    let _ = Command::new("pkill").args(["-USR1", "nvim"]).status();
}

fn insert(colors: &mut BTreeMap<String, Token>, name: &str, value: Argb) {
    colors.insert(
        name.to_owned(),
        Token {
            default: HexValue {
                hex: value.to_hex_with_pound(),
            },
        },
    );
}

fn state_from_scheme(path: &Path, source: Argb, scheme: &Scheme) -> ThemeState {
    let mut colors = BTreeMap::new();
    macro_rules! roles {
        ($($name:ident),+ $(,)?) => { $(insert(&mut colors, stringify!($name), scheme.$name);)+ };
    }
    roles!(
        primary,
        on_primary,
        primary_container,
        on_primary_container,
        inverse_primary,
        primary_fixed,
        primary_fixed_dim,
        on_primary_fixed,
        on_primary_fixed_variant,
        secondary,
        on_secondary,
        secondary_container,
        on_secondary_container,
        secondary_fixed,
        secondary_fixed_dim,
        on_secondary_fixed,
        on_secondary_fixed_variant,
        tertiary,
        on_tertiary,
        tertiary_container,
        on_tertiary_container,
        tertiary_fixed,
        tertiary_fixed_dim,
        on_tertiary_fixed,
        on_tertiary_fixed_variant,
        error,
        on_error,
        error_container,
        on_error_container,
        surface_dim,
        surface,
        surface_tint,
        surface_bright,
        surface_container_lowest,
        surface_container_low,
        surface_container,
        surface_container_high,
        surface_container_highest,
        on_surface,
        on_surface_variant,
        outline,
        outline_variant,
        inverse_surface,
        inverse_on_surface,
        surface_variant,
        background,
        on_background,
        shadow,
        scrim
    );
    for (name, value) in [
        ("terminal_background", scheme.surface),
        ("terminal_foreground", scheme.on_surface),
        ("terminal_cursor", scheme.on_surface),
        ("terminal_cursor_text", scheme.surface),
        ("terminal_selection_bg", scheme.surface_container_highest),
        ("terminal_selection_fg", scheme.on_surface),
        ("terminal_normal_black", scheme.surface_container_high),
        ("terminal_normal_red", scheme.error),
        ("terminal_normal_green", scheme.tertiary),
        ("terminal_normal_yellow", scheme.secondary),
        ("terminal_normal_blue", scheme.primary),
        ("terminal_normal_magenta", scheme.tertiary_fixed_dim),
        ("terminal_normal_cyan", scheme.secondary_fixed_dim),
        ("terminal_normal_white", scheme.on_surface),
        ("terminal_bright_black", scheme.on_surface_variant),
        ("terminal_bright_red", scheme.error_container),
        ("terminal_bright_green", scheme.tertiary_fixed),
        ("terminal_bright_yellow", scheme.secondary_fixed),
        ("terminal_bright_blue", scheme.primary_fixed),
        ("terminal_bright_magenta", scheme.tertiary_fixed),
        ("terminal_bright_cyan", scheme.secondary_fixed),
        ("terminal_bright_white", scheme.surface_bright),
    ] {
        insert(&mut colors, name, value);
    }
    ThemeState {
        version: 1,
        mode: "dark",
        scheme: "content",
        source: path.to_string_lossy().into_owned(),
        source_color: source.to_hex_with_pound(),
        colors,
    }
}


pub const IMAGE_EXTENSIONS: [&str; 5] = ["jpg", "jpeg", "png", "webp", "bmp"];

/// Wallpaper images in `directory`, sorted by path.
pub fn wallpapers(directory: &str) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let is_image = path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()));
        if path.is_file() && is_image {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// `--wallpaper-list`: one `file://` URL per wallpaper (read by the picker).
pub fn list(config: &mywm_config::Config) -> Result<()> {
    let mut output = std::io::stdout().lock();
    for path in wallpapers(&config.wallpaper_directory)? {
        if let Some(path) = path.to_str() {
            writeln!(output, "{}", file_url(path))?;
        }
    }
    Ok(())
}

pub fn file_url(path: &str) -> String {
    let mut url = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(&byte) {
            url.push(char::from(byte));
        } else {
            use std::fmt::Write;
            write!(url, "%{byte:02X}").unwrap();
        }
    }
    url
}

fn state_dir() -> Result<PathBuf> {
    let home = std::env::var_os("XDG_STATE_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or("HOME or XDG_STATE_HOME is required for wallpaper persistence")?;
    let dir = home.join("mywm");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Replace this process by the Quickshell wallpaper layer (`--wallpaper`).
pub fn run_wallpaper(config: &mywm_config::Config) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe()?;
    let mut command = Command::new("qs");
    command
        .arg("--path")
        .arg(mywm_config::shell_qml("wallpaper.qml"))
        .arg("--no-duplicate")
        .env("MYWM_WALLPAPER_HELPER", &exe)
        .env("MYWM_THEME_HELPER", &exe)
        .env("MYWM_WALLPAPER_DIRECTORY", &config.wallpaper_directory)
        .env("MYWM_WALLPAPER_STATE", state_dir()?.join("wallpaper.json"))
        .envs(config.theme_env());
    Err(command.exec().into())
}

/// The command that opens the picker of the running wallpaper layer at `(x, y)`.
pub fn picker_command(x: i32, y: i32) -> Command {
    let mut command = Command::new("qs");
    command
        .args(["ipc", "--path"])
        .arg(mywm_config::shell_qml("wallpaper.qml"))
        .args(["call", "wallpaper", "openPicker", &x.to_string(), &y.to_string()]);
    command
}

/// Handle the helper modes of the binary (`--wallpaper`, `--theme-from-wallpaper`, ...).
/// `None` if `args` is not one of them.
pub fn client_mode(args: &[String], config: &mywm_config::Config) -> Option<Result<()>> {
    use std::os::unix::process::CommandExt;
    let arg = |n: usize| args.get(n).map(String::as_str);
    Some(match arg(1)? {
        "--theme-from-wallpaper" => match arg(2) {
            Some(path) => apply(Path::new(path)),
            None => Err("--theme-from-wallpaper requires an image path".into()),
        },
        "--theme-from-state" => match arg(2) {
            Some(state) => apply_from_wallpaper_state(Path::new(state), Path::new(arg(3).unwrap_or(&config.wallpaper_directory))),
            None => Err("--theme-from-state requires a wallpaper state path".into()),
        },
        "--wallpaper-list" => list(config),
        "--wallpaper" => run_wallpaper(config),
        "--wallpaper-picker" => match picker_command(0, 0).status() {
            Ok(status) if status.success() => Ok(()),
            Ok(_) => Err("wallpaper picker exited unsuccessfully".into()),
            Err(error) => Err(error.into()),
        },
        "--bar" => {
            let mut command = Command::new("qs");
            command.arg("--path").arg(mywm_config::shell_qml("bar.qml")).arg("--no-duplicate").envs(config.theme_env());
            Err(command.exec().into())
        }
        "--launcher" => {
            let mut command = Command::new(&config.launcher[0]);
            command.args(&config.launcher[1..]).envs(config.theme_env()).envs(config.terminal_env());
            Err(command.exec().into())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_state_has_noctalia_compatible_tokens() {
        let theme = ThemeBuilder::with_source(Argb::from_u32(0xff336699)).build();
        let state = state_from_scheme(
            Path::new("/wallpaper.png"),
            theme.source,
            &theme.schemes.dark,
        );
        assert!(state.colors.contains_key("primary"));
        assert!(state.colors.contains_key("surface_container"));
        assert!(state.colors.contains_key("terminal_bright_white"));
        assert_eq!(state.colors["primary"].default.hex.len(), 7);
    }

    #[test]
    fn decodes_wallpaper_file_urls() {
        assert_eq!(
            decode_file_url("file:///tmp/Bild%20%231%25%20%C3%A4.png").unwrap(),
            PathBuf::from("/tmp/Bild #1% ä.png")
        );
        assert!(decode_file_url("https://example.com/wallpaper.png").is_none());
    }

    #[test]
    fn paths_are_encoded_without_interpreting_url_delimiters() {
        assert_eq!(file_url("/tmp/Bild #1% ä.png"), "file:///tmp/Bild%20%231%25%20%C3%A4.png");
    }

    #[test]
    fn picker_command_uses_separate_coordinate_arguments() {
        let command = picker_command(-1280, 720);
        let args: Vec<_> = command.get_args().map(|s| s.to_str().unwrap()).collect();
        let tail = &args[args.len() - 5..];
        assert_eq!(tail, ["call", "wallpaper", "openPicker", "-1280", "720"]);
    }
}
