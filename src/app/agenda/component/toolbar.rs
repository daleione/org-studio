use super::super::{
    UiIntent,
    state::{AgendaProjection, AgendaViewState, AgendaWorkspace, CalendarRange},
};
use crate::{app::WorkspaceWindow, i18n::Language};
use gpui::{
    Div, Entity, MouseButton, Stateful, StatefulInteractiveElement, div, prelude::*, px, rgb,
};

fn button(
    workspace: Entity<WorkspaceWindow>,
    label: impl Into<String>,
    intent: UiIntent,
    selected: bool,
    enabled: bool,
) -> Stateful<Div> {
    let label = label.into();
    div()
        .id(format!("agenda-toolbar-button-{label}"))
        .debug_selector({
            let label = label.clone();
            move || format!("agenda-toolbar-{label}")
        })
        .flex_none()
        .h(px(34.))
        .px(px(9.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .border_1()
        .border_color(rgb(0xdfe0e3))
        .bg(rgb(if selected { 0xe8f2ff } else { 0xffffff }))
        .text_color(rgb(if !enabled {
            0xa2a5aa
        } else if selected {
            0x1688ff
        } else {
            0x454950
        }))
        .text_size(px(12.))
        .child(label)
        .hover(move |style| {
            if selected {
                style.bg(rgb(0xd8eaff)).border_color(rgb(0x76b6f2))
            } else if enabled {
                style.bg(rgb(0xe9eaed)).border_color(rgb(0xbfc2c8))
            } else {
                style
                    .bg(rgb(0xf1f2f4))
                    .border_color(rgb(0xd1d3d7))
                    .text_color(rgb(0x858990))
            }
        })
        .active(|style| style.opacity(0.72))
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    workspace.update(cx, |this, cx| {
                        this.dispatch_agenda_intent(intent.clone(), cx)
                    });
                })
        })
}

fn icon(workspace: Entity<WorkspaceWindow>, path: &'static str, intent: UiIntent) -> Stateful<Div> {
    super::action_icon_button(workspace, path, intent, false)
        .debug_selector(move || format!("agenda-toolbar-icon-{path}"))
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render, Window};

    #[gpui::test]
    fn narrow_toolbar_preserves_control_geometry(cx: &mut gpui::TestAppContext) {
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        struct Harness(Entity<WorkspaceWindow>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(250.)).child(agenda_toolbar(
                    self.0.clone(),
                    &AgendaViewState::default(),
                    Language::Chinese,
                    250.,
                    None,
                ))
            }
        }
        let (_, cx) = cx.add_window_view(|_, _| Harness(workspace));
        for selector in [
            "agenda-toolbar-icon-assets/icons/agenda/search.svg",
            "agenda-toolbar-icon-assets/icons/agenda/caret-left.svg",
            "agenda-toolbar-icon-assets/icons/agenda/caret-right.svg",
        ] {
            let bounds = cx.debug_bounds(selector).unwrap();
            assert_eq!(bounds.size.width, px(38.));
            assert_eq!(bounds.size.height, px(38.));
            assert!(bounds.right() <= px(250.));
        }
        for name in [
            "agenda-toolbar-今天",
            "agenda-toolbar-日",
            "agenda-toolbar-周",
            "agenda-toolbar-月",
        ] {
            let bounds = cx.debug_bounds(name).unwrap();
            assert_eq!(bounds.size.height, px(34.));
            assert!(bounds.right() <= px(250.), "{name}");
        }
        for selector in [
            "agenda-projection-List",
            "agenda-projection-Calendar",
            "agenda-projection-Source",
        ] {
            let bounds = cx.debug_bounds(selector).unwrap();
            assert_eq!(bounds.size.width, px(40.));
            assert_eq!(bounds.size.height, px(32.));
            assert!(bounds.right() <= px(250.));
        }
    }
}

