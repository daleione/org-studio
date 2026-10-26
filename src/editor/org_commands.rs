use std::{
    hash::{Hash, Hasher},
    ops::Range,
    path::Path,
};

mod batch;
mod edit;
mod formula;
mod typing;
pub(super) use batch::resize_table;
pub(crate) use batch::{TableAlignmentPlan, plan_table_alignment};
pub(super) use edit::EditableTable;
pub(super) use formula::recalculate as recalculate_table;
pub(super) use formula::recalculate_buffer_tables;
pub(super) use formula::recalculate_iteratively as iterate_table;
pub(super) use formula::recalculate_marked_row_on_navigation;
pub(super) use typing::edit_table_field;

use crate::{
    document::{
        ByteOffset, ByteRange, DocumentFormat, DocumentSnapshot, HeadingIndex, LineIndex,
        TextSnapshot, table as source_table,
    },
    org_syntax::{BlockKind, parse},
};

pub(super) type LineCycle = fn(&str) -> Option<(Range<usize>, &'static str)>;
pub(super) const MIN_RESIZED_COLUMN_WIDTH: usize = 3;

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
                let previous = index.saturating_sub(1);
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
    let aligned = source_rows_aligned(&rows, columns, format);
    let widths = formatting_widths(&rows, &parsed, columns, format);
    let numeric_columns =
        (format == DocumentFormat::Org).then(|| org_numeric_columns(&parsed, columns));
    let mut replacement = String::new();
    let mut caret_local = None;
    for (row_index, row) in parsed.iter().enumerate() {
        let line = start + row_index as u64;
        let replacement_start = replacement.len();
        let caret_position = (row_index == target.0).then_some((target.1, target_character));
        let (formatted, mut caret) = if aligned {
            let text = rows[row_index].clone();
            let caret = caret_position
                .map(|(column, character)| cell_offset(&text, column, character, format));
            (text, caret)
        } else {
            aligned_row(
                row,
                &widths,
                caret_position,
                format,
                numeric_columns.as_deref(),
            )
        };
        if navigation != TableNavigation::Stay && row_index == target.0 {
            caret = Some(cell_input_offset(&formatted, target.1, format));
        }
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
        let (formatted, _) = aligned_row(&blank, &widths, None, format, numeric_columns.as_deref());
        replacement.push_str(&formatted);
        caret_local = Some(replacement_start + cell_input_offset(&formatted, target.1, format));
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

fn minimum_table_widths(rows: &[ParsedRow], columns: usize, format: DocumentFormat) -> Vec<usize> {
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
        }
    }
    widths
}

fn table_widths(rows: &[ParsedRow], columns: usize, format: DocumentFormat) -> Vec<usize> {
    let mut widths = minimum_table_widths(rows, columns, format);
    for row in rows.iter().filter(|row| !row.separator) {
        for (index, cell) in row.cells.iter().enumerate() {
            widths[index] = widths[index].max(source_table::text_width(cell, format));
        }
    }
    widths
}

/// Compare display columns, not byte offsets: Chinese text and escaped/literal pipes
/// must not make an otherwise aligned table look misaligned.
fn source_rows_aligned(rows: &[String], columns: usize, format: DocumentFormat) -> bool {
    let positions = |row: &str| {
        let mut start = 0;
        let mut position = 0;
        source_table::delimiter_offsets(row, format)
            .into_iter()
            .map(|offset| {
                position += source_table::text_width(&row[start..offset], format);
                start = offset + 1;
                let current = position;
                position += 1;
                current
            })
            .collect::<Vec<_>>()
    };
    let Some(first) = rows.first().map(|row| positions(row)) else {
        return false;
    };
    first.len() == columns + 1 && rows.iter().all(|row| positions(row) == first)
}

/// Recover serialized widths when all rows match the formatter's source shape.
/// This validates the widths; it does not establish an intent to wrap cells.
fn source_table_widths(
    source: &[String],
    rows: &[ParsedRow],
    columns: usize,
    format: DocumentFormat,
) -> Option<Vec<usize>> {
    let index = rows.iter().position(|row| row.separator).unwrap_or(0);
    let text = source.get(index)?;
    let widths = source_row_widths(text, columns, format)?;
    let natural = table_widths(rows, columns, format);
    // Short, unfinished fields must not be mistaken for a column resize.
    if widths
        .iter()
        .zip(&natural)
        .any(|(width, natural)| *width < MIN_RESIZED_COLUMN_WIDTH && width < natural)
    {
        return None;
    }
    if !source_rows_aligned(source, columns, format) {
        let canonical = source.iter().zip(rows).all(|(text, row)| {
            if row.cells.len() != columns {
                return false;
            }
            let cells = source_table::parse_line(text, format).cells;
            let alignment = cells
                .iter()
                .map(|cell| cell.align_right)
                .collect::<Vec<_>>();
            aligned_row(row, &widths, None, format, Some(&alignment)).0 == *text
        });
        if !canonical {
            return None;
        }
    }
    Some(widths)
}

