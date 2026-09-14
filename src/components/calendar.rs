//! Reusable month grid. Hosts receive dates/ranges; no Org or document knowledge.
use gpui::{
    App, IntoElement, MouseButton, ParentElement, Styled, Window, div, prelude::*, px, rgb,
};
use jiff::{Span, civil::Date};
use std::{cell::Cell, rc::Rc};

type SelectDate = Rc<dyn Fn(Date, &mut Window, &mut App)>;
type SelectRange = Rc<dyn Fn(Date, Date, &mut Window, &mut App)>;
/// Shared across renders so pointer gestures survive live selection updates.
pub(crate) type CalendarDrag = Rc<Cell<Option<(Date, Date)>>>;

use crate::i18n::Language;

pub(crate) struct Calendar {
    pub(crate) language: Language,
    pub(crate) month: Date,
    pub(crate) selected: Date,
    pub(crate) range: Option<(Date, Date)>,
    pub(crate) today: Date,
    pub(crate) drag: CalendarDrag,
    pub(crate) hide_outside_month: bool,
    pub(crate) navigation: (bool, bool),
    pub(crate) on_preview: SelectDate,
    pub(crate) on_select: SelectDate,
    pub(crate) on_range: SelectRange,
    pub(crate) on_month: SelectDate,
}

fn month_label(date: Date, language: Language) -> &'static str {
    language.text(
        [
            "calendar.month.1",
            "calendar.month.2",
            "calendar.month.3",
            "calendar.month.4",
            "calendar.month.5",
            "calendar.month.6",
            "calendar.month.7",
            "calendar.month.8",
            "calendar.month.9",
            "calendar.month.10",
            "calendar.month.11",
            "calendar.month.12",
        ][date.month() as usize - 1],
    )
}

pub(crate) fn date_label(date: Date, language: Language) -> String {
    let number = |n: i8| {
        if language == Language::English {
            format!("{n:02}")
        } else {
            n.to_string()
        }
    };
    language
        .text("calendar.date")
        .replace("{year}", &date.year().to_string())
        .replace("{month}", &number(date.month()))
        .replace("{day}", &number(date.day()))
}

pub(crate) fn month_days(month: Date) -> Vec<Date> {
    let first = month.first_of_month();
    let offset = first.weekday().to_monday_zero_offset();
    let count = ((i32::from(offset) + i32::from(first.days_in_month()) + 6) / 7) * 7;
    (0..count)
        .filter_map(|day| {
            first
                .checked_add(Span::new().days(day - i32::from(offset)))
                .ok()
        })
        .collect()
}

