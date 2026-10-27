//! Splice compositor entry point: DRM/libinput on hardware, or nested in a window for development.

mod attention;
mod barriers;
mod floating_memory;
mod blur;
mod color_picker;
mod cursor;
mod desktop;
mod effects;
mod gamma;
mod handlers;
mod image_capture;
mod input;
mod ipc;
mod layers;
mod logging;
mod monitors;
mod output_management;
mod overview;
mod pacing;
mod protocols;
mod render;
mod screencopy;
mod screenshot;
mod session;
mod shell;
mod shell_calendar;
mod shell_greeter;
mod shell_launcher;
mod shell_lock;
mod shell_control;
mod shell_notifications;
mod shell_panels;
mod shell_services;
mod shell_tray;
mod shell_wallpaper;
mod shutdown;
mod udev;
mod wallpaper;
mod xwayland;
mod state;
mod winit;

use calloop::EventLoop;
use tracing_subscriber::EnvFilter;

pub use state::State;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    splice_config::migrate_legacy_state_dir();
    // Client modes (`--lock`, `--idle`) write to the terminal only; the compositor logs to a file too.
    let client_mode = matches!(
        std::env::args().nth(1).as_deref(),
        Some(
            "--lock" | "--idle" | "--theme-from-wallpaper" | "--theme-from-state"
        )
    );
    let log_file = if client_mode {
        tracing_subscriber::fmt().with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))).init();
        None
    } else {
        logging::init()
    };
    if !client_mode {
        tracing::info!(
            "splice-compositor {} starting (log: {})",
            env!("CARGO_PKG_VERSION"),
            log_file.as_ref().map_or("stderr only".into(), |p| p.display().to_string())
        );
    }

    // Client modes used by the session scripts and swayidle.
    match std::env::args().nth(1).as_deref() {
        Some("--lock") => return splice_config::session::lock_and_wait(),
        Some("--idle") => return splice_config::session::exec_idle(&config_or_default().idle),
        _ => {}
    }
    let args: Vec<String> = std::env::args().collect();
    let (config, config_error) = splice_config::Config::load_or_default();
    if let Some(result) = splice_theme::client_mode(&args, &config) {
        if let Some(error) = &config_error {
            tracing::warn!("configuration not loaded, using the defaults: {error}");
        }
        return result;
    }

    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let display = smithay::reexports::wayland_server::Display::new()?;
    tracing::info!("configuration: {}", splice_config::Config::path().map_or("defaults".into(), |p| p.display().to_string()));
    if let Some(error) = &config_error {
        tracing::error!("configuration not loaded, using the defaults: {error}");
    }
    // Inside another compositor or X server we run nested; on a bare seat we drive the hardware.
    // SPLICE_BACKEND=winit|drm overrides the guess.
    let nested = match std::env::var("SPLICE_BACKEND").as_deref() {
        Ok("winit") => true,
        Ok("drm" | "udev") => false,
        _ => std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some(),
    };
    // Nested, the host owns Super; SPLICE_MODKEY=super keeps the configured modifiers.
    let remap_super = nested && std::env::var("SPLICE_MODKEY").map_or(true, |v| !v.eq_ignore_ascii_case("super"));
    // SAFETY: nothing else runs yet.
    unsafe { cursor::export_theme() };
    let mut state = State::new(&mut event_loop, display, config, remap_super);
    // `State::new` may have found an unknown keymap already; a file that does not load has none.
    if config_error.is_some() {
        state.config_error = config_error;
    }

    if nested {
        winit::init(&mut event_loop, &mut state)?;
    } else {
        udev::init(&mut event_loop, &mut state)?;
    }

    // SAFETY: single-threaded at this point; no other thread reads the environment.
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
        if !nested {
            // The desktop portals pick their backends by this name (`splice-portals.conf`,
            // `UseIn=splice` of splice-portal); `splice-session` sets the same.
            if std::env::var_os("XDG_CURRENT_DESKTOP").is_none() {
                std::env::set_var("XDG_CURRENT_DESKTOP", "splice");
            }
            std::env::set_var("XDG_SESSION_TYPE", "wayland");
        }
    }
    tracing::info!("listening on WAYLAND_DISPLAY={:?}", state.socket_name);

    // Shut down cleanly (removing the bar socket) on SIGTERM/SIGINT.
    shutdown::install(&event_loop.handle())?;

    xwayland::start(&mut state);
    // `--greeter` alone: the built-in login screen.
    let builtin_greeter = args.get(1).is_some_and(|a| a == "--greeter") && args.get(2).is_none();
    if builtin_greeter {
        state.greeter_setup();
    }
    state.shell_init();

    if let Err(error) = ipc::init(&event_loop.handle(), &mut state) {
        tracing::error!("cannot start the bar socket: {error}");
    }

    // On hardware, idle handling (lock after a while, then monitors off) is swayidle's job.
    if !nested
        && let Ok(executable) = std::env::current_exe()
        && let Some(mut command) = splice_config::session::idle_command(&state.config.idle, &executable.to_string_lossy())
    {
        command.env("WAYLAND_DISPLAY", &state.socket_name);
        match command.spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(error) => tracing::warn!("cannot start swayidle: {error}"),
        }
    }

    // `--greeter <command>`: the login screen; the compositor ends with it, so greetd can start the session. (Without
    // a command the built-in one shows, set up above.)
    match args.get(1).map(String::as_str) {
        Some("--greeter") => {
            if let Some(cmd) = args.get(2) {
                state.spawn_greeter(cmd);
            }
        }
        Some(cmd) => state.spawn_command(&["sh".into(), "-c".into(), cmd.into()], false),
        None => {}
    }

    event_loop.run(None, &mut state, |state| {
        state.space.refresh();
        state.popups.cleanup();
        state.notify_input_activity();
        // Before the flush: the shell looks at what changed since the bar's clients last heard.
        state.shell_refresh();
        state.ipc_flush();
        let _ = state.display_handle.flush_clients();
    })?;
    Ok(())
}

/// The configuration for the client modes (lock screen, idle): its defaults if the file is broken.
fn config_or_default() -> splice_config::Config {
    let (config, error) = splice_config::Config::load_or_default();
    if let Some(error) = error {
        tracing::warn!("configuration not loaded, using the defaults: {error}");
    }
    config
}
