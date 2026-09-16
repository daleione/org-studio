use std::sync::Arc;

use crate::{
    agenda::{AgendaCommand, TaskKey},
    app::WorkspaceWindow,
};
use gpui::{
    Div, Entity, InteractiveElement, MouseButton, ParentElement, ScrollHandle,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px, rgb, svg,
};

fn section_title(title: &'static str) -> Div {
    let theme = crate::theme::current_theme();
    div()
        .flex_none()
        .pt_4()
        .border_t_1()
        .border_color(rgb(theme.divider))
        .text_size(px(10.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(theme.foreground_dim))
        .child(title)
}

fn inspector_field(label: &'static str, value: &'static str) -> Div {
    let theme = crate::theme::current_theme();
    div()
        .flex_none()
        .h(px(38.))
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .w(px(82.))
                .text_size(px(11.))
                .text_color(rgb(theme.foreground_dim))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .h(px(30.))
                .px_2()
                .flex()
                .items_center()
                .rounded(px(6.))
                .bg(rgb(theme.elevated))
                .text_size(px(11.))
                .text_color(rgb(theme.foreground))
                .child(value),
        )
}

#[derive(Clone)]
pub(crate) struct InspectorProps {
    pub(crate) language: crate::i18n::Language,
    pub(crate) workspace: Entity<WorkspaceWindow>,
    pub(crate) task: TaskKey,
    pub(crate) title: Arc<str>,
    pub(crate) todo: Arc<str>,
    pub(crate) priority: Option<char>,
    pub(crate) tags: Arc<[Arc<str>]>,
    pub(crate) allowed_todo_states: Arc<[Arc<str>]>,
    pub(crate) pending: bool,
    pub(crate) error: Option<Arc<str>>,
    pub(crate) confirm_delete: bool,
    pub(crate) scroll_handle: ScrollHandle,
    pub(crate) clock_active: bool,
    pub(crate) habit: Option<crate::agenda::HabitStats>,
}

