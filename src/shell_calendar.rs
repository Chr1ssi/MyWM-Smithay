//! The calendar panel's own parts in the built-in shell: the events of the month shown and the calendars come from
//! the `splice-calendar` helper (khal) on threads of their own, the form creates events with it, and its fields take
//! the keyboard. Every opening of the panel starts afresh on today, as the Quickshell panel's loader did; answers
//! for an earlier opening are dropped.
use splice_ui::{
    bar::Panel,
    calendar::{self, CalendarError, CalendarModel, CalendarTarget, Date, EventView, Field, Form},
    clock::LocalTime,
    panel::PanelTarget,
};
use smithay::{
    input::keyboard::{Keysym, keysyms},
    reexports::calloop::channel,
};

use crate::State;

const HELPER: &str = "splice-calendar";

#[derive(Default)]
pub struct Calendar {
    /// Tells this opening of the panel apart from earlier ones, whose helpers may still answer.
    opening: u64,
    view: Option<View>,
}

struct View {
    month: Date,
    selected: Date,
    events: Vec<EventView>,
    calendars: Vec<String>,
    error: Option<CalendarError>,
    creating: bool,
    form: Form,
    /// The events are being asked for (meanwhile another reload does nothing, as in the QML panel).
    loading: bool,
}

impl View {
    fn new(today: Date) -> Self {
        Self {
            month: today.first_of_month(0),
            selected: today,
            events: Vec::new(),
            calendars: Vec::new(),
            error: None,
            creating: false,
            form: Form::default(),
            loading: false,
        }
    }
}

/// How a helper ended: whether it succeeded, and what it printed. `None` if it did not start.
type Outcome = Option<(bool, String)>;

fn today() -> Date {
    LocalTime::now().map_or(Date::new(1970, 1, 1), |now| Date::new(now.year, now.month, now.day))
}

impl State {
    /// The panel opened: on today's month, with the calendars and the events asked for.
    pub fn calendar_open(&mut self) {
        let calendar = &mut self.shell.calendar;
        calendar.opening += 1;
        calendar.view = Some(View::new(today()));
        self.calendar_helper(vec!["calendars".into()], |state, outcome| {
            if let (Some(view), Some((_, text))) = (state.shell.calendar.view.as_mut(), outcome) {
                view.calendars = calendar::parse_calendars(&text);
                // A choice shows the first calendar until another is chosen, as a `ComboBox` does.
                if view.form.calendar.is_none_or(|index| index >= view.calendars.len()) {
                    view.form.calendar = (!view.calendars.is_empty()).then_some(0);
                }
            }
        });
        self.calendar_reload();
    }

    /// Run the helper with `args`; `done` gets its outcome if the panel is still the same opening.
    fn calendar_helper(&mut self, args: Vec<String>, done: impl FnOnce(&mut State, Outcome) + 'static) {
        let opening = self.shell.calendar.opening;
        let (sender, answers) = channel::channel();
        let mut done = Some(done);
        let answered = self.loop_handle.insert_source(answers, move |event, _, state: &mut State| {
            if let channel::Event::Msg(outcome) = event
                && state.shell.calendar.opening == opening
                && let Some(done) = done.take()
            {
                done(state, outcome);
                state.shell_changed();
            }
        });
        if let Err(error) = answered {
            return tracing::warn!("calendar: cannot wait for the helper: {}", error.error);
        }
        let spawned = std::thread::Builder::new().name("calendar".into()).spawn(move || {
            let output = std::process::Command::new(HELPER).args(&args).stdin(std::process::Stdio::null()).output();
            let outcome = match output {
                Ok(output) => Some((output.status.success(), String::from_utf8_lossy(&output.stdout).into_owned())),
                Err(error) => {
                    tracing::warn!("calendar: cannot run {HELPER} {}: {error}", args.first().map_or("", String::as_str));
                    None
                }
            };
            let _ = sender.send(outcome);
        });
        if let Err(error) = spawned {
            tracing::warn!("calendar: cannot run the helper: {error}");
        }
    }

    /// Ask for the events of the month shown and the first week of the next one.
    fn calendar_reload(&mut self) {
        let Some(view) = self.shell.calendar.view.as_mut().filter(|v| !v.loading) else { return };
        let next = view.month.first_of_month(1);
        let args = vec!["events".into(), view.month.iso(), Date::new(next.year, next.month, 8).iso()];
        view.error = None;
        view.loading = true;
        self.calendar_helper(args, |state, outcome| {
            let Some(view) = state.shell.calendar.view.as_mut() else { return };
            view.loading = false;
            match outcome {
                Some((success, text)) => {
                    let events = calendar::parse_events(&text);
                    view.error = if !success { Some(CalendarError::NotSetUp) } else { events.is_none().then_some(CalendarError::Unreadable) };
                    view.events = events.unwrap_or_default();
                }
                // Without the helper the calendar is not set up either.
                None => view.error = Some(CalendarError::NotSetUp),
            }
        });
        self.shell_changed();
    }

    fn calendar_create(&mut self) {
        let Some(view) = self.shell.calendar.view.as_mut().filter(|v| !v.form.saving) else { return };
        let event = match calendar::new_event(&view.form, &view.calendars, view.selected) {
            Ok(event) => event,
            Err(error) => {
                view.error = Some(error);
                return self.shell_changed();
            }
        };
        view.error = None;
        view.form.saving = true;
        let args = vec!["create".into(), event.calendar, event.date.iso(), event.start, event.end, event.title, event.location];
        self.calendar_helper(args, |state, outcome| {
            let Some(view) = state.shell.calendar.view.as_mut() else { return };
            view.form.saving = false;
            if outcome.is_some_and(|(success, _)| success) {
                view.form.input_mut(Field::Title).clear();
                view.form.input_mut(Field::Location).clear();
                view.creating = false;
                view.form.focus = None;
                state.calendar_reload();
            } else {
                view.error = Some(CalendarError::CreateFailed);
            }
        });
        self.shell_changed();
    }

