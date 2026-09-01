use std::{ops::Range, sync::Arc};

use crate::document::Revision;

const LAYOUT_CHUNK_ROWS: usize = 256;
const READING_MIN_CONTENT_WIDTH: f32 = 120.0;

pub(in crate::preview) fn reading_content_width(
    pane_width: f32,
    minimap_width: f32,
    style: super::style::PreviewStyle,
) -> f32 {
    (reading_frame_width(pane_width, minimap_width, style) - style.spacing.horizontal_padding)
        .max(READING_MIN_CONTENT_WIDTH)
}

pub(in crate::preview) fn reading_frame_width(
    pane_width: f32,
    minimap_width: f32,
    style: super::style::PreviewStyle,
) -> f32 {
    let available = (pane_width - minimap_width).max(0.0);
    let fluid = available * style.spacing.wide_pane_fill;
    let readable_floor = available.min(style.spacing.content_min_width);
    fluid
        .max(readable_floor)
        .min(style.spacing.content_max_width)
        .min(available)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::preview) struct ResolvedRow {
    pub(in crate::preview) display_lines: u32,
    pub(in crate::preview) pixels: f32,
    pub(in crate::preview) exact: bool,
}

#[derive(Clone)]
pub(in crate::preview) struct LayoutChunk {
    pub(in crate::preview) measures: Arc<[ResolvedRow]>,
    pub(in crate::preview) display_lines: usize,
    pub(in crate::preview) pixels: f32,
    pub(in crate::preview) exact_rows: usize,
}

#[derive(Clone)]
pub(in crate::preview) struct LayoutSnapshot {
    pub(in crate::preview) chunks: Arc<[Arc<LayoutChunk>]>,
    row_prefix: Arc<[usize]>,
    display_prefix: Arc<[usize]>,
    pixel_prefix: Arc<[f32]>,
    pub(in crate::preview) rows: usize,
    pub(in crate::preview) exact_rows: usize,
}

impl ResolvedRow {
    pub(in crate::preview) fn new(display_lines: usize, pixels: f32, exact: bool) -> Self {
        Self {
            display_lines: display_lines.max(1).min(u32::MAX as usize) as u32,
            pixels: pixels.max(0.0),
            exact,
        }
    }
}

impl LayoutChunk {
    fn new(measures: Vec<ResolvedRow>) -> Self {
        let display_lines = measures.iter().map(|row| row.display_lines as usize).sum();
        let pixels = measures.iter().map(|row| row.pixels).sum();
        let exact_rows = measures.iter().filter(|row| row.exact).count();
        Self {
            measures: measures.into(),
            display_lines,
            pixels,
            exact_rows,
        }
    }
}

impl LayoutSnapshot {
    pub(in crate::preview) fn new(measures: Vec<ResolvedRow>) -> Self {
        Self::from_chunks(
            measures
                .chunks(LAYOUT_CHUNK_ROWS)
                .map(|rows| Arc::new(LayoutChunk::new(rows.to_vec())))
                .collect(),
        )
    }

    fn from_chunks(chunks: Vec<Arc<LayoutChunk>>) -> Self {
        let mut row_prefix = vec![0usize];
        let mut display_prefix = vec![0usize];
        let mut pixel_prefix = vec![0.0f32];
        let mut rows = 0usize;
        let mut exact_rows = 0usize;
        for chunk in &chunks {
            rows += chunk.measures.len();
            exact_rows += chunk.exact_rows;
            row_prefix.push(rows);
            display_prefix.push(
                display_prefix
                    .last()
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(chunk.display_lines),
            );
            pixel_prefix.push(pixel_prefix.last().copied().unwrap_or(0.0) + chunk.pixels);
        }
        Self {
            chunks: chunks.into(),
            row_prefix: row_prefix.into(),
            display_prefix: display_prefix.into(),
            pixel_prefix: pixel_prefix.into(),
            rows,
            exact_rows,
        }
    }

