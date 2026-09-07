use gpui::{
    Div, Entity, MouseButton, ParentElement, Stateful, StatefulInteractiveElement, Styled, div,
    prelude::*, px, rgb,
};

use super::TaskColumns;
use crate::{
    agenda::{AgendaDateKind, AgendaRow},
    app::WorkspaceWindow,
};

pub(crate) fn task_row(
    language: crate::i18n::Language,
    workspace: Entity<WorkspaceWindow>,
    row: &AgendaRow,
    row_id: usize,
    selected: bool,
    columns: TaskColumns,
) -> Stateful<Div> {
    let source = row
        .source
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("—");
    let time = row
        .time
        .map(|time| time.strftime("%H:%M").to_string())
        .unwrap_or_else(|| "—".into());
    let plan = match row.date_kind {
        Some(AgendaDateKind::Deadline) => language.text("agenda.deadline"),
        Some(AgendaDateKind::Scheduled) => language.text("agenda.scheduled"),
        Some(AgendaDateKind::Plain) => language.text("agenda.time"),
        None => language.text("agenda.unscheduled"),
    };
    let waiting = row.todo.eq_ignore_ascii_case("WAITING") || row.todo.eq_ignore_ascii_case("WAIT");
    let select = |workspace: Entity<WorkspaceWindow>| {
        move |_: &gpui::MouseDownEvent, _: &mut gpui::Window, cx: &mut gpui::App| {
            cx.stop_propagation();
            workspace.update(cx, |this, cx| {
                this.dispatch_agenda_intent(super::super::UiIntent::SelectRow(row_id), cx)
            });
        }
    };
    TaskColumns::row()
        .id(("agenda-task-row", row_id))
        .h(px(super::super::style::TASK_ROW_HEIGHT))
        .border_t_1()
        .border_color(rgb(0xf0f0f2))
        .bg(rgb(if selected {
            super::super::style::BLUE_SELECTION
        } else {
            0xffffff
        }))
        .cursor_pointer()
        .hover(move |style| style.bg(rgb(if selected { 0xddeeff } else { 0xeeeeF1 })))
        .active(|style| style.opacity(0.78))
        .child(
            TaskColumns::cell(TaskColumns::CHECK)
                .id(("agenda-task-check", row_id))
                .h(px(18.))
                .rounded_full()
                .border_1()
                .border_color(rgb(0xc7cad0))
                .hover(|style| style.border_color(rgb(0x1688ff)).bg(rgb(0xe8f2ff)))
                .active(|style| style.opacity(0.7))
                .on_mouse_down(MouseButton::Left, select(workspace.clone())),
        )
        .when(columns.source, |line| {
            let open = workspace.clone();
            let task = row.task;
            line.child(
                TaskColumns::cell(TaskColumns::SOURCE)
                    .id(("agenda-task-source", row_id))
                    .h(px(28.))
                    .px_1()
                    .flex()
                    .items_center()
                    .rounded(px(5.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.))
                    .text_color(rgb(0x656a72))
                    .hover(|style| style.bg(rgb(0xe4effc)).text_color(rgb(0x0876df)))
                    .active(|style| style.opacity(0.72))
                    .child(source.to_owned())
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        open.update(cx, |this, cx| {
                            this.dispatch_agenda_intent(
                                super::super::UiIntent::OpenSource(task),
                                cx,
                            )
                        });
                    }),
            )
        })
        .child(
            TaskColumns::cell(TaskColumns::TIME)
                .id(("agenda-task-time", row_id))
                .h(px(28.))
                .px_1()
                .flex()
                .items_center()
                .rounded(px(5.))
                .font_family("SFMono-Regular")
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_size(px(12.))
                .hover(|style| style.bg(rgb(0xe4effc)).text_color(rgb(0x0876df)))
                .active(|style| style.opacity(0.72))
                .child(time)
                .on_mouse_down(MouseButton::Left, select(workspace.clone())),
        )
        .when(columns.plan, |line| {
            line.child(
                TaskColumns::cell(TaskColumns::plan_width(language))
                    .id(("agenda-task-plan", row_id))
                    .h(px(28.))
                    .px_1()
                    .flex()
                    .items_center()
                    .rounded(px(5.))
                    .text_size(px(11.))
                    .text_color(rgb(
                        if matches!(row.date_kind, Some(AgendaDateKind::Deadline)) {
                            0xa66a00
                        } else {
                            0x73777e
                        },
                    ))
                    .hover(|style| style.bg(rgb(0xf7eedc)))
                    .active(|style| style.opacity(0.72))
                    .child(plan)
                    .on_mouse_down(MouseButton::Left, select(workspace.clone())),
            )
        })
        .child(
            TaskColumns::cell(TaskColumns::STATUS)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id(("agenda-task-status", row_id))
                        .max_w_full()
                        .h(px(22.))
                        .px(px(7.))
                        .flex()
                        .items_center()
                        .rounded(px(5.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .bg(rgb(if waiting { 0xebf3fb } else { 0xe8f4eb }))
                        .text_color(rgb(if waiting { 0x2771b5 } else { 0x358342 }))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_size(px(10.))
                        .hover(|style| {
                            style
                                .bg(rgb(if waiting { 0xd8eafb } else { 0xd8eddc }))
                                .shadow_sm()
                        })
                        .active(|style| style.opacity(0.7))
                        .child(row.todo.to_string())
                        .on_mouse_down(MouseButton::Left, select(workspace.clone())),
                ),
        )
        .when(columns.priority, |line| {
            line.child(row.priority.map_or_else(
                || TaskColumns::cell(TaskColumns::PRIORITY).h(px(24.)),
                |priority| {
                    TaskColumns::cell(TaskColumns::PRIORITY)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .w(px(24.))
                                .h(px(22.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(5.))
                                .border_1()
                                .border_color(rgb(0xf0d59c))
                                .bg(rgb(0xfff8e9))
                                .text_color(rgb(0xa36500))
                                .text_size(px(10.))
                                .child(priority.to_string()),
                        )
                },
            ))
        })
        .child(
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(rgb(0x202329))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_size(px(14.))
                .child(row.title.to_string()),
        )
        .when(columns.tags, |line| {
            line.child(
                TaskColumns::cell(TaskColumns::TAGS)
                    .overflow_hidden()
                    .flex()
                    .gap_1()
                    .children(row.tags.iter().take(2).enumerate().map(|(tag_index, tag)| {
                        let workspace = workspace.clone();
                        let tag_value = tag.clone();
                        div()
                            .id(("agenda-task-tag", row_id * 2 + tag_index))
                            .h(px(24.))
                            .px_3()
                            .flex()
                            .items_center()
                            .rounded_full()
                            .bg(rgb(0xf3edf5))
                            .text_color(rgb(0x754c7d))
                            .text_size(px(11.))
                            .hover(|style| style.bg(rgb(0xe5d8eb)))
                            .active(|style| style.opacity(0.72))
                            .child(tag.to_string())
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                                workspace.update(cx, |this, cx| {
                                    this.dispatch_agenda_intent(
                                        super::super::UiIntent::SetTag(Some(tag_value.clone())),
                                        cx,
                                    )
                                });
                            })
                    })),
            )
        })
}
