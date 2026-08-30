use std::ops::Range;

use crate::document::{ByteRange, DocumentSnapshot, LineIndex, TextSnapshot};

const DEFAULT_MAX_SHAPED_LINE_BYTES: usize = 64 * 1024;
const CARET_CONTEXT_BYTES: u64 = 64;

pub(super) struct VisibleSourceLine {
    pub(super) source_range: ByteRange,
    pub(super) visible_range: ByteRange,
    pub(super) text: String,
}

pub(super) struct SourceDisplayMap {
    line_height: f32,
    max_shaped_line_bytes: usize,
}

impl Default for SourceDisplayMap {
    fn default() -> Self {
        Self {
            line_height: super::LINE_HEIGHT,
            max_shaped_line_bytes: DEFAULT_MAX_SHAPED_LINE_BYTES,
        }
    }
}

impl SourceDisplayMap {
    pub(super) fn visible_line_range(
        &self,
        snapshot: &DocumentSnapshot,
        scroll_y: f32,
        viewport_height: f32,
    ) -> Range<u64> {
        let first = (scroll_y.max(0.0) / self.line_height).floor() as u64;
        let count = (viewport_height.max(0.0) / self.line_height).ceil() as u64 + 2;
        first.min(snapshot.len_lines())..(first + count).min(snapshot.len_lines())
    }

    pub(super) fn source_line(
        &self,
        snapshot: &DocumentSnapshot,
        line: LineIndex,
        anchor: Option<crate::document::ByteOffset>,
    ) -> Option<VisibleSourceLine> {
        let source_range = snapshot.line_content_range(line).ok()?;
        let mut visible_start = source_range.start.0;
        if source_range.len() > self.max_shaped_line_bytes as u64
            && let Some(anchor) =
                anchor.filter(|anchor| *anchor >= source_range.start && *anchor <= source_range.end)
        {
            visible_start = anchor
                .0
                .saturating_sub(CARET_CONTEXT_BYTES)
                .max(source_range.start.0);
            while visible_start > source_range.start.0
                && !snapshot.is_char_boundary(crate::document::ByteOffset(visible_start))
            {
                visible_start -= 1;
            }
        }
        let mut visible_end =
            (visible_start + self.max_shaped_line_bytes as u64).min(source_range.end.0);
        while visible_end > visible_start
            && !snapshot.is_char_boundary(crate::document::ByteOffset(visible_end))
        {
            visible_end -= 1;
        }
        let visible_range = ByteRange::new(visible_start, visible_end);
        let text = snapshot.copy_range(visible_range);
        Some(VisibleSourceLine {
            source_range,
            visible_range,
            text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentSnapshot;

    #[test]
    fn queries_only_viewport_lines_and_caps_extreme_shaping() {
        let mut source = "line\n".repeat(100_000);
        source.push_str(&"界".repeat(30_000));
        let snapshot = DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
        let map = SourceDisplayMap::default();
        let visible = map.visible_line_range(&snapshot, 50_000.0, 720.0);
        assert!(visible.end - visible.start <= 35);
        let last = map
            .source_line(&snapshot, LineIndex(snapshot.len_lines() - 1), None)
            .unwrap();
        assert!(last.visible_range.end < last.source_range.end);
        assert!(last.text.len() <= DEFAULT_MAX_SHAPED_LINE_BYTES);
        assert!(last.text.is_char_boundary(last.text.len()));

        let anchored = map
            .source_line(
                &snapshot,
                LineIndex(snapshot.len_lines() - 1),
                Some(last.source_range.end),
            )
            .unwrap();
        assert_eq!(anchored.visible_range.end, anchored.source_range.end);
        assert!(anchored.visible_range.start > anchored.source_range.start);
    }
}
