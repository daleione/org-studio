use std::sync::Arc;

use crate::{
    document::{
        ByteOffset, ByteRange, DocumentCommand, DocumentSession, EditOrigin, EditTransaction,
        RevisionRange, Selection, TextEdit, TextSnapshot,
    },
    org_semantic::{OrgTimestamp, TimestampKind, analyze},
    org_syntax,
};

use super::{SourceVersion, TaskRecord};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimestampTarget {
    Scheduled,
    Deadline,
    Plain,
}

#[derive(Clone, Debug)]
pub(crate) enum AgendaCommand {
    SetTodo(Option<Arc<str>>),
    TransitionTodo(super::TodoTransition),
    SetPriority(Option<char>),
    SetTimestamp {
        target: TimestampTarget,
        value: Option<OrgTimestamp>,
    },
    AppendLogbook(Arc<str>),
    CompleteRepeat {
        timestamp: OrgTimestamp,
        repeat_to_state: Arc<str>,
        completed_at: Arc<str>,
    },
    RefileSameFile(Box<TaskRecord>),
    DeleteSubtree,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AgendaEditError {
    DifferentFile,
    ExternalConflict,
    SourceChanged,
    AmbiguousSource,
    InvalidValue(&'static str),
}

pub(crate) struct PreparedAgendaEdit {
    pub(crate) command: DocumentCommand,
}

pub(crate) fn prepare_edit(
    session: &DocumentSession,
    task: &TaskRecord,
    operation: &AgendaCommand,
) -> Result<PreparedAgendaEdit, AgendaEditError> {
    if session.path() != task.source.path.as_path() {
        return Err(AgendaEditError::DifferentFile);
    }
    validate_version(session, task)?;
    let snapshot = session.snapshot();
    let heading = resolve_heading(session, task)?;
    let source = snapshot.copy_range(heading.source);
    let edit = match operation {
        AgendaCommand::SetTodo(todo) => {
            replace_heading(&snapshot, heading.content, &heading, |metadata| {
                metadata.todo = todo.clone()
            })?
        }
        AgendaCommand::TransitionTodo(transition) => TextEdit::new(
            heading.source,
            super::transition_subtree(&source, &task.todo, transition)
                .map_err(|_| AgendaEditError::SourceChanged)?,
        ),
        AgendaCommand::SetPriority(priority) => {
            replace_heading(&snapshot, heading.content, &heading, |metadata| {
                metadata.priority = *priority
            })?
        }
        AgendaCommand::SetTimestamp { target, value } => timestamp_edit(
            &snapshot,
            heading.source,
            &heading.timestamps,
            *target,
            value.as_ref(),
        )?,
        AgendaCommand::AppendLogbook(entry) => append_logbook(&snapshot, heading.source, entry)?,
        AgendaCommand::CompleteRepeat {
            timestamp,
            repeat_to_state,
            completed_at,
        } => {
            if matches!(task.source.version, SourceVersion::Live { revision, .. } if revision != session.revision())
            {
                return Err(AgendaEditError::SourceChanged);
            }
            let relative_start = timestamp
                .source_range
                .start
                .0
                .checked_sub(heading.source.start.0)
                .ok_or(AgendaEditError::SourceChanged)? as usize;
            let relative_end = timestamp
                .source_range
                .end
                .0
                .checked_sub(heading.source.start.0)
                .ok_or(AgendaEditError::SourceChanged)? as usize;
            if relative_end > source.len()
                || relative_start >= relative_end
                || !source.is_char_boundary(relative_start)
                || !source.is_char_boundary(relative_end)
            {
                return Err(AgendaEditError::SourceChanged);
            }
            let mut updated = source.clone();
            updated.replace_range(relative_start..relative_end, &format_timestamp(timestamp));
            updated = super::transition_subtree(
                &updated,
                &task.todo,
                &super::TodoTransition {
                    target: repeat_to_state.clone(),
                    target_kind: crate::org_semantic::TodoStateKind::Open,
                    timestamp: completed_at.clone(),
                    log_state: true,
                    add_tags: Arc::from([]),
                    remove_tags: Arc::from([]),
                },
            )
            .map_err(|_| AgendaEditError::SourceChanged)?;
            updated = set_property_in_subtree(updated, "LAST_REPEAT", completed_at)?;
            TextEdit::new(heading.source, updated)
        }
        AgendaCommand::DeleteSubtree => TextEdit::new(heading.source, ""),
        AgendaCommand::RefileSameFile(_) => TextEdit::new(ByteRange::new(0, 0), ""),
    };
    let edits = if let AgendaCommand::RefileSameFile(target) = operation {
        let target_heading = resolve_heading(session, target)?;
        if target_heading.source.start >= heading.source.start
            && target_heading.source.start < heading.source.end
        {
            return Err(AgendaEditError::InvalidValue(
                "cannot refile a subtree into itself",
            ));
        }
        let mut moved = relevel_subtree(
            &snapshot.copy_range(heading.source),
            heading.level,
            target_heading.level + 1,
        );
        if !snapshot.copy_range(target_heading.source).ends_with('\n') {
            moved.insert(0, '\n');
        }
        if !moved.ends_with('\n') {
            moved.push('\n');
        }
        vec![
            TextEdit::new(heading.source, ""),
            TextEdit::new(
                ByteRange::new(target_heading.source.end.0, target_heading.source.end.0),
                moved,
            ),
        ]
    } else {
        vec![edit]
    };
    let caret = ByteOffset(edits.last().map_or(heading.source.start.0, |edit| {
        edit.range.start.0 + edit.replacement.len() as u64
    }));
    let before = Selection::caret(heading.content.start);
    let after = Selection::caret(caret);
    let transaction = EditTransaction::new(snapshot.revision(), edits);
    let command = DocumentCommand::new(transaction, before, after, EditOrigin::Other);
    Ok(PreparedAgendaEdit { command })
}

fn set_property_in_subtree(
    mut source: String,
    key: &str,
    value: &str,
) -> Result<String, AgendaEditError> {
    let snapshot = crate::document::DocumentSnapshot::from_utf8(source.as_bytes().to_vec())
        .expect("source is UTF-8");
    let edit = property_edit(
        &snapshot,
        ByteRange::new(0, source.len() as u64),
        key,
        Some(value),
    )?;
    source.replace_range(
        edit.range.start.0 as usize..edit.range.end.0 as usize,
        &edit.replacement,
    );
    Ok(source)
}

fn relevel_subtree(source: &str, old_level: u16, new_level: u16) -> String {
    let delta = i32::from(new_level) - i32::from(old_level);
    source
        .split_inclusive('\n')
        .map(|line| {
            let stars = line.bytes().take_while(|byte| *byte == b'*').count();
            if stars > 0 && line.as_bytes().get(stars) == Some(&b' ') {
                let level = (stars as i32 + delta).max(1) as usize;
                format!("{}{}", "*".repeat(level), &line[stars..])
            } else {
                line.to_owned()
            }
        })
        .collect()
}

fn validate_version(session: &DocumentSession, task: &TaskRecord) -> Result<(), AgendaEditError> {
    match &task.source.version {
        SourceVersion::Live { document, .. } if *document != session.id() => {
            Err(AgendaEditError::DifferentFile)
        }
        SourceVersion::Live { .. } => Ok(()),
        SourceVersion::Disk(expected) => {
            if session.is_dirty() {
                return Err(AgendaEditError::ExternalConflict);
            }
            let actual = crate::document::FileStamp::read(session.path())
                .map_err(|_| AgendaEditError::ExternalConflict)?;
            (actual == **expected)
                .then_some(())
                .ok_or(AgendaEditError::ExternalConflict)
        }
    }
}

fn resolve_heading(
    session: &DocumentSession,
    task: &TaskRecord,
) -> Result<crate::org_semantic::OrgHeading, AgendaEditError> {
    let revision = match task.source.version {
        SourceVersion::Live { revision, .. } => revision,
        SourceVersion::Disk(_) => crate::document::Revision::INITIAL,
    };
    if let Ok(mapped) =
        session.map_range_to_current(RevisionRange::new(revision, task.source.heading_range))
    {
        let snapshot = session.snapshot();
        let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
        if let Some(heading) = analysis
            .headings
            .iter()
            .find(|heading| heading.source == mapped.range && heading.title == task.title)
        {
            return Ok(heading.clone());
        }
    }
    let snapshot = session.snapshot();
    let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
    if let Some(anchor) = &task.source.anchor {
        let matches = analysis
            .headings
            .iter()
            .filter(|heading| {
                heading.properties.iter().any(|(key, value)| {
                    key.eq_ignore_ascii_case(&anchor.kind) && value == &anchor.value
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [heading] => return Ok(heading.clone()),
            [_, ..] => return Err(AgendaEditError::AmbiguousSource),
            [] => {}
        }
    }
    let candidates = analysis
        .headings
        .iter()
        .filter(|heading| {
            heading.level == task.source.fingerprint.level
                && heading.title == task.source.fingerprint.title
                && task
                    .source
                    .fingerprint
                    .parent_title
                    .as_ref()
                    .is_none_or(|expected| {
                        heading
                            .parent
                            .and_then(|parent| {
                                analysis
                                    .headings
                                    .iter()
                                    .find(|candidate| candidate.syntax_id == parent)
                            })
                            .is_some_and(|parent| &parent.title == expected)
                    })
        })
        .cloned()
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [heading] => Ok(heading.clone()),
        [] => Err(AgendaEditError::SourceChanged),
        _ => Err(AgendaEditError::AmbiguousSource),
    }
}

struct HeadingMetadata {
    todo: Option<Arc<str>>,
    priority: Option<char>,
    title: Arc<str>,
    tags: Arc<[Arc<str>]>,
}

fn replace_heading(
    snapshot: &dyn TextSnapshot,
    range: ByteRange,
    heading: &crate::org_semantic::OrgHeading,
    change: impl FnOnce(&mut HeadingMetadata),
) -> Result<TextEdit, AgendaEditError> {
    let mut metadata = HeadingMetadata {
        todo: heading.todo.as_ref().map(|todo| todo.keyword.clone()),
        priority: heading.priority,
        title: heading.title.clone(),
        tags: heading.tags.clone(),
    };
    change(&mut metadata);
    validate_inline(&metadata.title)?;
    let mut replacement = String::new();
    if let Some(todo) = metadata.todo {
        validate_token(&todo)?;
        replacement.push_str(&todo);
        replacement.push(' ');
    }
    if let Some(priority) = metadata.priority {
        if !priority.is_ascii_alphabetic() {
            return Err(AgendaEditError::InvalidValue(
                "priority must be an ASCII letter",
            ));
        }
        replacement.push_str(&format!("[#{priority}] "));
    }
    replacement.push_str(&metadata.title);
    if !metadata.tags.is_empty() {
        replacement.push(' ');
        replacement.push(':');
        for tag in metadata.tags.iter() {
            replacement.push_str(tag);
            replacement.push(':');
        }
    }
    let original = snapshot.copy_range(range);
    if original.ends_with('\n') {
        replacement.push('\n');
    }
    Ok(TextEdit::new(range, replacement))
}

fn property_edit(
    snapshot: &dyn TextSnapshot,
    subtree: ByteRange,
    key: &str,
    value: Option<&str>,
) -> Result<TextEdit, AgendaEditError> {
    validate_token(key)?;
    if key.contains(':') {
        return Err(AgendaEditError::InvalidValue("invalid property key"));
    }
    if let Some(value) = value {
        validate_inline(value)?;
    }
    let text = snapshot.copy_range(subtree);
    let first_end = text.find('\n').map_or(text.len(), |index| index + 1);
    if let Some((drawer_start, end)) = own_drawer(&text, "PROPERTIES") {
        let drawer = &text[drawer_start..end];
        let prefix = format!(":{}:", key.to_ascii_uppercase());
        if let Some((start, finish)) = find_property_line(drawer, &prefix) {
            let range = ByteRange::new(
                subtree.start.0 + drawer_start as u64 + start as u64,
                subtree.start.0 + drawer_start as u64 + finish as u64,
            );
            return Ok(TextEdit::new(
                range,
                value
                    .map(|value| format!("{prefix} {value}\n"))
                    .unwrap_or_default(),
            ));
        }
        if let Some(value) = value {
            return Ok(TextEdit::new(
                ByteRange::new(subtree.start.0 + end as u64, subtree.start.0 + end as u64),
                format!("{prefix} {value}\n"),
            ));
        }
        return Ok(TextEdit::new(
            ByteRange::new(subtree.start.0, subtree.start.0),
            "",
        ));
    }
    let Some(value) = value else {
        return Ok(TextEdit::new(
            ByteRange::new(subtree.start.0, subtree.start.0),
            "",
        ));
    };
    Ok(TextEdit::new(
        ByteRange::new(
            subtree.start.0 + first_end as u64,
            subtree.start.0 + first_end as u64,
        ),
        format!(
            "{}:PROPERTIES:\n:{}: {value}\n:END:\n",
            if first_end == text.len() && !text.ends_with('\n') {
                "\n"
            } else {
                ""
            },
            key.to_ascii_uppercase()
        ),
    ))
}

fn find_property_line(drawer: &str, prefix: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    for line in drawer.split_inclusive('\n') {
        if line.trim_start().to_ascii_uppercase().starts_with(prefix) {
            return Some((offset, offset + line.len()));
        }
        offset += line.len();
    }
    None
}

fn append_logbook(
    snapshot: &dyn TextSnapshot,
    subtree: ByteRange,
    entry: &str,
) -> Result<TextEdit, AgendaEditError> {
    validate_inline(entry)?;
    let text = snapshot.copy_range(subtree);
    if let Some((start, _)) = own_drawer(&text, "LOGBOOK") {
        let insertion = subtree.start.0 + start as u64;
        Ok(TextEdit::new(
            ByteRange::new(insertion, insertion),
            format!("{entry}\n"),
        ))
    } else {
        let line_end = text.find('\n').map_or(text.len(), |index| index + 1);
        let insertion = subtree.start.0 + line_end as u64;
        Ok(TextEdit::new(
            ByteRange::new(insertion, insertion),
            format!(
                "{}:LOGBOOK:\n{entry}\n:END:\n",
                if line_end == text.len() && !text.ends_with('\n') {
                    "\n"
                } else {
                    ""
                }
            ),
        ))
    }
}

fn timestamp_edit(
    snapshot: &dyn TextSnapshot,
    subtree: ByteRange,
    timestamps: &[OrgTimestamp],
    target: TimestampTarget,
    value: Option<&OrgTimestamp>,
) -> Result<TextEdit, AgendaEditError> {
    let kind = match target {
        TimestampTarget::Scheduled => TimestampKind::Scheduled,
        TimestampTarget::Deadline => TimestampKind::Deadline,
        TimestampTarget::Plain => TimestampKind::Plain,
    };
    if let Some(existing) = timestamps.iter().find(|timestamp| timestamp.kind == kind) {
        return Ok(TextEdit::new(
            existing.source_range,
            value.map(format_timestamp).unwrap_or_default(),
        ));
    }
    let Some(value) = value else {
        return Ok(TextEdit::new(
            ByteRange::new(subtree.start.0, subtree.start.0),
            "",
        ));
    };
    let text = snapshot.copy_range(subtree);
    let line_end = text.find('\n').map_or(text.len(), |index| index + 1);
    let label = match target {
        TimestampTarget::Scheduled => "SCHEDULED: ",
        TimestampTarget::Deadline => "DEADLINE: ",
        TimestampTarget::Plain => "",
    };
    Ok(TextEdit::new(
        ByteRange::new(
            subtree.start.0 + line_end as u64,
            subtree.start.0 + line_end as u64,
        ),
        format!(
            "{}{label}{}\n",
            if line_end == text.len() && !text.ends_with('\n') {
                "\n"
            } else {
                ""
            },
            format_timestamp(value)
        ),
    ))
}

fn format_timestamp(value: &OrgTimestamp) -> String {
    use crate::org_semantic::{RepeaterMode, TimeUnit};
    let unit = |unit| match unit {
        TimeUnit::Hour => 'h',
        TimeUnit::Day => 'd',
        TimeUnit::Week => 'w',
        TimeUnit::Month => 'm',
        TimeUnit::Year => 'y',
    };
    let (open, close) = if value.active { ('<', '>') } else { ('[', ']') };
    let mut text = format!("{open}{}", value.start_date);
    if let Some(time) = value.start_time {
        text.push(' ');
        text.push_str(&time.strftime("%H:%M").to_string());
        if value.end_date.is_none()
            && let Some(end) = value.end_time
        {
            text.push_str(&format!("-{}", end.strftime("%H:%M")));
        }
    }
    if let Some(repeater) = value.repeater {
        let mode = match repeater.mode {
            RepeaterMode::Cumulative => "+",
            RepeaterMode::CatchUp => "++",
            RepeaterMode::Restart => ".+",
        };
        text.push_str(&format!(" {mode}{}{}", repeater.value, unit(repeater.unit)));
    }
    if let Some(warning) = value.warning {
        text.push_str(&format!(
            " {}{}{}",
            if warning.delayed { "--" } else { "-" },
            warning.value,
            unit(warning.unit)
        ));
    }
    text.push(close);
    if let Some(end) = value.end_date {
        text.push_str(&format!("--{open}{end}"));
        if let Some(time) = value.end_time {
            text.push_str(&format!(" {}", time.strftime("%H:%M")));
        }
        text.push(close);
    }
    text
}

pub(super) fn own_drawer(text: &str, name: &str) -> Option<(usize, usize)> {
    let snapshot = crate::document::DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).ok()?;
    let blocks = org_syntax::parse(&snapshot);
    let (heading, _) = blocks
        .nodes()
        .iter()
        .enumerate()
        .find(|(_, node)| matches!(node.kind, org_syntax::BlockKind::Heading { .. }))?;
    let drawer = blocks.nodes().iter().find(|node| {
        node.parent == Some(heading as u32)
            && matches!(&node.kind, org_syntax::BlockKind::Drawer { name: found } if found.eq_ignore_ascii_case(name))
    })?;
    let mut offset = drawer.source.start.0 as usize;
    let mut content_start = None;
    for line in text
        .get(offset..drawer.source.end.0 as usize)?
        .split_inclusive('\n')
    {
        if line.trim().eq_ignore_ascii_case(":END:") {
            return Some((content_start?, offset));
        }
        offset += line.len();
        content_start.get_or_insert(offset);
    }
    None
}

fn validate_inline(value: &str) -> Result<(), AgendaEditError> {
    if value.contains(['\r', '\n']) {
        Err(AgendaEditError::InvalidValue("value must fit on one line"))
    } else {
        Ok(())
    }
}
fn validate_token(value: &str) -> Result<(), AgendaEditError> {
    if value.is_empty() || value.contains(char::is_whitespace) {
        Err(AgendaEditError::InvalidValue("value must be one token"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(source: &str, build: impl FnOnce(&dyn TextSnapshot, ByteRange) -> TextEdit) -> String {
        let snapshot =
            crate::document::DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let edit = build(&snapshot, ByteRange::new(0, source.len() as u64));
        let mut result = source.to_owned();
        result.replace_range(
            edit.range.start.0 as usize..edit.range.end.0 as usize,
            &edit.replacement,
        );
        result
    }

    #[test]
    fn property_and_logbook_writes_do_not_touch_child_drawers() {
        let source = "* TODO Parent\n** TODO Child\n:PROPERTIES:\n:OWNER: child\n:END:\n:LOGBOOK:\nchild log\n:END:\n";
        let result = apply(source, |snapshot, range| {
            property_edit(snapshot, range, "OWNER", Some("parent")).unwrap()
        });
        assert_eq!(
            result,
            format!(
                "* TODO Parent\n:PROPERTIES:\n:OWNER: parent\n:END:\n{}",
                &source[14..]
            )
        );
        let result = apply(source, |snapshot, range| {
            append_logbook(snapshot, range, "parent log").unwrap()
        });
        assert_eq!(
            result,
            format!(
                "* TODO Parent\n:LOGBOOK:\nparent log\n:END:\n{}",
                &source[14..]
            )
        );
    }

    #[test]
    fn property_does_not_use_logbook_as_properties_drawer() {
        let source = "* TODO Task\n:LOGBOOK:\nentry\n:END:\n";
        let result = apply(source, |snapshot, range| {
            property_edit(snapshot, range, "OWNER", Some("me")).unwrap()
        });
        assert_eq!(
            result,
            "* TODO Task\n:PROPERTIES:\n:OWNER: me\n:END:\n:LOGBOOK:\nentry\n:END:\n"
        );
    }

    #[test]
    fn property_update_reuses_drawer_after_planning() {
        let source = "* TODO Task\nSCHEDULED: <2026-09-07>\n:PROPERTIES:\n:OWNER: old\n:END:\n";
        let result = apply(source, |snapshot, range| {
            property_edit(snapshot, range, "OWNER", Some("new")).unwrap()
        });
        assert_eq!(result, source.replace(":OWNER: old", ":OWNER: new"));
    }

    #[test]
    fn writes_after_unterminated_heading_add_line_break() {
        assert_eq!(
            apply("* TODO Task", |s, r| property_edit(s, r, "ID", Some("id"))
                .unwrap()),
            "* TODO Task\n:PROPERTIES:\n:ID: id\n:END:\n"
        );
    }

    #[test]
    fn timestamp_serialization_preserves_ranges_repeaters_and_warnings() {
        for timestamp in [
            "<2026-09-07 10:00-11:30 ++1w -2d>",
            "<2026-09-07 20:00 .+1m>--<2026-09-08 09:00>",
        ] {
            let source = format!("* TODO Task\n{timestamp}\n");
            let snapshot =
                crate::document::DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
            let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
            assert_eq!(
                format_timestamp(&analysis.headings[0].timestamps[0]),
                timestamp
            );
        }
    }
}
