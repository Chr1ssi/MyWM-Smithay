//! MyWM compositor entry point: DRM/libinput on hardware, or nested in a window for development.

mod cursor;
mod desktop;
mod handlers;
mod input;
mod ipc;
mod layers;
mod logging;
mod monitors;
mod pacing;
mod protocols;
mod render;
mod screencopy;
mod session;
mod udev;
mod xwayland;
mod state;
mod winit;

use calloop::{
    EventLoop,
    signals::{Signal, Signals},
};
use tracing_subscriber::EnvFilter;

pub use state::State;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Client modes (`--lock`, `--idle`) write to the terminal only; the compositor logs to a file too.
    let client_mode = matches!(std::env::args().nth(1).as_deref(), Some("--lock" | "--idle"));
    let log_file = if client_mode {
        tracing_subscriber::fmt().with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))).init();
        None
    } else {
        logging::init()
    };
    if !client_mode {
        tracing::info!(
            "mywm-compositor {} starting (log: {})",
            env!("CARGO_PKG_VERSION"),
            log_file.as_ref().map_or("stderr only".into(), |p| p.display().to_string())
        );
    }

    // Client modes shared with the River-based MyWM, used by the session scripts and swayidle.
    match std::env::args().nth(1).as_deref() {
        Some("--lock") => return mywm_config::session::lock_and_wait(&mywm_config::Config::load()?),
        Some("--idle") => return mywm_config::session::exec_idle(&mywm_config::Config::load()?.idle),
        _ => {}
    }

    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let display = smithay::reexports::wayland_server::Display::new()?;
    let config = mywm_config::Config::load()?;
    tracing::info!("configuration: {}", mywm_config::Config::path().map_or("defaults".into(), |p| p.display().to_string()));
    // Inside another compositor or X server we run nested; on a bare seat we drive the hardware.
    // MYWM_BACKEND=winit|drm overrides the guess.
    let nested = match std::env::var("MYWM_BACKEND").as_deref() {
        Ok("winit") => true,
        Ok("drm" | "udev") => false,
        _ => std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some(),
    };
    // Nested, the host owns Super; MYWM_MODKEY=super keeps the configured modifiers.
    let remap_super = nested && std::env::var("MYWM_MODKEY").map_or(true, |v| !v.eq_ignore_ascii_case("super"));
    let mut state = State::new(&mut event_loop, display, config, remap_super);

    if nested {
        winit::init(&mut event_loop, &mut state)?;
    } else {
        udev::init(&mut event_loop, &mut state)?;
    }

    // SAFETY: single-threaded at this point; no other thread reads the environment.
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
        if !nested {
            // The desktop portals pick their backends by this name. `river` is what the
            // existing MyWM setup already configures xdg-desktop-portal-wlr for.
            if std::env::var_os("XDG_CURRENT_DESKTOP").is_none() {
                std::env::set_var("XDG_CURRENT_DESKTOP", "river");
            }
            std::env::set_var("XDG_SESSION_TYPE", "wayland");
        }
    }
    tracing::info!("listening on WAYLAND_DISPLAY={:?}", state.socket_name);

    // Shut down cleanly (removing the bar socket) on SIGTERM/SIGINT.
    let signals = Signals::new(&[Signal::SIGTERM, Signal::SIGINT])?;
    event_loop
        .handle()
        .insert_source(signals, |_, _, state| state.loop_signal.stop())
        .map_err(|e| e.error)?;

    xwayland::start(&mut state);

    if let Err(error) = ipc::init(&event_loop.handle(), &mut state) {
        tracing::error!("cannot start the bar socket: {error}");
    }

    // On hardware, idle handling (lock after a while, then monitors off) is swayidle's job.
    if !nested
        && let Ok(executable) = std::env::current_exe()
        && let Some(mut command) = mywm_config::session::idle_command(&state.config.idle, &executable.to_string_lossy())
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

    if let Some(cmd) = std::env::args().nth(1) {
        state.spawn_command(&["sh".into(), "-c".into(), cmd], false);
    }

    event_loop.run(None, &mut state, |state| {
        state.space.refresh();
        state.popups.cleanup();
        state.ipc_flush();
        let _ = state.display_handle.flush_clients();
    })?;
    Ok(())
}
