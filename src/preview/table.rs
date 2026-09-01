use std::{collections::HashMap, ops::Range, sync::Arc};

use gpui::{AnyElement, Div, ScrollHandle, div, prelude::*, px, rgb};
use smallvec::SmallVec;
use unicode_width::UnicodeWidthStr;

use crate::{
    document::{ByteRange, TextSnapshot},
    org_syntax::{BlockArena, BlockId, BlockKind},
    theme::current_theme,
};

use super::{
    DocumentFormat,
    markdown::{MarkdownBlock, MarkdownKind},
    parse_document_inline,
    view::styled_inline_runs,
};

const CELL_WIDTH_PX: f32 = 8.45;
const PIPE_WIDTH_PX: f32 = 12.0;
const CELL_PADDING_PX: f32 = 18.0;
const CELL_CONTENT_INSET_PX: f32 = 4.0;
const MAX_COLUMN_WIDTH: usize = 64;
const PROJECTED_HORIZONTAL_INSET_PX: f32 = 4.0;
const MIN_COLUMN_CONTENT_CHARS: usize = 4;
const TABLE_FRAME_WIDTH_PX: f32 = 2.0;

#[derive(Clone, Debug)]
pub(super) struct TableRenderProjection {
    columns: Arc<[TableColumnSpec]>,
}

#[derive(Clone, Debug)]
pub(super) struct TableRowProjection {
    table: Arc<TableRenderProjection>,
    group_id: BlockId,
    cells: Arc<[TableCell]>,
    separator: bool,
    header: bool,
    first: bool,
}

impl TableRenderProjection {
    pub(super) fn columns(&self) -> &[TableColumnSpec] {
        &self.columns
    }

    pub(super) fn resolve(&self) -> ResolvedTable {
        let mut cursor = PIPE_WIDTH_PX;
        let mut separators = Vec::with_capacity(self.columns.len() + 1);
        separators.push(PIPE_WIDTH_PX * 0.5);
        let columns = self
            .columns
            .iter()
            .map(|column| {
                let width = column.width_px();
                let resolved = ResolvedTableColumn {
                    start_x: cursor,
                    end_x: cursor + width,
                    content_start_x: cursor + CELL_CONTENT_INSET_PX,
                    content_end_x: cursor + width - CELL_CONTENT_INSET_PX,
                };
                cursor += width + PIPE_WIDTH_PX;
                separators.push(cursor - PIPE_WIDTH_PX * 0.5);
                resolved
            })
            .collect::<Vec<_>>();
        ResolvedTable {
            width: cursor,
            columns: columns.into(),
            separators: separators.into(),
        }
    }

    pub(super) fn same_geometry(&self, other: &Self) -> bool {
        self.columns == other.columns
    }

    fn resolve_for_viewport(&self, available_width: f32, zoom: f32) -> ReadingTableLayout {
        let available_width = available_width.max(1.0);
        let zoom = zoom.max(0.01);
        let preferred = self
            .columns
            .iter()
            .map(|column| column.width_px() * zoom)
            .collect::<Vec<_>>();
        let minimum = self
            .columns
            .iter()
            .map(|column| {
                let minimum_chars = column.width_chars.clamp(1, MIN_COLUMN_CONTENT_CHARS);
                minimum_chars as f32 * CELL_WIDTH_PX * zoom + CELL_PADDING_PX * zoom
            })
            .collect::<Vec<_>>();
        let minimum_width = minimum.iter().sum::<f32>();
        let preferred_width = preferred.iter().sum::<f32>();

        if minimum_width > available_width {
            return ReadingTableLayout {
                column_widths: minimum.clone().into(),
                content_width: minimum_width,
                overflow: true,
            };
        }

        let mut column_widths = minimum.clone();
        let mut remaining = available_width - minimum_width;
        let preferred_deficit = (preferred_width - minimum_width).max(0.0);
        if preferred_deficit > 0.0 {
            let supplied = remaining.min(preferred_deficit);
            for ((width, preferred), minimum) in column_widths
                .iter_mut()
                .zip(preferred.iter())
                .zip(minimum.iter())
            {
                *width += supplied * (preferred - minimum) / preferred_deficit;
            }
            remaining -= supplied;
        }
        if remaining > 0.0 && !column_widths.is_empty() {
            for (width, preferred) in column_widths.iter_mut().zip(preferred.iter()) {
                *width += remaining * preferred / preferred_width.max(1.0);
            }
        }
        ReadingTableLayout {
            column_widths: column_widths.into(),
            content_width: available_width,
            overflow: false,
        }
    }
}

