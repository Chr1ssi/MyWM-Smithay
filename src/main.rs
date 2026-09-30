//! MyWM compositor entry point: DRM/libinput on hardware, or nested in a window for development.

mod cursor;
mod desktop;
mod handlers;
mod input;
mod ipc;
mod monitors;
mod protocols;
mod udev;
mod state;
mod winit;

use calloop::{
    EventLoop,
    signals::{Signal, Signals},
};
use tracing_subscriber::EnvFilter;

pub use state::State;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let display = smithay::reexports::wayland_server::Display::new()?;
    let config = mywm_config::Config::load()?;
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
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name) };
    tracing::info!("listening on WAYLAND_DISPLAY={:?}", state.socket_name);

    // Shut down cleanly (removing the bar socket) on SIGTERM/SIGINT.
    let signals = Signals::new(&[Signal::SIGTERM, Signal::SIGINT])?;
    event_loop
        .handle()
        .insert_source(signals, |_, _, state| state.loop_signal.stop())
        .map_err(|e| e.error)?;

    if let Err(error) = ipc::init(&event_loop.handle(), &mut state) {
        tracing::error!("cannot start the bar socket: {error}");
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
