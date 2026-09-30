use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    },
    input::{
        keyboard::{keysyms, FilterResult},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    output::Output,
    utils::SERIAL_COUNTER,
};

use crate::State;

enum Action {
    Spawn,
    Close,
    Quit,
}

impl State {
    pub fn process_input_event<B: InputBackend>(&mut self, event: InputEvent<B>, output: &Output) {
        match event {
            InputEvent::Keyboard { event } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);
                let keyboard = self.seat.get_keyboard().unwrap();
                let action = keyboard.input::<Action, _>(
                    self,
                    event.key_code(),
                    event.state(),
                    serial,
                    time,
                    |_, mods, handle| {
                        // Alt as modifier while nested (the host compositor owns Super).
                        if event.state() == KeyState::Pressed && mods.alt {
                            match handle.modified_sym().raw() {
                                keysyms::KEY_Return => return FilterResult::Intercept(Action::Spawn),
                                keysyms::KEY_q => return FilterResult::Intercept(Action::Close),
                                keysyms::KEY_e if mods.shift => return FilterResult::Intercept(Action::Quit),
                                _ => {}
                            }
                        }
                        FilterResult::Forward
                    },
                );
                match action {
                    Some(Action::Spawn) => {
                        let term = std::env::var("MYWM_TERMINAL").unwrap_or_else(|_| "kitty".into());
                        self.spawn(&term);
                    }
                    Some(Action::Close) => {
                        if let Some(top) = keyboard
                            .current_focus()
                            .and_then(|s| self.space.elements().find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == &s)))
                            .and_then(|w| w.toplevel())
                        {
                            top.send_close();
                        }
                    }
                    Some(Action::Quit) => self.loop_signal.stop(),
                    None => {}
                }
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let Some(geo) = self.space.output_geometry(output) else { return };
                let pos = event.position_transformed(geo.size) + geo.loc.to_f64();
                self.pointer_location = pos;
                self.pointer_motion(event.time_msec());
            }
            InputEvent::PointerButton { event } => {
                let pointer = self.seat.get_pointer().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                if event.state() == ButtonState::Pressed && !pointer.is_grabbed() {
                    let under = self
                        .space
                        .element_under(self.pointer_location)
                        .map(|(w, _)| w.clone());
                    if let Some(window) = under {
                        self.space.raise_element(&window, true);
                        self.set_keyboard_focus(window.toplevel().map(|t| t.wl_surface().clone()));
                    }
                }
                pointer.button(
                    self,
                    &ButtonEvent {
                        button: event.button_code(),
                        state: event.state(),
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event } => {
                let source = event.source();
                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let amount = event.amount(axis).unwrap_or_else(|| {
                        event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0
                    });
                    if amount != 0.0 {
                        frame = frame.value(axis, amount);
                        if let Some(v120) = event.amount_v120(axis) {
                            frame = frame.v120(axis, v120 as i32);
                        }
                    } else if source == AxisSource::Finger {
                        frame = frame.stop(axis);
                    }
                }
                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }

    fn pointer_motion(&mut self, time: u32) {
        let pointer = self.seat.get_pointer().unwrap();
        let serial = SERIAL_COUNTER.next_serial();
        let under = self
            .space
            .element_under(self.pointer_location)
            .and_then(|(window, loc)| {
                window
                    .surface_under(self.pointer_location - loc.to_f64(), smithay::desktop::WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + loc).to_f64()))
            });
        // Focus follows mouse, like MyWM on River.
        if let Some((surface, _)) = &under
            && self.seat.get_keyboard().and_then(|k| k.current_focus()).as_ref() != Some(surface)
        {
            self.set_keyboard_focus(Some(surface.clone()));
        }
        pointer.motion(
            self,
            under,
            &MotionEvent { location: self.pointer_location, serial, time },
        );
        pointer.frame(self);
    }
}
