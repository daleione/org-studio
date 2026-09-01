use std::ops::Range;

use crate::document::{ByteRange, DocumentSnapshot, LineIndex, TextEdit, TextSnapshot};

use super::list::{CheckboxState, parse_list_line};

/// Returns the exact checkbox token edit for one Org line.
///
/// This is the single checkbox state machine used by both Editor commands and Reading actions.
/// The caller remains responsible for revision mapping and committing the returned edit through
/// `DocumentCommand`.
#[cfg(test)]
pub(crate) fn cycle_checkbox(text: &str) -> Option<(Range<usize>, &'static str)> {
    let (range, current) = checkbox_token(text)?;
    Some((range, CheckboxState::from_token(current)?.toggled().token()))
}

pub(crate) fn checkbox_token(text: &str) -> Option<(Range<usize>, &str)> {
    super::list::checkbox_token(text)
}

/// Builds one atomic checkbox transaction, including affected parent states and statistics
/// cookies. Only the containing list is scanned; the rest of a large document is never copied.
pub(crate) fn checkbox_transaction(
    snapshot: &DocumentSnapshot,
    target: ByteRange,
) -> Option<Vec<TextEdit>> {
    let target_line = snapshot.line_index_at(target.start).ok()?.0;
    let mut first_line = target_line;
    while first_line > 0 {
        if line_is_boundary(snapshot, first_line - 1)? {
            break;
        }
        first_line -= 1;
    }
    let mut lines = Vec::new();
    let mut line = first_line;
    while line < snapshot.len_lines() {
        if line > target_line && line_is_boundary(snapshot, line)? {
            break;
        }
        let text = line_text(snapshot, line)?;
        if let Some(item) = ListItem::parse(snapshot, line, &text) {
            lines.push(item);
        }
        line += 1;
    }
    let clicked = lines.iter().position(|line| {
        line.checkbox
            .as_ref()
            .is_some_and(|checkbox| checkbox.range == target)
    })?;

    let mut parents = vec![None; lines.len()];
    let mut stack = Vec::<usize>::new();
    for index in 0..lines.len() {
        while stack
            .last()
            .is_some_and(|parent| lines[*parent].indent >= lines[index].indent)
        {
            stack.pop();
        }
        parents[index] = stack.last().copied();
        stack.push(index);
    }

    let clicked_checkbox = lines[clicked].checkbox.as_ref()?;
    let replacement = clicked_checkbox.state.toggled().token();
    let mut states = lines
        .iter()
        .map(|line| line.checkbox.as_ref().map(|checkbox| checkbox.state))
        .collect::<Vec<_>>();
    states[clicked] = Some(CheckboxState::from_token(replacement)?);
    let mut edits = vec![TextEdit::new(clicked_checkbox.range, replacement)];

    let mut ancestor = parents[clicked];
    while let Some(index) = ancestor {
        let children = parents
            .iter()
            .enumerate()
            .filter(|(child, parent)| **parent == Some(index) && states[*child].is_some())
            .map(|(child, _)| states[child].expect("filtered checkbox child"))
            .collect::<Vec<_>>();
        if !children.is_empty() {
            let done = children
                .iter()
                .filter(|state| **state == CheckboxState::Checked)
                .count();
            if let Some(parent_checkbox) = &lines[index].checkbox {
                let next = if done == children.len() {
                    CheckboxState::Checked
                } else if children.iter().all(|state| *state == CheckboxState::Empty) {
                    CheckboxState::Empty
                } else {
                    CheckboxState::Partial
                };
                states[index] = Some(next);
                if parent_checkbox.state != next {
                    edits.push(TextEdit::new(parent_checkbox.range, next.token()));
                }
            }
            if let Some(cookie) = &lines[index].cookie {
                let replacement = cookie.replacement(done, children.len());
                if replacement != cookie.token {
                    edits.push(TextEdit::new(cookie.range, replacement));
                }
            }
        }
        ancestor = parents[index];
    }
    if let Some(edit) = heading_statistics_edit(snapshot, target_line, &edits) {
        edits.push(edit);
    }
    edits.sort_by_key(|edit| edit.range.start);
    Some(edits)
}

