//! Shared text shaping helpers, cache keys, and folded/marked display text.

use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use gpui::{Pixels, TextRun, WrappedLine};

use crate::document::ByteRange;
use crate::editor::syntax;

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

/// The complete inputs to shaping and its optional fence-glyph adjustment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::editor) struct ShapeKey {
    text: gpui::SharedString,
    runs: Arc<[TextRun]>,
    font_size_bits: u32,
    wrap_width_bits: Option<u32>,
    fence_backticks: Option<Range<usize>>,
}

impl Hash for ShapeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // TextRun supports equality but not Hash. Hash the geometry and run
        // count; full equality also checks every font and paint attribute.
        (
            &self.text,
            self.font_size_bits,
            self.wrap_width_bits,
            &self.fence_backticks,
            self.runs.len(),
        )
            .hash(state);
    }
}

pub(super) fn shape_key(
    text: &gpui::SharedString,
    runs: &Arc<[TextRun]>,
    font_size: Pixels,
    wrap_width: Option<Pixels>,
    fence_backticks: Option<Range<usize>>,
) -> ShapeKey {
    ShapeKey {
        text: text.clone(),
        runs: runs.clone(),
        font_size_bits: f32::from(font_size).to_bits(),
        wrap_width_bits: wrap_width.map(|width| f32::from(width).to_bits()),
        fence_backticks,
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