    pub(in crate::preview) fn replacing(&self, updates: &[(usize, ResolvedRow)]) -> Self {
        if updates.is_empty() {
            return self.clone();
        }
        let mut chunks = self.chunks.to_vec();
        let mut cursor = 0usize;
        while cursor < updates.len() {
            let chunk_index = self.chunk_for_row(updates[cursor].0);
            if chunk_index >= chunks.len() {
                break;
            }
            let mut rows = chunks[chunk_index].measures.to_vec();
            while cursor < updates.len() && self.chunk_for_row(updates[cursor].0) == chunk_index {
                let local = updates[cursor].0 - self.row_prefix[chunk_index];
                if local < rows.len() {
                    rows[local] = updates[cursor].1;
                }
                cursor += 1;
            }
            chunks[chunk_index] = Arc::new(LayoutChunk::new(rows));
        }
        Self::from_chunks(chunks)
    }

    pub(in crate::preview) fn replacing_range(
        &self,
        range: Range<usize>,
        replacements: Vec<ResolvedRow>,
    ) -> Self {
        assert!(range.start <= range.end && range.end <= self.rows);
        let (start_chunk, start_local) = self.locate_boundary(range.start);
        let (end_chunk, end_local) = self.locate_boundary(range.end);
        let mut chunks = self.chunks[..start_chunk].to_vec();
        let mut middle = Vec::new();
        if let Some(chunk) = self.chunks.get(start_chunk) {
            middle.extend_from_slice(&chunk.measures[..start_local]);
        }
        middle.extend(replacements);
        let after_start = if let Some(chunk) = self.chunks.get(end_chunk) {
            if end_chunk == start_chunk || end_local > 0 {
                middle.extend_from_slice(&chunk.measures[end_local..]);
                end_chunk + 1
            } else {
                end_chunk
            }
        } else {
            self.chunks.len()
        };
        chunks.extend(
            middle
                .chunks(LAYOUT_CHUNK_ROWS)
                .filter(|rows| !rows.is_empty())
                .map(|rows| Arc::new(LayoutChunk::new(rows.to_vec()))),
        );
        chunks.extend_from_slice(&self.chunks[after_start..]);
        Self::from_chunks(chunks)
    }

    fn chunk_for_row(&self, row: usize) -> usize {
        self.row_prefix
            .partition_point(|&start| start <= row)
            .saturating_sub(1)
            .min(self.chunks.len())
    }

    fn locate_boundary(&self, row: usize) -> (usize, usize) {
        if row >= self.rows {
            return (self.chunks.len(), 0);
        }
        let chunk = self.chunk_for_row(row);
        (chunk, row - self.row_prefix[chunk])
    }

    pub(in crate::preview) fn total_display_lines(&self) -> usize {
        self.display_prefix.last().copied().unwrap_or(0)
    }
    pub(in crate::preview) fn total_pixels(&self) -> f32 {
        self.pixel_prefix.last().copied().unwrap_or(0.0)
    }
    pub(in crate::preview) fn estimated_heap_bytes(&self) -> usize {
        self.rows * std::mem::size_of::<ResolvedRow>()
            + self.chunks.len() * std::mem::size_of::<Arc<LayoutChunk>>()
            + self.row_prefix.len() * std::mem::size_of::<usize>()
            + self.display_prefix.len() * std::mem::size_of::<usize>()
            + self.pixel_prefix.len() * std::mem::size_of::<f32>()
    }

    pub(in crate::preview) fn prefix_for_row(&self, row: usize) -> (usize, f32) {
        if self.rows == 0 {
            return (0, 0.0);
        }
        let row = row.min(self.rows);
        if row == self.rows {
            return (self.total_display_lines(), self.total_pixels());
        }
        let chunk = self.chunk_for_row(row);
        let mut display = self.display_prefix[chunk];
        let mut pixels = self.pixel_prefix[chunk];
        for measure in self.chunks[chunk]
            .measures
            .iter()
            .take(row - self.row_prefix[chunk])
        {
            display = display.saturating_add(measure.display_lines as usize);
            pixels += measure.pixels;
        }
        (display, pixels)
    }

    pub(in crate::preview) fn measure(&self, row: usize) -> ResolvedRow {
        let chunk = self.chunk_for_row(row);
        self.chunks[chunk].measures[row - self.row_prefix[chunk]]
    }

    pub(in crate::preview) fn locate_display(&self, display_line: usize) -> (usize, usize) {
        if self.rows == 0 {
            return (0, 0);
        }
        let chunk = self
            .display_prefix
            .partition_point(|&start| start <= display_line)
            .saturating_sub(1)
            .min(self.chunks.len() - 1);
        let mut remaining = display_line.saturating_sub(self.display_prefix[chunk]);
        for (local, measure) in self.chunks[chunk].measures.iter().enumerate() {
            let count = measure.display_lines as usize;
            if remaining < count {
                return (self.row_prefix[chunk] + local, remaining);
            }
            remaining = remaining.saturating_sub(count);
        }
        (
            self.rows - 1,
            self.measure(self.rows - 1).display_lines as usize,
        )
    }

