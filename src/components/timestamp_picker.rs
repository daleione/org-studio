//! Reusable Org timestamp editor. Emits an applied source value or cancellation;
//! it never reads an editor, writes a document, or owns its popup position.
mod view;

use std::rc::Rc;

use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, MouseButton, Render, Subscription,
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
    warning_error: bool,
    time_error: bool,
    end_time_error: bool,
    count_error: bool,
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
            time_input.clone(),
            end_time_input.clone(),
            count_input.clone(),
            source_input.clone(),
            warning_input.clone(),
        ]
        .into_iter()
        .enumerate()
        {
            subscriptions.push(
                cx.subscribe(
                    &entity,
                    move |this, _, event: &InputEvent, cx| match event {
                        InputEvent::Changed(text) => this.input_changed(field, text, cx),
                        InputEvent::Command { key, .. } if key == "escape" => {
                            cx.emit(TimestampPickerEvent::Cancelled)
                        }
                        InputEvent::Command { key, .. } if key == "enter" => {
                            if field < 2 && this.expanded == Some(ExpandedField::Time) {
                                if !this.time_error && !this.end_time_error {
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
            warning_error: false,
            time_error: false,
            end_time_error: false,
            count_error: false,
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
        if self.time_error || self.end_time_error || self.count_error || self.warning_error {
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
            i.invalid = false;
        });
        self.warning_error = false;
        self.time_error = false;
        self.end_time_error = false;
        self.count_error = false;
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(draft) = &self.draft {
            self.raw = draft.serialize();
        }
        self.source_input
            .update(cx, |input, cx| input.sync(&self.raw, cx));
        cx.notify();
    }
    fn input_changed(&mut self, field: usize, text: &str, cx: &mut Context<Self>) {
        if field == 3 {
            self.expanded = None;
            self.raw = text.to_owned();
            self.draft = TimestampDraft::parse(text, self.kind);
            self.editing_end &= self.draft.as_ref().is_some_and(|d| d.end.is_some());
            self.sync_inputs(cx);
            cx.notify();
            return;
        }
        if field == 4 {
            let count = text.parse::<u32>().ok().filter(|n| *n <= 9999);
            self.warning_error = count.is_none();
            self.warning_input.update(cx, |input, cx| {
                input.invalid = count.is_none();
                cx.notify();
            });
            if let Some(count) = count
                && let Some(e) = self.endpoint_mut()
            {
                e.value
                    .warning
                    .get_or_insert(WarningPeriod {
                        delayed: false,
                        value: count,
                        unit: TimeUnit::Day,
                    })
                    .value = count;
            }
            self.changed(cx);
            return;
        }
        if field == 2 {
            let count = text.parse::<u32>().ok().filter(|n| *n > 0 && *n <= 9999);
            self.count_error = count.is_none();
            self.count_input.update(cx, |i, cx| {
                i.invalid = count.is_none();
                cx.notify();
            });
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
        } else {
            let value = if text.is_empty() {
                None
            } else {
                parse_time(text)
            };
            let invalid = !text.is_empty() && value.is_none();
            if field == 0 {
                self.time_error = invalid;
            } else {
                self.end_time_error = invalid;
            }
            let input = if field == 0 {
                &self.time_input
            } else {
                &self.end_time_input
            };
            input.update(cx, |i, cx| {
                i.invalid = invalid;
                cx.notify();
            });
            if !invalid && let Some(endpoint) = self.endpoint_mut() {
                if field == 0 {
                    endpoint.value.start_time = value;
                    if value.is_none() {
                        endpoint.value.end_time = None;
                    }
                } else {
                    endpoint.value.end_time = value;
                }
            }
        }
        self.changed(cx);
    }
    fn valid(&self) -> bool {
        !self.time_error
            && !self.end_time_error
            && !self.count_error
            && !self.warning_error
            && (self.draft.as_ref().is_some_and(TimestampDraft::valid) || is_diary(&self.raw))
    }
    fn apply(&mut self, cx: &mut Context<Self>) {
        if self.valid() {
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
mod tests {
    use super::*;

    fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        let point = cx.debug_bounds(selector).unwrap().center();
        cx.simulate_click(point, gpui::Modifiers::default());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn repeat_start_date_stays_in_repeat_settings_until_done(cx: &mut gpui::TestAppContext) {
        cx.update(init);
        let (picker, cx) = cx.add_window_view(|_, cx| {
            TimestampPicker::new(
                "<2026-09-15 Tue 14:00>".into(),
                TimestampKind::Plain,
                Date::new(2026, 9, 14).unwrap(),
                Language::Chinese,
                cx,
            )
        });
        cx.simulate_resize(gpui::size(px(500.), px(1100.)));
        cx.run_until_parked();
        click(cx, "repeat-page");
        click(cx, "repeat-start-date");
        assert!(cx.debug_bounds("single").is_none());
        assert!(cx.debug_bounds("repeat-done").is_some());
        click(cx, "next");
        click(cx, "calendar-day-2026-10-05");
        assert!(cx.debug_bounds("inline-calendar").is_none());
        cx.read(|cx| {
            let p = picker.read(cx);
            assert!(p.repeat_page);
            assert_eq!(p.raw, "<2026-10-05 Mon 14:00>");
        });
        click(cx, "repeat-done");
        cx.read(|cx| {
            let p = picker.read(cx);
            assert!(!p.repeat_page);
            assert_eq!(p.raw, "<2026-10-05 Mon 14:00 ++1w>");
        });

        // A custom repeat and unfinished invalid input survive date selection too.
        click(cx, "repeat-page");
        click(cx, "restart");
        picker.update(cx, |p, cx| p.input_changed(2, "3", cx));
        click(cx, "repeat-unit");
        click(cx, "months");
        picker.update(cx, |p, cx| p.input_changed(2, "0", cx));
        click(cx, "repeat-start-date");
        click(cx, "calendar-day-2026-10-12");
        click(cx, "repeat-done");
        cx.read(|cx| {
            let p = picker.read(cx);
            assert!(p.repeat_page);
            assert!(p.count_error);
            assert_eq!(p.raw, "<2026-10-12 Mon 14:00 .+3m>");
        });
        picker.update(cx, |p, cx| p.input_changed(2, "2", cx));
        click(cx, "repeat-done");
        cx.read(|cx| {
            let p = picker.read(cx);
            assert!(!p.repeat_page);
            assert!(p.valid());
            assert_eq!(p.raw, "<2026-10-12 Mon 14:00 .+2m>");
        });
    }

    #[gpui::test]
    fn calendar_expands_on_demand_and_keeps_seven_columns(cx: &mut gpui::TestAppContext) {
        cx.update(init);
        let (picker, cx) = cx.add_window_view(|_, cx| {
            TimestampPicker::new(
                "<2026-09-15 Tue 14:00>".into(),
                TimestampKind::Plain,
                Date::new(2026, 9, 14).unwrap(),
                Language::Chinese,
                cx,
            )
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("inline-calendar").is_none());
        assert!(cx.debug_bounds("end-date").is_none());
        click(cx, "start-date");
        for (language, width) in [
            (Language::Chinese, 300.),
            (Language::Chinese, 360.),
            (Language::Chinese, 480.),
            (Language::English, 300.),
            (Language::English, 400.),
            (Language::English, 480.),
        ] {
            picker.update(cx, |p, cx| p.set_language(language, cx));
            cx.simulate_resize(gpui::size(px(width), px(850.)));
            cx.run_until_parked();
            let mon = cx.debug_bounds("calendar-weekday-0").unwrap();
            let sun = cx.debug_bounds("calendar-weekday-6").unwrap();
            let first = cx.debug_bounds("calendar-day-2026-08-31").unwrap();
            let sunday = cx.debug_bounds("calendar-day-2026-09-06").unwrap();
            let tuesday = cx.debug_bounds("calendar-day-2026-09-15").unwrap();
            let next = cx.debug_bounds("calendar-day-2026-09-16").unwrap();
            assert!((f32::from(mon.top() - sun.top())).abs() < 0.5);
            assert!((f32::from(first.top() - sunday.top())).abs() < 0.5);
            assert!((f32::from(sun.center().x - sunday.center().x)).abs() < 1.0);
            assert!((f32::from(tuesday.top() - next.top())).abs() < 0.5);
        }
        click(cx, "calendar-day-2026-09-18");
        assert!(cx.debug_bounds("inline-calendar").is_none());
        cx.read(|cx| assert_eq!(picker.read(cx).raw, "<2026-09-18 Fri 14:00>"));
        click(cx, "range");
        assert!(cx.debug_bounds("end-date").is_some());
        assert!(cx.debug_bounds("inline-calendar").is_none());
        click(cx, "end-date");
        click(cx, "next");
        click(cx, "calendar-day-2026-10-05");
        cx.read(|cx| {
            assert_eq!(
                picker.read(cx).raw,
                "<2026-09-18 Fri 14:00>--<2026-10-05 Mon>"
            )
        });
        click(cx, "single");
        cx.read(|cx| assert_eq!(picker.read(cx).raw, "<2026-09-18 Fri 14:00>"));
    }

    #[gpui::test]
    fn inline_range_endpoints_navigate_months_independently(cx: &mut gpui::TestAppContext) {
        cx.update(init);
        let (picker, cx) = cx.add_window_view(|_, cx| {
            TimestampPicker::new(
                "<2026-09-15 Tue 14:00 +1w>--[2026-09-18 Fri 18:00 ++2m -3d]".into(),
                TimestampKind::Plain,
                Date::new(2026, 9, 14).unwrap(),
                Language::Chinese,
                cx,
            )
        });
        cx.simulate_resize(gpui::size(px(950.), px(850.)));
        cx.run_until_parked();
        assert!(f32::from(cx.debug_bounds("timestamp-picker").unwrap().size.width) <= 360.);
        click(cx, "end-date");
        for _ in 0..4 {
            click(cx, "next");
        }
        click(cx, "calendar-day-2027-01-05");
        assert!(cx.debug_bounds("inline-calendar").is_none());
        cx.read(|cx| {
            assert_eq!(
                picker.read(cx).raw,
                "<2026-09-15 Tue 14:00 +1w>--[2027-01-05 Tue 18:00 ++2m -3d]"
            );
        });
        click(cx, "start-date");
        cx.read(|cx| assert_eq!(picker.read(cx).month.month(), 9));
        click(cx, "calendar-day-2026-09-22");
        cx.read(|cx| {
            assert_eq!(
                picker.read(cx).raw,
                "<2026-09-22 Tue 14:00 +1w>--[2027-01-05 Tue 18:00 ++2m -3d]"
            );
        });
        click(cx, "end-date");
        // An invalid date order stays editable and cannot be applied.
        for _ in 0..5 {
            click(cx, "previous");
        }
        click(cx, "calendar-day-2026-08-05");
        cx.read(|cx| assert!(!picker.read(cx).valid()));
        click(cx, "end-date");
        click(cx, "next");
        click(cx, "calendar-day-2026-09-30");
        cx.read(|cx| assert!(picker.read(cx).valid()));
    }

    #[gpui::test]
    fn time_pills_edit_only_the_selected_endpoint(cx: &mut gpui::TestAppContext) {
        cx.update(init);
        let (picker, cx) = cx.add_window_view(|_, cx| {
            TimestampPicker::new(
                "<2026-09-15 Tue 09:00-10:00 +1w>--<2026-09-18 Fri 18:00>".into(),
                TimestampKind::Plain,
                Date::new(2026, 9, 14).unwrap(),
                Language::Chinese,
                cx,
            )
        });
        cx.run_until_parked();
        click(cx, "end-time");
        picker.update(cx, |p, cx| p.input_changed(0, "99:99", cx));
        click(cx, "start-date");
        cx.read(|cx| {
            assert!(picker.read(cx).editing_end);
            assert_eq!(picker.read(cx).expanded, Some(ExpandedField::Time));
        });
        picker.update(cx, |p, cx| p.input_changed(0, "19:30", cx));
        click(cx, "time-done");
        cx.read(|cx| {
            assert!(picker.read(cx).expanded.is_none());
            assert_eq!(
                picker.read(cx).raw,
                "<2026-09-15 Tue 09:00-10:00 +1w>--<2026-09-18 Fri 19:30>"
            );
        });
        click(cx, "start-time");
        click(cx, "date-only");
        click(cx, "time-done");
        cx.read(|cx| {
            assert_eq!(
                picker.read(cx).raw,
                "<2026-09-15 Tue  +1w>--<2026-09-18 Fri 19:30>"
            )
        });
    }

    #[test]
    fn recurrence_examples_match_org_completion_semantics() {
        let start = Date::new(2026, 9, 15).unwrap();
        for (mode, expected) in [
            (RepeaterMode::Cumulative, "9月22日"),
            (RepeaterMode::CatchUp, "10月6日"),
            (RepeaterMode::Restart, "10月7日"),
        ] {
            assert!(
                repeat_example(
                    start,
                    Repeater {
                        mode,
                        value: 1,
                        unit: TimeUnit::Week
                    },
                    Language::Chinese
                )
                .ends_with(expected)
            );
        }
        let english = repeat_example(
            start,
            Repeater {
                mode: RepeaterMode::CatchUp,
                value: 1,
                unit: TimeUnit::Week,
            },
            Language::English,
        );
        assert_eq!(
            english,
            "Originally 2026-09-15, every 1 week\nIf completed on 2026-09-30\nNext: 2026-10-06"
        );
        assert_eq!(unit_text(TimeUnit::Week, 2, Language::English), "weeks");
        assert_eq!(
            Language::English.text("timestamp.add_to_agenda"),
            "Add to agenda"
        );
        assert_eq!(
            Language::Chinese.text("timestamp.add_to_agenda"),
            "加入日程"
        );
    }

    #[gpui::test]
    fn inputs_validate_and_edit_independent_range_endpoints(cx: &mut gpui::TestAppContext) {
        let source = "<2026-09-15 Tue 09:00-10:00 +1w>--<2026-09-18 Fri 11:00-12:00 ++2m -3d>";
        let picker = cx.new(|cx| {
            TimestampPicker::new(
                source.into(),
                TimestampKind::Plain,
                Date::new(2026, 9, 14).unwrap(),
                Language::Chinese,
                cx,
            )
        });
        picker.update(cx, |p, cx| {
            p.editing_end = true;
            p.sync_inputs(cx);
            p.input_changed(0, "99:99", cx);
            assert!(!p.valid());
            assert_eq!(p.raw, source);
            p.input_changed(0, "11:45", cx);
            assert!(p.valid());
            assert!(p.raw.starts_with("<2026-09-15 Tue 09:00-10:00 +1w>--"));
            assert!(p.raw.ends_with("Fri 11:45-12:00 ++2m -3d>"));
            p.input_changed(2, "0", cx);
            assert!(!p.valid());
            p.input_changed(2, "3", cx);
            assert!(p.valid());
            assert!(p.raw.ends_with("++3m -3d>"));
        });
    }

    #[gpui::test]
    fn source_edits_and_warning_values_are_validated(cx: &mut gpui::TestAppContext) {
        let picker = cx.new(|cx| {
            TimestampPicker::new(
                "<2026-09-15 Tue>".into(),
                TimestampKind::Scheduled,
                Date::new(2026, 9, 14).unwrap(),
                Language::Chinese,
                cx,
            )
        });
        picker.update(cx, |p, cx| {
            p.input_changed(2, "4", cx);
            assert!(p.raw.ends_with("++4w>"));
            p.input_changed(4, "0", cx);
            assert!(p.valid());
            assert!(p.raw.ends_with("++4w -0d>"));
            p.input_changed(3, "<2026-02-30>", cx);
            assert!(!p.valid());
            p.input_changed(3, "<%%(diary-float t 4 2) 22:00-23:00>", cx);
            assert!(p.valid());
            assert!(p.draft.is_none());
        });
    }
}