impl TableRowProjection {
    pub(super) fn table(&self) -> &TableRenderProjection {
        &self.table
    }

    pub(super) fn columns(&self) -> &[TableColumnSpec] {
        self.table.columns()
    }

    pub(super) fn group_id(&self) -> BlockId {
        self.group_id
    }

    pub(super) fn cells(&self) -> &[TableCell] {
        &self.cells
    }

    pub(super) fn is_separator(&self) -> bool {
        self.separator
    }

    pub(super) fn is_header(&self) -> bool {
        self.header
    }

    pub(super) fn is_first(&self) -> bool {
        self.first
    }

    #[cfg(test)]
    pub(super) fn resolve(&self) -> ResolvedTable {
        self.table.resolve()
    }

    pub(super) fn estimated_line_count(&self, source: &str, available_width: f32) -> usize {
        if self.separator || self.cells.is_empty() {
            return 1;
        }
        let layout = self
            .table
            .resolve_for_viewport((available_width - TABLE_FRAME_WIDTH_PX).max(1.0), 1.0);
        self.columns()
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let cell_width = layout.column_widths[index];
                let content_chars = ((cell_width - 24.0).max(CELL_WIDTH_PX) / CELL_WIDTH_PX)
                    .floor()
                    .max(1.0) as usize;
                let display_width = self
                    .cells
                    .get(index)
                    .map(|cell| UnicodeWidthStr::width(cell.text(source)))
                    .unwrap_or(0);
                display_width.div_ceil(content_chars).max(1)
            })
            .max()
            .unwrap_or(1)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Alignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TableCell {
    text_range: Range<usize>,
    align_right: bool,
}