pub(crate) fn agenda_inspector(props: InspectorProps) -> Div {
    let language = props.language;
    let theme = crate::theme::current_theme();
    let state_buttons = props.allowed_todo_states.iter().fold(
        div().flex_none().flex().flex_wrap().gap_1(),
        |row, state| {
            let workspace = props.workspace.clone();
            let task = props.task;
            let value = state.clone();
            row.child(
                div()
                    .h(px(25.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .bg(rgb(if state.as_ref() == props.todo.as_ref() {
                        theme.hover
                    } else {
                        theme.elevated
                    }))
                    .text_color(rgb(if state.as_ref() == props.todo.as_ref() {
                        theme.success
                    } else {
                        theme.foreground_muted
                    }))
                    .text_size(px(10.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(theme.hover)))
                    .child(state.to_string())
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        workspace.update(cx, |this, cx| {
                            this.dispatch_agenda_intent(
                                super::super::UiIntent::RequestTodoTransition(task, value.clone()),
                                cx,
                            )
                        })
                    }),
            )
        },
    );
    let priorities = [Some('A'), Some('B'), Some('C'), None].into_iter().fold(
        div().flex_none().flex().gap_1(),
        |row, priority| {
            let workspace = props.workspace.clone();
            let task = props.task;
            row.child(
                div()
                    .min_w(px(27.))
                    .h(px(25.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .bg(rgb(if priority == props.priority {
                        theme.background
                    } else {
                        theme.hover
                    }))
                    .text_color(rgb(if priority == props.priority {
                        theme.warning
                    } else {
                        theme.foreground_muted
                    }))
                    .text_size(px(10.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(theme.background)).shadow_sm())
                    .child(
                        priority.map_or(language.text("agenda.clear").into(), |value| {
                            value.to_string()
                        }),
                    )
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        workspace.update(cx, |this, cx| {
                            this.dispatch_agenda_intent(
                                super::super::UiIntent::Apply(
                                    task,
                                    AgendaCommand::SetPriority(priority),
                                ),
                                cx,
                            )
                        })
                    }),
            )
        },
    );
    let delete = if props.confirm_delete {
        let cancel = props.workspace.clone();
        let confirm = props.workspace.clone();
        let task = props.task;
        div()
            .flex()
            .gap_2()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .bg(rgb(theme.error))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(theme.error)))
                    .child(language.text("agenda.confirm_delete"))
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        confirm.update(cx, |this, cx| {
                            this.dispatch_agenda_intent(
                                super::super::UiIntent::ConfirmDelete(task),
                                cx,
                            )
                        })
                    }),
            )
            .child(
                div()
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(theme.hover)))
                    .child(language.text("agenda.cancel"))
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cancel.update(cx, |this, cx| {
                            this.dispatch_agenda_intent(super::super::UiIntent::CancelDelete, cx)
                        })
                    }),
            )
    } else {
        let workspace = props.workspace.clone();
        div().child(
            div()
                .px_2()
                .py_1()
                .text_color(rgb(theme.error))
                .cursor_pointer()
                .rounded(px(5.))
                .hover(|style| style.bg(rgb(theme.hover)))
                .child(language.text("agenda.delete_subtree"))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    workspace.update(cx, |this, cx| {
                        this.dispatch_agenda_intent(super::super::UiIntent::RequestDelete, cx)
                    })
                }),
        )
    };
    div()
        .w(px(360.0))
        .h_full()
        .flex()
        .flex_col()
        .border_l_1()
        .border_color(rgb(theme.border))
        .bg(rgb(theme.surface))
        .child(
            div()
                .flex_none()
                .h(px(48.))
                .px_5()
                .flex()
                .items_center()
                .border_b_1()
                .border_color(rgb(theme.border))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(language.text("agenda.details")),
                )
                .child(div().flex_1())
                .child({
                    let workspace = props.workspace.clone();
                    div()
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(theme.hover)))
                        .child(
                            svg()
                                .data(super::super::icon::agenda_icon("assets/icons/agenda/x.svg"))
                                .size(px(16.))
                                .text_color(rgb(theme.foreground_dim)),
                        )
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            workspace.update(cx, |this, cx| {
                                this.dispatch_agenda_intent(
                                    super::super::UiIntent::CloseInspector,
                                    cx,
                                )
                            })
                        })
                }),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .id("agenda-inspector-scroll")
                .overflow_y_scroll()
                .restrict_scroll_to_axis()
                .track_scroll(&props.scroll_handle)
                .on_scroll_wheel(|event, _, cx| {
                    let delta = event.delta.pixel_delta(px(16.0));
                    if !event.delta.precise() || delta.y.abs() >= delta.x.abs() {
                        cx.stop_propagation();
                    }
                })
                .p_5()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .flex_none()
                        .text_size(px(20.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(theme.foreground))
                        .child(props.title.to_string()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .text_color(rgb(theme.foreground_dim))
                        .child(language.text("agenda.status")),
                )
                .child(state_buttons)
                .child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .text_color(rgb(theme.foreground_dim))
                        .child(language.text("agenda.priority")),
                )
                .child(priorities)
                .child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .text_color(rgb(theme.foreground_dim))
                        .child(language.text("agenda.tags")),
                )
                .child(
                    div().flex_none().flex().flex_wrap().gap_2().children(
                        props
                            .tags
                            .iter()
                            .map(|tag| super::super::component::pill(tag.to_string())),
                    ),
                )
                .child(section_title(language.text("agenda.dates")))
                .child(inspector_field(language.text("agenda.date"), "—"))
                .child(inspector_field(
                    language.text("agenda.scheduled"),
                    "2026-09-05",
                ))
                .child(inspector_field(language.text("agenda.deadline"), "—"))
                .child(section_title(language.text("agenda.repeat_reminders")))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            inspector_field(
                                language.text("agenda.repeat"),
                                language.text("agenda.no_repeat"),
                            )
                            .flex_1(),
                        )
                        .child(
                            inspector_field(
                                language.text("agenda.reminder"),
                                language.text("agenda.default"),
                            )
                            .flex_1(),
                        ),
                )
                .when_some(props.habit, |panel, stats| {
                    panel
                        .child(section_title(language.text("agenda.habits")))
                        .child(
                            div().flex().justify_between().children(
                                [
                                    (
                                        stats.current_streak.to_string(),
                                        language.text("agenda.streak"),
                                    ),
                                    (
                                        stats.best_streak.to_string(),
                                        language.text("agenda.best_streak"),
                                    ),
                                    (
                                        format!("{}%", stats.completion_percent),
                                        language.text("agenda.completion"),
                                    ),
                                ]
                                .map(|(value, label)| {
                                    div()
                                        .text_center()
                                        .child(
                                            div()
                                                .text_size(px(17.))
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .child(value),
                                        )
                                        .child(
                                            div()
                                                .mt_1()
                                                .text_size(px(9.))
                                                .text_color(rgb(theme.foreground_dim))
                                                .child(label),
                                        )
                                }),
                            ),
                        )
                        .child(
                            div()
                                .mt_3()
                                .flex()
                                .flex_wrap()
                                .gap_1()
                                .children((0..28).map(|index| {
                                    div().size(px(13.)).rounded(px(3.)).bg(rgb(
                                        if index < usize::from(stats.completed) {
                                            theme.success
                                        } else {
                                            theme.border
                                        },
                                    ))
                                })),
                        )
                })
                .child(section_title(language.text("agenda.logbook")))
                .child({
                    let workspace = props.workspace.clone();
                    let task = props.task;
                    div()
                        .h(px(52.))
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_3()
                        .rounded(px(8.))
                        .bg(rgb(if props.clock_active {
                            theme.hover
                        } else {
                            theme.elevated
                        }))
                        .cursor_pointer()
                        .hover(|style| {
                            style.bg(rgb(if props.clock_active {
                                theme.selected
                            } else {
                                theme.hover
                            }))
                        })
                        .child(
                            div()
                                .size(px(30.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(rgb(if props.clock_active {
                                    theme.success
                                } else {
                                    theme.todo_active
                                }))
                                .child(
                                    svg()
                                        .data(super::super::icon::agenda_icon(
                                            if props.clock_active {
                                                "assets/icons/agenda/check-circle.svg"
                                            } else {
                                                "assets/icons/agenda/caret-right.svg"
                                            },
                                        ))
                                        .size(px(16.))
                                        .text_color(rgb(0xffffff)),
                                ),
                        )
                        .child(
                            div()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(if props.clock_active {
                                            language.text("agenda.clock_running")
                                        } else {
                                            "00:00:00"
                                        }),
                                )
                                .child(
                                    div()
                                        .mt_1()
                                        .text_size(px(9.))
                                        .text_color(rgb(theme.foreground_muted))
                                        .child(if props.clock_active {
                                            language.text("agenda.clock_stop")
                                        } else {
                                            language.text("agenda.clock_start")
                                        }),
                                ),
                        )
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            workspace.update(cx, |this, cx| {
                                this.dispatch_agenda_intent(
                                    super::super::UiIntent::ToggleClock(task),
                                    cx,
                                )
                            });
                        })
                })
                .child(section_title(language.text("agenda.properties")))
                .child(inspector_field(language.text("agenda.effort"), "0:30"))
                .child(inspector_field("CATEGORY", "Demo"))
                .child(section_title(language.text("agenda.notes")))
                .child(
                    div()
                        .flex_none()
                        .min_h(px(78.))
                        .p_3()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(rgb(theme.border))
                        .bg(rgb(theme.background))
                        .text_size(px(11.))
                        .text_color(rgb(theme.foreground_muted))
                        .child(language.text("agenda.notes_hint")),
                )
                .child(section_title(language.text("agenda.source")))
                .child(
                    div()
                        .flex_none()
                        .mt_2()
                        .p_3()
                        .rounded(px(8.))
                        .bg(rgb(theme.accent_bg))
                        .text_color(rgb(theme.accent))
                        .text_size(px(11.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            svg()
                                .data(super::super::icon::agenda_icon(
                                    "assets/icons/agenda/file-text.svg",
                                ))
                                .size(px(15.))
                                .text_color(rgb(theme.accent)),
                        )
                        .child(language.text("agenda.open_original")),
                )
                .child(div().flex_1())
                .child(delete),
        )
        .when(props.pending, |panel| {
            panel.child(
                div()
                    .text_color(rgb(theme.warning))
                    .child(language.text("agenda.saving")),
            )
        })
        .when_some(props.error, |panel, error| {
            panel.child(div().text_color(rgb(theme.error)).child(error.to_string()))
        })
}
