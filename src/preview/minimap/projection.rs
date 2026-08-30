use std::{
    hash::{Hash, Hasher},
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    document::Revision,
    preview::layout::{LayoutKey, LayoutSnapshot, ResolvedRow},
};
use gpui::{ListOffset, px};

use super::{
    MINIMAP_INDEX_FRAME_BUDGET, PreviewDisplayMap, RASTER_TILE_ROWS, minimap_perf_enabled,
    minimap_trace_enabled, width::Density as MinimapDensity,
};

#[derive(Clone)]
pub(in crate::preview) struct MinimapLineIndex {
    pub(in crate::preview) layout: LayoutKey,
    pub(in crate::preview) width: u16,
    pub(in crate::preview) rows_signature: u64,
    pub(in crate::preview) density: MinimapDensity,
    pub(in crate::preview) projection: Arc<LayoutSnapshot>,
    pub(in crate::preview) total: usize,
}

#[derive(Clone)]
pub(in crate::preview) struct CachedMinimapLineIndex {
    pub(in crate::preview) presentation_rows: Arc<Vec<usize>>,
    pub(in crate::preview) index: MinimapLineIndex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::preview) struct MinimapLineIndexKey {
    pub(in crate::preview) presentation_rows: usize,
    pub(in crate::preview) width: u16,
    pub(in crate::preview) density: MinimapDensity,
    pub(in crate::preview) layout: LayoutKey,
}

pub(in crate::preview) struct MinimapLineIndexBuilder {
    pub(in crate::preview) key: MinimapLineIndexKey,
    pub(in crate::preview) presentation_rows: Arc<Vec<usize>>,
    pub(in crate::preview) rows_signature: u64,
    pub(in crate::preview) sequential_cursor: usize,
    pub(in crate::preview) priority_range: Range<usize>,
    pub(in crate::preview) priority_cursor: usize,
    pub(in crate::preview) exact_bits: Vec<u64>,
    pub(in crate::preview) exact_rows: usize,
    pub(in crate::preview) started_at: Instant,
    pub(in crate::preview) projection: Arc<LayoutSnapshot>,
    pub(in crate::preview) pending_updates: Vec<(usize, ResolvedRow)>,
    pub(in crate::preview) slices: usize,
    pub(in crate::preview) work: Duration,
    pub(in crate::preview) max_slice: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::preview) enum MinimapProjectionReadiness {
    Estimated,
    PartiallyExact,
    Exact,
}

pub(in crate::preview) struct MinimapLineIndexProgress {
    pub(in crate::preview) index: MinimapLineIndex,
    pub(in crate::preview) readiness: MinimapProjectionReadiness,
    pub(in crate::preview) exact_rows: usize,
}

pub(super) struct MinimapRefinement {
    pub(super) priority_row: usize,
    pub(super) allow: bool,
    pub(super) fold_revision: u64,
}

impl MinimapLineIndexKey {
    pub(in crate::preview) fn new(
        presentation_rows: &Arc<Vec<usize>>,
        available_width: f32,
        density: MinimapDensity,
        document_revision: Revision,
        fold_revision: u64,
    ) -> Self {
        Self {
            presentation_rows: Arc::as_ptr(presentation_rows) as usize,
            width: available_width.round().clamp(1.0, u16::MAX as f32) as u16,
            density,
            layout: LayoutKey {
                document_revision,
                content_width_px: available_width.round().clamp(1.0, u16::MAX as f32) as u16,
                text_metrics_revision: 0,
                fold_revision,
            },
        }
    }
}

