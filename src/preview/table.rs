use std::{collections::HashMap, ops::Range, sync::Arc};

use gpui::{
    AnyElement, Div, FontWeight, ScrollHandle, SharedString, TextRun, div, font, prelude::*, px,
    rgb,
};
use smallvec::SmallVec;
#[cfg(test)]
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use crate::{
    document::{ByteRange, TextSnapshot},
    org_syntax::{BlockArena, BlockId, BlockKind},
};

use super::{
    DocumentFormat,
    markdown::{MarkdownBlock, MarkdownKind},
    parse_document_inline,
    view::{ReadingInteraction, SelectableReadingText, reading_row_selection, styled_inline_runs},
};

#[cfg(test)]
const CELL_WIDTH_PX: f32 = 8.45;
#[cfg(test)]
const PIPE_WIDTH_PX: f32 = 12.0;
#[cfg(test)]
const CELL_PADDING_PX: f32 = 18.0;
#[cfg(test)]
const CELL_CONTENT_INSET_PX: f32 = 4.0;
const MAX_COLUMN_WIDTH: usize = 64;
const PROJECTED_HORIZONTAL_INSET_PX: f32 = 4.0;
const MIN_COLUMN_CONTENT_CHARS: usize = 4;
const TABLE_FRAME_WIDTH_PX: f32 = 2.0;

#[derive(Clone, Debug)]
pub(crate) struct TableRenderProjection {
    columns: Arc<[TableColumnSpec]>,
}

#[derive(Clone, Debug)]
pub(crate) struct TableRowProjection {
    table: Arc<TableRenderProjection>,
    group_id: BlockId,
    format: DocumentFormat,
    cells: Arc<[TableCell]>,
    separator: bool,
    header: bool,
    first: bool,
}

impl TableRenderProjection {
    pub(crate) fn columns(&self) -> &[TableColumnSpec] {
        &self.columns
    }

