use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{Context, PathPromptOptions};

use crate::preview::{WorkspaceReloadedDocument, reload_workspace_document};
use crate::{
    app::{ContentRoute, PanePair, ReadyDocument, WorkspaceLoadState, WorkspaceWindow},
    preview::{
        InitialDocumentLoad, ReadingPreviewPanel, WorkspaceLoadedDocument, accept_generation,
        is_supported_document, load_workspace_document, minimap,
    },
};
use gpui::AppContext;

impl WorkspaceWindow {
    pub(crate) fn show_home_now(&mut self, cx: &mut Context<Self>) {
        self.suspend_derived_preview();
        self.derived.latest = None;
        self.generation = self.generation.wrapping_add(1);
        self.load_task = None;
        self.stop_document_watch();
        self.editor_minimap_width_subscriptions.clear();
        self.save.status = None;
        self.save.interaction = crate::app::save::SaveInteraction::Idle;
        self.state = WorkspaceLoadState::Empty;
        self.opened_at = None;
        self.first_frame_scheduled = None;
        self.home_error = None;
        self.pending_navigation = None;
        self.content_route = ContentRoute::Document;
        self.file_manager.reset_for_document();
        self.stop_dired_directory_watch();
        self.install_document_keymap();
        cx.notify();
    }

    pub(crate) fn begin_open(&mut self, path: PathBuf, opened_at: Instant) -> u64 {
        self.begin_open_with_previous(path, opened_at, true)
    }

    fn begin_open_with_previous(
        &mut self,
        path: PathBuf,
        opened_at: Instant,
        preserve_previous: bool,
    ) -> u64 {
        self.home_error = None;
        self.save.status = None;
        self.pending_navigation = None;
        self.generation += 1;
        self.opened_at = Some(opened_at);
        self.first_frame_scheduled = None;
        let generation = self.generation;
        let previous = preserve_previous.then(|| self.state.take_ready()).flatten();
        self.state = WorkspaceLoadState::Loading {
            path: path.clone(),
            previous,
        };
        generation
    }

    pub fn open_initial(&mut self, load: InitialDocumentLoad, cx: &mut Context<Self>) {
        let InitialDocumentLoad {
            path,
            started_at,
            receiver,
        } = load;
        let generation = self.begin_open(path.clone(), started_at);
        match receiver.try_recv() {
            Ok(result) => {
                if self.apply_load_result(generation, result, cx) {
                    self.reconcile_derived_preview(cx);
                    self.sync_document_watch(cx);
                    cx.notify();
                }
            }
            Err(async_channel::TryRecvError::Empty) => {
                self.load_task = Some(cx.spawn(async move |this, cx| {
                    let result = receiver.recv().await.unwrap_or_else(|_| {
                        Err((path, "initial document loader stopped".to_owned()))
                    });
                    let _ = this.update(cx, |this, cx| {
                        if this.apply_load_result(generation, result, cx) {
                            this.reconcile_derived_preview(cx);
                            this.sync_document_watch(cx);
                            cx.notify();
                        }
                    });
                }));
            }
            Err(async_channel::TryRecvError::Closed) => {
                let result: Result<WorkspaceLoadedDocument, _> =
                    Err((path, "initial document loader stopped".to_owned()));
                if self.apply_load_result(generation, result, cx) {
                    self.reconcile_derived_preview(cx);
                    self.sync_document_watch(cx);
                    cx.notify();
                }
            }
        }
    }

