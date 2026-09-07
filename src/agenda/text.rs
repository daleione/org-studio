use super::{AgendaDateKind, AgendaPlacementKey, AgendaResultSnapshot};
use jiff::civil::Date;
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

/// Presentation-neutral text and UTF-8 ranges for Org agenda faces.
#[derive(Clone)]
pub(crate) struct AgendaTextLine {
    pub(crate) text: String,
    pub(crate) todo: Range<usize>,
    pub(crate) priority: Option<Range<usize>>,
    pub(crate) tags: Option<Range<usize>>,
}

pub(crate) enum AgendaProjectedLine {
    Header {
        date: Option<Date>,
        text: String,
    },
    Entry {
        entry: usize,
        placement: AgendaPlacementKey,
        line: AgendaTextLine,
    },
}

pub(crate) fn project_agenda_text(
    result: &AgendaResultSnapshot,
    scheduled: &str,
    deadline: &str,
    unscheduled: &str,
    window: Option<(Date, Date)>,
    mut date_heading: impl FnMut(Date, bool) -> String,
) -> Vec<AgendaProjectedLine> {
    let mut groups =
        std::collections::BTreeMap::<Option<Date>, Vec<(usize, AgendaPlacementKey)>>::new();
    for group in result.placement_groups.iter() {
        groups.entry(group.date).or_default().extend(
            result.placements[group.placements.clone()]
                .iter()
                .map(|placement| (placement.entry.0 as usize, placement.key)),
        );
    }
    if let Some((start, end)) = window {
        let mut date = start;
        while date <= end {
            groups.entry(Some(date)).or_default();
            let Ok(next) = date.checked_add(jiff::Span::new().days(1)) else {
                break;
            };
            date = next;
        }
    }
    let formatted = format_agenda_text(result, scheduled, deadline);
    let mut output = Vec::new();
    for (group_index, (date, placements)) in groups.into_iter().enumerate() {
        output.push(AgendaProjectedLine::Header {
            date,
            text: date.map_or_else(
                || unscheduled.to_owned(),
                |date| {
                    date_heading(
                        date,
                        group_index == 0 || date.weekday().to_monday_zero_offset() == 0,
                    )
                },
            ),
        });
        for (entry, placement) in placements {
            let Some(line) = formatted.get(entry).cloned() else {
                continue;
            };
            output.push(AgendaProjectedLine::Entry {
                entry,
                placement,
                line,
            });
        }
    }
    output
}

pub(crate) fn format_agenda_text(
    result: &AgendaResultSnapshot,
    scheduled: &str,
    deadline: &str,
) -> Vec<AgendaTextLine> {
    let category_width = result
        .entries
        .iter()
        .map(|entry| UnicodeWidthStr::width(category(&entry.row).as_str()))
        .max()
        .unwrap_or(0)
        .max(10);
    result
        .entries
        .iter()
        .map(|entry| {
            let row = &entry.row;
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
