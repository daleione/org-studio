use crate::{agenda::AgendaResultSnapshot, app::WorkspaceWindow};
use gpui::{
    AnyElement, Entity, InteractiveElement, ListState, MouseButton, div, list, prelude::*, px, rgb,
    svg,
};
use std::sync::Arc;

pub(crate) fn agenda_list(
    language: crate::i18n::Language,
    workspace: Entity<WorkspaceWindow>,
    result: Arc<AgendaResultSnapshot>,
    state: ListState,
    selected: Option<usize>,
    collapsed_days: std::collections::BTreeSet<Option<jiff::civil::Date>>,
    columns: super::super::component::TaskColumns,
) -> AnyElement {
    list(state, move |group_index, _, _| {
        let group = &result.groups[group_index];
        let collapsed = collapsed_days.contains(&group.date);
        let group_date = group.date;
        let toggle_workspace = workspace.clone();
        let (day, date, marker) = group.date.map_or_else(
            || {
                (
                    language.text("agenda.unscheduled").to_owned(),
                    language.text("agenda.unscheduled").to_owned(),
                    None,
                )
            },
            |date| {
                let is_today = date == jiff::Zoned::now().date();
                (
                    if is_today {
                        language.text("agenda.today").to_owned()
                    } else {
                        super::calendar::weekday(language, date).to_owned()
                    },
                    date.to_string(),
                    is_today.then_some(language.text("agenda.today")),
                )
            },
        );
        let rows = (!collapsed).then(|| {
            group
                .rows
                .clone()
                .fold(div().flex().flex_col(), |container, index| {
                    let workspace = workspace.clone();
                    let row = &result.rows[index];
                    container.child(
                        super::super::component::task_row(
                            language,
                            row,
                            selected == Some(index),
                            columns,
                        )
                        .on_mouse_down(MouseButton::Left, {
                            let workspace = workspace.clone();
                            move |_, _, cx| {
                                workspace.update(cx, |this, cx| {
                                    this.dispatch_agenda_intent(
                                        super::super::UiIntent::SelectRow(index),
                                        cx,
                                    )
                                })
                            }
                        })
                        .on_mouse_down(MouseButton::Right, {
                            let workspace = workspace.clone();
                            let key = row.task;
                            move |_, _, cx| {
                                workspace.update(cx, |this, cx| {
                                    this.dispatch_agenda_intent(
                                        super::super::UiIntent::OpenSource(key),
                                        cx,
                                    )
                                })
                            }
                        }),
                    )
                })
        });
        let card = div()
            .border_1()
            .border_color(gpui::rgb(0xe8e9eb))
            .relative()
            .rounded(gpui::px(7.))
            .overflow_hidden()
            .when(marker.is_some(), |group| {
                group.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(3.))
                        .bg(rgb(0x1688ff)),
                )
            })
            .child(
                div()
                    .id(("agenda-day-header", group_index))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        toggle_workspace.update(cx, |this, cx| {
                            this.dispatch_agenda_intent(
                                super::super::UiIntent::ToggleDayGroup(group_date),
                                cx,
                            );
                        });
                    })
                    .h(gpui::px(super::super::style::DAY_HEADER_HEIGHT))
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(!collapsed, |header| header.border_b_1())
                    .border_color(gpui::rgb(0xececee))
                    .text_color(rgb(super::super::style::PURPLE))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(
                        svg()
                            .data(super::super::icon::agenda_icon(
                                "assets/icons/agenda/caret-right.svg",
                            ))
                            .size(px(13.))
                            .with_transformation(gpui::Transformation::rotate(gpui::radians(
                                if collapsed {
                                    0.
                                } else {
                                    std::f32::consts::FRAC_PI_2
                                },
                            )))
                            .text_color(rgb(0x51565e)),
                    )
                    .child(div().text_size(px(13.)).child(day))
                    .child(div().size(px(3.)).rounded_full().bg(rgb(0xa2a5aa)))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::NORMAL)
                            .text_color(rgb(0x666b72))
                            .child(date),
                    )
                    .when_some(marker, |header, marker| {
                        header.child(
                            div()
                                .ml_1()
                                .h(px(22.))
                                .px_2()
                                .flex()
                                .items_center()
                                .rounded_full()
                                .bg(rgb(0x68adf5))
                                .text_color(rgb(0xffffff))
                                .text_size(px(10.))
                                .child(marker),
                        )
                    })
                    .when(marker.is_some(), |header| {
                        header.child(
                            div()
                                .h(px(22.))
                                .px_2()
                                .flex()
                                .items_center()
                                .rounded_full()
                                .bg(rgb(0xf1e8f8))
                                .text_color(rgb(0x7a4691))
                                .text_size(px(10.))
                                .child("W36"),
                        )
                    }),
            )
            .when_some(rows, |card, rows| card.child(rows));
        div().pb_2().child(card).into_any_element()
    })
    .size_full()
    .into_any_element()
}