    pub fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.open_with_previous(path, true, cx);
    }

    pub(crate) fn open_discarding_current(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.open_with_previous(path, false, cx);
    }

    fn open_with_previous(
        &mut self,
        path: PathBuf,
        preserve_previous: bool,
        cx: &mut Context<Self>,
    ) {
        if !preserve_previous {
            self.stop_document_watch();
        }
        let generation =
            self.begin_open_with_previous(path.clone(), Instant::now(), preserve_previous);
        let build_preview = self.document_workspace.needs_reading();

        // Opening the requested document is user-visible latency. Submit it before synchronous
        // file-watcher setup and before one-time font startup work so a small local file is not
        // queued behind either of them.
        let opened_at = self.opened_at.unwrap_or_else(Instant::now);
        let background =
            cx.background_executor()
                .spawn_with_priority(gpui::Priority::High, async move {
                    if minimap::minimap_perf_enabled() {
                        eprintln!(
                            "org_preview_document_load_start generation={} since_open_ms={:.3}",
                            generation,
                            opened_at.elapsed().as_secs_f64() * 1000.0,
                        );
                    }
                    let result = load_workspace_document(path, build_preview);
                    if minimap::minimap_perf_enabled() {
                        eprintln!(
                            "org_preview_document_load_complete generation={} since_open_ms={:.3}",
                            generation,
                            opened_at.elapsed().as_secs_f64() * 1000.0,
                        );
                    }
                    result
                });
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = background.await;
            let _ = this.update(cx, |this, cx| {
                if this.apply_load_result(generation, result, cx) {
                    this.reconcile_derived_preview(cx);
                    this.sync_document_watch(cx);
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    pub(crate) fn reload_current(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.document_session().cloned() else {
            return;
        };
        let request = match session.read(cx).reload_request() {
            Ok(request) => request,
            Err(crate::document::ReloadError::Dirty) => {
                self.set_document_notice(Some(
                    "The file changed on disk, but the document has unsaved edits. Reload was not applied."
                        .into(),
                ));
                cx.notify();
                return;
            }
            Err(error) => {
                self.set_document_notice(Some(
                    format!("Could not prepare reload: {error:?}").into(),
                ));
                cx.notify();
                return;
            }
        };
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.opened_at = Some(Instant::now());
        self.first_frame_scheduled = None;
        self.set_document_notice(None);
        let build_preview = self.document_workspace.needs_reading();
        let background = cx
            .background_executor()
            .spawn_with_priority(gpui::Priority::High, async move {
                reload_workspace_document(request, build_preview)
            });
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = background.await;
            let _ = this.update(cx, |this, cx| {
                if this.apply_reload_result(generation, result, cx) {
                    cx.notify();
                }
            });
        }));
    }

    pub(crate) fn set_document_notice(&mut self, error: Option<Arc<str>>) {
        if let Some(document) = self.state.ready_mut() {
            document.notice = error;
        }
    }

    fn apply_reload_result(
        &mut self,
        generation: u64,
        result: Result<WorkspaceReloadedDocument, (PathBuf, String)>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !accept_generation(self.generation, generation) {
            return false;
        }
        let session = match self.state.ready() {
            Some(document) => document.session.clone(),
            None => return false,
        };
        let reloaded = match result {
            Ok(reloaded) => reloaded,
            Err((path, message)) => {
                self.set_document_notice(Some(
                    format!("Could not reload {}: {message}", path.display()).into(),
                ));
                return true;
            }
        };
        let (prepared, preview) = match reloaded {
            WorkspaceReloadedDocument::Source(prepared) => (prepared, None),
            WorkspaceReloadedDocument::Preview(reloaded) => {
                let (prepared, preview) = reloaded.into_parts();
                (prepared, Some(preview))
            }
        };
        if let Err(error) = session.update(cx, |session, cx| session.apply_reload(prepared, cx)) {
            self.set_document_notice(Some(format!("Reload was not applied: {error:?}").into()));
            return true;
        }
        if !self.document_workspace.needs_reading() {
            self.suspend_derived_preview();
        } else if let Some(preview) = preview {
            let document = Arc::new(preview);
            self.derived
                .pending
                .lock()
                .expect("derived request slot poisoned")
                .take();
            self.derived.latest = Some(document);
            self.reconcile_visible_reading_panes(cx);
        } else {
            self.derived.latest = None;
            self.schedule_derived_update(cx);
        }
        self.set_document_notice(None);
        true
    }

    pub(crate) fn watch_document_profiled(
        &mut self,
        path: PathBuf,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let directory = path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();
        if self.file_watch_task.is_some()
            && self.file_watch_directory.as_ref() == Some(&directory)
            && let Some(target) = self.file_watch_target.as_ref()
        {
            target.set(path);
            return;
        }

        let watch_started = Instant::now();
        let opened_at = self.opened_at.unwrap_or_else(Instant::now);
        self.file_watch_request = self.file_watch_request.wrapping_add(1);
        let request = self.file_watch_request;
        let target = crate::file_watcher::FileWatchTarget::new(path);
        self.file_watch_directory = Some(directory);
        self.file_watch_target = Some(target.clone());
        // `notify` can spend hundreds of milliseconds initializing FSEvents on macOS. Keep that
        // blocking setup off GPUI's executor; otherwise completion of an already-loaded document
        // can sit behind the watcher even though parsing took less than a millisecond.
        let (setup_sender, setup_receiver) = async_channel::bounded(1);
        let setup_started = std::thread::Builder::new()
            .name("org-studio-file-watch-setup".into())
            .spawn(move || {
                let _ = setup_sender.send_blocking(crate::file_watcher::FileWatch::new(target));
            });
        if setup_started.is_err() {
            self.file_watch_directory = None;
            self.file_watch_target = None;
            self.file_watch_task = None;
            self.set_document_notice(Some("Could not start the file watcher thread.".into()));
            cx.notify();
            return;
        }
        self.file_watch_task = Some(cx.spawn(async move |this, cx| {
            let watch = match setup_receiver.recv().await {
                Ok(Ok(watch)) => watch,
                Ok(Err(error)) => {
                    let message: Arc<str> = format!("Could not watch the document: {error}").into();
                    let _ = this.update(cx, |this, cx| {
                        if this.file_watch_request == request {
                            this.file_watch_task = None;
                            this.file_watch_directory = None;
                            this.file_watch_target = None;
                            this.set_document_notice(Some(message));
                            cx.notify();
                        }
                    });
                    return;
                }
                Err(_) => {
                    let _ = this.update(cx, |this, cx| {
                        if this.file_watch_request == request {
                            this.file_watch_task = None;
                            this.file_watch_directory = None;
                            this.file_watch_target = None;
                            this.set_document_notice(Some("The file watcher stopped during setup.".into()));
                            cx.notify();
                        }
                    });
                    return;
                }
            };
            if minimap::minimap_perf_enabled() {
                eprintln!(
                    "org_preview_file_watch_ready generation={} elapsed_ms={:.3} since_open_ms={:.3}",
                    generation,
                    watch_started.elapsed().as_secs_f64() * 1000.0,
                    opened_at.elapsed().as_secs_f64() * 1000.0,
                );
            }
            loop {
                let Some(changed_path) = watch.changed().await else {
                    let _ = this.update(cx, |this, _| {
                        if this.file_watch_request == request {
                            this.file_watch_task = None;
                            this.file_watch_directory = None;
                            this.file_watch_target = None;
                        }
                    });
                    return;
                };
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                watch.drain();
                let observed = cx
                    .background_executor()
                    .spawn({
                        let changed_path = changed_path.clone();
                        async move {
                            match crate::document::FileStamp::read(&changed_path) {
                                Ok(stamp) => Ok(Some(stamp)),
                                Err(error)
                                    if error.kind() == std::io::ErrorKind::NotFound =>
                                {
                                    Ok(None)
                                }
                                Err(error) => Err(error.to_string()),
                            }
                        }
                    })
                    .await;
                let keep_watching = this
                    .update(cx, |this, cx| {
                        if this.file_watch_request != request {
                            return false;
                        }
                        let active_session = this
                            .state
                            .ready()
                            .filter(|document| {
                                document.session.read(cx).path() == changed_path.as_path()
                            })
                            .map(|document| document.session.clone());
                        if let Some(session) = active_session {
                            match observed {
                                Ok(observed) => {
                                    let action = session.update(cx, |session, cx| {
                                        session.disk_changed(cx);
                                        session.observe_disk(observed)
                                    });
                                    match action {
                                        crate::document::DiskChangeAction::Ignore
                                        | crate::document::DiskChangeAction::Defer => {}
                                        crate::document::DiskChangeAction::Recovered => {
                                            this.set_document_notice(None);
                                            cx.notify();
                                        }
                                        crate::document::DiskChangeAction::Reload => {
                                            this.reload_current(cx)
                                        }
                                        crate::document::DiskChangeAction::Conflict => {
                                            this.set_document_notice(Some(
                                                "The file changed on disk while this document has unsaved edits. Your edits were kept; Save requires an explicit conflict choice."
                                                    .into(),
                                            ));
                                            cx.notify();
                                        }
                                        crate::document::DiskChangeAction::Missing => {
                                            this.set_document_notice(Some(
                                                "The file was removed or renamed on disk. Your in-memory document is safe; Save will recreate it, or use Save As."
                                                    .into(),
                                            ));
                                            cx.notify();
                                        }
                                    }
                                }
                                Err(message) => {
                                    this.set_document_notice(Some(
                                        format!("Could not inspect the changed file: {message}")
                                            .into(),
                                    ));
                                    cx.notify();
                                }
                            }
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep_watching {
                    return;
                }
            }
        }));
    }

    fn stop_document_watch(&mut self) {
        self.file_watch_request = self.file_watch_request.wrapping_add(1);
        self.file_watch_task = None;
        self.file_watch_directory = None;
        self.file_watch_target = None;
    }

    fn sync_document_watch(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self
            .state
            .ready()
            .map(|document| document.session.read(cx).path().to_path_buf())
        {
            self.watch_document_profiled(path, self.generation, cx);
        } else {
            self.stop_document_watch();
        }
    }

    pub(crate) fn apply_load_result<T>(
        &mut self,
        generation: u64,
        result: Result<T, (PathBuf, String)>,
        cx: &mut impl AppContext,
    ) -> bool
    where
        T: Into<WorkspaceLoadedDocument>,
    {
        if !accept_generation(self.generation, generation) {
            return false;
        }
        let previous = self.state.take_ready();
        self.state = match result {
            Ok(loaded) => {
                let (session, preview) = match loaded.into() {
                    WorkspaceLoadedDocument::Source(session) => (session, None),
                    WorkspaceLoadedDocument::Preview(loaded) => {
                        let (session, preview) = loaded.into_parts();
                        (session, Some(preview))
                    }
                };
                let preview = self
                    .document_workspace
                    .needs_reading()
                    .then_some(preview)
                    .flatten();
                if minimap::minimap_perf_enabled()
                    && let Some(preview) = preview.as_ref()
                {
                    eprintln!(
                        "org_preview_document_ready generation={} bytes={} rows={} read_ms={:.3} rope_ms={:.3} parse_ms={:.3} display_map_ms={:.3} load_total_ms={:.3} since_open_ms={:.3}",
                        generation,
                        preview.metrics.bytes,
                        preview.row_count(),
                        preview.metrics.read.as_secs_f64() * 1000.0,
                        preview.metrics.rope.as_secs_f64() * 1000.0,
                        preview.metrics.parse.as_secs_f64() * 1000.0,
                        preview.metrics.display_map.as_secs_f64() * 1000.0,
                        preview.metrics.total.as_secs_f64() * 1000.0,
                        self.opened_at
                            .map_or(0.0, |opened_at| opened_at.elapsed().as_secs_f64() * 1000.0),
                    );
                }
                crate::recent_documents::record_success(
                    &mut self.recent_documents,
                    session.path().to_path_buf(),
                );
                self.home_error = None;
                let session = cx.new(|_| session);
                self.editor_minimap_width_subscriptions.clear();
                let minimap_visible = self.minimap_visible
                    || (cfg!(feature = "benchmarks")
                        && std::env::var_os("ORG_STUDIO_EDITOR_MINIMAP_BENCH").is_some());
                let document_workspace = self.document_workspace;
                let soft_wrap = self.soft_wrap;
                let minimap_width = self.minimap_width;
                let mut create_editor = |pane| {
                    document_workspace
                        .shows(pane, crate::app::PaneSurface::Editor)
                        .then(|| {
                            let session = session.clone();
                            cx.new(|cx| {
                                let mut editor = crate::editor::SemanticEditor::new_with_autofocus(
                                    session,
                                    document_workspace.active_pane == pane,
                                    cx,
                                );
                                editor.set_soft_wrap(soft_wrap, cx);
                                editor.set_minimap(minimap_visible, minimap_width, cx);
                                editor
                            })
                        })
                };
                let left_editor = create_editor(crate::app::PaneSide::Left);
                let right_editor = create_editor(crate::app::PaneSide::Right);
                let preview = preview.map(Arc::new);
                self.derived.latest = preview.clone();
                let list_overdraw = self.list_overdraw;
                let readers = preview
                    .map(|document| PanePair {
                        left: document_workspace
                            .shows(crate::app::PaneSide::Left, crate::app::PaneSurface::Reading)
                            .then(|| {
                                cx.new({
                                    let document = document.clone();
                                    move |_| ReadingPreviewPanel::new(document, list_overdraw)
                                })
                            }),
                        right: document_workspace
                            .shows(
                                crate::app::PaneSide::Right,
                                crate::app::PaneSurface::Reading,
                            )
                            .then(|| {
                                cx.new(move |_| ReadingPreviewPanel::new(document, list_overdraw))
                            }),
                    })
                    .unwrap_or(PanePair {
                        left: None,
                        right: None,
                    });
                WorkspaceLoadState::Ready {
                    document: ReadyDocument {
                        session,
                        editors: PanePair {
                            left: left_editor,
                            right: right_editor,
                        },
                        readers,
                        notice: None,
                    },
                }
            }
            Err((path, message)) => {
                if previous.is_none() {
                    self.derived.latest = None;
                }
                WorkspaceLoadState::Failed {
                    path,
                    message,
                    previous,
                }
            }
        };
        self.apply_pending_navigation(generation, cx);
        true
    }

    pub(crate) fn apply_pending_navigation(&mut self, generation: u64, cx: &mut impl AppContext) {
        let Some((pending_generation, anchor)) = self.pending_navigation.clone() else {
            return;
        };
        if pending_generation != generation {
            self.pending_navigation = None;
            return;
        }
        let Some(ready) = self.state.ready() else {
            return;
        };
        let panels = [ready.readers.left.clone(), ready.readers.right.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if panels.is_empty() {
            return;
        }
        let found = panels
            .into_iter()
            .any(|panel| panel.update(cx, |panel, _| panel.jump_to_destination(&anchor)));
        self.pending_navigation = None;
        if !found {
            self.set_document_notice(Some("Link target was not found".into()));
        }
    }

    pub(crate) fn choose_file(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open Org document".into()),
        });

        self.picker_task = Some(cx.spawn_in(window, async move |this, cx| {
            let selected = receiver.await;
            if let Ok(Ok(Some(paths))) = selected
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| this.request_open(path, window, cx));
            }
        }));
    }

    pub(crate) fn open_recent(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if is_supported_document(&path) {
            self.open(path, cx);
        } else {
            crate::recent_documents::remove(&mut self.recent_documents, &path);
            self.home_error = Some(
                format!(
                    "The recent document is no longer available: {}",
                    path.display()
                )
                .into(),
            );
            cx.notify();
        }
    }

    pub(crate) fn clear_recent_documents(&mut self, cx: &mut Context<Self>) {
        crate::recent_documents::clear(&mut self.recent_documents);
        self.home_error = None;
        cx.notify();
    }

    pub(crate) fn open_dropped_paths(
        &mut self,
        paths: &gpui::ExternalPaths,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = paths
            .paths()
            .iter()
            .find(|path| is_supported_document(path))
        {
            self.request_open(path.clone(), window, cx);
            return;
        }
        self.home_error = Some("Drop an Org or Markdown document to open it.".into());
        cx.notify();
    }

    pub(crate) fn reload(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let path = match &self.state {
            WorkspaceLoadState::Loading { path, .. } | WorkspaceLoadState::Failed { path, .. } => {
                Some(path.clone())
            }
            WorkspaceLoadState::Ready { document } if document.session.read(cx).is_dirty() => {
                let path = document.session.read(cx).path().to_path_buf();
                let answer = window.prompt(
                    gpui::PromptLevel::Warning,
                    "Discard local edits and reload from disk?",
                    Some("Reloading will permanently discard the unsaved in-memory version."),
                    &[
                        gpui::PromptButton::ok("Reload from Disk"),
                        gpui::PromptButton::new("Open Disk Version"),
                        gpui::PromptButton::cancel("Cancel"),
                    ],
                    cx,
                );
                self.picker_task = Some(cx.spawn_in(window, async move |this, cx| {
                    let Ok(choice) = answer.await else {
                        return;
                    };
                    let _ = this.update_in(cx, |this, _window, cx| match choice {
                        0 => this.open_discarding_current(path.clone(), cx),
                        1 => cx.open_with_system(&path),
                        _ => {}
                    });
                }));
                None
            }
            WorkspaceLoadState::Ready { .. } => {
                self.reload_current(cx);
                None
            }
            WorkspaceLoadState::Empty => None,
        };
        if let Some(path) = path {
            self.open(path, cx);
        }
    }

    pub fn toggle_minimap(&mut self, cx: &mut Context<Self>) {
        self.minimap_visible = !self.minimap_visible;
        if let Some(document) = self.state.ready() {
            for editor in [&document.editors.left, &document.editors.right]
                .into_iter()
                .flatten()
            {
                editor.update(cx, |editor, cx| {
                    editor.set_minimap(self.minimap_visible, self.minimap_width, cx)
                });
            }
        }
        if self.minimap_visible {
            cx.background_spawn(async { minimap::prewarm_text_rasterizer() })
                .detach();
        }
        self.save_preview_settings();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn failed_open_keeps_the_active_document_title_and_watch_target(cx: &mut gpui::TestAppContext) {
        let root = std::env::temp_dir().join(format!(
            "org-studio-failed-open-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let old_directory = root.join("old");
        let new_directory = root.join("new");
        std::fs::create_dir_all(&old_directory).unwrap();
        std::fs::create_dir_all(&new_directory).unwrap();
        let old_path = old_directory.join("old.org");
        let failed_path = new_directory.join("missing.org");
        std::fs::write(&old_path, b"* Active").unwrap();
        let loaded = load_workspace_document(old_path.clone(), false).unwrap();
        let active_path = old_path.clone();
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));

        workspace.update(cx, |workspace, cx| {
            workspace.generation = 1;
            assert!(workspace.apply_load_result(1, Ok(loaded), cx));
            let generation = workspace.begin_open(failed_path.clone(), Instant::now());
            assert!(workspace.apply_load_result(
                generation,
                Err::<WorkspaceLoadedDocument, _>((failed_path, "could not load".into())),
                cx,
            ));
            workspace.sync_document_watch(cx);
            assert_eq!(
                workspace.document_session().unwrap().read(cx).path(),
                active_path
            );
            assert_eq!(
                workspace.file_watch_directory.as_deref(),
                active_path.parent()
            );
        });
        assert_eq!(
            cx.read(|app| workspace.read(app).window_title(app)),
            "old.org"
        );
        workspace.update(cx, |workspace, _| workspace.stop_document_watch());
        std::fs::remove_dir_all(root).unwrap();
    }
}