    pub(in crate::preview) fn locate_pixel(&self, pixel: f32) -> (usize, f32) {
        if self.rows == 0 {
            return (0, 0.0);
        }
        let pixel = pixel.clamp(0.0, self.total_pixels());
        let chunk = self
            .pixel_prefix
            .partition_point(|&start| start <= pixel)
            .saturating_sub(1)
            .min(self.chunks.len() - 1);
        let mut remaining = (pixel - self.pixel_prefix[chunk]).max(0.0);
        let mut last_nonzero = None;
        for (local, measure) in self.chunks[chunk].measures.iter().enumerate() {
            // Zero-height semantic rows (for example a Markdown closing fence in Reading)
            // occupy no pixel interval. Returning one here would make every later pixel in the
            // chunk resolve to that row until the next chunk boundary.
            if measure.pixels <= f32::EPSILON {
                continue;
            }
            last_nonzero = Some((self.row_prefix[chunk] + local, measure.pixels));
            if remaining < measure.pixels {
                return (self.row_prefix[chunk] + local, remaining);
            }
            remaining -= measure.pixels;
        }
        if let Some(last_nonzero) = last_nonzero {
            return last_nonzero;
        }
        let last = self.rows - 1;
        (last, self.measure(last).pixels)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::preview) struct LayoutKey {
    pub(in crate::preview) document_revision: Revision,
    pub(in crate::preview) content_width_px: u16,
    pub(in crate::preview) text_metrics_revision: u64,
    pub(in crate::preview) fold_revision: u64,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn reading_content_width_matches_the_rendered_frame_and_padding() {
        let style = *super::super::preview_style(super::super::PreviewStyleId::Base);
        assert_eq!(reading_content_width(2_000.0, 0.0, style), 912.0);
        assert_eq!(reading_content_width(800.0, 100.0, style), 652.0);
        assert_eq!(reading_content_width(140.0, 0.0, style), 120.0);

        let warm = *super::super::preview_style(super::super::PreviewStyleId::WarmClay);
        assert_eq!(reading_frame_width(700.0, 0.0, warm), 700.0);
        assert_eq!(reading_frame_width(1_000.0, 0.0, warm), 780.0);
        assert_eq!(reading_frame_width(2_000.0, 0.0, warm), 1120.0);
        assert_eq!(reading_content_width(2_000.0, 0.0, warm), 1088.0);
    }

    #[test]
    fn local_geometry_patch_reuses_more_than_ninety_nine_percent_of_large_layout_chunks() {
        let rows = 256 * 500;
        let snapshot =
            LayoutSnapshot::new((0..rows).map(|_| ResolvedRow::new(1, 24.0, true)).collect());
        let updated =
            snapshot.replacing_range(64_000..64_001, vec![ResolvedRow::new(2, 48.0, false)]);
        let shared = snapshot
            .chunks
            .iter()
            .zip(updated.chunks.iter())
            .filter(|(old, new)| Arc::ptr_eq(old, new))
            .count();
        assert!(shared as f32 / snapshot.chunks.len() as f32 >= 0.99);
        assert_eq!(updated.measure(64_000).display_lines, 2);
    }

    #[test]
    fn pixel_lookup_skips_zero_height_rows_in_the_middle_of_a_chunk() {
        let snapshot = LayoutSnapshot::new(vec![
            ResolvedRow::new(1, 24.0, true),
            ResolvedRow::new(1, 0.0, true),
            ResolvedRow::new(1, 40.0, true),
        ]);

        assert_eq!(snapshot.locate_pixel(23.0), (0, 23.0));
        assert_eq!(snapshot.locate_pixel(24.0), (2, 0.0));
        assert_eq!(snapshot.locate_pixel(50.0), (2, 26.0));

        let trailing_zero = LayoutSnapshot::new(vec![
            ResolvedRow::new(1, 24.0, true),
            ResolvedRow::new(1, 0.0, true),
        ]);
        assert_eq!(trailing_zero.locate_pixel(24.0), (0, 24.0));
    }
}
