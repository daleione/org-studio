use std::{collections::HashMap, sync::Arc};

use gpui::{
    Context, Div, Entity, InteractiveElement, MouseButton, ParentElement, Pixels, Point, Render,
    Stateful, StatefulInteractiveElement, Styled, Window, div, prelude::*, px, rgb,
};
#[cfg(test)]
use jiff::Span;
use jiff::civil::Date;

use crate::{
    agenda::{AgendaResultSnapshot, AgendaRow},
    app::WorkspaceWindow,
};

use super::super::{UiIntent, state::CalendarRange};

#[derive(Clone)]
struct CalendarDrag {
    index: usize,
    title: Arc<str>,
    position: Point<Pixels>,
    resize: bool,
}

impl Render for CalendarDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div().pl(self.position.x).pt(self.position.y).child(
            div()
                .w(px(170.))
                .p_2()
                .rounded(px(6.))
                .bg(rgb(0xeaf5ec))
                .text_color(rgb(0x286f35))
                .text_size(px(10.))
                .shadow_md()
                .child(self.title.to_string()),
        )
    }
}

fn draggable_task(child: impl gpui::IntoElement, row: &AgendaRow, index: usize) -> Stateful<Div> {
    let drag = CalendarDrag {
        index,
        title: row.title.clone(),
        position: Point::default(),
        resize: false,
    };
    div()
        .id(("calendar-drag", index))
        .size_full()
        .cursor_move()
        .child(child)
        .on_drag(drag, |info: &CalendarDrag, position, _, cx| {
            cx.new(|_| CalendarDrag {
                position,
                ..info.clone()
            })
        })
}

fn resize_handle(row: &AgendaRow, index: usize) -> Option<Stateful<Div>> {
    row.end_time?;
    let drag = CalendarDrag {
        index,
        title: row.title.clone(),
        position: Point::default(),
        resize: true,
    };
    Some(
        div()
            .id(("calendar-resize", index))
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .h(px(12.))
            .cursor_ns_resize()
            .on_drag(drag, |info: &CalendarDrag, position, _, cx| {
                cx.new(|_| CalendarDrag {
                    position,
                    ..info.clone()
                })
            }),
    )
}

pub(super) fn weekday(language: crate::i18n::Language, date: Date) -> &'static str {
    match format!("{:?}", date.weekday()).as_str() {
        "Monday" => language.text("agenda.mon"),
        "Tuesday" => language.text("agenda.tue"),
        "Wednesday" => language.text("agenda.wed"),
        "Thursday" => language.text("agenda.thu"),
        "Friday" => language.text("agenda.fri"),
        "Saturday" => language.text("agenda.sat"),
        _ => language.text("agenda.sun"),
    }
}

fn tone(row: &AgendaRow) -> (u32, u32) {
    if row.todo.eq_ignore_ascii_case("WAITING") || row.todo.eq_ignore_ascii_case("WAIT") {
        (0xeaf2fb, 0x27699f)
    } else if row.todo.eq_ignore_ascii_case("NEXT") {
        (0xe5f3e9, 0x246632)
    } else if matches!(row.date_kind, Some(crate::agenda::AgendaDateKind::Deadline)) {
        (0xfbf4e8, 0x9a6815)
    } else {
        (0xeaf5ec, 0x286f35)
    }
}

fn task_chip(workspace: Entity<WorkspaceWindow>, row: &AgendaRow, index: usize) -> Stateful<Div> {
    let (background, foreground) = tone(row);
    div()
        .id(("calendar-task", index))
        .min_h(px(36.))
        .p_2()
        .rounded(px(5.))
        .bg(rgb(background))
        .text_color(rgb(foreground))
        .text_size(px(10.))
        .cursor_pointer()
        .hover(|style| style.opacity(0.84).shadow_sm())
        .active(|style| style.opacity(0.68))
        .overflow_hidden()
        .child(
            div()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(row.title.to_string()),
        )
        .child(div().mt_1().text_size(px(9.)).child(row.todo.to_string()))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            workspace.update(cx, |this, cx| {
                this.dispatch_agenda_intent(UiIntent::SelectRow(index), cx)
            });
        })
}

#[cfg(test)]
fn dates(anchor: Date, range: CalendarRange) -> Vec<Date> {
    let count = match range {
        CalendarRange::Day => 1,
        CalendarRange::Week => 7,
        CalendarRange::Month => 35,
    };
    (0..count)
        .filter_map(|day| anchor.checked_add(Span::new().days(day)).ok())
        .collect()
}

