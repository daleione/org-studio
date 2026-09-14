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
    picker.update(cx, |p, cx| {
        p.input_changed(InputField::RepeatCount, "3", cx)
    });
    click(cx, "repeat-unit");
    click(cx, "months");
    picker.update(cx, |p, cx| {
        p.input_changed(InputField::RepeatCount, "0", cx)
    });
    click(cx, "repeat-start-date");
    click(cx, "calendar-day-2026-10-12");
    click(cx, "repeat-done");
    cx.read(|cx| {
        let p = picker.read(cx);
        assert!(p.repeat_page);
        assert!(p.input_invalid(InputField::RepeatCount, cx));
        assert_eq!(p.raw, "<2026-10-12 Mon 14:00 .+3m>");
    });
    picker.update(cx, |p, cx| {
        p.input_changed(InputField::RepeatCount, "2", cx)
    });
    click(cx, "repeat-done");
    cx.read(|cx| {
        let p = picker.read(cx);
        assert!(!p.repeat_page);
        assert!(p.valid(cx));
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
    cx.read(|cx| assert!(!picker.read(cx).valid(cx)));
    click(cx, "end-date");
    click(cx, "next");
    click(cx, "calendar-day-2026-09-30");
    cx.read(|cx| assert!(picker.read(cx).valid(cx)));
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
    picker.update(cx, |p, cx| {
        p.input_changed(InputField::StartTime, "99:99", cx)
    });
    click(cx, "start-date");
    cx.read(|cx| {
        assert!(picker.read(cx).editing_end);
        assert_eq!(picker.read(cx).expanded, Some(ExpandedField::Time));
    });
    picker.update(cx, |p, cx| {
        p.input_changed(InputField::StartTime, "19:30", cx)
    });
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
        p.input_changed(InputField::StartTime, "99:99", cx);
        assert!(!p.valid(cx));
        assert_eq!(p.raw, source);
        p.input_changed(InputField::StartTime, "11:45", cx);
        assert!(p.valid(cx));
        assert!(p.raw.starts_with("<2026-09-15 Tue 09:00-10:00 +1w>--"));
        assert!(p.raw.ends_with("Fri 11:45-12:00 ++2m -3d>"));
        p.input_changed(InputField::RepeatCount, "0", cx);
        assert!(!p.valid(cx));
        p.input_changed(InputField::RepeatCount, "3", cx);
        assert!(p.valid(cx));
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
        p.input_changed(InputField::RepeatCount, "4", cx);
        assert!(p.raw.ends_with("++4w>"));
        p.input_changed(InputField::WarningCount, "0", cx);
        assert!(p.valid(cx));
        assert!(p.raw.ends_with("++4w -0d>"));
        p.input_changed(InputField::Source, "<2026-02-30>", cx);
        assert!(!p.valid(cx));
        p.input_changed(
            InputField::Source,
            "<%%(diary-float t 4 2) 22:00-23:00>",
            cx,
        );
        assert!(p.valid(cx));
        assert!(p.draft.is_none());
    });
}

#[gpui::test]
fn valid_source_replaces_invalid_field_text_and_clears_error_borders(
    cx: &mut gpui::TestAppContext,
) {
    let picker = cx.new(|cx| {
        TimestampPicker::new(
            "<2026-09-15 Tue 09:00-10:00 ++1w -2d>".into(),
            TimestampKind::Scheduled,
            Date::new(2026, 9, 14).unwrap(),
            Language::English,
            cx,
        )
    });
    picker.update(cx, |picker, cx| {
        for (field, text) in [
            (InputField::StartTime, "99:99"),
            (InputField::EndTime, "invalid"),
            (InputField::RepeatCount, "0"),
            (InputField::WarningCount, "-1"),
        ] {
            picker
                .input(field)
                .update(cx, |input, cx| input.sync(text, cx));
            picker.input_changed(field, text, cx);
            assert!(picker.input(field).read(cx).invalid);
        }
        assert!(!picker.valid(cx));
        let corrected = "<2026-09-16 Wed 11:00-12:00 ++2w -3d>";
        picker.input_changed(InputField::Source, corrected, cx);
        assert_eq!(picker.raw, corrected);
        assert!(picker.valid(cx));
        for (field, text) in [
            (InputField::StartTime, "11:00"),
            (InputField::EndTime, "12:00"),
            (InputField::RepeatCount, "2"),
            (InputField::WarningCount, "3"),
        ] {
            let input = picker.input(field).read(cx);
            assert!(!input.invalid, "valid source must clear stale red borders");
            assert_eq!(input.text, text);
        }
    });
}