impl MinimapLineIndexBuilder {
    pub(in crate::preview) fn new(
        model: &PreviewDisplayMap,
        key: MinimapLineIndexKey,
        presentation_rows: Arc<Vec<usize>>,
        available_width: f32,
        density: MinimapDensity,
    ) -> MinimapLineIndexBuilder {
        let started_at = Instant::now();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        presentation_rows.hash(&mut hasher);
        let rows_signature = hasher.finish();
        let estimated_index = model.estimated_minimap_line_index(
            &presentation_rows,
            key.width,
            rows_signature,
            available_width,
            density,
            key.layout,
        );
        Self {
            key,
            projection: estimated_index.projection,
            presentation_rows,
            rows_signature,
            sequential_cursor: 0,
            priority_range: 0..0,
            priority_cursor: 0,
            exact_bits: vec![0; key.presentation_rows.div_ceil(64)],
            exact_rows: 0,
            started_at,
            pending_updates: Vec::with_capacity(16),
            slices: 0,
            work: Duration::ZERO,
            max_slice: Duration::ZERO,
        }
    }

    pub(in crate::preview) fn record_slice(&mut self, elapsed: Duration) {
        self.slices += 1;
        self.work += elapsed;
        self.max_slice = self.max_slice.max(elapsed);
    }

    pub(in crate::preview) fn publish_pending(&mut self) {
        if self.pending_updates.is_empty() {
            return;
        }
        self.projection = Arc::new(self.projection.replacing(&self.pending_updates));
        self.pending_updates.clear();
    }

    pub(in crate::preview) fn index(&self, density: MinimapDensity) -> MinimapLineIndex {
        MinimapLineIndex {
            layout: self.key.layout,
            width: self.key.width,
            rows_signature: self.rows_signature,
            density,
            total: self.projection.total_display_lines(),
            projection: self.projection.clone(),
        }
    }

    pub(in crate::preview) fn is_exact(&self, row: usize) -> bool {
        self.exact_bits
            .get(row / 64)
            .is_some_and(|bits| bits & (1u64 << (row % 64)) != 0)
    }

    pub(in crate::preview) fn mark_exact(&mut self, row: usize) {
        let bit = 1u64 << (row % 64);
        let word = &mut self.exact_bits[row / 64];
        if *word & bit == 0 {
            *word |= bit;
            self.exact_rows += 1;
        }
    }

    pub(in crate::preview) fn prioritize(&mut self, center: usize) {
        if self.presentation_rows.is_empty() {
            return;
        }
        let center = center.min(self.presentation_rows.len() - 1);
        if self.priority_range.contains(&center) {
            return;
        }
        let start = center.saturating_sub(RASTER_TILE_ROWS);
        let end = (center + RASTER_TILE_ROWS * 2).min(self.presentation_rows.len());
        self.priority_range = start..end;
        self.priority_cursor = start;
    }

    pub(in crate::preview) fn next_candidate(&mut self) -> Option<usize> {
        while self.priority_cursor < self.priority_range.end {
            let row = self.priority_cursor;
            self.priority_cursor += 1;
            if !self.is_exact(row) {
                return Some(row);
            }
        }
        while self.sequential_cursor < self.presentation_rows.len() {
            let row = self.sequential_cursor;
            self.sequential_cursor += 1;
            if !self.is_exact(row) {
                return Some(row);
            }
        }
        None
    }
}

impl MinimapLineIndex {
    pub(in crate::preview) fn locate(&self, display_line: usize) -> (usize, usize) {
        self.projection.locate_display(display_line)
    }

    pub(in crate::preview) fn pixel_for_list_offset(&self, offset: ListOffset) -> f32 {
        if self.projection.rows == 0 {
            return 0.0;
        }
        let row = offset.item_ix.min(self.projection.rows - 1);
        let (_, pixel_start) = self.projection.prefix_for_row(row);
        let row_height = self.projection.measure(row).pixels;
        (pixel_start + f32::from(offset.offset_in_item).clamp(0.0, row_height))
            .clamp(0.0, self.projection.total_pixels())
    }

