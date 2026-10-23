use super::{PlotCandidate, PlotDisplay, PlotId, PlotOutputTarget, PlotPreparation};
use crate::{
    document::{DocumentId, Revision, RevisionDelta, SharedTextSnapshot},
    org_syntax::BlockArena,
};
use gpui::{App, AppContext, Context, Entity, Global, WeakEntity};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

mod index;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub(crate) struct PlotEntry {
    pub(crate) id: PlotId,
    pub(crate) source: PlotCandidate,
    pub(crate) prepared: PlotPreparation,
    pub(crate) display: PlotDisplay,
    pub(crate) output: PlotOutputTarget,
    raw: Arc<str>,
}

impl PlotEntry {
    pub(crate) fn placeholder(source: PlotCandidate) -> Arc<Self> {
        let output = super::parse::parse_spec(&source.options)
            .map(|spec| spec.output)
            .unwrap_or_default();
        Arc::new(Self {
            id: PlotId(u64::MAX - u64::from(source.owner)),
            source,
            prepared: PlotPreparation::Empty,
            display: Arc::default(),
            output,
            raw: Arc::from(""),
        })
    }
}

#[derive(Default)]
struct PlotDocuments(HashMap<DocumentId, WeakEntity<PlotDocument>>);
impl Global for PlotDocuments {}

pub(crate) struct PlotDocument {
    pub(crate) revision: Revision,
    pub(crate) display_revision: u64,
    pub(crate) entries: Arc<[Arc<PlotEntry>]>,
    target_revision: Revision,
    arena: Option<Arc<BlockArena>>,
    indexed_snapshot: Option<SharedTextSnapshot>,
    deltas: Vec<RevisionDelta>,
    task: Option<gpui::Task<()>>,
    next_id: u64,
    file_path: Option<PathBuf>,
    output_epoch: Arc<Mutex<u64>>,
    // None keeps a requested write pending while a source change rebuilds the index.
    pending_outputs: HashMap<PlotId, Option<gpui::Task<()>>>,
    pub(crate) output_errors: HashMap<PlotId, Arc<super::PlotDiagnostic>>,
}

impl PlotDocument {
    pub(crate) fn is_current(&self, revision: Revision) -> bool {
        self.arena.is_some() && self.revision == revision
    }
    pub(crate) fn shared(snapshot: SharedTextSnapshot, cx: &mut App) -> Entity<Self> {
        let id = snapshot.document_id();
        if let Some(document) = cx
            .try_global::<PlotDocuments>()
            .and_then(|documents| documents.0.get(&id))
            .and_then(WeakEntity::upgrade)
        {
            document.update(cx, |document, cx| document.refresh(snapshot, None, cx));
            return document;
        }
        let document = cx.new(|cx| {
            let revision = snapshot.revision();
            let mut document = Self {
                revision,
                display_revision: 0,
                entries: Arc::from([]),
                target_revision: revision,
                arena: None,
                indexed_snapshot: None,
                deltas: Vec::new(),
                task: None,
                next_id: 1,
                file_path: None,
                output_epoch: Arc::default(),
                pending_outputs: HashMap::new(),
                output_errors: HashMap::new(),
            };
            document.refresh(snapshot, None, cx);
            document
        });
        if cx.try_global::<PlotDocuments>().is_none() {
            cx.set_global(PlotDocuments::default());
        }
        let documents = cx.global_mut::<PlotDocuments>();
        documents
            .0
            .retain(|_, document| document.upgrade().is_some());
        documents.0.insert(id, document.downgrade());
        document
    }

    pub(crate) fn select(&mut self, id: Option<PlotId>, cx: &mut Context<Self>) {
        let mut changed = false;
        for entry in self.entries.iter() {
            let mut display = entry.display.lock().unwrap();
            let selected = id == Some(entry.id);
            changed |= selected != display.selected;
            display.selected = selected;
        }
        if changed {
            cx.notify();
        }
    }

    pub(crate) fn reset_size(&mut self, id: PlotId, cx: &mut Context<Self>) {
        self.resize_canvas(id, None, None, false, cx);
    }

