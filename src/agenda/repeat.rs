use crate::org_semantic::{Repeater, RepeaterMode, TimeUnit, TodoStateKind};
use jiff::{Span, civil::Date};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RepeatCompletionAction {
    Occurrence,
    Series,
    Cancel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TodoTransition {
    pub(crate) target: Arc<str>,
    pub(crate) target_kind: TodoStateKind,
    pub(crate) timestamp: Arc<str>,
    pub(crate) log_state: bool,
    pub(crate) add_tags: Arc<[Arc<str>]>,
    pub(crate) remove_tags: Arc<[Arc<str>]>,
}

pub(crate) fn transition_subtree(
    source: &str,
    old_todo: &str,
    transition: &TodoTransition,
) -> Result<String, &'static str> {
    let line_end = source.find('\n').unwrap_or(source.len());
    let marker = format!(" {old_todo} ");
    let position = source[..line_end]
        .find(&marker)
        .ok_or("source TODO changed")?;
    let mut result = source.to_owned();
    result.replace_range(
        position + 1..position + 1 + old_todo.len(),
        &transition.target,
    );
    apply_heading_tags(&mut result, transition);
    if !result.contains('\n') {
        result.push('\n');
    }
    let heading_end = result.find('\n').unwrap_or(result.len());
    let body_start = if heading_end < result.len() {
        heading_end + 1
    } else {
        heading_end
    };
    let closed = result[body_start..]
        .lines()
        .next()
        .filter(|line| line.trim_start().starts_with("CLOSED:"))
        .map(|line| {
            (
                body_start,
                body_start + line.len() + usize::from(body_start + line.len() < result.len()),
            )
        });
    if matches!(transition.target_kind, TodoStateKind::Done) {
        if closed.is_none() {
            result.insert_str(body_start, &format!("CLOSED: [{}]\n", transition.timestamp));
        }
    } else if let Some((start, end)) = closed {
        result.replace_range(start..end, "");
    }
    if transition.log_state {
        let entry = format!(
            "- State \"{}\" from \"{}\" [{}]\n",
            transition.target, old_todo, transition.timestamp
        );
        if let Some((start, _)) = super::command::own_drawer(&result, "LOGBOOK") {
            result.insert_str(start, &entry);
        } else {
            let insertion = result.find('\n').map_or(result.len(), |index| index + 1);
            result.insert_str(insertion, &format!(":LOGBOOK:\n{entry}:END:\n"));
        }
    }
    Ok(result)
}

fn apply_heading_tags(source: &mut String, transition: &TodoTransition) {
    let line_end = source.find('\n').unwrap_or(source.len());
    let line = &source[..line_end];
    let mut tags = line
        .split_whitespace()
        .last()
        .filter(|value| value.starts_with(':') && value.ends_with(':'))
        .map(|value| {
            value
                .trim_matches(':')
                .split(':')
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    for tag in transition.remove_tags.iter() {
        tags.remove(tag.as_ref());
    }
    tags.extend(transition.add_tags.iter().map(ToString::to_string));
    let old_start = line
        .rfind(" :")
        .filter(|start| line[*start + 1..].ends_with(':'));
    let replacement = if tags.is_empty() {
        String::new()
    } else {
        format!(" :{}:", tags.into_iter().collect::<Vec<_>>().join(":"))
    };
    if let Some(start) = old_start {
        source.replace_range(start..line_end, &replacement);
    } else {
        source.insert_str(line_end, &replacement);
    }
}

pub(crate) fn next_repeat_date(
    scheduled: Date,
    completed: Date,
    repeater: Repeater,
) -> Option<Date> {
    let span = match repeater.unit {
        TimeUnit::Hour | TimeUnit::Day => Span::new().days(i64::from(repeater.value)),
        TimeUnit::Week => Span::new().weeks(i64::from(repeater.value)),
        TimeUnit::Month => Span::new().months(i64::from(repeater.value)),
        TimeUnit::Year => Span::new().years(i64::from(repeater.value)),
    };
    match repeater.mode {
        RepeaterMode::Cumulative => scheduled.checked_add(span).ok(),
        RepeaterMode::Restart => completed.checked_add(span).ok(),
        RepeaterMode::CatchUp => {
            let mut next = scheduled;
            for _ in 0..100_000 {
                next = next.checked_add(span).ok()?;
                if next > completed {
                    return Some(next);
                }
            }
            None
        }
    }
}
