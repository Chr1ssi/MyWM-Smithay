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

/// What is under the pointer: a client's surface with the global position of its origin, or the built-in shell.
enum PointerTarget {
    Surface(WlSurface, Point<f64, Logical>),
    Shell(crate::shell::ShellHit),
}

/// What a key press the compositor claims turns into.
enum Intercepted {
    Action(Action),
    Vt(i32),
    Modal(crate::screenshot::ModalKey),
    /// A key for the built-in shell's power menu.
    Shell(Keysym),
    /// The release of a key whose press was intercepted.
    Release,
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
        // Behind a lock nothing but locking again and the program bindings that allow it does anything.
        if self.session_lock.is_active() && !self.config.allowed_when_locked(action) {
            return;
        }
        tracing::debug!("action {action:?}");
        match action {
            Action::Terminal => self.spawn_command(&self.config.terminal.clone(), false),
            Action::Launcher => self.spawn_command(&self.config.launcher.clone(), true),
            Action::Program(index) => {
                let Some(binding) = self.config.program_bindings.values().nth(index) else { return };
                // Run or raise: an open window of the app is shown instead of starting another.
                let open = binding.app_id.as_deref().and_then(|pattern| {
                    self.next_window_where(|m| m.app_id.as_deref().is_some_and(|id| mywm_config::glob(pattern, id)))
                });
                match open {
                    Some(id) => self.show_window(id),
                    None => self.spawn_command(&binding.command.clone(), false),
                }
            }
            Action::ToggleSticky => self.toggle_sticky(),
            Action::CastWindow => {
                let target = self.desktop.focused().and_then(|id| self.desktop.get(id)).and_then(|m| m.foreign.as_ref());
                if let Some(handle) = target {
                    self.set_cast_target(mywm_ipc::CastTarget::Window(handle.identifier()));
                }
            }
            Action::CastMonitor => {
                let output = self.outputs.get(self.desktop.focused_monitor).map(|e| e.output.name());
                if let Some(name) = output {
                    self.set_cast_target(mywm_ipc::CastTarget::Monitor(name));
                }
            }
            Action::CastClear => self.set_cast_target(mywm_ipc::CastTarget::Nothing),
            Action::FocusUrgent => {
                if let Some(id) = self.next_window_where(|m| m.urgent) {
                    self.show_window(id);
                }
            }
            Action::Reload => self.reload_config(),
            Action::Lock => self.start_locker(),
            Action::Wallpaper => self.open_wallpaper_picker(),
            Action::Overview => self.toggle_overview(),
            Action::ColorPicker => {
                if self.picking.is_some() {
                    self.cancel_color_picker();
                } else {
                    self.start_color_picker();
                }
            }
            Action::ReleaseShortcuts => self.release_shortcuts(),
            Action::Screenshot => self.start_selection(),
            Action::ScreenshotScreen => self.screenshot_screen(),
            Action::ScreenshotWindow => self.screenshot_window(),
            Action::Close => {
                if let Some(m) = self.desktop.focused().and_then(|id| self.desktop.get(id)) {
                    m.close();
                }
            }
            Action::Exit => self.request_exit(),
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
        self.input_activity = true;
        match event {
            InputEvent::Keyboard { event } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);
                let keyboard = self.seat.get_keyboard().unwrap();
                let keycode = event.key_code();
                let action = keyboard.input::<Intercepted, _>(
                    self,
                    keycode,
                    event.state(),
                    serial,
                    time,
                    |state, mods, handle| {
                        if event.state() != KeyState::Pressed {
                            // The client never saw the press, and a lone release still acts: a browser
                            // toggles a video on Space's release after Super+Space opened the launcher.
                            return match state.suppressed_keys.iter().position(|k| *k == keycode) {
                                Some(index) => {
                                    state.suppressed_keys.swap_remove(index);
                                    FilterResult::Intercept(Intercepted::Release)
                                }
                                None => FilterResult::Forward,
                            };
                        }
                        // Ctrl+Alt+F<n> arrives as a dedicated keysym after xkb's processing.
                        let raw = handle.modified_sym().raw();
                        if (keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12).contains(&raw) {
                            return FilterResult::Intercept(Intercepted::Vt((raw - keysyms::KEY_XF86Switch_VT_1 + 1) as i32));
                        }
                        // While picking a screenshot region or a color the keyboard belongs to the picker.
                        if state.selecting.is_some() || state.picking.is_some() || state.overview_open() {
                            use crate::screenshot::ModalKey;
                            return FilterResult::Intercept(Intercepted::Modal(match raw {
                                keysyms::KEY_Escape => ModalKey::Cancel,
                                keysyms::KEY_Return | keysyms::KEY_KP_Enter => ModalKey::Confirm,
                                keysyms::KEY_Left => ModalKey::Left,
                                keysyms::KEY_Right => ModalKey::Right,
                                keysyms::KEY_Up => ModalKey::Up,
                                keysyms::KEY_Down => ModalKey::Down,
                                keysyms::KEY_d | keysyms::KEY_D => ModalKey::Dynamic,
                                keysyms::KEY_1..=keysyms::KEY_9 => ModalKey::Digit((raw - keysyms::KEY_1 + 1) as usize),
                                keysyms::KEY_q | keysyms::KEY_Q => ModalKey::Close,
                                _ => ModalKey::Ignore,
                            }));
                        }
                        // Unmodified symbol, so Shift+1 is still "1".
                        let action = handle.raw_syms().first().and_then(|sym| state.binding_for(mods, *sym));
                        // A locked session hands every key to the locker but the program bindings that allow it.
                        if state.session_lock.is_active() {
                            let allowed = |a: &Action| matches!(a, Action::Program(_)) && state.config.allowed_when_locked(*a);
                            return match action.filter(allowed) {
                                Some(action) => FilterResult::Intercept(Intercepted::Action(action)),
                                None => FilterResult::Forward,
                            };
                        }
                        // A client that took the shortcuts (a game, a VM) gets every key but the way out.
                        let action = action.filter(|a| !state.shortcuts_inhibited() || *a == Action::ReleaseShortcuts);
                        match action {
                            Some(action) => FilterResult::Intercept(Intercepted::Action(action)),
                            // The power menu takes every other key, as the exclusive panel it was did.
                            None if state.shell_takes_keyboard() => FilterResult::Intercept(Intercepted::Shell(handle.modified_sym())),
                            None => FilterResult::Forward,
                        }
                    },
                );
                // A new press settles the key either way: a release lost to a VT switch must not
                // swallow the release of a later, forwarded press.
                if event.state() == KeyState::Pressed {
                    self.suppressed_keys.retain(|k| *k != keycode);
                    if action.is_some() {
                        self.suppressed_keys.push(keycode);
                    }
                }
                match action {
                    Some(Intercepted::Action(action)) => self.run_action(action),
                    Some(Intercepted::Modal(key)) => {
                        if self.overview_open() {
                            self.overview_key(key);
                        } else if self.picking.is_some() {
                            self.picker_key(key);
                        } else {
                            self.selection_key(key);
                        }
                    }
                    Some(Intercepted::Shell(sym)) => self.shell_key(sym),
                    Some(Intercepted::Vt(vt)) => {
                        if let Some(session) = &mut self.session
                            && let Err(error) = session.change_vt(vt)
                        {
                            tracing::warn!("cannot switch to VT {vt}: {error}");
                        }
                    }
                    Some(Intercepted::Release) | None => {}
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
                let pointer = self.seat.get_pointer().unwrap();
                if !pointer.is_grabbed()
                    && let Some(hit) = self.shell_at_pointer()
                {
                    // Notches as a QML `MouseArea` counts them (positive: away from the user); a touchpad's
                    // continuous scrolling in the same units as the wheel turns into it (15 per notch).
                    let notches = event
                        .amount_v120(Axis::Vertical)
                        .map(|v120| v120 / 120.0)
                        .or_else(|| event.amount(Axis::Vertical).map(|amount| amount / 15.0))
                        .unwrap_or(0.0);
                    if notches != 0.0 {
                        self.shell_scroll(hit, -notches as f32);
                    }
                    return;
                }
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
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }

    fn pointer_button(&mut self, button: u32, state: ButtonState, time: u32) {
        if self.overview_open() {
            return self.overview_button(button, state);
        }
        if self.selecting.is_some() {
            return self.selection_button(button, state);
        }
        if self.picking.is_some() {
            return self.picker_button(button, state);
        }
        let pointer = self.seat.get_pointer().unwrap();
        // The shell's clicks: from a press on it until that button's release, unless a client holds the pointer.
        if self.shell_holds_pointer() || (state == ButtonState::Pressed && !pointer.is_grabbed() && self.shell_at_pointer().is_some()) {
            let hit = self.shell_at_pointer();
            self.shell_button(button, state == ButtonState::Pressed, hit);
            if !self.shell_holds_pointer() {
                self.refresh_pointer_focus();
            }
            return;
        }
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
                self.center_pressed_elsewhere();
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
        // Only the outputs the cursor leaves and enters change, and nothing moves on them while a client
        // (a game) hides the cursor; an empty frame there would hold back the game's next one.
        let current = self.space.output_under(self.pointer_location).next().cloned();
        let previous = self.cursor_output.take().filter(|p| Some(p) != current.as_ref());
        if !matches!(self.cursor_status, smithay::input::pointer::CursorImageStatus::Hidden) || self.picking.is_some() {
            if let Some(previous) = &previous {
                self.queue_redraw_output(previous);
            }
            if let Some(output) = &current {
                self.queue_redraw_output(output);
            }
        }
        self.cursor_output = current;
        if self.overview_open() {
            self.overview_moved();
            return self.queue_redraw_all();
        }
        if self.selecting.is_some() {
            return self.selection_moved();
        }
        // The cursor's output redraws (above) and takes a new sample for the magnifier.
        if self.picking.is_some() {
            return;
        }
        if self.desktop.drag.is_some() {
            self.update_drag();
        } else {
            self.pointer_motion(time);
        }
    }

    /// Tell idle clients of the input since the last call: each one's timer restarts once per event loop
    /// iteration, and a resume reaches it before the clients are flushed.
    pub fn notify_input_activity(&mut self) {
        if std::mem::take(&mut self.input_activity) {
            self.idle_notifier_state.notify_activity(&self.seat);
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
        self.pointer_focus_over(self.window_at(self.pointer_location).as_ref())
    }

    /// `pointer_focus` with the window under the pointer (`window_at`) already looked up. A client gets no
    /// focus while the pointer is over the shell or a button pressed on it is still down.
    fn pointer_focus_over(
        &self,
        window_hit: Option<&(smithay::desktop::Window, Point<i32, Logical>)>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        if self.shell_holds_pointer() && !self.session_lock.is_active() {
            return None;
        }
        match self.pointer_target_over(window_hit)? {
            PointerTarget::Surface(surface, origin) => Some((surface, origin)),
            PointerTarget::Shell(_) => None,
        }
    }

    /// What of the shell is under the pointer, if it is the shell's.
    fn shell_at_pointer(&self) -> Option<crate::shell::ShellHit> {
        match self.pointer_target_over(self.window_at(self.pointer_location).as_ref())? {
            PointerTarget::Shell(hit) => Some(hit),
            PointerTarget::Surface(..) => None,
        }
    }

    fn pointer_target_over(&self, window_hit: Option<&(smithay::desktop::Window, Point<i32, Logical>)>) -> Option<PointerTarget> {
        if self.session_lock.is_active() {
            return self.lock_surface_at_pointer().map(|(surface, origin)| PointerTarget::Surface(surface, origin));
        }
        // Same order as drawn: the power menu, overlays, the notices, popups and menus and fullscreen
        // windows, panels, the bar, the other windows, then wallpapers and the like.
        let surface_of = |(window, loc): &(smithay::desktop::Window, Point<i32, Logical>)| {
            window
                .surface_under(self.pointer_location - loc.to_f64(), WindowSurfaceType::ALL)
                .map(|(surface, origin)| (surface, (origin + *loc).to_f64()))
        };
        let in_front = window_hit.filter(|(window, _)| {
            self.desktop.windows.iter().find(|m| &m.window == window).is_none_or(|m| m.shown_fullscreen)
        });
        let surface = |hit: Option<(WlSurface, Point<f64, Logical>)>| hit.map(|(s, origin)| PointerTarget::Surface(s, origin));
        self.shell_power_at_pointer()
            .map(PointerTarget::Shell)
            .or_else(|| surface(self.layer_surface_at(&[Layer::Overlay])))
            .or_else(|| self.shell_notices_at_pointer().map(PointerTarget::Shell))
            .or_else(|| surface(in_front.and_then(surface_of)))
            .or_else(|| surface(self.layer_surface_at(&[Layer::Top])))
            .or_else(|| self.shell_bar_at_pointer().map(PointerTarget::Shell))
            .or_else(|| surface(window_hit.and_then(surface_of)))
            .or_else(|| surface(self.layer_surface_at(&[Layer::Bottom, Layer::Background])))
    }

    fn pointer_motion(&mut self, time: u32) {
        self.update_pointer_monitor();
        let pointer = self.seat.get_pointer().unwrap();
        let serial = SERIAL_COUNTER.next_serial();
        // One lookup per motion event serves the pointer focus, the shell's hover and focus follows mouse.
        let window_hit = self.window_at(self.pointer_location);
        let target = self.pointer_target_over(window_hit.as_ref());
        // A client holding the pointer (a drag) keeps it; the shell does not light up meanwhile.
        let shell = match &target {
            Some(PointerTarget::Shell(hit)) if !pointer.is_grabbed() => Some(*hit),
            _ => None,
        };
        self.shell_hover(shell);
        let under = match target {
            Some(PointerTarget::Surface(surface, origin)) if !self.shell_holds_pointer() => Some((surface, origin)),
            _ => None,
        };
        // Focus follows mouse, unless a panel or the shell's power menu holds the keyboard.
        if self.keyboard_layer().is_none()
            && !self.shell_takes_keyboard()
            && let Some((window, _)) = &window_hit
            && let Some(id) = self.desktop.windows.iter().find(|m| &m.window == window).map(|m| m.id)
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

    /// Re-evaluate what is under the pointer after the layout changed (a workspace switch, a
    /// window that closed, moved or went fullscreen). Pointer focus otherwise follows only
    /// motion, and a locked pointer (games) produces none: the hidden game would keep the
    /// pointer and its lock.
    pub fn refresh_pointer_focus(&mut self) {
        if self.overview_open() || self.selecting.is_some() || self.picking.is_some() || self.desktop.drag.is_some() {
            return;
        }
        let Some(pointer) = self.seat.get_pointer() else { return };
        let under = self.pointer_focus();
        let surface = under.as_ref().map(|(s, _)| s.clone());
        if surface == self.pointer_focus_surface {
            return;
        }
        let old = std::mem::replace(&mut self.pointer_focus_surface, surface.clone());
        self.pointer_focus_changed(old.as_ref(), surface.as_ref());
        let serial = SERIAL_COUNTER.next_serial();
        let time = self.start_time.elapsed().as_millis() as u32;
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
