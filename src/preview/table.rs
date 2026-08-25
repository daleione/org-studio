use std::{collections::HashMap, sync::Arc};

use gpui::{AnyElement, Div, div, prelude::*, px, rgb};
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
const MAX_COLUMN_WIDTH: usize = 64;

pub(super) struct TableRowStyle {
    widths: Arc<Vec<usize>>,
    separator: bool,
    alignments: Arc<Vec<Alignment>>,
}

#[derive(Clone, Copy, Default)]
enum Alignment {
    #[default]
    Left,
    Center,
    Right,
}

struct Cell {
    text: String,
    align_right: bool,
    alignment: Alignment,
}

pub(super) fn build_table_styles(
    text: &dyn TextSnapshot,
    blocks: &BlockArena,
) -> HashMap<BlockId, TableRowStyle> {
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
) -> HashMap<BlockId, TableRowStyle> {
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
    result: &mut HashMap<BlockId, TableRowStyle>,
) {
    let parsed = rows
        .iter()
        .map(|(id, range)| {
            let source = text.copy_range(*range);
            (*id, is_separator(&source), parse_cells(&source))
        })
        .collect::<Vec<_>>();
    let columns = parsed
        .iter()
        .map(|(_, _, cells)| cells.len())
        .max()
        .unwrap_or(0);
    let mut widths = vec![1; columns];
    for (_, separator, cells) in &parsed {
        if *separator {
            continue;
        }
        for (index, cell) in cells.iter().enumerate() {
            widths[index] = widths[index]
                .max(UnicodeWidthStr::width(cell.text.as_str()))
                .min(MAX_COLUMN_WIDTH);
        }
    }
    let widths = Arc::new(widths);
    let alignments = Arc::new(
        parsed
            .iter()
            .find(|(_, separator, _)| *separator)
            .map(|(_, _, cells)| cells.iter().map(|cell| cell.alignment).collect())
            .unwrap_or_else(|| vec![Alignment::Left; columns]),
    );
    for (index, separator, _) in parsed {
        result.insert(
            index,
            TableRowStyle {
                widths: widths.clone(),
                separator,
                alignments: alignments.clone(),
            },
        );
    }
}

pub(super) fn render_table_row(source: &str, style: &TableRowStyle) -> Div {
    let theme = current_theme();
    let cells = parse_cells(source);
    let mut elements: Vec<AnyElement> = Vec::with_capacity(style.widths.len() * 2 + 1);
    elements.push(table_token("|").into_any());
    for (index, width) in style.widths.iter().enumerate() {
        let width_px = *width as f32 * CELL_WIDTH_PX + CELL_PADDING_PX;
        if style.separator {
            elements.push(
                div()
                    .flex_none()
                    .w(px(width_px))
                    .h(px(22.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .whitespace_nowrap()
                    .child("-".repeat(width + 2))
                    .into_any(),
            );
            elements.push(
                table_token(if index + 1 == style.widths.len() {
                    "|"
                } else {
                    "+"
                })
                .into_any(),
            );
            continue;
        }
        let cell = cells.get(index);
        elements.push(
            div()
                .flex_none()
                .w(px(width_px))
                .px_1()
                .flex()
                .items_center()
                .when(
                    matches!(style.alignments.get(index), Some(Alignment::Center)),
                    |element| element.justify_center(),
                )
                .when(
                    matches!(style.alignments.get(index), Some(Alignment::Right))
                        || cell.is_some_and(|cell| cell.align_right),
                    |element| element.justify_end(),
                )
                .child(cell.map(|cell| cell.text.clone()).unwrap_or_default())
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

fn parse_cells(source: &str) -> Vec<Cell> {
    if is_separator(source) {
        return source
            .trim()
            .trim_matches('|')
            .split(['+', '|'])
            .map(|raw| Cell {
                text: String::new(),
                align_right: false,
                alignment: match (raw.trim().starts_with(':'), raw.trim().ends_with(':')) {
                    (true, true) => Alignment::Center,
                    (false, true) => Alignment::Right,
                    _ => Alignment::Left,
                },
            })
            .collect();
    }
    source
        .trim()
        .trim_matches('|')
        .split('|')
        .map(|raw| {
            let leading = raw.len() - raw.trim_start().len();
            let trailing = raw.len() - raw.trim_end().len();
            Cell {
                text: raw.trim().to_owned(),
                align_right: leading > trailing,
                alignment: Alignment::Left,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{is_separator, parse_cells};

    #[test]
    fn preserves_per_cell_org_alignment() {
        let cells = parse_cells("| apple |       42 |    score |");
        assert!(!cells[0].align_right);
        assert!(cells[1].align_right);
        assert!(cells[2].align_right);
    }

    #[test]
    fn recognizes_standard_separator() {
        assert!(is_separator("|----------+-----------------------|"));
    }

    #[test]
    fn parses_markdown_separator_columns_and_alignment() {
        let cells = parse_cells("| :--- | :---: | ---: |");
        assert_eq!(cells.len(), 3);
        assert!(matches!(cells[0].alignment, super::Alignment::Left));
        assert!(matches!(cells[1].alignment, super::Alignment::Center));
        assert!(matches!(cells[2].alignment, super::Alignment::Right));
    }
}
