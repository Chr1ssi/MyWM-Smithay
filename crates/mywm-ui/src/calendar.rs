//! The calendar panel's content (`CalendarPanelInteractive.qml`): a month to page through, the selected day's events
//! and the next ones from `mywm-calendar` (khal), a form that creates an event on the selected day, and a button
//! that opens the calendar app. The form's fields and calendar choice are Qt Quick's Fusion controls, which keep
//! their own look and font (the system's sans-serif font at Qt's default 9 pt) instead of the shell's theme.
use crate::{
    Align, Color, Measure, Node, Rect, Scene, TextStyle, Theme,
    bar::{BUTTON_HEIGHT, button_shape},
    edit::Input,
    i18n::Language,
    panel::{Content, PanelTarget},
};

/// A day of the Gregorian calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i32,
    /// 1 is January.
    pub month: u32,
    pub day: u32,
}

impl Date {
    pub const fn new(year: i32, month: u32, day: u32) -> Self {
        Self { year, month, day }
    }

    /// `yyyy-mm-dd`, as the helper reads and writes dates.
    pub fn iso(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    pub fn days_in_month(year: i32, month: u32) -> u32 {
        match month {
            2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    }

    /// The first day of the month `offset` months from this date's.
    pub fn first_of_month(&self, offset: i32) -> Self {
        let months = self.year * 12 + self.month as i32 - 1 + offset;
        Self::new(months.div_euclid(12), months.rem_euclid(12) as u32 + 1, 1)
    }

    /// 0 is Monday, 6 Sunday.
    pub fn weekday(&self) -> u32 {
        // Sakamoto's method, which counts from Sunday.
        const OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let year = if self.month < 3 { self.year - 1 } else { self.year };
        let index = (self.month.clamp(1, 12) - 1) as usize;
        let sunday_first = (year + year.div_euclid(4) - year.div_euclid(100) + year.div_euclid(400) + OFFSETS[index] + self.day as i32).rem_euclid(7);
        ((sunday_first + 6) % 7) as u32
    }
}

const MONTHS_DE: [&str; 12] = ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];
const MONTHS_EN: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
/// Qt's stand-alone short day names (CLDR), from Monday.
const DAYS_DE: [&str; 7] = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"];
const DAYS_EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn month_name(month: u32, language: Language) -> &'static str {
    let names = if language == Language::German { MONTHS_DE } else { MONTHS_EN };
    names[(month.clamp(1, 12) - 1) as usize]
}

/// An event as the helper lists it (`mywm-calendar events`): dates as `yyyy-mm-dd`, times as `HH:MM`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EventView {
    pub title: String,
    pub location: String,
    pub calendar: String,
    pub start_date: String,
    pub end_date: String,
    pub start_time: String,
    pub end_time: String,
    pub all_day: bool,
    pub color: Option<Color>,
}

impl EventView {
    /// Whether it takes place on the day `iso` (`yyyy-mm-dd`, which compares like the dates it names).
    fn on(&self, iso: &str) -> bool {
        self.start_date.as_str() <= iso && self.end_date.as_str() >= iso
    }
}

/// The helper's list of events, `None` if it is no list. An empty answer is an empty list.
pub fn parse_events(text: &str) -> Option<Vec<EventView>> {
    let text = if text.trim().is_empty() { "[]" } else { text };
    let list: Vec<serde_json::Value> = serde_json::from_str(text).ok()?;
    let field = |event: &serde_json::Value, name: &str| event[name].as_str().unwrap_or_default().to_owned();
    Some(
        list.iter()
            .map(|event| EventView {
                title: field(event, "title"),
                location: field(event, "location"),
                calendar: field(event, "calendar"),
                start_date: field(event, "startDate"),
                end_date: field(event, "endDate"),
                start_time: field(event, "startTime"),
                end_time: field(event, "endTime"),
                all_day: event["allDay"].as_bool().unwrap_or(false),
                color: event["color"].as_str().and_then(Color::parse),
            })
            .collect(),
    )
}

/// The names of the helper's calendars (`mywm-calendar calendars`); nothing where it says nothing usable.
pub fn parse_calendars(text: &str) -> Vec<String> {
    let list: Vec<serde_json::Value> = serde_json::from_str(text).unwrap_or_default();
    list.iter().filter_map(|calendar| calendar["name"].as_str().map(str::to_owned)).collect()
}

/// What the panel says instead of, or about, the events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalendarError {
    /// The helper failed: khal has no configuration yet.
    NotSetUp,
    Unreadable,
    /// The form's title or times are missing or wrong.
    Invalid,
    CreateFailed,
}

