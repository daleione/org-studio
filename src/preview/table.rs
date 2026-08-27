use std::{collections::HashMap, ops::Range, sync::Arc};

use gpui::{AnyElement, Div, div, prelude::*, px, rgb};
use smallvec::SmallVec;
use unicode_width::UnicodeWidthStr;

use crate::{
    document::{ByteRange, TextSnapshot},
    org_syntax::{BlockArena, BlockId, BlockKind},
    theme::current_theme,
};

use super::markdown::{MarkdownBlock, MarkdownKind};

const CELL_WIDTH_PX: f32 = 8.45;
const PIPE_WIDTH_PX: f32 = 12.0;
const CELL_PADDING_PX: f32 = 18.0;
const CELL_CONTENT_INSET_PX: f32 = 4.0;
const MAX_COLUMN_WIDTH: usize = 64;
const PROJECTED_HORIZONTAL_INSET_PX: f32 = 4.0;

#[derive(Clone, Debug)]
pub(super) struct TableRenderProjection {
    columns: Arc<[TableColumnSpec]>,
}

#[derive(Clone, Debug)]
pub(super) struct TableRowProjection {
    table: Arc<TableRenderProjection>,
    cells: Arc<[TableCell]>,
    separator: bool,
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
}

impl TableRowProjection {
    pub(super) fn table(&self) -> &TableRenderProjection {
        &self.table
    }

    pub(super) fn columns(&self) -> &[TableColumnSpec] {
        self.table.columns()
    }

    pub(super) fn cells(&self) -> &[TableCell] {
        &self.cells
    }

    pub(super) fn is_separator(&self) -> bool {
        self.separator
    }

    pub(super) fn resolve(&self) -> ResolvedTable {
        self.table.resolve()
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
        build_table_group(text, &parsed, &mut result);
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
        build_table_group(text, &group, &mut result);
        start = end;
    }
    result
}

fn build_table_group(
    text: &dyn TextSnapshot,
    rows: &[(BlockId, ByteRange)],
    result: &mut HashMap<BlockId, TableRowProjection>,
) {
    let mut widths = Vec::new();
    let parsed = rows
        .iter()
        .map(|(id, range)| {
            let source = text.copy_range(*range);
            let separator = is_separator(&source);
            let cells = parse_cells(&source);
            if !separator {
                widths.resize(widths.len().max(cells.len()), 1);
                for (index, cell) in cells.iter().enumerate() {
                    widths[index] = widths[index]
                        .max(UnicodeWidthStr::width(cell.text(&source)))
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
    for (index, separator, cells, _) in parsed {
        result.insert(
            index,
            TableRowProjection {
                table: table.clone(),
                cells: cells.into(),
                separator,
            },
        );
    }
}

pub(super) fn render_table_row(source: &str, projection: &TableRowProjection) -> Div {
    let theme = current_theme();
    let resolved = projection.resolve();
    let mut elements: Vec<AnyElement> = Vec::with_capacity(projection.columns().len() * 2 + 1);
    elements.push(table_token("|").into_any());
    for (index, (column, geometry)) in projection
        .columns()
        .iter()
        .zip(resolved.columns.iter())
        .enumerate()
    {
        let width_px = geometry.end_x - geometry.start_x;
        if projection.separator {
            elements.push(
                div()
                    .flex_none()
                    .w(px(width_px))
                    .h(px(22.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .whitespace_nowrap()
                    .child("-".repeat(column.width_chars + 2))
                    .into_any(),
            );
            elements.push(
                table_token(if index + 1 == projection.columns().len() {
                    "|"
                } else {
                    "+"
                })
                .into_any(),
            );
            continue;
        }
        let cell = projection.cells.get(index);
        elements.push(
            div()
                .flex_none()
                .w(px(width_px))
                .px_1()
                .flex()
                .items_center()
                .when(column.alignment == Alignment::Center, |element| {
                    element.justify_center()
                })
                .when(
                    column.alignment == Alignment::Right
                        || cell.is_some_and(TableCell::align_right),
                    |element| element.justify_end(),
                )
                .child(
                    cell.map(|cell| cell.text(source))
                        .unwrap_or_default()
                        .to_owned(),
                )
                .into_any(),
        );
        elements.push(table_token("|").into_any());
    }
    div()
        .w_full()
        .flex()
        .items_center()
        .font_family("Menlo")
        .text_size(px(14.0))
        .line_height(px(22.0))
        .text_color(rgb(theme.foreground))
        .children(elements)
}

fn table_token(token: &'static str) -> Div {
    div()
        .flex_none()
        .w(px(PIPE_WIDTH_PX))
        .text_center()
        .child(token)
}

fn is_separator(source: &str) -> bool {
    let source = source.trim();
    source.contains('-')
        && source
            .chars()
            .all(|character| matches!(character, '|' | '+' | '-' | ':' | ' ' | '\t'))
}

fn parse_cells(source: &str) -> Vec<TableCell> {
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
    let delimiters: &[char] = if separator { &['+', '|'] } else { &['|'] };
    let mut cells = Vec::new();
    let mut start = 0usize;
    for (end, delimiter) in inner
        .char_indices()
        .filter(|(_, ch)| delimiters.contains(ch))
    {
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
    use crate::{document::RopeSnapshot, preview::markdown::parse_markdown};

    use super::{
        Alignment, CELL_PADDING_PX, CELL_WIDTH_PX, PIPE_WIDTH_PX, build_markdown_table_styles,
        is_separator, parse_cells, parse_separator_alignments, test_table_projection,
    };

    #[test]
    fn preserves_per_cell_org_alignment() {
        let source = "| apple |       42 |    score |";
        let cells = parse_cells(source);
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
        let cells = parse_cells(source);
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
        let snapshot = RopeSnapshot::from_utf8(
            b"| name | value |\n| --- | ---: |\n| a | 42 |\n\n| x | y |\n| --- | --- |\n".to_vec(),
        )
        .expect("valid fixture");
        let (blocks, _) = parse_markdown(&snapshot);
        let rows = build_markdown_table_styles(&snapshot, &blocks);

        assert!(std::ptr::eq(rows[&0].table(), rows[&1].table()));
        assert!(std::ptr::eq(rows[&1].table(), rows[&2].table()));
        assert!(!std::ptr::eq(rows[&2].table(), rows[&4].table()));
    }
}