    pub(crate) fn resize_canvas(
        &mut self,
        id: PlotId,
        width: Option<f32>,
        height: Option<f32>,
        resizing: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(entry) = self.entries.iter().find(|entry| entry.id == id) {
            let mut display = entry.display.lock().unwrap();
            if display.requested_width == width
                && display.requested_height == height
                && display.resizing == resizing
            {
                return;
            }
            display.requested_width = width;
            display.requested_height = height;
            display.resizing = resizing;
            drop(display);
            self.display_revision = self.display_revision.wrapping_add(1);
            cx.notify();
        }
    }

    pub(crate) fn set_file_path(&mut self, path: Option<PathBuf>) {
        if self.file_path != path {
            self.file_path = path;
            *self.output_epoch.lock().unwrap() += 1;
            self.pending_outputs.clear();
            self.output_errors.clear();
            if let (Some(snapshot), Some(arena)) = (&self.indexed_snapshot, &self.arena) {
                let mut sources = self
                    .entries
                    .iter()
                    .map(|entry| entry.source.clone())
                    .collect::<Vec<_>>();
                super::bind_images(
                    snapshot.as_ref(),
                    arena,
                    &mut sources,
                    self.file_path.as_deref(),
                );
                self.entries = self
                    .entries
                    .iter()
                    .zip(sources)
                    .map(|(entry, source)| {
                        let mut entry = (**entry).clone();
                        entry.source = source;
                        Arc::new(entry)
                    })
                    .collect::<Vec<_>>()
                    .into();
            }
        }
    }