fn heading_statistics_edit(
    snapshot: &DocumentSnapshot,
    target_line: u64,
    pending_edits: &[TextEdit],
) -> Option<TextEdit> {
    let (heading_line, cookie) = owning_heading_cookie(snapshot, target_line)?;
    let mut done = 0usize;
    let mut total = 0usize;
    let mut stack = Vec::<(usize, bool)>::new();
    for line in heading_line + 1..snapshot.len_lines() {
        let text = line_text(snapshot, line)?;
        if heading_level_of(&text).is_some() {
            break;
        }
        let Some(parsed) = parse_list_line(&text) else {
            if text.trim().is_empty() || !(text.starts_with(' ') || text.starts_with('\t')) {
                stack.clear();
            }
            continue;
        };
        while stack
            .last()
            .is_some_and(|(indent, _)| *indent >= parsed.indent_columns)
        {
            stack.pop();
        }
        let has_checkbox_ancestor = stack.iter().any(|(_, checkbox)| *checkbox);
        if let Some(checkbox) = &parsed.checkbox
            && !has_checkbox_ancestor
        {
            let line_range = snapshot.line_content_range(LineIndex(line)).ok()?;
            let range = ByteRange::new(
                line_range.start.0 + checkbox.range.start as u64,
                line_range.start.0 + checkbox.range.end as u64,
            );
            let state = pending_edits
                .iter()
                .find(|edit| edit.range == range)
                .and_then(|edit| CheckboxState::from_token(&edit.replacement))
                .unwrap_or(checkbox.state);
            total += 1;
            done += usize::from(state == CheckboxState::Checked);
        }
        stack.push((parsed.indent_columns, parsed.checkbox.is_some()));
    }
    let replacement = cookie.replacement(done, total);
    (replacement != cookie.token).then(|| TextEdit::new(cookie.range, replacement))
}

fn owning_heading_cookie(
    snapshot: &DocumentSnapshot,
    target_line: u64,
) -> Option<(u64, StatisticsCookie)> {
    for line in (0..target_line).rev() {
        let text = line_text(snapshot, line)?;
        if heading_level_of(&text).is_none() {
            continue;
        }
        let line_range = snapshot.line_content_range(LineIndex(line)).ok()?;
        let cookie =
            statistics_cookie(&text).map(|(range, token, percentage)| StatisticsCookie {
                range: ByteRange::new(
                    line_range.start.0 + range.start as u64,
                    line_range.start.0 + range.end as u64,
                ),
                token: token.to_owned(),
                percentage,
            })?;
        return Some((line, cookie));
    }
    None
}

fn heading_level_of(text: &str) -> Option<usize> {
    let level = text.bytes().take_while(|byte| *byte == b'*').count();
    (level > 0
        && text
            .as_bytes()
            .get(level)
            .is_some_and(u8::is_ascii_whitespace))
    .then_some(level)
}

struct Checkbox {
    range: ByteRange,
    state: CheckboxState,
}

struct StatisticsCookie {
    range: ByteRange,
    token: String,
    percentage: bool,
}

impl StatisticsCookie {
    fn replacement(&self, done: usize, total: usize) -> String {
        if self.percentage {
            let percentage = (done * 100).checked_div(total).unwrap_or(0);
            format!("[{percentage}%]")
        } else {
            format!("[{done}/{total}]")
        }
    }
}

struct ListItem {
    indent: usize,
    checkbox: Option<Checkbox>,
    cookie: Option<StatisticsCookie>,
}

impl ListItem {
    fn parse(snapshot: &DocumentSnapshot, line: u64, text: &str) -> Option<Self> {
        let parsed = parse_list_line(text)?;
        let line_range = snapshot.line_content_range(LineIndex(line)).ok()?;
        let checkbox = parsed.checkbox.map(|checkbox| Checkbox {
            range: ByteRange::new(
                line_range.start.0 + checkbox.range.start as u64,
                line_range.start.0 + checkbox.range.end as u64,
            ),
            state: checkbox.state,
        });
        let cookie = statistics_cookie(text).map(|(range, token, percentage)| StatisticsCookie {
            range: ByteRange::new(
                line_range.start.0 + range.start as u64,
                line_range.start.0 + range.end as u64,
            ),
            token: token.to_owned(),
            percentage,
        });
        Some(Self {
            indent: parsed.indent_columns,
            checkbox,
            cookie,
        })
    }
}

fn line_text(snapshot: &DocumentSnapshot, line: u64) -> Option<String> {
    let range = snapshot.line_content_range(LineIndex(line)).ok()?;
    Some(snapshot.copy_range(range))
}

