use mywm_config::{Action, Modifiers};
use mywm_layout::{DragKind, Edges};
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    },
    input::{
        keyboard::{FilterResult, Keysym, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    output::Output,
    utils::SERIAL_COUNTER,
};

use crate::State;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

fn modifiers(state: &ModifiersState) -> Modifiers {
    Modifiers { logo: state.logo, shift: state.shift, ctrl: state.ctrl, alt: state.alt }
}

impl State {
    fn binding_for(&self, mods: &ModifiersState, sym: Keysym) -> Option<Action> {
        let mods = modifiers(mods);
        self.bindings
            .iter()
            .find(|b| b.symbol == sym.raw() && b.modifiers == mods)
            .map(|b| b.action)
    }

    pub fn run_action(&mut self, action: Action) {
        match action {
            Action::Terminal => self.spawn_command(&self.config.terminal.clone(), false),
            Action::Launcher => self.spawn_command(&self.config.launcher.clone(), true),
            Action::Program(index) => {
                if let Some(binding) = self.config.program_bindings.values().nth(index) {
                    self.spawn_command(&binding.command.clone(), false);
                }
            }
            Action::Reload => self.reload_config(),
            Action::Wallpaper | Action::Lock => {
                tracing::warn!("{action:?} is not implemented in the Smithay compositor yet");
            }
            Action::Close => {
                if let Some(m) = self.desktop.focused().and_then(|id| self.desktop.get(id))
                    && let Some(top) = m.window.toplevel()
                {
                    top.send_close();
                }
            }
            Action::Exit => self.loop_signal.stop(),
            Action::Focus(d) => self.focus_step(d),
            Action::Move(d) => self.move_column(d),
            Action::Workspace(n) => self.select_workspace(n),
            Action::MoveToWorkspace(n) => self.move_to_workspace(n),
            Action::WorkspaceRelative(d) => self.cycle_workspace(d),
            Action::MoveToWorkspaceRelative(d) => self.move_to_workspace_relative(d),
            Action::NewWorkspace => self.new_workspace(),
            Action::MoveToNewWorkspace => self.move_to_new_workspace(),
            Action::ToggleFloating => self.toggle_floating(),
            Action::ToggleFullscreen => self.toggle_fullscreen(),
            Action::ResizeColumn(p) => self.resize_column(p),
            Action::ToggleScratchpad => self.toggle_scratchpad(),
            Action::MoveToScratchpad => self.move_to_scratchpad(),
            // Multi-monitor actions arrive with the DRM backend.
            Action::FocusOutput(_) | Action::MoveToOutput(_) => {}
        }
    }

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
                    |state, mods, handle| {
                        if event.state() != KeyState::Pressed {
                            return FilterResult::Forward;
                        }
                        // Unmodified symbol, so Shift+1 is still "1".
                        match handle.raw_syms().first().and_then(|sym| state.binding_for(mods, *sym)) {
                            Some(action) => FilterResult::Intercept(action),
                            None => FilterResult::Forward,
                        }
                    },
                );
                if let Some(action) = action {
                    self.run_action(action);
                }
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let Some(geo) = self.space.output_geometry(output) else { return };
                self.pointer_location = event.position_transformed(geo.size) + geo.loc.to_f64();
                if self.desktop.drag.is_some() {
                    self.update_drag();
                } else {
                    self.pointer_motion(event.time_msec());
                }
            }
            InputEvent::PointerButton { event } => self.pointer_button(
                event.button_code(),
                event.state(),
                event.time_msec(),
            ),
            InputEvent::PointerAxis { event } => {
                let source = event.source();
                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let amount = event
                        .amount(axis)
                        .unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0);
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

    fn pointer_button(&mut self, button: u32, state: ButtonState, time: u32) {
        let pointer = self.seat.get_pointer().unwrap();
        let serial = SERIAL_COUNTER.next_serial();
        if state == ButtonState::Released && self.desktop.drag.is_some() {
            self.end_drag();
            return;
        }
        if state == ButtonState::Pressed {
            let mods = modifiers(&self.seat.get_keyboard().unwrap().modifier_state());
            if mods == self.pointer_modifiers && (button == BTN_LEFT || button == BTN_RIGHT) {
                let kind = if button == BTN_LEFT {
                    DragKind::Move
                } else {
                    DragKind::Resize(Edges { right: true, bottom: true, ..Default::default() })
                };
                self.begin_drag(kind);
                if self.desktop.drag.is_some() {
                    return;
                }
            }
            let under = self.space.element_under(self.pointer_location).map(|(w, _)| w.clone());
            if let Some(window) = under {
                self.space.raise_element(&window, true);
                if let Some(id) = self.desktop.windows.iter().find(|m| m.window == window).map(|m| m.id) {
                    self.focus_window(id);
                }
            }
        }
        pointer.button(self, &ButtonEvent { button, state, serial, time });
        pointer.frame(self);
    }

    fn pointer_motion(&mut self, time: u32) {
        let pointer = self.seat.get_pointer().unwrap();
        let serial = SERIAL_COUNTER.next_serial();
        let under_window = self.space.element_under(self.pointer_location).map(|(w, l)| (w.clone(), l));
        let under = under_window.as_ref().and_then(|(window, loc)| {
            window
                .surface_under(
                    self.pointer_location - loc.to_f64(),
                    smithay::desktop::WindowSurfaceType::ALL,
                )
                .map(|(s, p)| (s, (p + *loc).to_f64()))
        });
        // Focus follows mouse, like MyWM on River.
        if let Some((window, _)) = &under_window
            && let Some(id) = self.desktop.windows.iter().find(|m| &m.window == window).map(|m| m.id)
        {
            self.focus_window(id);
        }
        pointer.motion(self, under, &MotionEvent { location: self.pointer_location, serial, time });
        pointer.frame(self);
    }
}
