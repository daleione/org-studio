use std::sync::Arc;

use gpui::{FontWeight, div, prelude::*, px, rgb};

use crate::{preview::command_window, theme::current_theme};

pub(super) fn dired_help_window(
    items: Arc<Vec<(Arc<str>, Arc<str>)>>,
    available_width: f32,
) -> gpui::Div {
    use command_window::{CommandGroup, CommandWindow};

    let take = |title: &str, keys: &[&str], columns| CommandGroup {
        title: Arc::from(title),
        max_columns: columns,
        items: keys
            .iter()
            .filter_map(|key| {
                items
                    .iter()
                    .find(|(candidate, _)| candidate.as_ref() == *key)
                    .cloned()
            })
            .collect(),
    };
    CommandWindow {
        title: Arc::from("Dired Commands"),
        close: Some((Arc::from("C-g"), Arc::from("Close"))),
        groups: vec![
            take(
                "NAVIGATION",
                &["n / j", "p / k", "^ / h", "H", "L", "g", "q"],
                3,
            ),
            take("MARKS", &["m", "u", "U", "t", "d"], 3),
            take("FILES", &["RET / l", "x"], 2),
            take("GLOBAL", &["C-x d", "C-x C-d"], 2),
        ],
    }
    .render(available_width)
}

pub(super) fn which_key_window(
    items: Arc<Vec<(Arc<str>, Arc<str>)>>,
    available_width: f32,
) -> gpui::Div {
    use command_window::{CommandGroup, CommandWindow};

    let columns = items.len().clamp(1, 6);
    CommandWindow {
        title: Arc::from("Available Commands"),
        close: None,
        groups: vec![CommandGroup {
            title: Arc::from("COMMANDS"),
            max_columns: columns,
            items: items.as_ref().clone(),
        }],
    }
    .render(available_width)
}

pub(in crate::preview) fn centered_message(title: &str, detail: &str) -> gpui::Div {
    let theme = current_theme();
    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(rgb(theme.background))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            div()
                .text_size(px(20.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme.heading[0]))
                .child(title.to_owned()),
        )
        .child(
            div()
                .max_w(px(520.0))
                .text_size(px(14.0))
                .line_height(px(21.0))
                .text_color(rgb(theme.foreground_dim))
                .child(detail.to_owned()),
        )
}