fn source_row_widths(text: &str, columns: usize, format: DocumentFormat) -> Option<Vec<usize>> {
    let row = source_table::parse_line(text, format);
    if row.cells.len() != columns {
        return None;
    }
    row.cells
        .iter()
        .map(|cell| source_table::text_width(&text[cell.raw_range.clone()], format).checked_sub(2))
        .collect()
}

fn formatting_widths(
    source: &[String],
    rows: &[ParsedRow],
    columns: usize,
    format: DocumentFormat,
) -> Vec<usize> {
    let mut widths = table_widths(rows, columns, format);
    let authored = rows
        .iter()
        .position(|row| row.separator)
        .and_then(|index| source_row_widths(&source[index], columns, format))
        .or_else(|| source_table_widths(source, rows, columns, format));
    if let Some(authored) = authored {
        for (width, authored) in widths.iter_mut().zip(authored) {
            *width = (*width).max(authored);
        }
    }
    widths
}

fn cell_offset(text: &str, column: usize, character: usize, format: DocumentFormat) -> usize {
    source_table::parse_line(text, format)
        .cells
        .get(column)
        .map_or(text.len(), |cell| {
            cell.text_range.start
                + text[cell.text_range.clone()]
                    .char_indices()
                    .nth(character)
                    .map_or(cell.text_range.len(), |(byte, _)| byte)
        })
}

fn cell_input_offset(text: &str, column: usize, format: DocumentFormat) -> usize {
    source_table::parse_line(text, format)
        .cells
        .get(column)
        .map_or(text.len(), |cell| {
            cell.raw_range.start + usize::from(text[cell.raw_range.clone()].starts_with(' '))
        })
}

fn aligned_row(
    row: &ParsedRow,
    widths: &[usize],
    caret_position: Option<(usize, usize)>,
    format: DocumentFormat,
    align_right: Option<&[bool]>,
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
            let padding = width.saturating_sub(source_table::text_width(cell, format));
            let right = align_right.is_some_and(|columns| columns[index]);
            if right {
                output.push_str(&" ".repeat(padding));
            }
            if let Some((_, character)) = caret_position.filter(|(column, _)| *column == index) {
                let byte = cell
                    .char_indices()
                    .nth(character)
                    .map_or(cell.len(), |(offset, _)| offset);
                caret = Some(output.len() + byte);
            }
            output.push_str(cell);
            if !right {
                output.push_str(&" ".repeat(padding));
            }
            output.push_str(" |");
        }
    }
    (output, caret)
}

fn org_numeric_columns(rows: &[ParsedRow], columns: usize) -> Vec<bool> {
    (0..columns)
        .map(|column| {
            let (numbers, nonempty) = rows
                .iter()
                .filter(|row| !row.separator)
                .filter_map(|row| row.cells.get(column))
                .filter(|cell| !cell.is_empty())
                .fold((0, 0), |(numbers, nonempty), cell| {
                    (numbers + usize::from(org_table_number(cell)), nonempty + 1)
                });
            numbers * 2 >= nonempty
        })
        .collect()
}

/// The permissive forms accepted by Org's default `org-table-number-regexp`.
fn org_table_number(text: &str) -> bool {
    if text.eq_ignore_ascii_case("nan")
        || text
            .strip_prefix(['+', '-', 'u', 'U'])
            .unwrap_or(text)
            .eq_ignore_ascii_case("inf")
    {
        return true;
    }
    let text = text.strip_prefix(['<', '>']).unwrap_or(text);
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    if let Some(hex) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
        && !hex.is_empty()
        && hex.bytes().all(|ch| ch.is_ascii_hexdigit() || ch == b'.')
    {
        return true;
    }
    if let Some((radix, digits)) = unsigned.split_once('#')
        && !radix.is_empty()
        && radix.bytes().all(|ch| ch.is_ascii_digit())
        && !digits.is_empty()
        && digits
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'.')
    {
        return true;
    }
    let Some(digit) = text.bytes().position(|ch| ch.is_ascii_digit()) else {
        return false;
    };
    text[..digit]
        .bytes()
        .all(|ch| matches!(ch, b'-' | b'+' | b'^' | b'.'))
        && text[digit..].bytes().all(|ch| {
            matches!(
                ch,
                b'0'..=b'9'
                    | b'-'
                    | b'+'
                    | b'^'
                    | b'.'
                    | b'e'
                    | b'E'
                    | b'd'
                    | b'D'
                    | b'x'
                    | b'X'
                    | b'('
                    | b')'
                    | b'%'
                    | b':'
            )
        })
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

