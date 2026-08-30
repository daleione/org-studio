use std::{sync::Arc, time::Duration};

use crate::document::TextSnapshot;
use gpui::{AppContext, Context};

use super::{PreviewLoadState, PreviewPanel, WorkspaceWindow, derive_preview_incremental};

pub(crate) struct DerivedRequest {
    path: std::path::PathBuf,
    snapshot: crate::document::DocumentSnapshot,
    deltas: Vec<crate::document::RevisionDelta>,
    previous: Option<Arc<super::PreviewSnapshot>>,
}

impl WorkspaceWindow {
    pub(super) fn ensure_right_preview_scroll_sync(&mut self, cx: &mut Context<Self>) {
        if !self.document_view.right_preview_open() {
            self.right_preview_scroll.source_subscription = None;
            self.right_preview_scroll.panel_revision = None;
            return;
        }
        let Some(ready) = self.state.ready() else {
            return;
        };
        let Some(panel) = ready.panel.clone() else {
            return;
        };
        let editor = ready.editor.clone();
        let snapshot = ready.session.read(cx).snapshot();
        if panel.read(cx).document().document_id != snapshot.document_id()
            || panel.read(cx).document().revision != snapshot.revision()
        {
            // A lagging projection has no safe source mapping. Hold both viewports until a
            // coherent snapshot is published.
            return;
        }
        if self.right_preview_scroll.source_subscription.is_none() {
            self.right_preview_scroll.source_subscription = Some(cx.subscribe(
                &editor,
                |this, _, _: &crate::editor::SourceScrollEvent, cx| {
                    this.sync_editor_scroll_to_right_preview(cx);
                },
            ));
        }
        let panel_revision = (snapshot.document_id(), snapshot.revision());
        if self.right_preview_scroll.panel_revision != Some(panel_revision) {
            let workspace = cx.entity().downgrade();
            panel
                .read(cx)
                .list_state()
                .set_scroll_handler(move |_, _, cx| {
                    let workspace = workspace.clone();
                    cx.defer(move |cx| {
                        let _ = workspace.update(cx, |this, cx| {
                            this.sync_right_preview_scroll_to_editor(cx);
                        });
                    });
                });
            self.right_preview_scroll.panel_revision = Some(panel_revision);
            self.sync_editor_scroll_to_right_preview(cx);
        }
    }

    fn sync_editor_scroll_to_right_preview(&mut self, cx: &mut Context<Self>) {
        if !self.document_view.right_preview_open() {
            return;
        }
        let Some(ready) = self.state.ready() else {
            return;
        };
        let Some(panel) = ready.panel.clone() else {
            return;
        };
        let snapshot = ready.session.read(cx).snapshot();
        if panel.read(cx).document().revision != snapshot.revision() {
            return;
        }
        let (source, fraction) = ready.editor.read(cx).top_source_anchor(&snapshot);
        panel.update(cx, |panel, _| {
            panel.scroll_to_source_anchor(source, fraction)
        });
    }

    fn sync_right_preview_scroll_to_editor(&mut self, cx: &mut Context<Self>) {
        if !self.document_view.right_preview_open() {
            return;
        }
        let Some(ready) = self.state.ready() else {
            return;
        };
        let Some(panel) = ready.panel.clone() else {
            return;
        };
        let Some((source, fraction)) = panel.read(cx).source_scroll_anchor() else {
            return;
        };
        ready.editor.update(cx, |editor, cx| {
            editor.scroll_to_source_anchor(source, fraction, cx)
        });
    }

    pub(super) fn discard_derived_preview(&mut self) {
        self.derived
            .pending
            .lock()
            .expect("derived request slot poisoned")
            .take();
        if let Some(document) = self.state.ready_mut() {
            document.panel = None;
        }
        self.derived.published = None;
        self.right_preview_scroll.source_subscription = None;
        self.right_preview_scroll.panel_revision = None;
        if let Some(sender) = &self.derived.sender {
            // Wake the worker so it can release its incremental base when no request remains.
            let _ = sender.try_send(());
        }
    }

    pub(super) fn reconcile_derived_preview(&mut self, cx: &mut Context<Self>) {
        if self.document_view.needs_preview() {
            self.schedule_derived_update(cx);
        } else {
            self.discard_derived_preview();
        }
    }

    /// Rebuilds a coherent preview snapshot off the UI thread. Publication is revision-gated, so
    /// an older parse can never replace a newer source revision.
    pub(super) fn schedule_derived_update(&mut self, cx: &mut Context<Self>) {
        self.schedule_derived_update_with_delta(None, cx);
    }

    pub(super) fn schedule_derived_update_with_delta(
        &mut self,
        delta: Option<crate::document::RevisionDelta>,
        cx: &mut Context<Self>,
    ) {
        if !self.document_view.needs_preview() {
            return;
        }
        let Some(session) = self.document_session().cloned() else {
            return;
        };
        let snapshot = session.read(cx).snapshot();
        let path = session.read(cx).path().to_path_buf();
        let document_id = snapshot.document_id();
        let revision = snapshot.revision();
        if self.derived.published == Some((document_id, revision))
            && self
                .preview_panel()
                .is_some_and(|panel| panel.read(cx).document().document_id == document_id)
        {
            return;
        }
        if self.derived.sender.is_none() {
            let (sender, receiver) = async_channel::bounded::<()>(1);
            self.derived.sender = Some(sender);
            let pending = self.derived.pending.clone();
            let executor = cx.background_executor().clone();
            self.derived.task = Some(cx.spawn(async move |this, cx| {
                let mut local_base: Option<Arc<super::PreviewSnapshot>> = None;
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
                    let base = local_base
                        .as_ref()
                        .filter(|base| {
                            base.document_id == request_document_id
                                && request
                                    .deltas
                                    .first()
                                    .is_some_and(|delta| delta.before == base.revision)
                        })
                        .cloned()
                        .or(request.previous);
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
                            if !this.document_view.needs_preview() {
                                return false;
                            }
                            let Some(current) = this.document_session() else {
                                return false;
                            };
                            if current.read(cx).id() != request_document_id
                                || current.read(cx).revision() != request_revision
                            {
                                return false;
                            }
                            if this.derived.published
                                == Some((request_document_id, request_revision))
                                && this.preview_panel().is_some_and(|panel| {
                                    let preview = panel.read(cx);
                                    preview.document().document_id == request_document_id
                                        && preview.document().revision == request_revision
                                })
                            {
                                return false;
                            }
                            let list_overdraw = this.list_overdraw;
                            if let Some(panel) = this.preview_panel() {
                                let document = document.clone();
                                panel.update(cx, |panel, cx| panel.replace_document(document, cx));
                            } else if let PreviewLoadState::Ready { document: ready } =
                                &mut this.state
                            {
                                let document = document.clone();
                                ready.panel =
                                    Some(cx.new(|_| PreviewPanel::new(document, list_overdraw)));
                            }
                            this.derived.published = Some((request_document_id, request_revision));
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    local_base = keep_base.then_some(document);
                }
            }));
        }
        if let Some(sender) = &self.derived.sender {
            let previous = self
                .preview_panel()
                .map(|panel| panel.read(cx).document().clone());
            let request = DerivedRequest {
                path,
                snapshot,
                deltas: delta.into_iter().collect(),
                previous,
            };
            let mut pending = self
                .derived
                .pending
                .lock()
                .expect("derived request slot poisoned");
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
            } else {
                *pending = Some(request);
            }
            drop(pending);
            let _ = sender.try_send(());
        }
        cx.notify();
    }
}
