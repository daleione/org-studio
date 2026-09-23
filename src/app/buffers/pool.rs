use super::*;
use crate::app::{ContentRoute, WorkspaceLoadState};
use gpui::AppContext;

impl WorkspaceWindow {
    pub(crate) fn park_document(&mut self, document: ReadyDocument) {
        self.buffers.parked.insert(
            0,
            ParkedDocument {
                document,
                workspace: self.document_workspace,
                soft_wrap: self.soft_wrap,
                preview: self.derived.latest.take(),
            },
        );
    }

    pub(crate) fn activate_buffer(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        if self.buffer_busy() {
            return;
        }
        if self.buffer_session(id, cx).is_none() {
            return;
        }
        let is_current = self
            .document_session()
            .is_some_and(|s| s.read(cx).id() == id);
        if !is_current {
            self.remember_navigation_location(self.capture_navigation_location(cx));
        }
        self.background_pending_open(cx);
        self.close_command_line(cx);
        self.close_search(false, cx);
        self.search.presentation = None;
        self.dismiss_buffer_panel(cx);
        self.content_route = ContentRoute::Document;
        if is_current {
            self.request_document_focus(cx);
            cx.notify();
            return;
        }
        let Some(index) = self
            .buffers
            .parked
            .iter()
            .position(|d| d.document.session.read(cx).id() == id)
        else {
            return;
        };
        let parked = self.buffers.parked.remove(index);
        let source_file =
            crate::preview::is_editor_only_document(parked.document.session.read(cx).syntax_path());
        self.suspend_derived_preview();
        if let Some(current) = self.state.take_ready() {
            self.park_document(current);
        }
        self.generation = self.generation.wrapping_add(1);
        self.load_task = None;
        self.pending_navigation = None;
        self.pending_surface_anchors.left = None;
        self.pending_surface_anchors.right = None;
        self.state = WorkspaceLoadState::Ready {
            document: parked.document,
        };
        self.document_workspace = parked.workspace;
        if source_file {
            for pane in [crate::app::PaneSide::Left, crate::app::PaneSide::Right] {
                self.document_workspace
                    .set_surface(pane, crate::app::PaneSurface::Editor);
            }
        }
        self.soft_wrap = parked.soft_wrap;
        self.derived.latest = parked.preview;
        self.editor_subscriptions.clear();
        self.reconcile_visible_editor_panes(cx);
        self.reconcile_derived_preview(cx);
        self.sync_document_watch(cx);
        self.request_document_focus(cx);
        cx.notify();
    }

    pub(crate) fn create_buffer(
        &mut self,
        name: String,
        target: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if self.buffer_busy() {
            return;
        }
        self.background_pending_open(cx);
        let session = if let Some(path) = target {
            let mut session = DocumentSession::from_utf8(path, Vec::new()).expect("empty UTF-8");
            session.observe_disk(None);
            session
        } else {
            DocumentSession::draft(name)
        };
        self.close_command_line(cx);
        self.close_search(false, cx);
        self.dismiss_buffer_panel(cx);
        self.suspend_derived_preview();
        self.generation = self.generation.wrapping_add(1);
        self.load_task = None;
        let generation = self.generation;
        self.apply_load_result(
            generation,
            Ok(crate::preview::WorkspaceLoadedDocument::Source(Box::new(
                session,
            ))),
            cx,
        );
        self.content_route = ContentRoute::Document;
        self.reconcile_derived_preview(cx);
        self.sync_document_watch(cx);
        self.request_document_focus(cx);
        cx.notify();
    }

    pub(crate) fn remove_buffer(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        if self
            .document_session()
            .is_some_and(|s| s.read(cx).id() == id)
        {
            self.close_command_line(cx);
            self.close_search(false, cx);
            self.state = WorkspaceLoadState::Empty;
            self.derived.latest = None;
            if let Some(next) = self
                .buffers
                .parked
                .first()
                .map(|d| d.document.session.read(cx).id())
            {
                self.activate_buffer(next, cx);
            } else {
                self.show_home_now(cx);
            }
        } else {
            self.buffers
                .parked
                .retain(|d| d.document.session.read(cx).id() != id);
        }
        cx.notify();
    }

    pub(crate) fn cycle_buffer(&mut self, forward: bool, cx: &mut Context<Self>) {
        let ids = self
            .buffer_sessions()
            .map(|s| s.read(cx).id())
            .collect::<Vec<_>>();
        if ids.len() < 2 {
            return;
        }
        if self.buffers.cycle.len() != ids.len()
            || ids.iter().any(|id| !self.buffers.cycle.contains(id))
        {
            self.buffers.cycle = ids;
        }
        let current = self.document_session().map(|s| s.read(cx).id());
        let index = self
            .buffers
            .cycle
            .iter()
            .position(|id| Some(*id) == current)
            .unwrap_or(0);
        let len = self.buffers.cycle.len();
        let next = (index + if forward { 1 } else { len - 1 }) % len;
        self.activate_buffer(self.buffers.cycle[next], cx);
    }

    pub(crate) fn open_buffers(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.buffer_busy() {
            return;
        }
        let mut paths = paths.into_iter();
        if let Some(first) = paths.next() {
            self.open(first, cx);
        }
        for path in paths {
            if !crate::preview::is_supported_image(&path) {
                self.open_background_buffer(path, cx);
            }
        }
    }

    /// A new foreground action keeps the previous requested file in the open set.
    /// Restart its load in the background so its completion cannot steal focus.
    pub(crate) fn background_pending_open(&mut self, cx: &mut Context<Self>) {
        let WorkspaceLoadState::Loading { path, .. } = &self.state else {
            return;
        };
        let path = path.clone();
        self.load_task = None;
        self.generation = self.generation.wrapping_add(1);
        self.pending_link_surface = None;
        if let Some(document) = self.state.take_ready() {
            self.state = WorkspaceLoadState::Ready { document };
        }
        self.open_background_buffer(path, cx);
    }

    pub(crate) fn open_background_buffer(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if self.buffer_for_path(&path, cx).is_some()
            || self.buffers.loads.contains_key(&path)
            || matches!(&self.state, WorkspaceLoadState::Loading { path: loading, .. } if same_file(loading, &path))
        {
            return;
        }
        let request_path = path.clone();
        let load = cx
            .background_executor()
            .spawn(async move { crate::preview::load_workspace_document(request_path, false) });
        let task_path = path.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = load.await;
            let _ = this.update(cx, |this, cx| {
                this.buffers.loads.remove(&task_path);
                match result {
                    Ok(crate::preview::WorkspaceLoadedDocument::Source(session)) => {
                        if this.buffer_for_path(&task_path, cx).is_some() {
                            return;
                        }
                        crate::recent_documents::record_success(
                            &mut this.recent_documents,
                            task_path,
                        );
                        let session = cx.new(|_| *session);
                        this.buffers.parked.push(ParkedDocument {
                            document: ReadyDocument {
                                session,
                                editor_syntax: Arc::default(),
                                editors: crate::app::PanePair {
                                    left: None,
                                    right: None,
                                },
                                readers: crate::app::PanePair {
                                    left: None,
                                    right: None,
                                },
                            },
                            workspace: this.document_workspace,
                            soft_wrap: true,
                            preview: None,
                        });
                    }
                    Ok(_) => unreachable!("source-only load"),
                    Err((path, error)) => {
                        this.home_error = Some(format!("{}: {error}", path.display()).into());
                        this.save.error = Some(format!("{}: {error}", path.display()).into());
                    }
                }
                cx.notify();
            });
        });
        self.buffers.loads.insert(path, task);
    }
}