    #[cfg(test)]
    pub(crate) fn resolve(&self) -> ResolvedTable {
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

    pub(crate) fn same_geometry(&self, other: &Self) -> bool {
        self.columns == other.columns
    }

    pub(crate) fn resolve_for_viewport(
        &self,
        available_width: f32,
        zoom: f32,
        style: super::PreviewStyle,
    ) -> Arc<ReadingTableLayout> {
        let available_width = available_width.max(1.0);
        let zoom = zoom.max(0.01);
        let character_width = (style.typography.body_size - 1.0).max(12.0) * 0.65 * zoom;
        let cell_padding = style.spacing.table_cell_x * 2.0 * zoom;
        let preferred = self
            .columns
            .iter()
            .map(|column| column.width_chars as f32 * character_width + cell_padding)
            .collect::<Vec<_>>();
        let minimum = self
            .columns
            .iter()
            .map(|column| {
                let minimum_chars = column.width_chars.clamp(1, MIN_COLUMN_CONTENT_CHARS);
                minimum_chars as f32 * character_width + cell_padding
            })
            .collect::<Vec<_>>();
        let minimum_width = minimum.iter().sum::<f32>();
        let preferred_width = preferred.iter().sum::<f32>();

        if minimum_width > available_width {
            return Arc::new(ReadingTableLayout {
                column_widths: minimum.clone().into(),
                content_width: minimum_width,
                overflow: true,
            });
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
        Arc::new(ReadingTableLayout {
            column_widths: column_widths.into(),
            content_width: available_width,
            overflow: false,
        })
    }
}

impl TableRowProjection {
    pub(crate) fn table(&self) -> &TableRenderProjection {
        &self.table
    }

    pub(crate) fn columns(&self) -> &[TableColumnSpec] {
        self.table.columns()
    }

    pub(crate) fn group_id(&self) -> BlockId {
        self.group_id
    }

    pub(crate) fn cells(&self) -> &[TableCell] {
        &self.cells
    }

    pub(crate) fn is_separator(&self) -> bool {
        self.separator
    }

    pub(crate) fn is_header(&self) -> bool {
        self.header
    }

    pub(crate) fn is_first(&self) -> bool {
        self.first
    }

    pub(crate) fn resolved_reading_layout(
        &self,
        available_width: f32,
        zoom: f32,
        style: super::PreviewStyle,
    ) -> Arc<ReadingTableLayout> {
        self.table.resolve_for_viewport(
            (available_width - TABLE_FRAME_WIDTH_PX).max(1.0),
            zoom,
            style,
        )
    }

    #[cfg(test)]
    pub(crate) fn resolve(&self) -> ResolvedTable {
        self.table.resolve()
    }

    pub(crate) fn estimated_line_count(
        &self,
        source: &str,
        available_width: f32,
        zoom: f32,
        style: super::PreviewStyle,
    ) -> usize {
        if self.separator || self.cells.is_empty() {
            return 1;
        }
        let content_columns = self.content_columns(available_width, zoom, style);
        self.columns()
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let content_chars = content_columns[index];
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

    pub(crate) fn resolved_content_widths(
        &self,
        layout: &ReadingTableLayout,
        zoom: f32,
        style: super::PreviewStyle,
    ) -> Vec<f32> {
        let cell_padding = style.spacing.table_cell_x * 2.0 * zoom;
        layout
            .column_widths
            .iter()
            .map(|width| (width - cell_padding).max(1.0))
            .collect()
    }

    pub(crate) fn display_cell_texts(&self, source: &str) -> Vec<String> {
        self.columns()
            .iter()
            .enumerate()
            .map(|(index, _)| {
                self.cells
                    .get(index)
                    .map(|cell| parse_document_inline(self.format, cell.text(source)).text)
                    .unwrap_or_default()
            })
            .collect()
    }

    pub(crate) fn shaped_display_cells(
        &self,
        source: &str,
        layout: &ReadingTableLayout,
        zoom: f32,
        style: super::PreviewStyle,
        text_system: &gpui::WindowTextSystem,
    ) -> Vec<Vec<String>> {
        if self.separator {
            return Vec::new();
        }
        let widths = self.resolved_content_widths(layout, zoom, style);
        let font_size = (style.typography.body_size - 1.0).max(12.0) * zoom;
        let mut table_font = font(style.typography.body_family);
        table_font.weight = if self.header {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        };
        self.display_cell_texts(source)
            .into_iter()
            .zip(widths)
            .map(|(text, width)| {
                if text.is_empty() {
                    return vec![String::new()];
                }
                let shared: SharedString = text.clone().into();
                let runs = [TextRun {
                    len: shared.len(),
                    font: table_font.clone(),
                    color: gpui::rgb(style.palette.foreground).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }];
                let mut starts = vec![0];
                if let Ok(shaped) = text_system.shape_text(
                    shared,
                    px(font_size),
                    &runs,
                    Some(px(width.max(1.0))),
                    None,
                ) && let Some(line) = shaped.first()
                {
                    starts.extend(line.wrap_boundaries().iter().filter_map(|boundary| {
                        line.runs()
                            .get(boundary.run_ix)
                            .and_then(|run| run.glyphs.get(boundary.glyph_ix))
                            .map(|glyph| glyph.index)
                    }));
                }
                starts.push(text.len());
                starts.sort_unstable();
                starts.dedup();
                let lines = starts
                    .windows(2)
                    .filter_map(|pair| {
                        let (start, end) = (pair[0], pair[1]);
                        (start < end && text.is_char_boundary(start) && text.is_char_boundary(end))
                            .then(|| text[start..end].to_owned())
                    })
                    .collect::<Vec<_>>();
                if lines.is_empty() { vec![text] } else { lines }
            })
            .collect()
    }

    /// Retained for estimator contract tests; live Reading and Minimap geometry uses
    /// `shaped_display_cells` so both consume the same glyph wrap boundaries.
    #[cfg(test)]
    pub(crate) fn wrapped_display_cells(
        &self,
        source: &str,
        available_width: f32,
        zoom: f32,
        style: super::PreviewStyle,
    ) -> Vec<Vec<String>> {
        if self.separator {
            return Vec::new();
        }
        let content_columns = self.content_columns(available_width, zoom, style);
        self.columns()
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let text = self
                    .cells
                    .get(index)
                    .map(|cell| parse_document_inline(self.format, cell.text(source)).text)
                    .unwrap_or_default();
                wrap_display_text(&text, content_columns[index])
            })
            .collect()
    }

    fn content_columns(
        &self,
        available_width: f32,
        zoom: f32,
        style: super::PreviewStyle,
    ) -> Vec<usize> {
        let layout = self.table.resolve_for_viewport(
            (available_width - TABLE_FRAME_WIDTH_PX).max(1.0),
            zoom,
            style,
        );
        let character_width = (style.typography.body_size - 1.0).max(12.0) * 0.65 * zoom;
        let cell_padding = style.spacing.table_cell_x * 2.0 * zoom;
        layout
            .column_widths
            .iter()
            .map(|cell_width| {
                ((cell_width - cell_padding).max(character_width) / character_width)
                    .floor()
                    .max(1.0) as usize
            })
            .collect()
    }
}

#[cfg(test)]
fn wrap_display_text(text: &str, max_columns: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let max_columns = max_columns.max(1);
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut width = 0usize;
    for (offset, character) in text.char_indices() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if width > 0 && width + character_width > max_columns {
            lines.push(text[start..offset].to_owned());
            start = offset;
            width = 0;
        }
        width += character_width;
    }
    lines.push(text[start..].to_owned());
    lines
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Alignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TableCell {
    text_range: Range<usize>,
    align_right: bool,
}

impl TableCell {
    pub(crate) fn text<'a>(&self, source: &'a str) -> &'a str {
        source.get(self.text_range.clone()).unwrap_or_default()
    }

    pub(crate) fn align_right(&self) -> bool {
        self.align_right
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TableColumnSpec {
    width_chars: usize,
    alignment: Alignment,
}

impl TableColumnSpec {
    pub(crate) fn alignment(self) -> Alignment {
        self.alignment
    }

    #[cfg(test)]
    pub(crate) fn width_px(self) -> f32 {
        self.width_chars as f32 * CELL_WIDTH_PX + CELL_PADDING_PX
    }
}

#[cfg(test)]
pub(crate) fn test_table_projection() -> TableRowProjection {
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
        format: DocumentFormat::Org,
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

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResolvedTableColumn {
    pub(crate) start_x: f32,
    pub(crate) end_x: f32,
    pub(crate) content_start_x: f32,
    pub(crate) content_end_x: f32,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedTable {
    pub(crate) width: f32,
    pub(crate) columns: Arc<[ResolvedTableColumn]>,
    pub(crate) separators: Arc<[f32]>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReadingTableLayout {
    column_widths: Arc<[f32]>,
    content_width: f32,
    overflow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ProjectedTableColumn {
    pub(crate) start_x: f32,
    pub(crate) end_x: f32,
    pub(crate) content_start_x: f32,
    pub(crate) content_end_x: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProjectedTable {
    pub(crate) columns: SmallVec<[ProjectedTableColumn; 8]>,
    pub(crate) separators: SmallVec<[f32; 9]>,
}

/// Projects an already resolved Reading layout into minimap coordinates. Callers that also
/// measure or shape the row should use this entry point so every consumer observes one layout
/// artifact rather than independently resolving column widths.
pub(crate) fn project_resolved_table(
    layout: &ReadingTableLayout,
    zoom: f32,
    target_width: f32,
    style: super::PreviewStyle,
) -> ProjectedTable {
    let drawable = (target_width - PROJECTED_HORIZONTAL_INSET_PX * 2.0).max(1.0);
    let scale = drawable / layout.content_width.max(1.0);
    let project_x = |x: f32| PROJECTED_HORIZONTAL_INSET_PX + x * scale;
    let content_inset = style.spacing.table_cell_x * zoom;
    let mut cursor = 0.0;
    let mut separators = SmallVec::with_capacity(layout.column_widths.len() + 1);
    separators.push(project_x(0.0));
    let columns = layout
        .column_widths
        .iter()
        .map(|width| {
            let start_x = cursor;
            let end_x = cursor + width;
            cursor = end_x;
            separators.push(project_x(end_x));
            ProjectedTableColumn {
                start_x: project_x(start_x),
                end_x: project_x(end_x),
                content_start_x: project_x((start_x + content_inset).min(end_x)),
                content_end_x: project_x((end_x - content_inset).max(start_x)),
            }
        })
        .collect();
    ProjectedTable {
        columns,
        separators,
    }
}

pub(crate) fn build_table_styles(
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

pub(crate) fn build_markdown_table_styles(
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
                format,
                cells: cells.into(),
                separator,
                header: separator_index.is_some_and(|separator| position < separator),
                first: position == 0,
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_table_row(
    source: &str,
    projection: &TableRowProjection,
    format: DocumentFormat,
    layout: &ReadingTableLayout,
    zoom: f32,
    row_id: usize,
    horizontal_scroll: Option<&ScrollHandle>,
    style: super::PreviewStyle,
    interaction: Option<&ReadingInteraction>,
) -> Div {
    let palette = style.palette;
    let horizontal_rules = style.variants.table == super::style::TableVariant::HorizontalRules;
    if projection.separator {
        return div()
            .w_full()
            .h(px(if horizontal_rules { 1.0 } else { 2.0 }))
            .bg(rgb(if horizontal_rules {
                palette.border_strong
            } else {
                palette.border
            }));
    }
    let displays = projection
        .columns()
        .iter()
        .enumerate()
        .map(|(index, _)| {
            projection
                .cells
                .get(index)
                .map(|cell| parse_document_inline(format, cell.text(source)))
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    let selection_text_len = displays
        .iter()
        .map(|display| display.text.len())
        .sum::<usize>()
        + displays.len().saturating_sub(1);
    let mut text_offset = 0;
    let mut elements: Vec<AnyElement> = Vec::with_capacity(projection.columns().len());
    for (index, (column, display)) in projection.columns().iter().zip(displays).enumerate() {
        let width_px = layout.column_widths[index];
        let cell = projection.cells.get(index);
        let align_right =
            column.alignment == Alignment::Right || cell.is_some_and(TableCell::align_right);
        let text: gpui::SharedString = display.text.into();
        let text_len = text.len();
        let styled = styled_inline_runs(text, display.spans.into(), style);
        let cell_text = if let Some(interaction) = interaction {
            let selection = reading_row_selection(interaction, row_id, selection_text_len)
                .and_then(|(selection, include_newline)| {
                    let start = selection.start.max(text_offset);
                    let end = selection.end.min(text_offset + text_len);
                    (start < end).then_some((
                        start - text_offset..end - text_offset,
                        include_newline && end == selection_text_len,
                    ))
                });
            SelectableReadingText::new(
                format!("reading-table-cell-{row_id}-{index}"),
                styled,
                interaction.panel.clone(),
                row_id,
                selection,
            )
            .with_text_offset(text_offset)
            .restrict_drag_to_bounds()
            .into_any_element()
        } else {
            styled.into_any_element()
        };
        let content = div()
            .w_full()
            .min_w_0()
            .flex_1()
            .whitespace_normal()
            .when(column.alignment == Alignment::Center, |element| {
                element.text_center()
            })
            .when(align_right, |element| element.text_right())
            .child(cell_text);
        elements.push(
            div()
                .w(px(width_px))
                .min_w_0()
                .flex_none()
                .overflow_hidden()
                .whitespace_normal()
                .px(px(style.spacing.table_cell_x * zoom))
                .py(px(style.spacing.table_cell_y * zoom))
                .flex()
                .flex_col()
                .items_start()
                .when(
                    !horizontal_rules && index + 1 < projection.columns().len(),
                    |element| element.border_r_1().border_color(rgb(palette.border)),
                )
                .child(content)
                .into_any(),
        );
        text_offset += text_len + usize::from(index + 1 < projection.columns().len());
    }
    let table = div()
        .w(px(layout.content_width + TABLE_FRAME_WIDTH_PX))
        .flex_none()
        .flex()
        .items_stretch()
        .when(!horizontal_rules, |element| {
            element.border_l_1().border_r_1()
        })
        .border_b_1()
        .when(projection.is_first(), |element| element.border_t_1())
        .border_color(rgb(if horizontal_rules {
            palette.border_strong
        } else {
            palette.border
        }))
        .when(projection.is_header(), |element| {
            element
                .bg(rgb(palette.surface))
                .font_weight(gpui::FontWeight::SEMIBOLD)
        })
        .when(horizontal_rules, |element| {
            element.hover(|hover| hover.bg(rgb(palette.hover)))
        })
        .font_family(style.typography.body_family)
        .text_size(px((style.typography.body_size - 1.0).max(12.0) * zoom))
        .line_height(px(
            (style.typography.body_line_height - 3.0).max(18.0) * zoom
        ))
        .text_color(rgb(palette.foreground))
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

    fn base_style() -> crate::preview::PreviewStyle {
        *crate::preview::preview_style(crate::preview::PreviewStyleId::Base)
    }

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
        assert!(rows[&2].estimated_line_count(&body_source, 220.0, 1.0, base_style()) > 1);
        assert_eq!(
            rows[&2].estimated_line_count(&body_source, 4_000.0, 1.0, base_style()),
            1
        );
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
        assert!(
            rows[&0].estimated_line_count(source.lines().next().unwrap(), 320.0, 1.0, base_style())
                > 100
        );
    }

    #[test]
    fn reading_columns_fill_the_viewport_without_flex_reallocation() {
        let projection = test_table_projection();
        let layout = projection
            .table()
            .resolve_for_viewport(480.0, 1.0, base_style());

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
        let layout = rows[&0]
            .table()
            .resolve_for_viewport(320.0, 1.0, base_style());

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
        assert!(rows[&2].estimated_line_count(&cjk, 240.0, 1.0, base_style()) > 1);
        assert!(rows[&3].estimated_line_count(&url, 240.0, 1.0, base_style()) > 1);
    }
}