fn line_is_boundary(snapshot: &DocumentSnapshot, line: u64) -> Option<bool> {
    const PREFIX_BYTES: u64 = 256;
    let range = snapshot.line_content_range(LineIndex(line)).ok()?;
    let mut end = range.end.0.min(range.start.0 + PREFIX_BYTES);
    while end > range.start.0 && !snapshot.is_char_boundary(crate::document::ByteOffset(end)) {
        end -= 1;
    }
    Some(list_boundary(
        &snapshot.copy_range(ByteRange::new(range.start.0, end)),
    ))
}

fn list_boundary(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty()
        || (text.starts_with('*') && text.as_bytes().get(1).is_some_and(u8::is_ascii_whitespace))
    {
        return true;
    }
    let indent = text.len() - text.trim_start_matches([' ', '\t']).len();
    indent == 0 && parse_list_line(text).is_none()
}

fn statistics_cookie(text: &str) -> Option<(Range<usize>, &str, bool)> {
    let mut offset = 0;
    while let Some(start) = text[offset..].find('[').map(|start| offset + start) {
        let end = text[start + 1..].find(']').map(|end| start + end + 2)?;
        let token = &text[start..end];
        let inner = &token[1..token.len() - 1];
        let percentage = inner == "%"
            || inner.strip_suffix('%').is_some_and(|number| {
                !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
            });
        let fraction = inner.split_once('/').is_some_and(|(done, total)| {
            (done.is_empty() || done.bytes().all(|byte| byte.is_ascii_digit()))
                && (total.is_empty() || total.bytes().all(|byte| byte.is_ascii_digit()))
        });
        if percentage || fraction {
            return Some((start..end, token, percentage));
        }
        offset = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkbox_cycle_and_token_are_one_shared_spec() {
        assert_eq!(cycle_checkbox("- [ ] item"), Some((2..5, "[X]")));
        assert_eq!(cycle_checkbox("- [X] item"), Some((2..5, "[ ]")));
        assert_eq!(cycle_checkbox("- [-] item"), Some((2..5, "[ ]")));
        assert_eq!(checkbox_token("  - [x] item"), Some((4..7, "[x]")));
    }

    #[test]
    fn one_transaction_updates_checkbox_ancestors_and_statistics() {
        let buffer = crate::document::DocumentBuffer::from_utf8(
            b"- [ ] parent [0/2]\n  - [ ] one\n  - [X] two\n".to_vec(),
        )
        .unwrap();
        let snapshot = buffer.snapshot();
        let edits = checkbox_transaction(&snapshot, ByteRange::new(23, 26)).unwrap();
        assert_eq!(
            edits,
            vec![
                TextEdit::new(ByteRange::new(2, 5), "[X]"),
                TextEdit::new(ByteRange::new(13, 18), "[2/2]"),
                TextEdit::new(ByteRange::new(23, 26), "[X]"),
            ]
        );
    }

    #[test]
    fn checkbox_scan_only_probes_a_large_non_list_boundary() {
        let prefix = "界".repeat(350_000);
        let source = format!("{prefix}\n- [ ] task\n");
        let buffer = crate::document::DocumentBuffer::from_utf8(source.into_bytes()).unwrap();
        let snapshot = buffer.snapshot();
        let start = prefix.len() as u64 + 3;
        assert_eq!(
            checkbox_transaction(&snapshot, ByteRange::new(start, start + 3)).unwrap(),
            vec![TextEdit::new(ByteRange::new(start, start + 3), "[X]")]
        );
    }

    #[test]
    fn transaction_rejects_checkbox_text_outside_the_list_prefix() {
        let source = "- literal [ ] text\n";
        let buffer =
            crate::document::DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
        let start = source.find("[ ]").unwrap() as u64;
        assert!(
            checkbox_transaction(&buffer.snapshot(), ByteRange::new(start, start + 3)).is_none()
        );
    }

    #[test]
    fn transaction_updates_plain_parent_and_heading_placeholder_cookies() {
        let source = "* Tasks [/]\n- parent [/]\n  - [ ] one\n  - [X] two\n";
        let buffer =
            crate::document::DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
        let start = source.find("[ ]").unwrap() as u64;
        let edits =
            checkbox_transaction(&buffer.snapshot(), ByteRange::new(start, start + 3)).unwrap();
        assert!(
            edits
                .iter()
                .any(|edit| edit.replacement == "[2/2]" && edit.range.start.0 == 8)
        );
        assert!(
            edits
                .iter()
                .any(|edit| edit.replacement == "[2/2]" && edit.range.start.0 == 21)
        );
        assert!(
            edits
                .iter()
                .any(|edit| edit.replacement == "[X]" && edit.range.start.0 == start)
        );
    }
}
