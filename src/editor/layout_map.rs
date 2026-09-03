use std::{collections::HashMap, ops::Range};

use crate::document::{ByteOffset, ByteRange, DocumentSnapshot, LineIndex, TextSnapshot};
use unicode_width::UnicodeWidthChar;

const DEFAULT_MAX_SHAPED_LINE_BYTES: usize = 64 * 1024;
const CARET_CONTEXT_BYTES: u64 = 64;
const DEFAULT_TAB_SIZE: usize = 4;
const CHUNK_LINES: u64 = 256;
const PIXEL_SCALE: f32 = 64.0;

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

/// Sparse pixel-height index. Every physical line has a baseline height; only measured lines
/// consume memory. Chunk totals use a Fenwick tree, avoiding a per-line allocation.
pub(super) struct EditorLayoutMap {
    line_height: f32,
    max_shaped_line_bytes: usize,
    tab_size: usize,
    soft_wrap: bool,
    line_count: u64,
    wrap_width_bits: u32,
    measured_heights: HashMap<u64, Box<[u32; CHUNK_LINES as usize]>>,
    height_fenwick: Vec<i64>,
    hidden_ranges: Vec<(Range<u64>, u64)>,
}

impl Default for EditorLayoutMap {
    fn default() -> Self {
        Self {
            line_height: super::LINE_HEIGHT,
            max_shaped_line_bytes: DEFAULT_MAX_SHAPED_LINE_BYTES,
            tab_size: DEFAULT_TAB_SIZE,
            soft_wrap: true,
            line_count: 0,
            wrap_width_bits: 0,
            measured_heights: HashMap::new(),
            height_fenwick: vec![0],
            hidden_ranges: Vec::new(),
        }
    }
}

impl EditorLayoutMap {
    pub(super) fn set_base_line_height(&mut self, line_height: f32) -> bool {
        let line_height = line_height.max(1.0);
        if (self.line_height - line_height).abs() < f32::EPSILON {
            return false;
        }
        self.line_height = line_height;
        self.clear_layout();
        true
    }

    pub(super) fn base_line_height(&self) -> f32 {
        self.line_height
    }

    pub(super) fn soft_wrap(&self) -> bool {
        self.soft_wrap
    }

    pub(super) fn wrap_width(&self) -> f32 {
        f32::from_bits(self.wrap_width_bits).max(1.0)
    }

    pub(super) fn line_count(&self) -> u64 {
        self.line_count
    }

    pub(super) fn visible_line_count(&self) -> u64 {
        self.line_count.saturating_sub(
            self.hidden_ranges
                .last()
                .map_or(0, |(_, cumulative)| *cumulative),
        )
    }

    pub(super) fn visible_ordinal_for_line(&self, line: u64) -> u64 {
        line.min(self.line_count)
            .saturating_sub(self.hidden_before(line.min(self.line_count)))
    }

