use std::{collections::HashMap, ops::Range};

use crate::document::{ByteOffset, ByteRange, DocumentSnapshot, LineIndex, TextSnapshot};
use unicode_width::UnicodeWidthChar;

const DEFAULT_MAX_SHAPED_LINE_BYTES: usize = 64 * 1024;
const CARET_CONTEXT_BYTES: u64 = 64;
const DEFAULT_TAB_SIZE: usize = 4;
const CHUNK_LINES: u64 = 256;

#[derive(Clone, Copy, Debug)]
struct TabExpansion {
    source: usize,
    display: usize,
    display_len: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct DisplayLineText {
    pub(super) text: String,
    tabs: Vec<TabExpansion>,
}

impl DisplayLineText {
    fn new(source: String, tab_size: usize) -> Self {
        if !source.contains('\t') {
            return Self {
                text: source,
                tabs: Vec::new(),
            };
        }
        let mut text = String::with_capacity(source.len());
        let mut tabs = Vec::new();
        let mut column = 0usize;
        for (source_index, ch) in source.char_indices() {
            if ch == '\t' {
                let display = text.len();
                let display_len = tab_size - column % tab_size;
                text.extend(std::iter::repeat_n(' ', display_len));
                tabs.push(TabExpansion {
                    source: source_index,
                    display,
                    display_len,
                });
                column += display_len;
            } else {
                text.push(ch);
                column += ch.width().unwrap_or(0);
            }
        }
        Self { text, tabs }
    }

    pub(super) fn source_to_display(&self, source: usize) -> usize {
        let adjustment = self
            .tabs
            .iter()
            .take_while(|tab| source > tab.source)
            .map(|tab| tab.display_len - 1)
            .sum::<usize>();
        source + adjustment
    }

    pub(super) fn display_to_source(&self, display: usize) -> usize {
        let mut adjustment = 0usize;
        for tab in &self.tabs {
            if display <= tab.display {
                break;
            }
            if display < tab.display + tab.display_len {
                return tab.source + usize::from(display - tab.display > tab.display_len / 2);
            }
            adjustment += tab.display_len - 1;
        }
        display.saturating_sub(adjustment)
    }
}

pub(super) struct VisibleSourceLine {
    pub(super) source_range: ByteRange,
    pub(super) visible_range: ByteRange,
    pub(super) display: DisplayLineText,
}

/// Sparse visual-row index. Every physical line has a one-row baseline; only measured wrapped
/// lines consume memory. Chunk totals use a Fenwick tree, avoiding a per-line allocation.
pub(super) struct SourceDisplayMap {
    line_height: f32,
    max_shaped_line_bytes: usize,
    tab_size: usize,
    soft_wrap: bool,
    line_count: u64,
    wrap_width_bits: u32,
    measured_chunks: HashMap<u64, Box<[u16; CHUNK_LINES as usize]>>,
    fenwick: Vec<i64>,
}

impl Default for SourceDisplayMap {
    fn default() -> Self {
        Self {
            line_height: super::LINE_HEIGHT,
            max_shaped_line_bytes: DEFAULT_MAX_SHAPED_LINE_BYTES,
            tab_size: DEFAULT_TAB_SIZE,
            soft_wrap: true,
            line_count: 0,
            wrap_width_bits: 0,
            measured_chunks: HashMap::new(),
            fenwick: vec![0],
        }
    }
}

impl SourceDisplayMap {
    pub(super) fn soft_wrap(&self) -> bool {
        self.soft_wrap
    }

    pub(super) fn wrap_width(&self) -> f32 {
        f32::from_bits(self.wrap_width_bits).max(1.0)
    }

    pub(super) fn set_soft_wrap(&mut self, soft_wrap: bool) -> bool {
        if self.soft_wrap == soft_wrap {
            return false;
        }
        self.soft_wrap = soft_wrap;
        self.clear_layout();
        true
    }

