use std::sync::Arc;

use crate::document::{ByteRange, LineCursor, TextSnapshot};

use super::model::{OrgFileConfig, TodoSequence, TodoState, TodoStateKind};

const TODO_DIRECTIVES: [&str; 3] = ["TODO", "SEQ_TODO", "TYP_TODO"];

pub(crate) fn extract_file_config(snapshot: &dyn TextSnapshot) -> OrgFileConfig {
    let mut sequences = Vec::new();
    let mut file_tags = Vec::new();
    let mut category = None;
    let mut property_defaults = Vec::new();
    let mut archive_location = None;
    if let Some(mut lines) = LineCursor::within(snapshot, ByteRange::new(0, snapshot.len_bytes())) {
        while let Some(line) = lines.next_line() {
            if let Some(sequence) = parse_todo_directive(line.text.trim_start()) {
                sequences.push(sequence);
                continue;
            }
            if let Some((key, value)) = parse_keyword(line.text.trim_start()) {
                match key.as_str() {
                    "FILETAGS" => file_tags.extend(
                        value
                            .trim_matches(':')
                            .split(':')
                            .filter(|tag| !tag.is_empty())
                            .map(Arc::from),
                    ),
                    "CATEGORY" => category = non_empty(value),
                    "PROPERTY" => {
                        if let Some((name, value)) = value.split_once(char::is_whitespace) {
                            property_defaults.push((Arc::from(name), Arc::from(value.trim())));
                        }
                    }
                    "ARCHIVE" => archive_location = non_empty(value),
                    _ => {}
                }
            }
        }
    }
    if sequences.is_empty() {
        sequences.push(TodoSequence {
            states: vec![
                TodoState {
                    keyword: Arc::from("TODO"),
                    kind: TodoStateKind::Open,
                    fast_key: None,
                    log_spec: None,
                },
                TodoState {
                    keyword: Arc::from("DONE"),
                    kind: TodoStateKind::Done,
                    fast_key: None,
                    log_spec: None,
                },
            ]
            .into(),
        });
    }
    OrgFileConfig::new(
        sequences,
        file_tags,
        category,
        property_defaults,
        archive_location,
    )
}

fn parse_keyword(line: &str) -> Option<(String, &str)> {
    let (key, value) = line.strip_prefix("#+")?.split_once(':')?;
    Some((key.to_ascii_uppercase(), value.trim()))
}

fn non_empty(value: &str) -> Option<Arc<str>> {
    (!value.is_empty()).then(|| Arc::from(value))
}

pub(crate) fn parse_todo_directive(line: &str) -> Option<TodoSequence> {
    let line = line.trim_end_matches(['\r', '\n']);
    let body = line.strip_prefix("#+")?;
    let (key, value) = body.split_once(':')?;
    if !TODO_DIRECTIVES
        .iter()
        .any(|candidate| key.eq_ignore_ascii_case(candidate))
    {
        return None;
    }

    let tokens = value.split_whitespace().collect::<Vec<_>>();
    let separator = tokens.iter().position(|token| *token == "|");
    let last_state = tokens.iter().rposition(|token| *token != "|")?;
    let mut states = Vec::new();
    for (index, token) in tokens.into_iter().enumerate() {
        if token == "|" {
            continue;
        }
        let (keyword, settings) = token
            .split_once('(')
            .map_or((token, None), |(keyword, rest)| {
                (keyword, rest.strip_suffix(')'))
            });
        if keyword.is_empty() {
            continue;
        }
        let kind = if separator.map_or(index == last_state, |pipe| index > pipe) {
            TodoStateKind::Done
        } else {
            TodoStateKind::Open
        };
        let (fast_key, log_spec) = parse_state_settings(settings);
        states.push(TodoState {
            keyword: Arc::from(keyword),
            kind,
            fast_key,
            log_spec,
        });
    }
    (!states.is_empty()).then(|| TodoSequence {
        states: states.into(),
    })
}

fn parse_state_settings(settings: Option<&str>) -> (Option<char>, Option<Arc<str>>) {
    let Some(settings) = settings else {
        return (None, None);
    };
    let mut characters = settings.chars();
    let first = characters.next();
    let (fast_key, log_start) = match first {
        Some(character) if character.is_ascii_alphanumeric() => {
            (Some(character), character.len_utf8())
        }
        _ => (None, 0),
    };
    let log = settings[log_start..].trim();
    (fast_key, (!log.is_empty()).then(|| Arc::from(log)))
}
