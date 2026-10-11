//! Content-update barriers of `wp-fifo-v1` and `wp-commit-timing-v1`.
//!
//! Smithay holds back a commit that waits on a fifo barrier or names a presentation time; the
//! compositor releases them in step with what it shows. A visible surface is released with every frame
//! of its output (see `send_frames`), a hidden one (covered, on a hidden workspace) by the fallback timer
//! (see `send_overdue_frames`), so a game slows down where nobody sees it but never stalls for good.
use std::{cell::RefCell, sync::Mutex, time::Duration};

use smithay::{
    desktop::utils::surface_primary_scanout_output,
    output::Output,
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_server::{Client, Resource, protocol::wl_surface::WlSurface},
    },
    utils::{Clock, Monotonic, Time},
    wayland::{
        commit_timing::{CommitTimerBarrierStateUserData, CommitTimerStateUserData},
        compositor::{CompositorHandler, SurfaceData, add_pre_commit_hook, with_states},
        fifo::FifoBarrierCachedState,
    },
};

use crate::State;

/// Clients with a released barrier; their held-back commits are applied by `State::apply_released`.
pub type Released = RefCell<Vec<Client>>;

/// When the surface's barriers were last released, in the compositor's frame time.
#[derive(Default)]
struct LastRelease(Mutex<Option<Duration>>);

/// The earliest time a commit released now can be shown on `output`: the frame being released is
/// rendered for a vblank at least one refresh interval away, so content never appears before its time.
pub fn next_frame_deadline(output: &Output) -> Time<Monotonic> {
    Clock::<Monotonic>::new().now() + refresh_interval(output)
}

fn refresh_interval(output: &Output) -> Duration {
    let refresh = output.current_mode().map_or(60_000, |mode| mode.refresh.max(1000)) as u64;
    Duration::from_nanos(1_000_000_000_000 / refresh)
}

/// The time up to which hidden surfaces get their timed commits: they are not shown, so only what is due.
pub fn now() -> Time<Monotonic> {
    Clock::<Monotonic>::new().now()
}

/// Release what `surface` waits on for a frame shown no earlier than `deadline`; `at` is the frame time.
pub fn release(surface: &WlSurface, states: &SurfaceData, deadline: Time<Monotonic>, at: Duration, released: &Released) {
    states.data_map.insert_if_missing_threadsafe(LastRelease::default);
    if let Some(last) = states.data_map.get::<LastRelease>()
        && let Ok(mut last) = last.0.lock()
    {
        *last = Some(at);
    }
    let fifo = states.cached_state.get::<FifoBarrierCachedState>().current().barrier.take();
    let mut any = fifo.is_some();
    if let Some(barrier) = fifo {
        barrier.signal();
    }
    if let Some(timer) = states.data_map.get::<CommitTimerBarrierStateUserData>()
        && let Ok(mut timer) = timer.lock()
    {
        any |= timer.signal_until(deadline);
    }
    if any && let Some(client) = surface.client() {
        let mut released = released.borrow_mut();
        if !released.contains(&client) {
            released.push(client);
        }
    }
}

/// Like `release`, but only if nothing released the surface for `throttle`: the same rule as for overdue
/// frame callbacks, so a surface no output redraws (hidden, or on a monitor that is off) keeps going
/// while a visible one is never released twice for one frame.
pub fn release_overdue(
    surface: &WlSurface,
    states: &SurfaceData,
    deadline: Time<Monotonic>,
    at: Duration,
    throttle: Duration,
    released: &Released,
) {
    let due = states
        .data_map
        .get::<LastRelease>()
        .and_then(|last| *last.0.lock().ok()?)
        .is_none_or(|last| at.saturating_sub(last) >= throttle);
    if due {
        release(surface, states, deadline, at, released);
    }
}

impl State {
    /// A commit that names a presentation time is released by a frame of its output. Make sure one comes
    /// in time even if nothing else redraws that output, or the commit would wait for the fallback timer.
    pub fn wake_for_timed_commits(&mut self, surface: &WlSurface) {
        add_pre_commit_hook::<State, _>(surface, |state, _, surface| {
            let target = with_states(surface, |states| {
                states.data_map.get::<CommitTimerStateUserData>().and_then(|timer| timer.borrow().timestamp)
            });
            if let Some(target) = target {
                state.redraw_before(surface.clone(), target.into());
            }
        });
    }

    /// Redraw `surface`'s output one refresh interval of the fastest output before `target`: that frame
    /// releases the commit (see `next_frame_deadline`), unless an earlier one already did.
    fn redraw_before(&mut self, surface: WlSurface, target: Time<Monotonic>) {
        let early = self.outputs.iter().map(|e| refresh_interval(&e.output)).min().unwrap_or_default();
        let delay = Time::elapsed(&now(), target).saturating_sub(early);
        let registered = self.loop_handle.insert_source(Timer::from_duration(delay), move |_, _, state| {
            let output = surface.is_alive().then(|| {
                with_states(&surface, |states| {
                    let waiting = states
                        .data_map
                        .get::<CommitTimerBarrierStateUserData>()
                        .is_some_and(|timer| timer.lock().is_ok_and(|timer| timer.next_deadline().is_some()));
                    // A hidden surface has no output to redraw; the fallback timer releases it.
                    waiting.then(|| surface_primary_scanout_output(&surface, states)).flatten()
                })
            });
            if let Some(output) = output.flatten() {
                state.queue_redraw_output(&output);
            }
            TimeoutAction::Drop
        });
        if let Err(error) = registered {
            tracing::warn!("cannot schedule the frame for a timed commit: {error}");
        }
    }

    /// Apply the commits that waited on the barriers just released, once the current event is handled:
    /// the barriers are released while an output renders, when the DRM backend is taken out of the state
    /// and the redraw a newly applied commit asks for would be lost.
    pub fn apply_released(&mut self, clients: Vec<Client>) {
        if clients.is_empty() {
            return;
        }
        self.loop_handle.insert_idle(move |state| {
            let display = state.display_handle.clone();
            for client in clients {
                state.client_compositor_state(&client).blocker_cleared(state, &display);
            }
        });
    }
}