pub(super) struct TableColumns {
    pub(super) format: DocumentFormat,
    pub(super) widths: Vec<usize>,
    pub(super) minimum_widths: Vec<usize>,
    pub(super) source_widths: Option<Vec<usize>>,
    pub(super) aligned: bool,
    pub(super) source_range: ByteRange,
    pub(super) source_fingerprint: u64,
}

pub(super) fn table_columns(
    snapshot: &DocumentSnapshot,
    start: u64,
    format: DocumentFormat,
) -> Option<TableColumns> {
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
        .map(|row| parse_source_row(row, format))
        .collect::<Vec<_>>();
    let columns = rows.iter().map(|row| row.cells.len()).max().unwrap_or(0);
    if columns == 0 {
        return None;
    }
    let widths = table_widths(&rows, columns, format);
    let minimum_widths = minimum_table_widths(&rows, columns, format);
    let aligned = source_rows_aligned(&source_rows, columns, format);
    let source_widths = source_table_widths(&source_rows, &rows, columns, format);
    let source_range = ByteRange {
        start: snapshot.line_content_range(LineIndex(start)).ok()?.start,
        end: snapshot.line_content_range(LineIndex(line - 1)).ok()?.end,
    };
    let mut fingerprint = std::hash::DefaultHasher::new();
    source_rows.hash(&mut fingerprint);
    Some(TableColumns {
        format,
        widths,
        minimum_widths,
        source_widths,
        aligned,
        source_range,
        source_fingerprint: fingerprint.finish(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn org_numeric_alignment_recognizes_default_org_number_forms() {
        for value in [
            "12",
            "12.2",
            "2.4e-08",
            "2x10^12",
            "4.034+-0.02",
            "2.7(10)",
            ">3.5",
            "0xAF",
            "16#FF",
            "nan",
            "uINF",
            "12:30",
        ] {
            assert!(org_table_number(value), "{value}");
        }
        for value in ["", "item12", "v1.2", "1 2", "1,234", "NULL"] {
            assert!(!org_table_number(value), "{value}");
        }
    }

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
        let columns = table_columns(&aligned_snapshot, 0, DocumentFormat::Markdown).unwrap();
        assert!(columns.aligned);
        assert_eq!(columns.widths, vec![6, 5]);
    }

    #[test]
    fn org_alignment_sizes_links_by_their_labels_and_preserves_targets() {
        let source = "| ID | Label | Link |\n|----+-------+------|\n| A | note | [[https://example.net/a/long/path][Help]] |\n| B | next | [[file:appendix.org][目录]] |\n";
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let context =
            EditorCommandContext::at(Path::new("links.org"), &snapshot, ByteOffset(2)).unwrap();
        let aligned = align_table(&snapshot, &context, "\n", TableNavigation::NextCell).unwrap();
        assert!(
            aligned
                .replacement
                .contains("[[https://example.net/a/long/path][Help]]")
        );
        assert!(aligned.replacement.contains("[[file:appendix.org][目录]]"));
        let snapshot = DocumentSnapshot::from_utf8(aligned.replacement.into_bytes()).unwrap();
        let columns = table_columns(&snapshot, 0, DocumentFormat::Org).unwrap();
        assert!(columns.aligned);
        assert_eq!(columns.widths, [2, 5, 4]);
        assert_eq!(columns.source_widths.unwrap(), [2, 5, 4]);
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
    fn todo_cycles_are_explicit_line_edits() {
        assert_eq!(cycle_todo("* Heading"), Some((2..2, "TODO ")));
        assert_eq!(cycle_todo("* TODO Heading"), Some((2..6, "DONE")));
        assert_eq!(cycle_todo("* DONE Heading"), Some((2..7, "")));
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