    pub fn calendar_clicked(&mut self, target: CalendarTarget) {
        // Pressed anywhere but on it, the open list closes (and the press does what it does there).
        let field_cursor = match target {
            CalendarTarget::Field(field) => self.calendar_cursor_at_pointer(field),
            _ => None,
        };
        let Some(view) = self.shell.calendar.view.as_mut() else { return };
        if !matches!(target, CalendarTarget::Choice | CalendarTarget::Calendar(_)) {
            view.form.choosing = false;
        }
        match target {
            CalendarTarget::PreviousMonth | CalendarTarget::NextMonth => {
                view.month = view.month.first_of_month(if target == CalendarTarget::NextMonth { 1 } else { -1 });
                view.selected = view.month;
                self.calendar_reload();
            }
            CalendarTarget::Day(day) => view.selected = Date::new(view.month.year, view.month.month, day),
            CalendarTarget::ToggleForm => {
                view.creating = !view.creating;
                if !view.creating {
                    // Hidden, its fields and list lose the focus.
                    view.form.focus = None;
                    view.form.choosing = false;
                }
            }
            CalendarTarget::Reload => self.calendar_reload(),
            CalendarTarget::Field(field) => {
                view.form.focus = Some(field);
                if let Some(cursor) = field_cursor {
                    view.form.input_mut(field).cursor = cursor;
                }
            }
            CalendarTarget::Choice => {
                view.form.focus = None;
                view.form.choosing = !view.form.choosing;
            }
            CalendarTarget::Calendar(index) => {
                view.form.calendar = Some(index);
                view.form.choosing = false;
            }
            CalendarTarget::Create => self.calendar_create(),
            CalendarTarget::OpenApp => {
                let title = self.shell.language.tr("Kalender");
                let command = ["foot", "--app-id", "splice-calendar", "--title", title, HELPER, "app"].map(str::to_owned);
                self.spawn_command(&command, true);
                // The app's window takes the keyboard, which closed the Quickshell panel.
                self.panel_close();
            }
        }
        self.shell_changed();
    }

    /// Where in `field`'s text the pointer is (the nearest gap between characters), if the field is on screen.
    fn calendar_cursor_at_pointer(&mut self, field: Field) -> Option<usize> {
        let panel = self.shell.panel.as_ref()?;
        let geo = self.space.output_geometry(&panel.output)?;
        let area = panel.painted.as_ref()?.scene.target_rect(&PanelTarget::Calendar(CalendarTarget::Field(field)))?;
        let (x, _) = self.panel_origin()?;
        let local = (self.pointer_location.x - f64::from(geo.loc.x) - x) as f32 - area.x;
        let input = self.shell.calendar.view.as_ref()?.form.input(field).clone();
        Some(calendar::cursor_at(self.shell.fonts.as_mut()?, &input, area.w, local))
    }

    /// A key while the calendar panel has the keyboard; `false` if the panel does nothing with it.
    /// `modified`: Ctrl, Alt or Super is held, so it types nothing.
    pub fn calendar_key(&mut self, sym: Keysym, modified: bool) -> bool {
        if !self.shell.panel.as_ref().is_some_and(|p| p.kind == Panel::Calendar) {
            return false;
        }
        let Some(view) = self.shell.calendar.view.as_mut() else { return false };
        if view.form.choosing && sym.raw() == keysyms::KEY_Escape {
            view.form.choosing = false;
            self.shell_changed();
            return true;
        }
        let Some(field) = view.form.focus.filter(|_| view.creating) else { return false };
        match sym.raw() {
            keysyms::KEY_BackSpace => view.form.input_mut(field).backspace(),
            keysyms::KEY_Delete | keysyms::KEY_KP_Delete => view.form.input_mut(field).delete(),
            keysyms::KEY_Left | keysyms::KEY_KP_Left => view.form.input_mut(field).left(),
            keysyms::KEY_Right | keysyms::KEY_KP_Right => view.form.input_mut(field).right(),
            keysyms::KEY_Home | keysyms::KEY_KP_Home => view.form.input_mut(field).home(),
            keysyms::KEY_End | keysyms::KEY_KP_End => view.form.input_mut(field).end(),
            raw @ (keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab) => {
                let fields = Field::ALL;
                let step = if raw == keysyms::KEY_Tab { 1 } else { fields.len() - 1 };
                view.form.focus = Some(fields[(field.index() + step) % fields.len()]);
            }
            keysyms::KEY_Escape => return false,
            _ => match sym.key_char().filter(|c| !c.is_control() && !modified) {
                Some(c) => view.form.input_mut(field).insert(c.encode_utf8(&mut [0; 4])),
                None => return false,
            },
        }
        self.shell_changed();
        true
    }

    pub fn calendar_model(&self) -> CalendarModel {
        let today = today();
        let fresh;
        let view = match &self.shell.calendar.view {
            Some(view) => view,
            None => {
                fresh = View::new(today);
                &fresh
            }
        };
        CalendarModel {
            today,
            month: view.month,
            selected: view.selected,
            events: view.events.clone(),
            calendars: view.calendars.clone(),
            error: view.error,
            creating: view.creating,
            form: view.form.clone(),
            time_language: self.shell.time_language,
        }
    }
}