    pub(super) fn configure(&mut self, line_count: u64, wrap_width: f32) -> bool {
        let wrap_width_bits = wrap_width.max(1.0).to_bits();
        if self.line_count == line_count && self.wrap_width_bits == wrap_width_bits {
            return false;
        }
        let width_changed = self.wrap_width_bits != wrap_width_bits;
        self.line_count = line_count;
        self.wrap_width_bits = wrap_width_bits;
        if width_changed {
            self.clear_layout();
        } else {
            self.measured_chunks
                .retain(|chunk, _| *chunk * CHUNK_LINES < line_count);
            self.rebuild_fenwick();
        }
        true
    }

    fn clear_layout(&mut self) {
        self.measured_chunks.clear();
        self.fenwick = vec![0; self.line_count.div_ceil(CHUNK_LINES) as usize + 1];
    }

    pub(super) fn invalidate_layout(&mut self) {
        self.clear_layout();
    }

    pub(super) fn invalidate_layout_from(&mut self, first_line: u64) {
        let first_chunk = first_line / CHUNK_LINES;
        let first_local = (first_line % CHUNK_LINES) as usize;
        if let Some(rows) = self.measured_chunks.get_mut(&first_chunk) {
            rows[first_local..].fill(0);
        }
        self.measured_chunks
            .retain(|chunk, rows| *chunk <= first_chunk && rows.iter().any(|rows| *rows != 0));
        self.rebuild_fenwick();
    }

    fn rebuild_fenwick(&mut self) {
        self.fenwick = vec![0; self.line_count.div_ceil(CHUNK_LINES) as usize + 1];
        for (&chunk, rows) in &self.measured_chunks {
            let delta = rows
                .iter()
                .map(|rows| i64::from(rows.saturating_sub(1)))
                .sum::<i64>();
            let mut index = chunk as usize + 1;
            while index < self.fenwick.len() {
                self.fenwick[index] += delta;
                index += index & index.wrapping_neg();
            }
        }
    }

    pub(super) fn update_line_rows(&mut self, line: u64, row_count: usize) -> bool {
        if line >= self.line_count {
            return false;
        }
        let row_count = if self.soft_wrap {
            row_count.max(1).min(u16::MAX as usize) as u16
        } else {
            1
        };
        let chunk = line / CHUNK_LINES;
        let local = (line % CHUNK_LINES) as usize;
        let old = self
            .measured_chunks
            .get(&chunk)
            .map_or(1, |rows| rows[local].max(1));
        if old == row_count {
            return false;
        }
        if row_count == 1 {
            if let Some(rows) = self.measured_chunks.get_mut(&chunk) {
                rows[local] = 0;
                if rows.iter().all(|rows| *rows == 0) {
                    self.measured_chunks.remove(&chunk);
                }
            }
        } else {
            self.measured_chunks
                .entry(chunk)
                .or_insert_with(|| Box::new([0; CHUNK_LINES as usize]))[local] = row_count;
        }
        let delta = i64::from(row_count) - i64::from(old);
        let mut index = (line / CHUNK_LINES) as usize + 1;
        while index < self.fenwick.len() {
            self.fenwick[index] += delta;
            index += index & index.wrapping_neg();
        }
        true
    }

    fn extra_before_chunk(&self, chunk: usize) -> u64 {
        let mut index = chunk;
        let mut total = 0i64;
        while index > 0 {
            total += self.fenwick[index];
            index &= index - 1;
        }
        total.max(0) as u64
    }

    pub(super) fn line_start_visual_row(&self, line: u64) -> u64 {
        let line = line.min(self.line_count);
        let chunk = line / CHUNK_LINES;
        let local = (line % CHUNK_LINES) as usize;
        let local_extra = self.measured_chunks.get(&chunk).map_or(0, |rows| {
            rows[..local]
                .iter()
                .map(|rows| u64::from(rows.saturating_sub(1)))
                .sum()
        });
        line + self.extra_before_chunk(chunk as usize) + local_extra
    }

    pub(super) fn total_visual_rows(&self) -> u64 {
        self.line_start_visual_row(self.line_count)
    }

