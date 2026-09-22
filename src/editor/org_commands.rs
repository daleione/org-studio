use std::{ops::Range, path::Path};

use unicode_width::UnicodeWidthStr;

mod batch;
pub(crate) use batch::{TableAlignmentPlan, plan_table_alignment};

use crate::{
    document::{
        ByteOffset, ByteRange, DocumentFormat, DocumentSnapshot, HeadingIndex, LineIndex,
        TextSnapshot, table as source_table,
    },
    org_syntax::{BlockKind, parse},
};

pub(super) type LineCycle = fn(&str) -> Option<(Range<usize>, &'static str)>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EditorCommandKind {
    TableCell { column: usize, character: usize },
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
    format: Option<DocumentFormat>,
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
                format: None,
            });
        };
        if format == DocumentFormat::Markdown {
            let text = snapshot.copy_range(line_range);
            let local = offset.0.saturating_sub(line_range.start.0) as usize;
            let (column, character) = table_position_at(&text, local, format);
            let markdown_table = crate::preview::markdown::parse_markdown(snapshot)
                .0
                .into_iter()
                .find(|block| block.source.start == line_range.start)
                .is_some_and(|block| {
                    matches!(block.kind, crate::preview::markdown::MarkdownKind::TableRow)
                });
            let is_heading = headings
                .and_then(|headings| headings.heading_at_line(line.0))
                .is_some_and(|heading| heading.start == line_range.start);
            return Some(Self {
                line,
                line_range,
                kind: if markdown_table {
                    EditorCommandKind::TableCell { column, character }
                } else if is_heading {
                    EditorCommandKind::Heading
                } else {
                    EditorCommandKind::Plain
                },
                format: Some(format),
            });
        }
        let text = snapshot.copy_range(line_range);
        let trimmed = text.trim_start();
        let local = offset.0.saturating_sub(line_range.start.0) as usize;
        let (column, character) = table_position_at(&text, local, format);
        let arena = parse(snapshot);
        let structural_kind = arena
            .nodes()
            .iter()
            .find(|node| node.source.start == line_range.start)
            .map(|node| &node.kind);
        let kind = if matches!(structural_kind, Some(BlockKind::TableRow))
            && source_table::is_table_row(trimmed, format)
        {
            EditorCommandKind::TableCell { column, character }
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
            format: Some(format),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TableAlignment {
    pub(super) range: ByteRange,
    pub(super) replacement: String,
    pub(super) caret: ByteOffset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TableNavigation {
    Stay,
    NextCell,
    PreviousCell,
    NextRow,
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
    navigation: TableNavigation,
) -> Option<TableAlignment> {
    let EditorCommandKind::TableCell { column, character } = context.kind else {
        return None;
    };
    let format = context.format?;
    let mut start = context.line.0;
    while start > 0 && table_line(snapshot, start - 1, format).is_some() {
        start -= 1;
    }
    let mut end = context.line.0 + 1;
    while end < snapshot.len_lines() && table_line(snapshot, end, format).is_some() {
        end += 1;
    }
    let rows = (start..end)
        .map(|line| table_line(snapshot, line, format))
        .collect::<Option<Vec<_>>>()?;
    let parsed = rows
        .iter()
        .map(|row| parse_row(row, format))
        .collect::<Vec<_>>();
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
    let current_row = (context.line.0 - start) as usize;
    let current = cells.iter().position(|cell| *cell == (current_row, column));
    let mut append_row = false;
    let target = match navigation {
        TableNavigation::Stay => (current_row, column.min(columns - 1)),
        TableNavigation::NextCell => match current {
            Some(index) if index + 1 < cells.len() => cells[index + 1],
            Some(_) => {
                append_row = true;
                (parsed.len(), 0)
            }
            None => cells
                .iter()
                .copied()
                .find(|(row, _)| *row > current_row)
                .or_else(|| cells.first().copied())
                .unwrap_or((0, 0)),
        },
        TableNavigation::PreviousCell => current
            .and_then(|index| {
                let previous = if index == 0 {
                    cells.len() - 1
                } else {
                    index - 1
                };
                cells.get(previous).copied()
            })
            .or_else(|| {
                cells
                    .iter()
                    .rev()
                    .copied()
                    .find(|(row, _)| *row < current_row)
            })
            .or_else(|| cells.last().copied())
            .unwrap_or((0, 0)),
        TableNavigation::NextRow => parsed
            .iter()
            .enumerate()
            .skip(current_row + 1)
            .find(|(_, row)| !row.separator)
            .map(|(row, _)| (row, column.min(columns - 1)))
            .unwrap_or_else(|| {
                append_row = true;
                (parsed.len(), column.min(columns - 1))
            }),
    };
    let target_character = match navigation {
        TableNavigation::Stay => character,
        _ => 0,
    };
    let widths = table_widths(&parsed, columns, format);
    let mut replacement = String::new();
    let mut caret_local = None;
    for (row_index, row) in parsed.iter().enumerate() {
        let line = start + row_index as u64;
        let replacement_start = replacement.len();
        let (formatted, caret) = aligned_row(
            row,
            &widths,
            (row_index == target.0).then_some((target.1, target_character)),
            format,
        );
        replacement.push_str(&formatted);
        if let Some(caret) = caret {
            caret_local = Some(replacement_start + caret);
        }
        if line + 1 < end {
            replacement.push_str(newline);
        }
    }
    if append_row {
        replacement.push_str(newline);
        let blank = ParsedRow {
            indent: parsed
                .get(current_row)
                .or_else(|| parsed.first())
                .map_or_else(String::new, |row| row.indent.clone()),
            cells: Vec::new(),
            separator: false,
            separator_alignments: Vec::new(),
        };
        let replacement_start = replacement.len();
        let (formatted, caret) = aligned_row(&blank, &widths, Some((target.1, 0)), format);
        replacement.push_str(&formatted);
        caret_local = caret.map(|caret| replacement_start + caret);
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
    separator_alignments: Vec<(bool, bool)>,
}

fn table_widths(rows: &[ParsedRow], columns: usize, format: DocumentFormat) -> Vec<usize> {
    let mut widths = vec![
        if format == DocumentFormat::Markdown {
            3
        } else {
            1
        };
        columns
    ];
    for row in rows {
        if row.separator && format == DocumentFormat::Markdown {
            for (index, (left, right)) in row.separator_alignments.iter().copied().enumerate() {
                widths[index] = widths[index].max(3 + usize::from(left) + usize::from(right));
            }
            continue;
        } else if row.separator {
            continue;
        }
        for (index, cell) in row.cells.iter().enumerate() {
            widths[index] = widths[index].max(UnicodeWidthStr::width(cell.as_str()));
        }
    }
    widths
}

fn aligned_row(
    row: &ParsedRow,
    widths: &[usize],
    caret_position: Option<(usize, usize)>,
    format: DocumentFormat,
) -> (String, Option<usize>) {
    let mut output = row.indent.clone();
    let mut caret = None;
    output.push('|');
    if row.separator {
        for (index, width) in widths.iter().enumerate() {
            if format == DocumentFormat::Markdown {
                let (left, right) = row
                    .separator_alignments
                    .get(index)
                    .copied()
                    .unwrap_or((false, false));
                output.push(' ');
                if caret_position.is_some_and(|(column, _)| column == index) {
                    caret = Some(output.len());
                }
                if left {
                    output.push(':');
                }
                output.push_str(
                    &"-".repeat(width.saturating_sub(usize::from(left) + usize::from(right))),
                );
                if right {
                    output.push(':');
                }
                output.push_str(" |");
            } else {
                if caret_position.is_some_and(|(column, _)| column == index) {
                    caret = Some(output.len());
                }
                output.push_str(&"-".repeat(width + 2));
                output.push(if index + 1 == widths.len() { '|' } else { '+' });
            }
        }
    } else {
        for (index, width) in widths.iter().enumerate() {
            output.push(' ');
            let cell = row.cells.get(index).map_or("", String::as_str);
            if let Some((_, character)) = caret_position.filter(|(column, _)| *column == index) {
                let byte = cell
                    .char_indices()
                    .nth(character)
                    .map_or(cell.len(), |(offset, _)| offset);
                caret = Some(output.len() + byte);
            }
            output.push_str(cell);
            output.push_str(&" ".repeat(width.saturating_sub(UnicodeWidthStr::width(cell))));
            output.push_str(" |");
        }
    }
    (output, caret)
}

fn parse_row(text: &str, format: DocumentFormat) -> ParsedRow {
    let mut row = parse_source_row(text, format);
    // Interactive table editing historically expands tabs; batch formatting preserves cell text.
    for cell in &mut row.cells {
        if cell.contains('\t') {
            *cell = cell.replace('\t', "    ");
        }
    }
    row
}

fn parse_source_row(text: &str, format: DocumentFormat) -> ParsedRow {
    let indent = text[..text.len() - text.trim_start().len()].to_owned();
    let parsed = source_table::parse_line(text, format);
    let cells = parsed
        .cells
        .iter()
        .map(|cell| text[cell.text_range.clone()].to_owned())
        .collect();
    let separator_alignments = parsed
        .cells
        .iter()
        .map(|cell| {
            (
                cell.separator_alignment.left,
                cell.separator_alignment.right,
            )
        })
        .collect();
    ParsedRow {
        indent,
        cells,
        separator: parsed.separator,
        separator_alignments,
    }
}

fn table_position_at(text: &str, local: usize, format: DocumentFormat) -> (usize, usize) {
    let parsed = source_table::parse_line(text, format);
    let local = local.min(text.len());
    parsed
        .cells
        .iter()
        .enumerate()
        .find(|(_, cell)| local <= cell.raw_range.end)
        .map(|(column, cell)| {
            let byte = local.clamp(cell.text_range.start, cell.text_range.end);
            let character = text[cell.text_range.start..byte].chars().count();
            (column, character)
        })
        .unwrap_or_else(|| (parsed.cells.len().saturating_sub(1), 0))
}

fn table_line(snapshot: &DocumentSnapshot, line: u64, format: DocumentFormat) -> Option<String> {
    let range = snapshot.line_content_range(LineIndex(line)).ok()?;
    let text = snapshot.copy_range(range);
    source_table::is_table_row(&text, format).then_some(text)
}

pub(super) fn table_start(
    snapshot: &DocumentSnapshot,
    line: u64,
    format: DocumentFormat,
) -> Option<u64> {
    table_line(snapshot, line, format)?;
    let mut start = line;
    while start > 0 && table_line(snapshot, start - 1, format).is_some() {
        start -= 1;
    }
    Some(start)
}

pub(super) fn aligned_table_column_widths(
    snapshot: &DocumentSnapshot,
    start: u64,
    format: DocumentFormat,
) -> Option<Vec<usize>> {
    let mut source_rows = Vec::new();
    let mut line = start;
    while line < snapshot.len_lines() {
        let Some(text) = table_line(snapshot, line, format) else {
            break;
        };
        source_rows.push(text);
        line += 1;
    }
    let rows = source_rows
        .iter()
        .map(|row| parse_row(row, format))
        .collect::<Vec<_>>();
    let columns = rows.iter().map(|row| row.cells.len()).max().unwrap_or(0);
    if columns == 0 {
        return None;
    }
    let widths = table_widths(&rows, columns, format);
    source_rows
        .iter()
        .zip(&rows)
        .all(|(source, row)| aligned_row(row, &widths, None, format).0 == *source)
        .then_some(widths)
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
    fn context_recognizes_markdown_tables_but_not_fenced_table_text() {
        let text = "```md\n| literal | table |\n```\n\n| Name | Value |\n| --- | --- |\n";
        let snapshot = DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap();
        let fenced = text.find("literal").unwrap() as u64;
        let table = text.find("Name").unwrap() as u64;

        assert_eq!(
            EditorCommandContext::at(Path::new("a.md"), &snapshot, ByteOffset(fenced))
                .unwrap()
                .kind,
            EditorCommandKind::Plain
        );
        assert!(matches!(
            EditorCommandContext::at(Path::new("a.md"), &snapshot, ByteOffset(table))
                .unwrap()
                .kind,
            EditorCommandKind::TableCell { column: 0, .. }
        ));
    }

    #[test]
    fn markdown_alignment_preserves_column_markers_and_inline_pipes() {
        let text = "| 名 | `a|b` |\n| :- | -: |\n| longer | x |\n";
        let snapshot = DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap();
        let context = EditorCommandContext::at(
            Path::new("a.md"),
            &snapshot,
            ByteOffset(text.find('名').unwrap() as u64),
        )
        .unwrap();
        let aligned = align_table(&snapshot, &context, "\n", TableNavigation::Stay).unwrap();

        assert_eq!(
            aligned.replacement,
            "| 名     | `a|b` |\n| :----- | ----: |\n| longer | x     |"
        );
        let aligned_snapshot =
            DocumentSnapshot::from_utf8(aligned.replacement.into_bytes()).unwrap();
        assert_eq!(
            aligned_table_column_widths(&aligned_snapshot, 0, DocumentFormat::Markdown),
            Some(vec![6, 5])
        );
    }

    #[test]
    fn org_alignment_preserves_pipes_inside_literal_spans() {
        let text = "| name | =a|b= and ~c|d~ |\n|--+--|\n";
        let snapshot = DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap();
        let context = EditorCommandContext::at(
            Path::new("a.org"),
            &snapshot,
            ByteOffset(text.find("name").unwrap() as u64),
        )
        .unwrap();
        let aligned = align_table(&snapshot, &context, "\n", TableNavigation::Stay).unwrap();

        assert!(aligned.replacement.contains("=a|b= and ~c|d~"));
        assert_eq!(
            parse_row(
                aligned.replacement.lines().next().unwrap(),
                DocumentFormat::Org
            )
            .cells
            .len(),
            2
        );
    }

    #[test]
    fn stay_navigation_keeps_the_caret_on_a_separator_row() {
        let text = "| alpha | beta |\n|-------+------|\n| gamma | x    |\n";
        let snapshot = DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap();
        let separator = text.find("-------").unwrap() as u64;
        let context =
            EditorCommandContext::at(Path::new("a.org"), &snapshot, ByteOffset(separator)).unwrap();
        let aligned = align_table(&snapshot, &context, "\n", TableNavigation::Stay).unwrap();
        let aligned_snapshot =
            DocumentSnapshot::from_utf8(aligned.replacement.as_bytes().to_vec()).unwrap();
        let local_caret = ByteOffset(aligned.caret.0 - aligned.range.start.0);

        assert_eq!(
            aligned_snapshot.line_index_at(local_caret).unwrap(),
            LineIndex(1)
        );
    }

    #[test]
    fn alignment_handles_wide_combining_emoji_tabs_and_separator_rows() {
        let text = "| 名 | e\u{301} | 🙂 |\n|---+---+---|\n| a\t | longer | x |\n";
        let snapshot = DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap();
        let context =
            EditorCommandContext::at(Path::new("a.org"), &snapshot, ByteOffset(2)).unwrap();
        let aligned = align_table(&snapshot, &context, "\n", TableNavigation::Stay).unwrap();
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
        assert!(align_table(&snapshot, &context, "\n", TableNavigation::Stay).is_none());
    }

    #[test]
    fn aligns_ten_thousand_rows_as_one_bounded_table_replacement() {
        let text = (0..10_000)
            .map(|index| format!("| row {index} | 值 |\n"))
            .collect::<String>();
        let snapshot = DocumentSnapshot::from_utf8(text.into_bytes()).unwrap();
        let context =
            EditorCommandContext::at(Path::new("large.org"), &snapshot, ByteOffset(2)).unwrap();
        let aligned = align_table(&snapshot, &context, "\n", TableNavigation::NextCell).unwrap();
        assert_eq!(aligned.replacement.lines().count(), 10_000);
        assert_eq!(aligned.range.start, ByteOffset(0));
    }

    #[test]
    fn return_moves_to_the_same_column_on_the_next_data_row() {
        let text = "| short | value |\n|-------+-------|\n| longer | next |\n";
        let snapshot = DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap();
        let context =
            EditorCommandContext::at(Path::new("a.org"), &snapshot, ByteOffset(12)).unwrap();
        let aligned = align_table(&snapshot, &context, "\n", TableNavigation::NextRow).unwrap();
        let caret = (aligned.caret.0 - aligned.range.start.0) as usize;
        assert!(aligned.replacement[caret..].starts_with("next"));
    }

    #[test]
    fn return_on_the_last_row_appends_an_aligned_row_and_keeps_the_column() {
        let text = "  | a | bbbb |\n  | c | d |";
        let snapshot = DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap();
        let context = EditorCommandContext::at(
            Path::new("a.org"),
            &snapshot,
            ByteOffset((text.len() - 3) as u64),
        )
        .unwrap();
        let aligned = align_table(&snapshot, &context, "\r\n", TableNavigation::NextRow).unwrap();
        let caret = (aligned.caret.0 - aligned.range.start.0) as usize;
        assert!(aligned.replacement.contains("\r\n  |"));
        assert_eq!(aligned.replacement.as_bytes()[caret - 1], b' ');
        assert!(aligned.replacement[caret..].starts_with("     |"));
    }
}
