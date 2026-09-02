use std::{ops::Range, path::Path};

use unicode_width::UnicodeWidthStr;

use crate::{
    document::{
        ByteOffset, ByteRange, DocumentFormat, DocumentSnapshot, HeadingIndex, LineIndex,
        TextSnapshot,
    },
    org_syntax::{BlockKind, parse},
};

pub(super) type LineCycle = fn(&str) -> Option<(Range<usize>, &'static str)>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EditorCommandKind {
    TableCell { column: usize },
    Heading,
    List,
    Plain,
    NonOrg,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EditorCommandContext {
    pub(super) line: LineIndex,
    pub(super) line_range: ByteRange,
    pub(super) kind: EditorCommandKind,
}

impl EditorCommandContext {
    pub(super) fn at(path: &Path, snapshot: &DocumentSnapshot, offset: ByteOffset) -> Option<Self> {
        let headings =
            DocumentFormat::detect(path).map(|format| HeadingIndex::parse(format, snapshot));
        Self::at_with_headings(path, snapshot, offset, headings.as_ref())
    }

    pub(super) fn at_with_headings(
        path: &Path,
        snapshot: &DocumentSnapshot,
        offset: ByteOffset,
        headings: Option<&HeadingIndex>,
    ) -> Option<Self> {
        let line = snapshot.line_index_at(offset).ok()?;
        let line_range = snapshot.line_content_range(line).ok()?;
        let Some(format) = DocumentFormat::detect(path) else {
            return Some(Self {
                line,
                line_range,
                kind: EditorCommandKind::NonOrg,
            });
        };
        if format == DocumentFormat::Markdown {
            let is_heading = headings
                .and_then(|headings| headings.heading_at_line(line.0))
                .is_some_and(|heading| heading.start == line_range.start);
            return Some(Self {
                line,
                line_range,
                kind: if is_heading {
                    EditorCommandKind::Heading
                } else {
                    EditorCommandKind::Plain
                },
            });
        }
        let text = snapshot.copy_range(line_range);
        let trimmed = text.trim_start();
        let local = offset.0.saturating_sub(line_range.start.0) as usize;
        let arena = parse(snapshot);
        let structural_kind = arena
            .nodes()
            .iter()
            .find(|node| node.source.start == line_range.start)
            .map(|node| &node.kind);
        let kind = if matches!(structural_kind, Some(BlockKind::TableRow)) && is_table_row(trimmed)
        {
            EditorCommandKind::TableCell {
                column: text[..local.min(text.len())]
                    .bytes()
                    .filter(|byte| *byte == b'|')
                    .count()
                    .saturating_sub(1),
            }
        } else if matches!(structural_kind, Some(BlockKind::Heading { .. })) {
            EditorCommandKind::Heading
        } else if matches!(structural_kind, Some(BlockKind::ListItem)) {
            EditorCommandKind::List
        } else {
            EditorCommandKind::Plain
        };
        Some(Self {
            line,
            line_range,
            kind,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TableAlignment {
    pub(super) range: ByteRange,
    pub(super) replacement: String,
    pub(super) caret: ByteOffset,
}

pub(super) fn cycle_todo(text: &str) -> Option<(Range<usize>, &'static str)> {
    let trimmed = text.trim_start();
    let indent = text.len() - trimmed.len();
    let stars = trimmed.bytes().take_while(|byte| *byte == b'*').count();
    if stars == 0 || trimmed.as_bytes().get(stars) != Some(&b' ') {
        return None;
    }
    let start = indent + stars + 1;
    if text[start..].starts_with("TODO ") {
        Some((start..start + 4, "DONE"))
    } else if text[start..].starts_with("DONE ") {
        Some((start..start + 5, ""))
    } else {
        Some((start..start, "TODO "))
    }
}

pub(super) fn align_table(
    snapshot: &DocumentSnapshot,
    context: &EditorCommandContext,
    newline: &str,
    cell_delta: isize,
) -> Option<TableAlignment> {
    let EditorCommandKind::TableCell { column } = context.kind else {
        return None;
    };
    let mut start = context.line.0;
    while start > 0 && table_line(snapshot, start - 1).is_some() {
        start -= 1;
    }
    let mut end = context.line.0 + 1;
    while end < snapshot.len_lines() && table_line(snapshot, end).is_some() {
        end += 1;
    }
    let rows = (start..end)
        .map(|line| table_line(snapshot, line))
        .collect::<Option<Vec<_>>>()?;
    let parsed = rows.iter().map(|row| parse_row(row)).collect::<Vec<_>>();
    let columns = parsed.iter().map(|row| row.cells.len()).max().unwrap_or(0);
    if columns == 0 {
        return None;
    }
    let cells = parsed
        .iter()
        .enumerate()
        .filter(|(_, row)| !row.separator)
        .flat_map(|(row, parsed)| (0..parsed.cells.len()).map(move |column| (row, column)))
        .collect::<Vec<_>>();
    let current = cells
        .iter()
        .position(|cell| *cell == ((context.line.0 - start) as usize, column))
        .unwrap_or(0);
    let append_row = cell_delta > 0 && current + 1 == cells.len();
    let target = if append_row {
        (parsed.len(), 0)
    } else if cells.is_empty() {
        (0, 0)
    } else {
        cells[(current as isize + cell_delta).rem_euclid(cells.len() as isize) as usize]
    };
    let mut widths = vec![1usize; columns];
    for row in &parsed {
        if row.separator {
            continue;
        }
        for (index, cell) in row.cells.iter().enumerate() {
            widths[index] = widths[index].max(UnicodeWidthStr::width(cell.as_str()));
        }
    }
    let mut replacement = String::new();
    let mut caret_local = None;
    for (row_index, row) in parsed.iter().enumerate() {
        let line = start + row_index as u64;
        replacement.push_str(&row.indent);
        if row.separator {
            replacement.push('|');
            for (index, width) in widths.iter().enumerate() {
                replacement.push_str(&"-".repeat(width + 2));
                replacement.push(if index + 1 == widths.len() { '|' } else { '+' });
            }
        } else {
            replacement.push('|');
            for (index, width) in widths.iter().enumerate() {
                replacement.push(' ');
                if (row_index, index) == target {
                    caret_local = Some(replacement.len());
                }
                let cell = row.cells.get(index).map_or("", String::as_str);
                replacement.push_str(cell);
                replacement
                    .push_str(&" ".repeat(width.saturating_sub(UnicodeWidthStr::width(cell))));
                replacement.push_str(" |");
            }
        }
        if line + 1 < end {
            replacement.push_str(newline);
        }
    }
    if append_row {
        replacement.push_str(newline);
        replacement.push('|');
        for (index, width) in widths.iter().enumerate() {
            replacement.push(' ');
            if index == 0 {
                caret_local = Some(replacement.len());
            }
            replacement.push_str(&" ".repeat(*width));
            replacement.push_str(" |");
        }
    }
    let start_range = snapshot.line_content_range(LineIndex(start)).ok()?;
    let end_range = snapshot.line_content_range(LineIndex(end - 1)).ok()?;
    let range = ByteRange::new(start_range.start.0, end_range.end.0);
    Some(TableAlignment {
        range,
        caret: ByteOffset(range.start.0 + caret_local.unwrap_or(0) as u64),
        replacement,
    })
}

struct ParsedRow {
    indent: String,
    cells: Vec<String>,
    separator: bool,
}

fn parse_row(text: &str) -> ParsedRow {
    let trimmed = text.trim();
    let indent = text[..text.len() - text.trim_start().len()].to_owned();
    let separator = trimmed
        .bytes()
        .all(|byte| matches!(byte, b'|' | b'+' | b'-' | b':' | b' '));
    let cells = if separator {
        trimmed
            .trim_matches('|')
            .split('+')
            .map(|cell| cell.trim_matches([' ', '-', ':']).to_owned())
            .collect()
    } else {
        trimmed
            .trim_matches('|')
            .split('|')
            .map(|cell| cell.trim().replace('\t', "    "))
            .collect()
    };
    ParsedRow {
        indent,
        cells,
        separator,
    }
}

fn table_line(snapshot: &DocumentSnapshot, line: u64) -> Option<String> {
    let range = snapshot.line_content_range(LineIndex(line)).ok()?;
    let text = snapshot.copy_range(range);
    is_table_row(text.trim_start()).then_some(text)
}

fn is_table_row(text: &str) -> bool {
    text.starts_with('|') && text[1..].contains(['|', '+'])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn context_distinguishes_org_structures_without_ui_state() {
        let snapshot =
            DocumentSnapshot::from_utf8(b"* Head\n- item\n| a | b |\nplain\n".to_vec()).unwrap();
        let kinds = [0, 7, 14, 24].map(|offset| {
            EditorCommandContext::at(Path::new("a.org"), &snapshot, ByteOffset(offset))
                .unwrap()
                .kind
        });
        assert_eq!(kinds[0], EditorCommandKind::Heading);
        assert_eq!(kinds[1], EditorCommandKind::List);
        assert!(matches!(kinds[2], EditorCommandKind::TableCell { .. }));
        assert_eq!(kinds[3], EditorCommandKind::Plain);
    }

    #[test]
    fn context_does_not_treat_example_contents_as_org_structures() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"#+begin_example\n* literal heading\n| literal | table |\n- literal list\n#+end_example\n"
                .to_vec(),
        )
        .unwrap();

        for offset in [17, 35, 55] {
            assert_eq!(
                EditorCommandContext::at(Path::new("a.org"), &snapshot, ByteOffset(offset))
                    .unwrap()
                    .kind,
                EditorCommandKind::Plain
            );
        }
    }

    #[test]
    fn context_recognizes_markdown_headings_but_not_fenced_heading_text() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"# Heading\n```md\n# literal heading\n```\nparagraph\n".to_vec(),
        )
        .unwrap();

        assert_eq!(
            EditorCommandContext::at(Path::new("a.md"), &snapshot, ByteOffset(2))
                .unwrap()
                .kind,
            EditorCommandKind::Heading
        );
        assert_eq!(
            EditorCommandContext::at(Path::new("a.md"), &snapshot, ByteOffset(18))
                .unwrap()
                .kind,
            EditorCommandKind::Plain
        );
        assert_eq!(
            EditorCommandContext::at(Path::new("a.markdown"), &snapshot, ByteOffset(42))
                .unwrap()
                .kind,
            EditorCommandKind::Plain
        );
    }

    #[test]
    fn alignment_handles_wide_combining_emoji_tabs_and_separator_rows() {
        let text = "| 名 | e\u{301} | 🙂 |\n|---+---+---|\n| a\t | longer | x |\n";
        let snapshot = DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap();
        let context =
            EditorCommandContext::at(Path::new("a.org"), &snapshot, ByteOffset(2)).unwrap();
        let aligned = align_table(&snapshot, &context, "\n", 0).unwrap();
        assert!(
            aligned.replacement.contains("| 名 |"),
            "{}",
            aligned.replacement
        );
        assert!(aligned.replacement.contains("+"));
        assert!(aligned.replacement.contains("longer"));
        assert!(aligned.caret >= aligned.range.start);
    }

    #[test]
    fn todo_and_checkbox_cycles_are_explicit_line_edits() {
        assert_eq!(cycle_todo("* Heading"), Some((2..2, "TODO ")));
        assert_eq!(cycle_todo("* TODO Heading"), Some((2..6, "DONE")));
        assert_eq!(cycle_todo("* DONE Heading"), Some((2..7, "")));
        assert_eq!(
            crate::org_syntax::command::cycle_checkbox("- [ ] item"),
            Some((2..5, "[X]"))
        );
        assert_eq!(
            crate::org_syntax::command::cycle_checkbox("- [X] item"),
            Some((2..5, "[ ]"))
        );
    }

    #[test]
    fn malformed_table_like_text_falls_back_to_plain_source() {
        let snapshot = DocumentSnapshot::from_utf8(b"| incomplete\n".to_vec()).unwrap();
        let context =
            EditorCommandContext::at(Path::new("a.org"), &snapshot, ByteOffset(2)).unwrap();
        assert_eq!(context.kind, EditorCommandKind::Plain);
        assert!(align_table(&snapshot, &context, "\n", 0).is_none());
    }

    #[test]
    fn aligns_ten_thousand_rows_as_one_bounded_table_replacement() {
        let text = (0..10_000)
            .map(|index| format!("| row {index} | 值 |\n"))
            .collect::<String>();
        let snapshot = DocumentSnapshot::from_utf8(text.into_bytes()).unwrap();
        let context =
            EditorCommandContext::at(Path::new("large.org"), &snapshot, ByteOffset(2)).unwrap();
        let aligned = align_table(&snapshot, &context, "\n", 1).unwrap();
        assert_eq!(aligned.replacement.lines().count(), 10_000);
        assert_eq!(aligned.range.start, ByteOffset(0));
    }
}