impl CalendarError {
    fn text(self, language: Language) -> &'static str {
        match self {
            CalendarError::NotSetUp => language.tr("Kalender ist noch nicht eingerichtet."),
            CalendarError::Unreadable => language.tr("Termine konnten nicht gelesen werden."),
            CalendarError::Invalid => language.tr("Bitte Titel sowie gültige Start- und Endzeit eingeben."),
            CalendarError::CreateFailed => language.tr("Der Termin konnte nicht erstellt werden."),
        }
    }
}

/// The form's text fields, in their order (Tab moves through them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Title,
    Start,
    End,
    Location,
}

impl Field {
    pub const ALL: [Field; 4] = [Field::Title, Field::Start, Field::End, Field::Location];

    pub fn index(self) -> usize {
        self as usize
    }
}

/// The form to create an event, which keeps what was typed while it is folded away.
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    /// By `Field::index`.
    pub inputs: [Input; 4],
    pub focus: Option<Field>,
    /// The calendar chosen from `CalendarModel::calendars`.
    pub calendar: Option<usize>,
    /// The calendar choice lists the calendars.
    pub choosing: bool,
    pub saving: bool,
}

impl Default for Form {
    fn default() -> Self {
        let inputs = [Input::default(), Input::new("09:00"), Input::new("10:00"), Input::default()];
        Self { inputs, focus: None, calendar: None, choosing: false, saving: false }
    }
}

impl Form {
    pub fn input(&self, field: Field) -> &Input {
        &self.inputs[field.index()]
    }

    pub fn input_mut(&mut self, field: Field) -> &mut Input {
        &mut self.inputs[field.index()]
    }
}

/// What the form asks the helper to create: calendar, date, start, end, title and location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewEvent {
    pub calendar: String,
    pub date: Date,
    pub start: String,
    pub end: String,
    pub title: String,
    pub location: String,
}

fn is_time(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 5 && bytes[2] == b':' && [0, 1, 3, 4].iter().all(|&i| bytes[i].is_ascii_digit())
}

