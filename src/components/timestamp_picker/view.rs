//! Calendar and repeat subviews for the shared timestamp picker.
use super::*;

impl TimestampPicker {
    fn button(
        &self,
        id: &'static str,
        text: impl Into<gpui::SharedString>,
        selected: bool,
        cx: &Context<Self>,
        click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .debug_selector(move || id.to_owned())
            .px_2()
            .py_1()
            .rounded(px(7.))
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(rgb(if selected { 0xffffff } else { 0x3f78f2 }))
            .when(selected, |s| s.bg(rgb(0x3f78f2)))
            .hover(|s| s.bg(rgb(if selected { 0x3269df } else { 0xeaf1ff })))
            .child(text.into())
            .on_click(cx.listener(move |this, _, window, cx| click(this, window, cx)))
    }
    fn segmented() -> gpui::Div {
        div()
            .flex()
            .w_full()
            .p(px(3.))
            .gap(px(3.))
            .rounded(px(9.))
            .bg(rgb(0xf4f5f8))
            .border_1()
            .border_color(rgb(0xe9ebf0))
    }
    fn segment(
        &self,
        id: &'static str,
        label: &'static str,
        selected: bool,
        cx: &Context<Self>,
        click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> gpui::Stateful<gpui::Div> {
        self.button(id, label, selected, cx, click)
            .flex_1()
            .min_w_0()
            .flex()
            .justify_center()
            .items_center()
            .when(!selected, |s| s.text_color(rgb(0x555b68)))
    }
    fn icon(name: &'static str) -> gpui::Svg {
        gpui::svg()
            .path(format!("assets/icons/agenda/{name}.svg"))
            .size(px(16.))
            .flex_none()
            .text_color(rgb(0x626c7d))
    }
    fn row(&self, label: impl Into<gpui::SharedString>) -> gpui::Div {
        let label = label.into();
        let icon = if label.as_ref() == self.t("timestamp.time") {
            Some("clock")
        } else if label.as_ref() == self.t("timestamp.repeat") {
            Some("arrows-clockwise")
        } else if label.as_ref() == self.t("timestamp.add_to_agenda") {
            Some("calendar-blank")
        } else {
            None
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .min_h(px(36.))
            .when_some(icon, |s, icon| s.child(Self::icon(icon)))
            .child(div().flex_1().min_w_0().child(label))
    }
    fn finish_repeat(&mut self, cx: &mut Context<Self>) {
        if self.count_error {
            return;
        }
        if self.endpoint().is_some_and(|e| e.value.repeater.is_none()) {
            self.set_mode(RepeaterMode::CatchUp, cx);
        }
        self.repeat_page = false;
        self.expanded = None;
        self.unit_menu = false;
        self.sync_inputs(cx);
        self.changed(cx);
    }
    fn group() -> gpui::Div {
        div()
            .p_2()
            .rounded(px(11.))
            .bg(rgb(0xf5f6f8))
            .flex()
            .flex_col()
            .gap_1()
    }

    fn date_editor(&self, cx: &Context<Self>) -> gpui::Div {
        let entity = cx.entity().downgrade();
        let month_entity = entity.clone();
        let mut panel = div()
            .debug_selector(|| "inline-calendar".into())
            .p_2()
            .border_t_1()
            .border_color(rgb(0xe9ebf0))
            .child(
                Calendar {
                    language: self.language,
                    month: self.month,
                    selected: self.endpoint().unwrap().value.start_date,
                    range: None,
                    today: self.today,
                    drag: self.calendar_drag.clone(),
                    hide_outside_month: false,
                    navigation: (true, true),
                    on_preview: Rc::new(|_, _, _| {}),
                    on_range: Rc::new(|_, _, _, _| {}),
                    on_select: Rc::new(move |date, window, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.select_date(date, cx);
                            window.focus(&this.focus, cx);
                        });
                    }),
                    on_month: Rc::new(move |date, _, cx| {
                        let _ = month_entity.update(cx, |this, cx| {
                            this.month = date;
                            cx.notify();
                        });
                    }),
                }
                .render(),
            );
        let mut shortcuts = div().flex().gap_2().mt_2();
        for (id, label, days) in [
            ("today", self.t("timestamp.today"), 0),
            ("tomorrow", self.t("timestamp.tomorrow"), 1),
            ("next-week", self.t("timestamp.next_week"), 7),
        ] {
            shortcuts = shortcuts.child(self.button(id, label, false, cx, move |this, _, cx| {
                if let Ok(date) = this.today.checked_add(Span::new().days(days)) {
                    this.select_date(date, cx);
                }
            }));
        }
        panel = panel.child(shortcuts);
        panel
    }

    fn time_editor(&self, cx: &Context<Self>) -> gpui::Div {
        let value = &self.endpoint().unwrap().value;
        let mut group = div().flex().flex_col().gap_1().p_2();
        let mut time_row = self.row(self.t("timestamp.time"));
        if value.start_time.is_some() || self.time_error {
            time_row = time_row
                .child(div().w(px(78.)).child(self.time_input.clone()))
                .child(self.button(
                    "date-only",
                    self.t("timestamp.date_only"),
                    false,
                    cx,
                    |this, _, cx| {
                        if let Some(e) = this.endpoint_mut() {
                            e.value.start_time = None;
                            e.value.end_time = None;
                        }
                        this.sync_inputs(cx);
                        this.changed(cx);
                    },
                ));
        } else {
            time_row = time_row.child(self.button(
                "add-time",
                self.t("timestamp.add_time"),
                false,
                cx,
                |this, _, cx| {
                    if let Some(e) = this.endpoint_mut() {
                        e.value.start_time = Time::new(9, 0, 0, 0).ok();
                    }
                    this.sync_inputs(cx);
                    this.changed(cx);
                },
            ));
        }
        group = group.child(time_row);
        if value.start_time.is_some() {
            group = group.child(if value.end_time.is_some() || self.end_time_error {
                self.row(self.t("timestamp.end_time"))
                    .child(div().w(px(78.)).child(self.end_time_input.clone()))
                    .child(self.button(
                        "remove-end-time",
                        self.t("timestamp.remove"),
                        false,
                        cx,
                        |this, _, cx| {
                            if let Some(e) = this.endpoint_mut() {
                                e.value.end_time = None;
                            }
                            this.sync_inputs(cx);
                            this.changed(cx);
                        },
                    ))
            } else {
                self.row("").min_h(px(24.)).child(self.button(
                    "add-end-time",
                    self.t("timestamp.add_end_time"),
                    false,
                    cx,
                    |this, _, cx| {
                        if let Some(e) = this.endpoint_mut() {
                            e.value.end_time = e
                                .value
                                .start_time
                                .and_then(|t| t.checked_add(Span::new().hours(1)).ok());
                        }
                        this.sync_inputs(cx);
                        this.changed(cx);
                    },
                ))
            });
        }

        group.child(self.row("").child(self.button(
            "time-done",
            self.t("timestamp.done"),
            false,
            cx,
            |this, window, cx| {
                if !this.time_error && !this.end_time_error {
                    this.expanded = None;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }
            },
        )))
    }

    fn calendar_page(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let Some(endpoint) = self.endpoint() else {
            return div()
                .p_3()
                .rounded_lg()
                .bg(rgb(0xf5f6f8))
                .child(self.t("timestamp.special"))
                .into_any_element();
        };
        let value = &endpoint.value;
        let draft = self.draft.as_ref().unwrap();
        let has_range = draft.end.is_some();
        let mut page = div().flex_none().flex().flex_col().gap_3().child(
            Self::segmented()
                .child(self.segment(
                    "single",
                    self.t("timestamp.single"),
                    !has_range,
                    cx,
                    |this, _, cx| {
                        if let Some(d) = &mut this.draft {
                            d.set_range(false);
                        }
                        this.editing_end = false;
                        this.expanded = None;
                        this.sync_inputs(cx);
                        this.changed(cx);
                    },
                ))
                .child(self.segment(
                    "range",
                    self.t("timestamp.range"),
                    has_range,
                    cx,
                    |this, _, cx| {
                        if let Some(d) = &mut this.draft {
                            d.set_range(true);
                        }
                        this.changed(cx);
                    },
                )),
        );
        let mut dates = div()
            .flex()
            .flex_col()
            .px_2()
            .rounded(px(11.))
            .border_1()
            .border_color(rgb(0xe9ebf0));
        for (end, endpoint) in [(false, Some(&draft.start)), (true, draft.end.as_ref())] {
            let Some(endpoint) = endpoint else {
                continue;
            };
            let date = endpoint.value.start_date;
            let time = endpoint.value.start_time.map_or_else(
                || self.t("timestamp.all_day").into(),
                |start| {
                    endpoint.value.end_time.map_or_else(
                        || time_text(start),
                        |end| format!("{}–{}", time_text(start), time_text(end)),
                    )
                },
            );
            let selected = self.editing_end == end;
            dates = dates.child(
                self.row(if has_range {
                    if end {
                        self.t("timestamp.end")
                    } else {
                        self.t("timestamp.start")
                    }
                } else {
                    self.t("timestamp.date")
                })
                .min_h(px(48.))
                .when(end, |s| s.border_t_1().border_color(rgb(0xe9ebf0)))
                .child(
                    self.button(
                        if end { "end-date" } else { "start-date" },
                        super::super::calendar::date_label(date, self.language),
                        selected && self.expanded == Some(ExpandedField::Date),
                        cx,
                        move |this, _, cx| this.expand_field(end, ExpandedField::Date, cx),
                    )
                    .when(
                        !(selected && self.expanded == Some(ExpandedField::Date)),
                        |s| s.bg(rgb(0xf0f1f4)).text_color(rgb(0x373942)),
                    ),
                )
                .child(
                    self.button(
                        if end { "end-time" } else { "start-time" },
                        time,
                        selected && self.expanded == Some(ExpandedField::Time),
                        cx,
                        move |this, _, cx| this.expand_field(end, ExpandedField::Time, cx),
                    )
                    .when(
                        !(selected && self.expanded == Some(ExpandedField::Time)),
                        |s| s.bg(rgb(0xf0f1f4)).text_color(rgb(0x373942)),
                    ),
                ),
            );
            if selected {
                dates = match self.expanded {
                    Some(ExpandedField::Date) => dates.child(self.date_editor(cx)),
                    Some(ExpandedField::Time) => dates.child(self.time_editor(cx)),
                    None => dates,
                };
            }
        }
        page = page.child(dates);
        if has_range {
            page = page.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(rgb(0x92949d))
                            .child(self.t("timestamp.endpoint_settings")),
                    )
                    .child(self.button(
                        "start-settings",
                        self.t("timestamp.start"),
                        !self.editing_end,
                        cx,
                        |this, _, cx| {
                            if !this.time_error
                                && !this.end_time_error
                                && !this.count_error
                                && !this.warning_error
                            {
                                this.editing_end = false;
                                this.expanded = None;
                                this.sync_inputs(cx);
                                cx.notify();
                            }
                        },
                    ))
                    .child(self.button(
                        "end-settings",
                        self.t("timestamp.end"),
                        self.editing_end,
                        cx,
                        |this, _, cx| {
                            if !this.time_error
                                && !this.end_time_error
                                && !this.count_error
                                && !this.warning_error
                            {
                                this.editing_end = true;
                                this.expanded = None;
                                this.sync_inputs(cx);
                                cx.notify();
                            }
                        },
                    )),
            );
        }
        let mut group = Self::group();
        if value.active {
            let summary = value.repeater.map_or_else(
                || self.t("timestamp.none").to_owned(),
                |r| {
                    self.t("timestamp.repeat_summary")
                        .replace("{count}", &r.value.to_string())
                        .replace("{unit}", unit_text(r.unit, r.value, self.language))
                        .replace(
                            "{mode}",
                            match r.mode {
                                RepeaterMode::Cumulative => self.t("timestamp.cumulative"),
                                RepeaterMode::CatchUp => self.t("timestamp.catch_up_short"),
                                RepeaterMode::Restart => self.t("timestamp.restart"),
                            },
                        )
                },
            );
            group = group.child(self.row(self.t("timestamp.repeat")).child(self.button(
                "repeat-page",
                format!("{summary}  ›"),
                false,
                cx,
                |this, _, cx| {
                    this.repeat_page = true;
                    this.expanded = None;
                    cx.notify();
                },
            )));
        }
        group = group
            .child(
                self.row(self.t("timestamp.add_to_agenda")).child(
                    div()
                        .id("active")
                        .w(px(36.))
                        .h(px(22.))
                        .p(px(3.))
                        .rounded_full()
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .when(value.active, |s| s.justify_end())
                        .bg(rgb(if value.active { 0x3f78f2 } else { 0xd9dce3 }))
                        .child(
                            div()
                                .size(px(16.))
                                .rounded_full()
                                .bg(rgb(0xffffff))
                                .shadow_sm(),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(e) = this.endpoint_mut() {
                                e.value.active = !e.value.active;
                            }
                            this.changed(cx);
                        })),
                ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(0x92949d))
                    .child(if value.active {
                        self.t("timestamp.inactive_hint")
                    } else {
                        self.t("timestamp.inactive")
                    }),
            );
        page = page.child(group);
        if matches!(
            value.kind,
            TimestampKind::Deadline | TimestampKind::Scheduled
        ) || value.warning.is_some()
        {
            let label = match value.kind {
                TimestampKind::Scheduled => self.t("timestamp.delay"),
                TimestampKind::Deadline => self.t("timestamp.warning"),
                _ => self.t("timestamp.extra_rules"),
            };
            let summary = value.warning.map_or_else(
                || self.t("timestamp.default").to_owned(),
                |w| format!("{} {}", w.value, unit_text(w.unit, w.value, self.language)),
            );
            let mut warning = Self::group().child(self.row(label).child(self.button(
                "warning-open",
                format!("{summary}  ›"),
                false,
                cx,
                |this, _, cx| {
                    this.warning_open = !this.warning_open;
                    cx.notify();
                },
            )));
            if self.warning_open {
                warning = warning.child(
                    self.row(self.t("timestamp.interval"))
                        .child(div().w(px(65.)).child(self.warning_input.clone())),
                );
                let mut units = div().flex().gap_1();
                for (id, unit) in [
                    ("warning-days", TimeUnit::Day),
                    ("warning-weeks", TimeUnit::Week),
                    ("warning-months", TimeUnit::Month),
                    ("warning-years", TimeUnit::Year),
                ] {
                    units = units.child(self.button(
                        id,
                        unit_text(unit, 1, self.language),
                        value.warning.is_some_and(|w| w.unit == unit),
                        cx,
                        move |this, _, cx| {
                            if let Some(e) = this.endpoint_mut() {
                                e.value
                                    .warning
                                    .get_or_insert(WarningPeriod {
                                        delayed: false,
                                        value: 1,
                                        unit,
                                    })
                                    .unit = unit;
                            }
                            this.changed(cx);
                        },
                    ));
                }
                warning = warning.child(units);
                if value.kind == TimestampKind::Scheduled {
                    warning = warning.child(self.button(
                        "first-only",
                        if value.warning.is_some_and(|w| w.delayed) {
                            self.t("timestamp.first_only")
                        } else {
                            self.t("timestamp.every_delay")
                        },
                        false,
                        cx,
                        |this, _, cx| {
                            if let Some(e) = this.endpoint_mut() {
                                let w = e.value.warning.get_or_insert(WarningPeriod {
                                    delayed: false,
                                    value: 1,
                                    unit: TimeUnit::Day,
                                });
                                w.delayed = !w.delayed;
                            }
                            this.changed(cx);
                        },
                    ));
                }
                warning = warning.child(self.button(
                    "warning-none",
                    self.t("timestamp.reset"),
                    false,
                    cx,
                    |this, _, cx| {
                        if let Some(e) = this.endpoint_mut() {
                            e.value.warning = None;
                        }
                        this.warning_open = false;
                        this.sync_inputs(cx);
                        this.changed(cx);
                    },
                ));
            }
            page = page.child(warning);
        }
        page.into_any_element()
    }

    fn repeat_page(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let Some(value) = self.endpoint().map(|e| &e.value) else {
            return div().into_any_element();
        };
        let repeater = value.repeater.unwrap_or(Repeater {
            mode: RepeaterMode::CatchUp,
            value: 1,
            unit: TimeUnit::Week,
        });
        let restart = repeater.mode == RepeaterMode::Restart;
        let mut page = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                Self::segmented()
                    .child(self.segment(
                        "planned",
                        self.t("timestamp.planned"),
                        !restart,
                        cx,
                        |this, _, cx| this.set_mode(RepeaterMode::CatchUp, cx),
                    ))
                    .child(self.segment(
                        "restart",
                        self.t("timestamp.restart"),
                        restart,
                        cx,
                        |this, _, cx| this.set_mode(RepeaterMode::Restart, cx),
                    )),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(0x737782))
                    .child(if restart {
                        self.t("timestamp.restart_hint")
                    } else {
                        self.t("timestamp.planned_hint")
                    }),
            );
        let mut units = div().flex().gap_1();
        for (id, unit) in [
            ("hours", TimeUnit::Hour),
            ("days", TimeUnit::Day),
            ("weeks", TimeUnit::Week),
            ("months", TimeUnit::Month),
            ("years", TimeUnit::Year),
        ] {
            units = units.child(self.button(
                id,
                unit_text(unit, 1, self.language),
                repeater.unit == unit,
                cx,
                move |this, _, cx| {
                    if let Some(e) = this.endpoint_mut() {
                        e.value
                            .repeater
                            .get_or_insert(Repeater {
                                mode: RepeaterMode::CatchUp,
                                value: 1,
                                unit,
                            })
                            .unit = unit;
                        if unit == TimeUnit::Hour && e.value.start_time.is_none() {
                            e.value.start_time = Time::new(9, 0, 0, 0).ok();
                        }
                    }
                    this.unit_menu = false;
                    this.changed(cx);
                },
            ));
        }
        page = page.child(
            Self::group()
                .child(
                    self.row(self.t("timestamp.repeat_interval"))
                        .child(self.t("timestamp.every"))
                        .child(div().w(px(48.)).child(self.count_input.clone()))
                        .child(self.button(
                            "repeat-unit",
                            format!(
                                "{}  ▾",
                                unit_text(repeater.unit, repeater.value, self.language)
                            ),
                            false,
                            cx,
                            |this, _, cx| {
                                this.unit_menu = !this.unit_menu;
                                cx.notify();
                            },
                        )),
                )
                .when(self.unit_menu, |s| s.child(units))
                .child(
                    self.row(self.t("timestamp.start_date"))
                        .border_t_1()
                        .border_color(rgb(0xe6e8ed))
                        .child(self.button(
                            "repeat-start-date",
                            format!(
                                "{}  ›",
                                super::super::calendar::date_label(value.start_date, self.language)
                            ),
                            self.expanded == Some(ExpandedField::Date),
                            cx,
                            |this, _, cx| {
                                this.expanded = if this.expanded == Some(ExpandedField::Date) {
                                    None
                                } else {
                                    Some(ExpandedField::Date)
                                };
                                if let Some(endpoint) = this.endpoint() {
                                    this.month = endpoint.value.start_date;
                                }
                                cx.notify();
                            },
                        )),
                )
                .when(self.expanded == Some(ExpandedField::Date), |s| {
                    s.child(self.date_editor(cx))
                })
                .child(
                    self.row(self.t("timestamp.weekday"))
                        .text_color(rgb(0x92949d))
                        .child(
                            [
                                self.t("timestamp.monday"),
                                self.t("timestamp.tuesday"),
                                self.t("timestamp.wednesday"),
                                self.t("timestamp.thursday"),
                                self.t("timestamp.friday"),
                                self.t("timestamp.saturday"),
                                self.t("timestamp.sunday"),
                            ]
                                [value.start_date.weekday().to_monday_zero_offset() as usize],
                        ),
                ),
        );
        if !restart {
            let mut choices = Self::group();
            for (id, mode, title, subtitle) in [
                (
                    "cumulative",
                    RepeaterMode::Cumulative,
                    self.t("timestamp.cumulative"),
                    self.t("timestamp.cumulative_hint"),
                ),
                (
                    "catch-up",
                    RepeaterMode::CatchUp,
                    self.t("timestamp.catch_up"),
                    self.t("timestamp.catch_up_hint"),
                ),
            ] {
                choices = choices.child(
                    div()
                        .id(id)
                        .debug_selector(move || id.to_owned())
                        .p_2()
                        .rounded_lg()
                        .cursor_pointer()
                        .when(mode == RepeaterMode::CatchUp, |s| {
                            s.border_t_1().border_color(rgb(0xe6e8ed))
                        })
                        .child(
                            div()
                                .text_color(rgb(if repeater.mode == mode {
                                    0x3f78f2
                                } else {
                                    0x373942
                                }))
                                .child(format!(
                                    "{}  {title}",
                                    if repeater.mode == mode { "●" } else { "○" }
                                )),
                        )
                        .child(
                            div()
                                .pl_4()
                                .mt_1()
                                .text_size(px(11.))
                                .text_color(rgb(0x737782))
                                .child(subtitle),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.set_mode(mode, cx))),
                );
            }
            page = page
                .child(div().text_size(px(12.)).child(self.t("timestamp.missed")))
                .child(choices);
        }
        let example = repeat_example(value.start_date, repeater, self.language);
        page = page.child(
            div()
                .p_3()
                .rounded_lg()
                .bg(rgb(0xeaf2ff))
                .text_color(rgb(0x3f78f2))
                .child(self.t("timestamp.example"))
                .child(
                    div()
                        .mt_2()
                        .text_size(px(12.))
                        .text_color(rgb(0x737782))
                        .children(example.lines().enumerate().map(|(i, line)| {
                            div()
                                .when(i == 2, |s| {
                                    s.mt_1()
                                        .text_size(px(15.))
                                        .text_color(rgb(0x3f78f2))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                })
                                .child(line.to_owned())
                        })),
                ),
        );
        page = page.child(self.button(
            "no-repeat",
            self.t("timestamp.none"),
            false,
            cx,
            |this, _, cx| {
                if let Some(e) = this.endpoint_mut() {
                    e.value.repeater = None;
                }
                this.repeat_page = false;
                this.expanded = None;
                this.count_error = false;
                this.changed(cx);
            },
        ));
        page.into_any_element()
    }
}

