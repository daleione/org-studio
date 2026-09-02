use std::sync::Arc;

use gpui::{FontWeight, ParentElement, Pixels, Styled, div, prelude::*, px, relative, rgb};

use crate::theme::current_theme;

pub(super) type CommandItem = (Arc<str>, Arc<str>);

pub(super) struct CommandGroup {
    pub title: Arc<str>,
    pub max_columns: usize,
    pub items: Vec<CommandItem>,
}

pub(super) struct CommandWindow {
    pub title: Arc<str>,
    pub close: Option<CommandItem>,
    pub groups: Vec<CommandGroup>,
}

impl CommandWindow {
    pub fn render(mut self, available_width: f32) -> gpui::Div {
        let total_items = self
            .groups
            .iter()
            .map(|group| group.items.len())
            .sum::<usize>()
            .max(1);
        for group in &mut self.groups {
            let group_width = available_width * group.items.len() as f32 / total_items as f32;
            let minimum_column_width = group
                .items
                .iter()
                .map(|(_, title)| {
                    let title_width = title
                        .chars()
                        .map(|character| if character.is_ascii() { 7.6 } else { 13.0 })
                        .sum::<f32>();
                    102.0 + title_width
                })
                .fold(190.0_f32, f32::max);
            let fitting_columns = (group_width / minimum_column_width).floor().max(1.0) as usize;
            group.max_columns = group.max_columns.min(fitting_columns).max(1);
        }
        let rows = self
            .groups
            .iter()
            .map(|group| group.items.len().div_ceil(group.max_columns))
            .max()
            .unwrap_or(1)
            .max(1);
        let height = px(70.0 + rows as f32 * 27.0);
        div()
            .absolute()
            .left(px(0.0))
            .right(px(0.0))
            .bottom(px(0.0))
            .h(height)
            .bg(rgb(current_theme().background_alt))
            .border_t_1()
            .border_color(rgb(current_theme().border))
            .flex()
            .flex_col()
            .child(command_window_header(self.title, self.close))
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h(px(0.0))
                    .pb_3()
                    .flex()
                    .children(self.groups.into_iter().map(|group| {
                        let share = group.items.len() as f32 / total_items as f32;
                        render_group(group).w(relative(share)).flex_none()
                    })),
            )
    }
}

fn command_window_header(title: Arc<str>, close: Option<CommandItem>) -> gpui::Div {
    div()
        .w_full()
        .h(px(38.0))
        .px_5()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(4.0))
                        .h(px(16.0))
                        .rounded_sm()
                        .bg(rgb(current_theme().heading[0])),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(13.0))
                        .child(title.to_string()),
                ),
        )
        .when_some(close, |header, (key, label)| {
            header.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.0))
                    .text_color(rgb(current_theme().foreground_dim))
                    .child(key_cap(key, px(44.0)))
                    .child(label.to_string()),
            )
        })
}

fn render_group(group: CommandGroup) -> gpui::Div {
    let columns = group.max_columns.max(1).min(group.items.len().max(1));
    let rows_per_column = group.items.len().div_ceil(columns);
    div()
        .h_full()
        .min_w(px(0.0))
        .pl_4()
        .pr_3()
        .flex()
        .flex_col()
        .border_l_1()
        .border_color(rgb(current_theme().border))
        .child(
            div()
                .h(px(20.0))
                .text_size(px(10.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(current_theme().foreground_dim))
                .child(group.title.to_string()),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .gap_3()
                .children(group.items.chunks(rows_per_column).map(|items| {
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .flex_col()
                        .children(items.iter().cloned().map(command_row))
                })),
        )
}

fn command_row((key, title): CommandItem) -> gpui::Div {
    div()
        .h(px(27.0))
        .flex()
        .items_center()
        .gap_2()
        .child(key_cap(key, px(54.0)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.5))
                .text_color(rgb(current_theme().foreground))
                .child(title.to_string()),
        )
}

fn key_cap(key: Arc<str>, width: Pixels) -> gpui::Div {
    div()
        .w(width)
        .h(px(22.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .bg(rgb(current_theme().code_boundary_background))
        .text_color(rgb(current_theme().heading[0]))
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(12.0))
        .child(key.to_string())
}
