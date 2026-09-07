use gpui::{Div, ParentElement, Styled, div, prelude::*, px, rgb};

use super::TaskColumns;
use crate::agenda::{AgendaDateKind, AgendaRow};

pub(crate) fn task_row(
    language: crate::i18n::Language,
    row: &AgendaRow,
    selected: bool,
    columns: TaskColumns,
) -> Div {
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
    TaskColumns::row()
        .h(px(super::super::style::TASK_ROW_HEIGHT))
        .border_t_1()
        .border_color(rgb(0xf0f0f2))
        .bg(rgb(if selected {
            super::super::style::BLUE_SELECTION
        } else {
            0xffffff
        }))
        .child(
            TaskColumns::cell(TaskColumns::CHECK)
                .h(px(18.))
                .rounded_full()
                .border_1()
                .border_color(rgb(0xc7cad0)),
        )
        .when(columns.source, |line| {
            line.child(
                TaskColumns::cell(TaskColumns::SOURCE)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.))
                    .text_color(rgb(0x656a72))
                    .child(source.to_owned()),
            )
        })
        .child(
            TaskColumns::cell(TaskColumns::TIME)
                .font_family("SFMono-Regular")
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_size(px(12.))
                .child(time),
        )
        .when(columns.plan, |line| {
            line.child(
                TaskColumns::cell(TaskColumns::plan_width(language))
                    .text_size(px(11.))
                    .text_color(rgb(
                        if matches!(row.date_kind, Some(AgendaDateKind::Deadline)) {
                            0xa66a00
                        } else {
                            0x73777e
                        },
                    ))
                    .child(plan),
            )
        })
        .child(
            TaskColumns::cell(TaskColumns::STATUS)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
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
                        .child(row.todo.to_string()),
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
                    .children(
                        row.tags
                            .iter()
                            .take(2)
                            .map(|tag| super::pill(tag.to_string())),
                    ),
            )
        })
}
