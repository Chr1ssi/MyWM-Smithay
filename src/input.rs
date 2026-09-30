use mywm_config::{Action, Modifiers};
use mywm_layout::{DragKind, Edges};
use smithay::{
    backend::session::Session,
    wayland::shell::wlr_layer::Layer,
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    input::{
        keyboard::{FilterResult, Keysym, ModifiersState, keysyms},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent},
    },
    output::Output,
    desktop::WindowSurfaceType,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::{
        compositor::RegionAttributes,
        pointer_constraints::{PointerConstraint, with_pointer_constraint},
    },
};

use crate::State;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

enum Constraint {
    Locked,
    /// Pointer stays within the region (the whole surface if `None`).
    Confined(Option<RegionAttributes>),
}

/// What a key press the compositor claims turns into.
enum Intercepted {
    Action(Action),
    Vt(i32),
    Modal(crate::screenshot::ModalKey),
}

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
        // Behind a lock nothing but locking again does anything.
        if self.session_lock.is_active() && !matches!(action, Action::Lock) {
            return;
        }
        tracing::debug!("action {action:?}");
        match action {
            Action::Terminal => self.spawn_command(&self.config.terminal.clone(), false),
            Action::Launcher => self.spawn_command(&self.config.launcher.clone(), true),
            Action::Program(index) => {
                if let Some(binding) = self.config.program_bindings.values().nth(index) {
                    self.spawn_command(&binding.command.clone(), false);
                }
            }
            Action::Reload => self.reload_config(),
            Action::Lock => self.start_locker(),
            Action::Wallpaper => self.open_wallpaper_picker(),
            Action::Overview => self.toggle_overview(),
            Action::ReleaseShortcuts => self.release_shortcuts(),
            Action::Screenshot => self.start_selection(),
            Action::ScreenshotScreen => self.screenshot_screen(),
            Action::ScreenshotWindow => self.screenshot_window(),
            Action::Close => {
                if let Some(m) = self.desktop.focused().and_then(|id| self.desktop.get(id)) {
                    m.close();
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
            Action::FocusOutput(d) => self.focus_output_direction(d),
            Action::MoveToOutput(d) => self.move_to_output_direction(d),
        }
    }

    pub fn process_input_event<B: InputBackend>(&mut self, event: InputEvent<B>, output: Option<&Output>) {
        self.idle_notifier_state.notify_activity(&self.seat);
        match event {
            InputEvent::Keyboard { event } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);
                let keyboard = self.seat.get_keyboard().unwrap();
                let action = keyboard.input::<Intercepted, _>(
                    self,
                    event.key_code(),
                    event.state(),
                    serial,
                    time,
                    |state, mods, handle| {
                        if event.state() != KeyState::Pressed {
                            return FilterResult::Forward;
                        }
                        // Ctrl+Alt+F<n> arrives as a dedicated keysym after xkb's processing.
                        let raw = handle.modified_sym().raw();
                        if (keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12).contains(&raw) {
                            return FilterResult::Intercept(Intercepted::Vt((raw - keysyms::KEY_XF86Switch_VT_1 + 1) as i32));
                        }
                        // While picking a screenshot region the keyboard belongs to the picker.
                        if state.selecting.is_some() || state.overview.is_some() {
                            use crate::screenshot::ModalKey;
                            return FilterResult::Intercept(Intercepted::Modal(match raw {
                                keysyms::KEY_Escape => ModalKey::Cancel,
                                keysyms::KEY_Return | keysyms::KEY_KP_Enter => ModalKey::Confirm,
                                keysyms::KEY_Left => ModalKey::Left,
                                keysyms::KEY_Right => ModalKey::Right,
                                keysyms::KEY_Up => ModalKey::Up,
                                keysyms::KEY_Down => ModalKey::Down,
                                _ => ModalKey::Ignore,
                            }));
                        }
                        // A locked session hands every other key to the locker.
                        if state.session_lock.is_active() {
                            return FilterResult::Forward;
                        }
                        // Unmodified symbol, so Shift+1 is still "1".
                        let action = handle.raw_syms().first().and_then(|sym| state.binding_for(mods, *sym));
                        // A client that took the shortcuts (a game, a VM) gets every key but the way out.
                        let action = action.filter(|a| !state.shortcuts_inhibited() || *a == Action::ReleaseShortcuts);
                        match action {
                            Some(action) => FilterResult::Intercept(Intercepted::Action(action)),
                            None => FilterResult::Forward,
                        }
                    },
                );
                match action {
                    Some(Intercepted::Action(action)) => self.run_action(action),
                    Some(Intercepted::Modal(key)) => {
                        if self.overview.is_some() {
                            self.overview_key(key);
                        } else {
                            self.selection_key(key);
                        }
                    }
                    Some(Intercepted::Vt(vt)) => {
                        if let Some(session) = &mut self.session
                            && let Err(error) = session.change_vt(vt)
                        {
                            tracing::warn!("cannot switch to VT {vt}: {error}");
                        }
                    }
                    None => {}
                }
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let Some(geo) = output.and_then(|o| self.space.output_geometry(o)) else { return };
                self.pointer_location = event.position_transformed(geo.size) + geo.loc.to_f64();
                self.pointer_moved(event.time_msec());
            }
            InputEvent::PointerMotion { event } => {
                let pointer = self.seat.get_pointer().unwrap();
                let focus = self.pointer_focus();
                // Games read raw deltas, also while the cursor is held in place.
                pointer.relative_motion(
                    self,
                    focus.clone(),
                    &RelativeMotionEvent {
                        delta: event.delta(),
                        delta_unaccel: event.delta_unaccel(),
                        utime: event.time(),
                    },
                );
                let mut target = self.pointer_location + event.delta();
                if let Some((surface, origin)) = &focus {
                    match self.active_constraint(surface) {
                        Some(Constraint::Locked) => return pointer.frame(self),
                        Some(Constraint::Confined(region)) => {
                            let inside = region.as_ref().is_none_or(|r| r.contains((target - *origin).to_i32_floor()));
                            if !inside {
                                target = self.pointer_location;
                            }
                        }
                        None => {}
                    }
                }
                let (x, y) = self.desktop.desk.clamp_to_monitors((target.x, target.y));
                self.pointer_location = (x, y).into();
                self.pointer_moved(event.time_msec());
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
        if self.overview.is_some() {
            return self.overview_button(button, state);
        }
        if self.selecting.is_some() {
            return self.selection_button(button, state);
        }
        let pointer = self.seat.get_pointer().unwrap();
        let serial = SERIAL_COUNTER.next_serial();
        if state == ButtonState::Released && self.desktop.drag.is_some() {
            self.end_drag();
            return;
        }
        if state == ButtonState::Pressed {
            let locked = self.session_lock.is_active();
            let mods = modifiers(&self.seat.get_keyboard().unwrap().modifier_state());
            if !locked && mods == self.pointer_modifiers && (button == BTN_LEFT || button == BTN_RIGHT) {
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
            if !locked {
                let clicked = self.pointer_focus().map(|(surface, _)| surface);
                self.note_click(clicked.as_ref());
            }
            let under = self.window_at(self.pointer_location).map(|(w, _)| w);
            if let Some(window) = under.filter(|_| self.layer_focus.is_none() && !locked) {
                self.space.raise_element(&window, true);
                if let Some(id) = self.desktop.windows.iter().find(|m| m.window == window).map(|m| m.id) {
                    self.focus_window(id);
                }
            }
        }
        pointer.button(self, &ButtonEvent { button, state, serial, time });
        pointer.frame(self);
    }

    fn pointer_moved(&mut self, time: u32) {
        // Only the outputs the cursor leaves and enters change.
        let current = self.space.output_under(self.pointer_location).next().cloned();
        if let Some(previous) = self.cursor_output.take().filter(|p| Some(p) != current.as_ref()) {
            self.queue_redraw_output(&previous);
        }
        if let Some(output) = &current {
            self.queue_redraw_output(output);
        }
        self.cursor_output = current;
        if self.overview.is_some() {
            self.overview_moved();
            return self.queue_redraw_all();
        }
        if self.selecting.is_some() {
            return self.selection_moved();
        }
        if self.desktop.drag.is_some() {
            self.update_drag();
        } else {
            self.pointer_motion(time);
        }
    }

    /// Move the pointer to `location` as if the user did.
    pub fn warp_pointer(&mut self, location: Point<f64, Logical>) {
        self.pointer_location = location;
        let time = self.start_time.elapsed().as_millis() as u32;
        self.pointer_motion(time);
    }

    /// The surface under the pointer and the global position of its origin.
    fn pointer_focus(&self) -> Option<(WlSurface, Point<f64, Logical>)> {
        if self.session_lock.is_active() {
            return self.lock_surface_at_pointer();
        }
        // Same order as drawn: overlays, popups and menus and fullscreen windows, panels, the
        // other windows, then wallpapers and the like.
        let window_hit = self.window_at(self.pointer_location);
        let surface_of = |(window, loc): &(smithay::desktop::Window, Point<i32, Logical>)| {
            window
                .surface_under(self.pointer_location - loc.to_f64(), WindowSurfaceType::ALL)
                .map(|(surface, origin)| (surface, (origin + *loc).to_f64()))
        };
        let in_front = window_hit.as_ref().filter(|(window, _)| {
            self.desktop.windows.iter().find(|m| &m.window == window).is_none_or(|m| m.shown_fullscreen)
        });
        self.layer_surface_at(&[Layer::Overlay])
            .or_else(|| in_front.and_then(surface_of))
            .or_else(|| self.layer_surface_at(&[Layer::Top]))
            .or_else(|| window_hit.as_ref().and_then(surface_of))
            .or_else(|| self.layer_surface_at(&[Layer::Bottom, Layer::Background]))
    }

    fn pointer_motion(&mut self, time: u32) {
        self.update_pointer_monitor();
        let pointer = self.seat.get_pointer().unwrap();
        let serial = SERIAL_COUNTER.next_serial();
        let under = self.pointer_focus();
        // Focus follows mouse, like MyWM on River, unless a panel holds the keyboard.
        if self.keyboard_layer().is_none()
            && let Some((window, _)) = self.window_at(self.pointer_location)
            && let Some(id) = self.desktop.windows.iter().find(|m| m.window == window).map(|m| m.id)
        {
            self.focus_window(id);
        }
        let surface = under.as_ref().map(|(s, _)| s.clone());
        if surface != self.pointer_focus_surface {
            let old = std::mem::replace(&mut self.pointer_focus_surface, surface.clone());
            self.pointer_focus_changed(old.as_ref(), surface.as_ref());
        }
        pointer.motion(self, under, &MotionEvent { location: self.pointer_location, serial, time });
        pointer.frame(self);
    }

    /// The pointer constraint of `surface`, if one is active.
    fn active_constraint(&self, surface: &WlSurface) -> Option<Constraint> {
        let pointer = self.seat.get_pointer()?;
        with_pointer_constraint(surface, &pointer, |constraint| {
            let constraint = constraint.filter(|c| c.is_active())?;
            Some(match &*constraint {
                PointerConstraint::Locked(_) => Constraint::Locked,
                PointerConstraint::Confined(_) => Constraint::Confined(constraint.region().cloned()),
            })
        })
    }
}