fn month_view(
    language: crate::i18n::Language,
    workspace: Entity<WorkspaceWindow>,
    result: &Arc<AgendaResultSnapshot>,
    days: &[Date],
) -> gpui::AnyElement {
    let week_count = days.len().div_ceil(7).max(1) as f32;
    let mut grid = div()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .flex()
        .flex_wrap()
        .content_start()
        .id("calendar-month-scroll")
        .overflow_y_scroll();
    for date in days {
        let rows: Vec<_> = result
            .placements
            .iter()
            .enumerate()
            .filter(|(_, placement)| placement.date == Some(*date))
            .filter_map(|(index, placement)| {
                result
                    .entries
                    .get(placement.entry.0 as usize)
                    .map(|entry| (index, &entry.row))
            })
            .collect();
        let mut cell = div()
            .w(gpui::relative(1. / 7.))
            .h(gpui::relative(1. / week_count))
            .flex_none()
            .min_w_0()
            .min_h(px(90.))
            .p_2()
            .border_r_1()
            .border_b_1()
            .border_color(rgb(0xe8e9eb))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(0x777b82))
                    .child(format!("{} · {}", weekday(language, *date), date.day())),
            );
        for (index, row) in rows.into_iter().take(3) {
            cell = cell.child(draggable_task(
                task_chip(workspace.clone(), row, index),
                row,
                index,
            ));
        }
        grid = grid.child(cell);
    }
    grid.into_any_element()
}

fn time_view(
    language: crate::i18n::Language,
    workspace: Entity<WorkspaceWindow>,
    result: &Arc<AgendaResultSnapshot>,
    days: &[Date],
    all_day: bool,
) -> gpui::AnyElement {
    let mut header = div()
        .flex_none()
        .min_w_0()
        .h(px(72.))
        .flex()
        .border_b_1()
        .border_color(rgb(0xdedfe2))
        .child(div().w(px(58.)).border_r_1().border_color(rgb(0xe5e6e8)));
    for date in days {
        header = header.child(
            div()
                .flex_1()
                .min_w_0()
                .border_r_1()
                .border_color(rgb(0xececef))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(0x70747b))
                        .child(weekday(language, *date)),
                )
                .child(
                    div()
                        .size(px(32.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(date.day().to_string()),
                ),
        );
    }

    let all_day_row = all_day.then(|| {
        let mut row = div()
            .flex_none()
            .min_w_0()
            .min_h(px(72.))
            .flex()
            .border_b_1()
            .border_color(rgb(0xd9dade))
            .child(
                div()
                    .w(px(58.))
                    .pt_3()
                    .pr_2()
                    .text_right()
                    .text_size(px(10.))
                    .text_color(rgb(0x9a9da3))
                    .border_r_1()
                    .border_color(rgb(0xe5e6e8))
                    .child(language.text("agenda.all_day")),
            );
        for date in days {
            let drop_workspace = workspace.clone();
            let drop_date = *date;
            let mut cell = div()
                .flex_1()
                .min_w_0()
                .p_1()
                .flex()
                .flex_col()
                .gap_1()
                .border_r_1()
                .border_color(rgb(0xececef));
            for (index, task) in result
                .placements
                .iter()
                .enumerate()
                .filter(|(_, placement)| {
                    placement.date == Some(*date) && placement.start_time.is_none()
                })
                .filter_map(|(index, placement)| {
                    result
                        .entries
                        .get(placement.entry.0 as usize)
                        .map(|entry| (index, &entry.row))
                })
                .take(2)
            {
                cell = cell.child(draggable_task(
                    task_chip(workspace.clone(), task, index),
                    task,
                    index,
                ));
            }
            row = row.child(cell.on_drop(move |drag: &CalendarDrag, _, cx| {
                if drag.resize {
                    return;
                }
                drop_workspace.update(cx, |this, cx| {
                    this.dispatch_agenda_intent(
                        UiIntent::MoveOccurrence(drag.index, drop_date, None),
                        cx,
                    )
                });
            }));
        }
        row
    });

    let mut lanes = HashMap::new();
    let mut body = div()
        .relative()
        .w_full()
        .min_w_0()
        .h(px(702.))
        .flex_none()
        .flex()
        .child(
            div()
                .w(px(58.))
                .flex_none()
                .flex()
                .flex_col()
                .children((8..21).map(|hour| {
                    div()
                        .h(px(54.))
                        .pr_2()
                        .text_right()
                        .text_size(px(9.))
                        .text_color(rgb(0x9a9da3))
                        .child(format!("{hour:02}:00"))
                })),
        );
    for (day_index, date) in days.iter().enumerate() {
        let mut track = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h(px(702.))
            .border_l_1()
            .border_color(rgb(0xececef));
        for hour in 0..13 {
            let drop_workspace = workspace.clone();
            let drop_date = *date;
            track = track.child(
                div()
                    .id(("calendar-drop", day_index * 24 + hour))
                    .absolute()
                    .top(px(hour as f32 * 54.))
                    .left_0()
                    .right_0()
                    .h(px(54.))
                    .border_t_1()
                    .border_color(rgb(0xf0f0f2))
                    .on_drop(move |drag: &CalendarDrag, _, cx| {
                        let time = jiff::civil::Time::new((hour + 8) as i8, 0, 0, 0).ok();
                        drop_workspace.update(cx, |this, cx| {
                            let intent = if drag.resize {
                                UiIntent::ResizeOccurrence(drag.index, drop_date, time.unwrap())
                            } else {
                                UiIntent::MoveOccurrence(drag.index, drop_date, time)
                            };
                            this.dispatch_agenda_intent(intent, cx)
                        });
                    }),
            );
        }
        for (index, placement) in result
            .placements
            .iter()
            .enumerate()
            .filter(|(_, placement)| {
                placement.date == Some(*date) && placement.start_time.is_some()
            })
        {
            let Some(task) = result
                .entries
                .get(placement.entry.0 as usize)
                .map(|entry| &entry.row)
            else {
                continue;
            };
            let time = placement.start_time.unwrap();
            let top = ((time.hour() as i32 - 8).max(0) as f32 * 54.) + time.minute() as f32 * 0.9;
            let height = placement.end_time.map_or(80., |end| {
                let start_minutes = time.hour() as i32 * 60 + time.minute() as i32;
                let end_minutes = end.hour() as i32 * 60 + end.minute() as i32;
                ((end_minutes - start_minutes).max(30) as f32 * 0.9).clamp(36., 216.)
            });
            let lane = lanes
                .entry((*date, time.hour(), time.minute()))
                .or_insert(0usize);
            let lane_index = *lane;
            let lane_count = result
                .placements
                .iter()
                .filter(|candidate| {
                    candidate.date == Some(*date)
                        && candidate
                            .start_time
                            .map(|value| (value.hour(), value.minute()))
                            == Some((time.hour(), time.minute()))
                })
                .count()
                .max(1);
            *lane += 1;
            track = track.child(
                div()
                    .absolute()
                    .top(px(top))
                    .left(gpui::relative(lane_index as f32 / lane_count as f32))
                    .w(gpui::relative(1. / lane_count as f32))
                    .px_1()
                    .h(px(height))
                    .child(draggable_task(
                        task_chip(workspace.clone(), task, index),
                        task,
                        index,
                    ))
                    .when_some(resize_handle(task, index), |event, handle| {
                        event.child(handle)
                    }),
            );
        }
        body = body.child(track);
    }
    div()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .flex()
        .flex_col()
        .child(header)
        .when_some(all_day_row, |view, row| view.child(row))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .id("calendar-time-scroll")
                .overflow_y_scroll()
                .child(body),
        )
        .into_any_element()
}

