//! Shared text shaping helpers, cache keys, and folded/marked display text.

use std::ops::Range;
use std::sync::Arc;

use gpui::{Pixels, WrappedLine};

use crate::document::ByteRange;
use crate::editor::{ShapeKey, syntax};

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
