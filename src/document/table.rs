use std::ops::Range;

use super::DocumentFormat;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SeparatorAlignment {
    pub(crate) left: bool,
    pub(crate) right: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParsedTableCell {
    pub(crate) raw_range: Range<usize>,
    pub(crate) text_range: Range<usize>,
    pub(crate) align_right: bool,
    pub(crate) separator_alignment: SeparatorAlignment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParsedTableLine {
    pub(crate) separator: bool,
    pub(crate) cells: Vec<ParsedTableCell>,
}

pub(crate) fn is_separator(source: &str) -> bool {
    let source = source.trim();
    source.contains('-')
        && source
            .chars()
            .all(|character| matches!(character, '|' | '+' | '-' | ':' | ' ' | '\t'))
}

pub(crate) fn is_table_row(source: &str, format: DocumentFormat) -> bool {
    let source = source.trim_start();
    source.starts_with('|')
        && source[1..].contains(['|', '+'])
        && (format != DocumentFormat::Markdown || source.trim_end().ends_with('|'))
}

pub(crate) fn parse_line(source: &str, format: DocumentFormat) -> ParsedTableLine {
    let trimmed_start = source.len() - source.trim_start().len();
    let trimmed_end = source.trim_end().len();
    let separator = is_separator(source);
    if trimmed_start >= trimmed_end {
        return ParsedTableLine {
            separator,
            cells: Vec::new(),
        };
    }

    let mut inner_start = trimmed_start;
    let mut inner_end = trimmed_end;
    if source[inner_start..inner_end].starts_with('|') {
        inner_start += 1;
    }
    if source[inner_start..inner_end].ends_with('|') {
        inner_end -= 1;
    }
    let inner = &source[inner_start..inner_end];
    let mut cells = Vec::new();
    let mut start = 0usize;
    for (end, delimiter) in cell_delimiters(inner, format, separator) {
        cells.push(parsed_cell(
            source,
            inner_start + start..inner_start + end,
            separator,
        ));
        start = end + delimiter.len_utf8();
    }
    cells.push(parsed_cell(
        source,
        inner_start + start..inner_end,
        separator,
    ));
    ParsedTableLine { separator, cells }
}

pub(crate) fn delimiter_offsets(source: &str, format: DocumentFormat) -> Vec<usize> {
    cell_delimiters(source, format, is_separator(source))
        .into_iter()
        .map(|(offset, _)| offset)
        .collect()
}

fn parsed_cell(source: &str, raw_range: Range<usize>, separator: bool) -> ParsedTableCell {
    let raw = &source[raw_range.clone()];
    let marker = raw.trim();
    if separator {
        return ParsedTableCell {
            text_range: raw_range.start..raw_range.start,
            raw_range,
            align_right: false,
            separator_alignment: SeparatorAlignment {
                left: marker.starts_with(':'),
                right: marker.ends_with(':'),
            },
        };
    }
    let leading = raw.len() - raw.trim_start().len();
    let trailing = raw.len() - raw.trim_end().len();
    let content_start = raw_range.start + leading;
    let content_end = raw_range.end - trailing;
    // A cell whose content is entirely whitespace (for example a freshly
    // appended empty table row) trims down to nothing, which would produce an
    // inverted range like `9..1`. Callers slice and clamp on this range, so
    // keep it well-formed: an empty range at the start of the content area.
    let text_range = if content_start <= content_end {
        content_start..content_end
    } else {
        content_start..content_start
    };
    ParsedTableCell {
        text_range,
        raw_range,
        align_right: leading > trailing,
        separator_alignment: SeparatorAlignment::default(),
    }
}

fn cell_delimiters(source: &str, format: DocumentFormat, separator: bool) -> Vec<(usize, char)> {
    if separator {
        return source
            .char_indices()
            .filter(|(_, character)| matches!(character, '+' | '|'))
            .collect();
    }
    let mut delimiters = Vec::new();
    let mut cursor = 0usize;
    while cursor < source.len() {
        let character = source[cursor..]
            .chars()
            .next()
            .expect("cursor stays on a character boundary");
        if character == '\\' {
            cursor += character.len_utf8();
            if cursor < source.len() {
                cursor += source[cursor..]
                    .chars()
                    .next()
                    .expect("escaped character exists")
                    .len_utf8();
            }
            continue;
        }
        let span_end = match format {
            DocumentFormat::Markdown if character == '`' => code_span_end(source, cursor, '`'),
            DocumentFormat::Org if matches!(character, '=' | '~') => {
                org_literal_span_end(source, cursor, character)
            }
            _ => None,
        };
        if let Some(end) = span_end {
            cursor = end;
            continue;
        }
        if character == '|' {
            delimiters.push((cursor, character));
        }
        cursor += character.len_utf8();
    }
    delimiters
}

fn code_span_end(source: &str, start: usize, marker: char) -> Option<usize> {
    let marker = marker as u8;
    let count = source[start..]
        .bytes()
        .take_while(|byte| *byte == marker)
        .count();
    let mut cursor = start + count;
    while cursor < source.len() {
        let run = source[cursor..]
            .bytes()
            .take_while(|byte| *byte == marker)
            .count();
        if run == count {
            return Some(cursor + run);
        }
        cursor += if run > 0 {
            run
        } else {
            source[cursor..].chars().next()?.len_utf8()
        };
    }
    None
}

fn org_literal_span_end(source: &str, start: usize, marker: char) -> Option<usize> {
    let opener_follows_boundary = start == 0
        || source[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace);
    let content = &source[start + marker.len_utf8()..];
    if !opener_follows_boundary || content.chars().next().is_none_or(char::is_whitespace) {
        return None;
    }
    content
        .find(marker)
        .map(|offset| start + marker.len_utf8() + offset + marker.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_and_escaped_pipes_stay_inside_cells() {
        let markdown = "| a | `x|y` and \\| |";
        let parsed = parse_line(markdown, DocumentFormat::Markdown);
        assert_eq!(parsed.cells.len(), 2);
        assert_eq!(
            &markdown[parsed.cells[1].text_range.clone()],
            "`x|y` and \\|"
        );

        let org = "| a | =x|y= and ~z|w~ |";
        let parsed = parse_line(org, DocumentFormat::Org);
        assert_eq!(parsed.cells.len(), 2);
        assert_eq!(&org[parsed.cells[1].text_range.clone()], "=x|y= and ~z|w~");
    }

    #[test]
    fn whitespace_only_cells_keep_well_formed_empty_text_ranges() {
        // A freshly appended empty table row contains only padding spaces, so
        // every cell trims to nothing. The resulting text range must stay
        // well-formed (`start <= end`); an inverted range like `9..1` made
        // table navigation panic in `usize::clamp` when tabbing past the last
        // cell.
        for row in ["|        |       |", "|  |  |"] {
            let parsed = parse_line(row, DocumentFormat::Markdown);
            assert!(!parsed.separator);
            assert_eq!(parsed.cells.len(), 2);
            for cell in &parsed.cells {
                assert!(
                    cell.text_range.start <= cell.text_range.end,
                    "inverted text range {cell:?} in {row:?}"
                );
                assert_eq!(&row[cell.text_range.clone()], "");
            }
        }
    }
}
