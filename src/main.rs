//! MyWM compositor entry point (M0: nested winit backend for development).

mod handlers;
mod input;
mod state;
mod winit;

use smithay::reexports::calloop::EventLoop;
use tracing_subscriber::EnvFilter;

pub use state::State;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let display = smithay::reexports::wayland_server::Display::new()?;
    let mut state = State::new(&mut event_loop, display);

    winit::init(&mut event_loop, &mut state)?;

    // SAFETY: single-threaded at this point; no other thread reads the environment.
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name) };
    tracing::info!("listening on WAYLAND_DISPLAY={:?}", state.socket_name);

    if let Some(cmd) = std::env::args().nth(1) {
        state.spawn(&cmd);
    }

    event_loop.run(None, &mut state, |state| {
        state.space.refresh();
        state.popups.cleanup();
        let _ = state.display_handle.flush_clients();
    })?;
    Ok(())
}
