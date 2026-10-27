use crate::document::{DocumentId, Revision};
use gpui::{Bounds, Pixels};
use std::{
    collections::{HashMap, VecDeque},
    rc::Rc,
};

const CACHE_BUDGET: usize = 48 * 1024 * 1024;

pub(super) struct DocumentFrame {
    pub source: Rc<crate::document_preview::PreviewFrame>,
    pub revision: Revision,
    pub palette: u32,
    bytes: usize,
}

impl DocumentFrame {
    pub fn from_painted(
        drawn: &crate::document_preview::FrameSlot,
        bounds: Bounds<Pixels>,
        revision: Revision,
    ) -> Option<Rc<Self>> {
        let source = drawn.latest()?;
        if source.bounds != bounds {
            return None;
        }
        let bytes = source.estimated_bytes();
        Some(Rc::new(Self {
            source,
            revision,
            palette: crate::theme::current_theme().background,
            bytes,
        }))
    }
}

#[derive(Default)]
pub(super) struct FrameCache {
    frames: HashMap<DocumentId, Rc<DocumentFrame>>,
    lru: VecDeque<DocumentId>,
    bytes: usize,
}

impl FrameCache {
    pub fn get(&self, id: DocumentId, revision: Revision) -> Option<&DocumentFrame> {
        // Static cards retain their old paint until the replacement is ready.
        // Opening a document always prepares a fresh frame before animating.
        self.frames
            .get(&id)
            .filter(|frame| frame.revision == revision)
            .map(Rc::as_ref)
    }

    pub fn insert(&mut self, id: DocumentId, frame: Rc<DocumentFrame>) {
        self.remove(id);
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

    pub fn needs_refresh(
        &self,
        id: DocumentId,
        revision: Revision,
        bounds: Bounds<Pixels>,
    ) -> bool {
        self.frames.get(&id).is_some_and(|frame| {
            frame.revision == revision
                && (frame.palette != crate::theme::current_theme().background
                    || frame.source.bounds != bounds)
        })
    }

    pub fn remove(&mut self, id: DocumentId) {
        if let Some(old) = self.frames.remove(&id) {
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }
        self.lru.retain(|entry| *entry != id);
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
