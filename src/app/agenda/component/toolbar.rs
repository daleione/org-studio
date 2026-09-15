use super::super::{
    UiIntent,
    state::{AgendaProjection, AgendaViewState, AgendaWorkspace, CalendarRange},
};
use crate::{
    app::{WorkspaceWindow, component::ModeSwitch},
    i18n::Language,
};
use gpui::{
    Div, Entity, MouseButton, Stateful, StatefulInteractiveElement, div, prelude::*, px, rgb, svg,
};

pub(crate) struct AgendaToolbarProps<'a> {
    pub(crate) workspace: Entity<WorkspaceWindow>,
    pub(crate) state: &'a AgendaViewState,
    pub(crate) language: Language,
    pub(crate) window_width: f32,
    pub(crate) main_content_offset: f32,
    pub(crate) window_title: String,
    pub(crate) search: Option<Entity<crate::app::native_input::NativeInput>>,
    pub(crate) motion_enabled: bool,
    /// Left inset of the titlebar row; shrinks while fullscreen hides the
    /// traffic lights, and grows again when the pointer reveals them.
    pub(crate) titlebar_inset: f32,
}

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

/// Compact edit button beside the traffic lights that returns from the
/// agenda to the document surface, mirroring the `q` key binding.
fn edit_icon_button(workspace: Entity<WorkspaceWindow>) -> Stateful<Div> {
    div()
        .id("agenda-titlebar-edit-toggle")
        .debug_selector(|| "agenda-titlebar-edit-toggle".to_owned())
        .size(px(26.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(rgb(super::super::style::BORDER))
        .bg(rgb(super::super::style::TOOLBAR))
        .cursor_pointer()
        .hover(move |style| style.bg(rgb(0xe9eaed)).border_color(rgb(0xbfc2c8)))
        .active(|style| style.opacity(0.72))
        .child(
            svg()
                .debug_selector(|| "agenda-titlebar-edit-icon".to_owned())
                .data(include_bytes!("../../assets/edit.svg"))
                .size(px(14.))
                .text_color(rgb(super::super::style::INK)),
        )
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            workspace.update(cx, |this, cx| this.return_to_document(cx));
        })
}

pub(crate) fn agenda_toolbar(props: AgendaToolbarProps<'_>) -> Div {
    let AgendaToolbarProps {
        workspace,
        state,
        language,
        window_width,
        main_content_offset,
        window_title,
        search,
        motion_enabled,
        titlebar_inset,
    } = props;
    let narrow = window_width < 900.;
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
        .debug_selector(|| "agenda-toolbar-title-row".to_owned())
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
        );
    let titlebar_actions = div()
        .debug_selector(|| "agenda-toolbar-actions".to_owned())
        .flex_none()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(8.))
        .when(!narrow, |row| {
            row.child(
                super::field(div().children(search.clone()))
                    .w(px(240.))
                    .h(px(30.))
                    .flex_none(),
            )
        })
        .when(narrow, |row| {
            row.child(
                icon(
                    workspace.clone(),
                    "assets/icons/agenda/search.svg",
                    UiIntent::ToggleSearch,
                )
                .size(px(30.)),
            )
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
                    motion_enabled,
                ))
            },
        );
    let mut toolbar = div()
        .flex_none()
        .min_w_0()
        .relative()
        .px(px(24.))
        .pt(px(crate::app::TITLEBAR_HEIGHT + 12.0))
        .pb(px(12.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .bg(rgb(super::super::style::TOOLBAR))
        .border_b_1()
        .border_color(rgb(super::super::style::BORDER))
        .child(
            div()
                .debug_selector(|| "agenda-titlebar-file-row".to_owned())
                .absolute()
                .top_0()
                .left(px(titlebar_inset + 6. - main_content_offset))
                .h(px(crate::app::TITLEBAR_HEIGHT))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(edit_icon_button(workspace.clone()))
                .child(
                    div()
                        .debug_selector(|| "agenda-titlebar-file-name".to_owned())
                        .w(px(240.))
                        .h(px(crate::app::TITLEBAR_HEIGHT))
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(rgb(super::super::style::MUTED))
                        .child(window_title),
                ),
        )
        .child(titlebar_actions.absolute().top(px(4.)).right(px(24.)))
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
                .debug_selector(|| "agenda-toolbar-date-row".to_owned())
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
                .child(calendar_range_switch(
                    workspace.clone(),
                    language,
                    state.calendar_range,
                    motion_enabled,
                )),
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

fn calendar_range_switch(
    workspace: Entity<WorkspaceWindow>,
    language: Language,
    selected_range: CalendarRange,
    motion_enabled: bool,
) -> Div {
    let item_width = match language {
        Language::Chinese => px(44.),
        Language::English => px(56.),
    };
    let selected_index = match selected_range {
        CalendarRange::Day => 0,
        CalendarRange::Week => 1,
        CalendarRange::Month => 2,
    };
    let control = ModeSwitch::new("agenda-calendar-range-slider", selected_index, item_width)
        .motion_enabled(motion_enabled);
    control
        .render(
            [
                (CalendarRange::Day, "agenda.day"),
                (CalendarRange::Week, "agenda.week"),
                (CalendarRange::Month, "agenda.month"),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (range, key))| {
                let label = language.text(key);
                let workspace = workspace.clone();
                control
                    .item(index, format!("agenda-calendar-range-{range:?}"), |_| label)
                    .debug_selector(move || format!("agenda-toolbar-{label}"))
                    .text_size(px(12.))
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        workspace.update(cx, |this, cx| {
                            this.dispatch_agenda_intent(UiIntent::SetCalendarRange(range), cx)
                        });
                    })
            }),
        )
        .debug_selector(|| "agenda-calendar-range-switch".to_owned())
}

