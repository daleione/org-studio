use std::sync::Arc;

use jiff::{Span, civil::Date};

use crate::org_semantic::TimestampKind;

use super::{
    AgendaDateKind, AgendaEntry, AgendaEntryKey, AgendaFacets, AgendaIndexSnapshot,
    AgendaOccurrence, AgendaPlacement, AgendaPlacementGroup, AgendaPlacementKey,
    AgendaResultSnapshot, AgendaRow, AgendaTimestampIdentity, TaskRecord,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BuiltinQuery {
    Today,
    NextSevenDays,
    Overdue,
    Next,
    Waiting,
    Unscheduled,
}

#[derive(Clone, Debug)]
pub(crate) struct AgendaQuery {
    pub(crate) builtin: BuiltinQuery,
    pub(crate) today: Date,
    pub(crate) text: Option<Arc<str>>,
    pub(crate) tag: Option<Arc<str>>,
    pub(crate) source: Option<super::FileId>,
    pub(crate) todo: Option<Arc<str>>,
    pub(crate) scheduled: Option<bool>,
    pub(crate) window: Option<(Date, Date)>,
}

impl AgendaQuery {
    pub(crate) fn builtin(builtin: BuiltinQuery, today: Date) -> Self {
        Self {
            builtin,
            today,
            text: None,
            tag: None,
            source: None,
            todo: None,
            scheduled: None,
            window: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct QueryEngine {
    next_generation: u64,
}

impl QueryEngine {
    pub(crate) fn execute(
        &mut self,
        index: Arc<AgendaIndexSnapshot>,
        query: &AgendaQuery,
    ) -> AgendaResultSnapshot {
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("query generation exhausted");
        let end = query
            .today
            .checked_add(Span::new().days(6))
            .unwrap_or(query.today);
        let (visible_start, visible_end) = query.window.unwrap_or((query.today, end));
        let mut rows = Vec::new();
        let mut facets = AgendaFacets::default();
        let diagnostics = index
            .files
            .iter()
            .flat_map(|shard| shard.diagnostics.iter().cloned())
            .collect::<Vec<_>>();
        for shard in index
            .files
            .iter()
            .filter(|shard| query.source.is_none_or(|file| shard.file == file))
        {
            for task in shard.tasks.iter() {
                if matches!(task.todo_kind, crate::org_semantic::TodoStateKind::Done) {
                    continue;
                }
                let occurrences =
                    active_occurrences(task, visible_start.min(query.today), visible_end.max(end));
                update_facets(task, &occurrences, query.today, end, &mut facets);
                if !matches_filters(task, query) {
                    continue;
                }
                if query.window.is_some() {
                    rows.extend(occurrences.into_iter().filter(|row| {
                        row.date.is_some_and(|date| {
                            date <= visible_end && row.end_date.unwrap_or(date) >= visible_start
                        })
                    }));
                    continue;
                }
                match query.builtin {
                    BuiltinQuery::Today => rows.extend(
                        occurrences
                            .iter()
                            .filter(|row| row.date == Some(query.today))
                            .cloned(),
                    ),
                    BuiltinQuery::NextSevenDays => rows.extend(
                        occurrences
                            .iter()
                            .filter(|row| {
                                row.date
                                    .is_some_and(|date| date >= query.today && date <= end)
                            })
                            .cloned(),
                    ),
                    BuiltinQuery::Overdue => rows.extend(
                        occurrences
                            .iter()
                            .filter(|row| row.date.is_some_and(|date| date < query.today))
                            .cloned(),
                    ),
                    BuiltinQuery::Next if task.todo.eq_ignore_ascii_case("NEXT") => {
                        rows.push(task_row(task))
                    }
                    BuiltinQuery::Waiting
                        if task.todo.eq_ignore_ascii_case("WAIT")
                            || task.todo.eq_ignore_ascii_case("WAITING") =>
                    {
                        rows.push(task_row(task))
                    }
                    BuiltinQuery::Unscheduled if occurrences.is_empty() => {
                        rows.push(task_row(task))
                    }
                    _ => {}
                }
            }
        }
        rows.sort_by(compare_rows);
        let entries = rows
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, row)| {
                let occurrence = row.timestamp_range.zip(row.date_kind).zip(row.date).map(
                    |((source_range, kind), start_date)| AgendaOccurrence {
                        timestamp: AgendaTimestampIdentity { source_range, kind },
                        start_date,
                        start_time: row.time,
                        end_date: row.end_date,
                        end_time: row.end_time,
                    },
                );
                AgendaEntry {
                    key: AgendaEntryKey(index as u32),
                    row,
                    occurrence,
                }
            })
            .collect::<Vec<_>>();
        let mut placements = entries
            .iter()
            .flat_map(|entry| entry_placements(entry, query.window))
            .collect::<Vec<_>>();
        placements.sort_by(|left, right| {
            left.date
                .cmp(&right.date)
                .then_with(|| left.start_time.cmp(&right.start_time))
                .then_with(|| left.entry.cmp(&right.entry))
        });
        for (index, placement) in placements.iter_mut().enumerate() {
            placement.key = AgendaPlacementKey(index as u32);
        }
        let mut placement_groups = Vec::new();
        let mut placement_start = 0;
        while placement_start < placements.len() {
            let date = placements[placement_start].date;
            let mut placement_end = placement_start + 1;
            while placement_end < placements.len() && placements[placement_end].date == date {
                placement_end += 1;
            }
            placement_groups.push(AgendaPlacementGroup {
                date,
                placements: placement_start..placement_end,
            });
            placement_start = placement_end;
        }
        AgendaResultSnapshot {
            query_id: None,
            query: Arc::new(query.clone()),
            index_generation: index.generation,
            query_generation: self.next_generation,
            facets,
            diagnostics: diagnostics.into(),
            entries: entries.into(),
            placements: placements.into(),
            placement_groups: placement_groups.into(),
        }
    }
}

fn matches_filters(task: &TaskRecord, query: &AgendaQuery) -> bool {
    let tag_matches = query.tag.as_ref().is_none_or(|tag| {
        task.effective_tags
            .iter()
            .any(|value| value.eq_ignore_ascii_case(tag))
    });
    let text_matches = query
        .text
        .as_ref()
        .is_none_or(|needle| task_matches_text(task, needle));
    let todo_matches = query
        .todo
        .as_ref()
        .is_none_or(|todo| task.todo.eq_ignore_ascii_case(todo));
    let scheduled_matches = query.scheduled.is_none_or(|expected| {
        task.timestamps.iter().any(|timestamp| {
            matches!(
                timestamp.kind,
                crate::org_semantic::TimestampKind::Scheduled
            )
        }) == expected
    });
    tag_matches && text_matches && todo_matches && scheduled_matches
}

pub(crate) fn task_matches_text(task: &TaskRecord, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    task.title.to_lowercase().contains(&needle)
        || task.todo.to_lowercase().contains(&needle)
        || task
            .effective_tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(&needle))
        || task
            .category
            .as_ref()
            .is_some_and(|category| category.to_lowercase().contains(&needle))
}

fn active_occurrences(task: &TaskRecord, start: Date, end: Date) -> Vec<AgendaRow> {
    let mut rows = Vec::new();
    for timestamp in task
        .timestamps
        .iter()
        .filter(|timestamp| timestamp.active && timestamp.kind != TimestampKind::Closed)
    {
        let mut date = timestamp.start_date;
        let mut first = true;
        loop {
            if first || date >= start || timestamp.repeater.is_none() {
                let kind = match timestamp.kind {
                    TimestampKind::Scheduled => AgendaDateKind::Scheduled,
                    TimestampKind::Deadline => AgendaDateKind::Deadline,
                    _ => AgendaDateKind::Plain,
                };
                rows.push(occurrence_row(
                    task,
                    date,
                    timestamp.start_time,
                    repeated_end_date(timestamp, date),
                    timestamp.end_time,
                    kind,
                    timestamp.source_range,
                ));
            }
            let Some(repeater) = timestamp.repeater else {
                break;
            };
            if date > end {
                break;
            }
            let span = match repeater.unit {
                crate::org_semantic::TimeUnit::Hour | crate::org_semantic::TimeUnit::Day => {
                    Span::new().days(repeater.value as i64)
                }
                crate::org_semantic::TimeUnit::Week => Span::new().weeks(repeater.value as i64),
                crate::org_semantic::TimeUnit::Month => Span::new().months(repeater.value as i64),
                crate::org_semantic::TimeUnit::Year => Span::new().years(repeater.value as i64),
            };
            let Ok(next) = date.checked_add(span) else {
                break;
            };
            if next <= date {
                break;
            }
            date = next;
            first = false;
        }
    }
    rows
}

fn update_facets(
    task: &TaskRecord,
    rows: &[AgendaRow],
    today: Date,
    end: Date,
    facets: &mut AgendaFacets,
) {
    if rows.iter().any(|row| row.date == Some(today)) {
        facets.today += 1;
    }
    if rows
        .iter()
        .any(|row| row.date.is_some_and(|date| date >= today && date <= end))
    {
        facets.next_seven_days += 1;
    }
    if rows
        .iter()
        .any(|row| row.date.is_some_and(|date| date < today))
    {
        facets.overdue += 1;
    }
    if task.todo.eq_ignore_ascii_case("NEXT") {
        facets.next += 1;
    }
    if task.todo.eq_ignore_ascii_case("WAIT") || task.todo.eq_ignore_ascii_case("WAITING") {
        facets.waiting += 1;
    }
    if rows.is_empty() {
        facets.unscheduled += 1;
    }
}

fn task_row(task: &TaskRecord) -> AgendaRow {
    AgendaRow {
        task: task.key,
        source: task.source.clone(),
        title: task.title.clone(),
        todo: task.todo.clone(),
        priority: task.priority,
        tags: task.effective_tags.clone(),
        category: task.category.clone(),
        date: None,
        time: None,
        end_date: None,
        end_time: None,
        date_kind: None,
        timestamp_range: None,
    }
}
fn occurrence_row(
    task: &TaskRecord,
    date: Date,
    time: Option<jiff::civil::Time>,
    end_date: Option<Date>,
    end_time: Option<jiff::civil::Time>,
    kind: AgendaDateKind,
    timestamp_range: crate::document::ByteRange,
) -> AgendaRow {
    let mut row = task_row(task);
    row.date = Some(date);
    row.time = time;
    row.end_date = end_date;
    row.end_time = end_time;
    row.date_kind = Some(kind);
    row.timestamp_range = Some(timestamp_range);
    row
}

fn repeated_end_date(
    timestamp: &crate::org_semantic::OrgTimestamp,
    occurrence: Date,
) -> Option<Date> {
    timestamp.end_date.and_then(|end| {
        let duration = end.since(timestamp.start_date).ok()?;
        occurrence.checked_add(duration).ok()
    })
}

fn entry_placements(entry: &AgendaEntry, window: Option<(Date, Date)>) -> Vec<AgendaPlacement> {
    let row = &entry.row;
    let Some(start) = row.date else {
        return vec![AgendaPlacement {
            key: AgendaPlacementKey(0),
            entry: entry.key,
            date: None,
            start_time: None,
            end_time: None,
            continues_before: false,
            continues_after: false,
        }];
    };
    let end = row.end_date.unwrap_or(start);
    let timed = row.time.is_some();
    let midnight = jiff::civil::Time::new(0, 0, 0, 0).expect("midnight is valid");
    let display_end = if timed && end > start && row.end_time == Some(midnight) {
        end.checked_sub(Span::new().days(1)).unwrap_or(start)
    } else {
        end
    };
    let mut result = Vec::new();
    let mut date = window.map_or(start, |(window_start, _)| start.max(window_start));
    let display_end = window.map_or(display_end, |(_, window_end)| display_end.min(window_end));
    while date <= display_end {
        result.push(AgendaPlacement {
            key: AgendaPlacementKey(0),
            entry: entry.key,
            date: Some(date),
            start_time: if date == start { row.time } else { None },
            end_time: if date == end { row.end_time } else { None },
            continues_before: date > start,
            continues_after: date < end,
        });
        let Ok(next) = date.checked_add(Span::new().days(1)) else {
            break;
        };
        date = next;
    }
    result
}

fn compare_rows(left: &AgendaRow, right: &AgendaRow) -> std::cmp::Ordering {
    left.date
        .cmp(&right.date)
        .then_with(|| left.time.cmp(&right.time))
        .then_with(|| right.priority.cmp(&left.priority))
        .then_with(|| left.category.cmp(&right.category))
        .then_with(|| left.title.cmp(&right.title))
}