    fn write_output(&mut self, id: PlotId, cx: &mut Context<Self>) {
        if !self.is_current(self.target_revision) {
            return;
        }
        self.pending_outputs.remove(&id);
        self.output_errors.remove(&id);
        let Some(entry) = self.entries.iter().find(|entry| entry.id == id) else {
            return;
        };
        let path = match super::output::destination(&entry.output, self.file_path.as_deref()) {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                self.output_errors.insert(id, Arc::new(error));
                cx.notify();
                return;
            }
        };
        let PlotPreparation::Ready(plot) = &entry.prepared else {
            return;
        };
        let request = super::PlotRenderRequest::natural(
            plot.clone(),
            super::PlotStyle::for_theme(&crate::theme::ORG_STUDIO_LIGHT, 12.0),
        );
        let epoch = self.output_epoch.clone();
        let format = crate::typst_runtime::OutputFormat::infer_from_path(&path)
            .expect("plot output format was validated during parsing");
        let expected = *epoch.lock().unwrap();
        let executor = cx.background_executor().clone();
        self.pending_outputs.insert(
            id,
            Some(cx.spawn(async move |this, cx| {
                let result = executor
                    .spawn(async move {
                        let bytes = super::render::render_file(
                            crate::typst_runtime::shared_engine(),
                            &request,
                            format,
                        )?;
                        super::output::write_file(&path, &bytes, &epoch, expected)
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if *this.output_epoch.lock().unwrap() != expected {
                        return;
                    }
                    this.pending_outputs.remove(&id);
                    if let Err(error) = result {
                        this.output_errors.insert(id, Arc::new(error));
                    }
                    cx.notify();
                });
            })),
        );
    }

    pub(crate) fn refresh(
        &mut self,
        snapshot: SharedTextSnapshot,
        delta: Option<RevisionDelta>,
        cx: &mut Context<Self>,
    ) {
        let revision = snapshot.revision();
        if revision < self.target_revision
            || revision == self.target_revision && (self.arena.is_some() || self.task.is_some())
        {
            return;
        }
        if let Some(delta) = delta {
            self.deltas.push(delta);
        } else if revision != self.target_revision {
            self.deltas.clear();
        }
        *self.output_epoch.lock().unwrap() += 1;
        for task in self.pending_outputs.values_mut() {
            *task = None;
        }
        self.target_revision = revision;
        let arena = self.arena.clone();
        let deltas = self.deltas.clone();
        let old = self.entries.clone();
        let previous_snapshot = self.indexed_snapshot.clone();
        let executor = cx.background_executor().clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            executor.timer(Duration::from_millis(80)).await;
            let index = executor
                .spawn(async move { index::build(snapshot, old, previous_snapshot, arena, deltas) })
                .await;
            let _ = this.update(cx, |this, cx| this.apply_index(index, cx));
        }));
    }

    fn apply_index(&mut self, index: index::PlotIndex, cx: &mut Context<Self>) {
        let index::PlotIndex {
            snapshot: indexed_snapshot,
            arena,
            mut candidates,
            mapped_anchors,
        } = index;
        let revision = indexed_snapshot.revision();
        let had_index = self.arena.is_some();
        if self.target_revision != revision {
            return;
        }
        let mut sources = candidates
            .iter()
            .map(|candidate| candidate.source.clone())
            .collect::<Vec<_>>();
        super::parse::bind_images(
            indexed_snapshot.as_ref(),
            &arena,
            &mut sources,
            self.file_path.as_deref(),
        );
        for (candidate, source) in candidates.iter_mut().zip(sources) {
            candidate.source = source;
        }
        let old = self.entries.clone();
        let mut existing = HashMap::new();
        for entry in old.iter() {
            let value = existing
                .entry(entry.source.identity.clone())
                .or_insert((entry.id, 0usize));
            value.1 += 1;
        }
        let mut counts = HashMap::new();
        for candidate in &candidates {
            *counts
                .entry(candidate.source.identity.clone())
                .or_insert(0usize) += 1;
        }
        let anchored = mapped_anchors.values().copied().collect::<HashSet<_>>();
        let mut used = HashSet::new();
        let entries = candidates
            .iter()
            .map(
                |index::IndexedPlot {
                     source,
                     raw,
                     prepared,
                 }| {
                    let reusable = existing.get(&source.identity).filter(|(_, count)| {
                        *count == 1 && counts.get(&source.identity) == Some(&1)
                    });
                    let previous_id = mapped_anchors
                        .get(&source.declaration.start.0)
                        .copied()
                        // Whole-result replacements invalidate byte anchors, not ownership.
                        .or_else(|| {
                            reusable
                                .map(|(id, _)| *id)
                                .filter(|id| !anchored.contains(id))
                        })
                        .filter(|id| used.insert(*id));
                    let id = if let Some(id) = previous_id {
                        id
                    } else {
                        let id = PlotId(self.next_id);
                        self.next_id += 1;
                        id
                    };
                    let previous = old.iter().find(|entry| entry.id == id);
                    let display: PlotDisplay = previous
                        .map(|entry| entry.display.clone())
                        .unwrap_or_default();
                    if source.image_link.is_some()
                        && previous.is_some_and(|old| {
                            old.source.image_link.is_none()
                                || old.source.image_attributes != source.image_attributes
                        })
                    {
                        let mut state = display.lock().unwrap();
                        state.requested_width = None;
                        state.requested_height = None;
                        state.resizing = false;
                    }
                    Arc::new(PlotEntry {
                        id,
                        source: source.clone(),
                        raw: raw.clone(),
                        prepared: prepared.clone(),
                        display,
                        output: super::parse::parse_spec(&source.options)
                            .map(|spec| spec.output)
                            .unwrap_or_default(),
                    })
                },
            )
            .collect::<Vec<_>>();
        let changed = entries
            .iter()
            .filter(|entry| {
                had_index
                    && (self.pending_outputs.contains_key(&entry.id)
                        || old
                            .iter()
                            .find(|old| old.id == entry.id)
                            .is_none_or(|old| old.raw != entry.raw))
            })
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        self.entries = entries.into();
        self.pending_outputs
            .retain(|id, _| self.entries.iter().any(|entry| entry.id == *id));
        self.output_errors
            .retain(|id, _| self.entries.iter().any(|entry| entry.id == *id));
        self.arena = Some(arena);
        self.indexed_snapshot = Some(indexed_snapshot);
        self.revision = revision;
        self.deltas.clear();
        self.task = None;
        for id in changed {
            self.write_output(id, cx);
        }
        if !old.is_empty() || !self.entries.is_empty() {
            cx.notify();
        }
    }
}

impl Drop for PlotDocument {
    fn drop(&mut self) {
        *self.output_epoch.lock().unwrap() += 1;
    }
}
