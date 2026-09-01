use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use super::{
    CachedMinimapLineIndex, MinimapDragSession, MinimapInteractionAnchor, MinimapLineIndexBuilder,
    RasterTileCache, RasterTilePaint,
};
use crate::preview::{PreviewSnapshot, PreviewStyle, projection::VisualPatch};

pub(in crate::preview) struct MinimapState {
    pub(in crate::preview) line_index: Mutex<Option<CachedMinimapLineIndex>>,
    pub(in crate::preview) line_index_build: Mutex<Option<MinimapLineIndexBuilder>>,
    pub(in crate::preview) raster_tiles: Mutex<RasterTileCache>,
    pub(in crate::preview) drag: Arc<Mutex<Option<MinimapDragSession>>>,
    pub(in crate::preview) resize_drag: Arc<Mutex<Option<ResizeSession>>>,
    pub(in crate::preview) interaction_anchor: Arc<Mutex<Option<MinimapInteractionAnchor>>>,
    pub(in crate::preview) retained_style_frame: Mutex<Vec<RasterTilePaint>>,
    pub(in crate::preview) retain_style_frame: AtomicBool,
    pub(in crate::preview) raster_epoch: AtomicU64,
    pub(in crate::preview) perf: PerfState,
}

impl MinimapState {
    pub(in crate::preview) fn new() -> Self {
        Self {
            line_index: Mutex::new(None),
            line_index_build: Mutex::new(None),
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
            perf: PerfState::new(),
        }
    }

    pub(in crate::preview) fn cancel_interaction(&self) -> bool {
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

    pub(in crate::preview) fn invalidate_style(&self, layout_changed: bool) {
        self.raster_epoch.fetch_add(1, Ordering::AcqRel);
        self.cancel_interaction();
        if layout_changed {
            *self.line_index.lock().expect("minimap line index poisoned") = None;
            *self
                .line_index_build
                .lock()
                .expect("minimap line-index builder poisoned") = None;
        }
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
        tiles.entries.clear();
        tiles.order.clear();
        tiles.in_flight.clear();
    }

    pub(in crate::preview) fn apply_document_patch(
        &self,
        document: &PreviewSnapshot,
        patch: &VisualPatch,
        presentation_rows: &Arc<Vec<usize>>,
        zoom: f32,
        style: PreviewStyle,
    ) {
        *self
            .line_index_build
            .lock()
            .expect("minimap line-index builder poisoned") = None;
        let mut cached = self.line_index.lock().expect("minimap line index poisoned");
        let Some(index) = cached.as_mut() else {
            return;
        };
        if !Arc::ptr_eq(&index.presentation_rows, presentation_rows)
            || patch.old_visual != patch.new_visual
        {
            *cached = None;
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
            return;
        }
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
                let measure =
                    display_map.estimated_measure(*row, index.index.width as f32, zoom, style);
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

pub(in crate::preview) struct PerfState {
    pub(in crate::preview) first_tile_completed: AtomicBool,
    pub(in crate::preview) first_pixels_painted: AtomicBool,
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
pub(in crate::preview) struct ResizeSession {
    pub(in crate::preview) start_pointer_x: f32,
    pub(in crate::preview) start_width: f32,
}

pub(in crate::preview) fn current_resize_session(
    state: &Mutex<Option<ResizeSession>>,
) -> Option<ResizeSession> {
    *state.lock().expect("minimap resize state poisoned")
}

pub(in crate::preview) fn take_resize_session(
    state: &Mutex<Option<ResizeSession>>,
) -> Option<ResizeSession> {
    state.lock().expect("minimap resize state poisoned").take()
}