impl TableCell {
    pub(super) fn text<'a>(&self, source: &'a str) -> &'a str {
        source.get(self.text_range.clone()).unwrap_or_default()
    }

    pub(super) fn align_right(&self) -> bool {
        self.align_right
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TableColumnSpec {
    width_chars: usize,
    alignment: Alignment,
}

impl TableColumnSpec {
    pub(super) fn alignment(self) -> Alignment {
        self.alignment
    }

    pub(super) fn width_px(self) -> f32 {
        self.width_chars as f32 * CELL_WIDTH_PX + CELL_PADDING_PX
    }
}

#[cfg(test)]
pub(in crate::preview) fn test_table_projection() -> TableRowProjection {
    TableRowProjection {
        table: Arc::new(TableRenderProjection {
            columns: Arc::from([
                TableColumnSpec {
                    width_chars: 5,
                    alignment: Alignment::Left,
                },
                TableColumnSpec {
                    width_chars: 8,
                    alignment: Alignment::Right,
                },
            ]),
        }),
        group_id: 0,
        cells: Arc::from([
            TableCell {
                text_range: 1..6,
                align_right: false,
            },
            TableCell {
                text_range: 7..9,
                align_right: true,
            },
        ]),
        separator: false,
        header: false,
        first: true,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ResolvedTableColumn {
    pub(super) start_x: f32,
    pub(super) end_x: f32,
    pub(super) content_start_x: f32,
    pub(super) content_end_x: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ResolvedTable {
    pub(super) width: f32,
    pub(super) columns: Arc<[ResolvedTableColumn]>,
    pub(super) separators: Arc<[f32]>,
}

#[derive(Clone, Debug, PartialEq)]
struct ReadingTableLayout {
    column_widths: Arc<[f32]>,
    content_width: f32,
    overflow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ProjectedTableColumn {
    pub(super) start_x: f32,
    pub(super) end_x: f32,
    pub(super) content_start_x: f32,
    pub(super) content_end_x: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ProjectedTable {
    pub(super) columns: SmallVec<[ProjectedTableColumn; 8]>,
    pub(super) separators: SmallVec<[f32; 9]>,
}

/// Transforms canonical table geometry into a bounded target viewport.
pub(super) fn project_table(
    projection: &TableRenderProjection,
    source_width: f32,
    target_width: f32,
) -> ProjectedTable {
    let resolved = projection.resolve();
    let drawable = (target_width - PROJECTED_HORIZONTAL_INSET_PX * 2.0).max(1.0);
    let scale = drawable / source_width.max(1.0);
    let project_x = |x: f32| {
        (PROJECTED_HORIZONTAL_INSET_PX + x * scale)
            .min(target_width - PROJECTED_HORIZONTAL_INSET_PX)
    };
    ProjectedTable {
        columns: resolved
            .columns
            .iter()
            .map(|column| ProjectedTableColumn {
                start_x: project_x(column.start_x),
                end_x: project_x(column.end_x),
                content_start_x: project_x(column.content_start_x),
                content_end_x: project_x(column.content_end_x),
            })
            .collect(),
        separators: resolved.separators.iter().copied().map(project_x).collect(),
    }
}

pub(super) fn build_table_styles(
    text: &dyn TextSnapshot,
    blocks: &BlockArena,
) -> HashMap<BlockId, TableRowProjection> {
    let nodes = blocks.nodes();
    let mut result = HashMap::new();
    let mut start = 0;
    while start < nodes.len() {
        if !matches!(nodes[start].kind, BlockKind::TableRow) {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < nodes.len() && matches!(nodes[end].kind, BlockKind::TableRow) {
            end += 1;
        }
        let parsed = (start..end)
            .map(|index| (index as BlockId, nodes[index].content))
            .collect::<Vec<_>>();
        build_table_group(text, DocumentFormat::Org, &parsed, &mut result);
        start = end;
    }
    result
}

pub(super) fn build_markdown_table_styles(
    text: &dyn TextSnapshot,
    blocks: &[MarkdownBlock],
) -> HashMap<BlockId, TableRowProjection> {
    let mut result = HashMap::new();
    let mut start = 0;
    while start < blocks.len() {
        if !matches!(blocks[start].kind, MarkdownKind::TableRow) {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < blocks.len() && matches!(blocks[end].kind, MarkdownKind::TableRow) {
            end += 1;
        }
        let group = (start..end)
            .map(|index| (index as BlockId, blocks[index].source))
            .collect::<Vec<_>>();
        build_table_group(text, DocumentFormat::Markdown, &group, &mut result);
        start = end;
    }
    result
}

fn build_table_group(
    text: &dyn TextSnapshot,
    format: DocumentFormat,
    rows: &[(BlockId, ByteRange)],
    result: &mut HashMap<BlockId, TableRowProjection>,
) {
    let mut widths = Vec::new();
    let parsed = rows
        .iter()
        .map(|(id, range)| {
            let source = text.copy_range(*range);
            let separator = is_separator(&source);
            let cells = parse_cells(&source, format);
            if !separator {
                widths.resize(widths.len().max(cells.len()), 1);
                for (index, cell) in cells.iter().enumerate() {
                    let display = parse_document_inline(format, cell.text(&source));
                    widths[index] = widths[index]
                        .max(UnicodeWidthStr::width(display.text.as_str()))
                        .min(MAX_COLUMN_WIDTH);
                }
            }
            (
                *id,
                separator,
                cells,
                separator.then(|| parse_separator_alignments(&source)),
            )
        })
        .collect::<Vec<_>>();
    let columns = widths.len().max(
        parsed
            .iter()
            .map(|(_, _, cells, _)| cells.len())
            .max()
            .unwrap_or(0),
    );
    widths.resize(columns, 1);
    let alignments = Arc::new(
        parsed
            .iter()
            .find_map(|(_, _, _, alignments)| alignments.clone())
            .unwrap_or_else(|| vec![Alignment::Left; columns]),
    );
    let table = Arc::new(TableRenderProjection {
        columns: widths
            .into_iter()
            .zip(alignments.iter().copied())
            .map(|(width_chars, alignment)| TableColumnSpec {
                width_chars,
                alignment,
            })
            .collect::<Arc<[_]>>(),
    });
    let separator_index = parsed.iter().position(|(_, separator, _, _)| *separator);
    let group_id = rows.first().map_or(0, |(id, _)| *id);
    for (position, (index, separator, cells, _)) in parsed.into_iter().enumerate() {
        result.insert(
            index,
            TableRowProjection {
                table: table.clone(),
                group_id,
                cells: cells.into(),
                separator,
                header: separator_index.is_some_and(|separator| position < separator),
                first: position == 0,
            },
        );
    }
}

pub(super) fn render_table_row(
    source: &str,
    projection: &TableRowProjection,
    format: DocumentFormat,
    available_width: f32,
    zoom: f32,
    row_id: usize,
    horizontal_scroll: Option<&ScrollHandle>,
) -> Div {
    let theme = current_theme();
    if projection.separator {
        return div().w_full().h(px(2.0)).bg(rgb(theme.border));
    }
    let layout = projection
        .table
        .resolve_for_viewport((available_width - TABLE_FRAME_WIDTH_PX).max(1.0), zoom);
    let mut elements: Vec<AnyElement> = Vec::with_capacity(projection.columns().len());
    for (index, column) in projection.columns().iter().enumerate() {
        let width_px = layout.column_widths[index];
        let cell = projection.cells.get(index);
        let display = cell
            .map(|cell| parse_document_inline(format, cell.text(source)))
            .unwrap_or_default();
        let align_right =
            column.alignment == Alignment::Right || cell.is_some_and(TableCell::align_right);
        let content = div()
            .w_full()
            .min_w_0()
            .flex_1()
            .whitespace_normal()
            .when(column.alignment == Alignment::Center, |element| {
                element.text_center()
            })
            .when(align_right, |element| element.text_right())
            .child(styled_inline_runs(
                display.text.into(),
                display.spans.into(),
            ));
        elements.push(
            div()
                .w(px(width_px))
                .min_w_0()
                .flex_none()
                .overflow_hidden()
                .whitespace_normal()
                .px(px(12.0 * zoom))
                .py(px(8.0 * zoom))
                .flex()
                .flex_col()
                .items_start()
                .when(index + 1 < projection.columns().len(), |element| {
                    element.border_r_1().border_color(rgb(theme.border))
                })
                .child(content)
                .into_any(),
        );
    }
    let table = div()
        .w(px(layout.content_width + TABLE_FRAME_WIDTH_PX))
        .flex_none()
        .flex()
        .items_stretch()
        .border_l_1()
        .border_r_1()
        .border_b_1()
        .when(projection.is_first(), |element| element.border_t_1())
        .border_color(rgb(theme.border))
        .when(projection.is_header(), |element| {
            element
                .bg(rgb(theme.background_alt))
                .font_weight(gpui::FontWeight::SEMIBOLD)
        })
        .text_size(px(14.0 * zoom))
        .line_height(px(21.0 * zoom))
        .text_color(rgb(theme.foreground))
        .children(elements);
    let scroller = div()
        .w_full()
        .id(("reading-table-row", row_id))
        .when(layout.overflow, |element| {
            element.overflow_x_scroll().restrict_scroll_to_axis()
        })
        .when_some(
            layout.overflow.then_some(horizontal_scroll).flatten(),
            |element, handle| element.track_scroll(handle),
        )
        .when(!layout.overflow, |element| element.overflow_hidden())
        .child(table);
    div().w_full().child(scroller)
}

fn is_separator(source: &str) -> bool {
    let source = source.trim();
    source.contains('-')
        && source
            .chars()
            .all(|character| matches!(character, '|' | '+' | '-' | ':' | ' ' | '\t'))
}

fn parse_cells(source: &str, format: DocumentFormat) -> Vec<TableCell> {
    let trimmed_start = source.len() - source.trim_start().len();
    let trimmed_end = source.trim_end().len();
    if trimmed_start >= trimmed_end {
        return Vec::new();
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
    let separator = is_separator(source);
    let mut cells = Vec::new();
    let mut start = 0usize;
    for (end, delimiter) in cell_delimiters(inner, format, separator) {
        let raw = &inner[start..end];
        cells.push(table_cell_from_range(inner_start + start, raw, separator));
        start = end + delimiter.len_utf8();
    }
    cells.push(table_cell_from_range(
        inner_start + start,
        &inner[start..],
        separator,
    ));
    cells
}

fn cell_delimiters(source: &str, format: DocumentFormat, separator: bool) -> Vec<(usize, char)> {
    if separator {
        return source
            .char_indices()
            .filter(|(_, ch)| matches!(ch, '+' | '|'))
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
            DocumentFormat::Markdown if character == '`' => markdown_code_span_end(source, cursor),
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

fn markdown_code_span_end(source: &str, start: usize) -> Option<usize> {
    let ticks = source[start..]
        .bytes()
        .take_while(|byte| *byte == b'`')
        .count();
    let mut cursor = start + ticks;
    while cursor < source.len() {
        let run = source[cursor..]
            .bytes()
            .take_while(|byte| *byte == b'`')
            .count();
        if run == ticks {
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

fn table_cell_from_range(start: usize, raw: &str, separator: bool) -> TableCell {
    if separator {
        return TableCell {
            text_range: start..start,
            align_right: false,
        };
    }
    let leading = raw.len() - raw.trim_start().len();
    let trailing = raw.len() - raw.trim_end().len();
    TableCell {
        text_range: start + leading..start + raw.len() - trailing,
        align_right: leading > trailing,
    }
}

fn parse_separator_alignments(source: &str) -> Vec<Alignment> {
    source
        .trim()
        .trim_matches('|')
        .split(['+', '|'])
        .map(
            |raw| match (raw.trim().starts_with(':'), raw.trim().ends_with(':')) {
                (true, true) => Alignment::Center,
                (false, true) => Alignment::Right,
                _ => Alignment::Left,
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{
        document::{DocumentSnapshot, TextSnapshot},
        preview::{DocumentFormat, markdown::parse_markdown, parse_document_inline},
    };

    use super::{
        Alignment, CELL_PADDING_PX, CELL_WIDTH_PX, PIPE_WIDTH_PX, build_markdown_table_styles,
        is_separator, parse_cells, parse_separator_alignments, test_table_projection,
    };

    #[test]
    fn preserves_per_cell_org_alignment() {
        let source = "| apple |       42 |    score |";
        let cells = parse_cells(source, DocumentFormat::Org);
        assert_eq!(cells[0].text(source), "apple");
        assert_eq!(cells[1].text(source), "42");
        assert_eq!(cells[2].text(source), "score");
        assert!(!cells[0].align_right);
        assert!(cells[1].align_right);
        assert!(cells[2].align_right);
    }

    #[test]
    fn cell_ranges_preserve_unicode_boundaries_without_copying_text() {
        let source = "  | 中文 | e\u{301} | 🦀 |  \n";
        let cells = parse_cells(source, DocumentFormat::Org);
        assert_eq!(cells[0].text(source), "中文");
        assert_eq!(cells[1].text(source), "e\u{301}");
        assert_eq!(cells[2].text(source), "🦀");
    }

    #[test]
    fn recognizes_standard_separator() {
        assert!(is_separator("|----------+-----------------------|"));
    }

    #[test]
    fn parses_markdown_separator_columns_and_alignment() {
        let alignments = parse_separator_alignments("| :--- | :---: | ---: |");
        assert_eq!(
            alignments,
            vec![Alignment::Left, Alignment::Center, Alignment::Right]
        );
    }

    #[test]
    fn inline_literal_pipes_do_not_split_reading_table_cells() {
        let markdown = "| 表格 | 显示 `|` 和分隔行，后面的内容不能丢失 |";
        let cells = parse_cells(markdown, DocumentFormat::Markdown);
        assert_eq!(cells.len(), 2);
        assert_eq!(
            cells[1].text(markdown),
            "显示 `|` 和分隔行，后面的内容不能丢失"
        );
        assert_eq!(
            parse_document_inline(DocumentFormat::Markdown, cells[1].text(markdown)).text,
            "显示 | 和分隔行，后面的内容不能丢失"
        );

        let org = "| 表格 | 显示 =|= 和分隔行，后面的内容不能丢失 |";
        let cells = parse_cells(org, DocumentFormat::Org);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[1].text(org), "显示 =|= 和分隔行，后面的内容不能丢失");
    }

    #[test]
    fn resolved_geometry_is_derived_from_the_same_columns_as_preview() {
        let projection = test_table_projection();
        let resolved = projection.resolve();
        let source = "|apple|42|";
        assert_eq!(projection.cells()[0].text(source), "apple");
        assert_eq!(projection.cells()[1].text(source), "42");
        assert_eq!(resolved.columns.len(), 2);
        assert_eq!(resolved.columns[0].start_x, PIPE_WIDTH_PX);
        assert_eq!(
            resolved.columns[0].end_x,
            PIPE_WIDTH_PX + 5.0 * CELL_WIDTH_PX + CELL_PADDING_PX
        );
        assert_eq!(
            resolved.width,
            PIPE_WIDTH_PX * 3.0 + (5.0 + 8.0) * CELL_WIDTH_PX + CELL_PADDING_PX * 2.0
        );
    }

    #[test]
    fn rows_share_one_table_projection_but_separate_groups_do_not() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"| name | value |\n| --- | ---: |\n| a | 42 |\n\n| x | y |\n| --- | --- |\n".to_vec(),
        )
        .expect("valid fixture");
        let (blocks, _) = parse_markdown(&snapshot);
        let rows = build_markdown_table_styles(&snapshot, &blocks);

        assert!(std::ptr::eq(rows[&0].table(), rows[&1].table()));
        assert!(std::ptr::eq(rows[&1].table(), rows[&2].table()));
        assert!(!std::ptr::eq(rows[&2].table(), rows[&4].table()));
    }

    #[test]
    fn reading_table_marks_header_and_wraps_only_when_width_requires_it() {
        let long = "semantic reading content ".repeat(12);
        let source = format!("| Name | Description |\n| :--- | :--- |\n| Ada | {long} |\n");
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let (blocks, _) = parse_markdown(&snapshot);
        let rows = build_markdown_table_styles(&snapshot, &blocks);

        assert!(rows[&0].is_header());
        assert!(rows[&1].is_separator());
        assert!(!rows[&2].is_header());
        let body_source = snapshot.copy_range(blocks[2].source);
        assert!(rows[&2].estimated_line_count(&body_source, 220.0) > 1);
        assert_eq!(rows[&2].estimated_line_count(&body_source, 4_000.0), 1);
    }

    #[test]
    fn ultra_wide_cells_are_bounded_in_shared_geometry() {
        let source = format!("| {} | value |\n| --- | --- |\n", "x".repeat(10_000));
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let (blocks, _) = parse_markdown(&snapshot);
        let rows = build_markdown_table_styles(&snapshot, &blocks);
        let resolved = rows[&0].resolve();

        assert!(
            resolved.width < 1_000.0,
            "resolved width: {}",
            resolved.width
        );
        assert!(rows[&0].estimated_line_count(source.lines().next().unwrap(), 320.0) > 100);
    }

    #[test]
    fn reading_columns_fill_the_viewport_without_flex_reallocation() {
        let projection = test_table_projection();
        let layout = projection.table().resolve_for_viewport(480.0, 1.0);

        assert!(!layout.overflow);
        assert!((layout.column_widths.iter().sum::<f32>() - 480.0).abs() < 0.01);
        assert_eq!(layout.column_widths.len(), 2);
        assert!(layout.column_widths.iter().all(|width| *width > 0.0));
    }

    #[test]
    fn many_columns_overflow_as_one_table_instead_of_collapsing_cells() {
        let header = (0..24)
            .map(|index| format!("column-{index}"))
            .collect::<Vec<_>>()
            .join(" | ");
        let separator = std::iter::repeat_n("---", 24)
            .collect::<Vec<_>>()
            .join(" | ");
        let source = format!("| {header} |\n| {separator} |\n");
        let snapshot = DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
        let (blocks, _) = parse_markdown(&snapshot);
        let rows = build_markdown_table_styles(&snapshot, &blocks);
        let layout = rows[&0].table().resolve_for_viewport(320.0, 1.0);

        assert!(layout.overflow);
        assert!(layout.content_width > 320.0);
        assert_eq!(layout.column_widths.len(), 24);
        assert!(layout.column_widths.iter().all(|width| *width >= 50.0));
    }

    #[test]
    fn long_cjk_and_unbroken_urls_wrap_inside_their_cells() {
        let source = concat!(
            "| 类型 | 内容 |\n",
            "| --- | --- |\n",
            "| 中文 | 这是没有空格但必须在单元格内部换行的很长中文内容 |\n",
            "| URL | https://example.com/one/very/long/unbroken/path/that/must/wrap |\n",
        );
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let (blocks, _) = parse_markdown(&snapshot);
        let rows = build_markdown_table_styles(&snapshot, &blocks);

        let cjk = snapshot.copy_range(blocks[2].source);
        let url = snapshot.copy_range(blocks[3].source);
        assert!(rows[&2].estimated_line_count(&cjk, 240.0) > 1);
        assert!(rows[&3].estimated_line_count(&url, 240.0) > 1);
    }
}
