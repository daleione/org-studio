use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use super::{
    CachedMinimapLineIndex, MinimapDragSession, MinimapInteractionAnchor, MinimapLineIndexBuilder,
    RasterTileCache, RasterTilePaint, projection::MinimapLineIndexKey,
};
use crate::preview::{PreviewSnapshot, PreviewStyle, projection::VisualPatch};

pub(crate) struct MinimapState {
    pub(crate) line_index: Mutex<Option<CachedMinimapLineIndex>>,
    line_index_history: Mutex<VecDeque<CachedMinimapLineIndex>>,
    pub(crate) line_index_build: Mutex<Option<MinimapLineIndexBuilder>>,
    line_index_build_history: Mutex<VecDeque<MinimapLineIndexBuilder>>,
    pub(crate) raster_tiles: Mutex<RasterTileCache>,
    pub(crate) drag: Arc<Mutex<Option<MinimapDragSession>>>,
    pub(crate) resize_drag: Arc<Mutex<Option<ResizeSession>>>,
    pub(crate) interaction_anchor: Arc<Mutex<Option<MinimapInteractionAnchor>>>,
    pub(crate) retained_style_frame: Mutex<Vec<RasterTilePaint>>,
    pub(crate) retain_style_frame: AtomicBool,
    pub(crate) raster_epoch: AtomicU64,
    pub(crate) raster_prefetch_signature: AtomicU64,
    pub(crate) perf: PerfState,
}

impl MinimapState {
    const RECENT_LINE_INDEX_VARIANTS: usize = 3;

    pub(crate) fn new() -> Self {
        Self {
            line_index: Mutex::new(None),
            line_index_history: Mutex::new(VecDeque::with_capacity(
                Self::RECENT_LINE_INDEX_VARIANTS,
            )),
            line_index_build: Mutex::new(None),
            line_index_build_history: Mutex::new(VecDeque::with_capacity(
                Self::RECENT_LINE_INDEX_VARIANTS,
            )),
            raster_tiles: Mutex::new(RasterTileCache {
                entries: HashMap::with_capacity(RasterTileCache::CAPACITY),
                order: VecDeque::with_capacity(RasterTileCache::CAPACITY),
                in_flight: HashSet::with_capacity(RasterTileCache::CAPACITY),
            }),
            drag: Arc::new(Mutex::new(None)),
            resize_drag: Arc::new(Mutex::new(None)),
            interaction_anchor: Arc::new(Mutex::new(None)),
            retained_style_frame: Mutex::new(Vec::new()),
            retain_style_frame: AtomicBool::new(false),
            raster_epoch: AtomicU64::new(0),
            raster_prefetch_signature: AtomicU64::new(0),
            perf: PerfState::new(),
        }
    }

    pub(crate) fn cancel_interaction(&self) -> bool {
        let was_dragging = self
            .drag
            .lock()
            .expect("minimap drag state poisoned")
            .take()
            .is_some();
        let was_resizing = self
            .resize_drag
            .lock()
            .expect("minimap resize state poisoned")
            .take()
            .is_some();
        self.interaction_anchor
            .lock()
            .expect("minimap anchor poisoned")
            .take();
        was_dragging || was_resizing
    }

    pub(crate) fn invalidate_style(&self) {
        self.raster_epoch.fetch_add(1, Ordering::AcqRel);
        self.raster_prefetch_signature.store(0, Ordering::Release);
        self.cancel_interaction();
        // Keep an in-progress line-index builder across layout-style switches. The projection
        // path parks it by key when the next style renders, then resumes it when the user switches
        // back. Discarding it here made every theme toggle rescan all presentation rows.
        let mut tiles = self
            .raster_tiles
            .lock()
            .expect("minimap raster tile cache poisoned");
        let has_retained_frame = !self
            .retained_style_frame
            .lock()
            .expect("retained minimap frame poisoned")
            .is_empty();
        self.retain_style_frame
            .store(has_retained_frame, Ordering::Release);
        // Raster keys contain paint, wrap, width and density signatures. Preserve completed
        // entries so switching back to a recently used built-in style is instant; the bounded
        // cache evicts stale variants naturally. Only in-flight work belongs to the old epoch.
        tiles.in_flight.clear();
    }

    pub(crate) fn cached_line_index(
        &self,
        presentation_rows: &Arc<Vec<usize>>,
        layout: crate::preview::layout::LayoutKey,
        density: super::MinimapDensity,
        minimap_width: u16,
    ) -> Option<super::MinimapLineIndex> {
        self.line_index
            .lock()
            .expect("minimap line index poisoned")
            .as_ref()
            .filter(|cached| {
                Arc::ptr_eq(&cached.presentation_rows, presentation_rows)
                    && cached.index.layout == layout
                    && cached.index.density == density
                    && cached.index.minimap_width == minimap_width
            })
            .map(|cached| cached.index.clone())
            .or_else(|| {
                self.line_index_history
                    .lock()
                    .expect("minimap line-index history poisoned")
                    .iter()
                    .rev()
                    .find(|cached| {
                        Arc::ptr_eq(&cached.presentation_rows, presentation_rows)
                            && cached.index.layout == layout
                            && cached.index.density == density
                            && cached.index.minimap_width == minimap_width
                    })
                    .map(|cached| cached.index.clone())
            })
    }