pub(crate) fn agenda_toolbar(
    workspace: Entity<WorkspaceWindow>,
    state: &AgendaViewState,
    language: Language,
    width: f32,
    search: Option<Entity<super::super::search::AgendaSearch>>,
) -> Div {
    let narrow = width < 780.;
    let today = jiff::Zoned::now().date();
    let date_browsing = state.browses_dates();
    let title = match state.workspace {
        AgendaWorkspace::Inbox => language.text("agenda.inbox"),
        AgendaWorkspace::Projects => language.text("agenda.projects"),
        AgendaWorkspace::Tasks => language.text("agenda.tasks"),
        AgendaWorkspace::Agenda if date_browsing => language.text("agenda.agenda"),
        AgendaWorkspace::Agenda => match state.builtin {
            crate::agenda::BuiltinQuery::Today => language.text("agenda.today"),
            crate::agenda::BuiltinQuery::NextSevenDays => language.text("agenda.next_seven"),
            crate::agenda::BuiltinQuery::Overdue => language.text("agenda.overdue"),
            crate::agenda::BuiltinQuery::Next => language.text("agenda.next"),
            crate::agenda::BuiltinQuery::Waiting => language.text("agenda.waiting"),
            crate::agenda::BuiltinQuery::Unscheduled => language.text("agenda.unscheduled"),
        },
    };
    let first = div()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(22.))
                .font_weight(gpui::FontWeight::BOLD)
                .child(title),
        )
        .when(!narrow, |row| {
            row.child(
                super::field(div().children(search.clone()))
                    .w(px(240.))
                    .flex_none(),
            )
        })
        .when(narrow, |row| {
            row.child(icon(
                workspace.clone(),
                "assets/icons/agenda/search.svg",
                UiIntent::ToggleSearch,
            ))
        })
        .when(
            matches!(
                state.workspace,
                AgendaWorkspace::Agenda | AgendaWorkspace::Tasks
            ),
            |row| {
                row.child(projection_switch(
                    workspace.clone(),
                    state.projection,
                    date_browsing,
                ))
            },
        );
    let mut toolbar = div()
        .flex_none()
        .min_w_0()
        .px(px(12.))
        .py(px(12.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .bg(rgb(super::super::style::TOOLBAR))
        .border_b_1()
        .border_color(rgb(super::super::style::BORDER))
        .child(first)
        .when(
            narrow && (state.search_expanded || !state.search.is_empty()),
            |bar| bar.child(super::field(div().children(search.clone())).w_full()),
        );
    if date_browsing {
        let (start, end) = state.calendar_window(today);
        let anchor = state.period_anchor(today);
        let label = if state.calendar_range == CalendarRange::Month {
            format!("{}-{:02}", anchor.year(), anchor.month())
        } else if start == end {
            start.to_string()
        } else {
            format!(
                "{:02}/{:02} – {:02}/{:02}",
                start.month(),
                start.day(),
                end.month(),
                end.day()
            )
        };
        let contains_today = if state.calendar_range == CalendarRange::Month {
            anchor.year() == today.year() && anchor.month() == today.month()
        } else {
            start <= today && today <= end
        };
        toolbar = toolbar.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(icon(
                            workspace.clone(),
                            "assets/icons/agenda/caret-left.svg",
                            UiIntent::ShiftCalendar(-1),
                        ))
                        .child(button(
                            workspace.clone(),
                            label,
                            UiIntent::ToggleDatePicker,
                            state.date_picker,
                            true,
                        ))
                        .child(icon(
                            workspace.clone(),
                            "assets/icons/agenda/caret-right.svg",
                            UiIntent::ShiftCalendar(1),
                        )),
                )
                .child(button(
                    workspace.clone(),
                    language.text("agenda.today"),
                    UiIntent::CalendarToday,
                    false,
                    !contains_today,
                ))
                .child(
                    div().flex_none().flex().gap(px(4.)).children(
                        [
                            (CalendarRange::Day, "agenda.day"),
                            (CalendarRange::Week, "agenda.week"),
                            (CalendarRange::Month, "agenda.month"),
                        ]
                        .into_iter()
                        .map(|(range, key)| {
                            button(
                                workspace.clone(),
                                language.text(key),
                                UiIntent::SetCalendarRange(range),
                                state.calendar_range == range,
                                true,
                            )
                        }),
                    ),
                ),
        );
    }
    if state.date_picker && date_browsing {
        let month = state
            .period_anchor(today)
            .first_of_month()
            .checked_add(jiff::Span::new().months(i64::from(state.picker_offset)))
            .unwrap_or(today.first_of_month());
        let offset = i64::from(month.weekday().to_monday_zero_offset());
        let start = month
            .checked_sub(jiff::Span::new().days(offset))
            .unwrap_or(month);
        let mut picker = div()
            .flex_none()
            .w(px(224.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(icon(
                        workspace.clone(),
                        "assets/icons/agenda/caret-left.svg",
                        UiIntent::ShiftDatePicker(-1),
                    ))
                    .child(format!("{}-{:02}", month.year(), month.month()))
                    .child(icon(
                        workspace.clone(),
                        "assets/icons/agenda/caret-right.svg",
                        UiIntent::ShiftDatePicker(1),
                    )),
            )
            .child(
                div().flex().children(
                    [
                        "agenda.mon",
                        "agenda.tue",
                        "agenda.wed",
                        "agenda.thu",
                        "agenda.fri",
                        "agenda.sat",
                        "agenda.sun",
                    ]
                    .into_iter()
                    .map(|key| {
                        div()
                            .w(px(32.))
                            .text_center()
                            .text_size(px(10.))
                            .child(language.text(key))
                    }),
                ),
            );
        for week in 0..6 {
            picker = picker.child(
                div().flex().children(
                    (0..7)
                        .filter_map(|day| {
                            start
                                .checked_add(jiff::Span::new().days(week * 7 + day))
                                .ok()
                        })
                        .map(|date| {
                            button(
                                workspace.clone(),
                                date.day().to_string(),
                                UiIntent::JumpToDate(date),
                                date == state.calendar_anchor.unwrap_or(today),
                                true,
                            )
                            .w(px(32.))
                            .px_0()
                            .when(date.month() != month.month(), |cell| {
                                cell.text_color(rgb(0xa2a5aa))
                            })
                        }),
                ),
            );
        }
        toolbar = toolbar.child(picker);
    }
    toolbar
}

