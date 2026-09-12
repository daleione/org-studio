use std::sync::Arc;

use super::command_window;

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
            take("GLOBAL", &["C-x C-b", "C-x d", "C-x C-d"], 3),
        ],
    }
    .render(available_width)
}
