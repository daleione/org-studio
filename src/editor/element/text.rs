//! Text shaping primitives shared by editor row layout and the minimap
//! layout pass: run slicing, table fragment shaping, shape-cache keys and
//! folded/marked display helpers.

use std::ops::Range;
use std::sync::Arc;

use gpui::{Pixels, TextRun, WrappedLine};

use crate::document::ByteRange;
use crate::editor::{ShapeKey, TableVisualFragment, TableVisualLayout, syntax};

pub(super) fn slice_text_runs(runs: &[TextRun], range: Range<usize>) -> Vec<TextRun> {
    let mut sliced = Vec::new();
    let mut offset = 0usize;
    for run in runs {
        let run_range = offset..offset + run.len;
        let start = run_range.start.max(range.start);
        let end = run_range.end.min(range.end);
        if start < end {
            let mut run = run.clone();
            run.len = end - start;
            sliced.push(run);
        }
        offset = run_range.end;
        if offset >= range.end {
            break;
        }
    }
    sliced
}

pub(super) fn table_visual_layout(
    text: &gpui::SharedString,
    runs: &[TextRun],
    font_size: Pixels,
    columns: &[usize],
    format: crate::document::DocumentFormat,
    text_system: &gpui::WindowTextSystem,
) -> Option<Arc<TableVisualLayout>> {
    let delimiters = crate::document::table::delimiter_offsets(text, format)
        .into_iter()
        .map(|start| start..start + 1)
        .collect::<Vec<_>>();
    if delimiters.len() < 2 || columns.is_empty() {
        return None;
    }

    let mut base = runs.first()?.clone();
    base.len = 1;
    let space: gpui::SharedString = " ".into();
    let space_advance = text_system
        .shape_line(space, font_size, std::slice::from_ref(&base), None)
        .width();
    let mut fragments = Vec::with_capacity(delimiters.len() * 2 + 1);
    let shape_fragment = |range: Range<usize>, x: Pixels| -> Option<TableVisualFragment> {
        if range.is_empty() {
            return None;
        }
        let fragment_text: gpui::SharedString = text[range.clone()].to_owned().into();
        let fragment_runs = slice_text_runs(runs, range.clone());
        (!fragment_runs.is_empty()).then(|| TableVisualFragment {
            display_range: range,
            x,
            layout: Arc::new(text_system.shape_line(
                fragment_text,
                font_size,
                &fragment_runs,
                None,
            )),
        })
    };

    let first = delimiters.first()?.clone();
    let indent = shape_fragment(0..first.start, Pixels::ZERO);
    let mut delimiter_x = indent
        .as_ref()
        .map_or(Pixels::ZERO, |fragment| fragment.layout.width());
    if let Some(indent) = indent {
        fragments.push(indent);
    }

    for (column, delimiter_range) in delimiters.iter().enumerate() {
        let delimiter = shape_fragment(delimiter_range.clone(), delimiter_x)?;
        let delimiter_width = delimiter.layout.width();
        fragments.push(delimiter);

        let segment_end = delimiters
            .get(column + 1)
            .map_or(text.len(), |next| next.start);
        if let Some(segment) = shape_fragment(
            delimiter_range.end..segment_end,
            delimiter_x + delimiter_width,
        ) {
            fragments.push(segment);
        }
        if delimiters.get(column + 1).is_some() {
            let logical_width = columns.get(column).copied().unwrap_or(1).saturating_add(2) as f32;
            delimiter_x += delimiter_width + space_advance * logical_width;
        }
    }

    let width = fragments.iter().fold(Pixels::ZERO, |width, fragment| {
        width.max(fragment.x + fragment.layout.width())
    });
    Some(Arc::new(TableVisualLayout {
        fragments: fragments.into(),
        width,
        len: text.len(),
    }))
}

// Keep source bytes and all horizontal/caret geometry intact. Only the glyphs
// of a Markdown backtick boundary receive an optical vertical adjustment.
pub(super) fn markdown_fence_backticks(
    text: &str,
    block: Option<&syntax::EditorBlockDecoration>,
) -> Option<Range<usize>> {
    let block = block?;
    if block.kind != syntax::EditorBlockKind::MarkdownFence
        || block.edge == syntax::EditorBlockEdge::Body
    {
        return None;
    }
    let trimmed = text.trim_start();
    let start = text.len() - trimmed.len();
    let count = trimmed.bytes().take_while(|byte| *byte == b'`').count();
    (count >= 3).then_some(start..start + count)
}

pub(super) fn lower_fence_backticks(line: &mut WrappedLine, range: Range<usize>, offset: Pixels) {
    let source = &line.unwrapped_layout;
    let mut runs = source.runs.clone();
    for glyph in runs.iter_mut().flat_map(|run| &mut run.glyphs) {
        if range.contains(&glyph.index) {
            glyph.position.y += offset;
        }
    }
    let layout = gpui::WrappedLineLayout {
        unwrapped_layout: Arc::new(gpui::LineLayout {
            font_size: source.font_size,
            width: source.width,
            ascent: source.ascent,
            descent: source.descent,
            runs,
            len: source.len,
        }),
        wrap_boundaries: line.wrap_boundaries.clone(),
        wrap_width: line.wrap_width,
    };
    *std::ops::DerefMut::deref_mut(line) = Arc::new(layout);
}

pub(super) fn shape_key(
    text: &gpui::SharedString,
    font_size: Pixels,
    marked: Option<std::ops::Range<usize>>,
    wrap_width: Option<Pixels>,
    syntax_key: u8,
    code_language: Option<Arc<str>>,
) -> ShapeKey {
    ShapeKey {
        generated_line: None,
        text: text.clone(),
        font_size_bits: f32::from(font_size).to_bits(),
        wrap_width_bits: wrap_width.map_or(0, |width| f32::from(width).to_bits()),
        syntax_key,
        code_language,
        marked: marked.map(|range| (range.start, range.end)),
        theme_generation: crate::theme::theme_generation(),
    }
}

pub(super) fn local_marked(
    marked: Option<ByteRange>,
    line: ByteRange,
    display: &crate::editor::layout_map::DisplayLineText,
) -> Option<std::ops::Range<usize>> {
    let marked = marked?;
    let start = marked.start.0.max(line.start.0);
    let end = marked.end.0.min(line.end.0);
    if start >= end {
        return None;
    }
    Some(
        display.source_to_display((start - line.start.0) as usize)
            ..display.source_to_display((end - line.start.0) as usize),
    )
}

pub(super) fn folded_display_text(mut text: String, folded: bool) -> String {
    if folded {
        text.push_str("...");
    }
    text
}
