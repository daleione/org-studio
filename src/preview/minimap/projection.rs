use std::{
    hash::{Hash, Hasher},
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    document::Revision,
    preview::{
        PreviewStyle,
        layout::{LayoutKey, LayoutSnapshot, ResolvedRow},
    },
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
    pub(super) geometry_revision: u64,
}

impl MinimapLineIndexKey {
    pub(in crate::preview) fn new(
        presentation_rows: &Arc<Vec<usize>>,
        available_width: f32,
        density: MinimapDensity,
        document_revision: Revision,
        geometry_revision: u64,
        zoom: f32,
        style: PreviewStyle,
    ) -> Self {
        Self {
            presentation_rows: Arc::as_ptr(presentation_rows) as usize,
            width: available_width.round().clamp(1.0, u16::MAX as f32) as u16,
            density,
            layout: LayoutKey {
                document_revision,
                content_width_px: available_width.round().clamp(1.0, u16::MAX as f32) as u16,
                text_metrics_revision: style.layout_key() ^ u64::from(zoom.to_bits()),
                fold_revision: geometry_revision,
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
        zoom: f32,
        style: PreviewStyle,
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
            zoom,
            style,
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

    #[allow(clippy::too_many_arguments)]
    pub(super) fn rebased(
        model: &PreviewDisplayMap,
        key: MinimapLineIndexKey,
        presentation_rows: Arc<Vec<usize>>,
        available_width: f32,
        previous_rows: &[usize],
        previous_projection: &LayoutSnapshot,
        zoom: f32,
        style: PreviewStyle,
    ) -> Self {
        let started_at = Instant::now();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        presentation_rows.hash(&mut hasher);
        let rows_signature = hasher.finish();
        let projection = if previous_projection.rows == previous_rows.len() {
            let common_prefix = previous_rows
                .iter()
                .zip(presentation_rows.iter())
                .take_while(|(previous, next)| previous == next)
                .count();
            let max_suffix = previous_rows
                .len()
                .saturating_sub(common_prefix)
                .min(presentation_rows.len().saturating_sub(common_prefix));
            let common_suffix = previous_rows
                .iter()
                .rev()
                .zip(presentation_rows.iter().rev())
                .take(max_suffix)
                .take_while(|(previous, next)| previous == next)
                .count();
            let previous_end = previous_rows.len() - common_suffix;
            let next_end = presentation_rows.len() - common_suffix;
            let mut previous_cursor = common_prefix;
            let replacements = presentation_rows[common_prefix..next_end]
                .iter()
                .enumerate()
                .map(|(offset, row)| {
                    let presentation_index = common_prefix + offset;
                    while previous_cursor < previous_end && previous_rows[previous_cursor] < *row {
                        previous_cursor += 1;
                    }
                    let measure = if previous_cursor < previous_end
                        && previous_rows[previous_cursor] == *row
                    {
                        model.without_presentation_tail_padding(
                            *row,
                            previous_cursor,
                            previous_rows.len(),
                            zoom,
                            style,
                            previous_projection.measure(previous_cursor),
                        )
                    } else {
                        model.estimated_measure(*row, available_width, zoom, style)
                    };
                    model.with_presentation_tail_padding(
                        *row,
                        presentation_index,
                        presentation_rows.len(),
                        zoom,
                        style,
                        measure,
                    )
                })
                .collect();
            Arc::new(previous_projection.replacing_range(common_prefix..previous_end, replacements))
        } else {
            Arc::new(LayoutSnapshot::new(
                presentation_rows
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        let measure = model.estimated_measure(*row, available_width, zoom, style);
                        model.with_presentation_tail_padding(
                            *row,
                            index,
                            presentation_rows.len(),
                            zoom,
                            style,
                            measure,
                        )
                    })
                    .collect(),
            ))
        };
        let mut exact_bits = vec![0u64; presentation_rows.len().div_ceil(64)];
        for (index, measure) in projection
            .chunks
            .iter()
            .flat_map(|chunk| chunk.measures.iter())
            .enumerate()
        {
            if measure.exact {
                exact_bits[index / 64] |= 1u64 << (index % 64);
            }
        }
        let exact_rows = projection.exact_rows;
        Self {
            key,
            projection,
            presentation_rows,
            rows_signature,
            sequential_cursor: 0,
            priority_range: 0..0,
            priority_cursor: 0,
            exact_bits,
            exact_rows,
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
    #[allow(clippy::too_many_arguments)]
    pub(super) fn estimated_minimap_line_index(
        &self,
        presentation_rows: &[usize],
        width: u16,
        rows_signature: u64,
        available_width: f32,
        density: MinimapDensity,
        layout: LayoutKey,
        zoom: f32,
        style: PreviewStyle,
    ) -> MinimapLineIndex {
        let mut measures = Vec::with_capacity(presentation_rows.len());

        for (index, &row) in presentation_rows.iter().enumerate() {
            let measure = self.estimated_measure(row, available_width, zoom, style);
            measures.push(self.with_presentation_tail_padding(
                row,
                index,
                presentation_rows.len(),
                zoom,
                style,
                measure,
            ));
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

    #[allow(clippy::too_many_arguments)]
    pub(super) fn advance_minimap_line_index(
        &self,
        state: &super::MinimapState,
        presentation_rows: &Arc<Vec<usize>>,
        available_width: f32,
        density: MinimapDensity,
        refinement: MinimapRefinement,
        zoom: f32,
        style: PreviewStyle,
        text_system: &gpui::WindowTextSystem,
    ) -> MinimapLineIndexProgress {
        let key = MinimapLineIndexKey::new(
            presentation_rows,
            available_width,
            density,
            self.projection.revision,
            refinement.geometry_revision,
            zoom,
            style,
        );
        let cached = state
            .line_index
            .lock()
            .expect("minimap line index poisoned")
            .as_ref()
            .filter(|cached| {
                Arc::ptr_eq(&cached.presentation_rows, presentation_rows)
                    && cached.index.layout == key.layout
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
            let reusable_build = build.as_ref().and_then(|builder| {
                (builder.key.width == key.width
                    && builder.key.density == key.density
                    && builder.key.layout == key.layout)
                    .then(|| {
                        (
                            builder.presentation_rows.clone(),
                            builder.projection.clone(),
                        )
                    })
            });
            let reusable = reusable_build.or_else(|| {
                state
                    .line_index
                    .lock()
                    .expect("minimap line index poisoned")
                    .as_ref()
                    .filter(|cached| {
                        cached.index.width == key.width
                            && cached.index.density == key.density
                            && cached.index.layout == key.layout
                    })
                    .map(|cached| {
                        (
                            cached.presentation_rows.clone(),
                            cached.index.projection.clone(),
                        )
                    })
            });
            *build = Some(
                if let Some((previous_rows, previous_projection)) = reusable {
                    MinimapLineIndexBuilder::rebased(
                        self,
                        key,
                        presentation_rows.clone(),
                        available_width,
                        &previous_rows,
                        &previous_projection,
                        zoom,
                        style,
                    )
                } else {
                    MinimapLineIndexBuilder::new(
                        self,
                        key,
                        presentation_rows.clone(),
                        available_width,
                        density,
                        zoom,
                        style,
                    )
                },
            );
        }
        if created {
            let builder = build.as_mut().expect("line-index builder initialized");
            if minimap_perf_enabled() {
                eprintln!(
                    "org_studio_minimap_ready readiness={} rows={} exact_rows={} width={} elapsed_ms={:.3} projection_bytes={}",
                    if builder.exact_rows == builder.presentation_rows.len() {
                        "exact"
                    } else if builder.exact_rows == 0 {
                        "estimated"
                    } else {
                        "partially_exact"
                    },
                    builder.presentation_rows.len(),
                    builder.exact_rows,
                    key.width,
                    builder.started_at.elapsed().as_secs_f64() * 1000.0,
                    builder.projection.estimated_heap_bytes(),
                );
            }
            if builder.exact_rows == builder.presentation_rows.len() {
                let builder = build.take().expect("exact line-index builder exists");
                return finish_line_index(state, builder, density);
            }
            return line_index_progress(builder, density);
        }
        if !refinement.allow {
            let builder = build.as_ref().expect("line-index builder initialized");
            return line_index_progress(builder, density);
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
            let lines = self.display_lines(row, available_width, zoom, style, text_system);
            let count = lines.ranges.len().max(1);
            let measure = ResolvedRow::new(count, lines.parent_height, true);
            builder.pending_updates.push((
                projection_row,
                self.with_presentation_tail_padding(
                    row,
                    projection_row,
                    builder.presentation_rows.len(),
                    zoom,
                    style,
                    measure,
                ),
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
                return line_index_progress(builder, density);
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
        finish_line_index(state, builder, density)
    }
}

fn line_index_progress(
    builder: &MinimapLineIndexBuilder,
    density: MinimapDensity,
) -> MinimapLineIndexProgress {
    let readiness = if builder.exact_rows == 0 {
        MinimapProjectionReadiness::Estimated
    } else if builder.exact_rows == builder.presentation_rows.len() {
        MinimapProjectionReadiness::Exact
    } else {
        MinimapProjectionReadiness::PartiallyExact
    };
    MinimapLineIndexProgress {
        index: builder.index(density),
        readiness,
        exact_rows: builder.exact_rows,
    }
}

fn finish_line_index(
    state: &super::MinimapState,
    builder: MinimapLineIndexBuilder,
    density: MinimapDensity,
) -> MinimapLineIndexProgress {
    let exact_rows = builder.presentation_rows.len();
    let index = builder.index(density);
    *state
        .line_index
        .lock()
        .expect("minimap line index poisoned") = Some(CachedMinimapLineIndex {
        presentation_rows: builder.presentation_rows,
        index: index.clone(),
    });
    MinimapLineIndexProgress {
        exact_rows,
        index,
        readiness: MinimapProjectionReadiness::Exact,
    }
}