impl Calendar {
    pub(crate) fn render(self) -> impl IntoElement {
        let mut header = div().flex().items_center().mb_2().child(
            div()
                .flex_1()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(
                    self.language
                        .text("calendar.month_heading")
                        .replace("{year}", &self.month.year().to_string())
                        .replace("{month}", month_label(self.month, self.language)),
                ),
        );
        for (id, label, delta) in [("previous", "‹", -1), ("next", "›", 1)] {
            if (delta < 0 && !self.navigation.0) || (delta > 0 && !self.navigation.1) {
                continue;
            }
            let on_month = self.on_month.clone();
            let target = self
                .month
                .first_of_month()
                .checked_add(Span::new().months(delta))
                .ok();
            header = header.child(
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .w(px(30.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0xf0f3f8)))
                    .child(label)
                    .on_click(move |_, window, cx| {
                        if let Some(date) = target {
                            on_month(date, window, cx);
                        }
                    }),
            );
        }
        // Explicit seven-cell rows avoid percentage rounding wrapping Sunday onto
        // the next line. Headers and dates share identical flex geometry.
        let mut weekdays = div().flex().w_full().h(px(28.));
        for (column, name) in [
            "calendar.weekday.0",
            "calendar.weekday.1",
            "calendar.weekday.2",
            "calendar.weekday.3",
            "calendar.weekday.4",
            "calendar.weekday.5",
            "calendar.weekday.6",
        ]
        .into_iter()
        .enumerate()
        {
            weekdays = weekdays.child(
                div()
                    .debug_selector(move || format!("calendar-weekday-{column}"))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(0x92949d))
                    .text_size(px(11.))
                    .child(self.language.text(name)),
            );
        }
        let mut grid = div().w_full().flex().flex_col().child(weekdays);
        let range_mode = self.range.is_some();
        for week in month_days(self.month).chunks(7) {
            let mut row = div().flex().w_full().h(px(34.)).flex_none();
            for (column, &date) in week.iter().enumerate() {
                if self.hide_outside_month && date.month() != self.month.month() {
                    row = row.child(div().flex_1().min_w_0().h_full());
                    continue;
                }
                let in_range = self
                    .range
                    .is_some_and(|(start, end)| date >= start && date <= end);
                let range_start = self.range.is_some_and(|(start, _)| date == start);
                let range_end = self.range.is_some_and(|(_, end)| date == end);
                let selected = if range_mode {
                    range_start
                } else {
                    date == self.selected
                };
                let down_drag = self.drag.clone();
                let move_drag = self.drag.clone();
                let up_drag = self.drag.clone();
                let down_select = self.on_select.clone();
                let up_select = self.on_select.clone();
                let preview = self.on_preview.clone();
                let move_range = self.on_range.clone();
                let up_range = self.on_range.clone();
                row = row.child(
                    div()
                        .id(gpui::SharedString::from(format!("calendar-cell-{date}")))
                        .debug_selector(move || format!("calendar-day-{date}"))
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .py(px(2.))
                        .cursor_pointer()
                        .child(
                            div()
                                .relative()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(in_range, |s| s.bg(rgb(0xe3eeff)))
                                .when(in_range && (range_start || column == 0), |s| {
                                    s.rounded_l_full()
                                })
                                .when(in_range && (range_end || column == 6), |s| {
                                    s.rounded_r_full()
                                })
                                .child(
                                    div()
                                        .relative()
                                        .size(px(30.))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .when(range_mode, |s| s.rounded_full())
                                        .when(!range_mode, |s| s.rounded(px(8.)))
                                        .when(selected, |s| s.bg(rgb(0x3f78f2)))
                                        .text_color(rgb(if selected {
                                            0xffffff
                                        } else if in_range {
                                            0x3f78f2
                                        } else if date.month() != self.month.month() {
                                            0xaeb2bd
                                        } else {
                                            0x373942
                                        }))
                                        .child(date.day().to_string())
                                        .when(date == self.today, |s| {
                                            s.child(
                                                div()
                                                    .absolute()
                                                    .bottom(px(1.))
                                                    .size(px(3.))
                                                    .rounded_full()
                                                    .bg(rgb(if selected {
                                                        0xffffff
                                                    } else {
                                                        0x3f78f2
                                                    })),
                                            )
                                        }),
                                ),
                        )
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            if range_mode {
                                down_drag.set(Some((date, date)));
                            } else {
                                down_select(date, window, cx);
                            }
                            cx.stop_propagation();
                        })
                        .on_mouse_move(move |event, window, cx| {
                            if event.pressed_button.is_none() {
                                preview(date, window, cx);
                            }
                            if event.pressed_button == Some(MouseButton::Left)
                                && let Some((anchor, last)) = move_drag.get()
                                && date != last
                            {
                                move_drag.set(Some((anchor, date)));
                                move_range(anchor.min(date), anchor.max(date), window, cx);
                            }
                        })
                        .on_mouse_up(MouseButton::Left, move |_, window, cx| {
                            if let Some((anchor, last)) = up_drag.take() {
                                if anchor == last && anchor == date {
                                    up_select(date, window, cx);
                                } else {
                                    up_range(anchor.min(date), anchor.max(date), window, cx);
                                }
                            }
                            cx.stop_propagation();
                        }),
                );
            }
            grid = grid.child(row);
        }
        div()
            .id(gpui::SharedString::from(format!(
                "calendar-{}",
                self.month.first_of_month()
            )))
            .w_full()
            .flex_none()
            .child(header)
            .child(grid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grid_covers_month_with_monday_first_and_correct_leap_days() {
        for (year, month) in [(2026, 9), (2024, 2), (2026, 3), (2026, 2)] {
            let date = Date::new(year, month, 1).unwrap();
            let days = month_days(date);
            assert_eq!(days[0].weekday().to_monday_zero_offset(), 0);
            assert_eq!(days.len() % 7, 0);
            assert_eq!(
                days.iter().filter(|d| d.month() == month).count(),
                date.days_in_month() as usize
            );
            assert!(days.windows(2).all(|w| w[0].tomorrow().unwrap() == w[1]));
        }
        assert_eq!(
            month_days(Date::new(2026, 9, 1).unwrap())[15],
            Date::new(2026, 9, 15).unwrap()
        );
    }
}
