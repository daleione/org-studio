use std::{sync::Arc, time::Duration};

use crate::document::TextSnapshot;
use gpui::{AppContext, Context};

use super::{PreviewLoadState, PreviewPanel, WorkspaceWindow, derive_preview};

pub(crate) struct DerivedRequest {
    path: std::path::PathBuf,
    snapshot: crate::document::DocumentSnapshot,
}

impl WorkspaceWindow {
    pub(super) fn ensure_split_scroll_sync(&mut self, cx: &mut Context<Self>) {
        if self.document_mode != crate::app::DocumentMode::Split {
            self.split_scroll.source_subscription = None;
            self.split_scroll.panel_revision = None;
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
        if self.split_scroll.source_subscription.is_none() {
            self.split_scroll.source_subscription = Some(cx.subscribe(
                &editor,
                |this, _, _: &crate::editor::SourceScrollEvent, cx| {
                    this.sync_source_scroll_to_preview(cx);
                },
            ));
        }
        let panel_revision = (snapshot.document_id(), snapshot.revision());
        if self.split_scroll.panel_revision != Some(panel_revision) {
            let workspace = cx.entity().downgrade();
            panel
                .read(cx)
                .list_state()
                .set_scroll_handler(move |_, _, cx| {
                    let workspace = workspace.clone();
                    cx.defer(move |cx| {
                        let _ = workspace.update(cx, |this, cx| {
                            this.sync_preview_scroll_to_source(cx);
                        });
                    });
                });
            self.split_scroll.panel_revision = Some(panel_revision);
            self.sync_source_scroll_to_preview(cx);
        }
    }

    fn sync_source_scroll_to_preview(&mut self, cx: &mut Context<Self>) {
        if self.document_mode != crate::app::DocumentMode::Split {
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
            panel.scroll_to_split_anchor(source, fraction)
        });
        self.split_scroll.source_anchor = Some((source, fraction.to_bits()));
        self.split_scroll.preview_anchor = panel
            .read(cx)
            .split_anchor()
            .map(|(source, fraction)| (source, fraction.to_bits()));
    }

    fn sync_preview_scroll_to_source(&mut self, cx: &mut Context<Self>) {
        if self.document_mode != crate::app::DocumentMode::Split {
            return;
        }
        let Some(ready) = self.state.ready() else {
            return;
        };
        let Some(panel) = ready.panel.clone() else {
            return;
        };
        let Some((source, fraction)) = panel.read(cx).split_anchor() else {
            return;
        };
        ready.editor.update(cx, |editor, cx| {
            editor.scroll_to_source_anchor(source, fraction, cx)
        });
        self.split_scroll.preview_anchor = Some((source, fraction.to_bits()));
        let snapshot = ready.session.read(cx).snapshot();
        self.split_scroll.source_anchor = Some({
            let (source, fraction) = ready.editor.read(cx).top_source_anchor(&snapshot);
            (source, fraction.to_bits())
        });
    }

    /// Rebuilds a coherent preview snapshot off the UI thread. Publication is revision-gated, so
    /// an older parse can never replace a newer source revision.
    pub(super) fn schedule_derived_update(&mut self, cx: &mut Context<Self>) {
        if self.document_mode == crate::app::DocumentMode::Source {
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
            let (sender, receiver) = async_channel::unbounded::<DerivedRequest>();
            self.derived.sender = Some(sender);
            let executor = cx.background_executor().clone();
            self.derived.task = Some(cx.spawn(async move |this, cx| {
                while let Ok(mut request) = receiver.recv().await {
                    executor.timer(Duration::from_millis(24)).await;
                    while let Ok(newer) = receiver.try_recv() {
                        request = newer;
                    }
                    let request_document_id = request.snapshot.document_id();
                    let request_revision = request.snapshot.revision();
                    let preview = executor
                        .spawn(async move { derive_preview(request.path, request.snapshot) })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        let Some(current) = this.document_session() else {
                            return;
                        };
                        if current.read(cx).id() != request_document_id
                            || current.read(cx).revision() != request_revision
                        {
                            return;
                        }
                        let document = Arc::new(preview);
                        let list_overdraw = this.list_overdraw;
                        if let Some(panel) = this.preview_panel() {
                            panel.update(cx, |panel, cx| {
                                panel.replace_document(document, list_overdraw, cx)
                            });
                        } else if let PreviewLoadState::Ready { document: ready } = &mut this.state
                        {
                            ready.panel =
                                Some(cx.new(|_| PreviewPanel::new(document, list_overdraw)));
                        }
                        this.derived.published = Some((request_document_id, request_revision));
                        cx.notify();
                    });
                }
            }));
        }
        if let Some(sender) = &self.derived.sender {
            let _ = sender.try_send(DerivedRequest { path, snapshot });
        }
        cx.notify();
    }
}