    pub(crate) fn publish_line_index(&self, cached: CachedMinimapLineIndex) {
        let previous = self
            .line_index
            .lock()
            .expect("minimap line index poisoned")
            .replace(cached);
        let Some(previous) = previous else {
            return;
        };
        let mut history = self
            .line_index_history
            .lock()
            .expect("minimap line-index history poisoned");
        if let Some(position) = history.iter().position(|cached| {
            Arc::ptr_eq(&cached.presentation_rows, &previous.presentation_rows)
                && cached.index.layout == previous.index.layout
                && cached.index.density == previous.index.density
        }) {
            history.remove(position);
        }
        if history.len() >= Self::RECENT_LINE_INDEX_VARIANTS {
            history.pop_front();
        }
        history.push_back(previous);
    }

    pub(crate) fn remember_line_index_builder(&self, builder: MinimapLineIndexBuilder) {
        let mut history = self
            .line_index_build_history
            .lock()
            .expect("minimap line-index builder history poisoned");
        if let Some(position) = history
            .iter()
            .position(|candidate| candidate.key == builder.key)
        {
            history.remove(position);
        }
        if history.len() >= Self::RECENT_LINE_INDEX_VARIANTS {
            history.pop_front();
        }
        history.push_back(builder);
    }

    pub(crate) fn take_line_index_builder(
        &self,
        key: MinimapLineIndexKey,
    ) -> Option<MinimapLineIndexBuilder> {
        let mut history = self
            .line_index_build_history
            .lock()
            .expect("minimap line-index builder history poisoned");
        let position = history.iter().position(|builder| builder.key == key)?;
        history.remove(position)
    }

    pub(crate) fn apply_document_patch(
        &self,
        document: &PreviewSnapshot,
        patch: &VisualPatch,
        presentation_rows: &Arc<Vec<usize>>,
        zoom: f32,
        style: PreviewStyle,
    ) {
        self.raster_prefetch_signature.store(0, Ordering::Release);
        *self
            .line_index_build
            .lock()
            .expect("minimap line-index builder poisoned") = None;
        self.line_index_build_history
            .lock()
            .expect("minimap line-index builder history poisoned")
            .clear();
        let mut cached = self.line_index.lock().expect("minimap line index poisoned");
        let Some(index) = cached.as_mut() else {
            return;
        };
        if !Arc::ptr_eq(&index.presentation_rows, presentation_rows)
            || patch.old_visual != patch.new_visual
        {
            *cached = None;
            self.line_index_history
                .lock()
                .expect("minimap line-index history poisoned")
                .clear();
            return;
        }
        if !patch
            .invalidation
            .contains(crate::preview::projection::InvalidationFlags::GEOMETRY)
        {
            // Paint-only edits (notably Checkbox state changes) retain every
            // exact row measure. Replacing them with estimates makes the thumb
            // move once for the estimate and again when shaping converges.
            index.index.layout.document_revision = document.revision;
            index.presentation_rows = presentation_rows.clone();
            for cached in self
                .line_index_history
                .lock()
                .expect("minimap line-index history poisoned")
                .iter_mut()
            {
                if Arc::ptr_eq(&cached.presentation_rows, presentation_rows) {
                    cached.index.layout.document_revision = document.revision;
                }
            }
            return;
        }
        self.line_index_history
            .lock()
            .expect("minimap line-index history poisoned")
            .clear();
        let Some(display_map) = document.display_map.as_deref() else {
            *cached = None;
            return;
        };
        let start = presentation_rows.partition_point(|row| *row < patch.new_visual.start);
        let end = presentation_rows.partition_point(|row| *row < patch.new_visual.end);
        let replacements = presentation_rows[start..end]
            .iter()
            .enumerate()
            .map(|(offset, row)| {
                let presentation_index = start + offset;
                let measure = display_map.minimap_measure(
                    *row,
                    display_map.estimated_measure(*row, index.index.width as f32, zoom, style),
                    f32::from(index.index.minimap_width),
                    index.index.density,
                    zoom,
                    style,
                );
                (
                    presentation_index,
                    display_map.with_presentation_tail_padding(
                        *row,
                        presentation_index,
                        presentation_rows.len(),
                        zoom,
                        style,
                        measure,
                    ),
                )
            })
            .collect::<Vec<_>>();
        index.index.projection = Arc::new(index.index.projection.replacing(&replacements));
        index.index.layout.document_revision = document.revision;
        index.index.total = index.index.projection.total_display_lines();
        index.presentation_rows = presentation_rows.clone();
    }
}

pub(crate) struct PerfState {
    pub(crate) first_tile_completed: AtomicBool,
    pub(crate) first_pixels_painted: AtomicBool,
}

impl PerfState {
    fn new() -> Self {
        Self {
            first_tile_completed: AtomicBool::new(false),
            first_pixels_painted: AtomicBool::new(false),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResizeSession {
    pub(crate) start_pointer_x: f32,
    pub(crate) start_width: f32,
}

pub(crate) fn current_resize_session(
    state: &Mutex<Option<ResizeSession>>,
) -> Option<ResizeSession> {
    *state.lock().expect("minimap resize state poisoned")
}

pub(crate) fn take_resize_session(state: &Mutex<Option<ResizeSession>>) -> Option<ResizeSession> {
    state.lock().expect("minimap resize state poisoned").take()
}