fn projection_switch(
    workspace: Entity<WorkspaceWindow>,
    projection: AgendaProjection,
    calendar: bool,
    motion_enabled: bool,
) -> Div {
    let item_width = px(36.);
    let options = [
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
    .collect::<Vec<_>>();
    let selected_index = options
        .iter()
        .position(|(value, _)| *value == projection)
        .unwrap_or_default();
    let control = ModeSwitch::new("agenda-projection-slider", selected_index, item_width)
        .motion_enabled(motion_enabled);
    control.render(
        options
            .into_iter()
            .enumerate()
            .map(|(index, (value, path))| {
                let workspace = workspace.clone();
                control
                    .item(
                        index,
                        format!("agenda-projection-{value:?}"),
                        |foreground| {
                            gpui::svg()
                                .data(super::super::icon::agenda_icon(path))
                                .size(px(16.))
                                .text_color(foreground)
                        },
                    )
                    .debug_selector(move || format!("agenda-projection-{value:?}"))
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        workspace.update(cx, |this, cx| {
                            this.dispatch_agenda_intent(UiIntent::SetProjection(value), cx)
                        });
                    })
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Modifiers, Render, Window};

    #[gpui::test]
    fn narrow_toolbar_preserves_control_geometry(cx: &mut gpui::TestAppContext) {
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        struct Harness(Entity<WorkspaceWindow>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(250.)).child(agenda_toolbar(AgendaToolbarProps {
                    workspace: self.0.clone(),
                    state: &AgendaViewState::default(),
                    language: Language::Chinese,
                    window_width: 250.,
                    main_content_offset: 0.,
                    window_title: "tasks.org".to_owned(),
                    search: None,
                    motion_enabled: true,
                    titlebar_inset: crate::app::TITLEBAR_LEADING_INSET,
                }))
            }
        }
        let (_, cx) = cx.add_window_view(|_, _| Harness(workspace));
        let search = cx
            .debug_bounds("agenda-toolbar-icon-assets/icons/agenda/search.svg")
            .unwrap();
        assert_eq!(search.size.width, px(30.));
        assert_eq!(search.size.height, px(30.));
        assert!(search.right() <= px(250.));
        for selector in [
            "agenda-toolbar-icon-assets/icons/agenda/caret-left.svg",
            "agenda-toolbar-icon-assets/icons/agenda/caret-right.svg",
        ] {
            let bounds = cx.debug_bounds(selector).unwrap();
            assert_eq!(bounds.size.width, px(38.));
            assert_eq!(bounds.size.height, px(38.));
            assert!(bounds.right() <= px(250.));
        }
        let today = cx.debug_bounds("agenda-toolbar-今天").unwrap();
        assert_eq!(today.size.height, px(34.));
        assert!(today.right() <= px(250.));

        let range_switch = cx.debug_bounds("agenda-calendar-range-switch").unwrap();
        assert_eq!(range_switch.size.height, px(30.));
        assert!(range_switch.right() <= px(250.));
        let day = cx.debug_bounds("agenda-toolbar-日").unwrap();
        let week = cx.debug_bounds("agenda-toolbar-周").unwrap();
        let month = cx.debug_bounds("agenda-toolbar-月").unwrap();
        for bounds in [day, week, month] {
            assert_eq!(bounds.size.height, px(26.));
            assert!(bounds.right() <= px(250.));
        }
        assert_eq!(day.right(), week.left());
        assert_eq!(week.right(), month.left());
        let range_indicator = cx
            .debug_bounds("agenda-calendar-range-slider-indicator")
            .unwrap();
        assert_eq!(range_indicator.size, week.size);
        assert_eq!(range_indicator.left(), week.left());
        for selector in [
            "agenda-projection-List",
            "agenda-projection-Calendar",
            "agenda-projection-Source",
        ] {
            let bounds = cx.debug_bounds(selector).unwrap();
            assert_eq!(bounds.size.width, px(36.));
            assert_eq!(bounds.size.height, px(26.));
            assert!(bounds.right() <= px(250.));
        }
        let projection_indicator = cx
            .debug_bounds("agenda-projection-slider-indicator")
            .unwrap();
        let list = cx.debug_bounds("agenda-projection-List").unwrap();
        assert_eq!(projection_indicator.size, list.size);
        assert_eq!(projection_indicator.left(), list.left());
    }

    #[gpui::test]
    fn wide_toolbar_places_search_and_modes_in_the_titlebar(cx: &mut gpui::TestAppContext) {
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        struct Harness(Entity<WorkspaceWindow>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(1000.)).child(agenda_toolbar(AgendaToolbarProps {
                    workspace: self.0.clone(),
                    state: &AgendaViewState::default(),
                    language: Language::Chinese,
                    window_width: 1000.,
                    main_content_offset: 0.,
                    window_title: "tasks.org".to_owned(),
                    search: None,
                    motion_enabled: true,
                    titlebar_inset: crate::app::TITLEBAR_LEADING_INSET,
                }))
            }
        }
        let (_, cx) = cx.add_window_view(|_, _| Harness(workspace));
        let actions = cx.debug_bounds("agenda-toolbar-actions").unwrap();
        let file_row = cx.debug_bounds("agenda-titlebar-file-row").unwrap();
        let edit_button = cx.debug_bounds("agenda-titlebar-edit-toggle").unwrap();
        let filename = cx.debug_bounds("agenda-titlebar-file-name").unwrap();
        let title = cx.debug_bounds("agenda-toolbar-title-row").unwrap();
        assert_eq!(actions.top(), px(4.));
        assert_eq!(actions.bottom(), px(crate::app::TITLEBAR_HEIGHT - 4.));
        assert_eq!(file_row.left(), px(84.));
        assert_eq!(edit_button.left(), px(84.));
        assert!(
            filename.left() >= edit_button.right(),
            "the file name must sit to the right of the edit button"
        );
        assert_eq!(filename.bottom(), px(crate::app::TITLEBAR_HEIGHT));
        assert!(title.top() > actions.bottom());
    }

    #[gpui::test]
    fn toolbar_edit_button_returns_to_the_document(cx: &mut gpui::TestAppContext) {
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        workspace.update(cx, |workspace, cx| workspace.open_agenda(cx));
        struct Harness(Entity<WorkspaceWindow>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(900.)).child(agenda_toolbar(AgendaToolbarProps {
                    workspace: self.0.clone(),
                    state: &AgendaViewState::default(),
                    language: Language::Chinese,
                    window_width: 900.,
                    main_content_offset: 0.,
                    window_title: "tasks.org".to_owned(),
                    search: None,
                    motion_enabled: true,
                    titlebar_inset: crate::app::TITLEBAR_LEADING_INSET,
                }))
            }
        }
        let (_, cx) = cx.add_window_view(|_, _| Harness(workspace.clone()));
        let edit_button = cx
            .debug_bounds("agenda-titlebar-edit-toggle")
            .expect("edit toggle should be rendered in the agenda toolbar");
        let edit_icon = cx
            .debug_bounds("agenda-titlebar-edit-icon")
            .expect("edit icon should be rendered");
        assert_eq!(edit_button.size, gpui::size(px(26.), px(26.)));
        assert_eq!(edit_icon.size, gpui::size(px(14.), px(14.)));
        assert!(
            edit_icon.left() >= edit_button.left() && edit_icon.right() <= edit_button.right(),
            "edit icon must stay inside its button"
        );
        cx.simulate_mouse_move(edit_button.center(), None, Modifiers::default());
        cx.simulate_click(edit_button.center(), Modifiers::default());
        assert_eq!(
            workspace.read_with(cx, |workspace, _| workspace.content_route),
            crate::app::ContentRoute::Document,
            "clicking the agenda toolbar edit button must return to the document route"
        );
    }
}
