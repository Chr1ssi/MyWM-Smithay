//! Nested backend for development: renders into a window of the host compositor or X server.
use smithay::{
    backend::{
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Rectangle, Transform},
};

use crate::State;

fn new_output(name: &str, model: &str, mode: Mode, state: &State) -> Output {
    let output = Output::new(
        name.to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "MyWM".into(),
            model: model.into(),
        },
    );
    output.create_global::<State>(&state.display_handle);
    output.set_preferred(mode);
    output
}

pub fn init(event_loop: &mut EventLoop<State>, state: &mut State) -> Result<(), Box<dyn std::error::Error>> {
    let (mut backend, winit) = winit::init::<GlesRenderer>()?;

    let mode = Mode { size: backend.window_size(), refresh: 60_000 };
    let output = new_output("winit", "Winit", mode, state);
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
    state.add_output(output.clone(), Some((0, 0).into()));

    // Extra outputs without a picture, to exercise multi-monitor logic (workspaces, focus,
    // the bar protocol) without a second screen: MYWM_VIRTUAL_OUTPUTS=<count>.
    let virtual_count: usize = std::env::var("MYWM_VIRTUAL_OUTPUTS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    for index in 1..=virtual_count {
        let mode = Mode { size: (1280, 800).into(), refresh: 60_000 };
        let extra = new_output(&format!("virtual-{index}"), "Virtual", mode, state);
        extra.change_current_state(Some(mode), Some(Transform::Normal), None, None);
        state.add_output(extra, None);
    }

    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    event_loop
        .handle()
        .insert_source(winit, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => {
                output.change_current_state(Some(Mode { size, refresh: 60_000 }), None, None, None);
                state.refresh();
            }
            WinitEvent::Input(event) => state.process_input_event(event, Some(&output)),
            WinitEvent::Redraw => {
                let size = backend.window_size();
                let damage = Rectangle::from_size(size);
                let elements;
                let had_damage;
                {
                    let age = backend.buffer_age().unwrap_or(0);
                    let (renderer, mut framebuffer) = backend.bind().expect("bind");
                    elements = state.output_elements(renderer, &output);
                    let clear = state.clear_color();
                    let result = damage_tracker.render_output(renderer, &mut framebuffer, age, &elements, clear).expect("render");
                    had_damage = result.damage.is_some_and(|d| !d.is_empty());
                    state.update_primary_outputs(&output, &result.states);
                }
                // Under Xvfb, presenting fails now and then (EGL cannot recreate the window surface while the
                // old one exists); the old surface stays usable, so the next frame presents again.
                if let Err(error) = backend.submit(Some(&[damage])) {
                    tracing::warn!("cannot present the nested window: {error}");
                }

                // Captures render offscreen, which must not happen while the window is being drawn.
                state.fulfill_screencopy(backend.renderer(), &output, &elements, true);
                state.fulfill_image_captures(backend.renderer(), &output, &elements, had_damage);
                state.fulfill_screenshots(backend.renderer(), &output, &elements);
                let outputs: Vec<_> = state.outputs.iter().map(|e| e.output.clone()).collect();
                // Virtual outputs have no window of their own, but can still be captured.
                for other in outputs.iter().filter(|o| *o != &output) {
                    if state.pending_copies.iter().any(|p| p.output() == other) || state.has_image_captures_for(other) || state.pending_shots.iter().any(|p| p.output() == other) {
                        let renderer = backend.renderer();
                        let elements = state.output_elements(renderer, other);
                        state.fulfill_screencopy(renderer, other, &elements, true);
                        state.fulfill_image_captures(renderer, other, &elements, true);
                        state.fulfill_screenshots(renderer, other, &elements);
                    }
                }
                for other in &outputs {
                    if other == &output {
                        state.send_frames(other);
                    } else {
                        state.send_frames_unpresented(other);
                    }
                }
                for output in &outputs {
                    // Outputs without a picture have nothing to hide: count them as showing the lock.
                    state.note_locked_frame(output);
                }
                backend.window().request_redraw();
            }
            WinitEvent::CloseRequested => state.loop_signal.stop(),
            _ => {}
        })
        .map_err(|e| e.error)?;
    Ok(())
}