pub(crate) fn agenda_calendar(
    language: crate::i18n::Language,
    workspace: Entity<WorkspaceWindow>,
    result: Arc<AgendaResultSnapshot>,
    range: CalendarRange,
    window: (Date, Date),
    all_day: bool,
) -> Div {
    let mut days = Vec::new();
    let mut date = window.0;
    while date <= window.1 {
        days.push(date);
        let Ok(next) = date.tomorrow() else {
            break;
        };
        date = next;
    }
    let toggle = workspace.clone();
    div()
        .size_full()
        .min_w_0()
        .min_h_0()
        .flex()
        .flex_col()
        .bg(rgb(0xffffff))
        .child(
            div()
                .flex_none()
                .h(px(38.))
                .px_5()
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(rgb(0xe1e2e5))
                .bg(rgb(0xfbfbfc))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_4()
                        .text_size(px(10.))
                        .text_color(rgb(0x979aa0))
                        .child(
                            div()
                                .id("calendar-toggle-all-day")
                                .h(px(30.))
                                .px_2()
                                .flex()
                                .items_center()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(0xeeeef1)))
                                .active(|style| style.opacity(0.72))
                                .text_color(rgb(0x62666d))
                                .child(if all_day {
                                    language.text("agenda.collapse_all_day")
                                } else {
                                    language.text("agenda.expand_all_day")
                                })
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    toggle.update(cx, |this, cx| {
                                        this.dispatch_agenda_intent(UiIntent::ToggleAllDay, cx)
                                    })
                                }),
                        ),
                ),
        )
        .child(if range == CalendarRange::Month {
            month_view(language, workspace, &result, &days)
        } else {
            time_view(language, workspace, &result, &days, all_day)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_calendar_keeps_seven_days_across_dst_boundary() {
        let start = Date::new(2026, 3, 8).unwrap();
        let days = dates(start, CalendarRange::Week);
        assert_eq!(days.len(), 7);
        assert_eq!(days[6], Date::new(2026, 3, 14).unwrap());
    }

    #[test]
    fn month_projection_has_five_complete_weeks() {
        assert_eq!(
            dates(Date::new(2026, 9, 1).unwrap(), CalendarRange::Month).len(),
            35
        );
    }
}
