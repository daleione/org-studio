use crate::document::{DocumentId, Revision};
use gpui::{Bounds, Pixels};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

const CACHE_BUDGET: usize = 48 * 1024 * 1024;

pub(super) struct DocumentFrame {
    pub source: Arc<crate::document_preview::PreviewFrame>,
    pub revision: Revision,
    pub palette: u32,
    bytes: usize,
}

impl DocumentFrame {
    pub fn from_painted(
        drawn: &crate::document_preview::FrameSlot,
        bounds: Bounds<Pixels>,
        revision: Revision,
    ) -> Option<Arc<Self>> {
        let source = drawn.latest()?;
        if source.bounds != bounds {
            return None;
        }
        let bytes = source.estimated_bytes();
        Some(Arc::new(Self {
            source,
            revision,
            palette: crate::theme::current_theme().background,
            bytes,
        }))
    }
}

#[derive(Default)]
pub(super) struct FrameCache {
    frames: HashMap<DocumentId, Arc<DocumentFrame>>,
    lru: VecDeque<DocumentId>,
    bytes: usize,
}

impl FrameCache {
    pub fn get(&self, id: DocumentId, revision: Revision) -> Option<&DocumentFrame> {
        self.frames
            .get(&id)
            .filter(|frame| {
                frame.revision == revision
                    && frame.palette == crate::theme::current_theme().background
            })
            .map(Arc::as_ref)
    }

    pub fn insert(&mut self, id: DocumentId, frame: Arc<DocumentFrame>) {
        if let Some(old) = self.frames.remove(&id) {
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }
        self.lru.retain(|entry| *entry != id);
        self.bytes += frame.bytes;
        self.frames.insert(id, frame);
        self.lru.push_back(id);
        while self.bytes > CACHE_BUDGET {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            if let Some(frame) = self.frames.remove(&oldest) {
                self.bytes -= frame.bytes;
            }
        }
    }

    pub fn retain(&mut self, ids: &[DocumentId]) {
        self.frames.retain(|id, _| ids.contains(id));
        self.lru.retain(|id| self.frames.contains_key(id));
        self.bytes = self.frames.values().map(|frame| frame.bytes).sum();
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}
