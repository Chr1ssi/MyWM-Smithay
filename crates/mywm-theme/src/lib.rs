use material_colors::{
    color::Argb,
    image::{FilterType, ImageReader},
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

mod palette;
use palette::Analysis;

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
    analysis: Analysis,
    colors: BTreeMap<String, Token>,
}

const KITTY: &str = include_str!("../assets/kitty.conf");
const FOOT: &str = include_str!("../assets/foot.ini");
const GTK3: &str = include_str!("../assets/gtk3.css");
const GTK4: &str = include_str!("../assets/gtk4.css");
const NVIM: &str = include_str!("../assets/nvim.lua");
const VESKTOP: &str = include_str!("../assets/vesktop.css");
const GREETER: &str = include_str!("../assets/greeter.css");

pub fn apply(wallpaper: &Path) -> Result<()> {
    let state_path = state_path().ok_or("HOME or XDG_STATE_HOME is required for theme persistence")?;
    // Decoding a large wallpaper is the expensive part (about 150 ms at every session start); the
    // colors and the files are always derived again, so new templates still reach them.
    let cached = cached_analysis(&state_path, wallpaper);
    let was_cached = cached.is_some();
    let analysis = match cached {
        Some(analysis) => analysis,
        None => {
            let mut image = ImageReader::open(wallpaper)?;
            image.resize(112, 112, FilterType::Triangle);
            Analysis::of_image(&image)
        }
    };
    let state = state_from_analysis(wallpaper, analysis);
    let directory = state_path
        .parent()
        .ok_or("theme state path has no parent directory")?;
    fs::create_dir_all(directory)?;

    let serialized = serde_json::to_string_pretty(&state)? + "\n";
    let mut changed = write_atomic(&state_path, &serialized)?;
    if !was_cached && !changed {
        // The same colors from a changed image: mark the theme as newer, so the next start skips decoding.
        fs::File::options().write(true).open(&state_path)?.set_modified(std::time::SystemTime::now())?;
    }
    changed |= render(directory.join("kitty.conf"), KITTY, &state)?;
    changed |= render(directory.join("foot.ini"), FOOT, &state)?;
    changed |= render(directory.join("gtk-3.css"), GTK3, &state)?;
    changed |= render(directory.join("gtk-4.css"), GTK4, &state)?;
    changed |= render(directory.join("nvim.lua"), NVIM, &state)?;
    changed |= render(directory.join("vesktop.css"), VESKTOP, &state)?;
    sync_greeter(wallpaper, &state);
    // Consumers already show an unchanged theme (and at session start none runs yet).
    if changed {
        notify_consumers(directory);
    }
    println!("Theme generated from {}", wallpaper.display());
    Ok(())
}

/// What the theme in `state_path` took from `wallpaper` if it was generated after the image last changed.
fn cached_analysis(state_path: &Path, wallpaper: &Path) -> Option<Analysis> {
    let generated = fs::metadata(state_path).ok()?.modified().ok()?;
    // A symlinked wallpaper (home-manager) can point at a new image whose Nix store time never changes;
    // the link itself is recreated then.
    let image_changed = [fs::metadata(wallpaper), fs::symlink_metadata(wallpaper)]
        .into_iter()
        .map(|metadata| metadata.ok()?.modified().ok())
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .max()?;
    if image_changed >= generated {
        return None;
    }
    let saved: serde_json::Value = serde_json::from_str(&fs::read_to_string(state_path).ok()?).ok()?;
    if saved["source"].as_str()? != wallpaper.to_string_lossy() {
        return None;
    }
    serde_json::from_value(saved["analysis"].clone()).ok()
}

pub fn apply_from_wallpaper_state(state: &Path, wallpaper_directory: &Path) -> Result<()> {
    apply(&current_wallpaper(state, wallpaper_directory)?)
}

/// The wallpaper in use: the picker's choice saved in `state`, else the first image in `wallpaper_directory`.
pub fn current_wallpaper(state: &Path, wallpaper_directory: &Path) -> Result<PathBuf> {
    let saved = fs::read_to_string(state)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value["wallpaper"].as_str().map(str::to_owned))
        .and_then(|url| decode_file_url(&url));
    match saved.filter(|path| path.is_file()) {
        Some(path) => Ok(path),
        None => first_wallpaper(wallpaper_directory),
    }
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