/// The event the form describes on `date`, or why there is none (as `submitEvent` checks it).
pub fn new_event(form: &Form, calendars: &[String], date: Date) -> Result<NewEvent, CalendarError> {
    let text = |field| form.input(field).text.trim().to_owned();
    let (title, start, end) = (text(Field::Title), text(Field::Start), text(Field::End));
    let calendar = form.calendar.and_then(|index| calendars.get(index)).filter(|name| !name.is_empty());
    match calendar {
        Some(calendar) if !title.is_empty() && is_time(&start) && is_time(&end) && end > start => {
            Ok(NewEvent { calendar: calendar.clone(), date, start, end, title, location: text(Field::Location) })
        }
        _ => Err(CalendarError::Invalid),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CalendarModel {
    pub today: Date,
    /// The first day of the month shown.
    pub month: Date,
    pub selected: Date,
    pub events: Vec<EventView>,
    pub calendars: Vec<String>,
    pub error: Option<CalendarError>,
    /// The form is open.
    pub creating: bool,
    pub form: Form,
    /// The language of dates (`LC_TIME`).
    pub time_language: Language,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalendarTarget {
    PreviousMonth,
    NextMonth,
    Day(u32),
    ToggleForm,
    Reload,
    Field(Field),
    /// The calendar choice: it lists the calendars.
    Choice,
    /// A calendar in the list.
    Calendar(usize),
    Create,
    OpenApp,
}

const SPACING: f32 = 14.0;
const TEXT: TextStyle = TextStyle::new(14.0);
/// Qt's default font (9 pt at 96 dpi) in the system's sans-serif family.
const CONTROL_TEXT: TextStyle = TextStyle::new(12.0).controls(true);
const CONTROL_HEIGHT: f32 = 24.0;
const ITEM_HEIGHT: f32 = 26.0;
const CONTROL_PADDING: f32 = 4.0;

/// The Fusion palette's colors (Qt's own, the controls do not follow the shell's theme).
mod fusion {
    use crate::Color;

    pub const BASE: Color = Color::rgba(0xff, 0xff, 0xff, 255);
    pub const TEXT: Color = Color::rgba(0, 0, 0, 255);
    pub const PLACEHOLDER: Color = Color::rgba(0, 0, 0, 128);
    pub const OUTLINE: Color = Color::rgba(0xab, 0xab, 0xab, 255);
    pub const HIGHLIGHTED_OUTLINE: Color = Color::rgba(0x26, 0x70, 0x9e, 255);
    pub const HIGHLIGHT: Color = Color::rgba(0x30, 0x8c, 0xc6, 255);
    pub const HIGHLIGHTED_TEXT: Color = Color::rgba(0xff, 0xff, 0xff, 255);
    pub const TOP_SHADOW: Color = Color::rgba(0, 0, 0, 18);
    pub const ARROW: Color = Color::rgba(0x44, 0x44, 0x44, 255);
    pub const POPUP: Color = Color::rgba(0xef, 0xef, 0xef, 255);
    pub const SHADOW: Color = Color::rgba(0, 0, 0, 16);
}

fn label(content: &mut Content, rect: Rect, text: String, style: TextStyle, color: Color, align: Align) {
    content.push(Node::Label { rect, text, style, color, align });
}

fn rect(content: &mut Content, rect: Rect, radius: f32, fill: Color, border: Option<(f32, Color)>) {
    content.push(Node::Rect { rect, radius, fill, border });
}

#[allow(clippy::too_many_arguments)]
fn button(content: &mut Content, theme: &Theme, rect: Rect, text: &str, selected: bool, enabled: bool, target: CalendarTarget, hover: Option<&PanelTarget>) {
    let target = PanelTarget::Calendar(target);
    let mut scene: Scene<()> = Scene::new(rect.w, 0.0);
    button_shape(&mut scene, theme, rect, text, false, selected, false, enabled && hover == Some(&target));
    let fade = if enabled { 1.0 } else { 0.45 };
    content.nodes.extend(scene.nodes.into_iter().map(|node| node.faded(fade)));
    if enabled {
        content.target(rect, target);
    }
}

/// A `BarButton` as wide as its text.
fn fitted_width(measure: &mut impl Measure, text: &str) -> f32 {
    measure.width(text, crate::bar::BUTTON_TEXT).ceil() + 16.0
}

/// How far a field `width` wide scrolls its line to the left: just enough that the cursor shows.
fn scrolled(measure: &mut impl Measure, input: &Input, width: f32) -> f32 {
    let cursor_x = measure.width(&input.text[..input.cursor], CONTROL_TEXT);
    (cursor_x - (width - 2.0 * CONTROL_PADDING) + 1.0).max(0.0)
}

/// The cursor position nearest to `x` (from the left edge of a field `width` wide showing `input`).
pub fn cursor_at(measure: &mut impl Measure, input: &Input, width: f32, x: f32) -> usize {
    let x = x - CONTROL_PADDING + scrolled(measure, input, width);
    input.nearest(measure, CONTROL_TEXT, x)
}

/// A Fusion `TextField`: the line scrolls so that the cursor stays in view.
fn text_field(content: &mut Content, measure: &mut impl Measure, area: Rect, input: &Input, placeholder: &str, focused: bool, field: Field) {
    let outline = if focused { fusion::HIGHLIGHTED_OUTLINE } else { fusion::OUTLINE };
    rect(content, area, 2.0, fusion::BASE, Some((1.0, outline)));
    if focused {
        rect(content, area.inset(1.0), 1.7, Color::TRANSPARENT, Some((1.0, fusion::HIGHLIGHTED_OUTLINE.with_alpha(40.0 / 255.0))));
    }
    rect(content, Rect::new(area.x + 2.0, area.y + 1.0, area.w - 4.0, 1.0), 0.0, fusion::TOP_SHADOW, None);
    let inner = Rect::new(area.x + CONTROL_PADDING, area.y, area.w - 2.0 * CONTROL_PADDING, area.h);
    let cursor_x = measure.width(&input.text[..input.cursor], CONTROL_TEXT);
    let scrolled = scrolled(measure, input, area.w);
    content.push(Node::Clip(Some(inner)));
    if input.text.is_empty() {
        label(content, inner, placeholder.to_owned(), CONTROL_TEXT, fusion::PLACEHOLDER, Align::Left);
    } else {
        // Wide enough that the line is not cut off with "…": the clip cuts it.
        let line = Rect::new(inner.x - scrolled, inner.y, measure.width(&input.text, CONTROL_TEXT).ceil() + 1.0, inner.h);
        label(content, line, input.text.clone(), CONTROL_TEXT, fusion::TEXT, Align::Left);
    }
    if focused {
        let height = measure.line_height(CONTROL_TEXT);
        let cursor = Rect::new((inner.x + cursor_x - scrolled).round(), area.y + ((area.h - height) / 2.0).round(), 1.0, height);
        rect(content, cursor, 0.0, fusion::TEXT, None);
    }
    content.push(Node::Clip(None));
    content.target(area, PanelTarget::Calendar(CalendarTarget::Field(field)));
}

/// A Fusion `ComboBox`: a light gradient, the choice and an arrow.
fn choice(content: &mut Content, area: Rect, text: Option<&str>) {
    rect(content, area, 2.0, fusion::BASE, Some((1.0, fusion::OUTLINE)));
    // The button's gradient, one line at a time, from #fefefe to #eeeeee with a lighter bottom line.
    let rows = (area.h - 2.0).max(0.0) as usize;
    for row in 0..rows {
        let value = if row + 1 == rows { 0xf0 } else { (254.0 - row as f32 * 0.8).round() as u8 };
        rect(content, Rect::new(area.x + 1.0, area.y + 1.0 + row as f32, area.w - 2.0, 1.0), 0.0, Color::rgba(value, value, value, 255), None);
    }
    if let Some(text) = text {
        let inner = Rect::new(area.x + CONTROL_PADDING, area.y, area.w - 2.0 * CONTROL_PADDING - 20.0, area.h);
        label(content, inner, text.to_owned(), CONTROL_TEXT, fusion::TEXT, Align::Left);
    }
    // The arrow: a triangle 7 wide and 4 high, its rows on half pixels as Qt antialiases it.
    for row in 0..4 {
        let width = 7.0 - 2.0 * row as f32;
        let line = Rect::new(area.right() - 13.5 + row as f32, area.y + 10.0 + row as f32, width, 1.0);
        rect(content, line, 0.0, fusion::ARROW, None);
    }
    content.target(area, PanelTarget::Calendar(CalendarTarget::Choice));
}

/// The open list of the calendar choice, over it: the calendar under the pointer, else the chosen one, highlighted.
fn choice_list(content: &mut Content, area: Rect, names: &[String], chosen: Option<usize>, hover: Option<&PanelTarget>) {
    let list = Rect::new(area.x, area.y, area.w, names.len() as f32 * ITEM_HEIGHT + 2.0);
    rect(content, list.translated(1.0, 1.0), 0.0, fusion::SHADOW, None);
    rect(content, list, 0.0, fusion::POPUP, Some((1.0, fusion::OUTLINE)));
    let hovered = names.iter().enumerate().map(|(i, _)| i).find(|i| hover == Some(&PanelTarget::Calendar(CalendarTarget::Calendar(*i))));
    let highlighted = hovered.or(chosen);
    for (index, name) in names.iter().enumerate() {
        let item = Rect::new(list.x + 1.0, list.y + 1.0 + index as f32 * ITEM_HEIGHT, list.w - 2.0, ITEM_HEIGHT);
        let color = if highlighted == Some(index) {
            rect(content, item, 0.0, fusion::HIGHLIGHT, None);
            fusion::HIGHLIGHTED_TEXT
        } else {
            fusion::TEXT
        };
        let style = CONTROL_TEXT.bold(chosen == Some(index));
        let text = Rect::new(item.x + 7.0, item.y, (item.w - 14.0).max(0.0), item.h);
        label(content, text, name.clone(), style, color, Align::Left);
        content.target(item, PanelTarget::Calendar(CalendarTarget::Calendar(index)));
    }
}

/// An event's card: a stripe in its calendar's color, its title and `detail`. Returns its height.
#[allow(clippy::too_many_arguments)]
fn card(content: &mut Content, theme: &Theme, measure: &mut impl Measure, y: f32, width: f32, event: &EventView, detail: String, language: Language) -> f32 {
    let (title_style, detail_style) = (TextStyle::new(15.0), TextStyle::new(13.0));
    let (title_h, detail_h) = (measure.line_height(title_style), measure.line_height(detail_style));
    let column_h = title_h + 2.0 + detail_h;
    let height = column_h + 14.0;
    rect(content, Rect::new(0.0, y, width, height), 12.0, theme.glass_surface(), None);
    let top = y + (height - column_h) / 2.0;
    rect(content, Rect::new(10.0, top, 4.0, column_h), 2.0, event.color.unwrap_or(theme.accent), None);
    let column_w = (width - 20.0 - 13.0).max(0.0);
    let title = if event.title.is_empty() { language.tr("(Ohne Titel)").to_owned() } else { event.title.clone() };
    label(content, Rect::new(23.0, top, column_w, title_h), title, title_style, theme.text, Align::Left);
    label(content, Rect::new(23.0, top + title_h + 2.0, column_w, detail_h), detail, detail_style, theme.muted, Align::Left);
    height
}

fn times(event: &EventView, language: Language) -> String {
    if event.all_day { language.tr("Ganztägig").to_owned() } else { format!("{}–{}", event.start_time, event.end_time) }
}

/// A line of `style` at `y`; returns its height.
fn line(content: &mut Content, measure: &mut impl Measure, y: f32, width: f32, text: &str, style: TextStyle, color: Color) -> f32 {
    let height = measure.line_height(style);
    label(content, Rect::new(0.0, y, width, height), text.to_owned(), style, color, Align::Left);
    height
}

pub fn content(model: &CalendarModel, hover: Option<&PanelTarget>, theme: &Theme, measure: &mut impl Measure, width: f32, language: Language) -> Content {
    let tr = |text| language.tr(text);
    let mut content = Content::default();

    // The month and the buttons around it.
    let previous_w = fitted_width(measure, "‹");
    button(&mut content, theme, Rect::new(0.0, 0.0, previous_w, BUTTON_HEIGHT), "‹", false, true, CalendarTarget::PreviousMonth, hover);
    let month = format!("{} {}", month_name(model.month.month, model.time_language), model.month.year);
    let month_rect = Rect::new(previous_w + 8.0, 0.0, (width - 96.0).max(0.0), BUTTON_HEIGHT);
    label(&mut content, month_rect, month, TextStyle::new(18.0), theme.text, Align::Center);
    let next = Rect::new(month_rect.right() + 8.0, 0.0, fitted_width(measure, "›"), BUTTON_HEIGHT);
    button(&mut content, theme, next, "›", false, true, CalendarTarget::NextMonth, hover);
    let mut y = BUTTON_HEIGHT + SPACING;

    // The days: names, then six weeks from Monday.
    let column = (width - 18.0) / 7.0;
    let names = if model.time_language == Language::German { DAYS_DE } else { DAYS_EN };
    let name_style = TextStyle::new(13.0);
    let name_h = measure.line_height(name_style);
    for (index, name) in names.iter().enumerate() {
        let rect = Rect::new(index as f32 * (column + 3.0), y, column, name_h);
        label(&mut content, rect, (*name).to_owned(), name_style, theme.muted, Align::Center);
    }
    let first = model.month.weekday() as i32;
    let days = Date::days_in_month(model.month.year, model.month.month) as i32;
    let number_style = TextStyle::new(14.0);
    let number_h = measure.line_height(number_style);
    for index in 0..42 {
        let day = index - first + 1;
        let cell = Rect::new((index % 7) as f32 * (column + 3.0), y + 22.0 + 3.0 + (index / 7) as f32 * 37.0, column, 34.0);
        if day < 1 || day > days {
            continue;
        }
        let date = Date::new(model.month.year, model.month.month, day as u32);
        let selected = date == model.selected;
        if selected {
            rect(&mut content, cell, 8.0, theme.accent, None);
        } else if date == model.today {
            rect(&mut content, cell, 8.0, theme.surface, None);
        }
        let color = if selected { theme.background } else { theme.text };
        label(&mut content, Rect::new(cell.x, cell.y + 3.0, cell.w, number_h), day.to_string(), number_style, color, Align::Center);
        let iso = date.iso();
        let dots: Vec<Color> = model.events.iter().filter(|e| e.on(&iso)).take(3).map(|e| e.color.unwrap_or(theme.accent)).collect();
        let dots_w = dots.len() as f32 * 6.0 - 2.0;
        for (dot, color) in dots.into_iter().enumerate() {
            let at = Rect::new(cell.x + (cell.w - dots_w) / 2.0 + dot as f32 * 6.0, cell.bottom() - 3.0 - 4.0, 4.0, 4.0);
            rect(&mut content, at, 2.0, color, None);
        }
        content.target(cell, PanelTarget::Calendar(CalendarTarget::Day(day as u32)));
    }
    y += 22.0 + 6.0 * 37.0 + SPACING;
    rect(&mut content, Rect::new(0.0, y, width, 1.0), 0.0, theme.border, None);
    y += 1.0 + SPACING;

    // The selected day.
    let day = format!("{:02}. {}", model.selected.day, month_name(model.selected.month, model.time_language));
    let header = TextStyle::new(16.0);
    let header_rect = Rect::new(0.0, y, (width - 104.0).max(0.0), measure.line_height(header));
    label(&mut content, header_rect, language.trf("Termine am %1", &[&day]), header, theme.text, Align::Left);
    let add = Rect::new(header_rect.right() + 8.0, y, fitted_width(measure, "+"), BUTTON_HEIGHT);
    button(&mut content, theme, add, "+", model.creating, true, CalendarTarget::ToggleForm, hover);
    let reload = Rect::new(add.right() + 8.0, y, fitted_width(measure, "󰑐"), BUTTON_HEIGHT);
    button(&mut content, theme, reload, "󰑐", false, true, CalendarTarget::Reload, hover);
    y += header_rect.h.max(BUTTON_HEIGHT) + SPACING;

    let mut choice_area = None;
    if model.creating {
        let form = &model.form;
        let focused = |field| form.focus == Some(field);
        let half = (width - 8.0) / 2.0;
        let fields = [
            (Field::Title, Rect::new(0.0, y, width, CONTROL_HEIGHT), tr("Titel")),
            (Field::Start, Rect::new(0.0, y + 32.0, half, CONTROL_HEIGHT), "Start (HH:MM)"),
            (Field::End, Rect::new(half + 8.0, y + 32.0, half, CONTROL_HEIGHT), tr("Ende (HH:MM)")),
            (Field::Location, Rect::new(0.0, y + 64.0, width, CONTROL_HEIGHT), tr("Ort (optional)")),
        ];
        for (field, area, placeholder) in fields {
            text_field(&mut content, measure, area, form.input(field), placeholder, focused(field), field);
        }
        let area = Rect::new(0.0, y + 96.0, width, CONTROL_HEIGHT);
        choice(&mut content, area, form.calendar.and_then(|index| model.calendars.get(index)).map(String::as_str));
        if form.choosing && !model.calendars.is_empty() {
            choice_area = Some(area);
        }
        let text = if form.saving { tr("Wird gespeichert …") } else { tr("Termin erstellen") };
        button(&mut content, theme, Rect::new(0.0, y + 128.0, width, BUTTON_HEIGHT), text, false, !form.saving, CalendarTarget::Create, hover);
        y += 128.0 + BUTTON_HEIGHT + SPACING;
    }

    if let Some(error) = model.error {
        let text = error.text(language);
        let height = measure.paragraph_height(text, TEXT, width);
        content.push(Node::Paragraph { rect: Rect::new(0.0, y, width, height), text: text.to_owned(), style: TEXT, color: theme.muted, align: Align::Left });
        y += height + SPACING;
    }
    let selected = model.selected.iso();
    let on_day: Vec<&EventView> = model.events.iter().filter(|e| e.on(&selected)).collect();
    for event in &on_day {
        let mut detail = format!("{}  ·  {}", times(event, language), event.calendar);
        if !event.location.is_empty() {
            detail += &format!("  ·  {}", event.location);
        }
        y += card(&mut content, theme, measure, y, width, event, detail, language) + SPACING;
    }
    if model.error.is_none() && on_day.is_empty() {
        y += line(&mut content, measure, y, width, tr("Keine Termine an diesem Tag."), TEXT, theme.muted) + SPACING;
    }
    rect(&mut content, Rect::new(0.0, y, width, 1.0), 0.0, theme.border, None);
    y += 1.0 + SPACING;

    // The next events.
    y += line(&mut content, measure, y, width, tr("Nächste Termine"), header, theme.text) + SPACING;
    let today = model.today.iso();
    let upcoming: Vec<&EventView> = model.events.iter().filter(|e| e.end_date >= today).take(8).collect();
    for event in &upcoming {
        // `yyyy-mm-dd` as `dd.mm.`.
        let mut parts = event.start_date.split('-').rev();
        let date = format!("{}.{}.", parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
        let detail = format!("{date}  ·  {}  ·  {}", times(event, language), event.calendar);
        y += card(&mut content, theme, measure, y, width, event, detail, language) + SPACING;
    }
    if model.error.is_none() && upcoming.is_empty() {
        y += line(&mut content, measure, y, width, tr("Keine anstehenden Termine im geladenen Zeitraum."), TEXT, theme.muted) + SPACING;
    }
    button(&mut content, theme, Rect::new(0.0, y, width, BUTTON_HEIGHT), tr("Kalender-App öffnen"), false, true, CalendarTarget::OpenApp, hover);
    content.height = y + BUTTON_HEIGHT;

    // The open list lies over everything after the choice (and takes the clicks there).
    if let Some(area) = choice_area {
        choice_list(&mut content, area, &model.calendars, model.form.calendar, hover);
    }
    content
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{scene::tests::Mono, theme::Palette};

    fn theme() -> Theme {
        let c = |v| Color::rgba(v, v, v, 255);
        Theme::new(Palette { background: c(10), surface: c(20), text: c(200), muted: c(150), accent: c(90), border: c(60) }, None)
    }

    fn texts(content: &Content) -> Vec<String> {
        content.nodes.iter().filter_map(|n| match n { Node::Label { text, .. } | Node::Paragraph { text, .. } => Some(text.clone()), _ => None }).collect()
    }

    fn target(content: &Content, target: CalendarTarget) -> Option<Rect> {
        content.targets.iter().rev().find(|(_, t)| *t == PanelTarget::Calendar(target)).map(|(rect, _)| *rect)
    }

    fn event(title: &str, start: &str, end: &str) -> EventView {
        EventView { title: title.into(), calendar: "Privat".into(), start_date: start.into(), end_date: end.into(), start_time: "09:00".into(), end_time: "10:00".into(), ..EventView::default() }
    }

    fn model() -> CalendarModel {
        let today = Date::new(2026, 10, 20);
        CalendarModel {
            today,
            month: today.first_of_month(0),
            selected: today,
            events: vec![event("Zahnarzt", "2026-10-20", "2026-10-20"), EventView { all_day: true, ..event("", "2026-10-23", "2026-10-24") }],
            calendars: vec!["Arbeit".into(), "Privat".into()],
            error: None,
            creating: false,
            form: Form::default(),
            time_language: Language::German,
        }
    }

    #[test]
    fn dates_count_like_the_calendar() {
        assert_eq!(Date::new(2026, 10, 1).weekday(), 3, "1 October 2026 is a Thursday");
        assert_eq!(Date::new(2024, 2, 29).weekday(), 3);
        assert_eq!(Date::new(2000, 1, 1).weekday(), 5);
        assert_eq!(Date::days_in_month(2024, 2), 29);
        assert_eq!(Date::days_in_month(1900, 2), 28);
        assert_eq!(Date::days_in_month(2000, 2), 29);
        assert_eq!(Date::new(2026, 1, 15).first_of_month(-1), Date::new(2025, 12, 1));
        assert_eq!(Date::new(2026, 12, 31).first_of_month(1), Date::new(2027, 1, 1));
        assert_eq!(Date::new(2026, 3, 9).iso(), "2026-03-09");
    }

    #[test]
    fn the_helpers_answers_parse() {
        let events = parse_events(r##"[{"title":"A","startDate":"2026-10-20","endDate":"2026-10-21","allDay":true,"color":"#ff8800","calendar":"Privat"},{"title":"B","color":""}]"##).unwrap();
        assert_eq!(events.len(), 2);
        assert!(events[0].all_day && events[0].color == Some(Color::rgba(0xff, 0x88, 0, 255)));
        assert!(events[0].on("2026-10-21") && !events[0].on("2026-10-22"));
        assert_eq!(events[1].color, None);
        assert_eq!(parse_events(""), Some(Vec::new()));
        assert_eq!(parse_events("Fehler"), None);
        assert_eq!(parse_calendars(r##"[{"name":"Arbeit","color":""},{"color":"#fff"}]"##), ["Arbeit"]);
        assert!(parse_calendars("kaputt").is_empty());
    }

    #[test]
    fn the_month_starts_on_its_weekday_and_marks_days() {
        let content = content(&model(), None, &theme(), &mut Mono, 360.0, Language::German);
        let texts = texts(&content);
        for text in ["Oktober 2026", "Mo", "So", "Termine am 20. Oktober", "Zahnarzt", "09:00–10:00  ·  Privat", "(Ohne Titel)", "23.10.  ·  Ganztägig  ·  Privat", "Kalender-App öffnen"] {
            assert!(texts.contains(&text.to_owned()), "{text} missing in {texts:?}");
        }
        assert!(!texts.contains(&"Keine Termine an diesem Tag.".to_owned()));
        // 1 October is a Thursday: the fourth column of the first week; the grid's cells are (360 - 18) / 7 wide.
        let first = target(&content, CalendarTarget::Day(1)).unwrap();
        let column = (360.0 - 18.0) / 7.0;
        assert_eq!(first, Rect::new(3.0 * (column + 3.0), 26.0 + 14.0 + 25.0, column, 34.0));
        assert!(target(&content, CalendarTarget::Day(31)).is_some() && target(&content, CalendarTarget::Day(32)).is_none());
        assert!(target(&content, CalendarTarget::Field(Field::Title)).is_none());
        // The 23rd and 24th have the all-day event's dot.
        let dots = |day| {
            let cell = target(&content, CalendarTarget::Day(day)).unwrap();
            content.nodes.iter().filter(|n| matches!(n, Node::Rect { rect, .. } if rect.w == 4.0 && rect.h == 4.0 && cell.contains(rect.x, rect.y))).count()
        };
        assert_eq!((dots(20), dots(23), dots(24), dots(25)), (1, 1, 1, 0));
    }

    #[test]
    fn the_form_has_qts_controls_and_lists_the_calendars_over_the_rest() {
        let mut model = model();
        model.creating = true;
        model.form.calendar = Some(0);
        let closed = content(&model, None, &theme(), &mut Mono, 360.0, Language::English);
        let title = target(&closed, CalendarTarget::Field(Field::Title)).unwrap();
        let end = target(&closed, CalendarTarget::Field(Field::End)).unwrap();
        let choice = target(&closed, CalendarTarget::Choice).unwrap();
        assert_eq!((title.h, end.x, end.w), (24.0, 184.0, 176.0));
        assert_eq!(choice.y, title.y + 96.0);
        let texts = texts(&closed);
        for text in ["Title", "09:00", "10:00", "Location (optional)", "Arbeit", "Create event"] {
            assert!(texts.contains(&text.to_owned()), "{text} missing in {texts:?}");
        }
        assert!(!texts.contains(&"End (HH:MM)".to_owned()), "the end has a text, its placeholder does not show");
        let mut emptied = model.clone();
        emptied.form.input_mut(Field::End).clear();
        assert!(texts_of(&emptied).contains(&"End (HH:MM)".to_owned()));

        model.form.choosing = true;
        let open = content(&model, None, &theme(), &mut Mono, 360.0, Language::English);
        let second = target(&open, CalendarTarget::Calendar(1)).unwrap();
        assert_eq!(second, Rect::new(1.0, choice.y + 27.0, 358.0, 26.0));
        // Over the button below the choice, the list takes the click.
        let create = target(&open, CalendarTarget::Create).unwrap();
        let mut scene: Scene<PanelTarget> = Scene::new(360.0, open.height);
        open.targets.iter().for_each(|(rect, t)| scene.target(*rect, *t));
        assert_eq!(scene.target_at(create.x + 10.0, second.y + 5.0), Some(PanelTarget::Calendar(CalendarTarget::Calendar(1))));
    }

    #[test]
    fn a_click_puts_the_cursor_between_the_nearest_characters() {
        // `Mono` makes the 12 px characters 6 wide; the text starts 4 in.
        let input = Input::new("09:00");
        assert_eq!(cursor_at(&mut Mono, &input, 176.0, 0.0), 0);
        assert_eq!(cursor_at(&mut Mono, &input, 176.0, 4.0 + 6.0 * 2.0 + 2.0), 2);
        assert_eq!(cursor_at(&mut Mono, &input, 176.0, 4.0 + 6.0 * 2.0 + 4.0), 3);
        assert_eq!(cursor_at(&mut Mono, &input, 176.0, 170.0), 5);
        // A long line scrolled to its end: the field's left edge is far into it.
        let long = Input::new(&"x".repeat(100));
        assert_eq!(cursor_at(&mut Mono, &long, 176.0, 4.0), 100 - 28);
    }

    #[test]
    fn the_form_checks_like_the_panel() {
        let calendars = ["Arbeit".to_owned()];
        let date = Date::new(2026, 10, 20);
        let mut form = Form { calendar: Some(0), ..Form::default() };
        assert_eq!(new_event(&form, &calendars, date), Err(CalendarError::Invalid), "a title is needed");
        form.input_mut(Field::Title).insert("  Zahnarzt ");
        form.input_mut(Field::Location).insert(" Praxis");
        let made = new_event(&form, &calendars, date).unwrap();
        assert_eq!((made.title.as_str(), made.location.as_str(), made.start.as_str(), made.calendar.as_str()), ("Zahnarzt", "Praxis", "09:00", "Arbeit"));
        for (start, end) in [("10:00", "09:00"), ("9:00", "10:00"), ("09:00", "10:00x"), ("09:00", "09:00")] {
            form.inputs[Field::Start.index()] = Input::new(start);
            form.inputs[Field::End.index()] = Input::new(end);
            assert_eq!(new_event(&form, &calendars, date), Err(CalendarError::Invalid), "{start}–{end}");
        }
        form.inputs[Field::End.index()] = Input::new("09:30");
        form.inputs[Field::Start.index()] = Input::new("09:00");
        assert!(new_event(&form, &calendars, date).is_ok());
        assert_eq!(new_event(&Form { calendar: None, ..form }, &calendars, date), Err(CalendarError::Invalid), "a calendar is needed");
    }

    #[test]
    fn errors_replace_the_empty_notes() {
        let model = CalendarModel { events: Vec::new(), error: Some(CalendarError::NotSetUp), ..model() };
        let texts = texts(&content(&model, None, &theme(), &mut Mono, 360.0, Language::English));
        assert!(texts.contains(&"The calendar is not set up yet.".to_owned()));
        assert!(!texts.iter().any(|t| t.starts_with("No ")), "{texts:?}");
        let english = CalendarModel { time_language: Language::English, ..model };
        let texts = texts_of(&english);
        assert!(texts.contains(&"October 2026".to_owned()) && texts.contains(&"Mon".to_owned()) && texts.contains(&"Events on 20. October".to_owned()));
    }

    fn texts_of(model: &CalendarModel) -> Vec<String> {
        texts(&content(model, None, &theme(), &mut Mono, 360.0, Language::English))
    }
}