impl Render for TimestampPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let max_height = (f32::from(window.viewport_size().height) - 24.).max(100.);
        let valid = self.valid();
        div()
            .id("timestamp-picker")
            .debug_selector(|| "timestamp-picker".to_owned())
            .flex_none()
            .key_context("TimestampPicker")
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|_, _: &CancelTimestamp, _, cx| {
                    cx.emit(TimestampPickerEvent::Cancelled)
                }),
            )
            .occlude()
            .w(px(if self.language == Language::English {
                400.
            } else {
                360.
            }))
            .max_w(px((f32::from(window.viewport_size().width) - 20.).max(200.)))
            .max_h(px(max_height))
            .overflow_y_scroll()
            .p_4()
            .rounded(px(14.))
            .border_1()
            .border_color(rgb(0xdedfe3))
            .bg(rgb(0xffffff))
            .shadow_lg()
            .font_family(".SystemUIFont")
            .font_weight(gpui::FontWeight::NORMAL)
            .text_size(px(13.))
            .line_height(px(20.))
            .text_color(rgb(0x373942))
            .cursor_default()
            .flex()
            .flex_col()
            .gap_3()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus, cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.calendar_drag.set(None);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.calendar_drag.set(None);
                }),
            )
            .on_mouse_move(|_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(|_, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(TimestampPickerEvent::Cancelled);
                    cx.stop_propagation();
                }
            }))
            .child(if self.repeat_page {
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .child(self.button(
                        "back",
                        self.t("timestamp.back"),
                        false,
                        cx,
                        |this, _, cx| {
                            this.repeat_page = false;
                            this.expanded = None;
                            this.sync_inputs(cx);
                            this.changed(cx);
                        },
                    ))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(self.t("timestamp.repeat")),
                    )
                    .child(self.button(
                        "repeat-done",
                        self.t("timestamp.done"),
                        true,
                        cx,
                        |this, _, cx| this.finish_repeat(cx),
                    ))
            } else {
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .child(Self::icon("calendar-blank"))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(self.t("timestamp.title")),
                    )
                    .child(
                        self.button("close", "×", false, cx, |_, _, cx| {
                            cx.emit(TimestampPickerEvent::Cancelled)
                        })
                        .text_color(rgb(0x92949d)),
                    )
            })
            .child(if self.repeat_page {
                self.repeat_page(cx)
            } else {
                self.calendar_page(cx)
            })
            .when(!self.repeat_page, |s| {
                s.child(self.button(
                    "source",
                    if self.source_open {
                        self.t("timestamp.source_open")
                    } else {
                        self.t("timestamp.source_closed")
                    },
                    false,
                    cx,
                    |this, _, cx| {
                        this.source_open = !this.source_open;
                        cx.notify();
                    },
                ))
                .when(self.source_open, |s| {
                    s.child(div().min_h(px(34.)).child(self.source_input.clone()))
                })
            })
            .when(!valid, |s| {
                s.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(0xd54848))
                        .child(self.t("timestamp.invalid")),
                )
            })
            .when(!self.repeat_page, |s| {
                s.child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .border_t_1()
                        .border_color(rgb(0xe9ebf0))
                        .pt_3()
                        .child(self.button(
                            "cancel",
                            self.t("timestamp.cancel"),
                            false,
                            cx,
                            |_, _, cx| cx.emit(TimestampPickerEvent::Cancelled),
                        ))
                        .child(
                            self.button(
                                "apply",
                                self.t("timestamp.apply"),
                                true,
                                cx,
                                |this, _, cx| this.apply(cx),
                            )
                            .min_w(px(64.))
                            .flex()
                            .justify_center()
                            .when(!valid, |s| s.opacity(0.4)),
                        ),
                )
            })
    }
}