/// Render the template into `path`; true if that changed the file.
fn render(path: PathBuf, source: &str, state: &ThemeState) -> Result<bool> {
    let mut env = Environment::new();
    env.add_template("theme", source)?;
    let output = env.get_template("theme")?.render(state)?;
    write_atomic(&path, &output)
}

/// Replace `path` with `contents` unless it holds them already; true if it was written.
fn write_atomic(path: &Path, contents: &str) -> Result<bool> {
    if fs::read(path).is_ok_and(|current| current == contents.as_bytes()) {
        return Ok(false);
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, contents)?;
    fs::rename(temporary, path)?;
    Ok(true)
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
        // theme.json for mywm-shell's login screen, theme.css for a GTK greeter.
        write_atomic(&directory.join("theme.json"), &(serde_json::to_string_pretty(state)? + "\n"))?;
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

fn state_from_analysis(path: &Path, analysis: Analysis) -> ThemeState {
    let mut colors = BTreeMap::new();
    for (name, value) in palette::roles(&analysis) {
        insert(&mut colors, name, value);
    }
    ThemeState {
        version: 1,
        mode: "dark",
        scheme: "wallpaper",
        source: path.to_string_lossy().into_owned(),
        source_color: analysis.source().to_hex_with_pound(),
        analysis,
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

fn state_home() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
}

/// Where the picker saves the chosen wallpaper.
pub fn wallpaper_state_path() -> Option<PathBuf> {
    Some(state_home()?.join("mywm/wallpaper.json"))
}

fn state_dir() -> Result<PathBuf> {
    let home = state_home().ok_or("HOME or XDG_STATE_HOME is required for wallpaper persistence")?;
    let dir = home.join("mywm");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The Quickshell wallpaper picker with the helpers and paths it needs; the compositor draws the image.
fn picker_process(config: &mywm_config::Config) -> Result<Command> {
    let exe = mywm_config::current_exe()?;
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
    Ok(command)
}

/// Replace this process by a resident wallpaper picker (`--wallpaper`) that waits for `openPicker` calls.
pub fn run_wallpaper(config: &mywm_config::Config) -> Result<()> {
    Err(picker_process(config)?.exec().into())
}

/// Show the picker at `(x, y)` (`--wallpaper-picker`): a running one moves there; otherwise this process
/// becomes a picker that ends once it is closed, so nothing of it stays in memory between uses.
pub fn open_picker(config: &mywm_config::Config, x: i32, y: i32) -> Result<()> {
    let running = picker_command(x, y)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if running {
        return Ok(());
    }
    let mut command = picker_process(config)?;
    command.env("MYWM_WALLPAPER_OPEN_AT", format!("{x},{y}"));
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
        "--wallpaper-picker" => {
            let coordinate = |n: usize| arg(n).map_or(Ok(0), |value| value.parse::<i32>());
            match (coordinate(2), coordinate(3)) {
                (Ok(x), Ok(y)) => open_picker(config, x, y),
                _ => Err("--wallpaper-picker takes the position as two integers: X Y".into()),
            }
        }
        // The compositor draws its own bar then; the session's startup still runs this mode.
        "--bar" if config.builtin_shell => Ok(()),
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

    fn sample_state() -> ThemeState {
        let analysis = Analysis::of_histogram([(Argb::from_u32(0xff336699), 60), (Argb::from_u32(0xffd4a017), 10), (Argb::from_u32(0xff121820), 30)]);
        state_from_analysis(Path::new("/wallpaper.png"), analysis)
    }

    #[test]
    fn generated_state_has_noctalia_compatible_tokens() {
        let state = sample_state();
        // Every role the Material-based generator wrote, so consumers and templates keep working.
        for name in [
            "primary", "on_primary", "primary_container", "on_primary_container", "inverse_primary", "primary_fixed",
            "primary_fixed_dim", "on_primary_fixed", "on_primary_fixed_variant", "secondary", "on_secondary",
            "secondary_container", "on_secondary_container", "secondary_fixed", "secondary_fixed_dim", "on_secondary_fixed",
            "on_secondary_fixed_variant", "tertiary", "on_tertiary", "tertiary_container", "on_tertiary_container",
            "tertiary_fixed", "tertiary_fixed_dim", "on_tertiary_fixed", "on_tertiary_fixed_variant", "error", "on_error",
            "error_container", "on_error_container", "surface_dim", "surface", "surface_tint", "surface_bright",
            "surface_container_lowest", "surface_container_low", "surface_container", "surface_container_high",
            "surface_container_highest", "on_surface", "on_surface_variant", "outline", "outline_variant", "inverse_surface",
            "inverse_on_surface", "surface_variant", "background", "on_background", "shadow", "scrim",
            "terminal_background", "terminal_foreground", "terminal_cursor", "terminal_cursor_text", "terminal_selection_bg",
            "terminal_selection_fg",
        ] {
            assert!(state.colors.contains_key(name), "{name}");
        }
        for group in ["normal", "bright"] {
            for color in ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] {
                assert!(state.colors.contains_key(&format!("terminal_{group}_{color}")), "{group} {color}");
            }
        }
        assert_eq!(state.colors.len(), 71);
        assert_eq!(state.colors["primary"].default.hex.len(), 7);
    }

    #[test]
    fn borders_and_text_stand_out_from_the_surface() {
        // WCAG luminance of the written sRGB values, independent of the HCT tones they come from.
        fn luminance(hex: &str) -> f64 {
            let color: Argb = hex.parse().unwrap();
            let channel = |value: u8| {
                let c = f64::from(value) / 255.0;
                if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * channel(color.red) + 0.7152 * channel(color.green) + 0.0722 * channel(color.blue)
        }
        let state = sample_state();
        let ratio = |a: &str, b: &str| {
            let (a, b) = (luminance(&state.colors[a].default.hex), luminance(&state.colors[b].default.hex));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        assert!(ratio("outline_variant", "surface") >= 3.0);
        assert!(ratio("on_surface_variant", "surface") >= 7.0);
        assert!(ratio("on_surface", "surface") > ratio("on_surface_variant", "surface"));
        assert!(ratio("error", "surface") >= 4.5);
        assert!(ratio("terminal_normal_blue", "terminal_background") >= 4.5);
    }

    #[test]
    fn foot_template_renders_colors_without_hash() {
        let state = sample_state();
        let mut env = Environment::new();
        env.add_template("foot", FOOT).unwrap();
        let output = env.get_template("foot").unwrap().render(&state).unwrap();
        let background = &state.colors["terminal_background"].default.hex[1..];
        assert!(output.contains(&format!("background={background}\n")));
        assert!(!output.contains("=#"));
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
    fn cached_analysis_needs_the_same_image_unchanged_since() {
        use std::time::{Duration, SystemTime};
        let dir = std::env::temp_dir().join(format!("mywm-theme-cache-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let image = dir.join("wall.png");
        let state = dir.join("theme.json");
        fs::write(&image, b"image").unwrap();
        let age = |path: &Path, time: SystemTime| fs::File::options().write(true).open(path).unwrap().set_modified(time).unwrap();
        age(&image, SystemTime::now() - Duration::from_secs(60));
        let analysis = sample_state().analysis;
        let saved = |source: &Path| {
            let json = serde_json::json!({ "source": source.to_string_lossy(), "analysis": analysis });
            fs::write(&state, json.to_string()).unwrap();
        };
        saved(&image);
        // JSON may round the last digit of a float; the colors are what has to survive.
        let cached = cached_analysis(&state, &image).unwrap();
        assert_eq!(palette::roles(&cached), palette::roles(&analysis));
        // Another image, or this one changed after the theme was generated.
        let other = dir.join("other.png");
        fs::write(&other, b"other").unwrap();
        age(&other, SystemTime::now() - Duration::from_secs(60));
        assert_eq!(cached_analysis(&state, &other), None);
        age(&image, SystemTime::now() + Duration::from_secs(60));
        assert_eq!(cached_analysis(&state, &image), None);
        // A link made after the theme counts as a new image, however old its target is.
        let link = dir.join("link.png");
        age(&other, SystemTime::now() - Duration::from_secs(60));
        saved(&link);
        std::os::unix::fs::symlink(&other, &link).unwrap();
        assert_eq!(cached_analysis(&state, &link), None);
        fs::remove_dir_all(&dir).unwrap();
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
