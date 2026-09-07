use super::{AgendaDateKind, AgendaResultSnapshot};
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

/// Presentation-neutral text and UTF-8 ranges for Org agenda faces.
pub(crate) struct AgendaTextLine {
    pub(crate) text: String,
    pub(crate) todo: Range<usize>,
    pub(crate) priority: Option<Range<usize>>,
    pub(crate) tags: Option<Range<usize>>,
}

pub(crate) fn format_agenda_text(
    result: &AgendaResultSnapshot,
    scheduled: &str,
    deadline: &str,
) -> Vec<AgendaTextLine> {
    let category_width = result
        .rows
        .iter()
        .map(|row| UnicodeWidthStr::width(category(row).as_str()))
        .max()
        .unwrap_or(0)
        .max(10);
    result
        .rows
        .iter()
        .map(|row| {
            let category = category(row);
            let padding = " "
                .repeat(category_width.saturating_sub(UnicodeWidthStr::width(category.as_str())));
            let mut text = format!("  {category}:{padding} ");
            if let Some(time) = row.time {
                let time = match (row.date, row.end_date, row.end_time) {
                    (Some(start), Some(end), end_time) if end != start => format!(
                        "{} {}–{}{}",
                        start.strftime("%m-%d"),
                        time.strftime("%H:%M"),
                        end.strftime("%m-%d"),
                        end_time
                            .map(|time| format!(" {}", time.strftime("%H:%M")))
                            .unwrap_or_default(),
                    ),
                    (_, _, Some(end)) => {
                        format!("{}-{}", time.strftime("%H:%M"), end.strftime("%H:%M"))
                    }
                    _ => time.strftime("%H:%M").to_string(),
                };
                text.push_str(&time);
                text.push_str(&".".repeat(12_usize.saturating_sub(time.len())));
                text.push(' ');
            }
            let prefix = match row.date_kind {
                Some(AgendaDateKind::Scheduled) => format!("{scheduled}:"),
                Some(AgendaDateKind::Deadline) => format!("{deadline}:"),
                _ => String::new(),
            };
            let prefix_width =
                (UnicodeWidthStr::width(scheduled) + 1).max(UnicodeWidthStr::width(deadline) + 1);
            if !prefix.is_empty() || row.time.is_none() && row.date.is_some() {
                text.push_str(&prefix);
                text.push_str(&" ".repeat(
                    prefix_width.saturating_sub(UnicodeWidthStr::width(prefix.as_str())) + 1,
                ));
            }
            let todo_start = text.len();
            text.push_str(&row.todo);
            let todo = todo_start..text.len();
            if !row.todo.is_empty() {
                text.push(' ');
            }
            let priority = row.priority.map(|priority| {
                let start = text.len();
                text.push_str(&format!("[#{priority}]"));
                let range = start..text.len();
                text.push(' ');
                range
            });
            text.push_str(&row.title);
            let tags = (!row.tags.is_empty()).then(|| {
                text.push_str(
                    &" ".repeat(
                        80_usize
                            .saturating_sub(UnicodeWidthStr::width(text.as_str()))
                            .max(2),
                    ),
                );
                let start = text.len();
                text.push(':');
                for tag in row.tags.iter() {
                    text.push_str(tag);
                    text.push(':');
                }
                start..text.len()
            });
            AgendaTextLine {
                text,
                todo,
                priority,
                tags,
            }
        })
        .collect()
}

fn category(row: &super::AgendaRow) -> String {
    row.category
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            row.source
                .path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
}
