use std::{collections::HashSet, sync::Arc};

use jiff::{Span, civil::Date};

use crate::org_semantic::TimestampKind;

use super::{
    AgendaDateKind, AgendaDayGroup, AgendaFacets, AgendaIndexSnapshot, AgendaResultSnapshot,
    AgendaRow, TaskRecord,
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
        let mut groups = Vec::new();
        let mut start = 0;
        while start < rows.len() {
            let date = rows[start].date;
            let mut end = start + 1;
            while end < rows.len() && rows[end].date == date {
                end += 1;
            }
            groups.push(AgendaDayGroup {
                date,
                rows: start..end,
            });
            start = end;
        }
        AgendaResultSnapshot {
            index_generation: index.generation,
            query_generation: self.next_generation,
            rows: rows.into(),
            facets,
            diagnostics: diagnostics.into(),
            groups: groups.into(),
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
    let mut seen = HashSet::new();
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
                if seen.insert((date, timestamp.start_time, kind as u8)) {
                    rows.push(occurrence_row(
                        task,
                        date,
                        timestamp.start_time,
                        timestamp.end_date,
                        timestamp.end_time,
                        kind,
                    ));
                }
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
    }
}
fn occurrence_row(
    task: &TaskRecord,
    date: Date,
    time: Option<jiff::civil::Time>,
    end_date: Option<Date>,
    end_time: Option<jiff::civil::Time>,
    kind: AgendaDateKind,
) -> AgendaRow {
    let mut row = task_row(task);
    row.date = Some(date);
    row.time = time;
    row.end_date = end_date;
    row.end_time = end_time;
    row.date_kind = Some(kind);
    row
}

fn compare_rows(left: &AgendaRow, right: &AgendaRow) -> std::cmp::Ordering {
    left.date
        .cmp(&right.date)
        .then_with(|| left.time.cmp(&right.time))
        .then_with(|| right.priority.cmp(&left.priority))
        .then_with(|| left.category.cmp(&right.category))
        .then_with(|| left.title.cmp(&right.title))
}
