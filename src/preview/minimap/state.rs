use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use super::{
    CachedMinimapLineIndex, MinimapInteractionAnchor, MinimapLineIndexBuilder, RasterTileCache,
};

pub(in crate::preview) struct MinimapState {
    pub(in crate::preview) line_index: Mutex<Option<CachedMinimapLineIndex>>,
    pub(in crate::preview) line_index_build: Mutex<Option<MinimapLineIndexBuilder>>,
    pub(in crate::preview) raster_tiles: Mutex<RasterTileCache>,
    pub(in crate::preview) drag: Arc<Mutex<Option<DragSession>>>,
    pub(in crate::preview) resize_drag: Arc<Mutex<Option<ResizeSession>>>,
    pub(in crate::preview) interaction_anchor: Arc<Mutex<Option<MinimapInteractionAnchor>>>,
    pub(in crate::preview) initial_visible_batch_ready: AtomicBool,
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
            initial_visible_batch_ready: AtomicBool::new(false),
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
pub(in crate::preview) struct DragSession {
    pub(in crate::preview) start_pointer_y: f32,
    pub(in crate::preview) start_thumb_top: f32,
    pub(in crate::preview) start_ratio: f32,
    pub(in crate::preview) current_thumb_top: f32,
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
