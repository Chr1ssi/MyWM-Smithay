use std::time::Duration;

use smithay::{
    backend::{
        renderer::{
            damage::OutputDamageTracker, element::surface::WaylandSurfaceRenderElement,
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent},
    },
    desktop::space::render_output,
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Rectangle, Transform},
};

use crate::State;

pub fn init(
    event_loop: &mut EventLoop<State>,
    state: &mut State,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut backend, winit) = winit::init::<GlesRenderer>()?;

    let mode = Mode { size: backend.window_size(), refresh: 60_000 };
    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "MyWM".into(),
            model: "Winit".into(),
        },
    );
    let _global = output.create_global::<State>(&state.display_handle);
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));

    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    event_loop
        .handle()
        .insert_source(winit, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => {
                output.change_current_state(Some(Mode { size, refresh: 60_000 }), None, None, None);
                state.relayout();
            }
            WinitEvent::Input(event) => state.process_input_event(event, &output),
            WinitEvent::Redraw => {
                let size = backend.window_size();
                let damage = Rectangle::from_size(size);
                {
                    let (renderer, mut framebuffer) = backend.bind().expect("bind");
                    render_output::<_, WaylandSurfaceRenderElement<GlesRenderer>, _, _>(
                        &output,
                        renderer,
                        &mut framebuffer,
                        1.0,
                        0,
                        [&state.space],
                        &[],
                        &mut damage_tracker,
                        [0.08, 0.08, 0.1, 1.0],
                    )
                    .expect("render");
                }
                backend.submit(Some(&[damage])).expect("submit");

                let elapsed = state.start_time.elapsed();
                for window in state.space.elements() {
                    window.send_frame(&output, elapsed, Some(Duration::ZERO), |_, _| Some(output.clone()));
                }
                backend.window().request_redraw();
            }
            WinitEvent::CloseRequested => state.loop_signal.stop(),
            _ => {}
        })
        .map_err(|e| e.error)?;
    Ok(())
}
