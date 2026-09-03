use std::{sync::Arc, time::Duration};

use crate::document::TextSnapshot;
use gpui::Context;

use super::WorkspaceWindow;
use crate::preview::{PreviewSnapshot, derive_preview_incremental};

pub(crate) struct DerivedRequest {
    path: std::path::PathBuf,
    snapshot: crate::document::DocumentSnapshot,
    deltas: Vec<crate::document::RevisionDelta>,
    previous: Option<Arc<PreviewSnapshot>>,
    force: bool,
}

impl WorkspaceWindow {
    pub(crate) fn suspend_derived_preview(&mut self) {
        self.derived
            .pending
            .lock()
            .expect("derived request slot poisoned")
            .take();
        if let Some(sender) = &self.derived.sender {
            // Wake the worker so it can release its incremental base when no request remains.
            let _ = sender.try_send(());
        }
    }

    pub(crate) fn reconcile_derived_preview(&mut self, cx: &mut Context<Self>) {
        if self.document_workspace.needs_reading() {
            self.schedule_derived_update(cx);
        } else {
            self.suspend_derived_preview();
        }
    }

    /// Rebuilds a coherent preview snapshot off the UI thread. Publication is revision-gated, so
    /// an older parse can never replace a newer source revision.
    pub(crate) fn schedule_derived_update(&mut self, cx: &mut Context<Self>) {
        self.schedule_derived_update_inner(None, false, cx);
    }

    pub(crate) fn schedule_derived_resource_update(&mut self, cx: &mut Context<Self>) {
        self.schedule_derived_update_inner(None, true, cx);
    }

    pub(crate) fn schedule_derived_update_with_delta(
        &mut self,
        delta: Option<crate::document::RevisionDelta>,
        cx: &mut Context<Self>,
    ) {
        self.schedule_derived_update_inner(delta, false, cx);
    }

    fn schedule_derived_update_inner(
        &mut self,
        delta: Option<crate::document::RevisionDelta>,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.document_workspace.needs_reading() {
            return;
        }
        let Some(session) = self.document_session().cloned() else {
            return;
        };
        let snapshot = session.read(cx).snapshot();
        let path = session.read(cx).path().to_path_buf();
        self.reconcile_visible_reading_panes(cx);
        if !force && self.visible_reading_panes_are_current(cx) {
            return;
        }
        if self.derived.sender.is_none() {
            let (sender, receiver) = async_channel::bounded::<()>(1);
            self.derived.sender = Some(sender);
            let pending = self.derived.pending.clone();
            let executor = cx.background_executor().clone();
            self.derived.task = Some(cx.spawn(async move |this, cx| {
                let mut local_base: Option<Arc<PreviewSnapshot>> = None;
                while receiver.recv().await.is_ok() {
                    executor.timer(Duration::from_millis(24)).await;
                    while receiver.try_recv().is_ok() {}
                    let Some(request) = pending
                        .lock()
                        .expect("derived request slot poisoned")
                        .take()
                    else {
                        local_base = None;
                        continue;
                    };
                    let request_document_id = request.snapshot.document_id();
                    let request_revision = request.snapshot.revision();
                    let force = request.force;
                    let previous = request.previous.filter(|base| {
                        base.document_id == request_document_id
                            && base.path == request.path
                            && request
                                .deltas
                                .first()
                                .is_none_or(|delta| delta.before == base.revision)
                    });
                    let base = (!force)
                        .then(|| {
                            local_base
                                .as_ref()
                                .filter(|base| {
                                    base.document_id == request_document_id
                                        && base.path == request.path
                                        && request
                                            .deltas
                                            .first()
                                            .is_some_and(|delta| delta.before == base.revision)
                                })
                                .cloned()
                                .or(previous)
                        })
                        .flatten();
                    let request_path = request.path.clone();
                    let preview = executor
                        .spawn(async move {
                            derive_preview_incremental(
                                request.path,
                                request.snapshot,
                                base.as_deref(),
                                &request.deltas,
                            )
                        })
                        .await;
                    let document = Arc::new(preview);
                    let keep_base = this
                        .update(cx, |this, cx| {
                            if !this.document_workspace.needs_reading() {
                                return false;
                            }
                            let Some(current) = this.document_session() else {
                                return false;
                            };
                            if current.read(cx).id() != request_document_id
                                || current.read(cx).revision() != request_revision
                                || current.read(cx).path() != request_path
                            {
                                return false;
                            }
                            this.reconcile_visible_reading_panes(cx);
                            if !force && this.visible_reading_panes_are_current(cx) {
                                return false;
                            }
                            this.derived.latest = Some(document.clone());
                            this.reconcile_visible_reading_panes(cx);
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    local_base = keep_base.then_some(document);
                }
            }));
        }
        if let Some(sender) = &self.derived.sender {
            let previous = self.derived.latest.clone();
            let mut request = DerivedRequest {
                path,
                snapshot,
                deltas: delta.into_iter().collect(),
                previous,
                force,
            };
            let mut pending = self
                .derived
                .pending
                .lock()
                .expect("derived request slot poisoned");
            request.force |= pending.as_ref().is_some_and(|queued| queued.force);
            if let Some(queued) = pending.as_mut()
                && queued
                    .deltas
                    .last()
                    .zip(request.deltas.first())
                    .is_some_and(|(old, new)| old.after == new.before)
            {
                queued.deltas.extend(request.deltas);
                queued.path = request.path;
                queued.snapshot = request.snapshot;
                queued.force |= request.force;
            } else {
                *pending = Some(request);
            }
            drop(pending);
            let _ = sender.try_send(());
        }
        cx.notify();
    }
}