    pub(super) fn line_at_visual_row(&self, visual_row: u64) -> u64 {
        if self.line_count == 0 {
            return 0;
        }
        let target = visual_row.min(self.total_visual_rows().saturating_sub(1));
        let (mut low, mut high) = (0u64, self.line_count);
        while low < high {
            let middle = low + (high - low) / 2;
            if self.line_start_visual_row(middle + 1) <= target {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low.min(self.line_count - 1)
    }

    pub(super) fn visible_line_range(
        &self,
        snapshot: &DocumentSnapshot,
        scroll_y: f32,
        viewport_height: f32,
    ) -> Range<u64> {
        let first_visual = (scroll_y.max(0.0) / self.line_height).floor() as u64;
        let last_visual =
            first_visual + (viewport_height.max(0.0) / self.line_height).ceil() as u64 + 2;
        let first = self
            .line_at_visual_row(first_visual)
            .min(snapshot.len_lines());
        let last = self
            .line_at_visual_row(last_visual)
            .saturating_add(2)
            .min(snapshot.len_lines());
        first..last.max(first)
    }

    pub(super) fn source_line(
        &self,
        snapshot: &DocumentSnapshot,
        line: LineIndex,
        anchor: Option<ByteOffset>,
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
                && !snapshot.is_char_boundary(ByteOffset(visible_start))
            {
                visible_start -= 1;
            }
        }
        let mut visible_end =
            (visible_start + self.max_shaped_line_bytes as u64).min(source_range.end.0);
        while visible_end > visible_start && !snapshot.is_char_boundary(ByteOffset(visible_end)) {
            visible_end -= 1;
        }
        let visible_range = ByteRange::new(visible_start, visible_end);
        let display = DisplayLineText::new(snapshot.copy_range(visible_range), self.tab_size);
        Some(VisibleSourceLine {
            source_range,
            visible_range,
            display,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_wrap_index_maps_both_directions() {
        let mut map = SourceDisplayMap::default();
        map.configure(1_000_000, 640.0);
        assert!(map.update_line_rows(10, 4));
        assert!(map.update_line_rows(511, 3));
        assert_eq!(map.line_start_visual_row(11), 14);
        assert_eq!(map.line_at_visual_row(11), 10);
        assert_eq!(map.line_start_visual_row(512), 517);
        assert_eq!(map.total_visual_rows(), 1_000_005);
        assert_eq!(map.measured_chunks.len(), 2);
    }

    #[test]
    fn editing_late_lines_preserves_measured_prefix() {
        let mut map = SourceDisplayMap::default();
        map.configure(1_000, 640.0);
        map.update_line_rows(10, 4);
        map.update_line_rows(800, 5);
        map.invalidate_layout_from(700);
        assert_eq!(map.line_start_visual_row(11), 14);
        assert_eq!(map.line_start_visual_row(801), 804);

        map.configure(1_001, 640.0);
        assert_eq!(map.line_start_visual_row(11), 14);
    }

    #[test]
    fn tab_expansion_round_trips_source_boundaries() {
        let display = DisplayLineText::new("a\tb".to_owned(), 4);
        assert_eq!(display.text, "a   b");
        assert_eq!(display.source_to_display(2), 4);
        assert_eq!(display.display_to_source(4), 2);
    }

    #[test]
    fn queries_only_viewport_lines_and_caps_extreme_shaping() {
        let mut source = "line\n".repeat(100_000);
        source.push_str(&"界".repeat(30_000));
        let snapshot = DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
        let mut map = SourceDisplayMap::default();
        map.configure(snapshot.len_lines(), 800.0);
        let visible = map.visible_line_range(&snapshot, 50_000.0, 720.0);
        assert!(visible.end - visible.start <= 40);
        let last = map
            .source_line(&snapshot, LineIndex(snapshot.len_lines() - 1), None)
            .unwrap();
        assert!(last.visible_range.end < last.source_range.end);
        assert!(last.display.text.len() <= DEFAULT_MAX_SHAPED_LINE_BYTES);
        let anchored = map
            .source_line(
                &snapshot,
                LineIndex(snapshot.len_lines() - 1),
                Some(last.source_range.end),
            )
            .unwrap();
        assert_eq!(anchored.visible_range.end, anchored.source_range.end);
    }
}
