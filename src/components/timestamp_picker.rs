//! Reusable Org timestamp editor. Emits an applied source value or cancellation;
//! it never reads an editor, writes a document, or owns its popup position.
mod view;

use std::rc::Rc;

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, MouseButton, Render, Subscription,
    Window, div, prelude::*, px, rgb,
};
use jiff::{
    Span,
    civil::{Date, Time},
};

use super::calendar::{Calendar, CalendarDrag};
use crate::{
    components::native_input::{InputConfig, InputEvent, NativeInput},
    i18n::Language,
    org_semantic::{
        Repeater, RepeaterMode, TimeUnit, TimestampKind, WarningPeriod,
        timestamp_edit::{TimestampDraft, TimestampEndpoint, is_diary, time_text},
    },
};

gpui::actions!(timestamp_picker, [CancelTimestamp]);

pub(crate) fn init(cx: &mut gpui::App) {
    cx.bind_keys([
        gpui::KeyBinding::new("escape", CancelTimestamp, Some("TimestampPicker")),
        gpui::KeyBinding::new("ctrl-g", CancelTimestamp, Some("TimestampPicker")),
    ]);
}

pub(crate) enum TimestampPickerEvent {
    Applied(String),
    Cancelled,
}
impl EventEmitter<TimestampPickerEvent> for TimestampPicker {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpandedField {
    Date,
    Time,
}

/// Input identity is independent of subscription order and UI layout.
#[derive(Clone, Copy, PartialEq, Eq)]
enum InputField {
    StartTime,
    EndTime,
    RepeatCount,
    Source,
    WarningCount,
}
impl InputField {
    const VALIDATED: [Self; 4] = [
        Self::StartTime,
        Self::EndTime,
        Self::RepeatCount,
        Self::WarningCount,
    ];
    fn is_time(self) -> bool {
        matches!(self, Self::StartTime | Self::EndTime)
    }
}

pub(crate) struct TimestampPicker {
    language: Language,
    focus: FocusHandle,
    draft: Option<TimestampDraft>,
    kind: TimestampKind,
    raw: String,
    source_open: bool,
    repeat_page: bool,
    warning_open: bool,
    unit_menu: bool,
    calendar_drag: CalendarDrag,
    editing_end: bool,
    expanded: Option<ExpandedField>,
    month: Date,
    today: Date,
    time_input: Entity<NativeInput>,
    end_time_input: Entity<NativeInput>,
    count_input: Entity<NativeInput>,
    source_input: Entity<NativeInput>,
    warning_input: Entity<NativeInput>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for TimestampPicker {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

fn input(id: &'static str, value: &str, cx: &mut gpui::App) -> Entity<NativeInput> {
    cx.new(|cx| {
        let mut input = NativeInput::new(
            InputConfig {
                id,
                outlined: true,
                font_size: 13.,
                ..Default::default()
            },
            cx,
        );
        input.sync(value, cx);
        input
    })
}

impl TimestampPicker {
    pub(crate) fn new(
        source: String,
        kind: TimestampKind,
        today: Date,
        language: Language,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = TimestampDraft::parse(&source, kind);
        let value = draft.as_ref().map(|d| &d.start.value);
        let month = value.map_or(today, |v| v.start_date);
        let time_input = input(
            "timestamp-time",
            &value
                .and_then(|v| v.start_time)
                .map(time_text)
                .unwrap_or_default(),
            cx,
        );
        let end_time_input = input(
            "timestamp-end-time",
            &value
                .and_then(|v| v.end_time)
                .map(time_text)
                .unwrap_or_default(),
            cx,
        );
        let count_input = input(
            "timestamp-repeat-count",
            &value
                .and_then(|v| v.repeater)
                .map_or(1, |r| r.value)
                .to_string(),
            cx,
        );
        let source_input = input("timestamp-source", &source, cx);
        let warning_input = input(
            "timestamp-warning-count",
            &value
                .and_then(|v| v.warning)
                .map_or(1, |w| w.value)
                .to_string(),
            cx,
        );
        let mut subscriptions = Vec::new();
        for (field, entity) in [
            (InputField::StartTime, time_input.clone()),
            (InputField::EndTime, end_time_input.clone()),
            (InputField::RepeatCount, count_input.clone()),
            (InputField::Source, source_input.clone()),
            (InputField::WarningCount, warning_input.clone()),
        ] {
            subscriptions.push(
                cx.subscribe(
                    &entity,
                    move |this, _, event: &InputEvent, cx| match event {
                        InputEvent::Changed(text) => this.input_changed(field, text, cx),
                        InputEvent::Command { key, .. } if key == "escape" => {
                            cx.emit(TimestampPickerEvent::Cancelled)
                        }
                        InputEvent::Command { key, .. } if key == "enter" => {
                            if field.is_time() && this.expanded == Some(ExpandedField::Time) {
                                if !this.input_invalid(InputField::StartTime, cx)
                                    && !this.input_invalid(InputField::EndTime, cx)
                                {
                                    this.expanded = None;
                                    cx.notify();
                                }
                            } else {
                                this.apply(cx);
                            }
                        }
                        _ => {}
                    },
                ),
            );
        }
        Self {
            language,
            focus: cx.focus_handle(),
            source_open: draft.is_none(),
            draft,
            kind,
            raw: source,
            repeat_page: false,
            warning_open: false,
            unit_menu: false,
            calendar_drag: Default::default(),
            editing_end: false,
            expanded: None,
            month,
            today,
            time_input,
            end_time_input,
            count_input,
            source_input,
            warning_input,
            _subscriptions: subscriptions,
        }
    }

    fn t(&self, key: &'static str) -> &'static str {
        self.language.text(key)
    }

    pub(crate) fn set_language(&mut self, language: Language, cx: &mut Context<Self>) {
        if self.language != language {
            self.language = language;
            cx.notify();
        }
    }

    fn expand_field(&mut self, end: bool, field: ExpandedField, cx: &mut Context<Self>) {
        // Do not discard invalid text when switching to another endpoint.
        if self.has_input_errors(cx) {
            return;
        }
        let close = self.editing_end == end && self.expanded == Some(field);
        self.editing_end = end;
        self.expanded = if close { None } else { Some(field) };
        self.warning_open = false;
        self.sync_inputs(cx);
        cx.notify();
    }
    fn select_date(&mut self, date: Date, cx: &mut Context<Self>) {
        if let Some(e) = self.endpoint_mut() {
            e.value.start_date = date;
        }
        self.expanded = None;
        // Selecting a recurrence date must preserve the in-progress repeat input,
        // including invalid text that the user still needs to correct.
        if !self.repeat_page {
            self.sync_inputs(cx);
        }
        self.changed(cx);
    }
    fn endpoint(&self) -> Option<&TimestampEndpoint> {
        let draft = self.draft.as_ref()?;
        if self.editing_end {
            draft.end.as_ref()
        } else {
            Some(&draft.start)
        }
    }
    fn endpoint_mut(&mut self) -> Option<&mut TimestampEndpoint> {
        let draft = self.draft.as_mut()?;
        if self.editing_end {
            draft.end.as_mut()
        } else {
            Some(&mut draft.start)
        }
    }
    fn sync_inputs(&mut self, cx: &mut Context<Self>) {
        let value = self.endpoint().map(|e| e.value.clone());
        if let Some(v) = &value {
            self.month = v.start_date;
            self.time_input.update(cx, |i, cx| {
                i.sync(&v.start_time.map(time_text).unwrap_or_default(), cx)
            });
            self.end_time_input.update(cx, |i, cx| {
                i.sync(&v.end_time.map(time_text).unwrap_or_default(), cx)
            });
            self.count_input.update(cx, |i, cx| {
                i.sync(&v.repeater.map_or(1, |r| r.value).to_string(), cx)
            });
        }
        self.warning_input.update(cx, |i, cx| {
            i.sync(
                &value
                    .as_ref()
                    .and_then(|v| v.warning)
                    .map_or(1, |w| w.value)
                    .to_string(),
                cx,
            );
        });
        for field in InputField::VALIDATED {
            self.set_input_invalid(field, false, cx);
        }
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(draft) = &self.draft {
            self.raw = draft.serialize();
        }
        self.source_input
            .update(cx, |input, cx| input.sync(&self.raw, cx));
        cx.notify();
    }
    fn input(&self, field: InputField) -> &Entity<NativeInput> {
        match field {
            InputField::StartTime => &self.time_input,
            InputField::EndTime => &self.end_time_input,
            InputField::RepeatCount => &self.count_input,
            InputField::Source => &self.source_input,
            InputField::WarningCount => &self.warning_input,
        }
    }
    fn input_invalid(&self, field: InputField, cx: &App) -> bool {
        self.input(field).read(cx).invalid
    }
    fn has_input_errors(&self, cx: &App) -> bool {
        InputField::VALIDATED
            .into_iter()
            .any(|field| self.input_invalid(field, cx))
    }
    fn set_input_invalid(&self, field: InputField, invalid: bool, cx: &mut Context<Self>) {
        self.input(field).update(cx, |input, cx| {
            if input.invalid != invalid {
                input.invalid = invalid;
                cx.notify();
            }
        });
    }
    fn input_changed(&mut self, field: InputField, text: &str, cx: &mut Context<Self>) {
        match field {
            InputField::Source => {
                self.expanded = None;
                self.raw = text.to_owned();
                self.draft = TimestampDraft::parse(text, self.kind);
                self.editing_end &= self.draft.as_ref().is_some_and(|d| d.end.is_some());
                self.sync_inputs(cx);
                cx.notify();
                return;
            }
            InputField::WarningCount => {
                let count = text.parse::<u32>().ok().filter(|n| *n <= 9999);
                self.set_input_invalid(field, count.is_none(), cx);
                if let Some(count) = count
                    && let Some(endpoint) = self.endpoint_mut()
                {
                    endpoint
                        .value
                        .warning
                        .get_or_insert(WarningPeriod {
                            delayed: false,
                            value: count,
                            unit: TimeUnit::Day,
                        })
                        .value = count;
                }
            }
            InputField::RepeatCount => {
                let count = text.parse::<u32>().ok().filter(|n| *n > 0 && *n <= 9999);
                self.set_input_invalid(field, count.is_none(), cx);
                if let Some(count) = count
                    && let Some(endpoint) = self.endpoint_mut()
                {
                    endpoint
                        .value
                        .repeater
                        .get_or_insert(Repeater {
                            mode: RepeaterMode::CatchUp,
                            value: count,
                            unit: TimeUnit::Week,
                        })
                        .value = count;
                }
            }
            InputField::StartTime | InputField::EndTime => {
                let value = parse_time(text);
                let invalid = !text.is_empty() && value.is_none();
                self.set_input_invalid(field, invalid, cx);
                if !invalid && let Some(endpoint) = self.endpoint_mut() {
                    if field == InputField::StartTime {
                        endpoint.value.start_time = value;
                        if value.is_none() {
                            endpoint.value.end_time = None;
                        }
                    } else {
                        endpoint.value.end_time = value;
                    }
                }
            }
        }
        self.changed(cx);
    }
    fn valid(&self, cx: &App) -> bool {
        !self.has_input_errors(cx)
            && (self.draft.as_ref().is_some_and(TimestampDraft::valid) || is_diary(&self.raw))
    }
    fn apply(&mut self, cx: &mut Context<Self>) {
        if self.valid(cx) {
            cx.emit(TimestampPickerEvent::Applied(self.raw.clone()));
        }
    }
    fn set_mode(&mut self, mode: RepeaterMode, cx: &mut Context<Self>) {
        if let Some(endpoint) = self.endpoint_mut() {
            endpoint
                .value
                .repeater
                .get_or_insert(Repeater {
                    mode,
                    value: 1,
                    unit: TimeUnit::Week,
                })
                .mode = mode;
        }
        self.changed(cx);
    }
}

fn parse_time(text: &str) -> Option<Time> {
    let (h, m) = text.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    Time::new(h.parse().ok()?, m.parse().ok()?, 0, 0).ok()
}

fn repeat_example(start: Date, repeat: Repeater, language: Language) -> String {
    // Examples use dates, never promise a wall-clock value for hourly repeats.
    if repeat.unit == TimeUnit::Hour {
        return language
            .text("timestamp.hourly_example")
            .replace("{count}", &repeat.value.to_string())
            .replace("{unit}", unit_text(repeat.unit, repeat.value, language))
            .replace(
                "{mode}",
                language.text(match repeat.mode {
                    RepeaterMode::Restart => "timestamp.hourly_restart",
                    RepeaterMode::Cumulative => "timestamp.hourly_cumulative",
                    RepeaterMode::CatchUp => "timestamp.hourly_catch_up",
                }),
            );
    }
    let Ok(completed) = start.checked_add(Span::new().days(15)) else {
        return String::new();
    };
    let span = match repeat.unit {
        TimeUnit::Day => Span::new().days(repeat.value),
        TimeUnit::Week => Span::new().weeks(repeat.value),
        TimeUnit::Month => Span::new().months(repeat.value),
        TimeUnit::Year => Span::new().years(repeat.value),
        TimeUnit::Hour => unreachable!(),
    };
    let mut next = if repeat.mode == RepeaterMode::Restart {
        completed
    } else {
        start
    };
    for _ in 0..17 {
        let Ok(date) = next.checked_add(span) else {
            return language.text("timestamp.example_overflow").into();
        };
        next = date;
        if repeat.mode != RepeaterMode::CatchUp || next > completed {
            break;
        }
    }
    let date = |date: Date| super::calendar::date_label(date, language);
    language
        .text("timestamp.example_dates")
        .replace("{start}", &date(start))
        .replace("{count}", &repeat.value.to_string())
        .replace("{unit}", unit_text(repeat.unit, repeat.value, language))
        .replace("{completed}", &date(completed))
        .replace("{next}", &date(next))
}

fn unit_text(unit: TimeUnit, count: u32, language: Language) -> &'static str {
    language.text(match (unit, count == 1) {
        (TimeUnit::Hour, true) => "timestamp.unit.hour",
        (TimeUnit::Hour, false) => "timestamp.unit.hour_plural",
        (TimeUnit::Day, true) => "timestamp.unit.day",
        (TimeUnit::Day, false) => "timestamp.unit.day_plural",
        (TimeUnit::Week, true) => "timestamp.unit.week",
        (TimeUnit::Week, false) => "timestamp.unit.week_plural",
        (TimeUnit::Month, true) => "timestamp.unit.month",
        (TimeUnit::Month, false) => "timestamp.unit.month_plural",
        (TimeUnit::Year, true) => "timestamp.unit.year",
        (TimeUnit::Year, false) => "timestamp.unit.year_plural",
    })
}

#[cfg(test)]
mod tests;
