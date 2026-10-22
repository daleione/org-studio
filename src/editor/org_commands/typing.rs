use std::ops::Range;

use crate::document::{DocumentFormat, EditOrigin, table};
use unicode_width::UnicodeWidthStr;

pub(in crate::editor) struct TableFieldEdit {
    pub range: Range<usize>,
    pub replacement: String,
    pub caret: usize,
}

/// Org's character commands consume or restore a trailing space in the source.
/// Formatting and movement between fields remain separate commands.
pub(in crate::editor) fn edit_table_field(
    line: &str,
    range: Range<usize>,
    text: &str,
    origin: EditOrigin,
    blank_field: bool,
) -> Option<TableFieldEdit> {
    let row = table::parse_line(line, DocumentFormat::Org);
    let field = row
        .cells
        .iter()
        .find(|cell| range.start >= cell.raw_range.start && range.end <= cell.raw_range.end)?;
    let field_end = field.raw_range.end;
    // An unfinished final field has no following separator to preserve.
    if line.as_bytes().get(field_end) != Some(&b'|') {
        return None;
    }
    match origin {
        EditOrigin::Typing if range.is_empty() && text.chars().count() == 1 => {
            if text.chars().next()?.is_control() {
                return None;
            }
            if blank_field && !row.separator {
                let width = UnicodeWidthStr::width(&line[field.raw_range.clone()]);
                let mut replacement = " ".repeat(width.max(1));
                replacement.insert_str(1, text);
                if width >= 3 {
                    replacement.pop();
                }
                return Some(TableFieldEdit {
                    range: field.raw_range.clone(),
                    replacement,
                    caret: field.raw_range.start + 1 + text.len(),
                });
            }
            if !line[range.start..field_end].ends_with("  ") {
                return None;
            }
            let end = field_end - 1;
            Some(TableFieldEdit {
                replacement: format!("{text}{}", &line[range.start..end - 1]),
                caret: range.start + text.len(),
                range: range.start..end,
            })
        }
        EditOrigin::DeleteBackward | EditOrigin::DeleteForward
            if !range.is_empty() && line[range.clone()].chars().count() == 1 =>
        {
            Some(TableFieldEdit {
                replacement: format!("{} ", &line[range.end..field_end]),
                caret: range.start,
                range: range.start..field_end,
            })
        }
        _ => None,
    }
}