    pub(super) fn source_line_for_visible_ordinal(&self, ordinal: u64) -> Option<u64> {
        let visible_count = self.visible_line_count();
        if visible_count == 0 {
            return None;
        }
        let target = ordinal.min(visible_count - 1);
        let (mut low, mut high) = (0, self.line_count);
        while low < high {
            let middle = low + (high - low) / 2;
            if self.visible_ordinal_for_line(middle) < target {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        self.hidden_ranges
            .iter()
            .find(|(range, _)| range.contains(&low))
            .map_or(Some(low), |(range, _)| {
                (range.end < self.line_count).then_some(range.end)
            })
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
            let last_chunk = line_count / CHUNK_LINES;
            let last_local = (line_count % CHUNK_LINES) as usize;
            self.measured_heights
                .retain(|chunk, _| *chunk * CHUNK_LINES < line_count);
            if last_local > 0
                && let Some(heights) = self.measured_heights.get_mut(&last_chunk)
            {
                heights[last_local..].fill(0);
            }
            self.rebuild_fenwick();
        }
        true
    }

    fn clear_layout(&mut self) {
        self.measured_heights.clear();
        self.height_fenwick = vec![0; self.line_count.div_ceil(CHUNK_LINES) as usize + 1];
    }

    pub(super) fn invalidate_layout(&mut self) {
        self.clear_layout();
    }

    pub(super) fn set_hidden_ranges(&mut self, ranges: Vec<Range<u64>>) -> bool {
        let mut hidden = 0;
        let hidden_ranges = ranges
            .into_iter()
            .filter(|range| range.start < range.end)
            .map(|range| {
                hidden += range.end - range.start;
                (range, hidden)
            })
            .collect::<Vec<_>>();
        if self.hidden_ranges == hidden_ranges {
            return false;
        }
        self.hidden_ranges = hidden_ranges;
        // Folding changes visibility, not the measured shape of a source line. Keep the sparse
        // measurements so expanding a subtree does not briefly fall back to baseline heights.
        // `rebuild_fenwick` excludes hidden lines from the visible height index.
        self.rebuild_fenwick();
        true
    }

    pub(super) fn is_hidden(&self, line: u64) -> bool {
        self.hidden_ranges
            .iter()
            .any(|(range, _)| range.contains(&line))
    }

    #[cfg(test)]
    pub(super) fn hidden_after(&self, line: u64) -> u64 {
        self.hidden_ranges
            .iter()
            .find(|(range, _)| range.start == line + 1)
            .map_or(0, |(range, _)| range.end - range.start)
    }

    fn hidden_before(&self, line: u64) -> u64 {
        let mut hidden = 0;
        for (range, cumulative) in &self.hidden_ranges {
            if line >= range.end {
                hidden = *cumulative;
            } else {
                hidden += line
                    .saturating_sub(range.start)
                    .min(range.end - range.start);
                break;
            }
        }
        hidden
    }

    pub(super) fn invalidate_layout_from(&mut self, first_line: u64) {
        let first_chunk = first_line / CHUNK_LINES;
        let first_local = (first_line % CHUNK_LINES) as usize;
        if let Some(heights) = self.measured_heights.get_mut(&first_chunk) {
            heights[first_local..].fill(0);
        }
        self.measured_heights.retain(|chunk, heights| {
            *chunk <= first_chunk && heights.iter().any(|height| *height != 0)
        });
        self.rebuild_fenwick();
    }

    /// Drops a single line's measured height without disturbing measurements on nearby lines.
    /// This is used when an editor-only decoration, such as an inline image preview, stops
    /// matching after a same-line text edit.
    pub(super) fn invalidate_line_layout(&mut self, line: u64) -> bool {
        if line >= self.line_count {
            return false;
        }
        let chunk = line / CHUNK_LINES;
        let local = (line % CHUNK_LINES) as usize;
        let Some(heights) = self.measured_heights.get_mut(&chunk) else {
            return false;
        };
        if heights[local] == 0 {
            return false;
        }
        heights[local] = 0;
        if heights.iter().all(|height| *height == 0) {
            self.measured_heights.remove(&chunk);
        }
        self.rebuild_fenwick();
        true
    }

    /// Applies a physical-line splice while retaining measurements whose source lines survive.
    /// The first replaced line keeps its old estimate until the next visible layout pass; rows
    /// after the splice move with their source lines instead of briefly falling back to defaults.
    pub(super) fn splice_lines(&mut self, old_range: Range<u64>, new_count: u64, new_total: u64) {
        let old_start = old_range.start.min(self.line_count);
        let old_end = old_range.end.min(self.line_count).max(old_start);
        let old_count = old_end - old_start;
        let measured = std::mem::take(&mut self.measured_heights);
        self.line_count = new_total;

        for (chunk, heights) in measured {
            let chunk_start = chunk * CHUNK_LINES;
            for (local, height) in heights.into_iter().enumerate() {
                if height == 0 {
                    continue;
                }
                let old_line = chunk_start + local as u64;
                let new_line = if old_line < old_start {
                    Some(old_line)
                } else if old_line == old_start && new_count > 0 {
                    Some(old_start)
                } else if old_line >= old_end {
                    Some(if new_count >= old_count {
                        old_line.saturating_add(new_count - old_count)
                    } else {
                        old_line.saturating_sub(old_count - new_count)
                    })
                } else {
                    None
                };
                let Some(new_line) = new_line.filter(|line| *line < new_total) else {
                    continue;
                };
                let new_chunk = new_line / CHUNK_LINES;
                let new_local = (new_line % CHUNK_LINES) as usize;
                self.measured_heights
                    .entry(new_chunk)
                    .or_insert_with(|| Box::new([0; CHUNK_LINES as usize]))[new_local] = height;
            }
        }
        self.rebuild_fenwick();
    }

    fn rebuild_fenwick(&mut self) {
        self.height_fenwick = vec![0; self.line_count.div_ceil(CHUNK_LINES) as usize + 1];
        let baseline = pixels_to_fixed(self.line_height);
        for (&chunk, heights) in &self.measured_heights {
            let delta = heights
                .iter()
                .enumerate()
                .filter(|(local, _)| {
                    let line = chunk * CHUNK_LINES + *local as u64;
                    line < self.line_count && !line_is_hidden(&self.hidden_ranges, line)
                })
                .map(|(_, height)| i64::from(height.saturating_sub(baseline)))
                .sum::<i64>();
            let mut index = chunk as usize + 1;
            while index < self.height_fenwick.len() {
                self.height_fenwick[index] += delta;
                index += index & index.wrapping_neg();
            }
        }
    }

    pub(super) fn update_line_layout(
        &mut self,
        line: u64,
        row_count: usize,
        line_height: f32,
        before: f32,
        after: f32,
    ) -> bool {
        if line >= self.line_count {
            return false;
        }
        if self.is_hidden(line) {
            return false;
        }
        let row_count = if self.soft_wrap {
            row_count.max(1).min(u16::MAX as usize) as u16
        } else {
            1
        };
        let chunk = line / CHUNK_LINES;
        let local = (line % CHUNK_LINES) as usize;
        let baseline = pixels_to_fixed(self.line_height);
        let total_height = pixels_to_fixed(
            before.max(0.0) + after.max(0.0) + row_count as f32 * line_height.max(self.line_height),
        );
        let old_height = self
            .measured_heights
            .get(&chunk)
            .map_or(baseline, |heights| heights[local].max(baseline));
        let height_changed = old_height != total_height;
        if height_changed && total_height == baseline {
            if let Some(heights) = self.measured_heights.get_mut(&chunk) {
                heights[local] = 0;
                if heights.iter().all(|height| *height == 0) {
                    self.measured_heights.remove(&chunk);
                }
            }
        } else if height_changed {
            self.measured_heights
                .entry(chunk)
                .or_insert_with(|| Box::new([0; CHUNK_LINES as usize]))[local] = total_height;
        }
        if height_changed {
            let delta = i64::from(total_height) - i64::from(old_height);
            let mut index = chunk as usize + 1;
            while index < self.height_fenwick.len() {
                self.height_fenwick[index] += delta;
                index += index & index.wrapping_neg();
            }
        }
        height_changed
    }

    fn extra_height_before_chunk(&self, chunk: usize) -> i64 {
        let mut index = chunk;
        let mut total = 0i64;
        while index > 0 {
            total += self.height_fenwick[index];
            index &= index - 1;
        }
        total
    }

    pub(super) fn line_height_px(&self, line: u64) -> f32 {
        if self.is_hidden(line) {
            return 0.0;
        }
        let chunk = line / CHUNK_LINES;
        let local = (line % CHUNK_LINES) as usize;
        self.measured_heights
            .get(&chunk)
            .map_or(self.line_height, |heights| {
                fixed_to_pixels(heights[local].max(pixels_to_fixed(self.line_height)))
            })
    }

    pub(super) fn line_start_y(&self, line: u64) -> f32 {
        let line = line.min(self.line_count);
        let chunk = line / CHUNK_LINES;
        let local = (line % CHUNK_LINES) as usize;
        let baseline = pixels_to_fixed(self.line_height);
        let local_extra = self.measured_heights.get(&chunk).map_or(0i64, |heights| {
            heights[..local]
                .iter()
                .enumerate()
                .filter(|(local, _)| {
                    !line_is_hidden(&self.hidden_ranges, chunk * CHUNK_LINES + *local as u64)
                })
                .map(|(_, height)| i64::from(height.saturating_sub(baseline)))
                .sum()
        });
        fixed_to_pixels_i64(
            (line - self.hidden_before(line)) as i64 * i64::from(baseline)
                + self.extra_height_before_chunk(chunk as usize)
                + local_extra,
        )
    }

    pub(super) fn total_height(&self) -> f32 {
        self.line_start_y(self.line_count)
    }

    pub(super) fn line_at_y(&self, y: f32) -> u64 {
        if self.line_count == 0 {
            return 0;
        }
        let target = y.max(0.0).min((self.total_height() - 0.01).max(0.0));
        let (mut low, mut high) = (0u64, self.line_count);
        while low < high {
            let middle = low + (high - low) / 2;
            if self.line_start_y(middle + 1) <= target {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low.min(self.line_count - 1)
    }

    #[cfg(test)]
    pub(super) fn visible_line_range(
        &self,
        snapshot: &DocumentSnapshot,
        scroll_y: f32,
        viewport_height: f32,
    ) -> Range<u64> {
        let first = self.line_at_y(scroll_y).min(snapshot.len_lines());
        let last = self
            .line_at_y(scroll_y.max(0.0) + viewport_height.max(0.0))
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

fn line_is_hidden(hidden_ranges: &[(Range<u64>, u64)], line: u64) -> bool {
    hidden_ranges.iter().any(|(range, _)| range.contains(&line))
}

fn pixels_to_fixed(pixels: f32) -> u32 {
    (pixels.max(0.0) * PIXEL_SCALE).round().min(u32::MAX as f32) as u32
}

fn fixed_to_pixels(pixels: u32) -> f32 {
    pixels as f32 / PIXEL_SCALE
}

fn fixed_to_pixels_i64(pixels: i64) -> f32 {
    pixels.max(0) as f32 / PIXEL_SCALE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_height_index_maps_both_directions() {
        let mut map = EditorLayoutMap::default();
        map.configure(1_000_000, 640.0);
        assert!(map.update_line_layout(10, 4, 30.0, 8.0, 4.0));
        assert!(map.update_line_layout(511, 3, 24.0, 2.0, 2.0));
        assert_eq!(map.line_start_y(11), 10.0 * 22.0 + 132.0);
        assert_eq!(map.line_at_y(10.0 * 22.0 + 40.0), 10);
        assert_eq!(map.line_start_y(512), 512.0 * 22.0 + 110.0 + 54.0);
        assert_eq!(map.measured_heights.len(), 2);
    }

    #[test]
    fn editing_late_lines_preserves_measured_prefix() {
        let mut map = EditorLayoutMap::default();
        map.configure(1_000, 640.0);
        map.update_line_layout(10, 4, 30.0, 8.0, 4.0);
        map.update_line_layout(800, 5, 22.0, 0.0, 0.0);
        map.invalidate_layout_from(700);
        assert_eq!(map.line_start_y(11), 10.0 * 22.0 + 132.0);
        assert_eq!(map.line_start_y(801), 801.0 * 22.0 + 110.0);

        map.configure(1_001, 640.0);
        assert_eq!(map.line_start_y(11), 10.0 * 22.0 + 132.0);
    }

    #[test]
    fn invalidating_one_line_restores_its_baseline_without_touching_neighbors() {
        let mut map = EditorLayoutMap::default();
        map.configure(1_000, 640.0);
        map.update_line_layout(66, 1, 240.0, 8.0, 8.0);
        map.update_line_layout(800, 5, 22.0, 0.0, 0.0);
        let before = map.total_height();

        assert!(map.invalidate_line_layout(66));

        assert_eq!(map.line_height_px(66), super::super::LINE_HEIGHT);
        assert_eq!(map.line_height_px(800), 110.0);
        assert_eq!(
            map.total_height(),
            before - (256.0 - super::super::LINE_HEIGHT)
        );
        assert!(!map.invalidate_line_layout(66));
    }

    #[test]
    fn line_splice_moves_downstream_measurements_without_a_default_height_phase() {
        let mut map = EditorLayoutMap::default();
        map.configure(1_000, 640.0);
        map.update_line_layout(500, 3, 24.0, 2.0, 2.0);
        map.update_line_layout(800, 5, 22.0, 0.0, 0.0);
        let before = map.total_height();

        map.splice_lines(500..501, 2, 1_001);

        assert_eq!(map.line_height_px(500), 76.0);
        assert_eq!(map.line_height_px(501), super::super::LINE_HEIGHT);
        assert_eq!(map.line_height_px(801), 110.0);
        assert_eq!(map.total_height(), before + super::super::LINE_HEIGHT);

        map.splice_lines(500..502, 1, 1_000);

        assert_eq!(map.line_height_px(500), 76.0);
        assert_eq!(map.line_height_px(800), 110.0);
        assert_eq!(map.total_height(), before);
    }

    #[test]
    fn shrinking_then_growing_does_not_reuse_deleted_line_measurements() {
        let mut map = EditorLayoutMap::default();
        map.configure(300, 640.0);
        map.update_line_layout(290, 3, 30.0, 8.0, 4.0);
        map.configure(280, 640.0);
        map.configure(300, 640.0);
        assert_eq!(map.line_height_px(290), 22.0);
        assert_eq!(map.total_height(), 300.0 * 22.0);
    }

    #[test]
    fn hidden_ranges_use_zero_height_and_keep_source_mapping() {
        let mut map = EditorLayoutMap::default();
        map.configure(10, 640.0);
        assert!(map.set_hidden_ranges(std::iter::once(2..5).collect()));
        assert_eq!(map.line_start_y(5), 2.0 * 22.0);
        assert_eq!(map.line_at_y(2.0 * 22.0), 5);
        assert_eq!(map.hidden_after(1), 3);
        assert!(!map.set_hidden_ranges(std::iter::once(2..5).collect()));
        assert_eq!(map.visible_line_count(), 7);
        assert_eq!(map.visible_ordinal_for_line(5), 2);
        assert_eq!(map.source_line_for_visible_ordinal(2), Some(5));
    }

    #[test]
    fn folding_preserves_measured_heights_for_expand_without_a_baseline_frame() {
        let mut map = EditorLayoutMap::default();
        map.configure(10, 640.0);
        map.update_line_layout(1, 4, 30.0, 8.0, 4.0);
        map.update_line_layout(5, 3, 24.0, 2.0, 2.0);
        let expanded_height = map.total_height();

        assert!(map.set_hidden_ranges(std::iter::once(1..5).collect()));
        assert_eq!(map.line_height_px(1), 0.0);
        assert_eq!(map.line_start_y(5), super::super::LINE_HEIGHT);
        assert_eq!(map.total_height(), expanded_height - 4.0 * 22.0 - 110.0);

        assert!(map.set_hidden_ranges(Vec::new()));
        assert_eq!(map.line_height_px(1), 132.0);
        assert_eq!(map.line_height_px(5), 76.0);
        assert_eq!(map.total_height(), expanded_height);
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
        let mut map = EditorLayoutMap::default();
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