fn projection_switch(
    workspace: Entity<WorkspaceWindow>,
    projection: AgendaProjection,
    calendar: bool,
) -> Div {
    [
        (
            AgendaProjection::List,
            "assets/icons/agenda/list-bullets.svg",
        ),
        (
            AgendaProjection::Calendar,
            "assets/icons/agenda/calendar-dots.svg",
        ),
        (
            AgendaProjection::Source,
            "assets/icons/agenda/file-code.svg",
        ),
    ]
    .into_iter()
    .filter(|(value, _)| calendar || *value != AgendaProjection::Calendar)
    .fold(
        div()
            .flex_none()
            .h(px(38.))
            .p(px(2.))
            .flex()
            .items_center()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(0xdfe0e3))
            .bg(rgb(0xf4f4f5)),
        |control, (value, path)| {
            let selected = value == projection;
            let workspace = workspace.clone();
            control.child(
                div()
                    .id(format!("agenda-projection-{value:?}"))
                    .debug_selector(move || format!("agenda-projection-{value:?}"))
                    .flex_none()
                    .w(px(40.))
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |button| button.bg(rgb(0xffffff)).shadow_sm())
                    .when(!selected, |button| {
                        button.hover(|style| style.bg(rgb(0xeaeaec)))
                    })
                    .when(selected, |button| {
                        button.hover(|style| style.bg(rgb(0xd8eaff)))
                    })
                    .active(|style| style.opacity(0.72))
                    .child(
                        gpui::svg()
                            .data(super::super::icon::agenda_icon(path))
                            .size(px(16.))
                            .text_color(rgb(if selected { 0x1688ff } else { 0x34373d })),
                    )
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        workspace.update(cx, |this, cx| {
                            this.dispatch_agenda_intent(UiIntent::SetProjection(value), cx)
                        });
                    }),
            )
        },
    )
}