    pub(in crate::preview) fn list_offset_for_display_position(&self, position: f32) -> ListOffset {
        if self.projection.rows == 0 {
            return ListOffset::default();
        }
        let position = position.clamp(0.0, self.total as f32);
        let (row, _) = self.locate(position.floor() as usize);
        let (display_start, _) = self.projection.prefix_for_row(row);
        let measure = self.projection.measure(row);
        let display_count = measure.display_lines.max(1) as f32;
        let pixel_height = measure.pixels;
        ListOffset {
            item_ix: row,
            offset_in_item: px(
                ((position - display_start as f32) / display_count).clamp(0.0, 1.0) * pixel_height,
            ),
        }
    }

    pub(in crate::preview) fn display_position_for_pixel(&self, pixel: f32) -> f32 {
        if self.projection.rows == 0 {
            return 0.0;
        }
        let document_pixels = self.projection.total_pixels();
        let pixel = pixel.clamp(0.0, document_pixels);
        let (row, pixel_in_row) = self.projection.locate_pixel(pixel);
        let (display_start, _) = self.projection.prefix_for_row(row);
        let measure = self.projection.measure(row);
        display_start as f32
            + (pixel_in_row / measure.pixels.max(1.0)).clamp(0.0, 1.0)
                * measure.display_lines.max(1) as f32
    }

    pub(in crate::preview) fn list_offset_for_pixel(&self, pixel: f32) -> ListOffset {
        if self.projection.rows == 0 {
            return ListOffset::default();
        }
        let (row, pixel_in_row) = self
            .projection
            .locate_pixel(pixel.clamp(0.0, self.projection.total_pixels()));
        ListOffset {
            item_ix: row,
            offset_in_item: px(pixel_in_row),
        }
    }

    pub(in crate::preview) fn document_pixels(&self) -> f32 {
        self.projection.total_pixels()
    }
}

impl PreviewDisplayMap {
    pub(super) fn estimated_minimap_line_index(
        &self,
        presentation_rows: &[usize],
        width: u16,
        rows_signature: u64,
        available_width: f32,
        density: MinimapDensity,
        layout: LayoutKey,
    ) -> MinimapLineIndex {
        let mut measures = Vec::with_capacity(presentation_rows.len());

        for &row in presentation_rows {
            measures.push(self.estimated_measure(row, available_width));
        }

        let projection = Arc::new(LayoutSnapshot::new(measures));
        MinimapLineIndex {
            layout,
            width,
            rows_signature,
            density,
            total: projection.total_display_lines(),
            projection,
        }
    }

    pub(super) fn advance_minimap_line_index(
        &self,
        state: &super::MinimapState,
        presentation_rows: &Arc<Vec<usize>>,
        available_width: f32,
        density: MinimapDensity,
        refinement: MinimapRefinement,
        text_system: &gpui::WindowTextSystem,
    ) -> MinimapLineIndexProgress {
        let key = MinimapLineIndexKey::new(
            presentation_rows,
            available_width,
            density,
            self.projection.revision,
            refinement.fold_revision,
        );
        let cached = state
            .line_index
            .lock()
            .expect("minimap line index poisoned")
            .as_ref()
            .filter(|cached| {
                Arc::ptr_eq(&cached.presentation_rows, presentation_rows)
                    && cached.index.width == key.width
                    && cached.index.density == density
            })
            .map(|cached| cached.index.clone());
        if let Some(index) = cached {
            return MinimapLineIndexProgress {
                exact_rows: presentation_rows.len(),
                index,
                readiness: MinimapProjectionReadiness::Exact,
            };
        }
        let mut build = state
            .line_index_build
            .lock()
            .expect("minimap line-index builder poisoned");
        let created = build.as_ref().is_none_or(|builder| builder.key != key);
        if created {
            *build = Some(MinimapLineIndexBuilder::new(
                self,
                key,
                presentation_rows.clone(),
                available_width,
                density,
            ));
        }
        if created {
            let builder = build.as_mut().expect("line-index builder initialized");
            if minimap_perf_enabled() {
                eprintln!(
                    "org_studio_minimap_ready readiness=estimated rows={} exact_rows=0 width={} elapsed_ms={:.3} projection_bytes={}",
                    builder.presentation_rows.len(),
                    key.width,
                    builder.started_at.elapsed().as_secs_f64() * 1000.0,
                    builder.projection.estimated_heap_bytes(),
                );
            }
            return MinimapLineIndexProgress {
                index: builder.index(density),
                readiness: MinimapProjectionReadiness::Estimated,
                exact_rows: 0,
            };
        }
        if !refinement.allow {
            let builder = build.as_ref().expect("line-index builder initialized");
            return MinimapLineIndexProgress {
                index: builder.index(density),
                readiness: if builder.exact_rows == 0 {
                    MinimapProjectionReadiness::Estimated
                } else {
                    MinimapProjectionReadiness::PartiallyExact
                },
                exact_rows: builder.exact_rows,
            };
        }
        let started = Instant::now();
        let builder = build.as_mut().expect("line-index builder initialized");
        builder.prioritize(refinement.priority_row);
        let mut processed = 0usize;
        while builder.exact_rows < builder.presentation_rows.len() {
            let Some(projection_row) = builder.next_candidate() else {
                break;
            };
            let row = builder.presentation_rows[projection_row];
            let lines = self.display_lines(row, available_width, text_system);
            let count = lines.ranges.len().max(1);
            builder.pending_updates.push((
                projection_row,
                ResolvedRow::new(count, lines.parent_height, true),
            ));
            builder.mark_exact(projection_row);
            processed += 1;
            if processed.is_multiple_of(4) && started.elapsed() >= MINIMAP_INDEX_FRAME_BUDGET {
                let elapsed = started.elapsed();
                builder.record_slice(elapsed);
                builder
                    .pending_updates
                    .sort_unstable_by_key(|update| update.0);
                let batch_rows = builder.pending_updates.len();
                let publish_started = Instant::now();
                builder.publish_pending();
                let publish_elapsed = publish_started.elapsed();
                if minimap_trace_enabled() || builder.projection.exact_rows == batch_rows {
                    eprintln!(
                        "org_studio_minimap_projection_publish readiness=partially_exact exact_rows={} rows={} batch_rows={} commit_ms={:.3}",
                        builder.projection.exact_rows,
                        builder.presentation_rows.len(),
                        batch_rows,
                        publish_elapsed.as_secs_f64() * 1000.0,
                    );
                }
                if minimap_trace_enabled() {
                    eprintln!(
                        "org_studio_minimap_index_slice rows_done={} rows_total={} slice_ms={:.3}",
                        builder.exact_rows,
                        builder.presentation_rows.len(),
                        elapsed.as_secs_f64() * 1000.0,
                    );
                }
                return MinimapLineIndexProgress {
                    index: builder.index(density),
                    readiness: MinimapProjectionReadiness::PartiallyExact,
                    exact_rows: builder.exact_rows,
                };
            }
        }
        builder.record_slice(started.elapsed());
        let builder = build.as_mut().expect("completed line-index builder");
        builder
            .pending_updates
            .sort_unstable_by_key(|update| update.0);
        builder.publish_pending();
        let builder = build.take().expect("completed line-index builder");
        if minimap_perf_enabled() {
            eprintln!(
                "org_studio_minimap_index_ready rows={} width={} density={:?} elapsed_ms={:.3} work_ms={:.3} max_slice_ms={:.3} slices={}",
                builder.presentation_rows.len(),
                key.width,
                density,
                builder.started_at.elapsed().as_secs_f64() * 1000.0,
                builder.work.as_secs_f64() * 1000.0,
                builder.max_slice.as_secs_f64() * 1000.0,
                builder.slices,
            );
        }
        let index = builder.index(density);
        *state
            .line_index
            .lock()
            .expect("minimap line index poisoned") = Some(CachedMinimapLineIndex {
            presentation_rows: builder.presentation_rows,
            index: index.clone(),
        });
        MinimapLineIndexProgress {
            exact_rows: presentation_rows.len(),
            index,
            readiness: MinimapProjectionReadiness::Exact,
        }
    }
}
