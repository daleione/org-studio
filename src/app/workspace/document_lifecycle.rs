use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{Context, PathPromptOptions};

use crate::preview::{WorkspaceReloadedDocument, reload_workspace_document};
use crate::{
    app::{ContentRoute, PanePair, ReadyDocument, WorkspaceLoadState, WorkspaceWindow},
    motion::{MINIMAP_MOTION, Tween},
    preview::{
        InitialDocumentLoad, ReadingPreviewPanel, WorkspaceLoadedDocument, accept_generation,
        is_supported_document, load_workspace_document, minimap,
    },
};
use gpui::AppContext;

impl WorkspaceWindow {
    pub(crate) fn minimap_reveal_at(&self, now: Instant) -> (f32, bool) {
        let Some(animation) = self.minimap_visibility_animation else {
            return (if self.minimap_visible { 1.0 } else { 0.0 }, false);
        };
        let sample = animation.sample(now);
        (sample.value, sample.active)
    }

    pub(crate) fn show_home_now(&mut self, cx: &mut Context<Self>) {
        self.end_prefix(cx);
        self.close_command_line(cx);
        self.close_search(false, cx);
        if let Some(document) = self.state.take_ready() {
            self.park_document(document);
        }
        self.suspend_derived_preview();
        self.derived.latest = None;
        self.generation = self.generation.wrapping_add(1);
        self.load_task = None;
        self.stop_document_watch();
        self.editor_subscriptions.clear();
        self.save.error = None;
        self.state = WorkspaceLoadState::Empty;
        self.opened_at = None;
        self.first_frame_scheduled = None;
        self.home_error = None;
        self.pending_navigation = None;
        self.pending_link_surface = None;
        self.image_viewer.clear(cx);
        self.pending_surface_anchors = PanePair {
            left: None,
            right: None,
        };
        self.content_route = ContentRoute::Document;
        self.file_manager.reset_for_document();
        self.stop_dired_directory_watch();
        self.install_document_keymap();
        self.focus_workspace_on_render = true;
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
        self.save.error = None;
        self.pending_navigation = None;
        self.pending_link_surface = None;
        self.pending_surface_anchors = PanePair {
            left: None,
            right: None,
        };
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
        if self.buffer_busy() {
            return;
        }
        if crate::preview::is_supported_image(&path) {
            if let Err(message) = self.open_image_viewer(path, cx) {
                self.set_document_notice(Some(message.into()));
                cx.notify();
            }
            return;
        }
        self.image_viewer.clear(cx);
        if let Some(session) = self.buffer_for_path(&path, cx) {
            let id = session.read(cx).id();
            self.activate_buffer(id, cx);
            return;
        }
        if matches!(&self.state, WorkspaceLoadState::Loading { path: pending, .. } if crate::app::buffers::same_file(pending, &path))
        {
            return;
        }
        self.background_pending_open(cx);
        let pending_key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        // Promoting an in-flight background load replaces its callback; only the
        // foreground completion may install the new active document.
        self.buffers.loads.remove(&pending_key);
        self.close_command_line(cx);
        self.close_search(false, cx);
        self.dismiss_buffer_panel(cx);
        self.content_route = ContentRoute::Document;
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
        let navigation_origin = self.capture_navigation_location(cx);
        if !preserve_previous {
            self.stop_document_watch();
        }
        let generation =
            self.begin_open_with_previous(path.clone(), Instant::now(), preserve_previous);
        let build_preview = self.document_workspace.needs_reading()
            && !crate::preview::is_editor_only_document(&path);

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
                if generation == this.generation && result.is_ok() {
                    this.remember_navigation_location(navigation_origin);
                }
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
        let build_preview = self.document_workspace.needs_reading()
            && !crate::preview::is_editor_only_document(request.path());
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
        self.set_echo_message(error.map(crate::app::echo_area::EchoMessage::error));
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
        let (setup_sender, setup_receiver) = std::sync::mpsc::sync_channel(1);
        let setup_started = std::thread::Builder::new()
            .name("org-studio-file-watch-setup".into())
            .spawn(move || {
                let _ = setup_sender.send(crate::file_watcher::FileWatch::new(target));
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
            // Match the buffer watcher: external threads do not wake GPUI tasks directly.
            let setup = loop {
                match setup_receiver.try_recv() {
                    Ok(result) => break Ok(result),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break Err(()),
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        cx.background_executor().timer(Duration::from_millis(50)).await;
                    }
                }
            };
            let watch = match setup {
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

    pub(crate) fn sync_document_watch(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self
            .state
            .ready()
            .and_then(|document| document.session.read(cx).file_path().map(PathBuf::from))
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
        let linked_surface =
            self.pending_link_surface
                .take()
                .and_then(|(pending_generation, surface)| {
                    (pending_generation == generation).then_some(surface)
                });
        let previous = self.state.take_ready();
        self.state = match result {
            Ok(loaded) => {
                if let Some(previous) = previous {
                    self.park_document(previous);
                }
                // Soft wrap is deliberately document-local. A transient M-z
                // choice must never leak into the next opened document.
                self.soft_wrap = true;
                let (session, preview) = match loaded.into() {
                    WorkspaceLoadedDocument::Source(session) => (*session, None),
                    WorkspaceLoadedDocument::Preview(loaded) => {
                        let (session, preview) = loaded.into_parts();
                        (session, Some(preview))
                    }
                };
                if crate::preview::is_editor_only_document(session.syntax_path()) {
                    for pane in [crate::app::PaneSide::Left, crate::app::PaneSide::Right] {
                        self.document_workspace
                            .set_surface(pane, crate::app::PaneSurface::Editor);
                    }
                } else if let Some(surface) = linked_surface {
                    self.document_workspace
                        .set_surface(self.document_workspace.active_pane, surface);
                }
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
                if let Some(path) = session.file_path() {
                    crate::recent_documents::record_success(
                        &mut self.recent_documents,
                        path.to_path_buf(),
                    );
                }
                self.home_error = None;
                let session = cx.new(|_| session);
                let editor_syntax = Arc::new(crate::editor::EditorSyntaxService::default());
                self.editor_subscriptions.clear();
                let minimap_visible = if cfg!(feature = "benchmarks") {
                    std::env::var("ORG_STUDIO_EDITOR_MINIMAP_BENCH")
                        .ok()
                        .map(|value| !matches!(value.as_str(), "0" | "false" | "off"))
                        .unwrap_or(self.minimap_visible)
                } else {
                    self.minimap_visible
                };
                let document_workspace = self.document_workspace;
                let language = self.language;
                let soft_wrap = self.soft_wrap;
                let minimap_width = self.minimap_width;
                let content_font_sizes = self.content_font_sizes.clone();
                let mut create_editor = |pane| {
                    document_workspace
                        .shows(pane, crate::app::PaneSurface::Editor)
                        .then(|| {
                            let session = session.clone();
                            let editor_syntax = editor_syntax.clone();
                            cx.new(|cx| {
                                let mut editor =
                                    crate::editor::SemanticEditor::new_with_syntax_service(
                                        session,
                                        document_workspace.active_pane == pane,
                                        editor_syntax,
                                        cx,
                                    );
                                editor.set_ui_language(language, cx);
                                editor.set_content_font_size(*content_font_sizes.get(pane), cx);
                                editor.set_bottom_overlay_clearance(
                                    crate::app::status_line::FLOATING_STATUS_CLEARANCE,
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
                let content_font_sizes = self.content_font_sizes.clone();
                let readers = preview
                    .map(|document| PanePair {
                        left: document_workspace
                            .shows(crate::app::PaneSide::Left, crate::app::PaneSurface::Reading)
                            .then(|| {
                                cx.new({
                                    let document = document.clone();
                                    let content_font_size =
                                        *content_font_sizes.get(crate::app::PaneSide::Left);
                                    move |_| {
                                        let mut panel =
                                            ReadingPreviewPanel::new(document, list_overdraw);
                                        panel.set_content_font_size(content_font_size);
                                        panel
                                    }
                                })
                            }),
                        right: document_workspace
                            .shows(
                                crate::app::PaneSide::Right,
                                crate::app::PaneSurface::Reading,
                            )
                            .then(|| {
                                let content_font_size =
                                    *content_font_sizes.get(crate::app::PaneSide::Right);
                                cx.new(move |_| {
                                    let mut panel =
                                        ReadingPreviewPanel::new(document, list_overdraw);
                                    panel.set_content_font_size(content_font_size);
                                    panel
                                })
                            }),
                    })
                    .unwrap_or(PanePair {
                        left: None,
                        right: None,
                    });
                WorkspaceLoadState::Ready {
                    document: ReadyDocument {
                        session,
                        editor_syntax,
                        editors: PanePair {
                            left: left_editor,
                            right: right_editor,
                        },
                        readers,
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
        if matches!(self.state, WorkspaceLoadState::Failed { .. }) {
            self.agenda.pending_text_task = None;
            self.agenda.pending_text_generation = None;
        }
        self.apply_pending_navigation(generation, cx);
        true
    }

    pub(crate) fn apply_pending_navigation(&mut self, generation: u64, cx: &mut impl AppContext) {
        // Loading can retain the previous document for display; it must not consume
        // a destination belonging to the incoming document.
        if !matches!(self.state, WorkspaceLoadState::Ready { .. }) {
            return;
        }
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
            multiple: true,
            prompt: Some(self.buffer_text("打开文档", "Open documents").into()),
        });

        self.picker_task = Some(cx.spawn_in(window, async move |this, cx| {
            let selected = receiver.await;
            if let Ok(Ok(Some(paths))) = selected {
                let _ = this.update_in(cx, |this, _, cx| this.open_buffers(paths, cx));
            }
        }));
    }

    pub(crate) fn open_recent(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if is_supported_document(&path) || crate::preview::is_supported_image(&path) {
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
        _window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let paths = paths
            .paths()
            .iter()
            .filter(|path| is_supported_document(path) || crate::preview::is_supported_image(path))
            .cloned()
            .collect::<Vec<_>>();
        if !paths.is_empty() {
            self.open_buffers(paths, cx);
            return;
        }
        self.home_error = Some("Drop a supported text document to open it.".into());
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
        let now = Instant::now();
        let from = self.minimap_reveal_at(now).0;
        let animate = !cx.reduce_motion();
        self.minimap_visible = !self.minimap_visible;
        self.minimap_visibility_animation = animate.then_some(Tween::new(
            now,
            from,
            if self.minimap_visible { 1.0 } else { 0.0 },
            MINIMAP_MOTION,
        ));
        if !self.minimap_visible {
            self.minimap_resize_preview = None;
            self.cancel_minimap_interaction(cx);
        }
        if !animate {
            self.propagate_editor_minimap_settings(cx);
        }
        if self.minimap_visible {
            cx.background_spawn(async {
                crate::editor::prewarm_minimap_text_rasterizer();
                minimap::prewarm_text_rasterizer();
            })
            .detach();
        }
        self.save_preview_settings();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_is_the_single_source_of_minimap_animation_progress() {
        let started_at = Instant::now();
        let mut workspace = WorkspaceWindow::with_split_layout(false);
        workspace.minimap_visible = false;
        workspace.minimap_visibility_animation =
            Some(Tween::new(started_at, 1.0, 0.0, MINIMAP_MOTION));

        assert_eq!(workspace.minimap_reveal_at(started_at), (1.0, true));
        let halfway = workspace
            .minimap_reveal_at(started_at + MINIMAP_MOTION.duration() / 2)
            .0;
        assert!(halfway > 0.0 && halfway < 1.0);
        assert_eq!(
            workspace.minimap_reveal_at(started_at + MINIMAP_MOTION.duration()),
            (0.0, false)
        );
    }

    #[gpui::test]
    fn minimap_toggle_finishes_immediately_when_motion_is_reduced(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));

        workspace.update(cx, |workspace, cx| {
            workspace.minimap_visible = true;
            workspace.minimap_visibility_animation = None;
            workspace.toggle_minimap(cx);
            assert!(!workspace.minimap_visible);
            assert!(workspace.minimap_visibility_animation.is_none());
            assert_eq!(workspace.minimap_reveal_at(Instant::now()), (0.0, false));
        });
    }

    #[gpui::test]
    fn opening_a_document_always_restores_soft_wrap(cx: &mut gpui::TestAppContext) {
        let path = std::env::temp_dir().join(format!(
            "org-studio-soft-wrap-default-{}-{}.org",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"* Wrapped by default\nbody\n").unwrap();
        let loaded = load_workspace_document(path.clone(), false).unwrap();
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));

        workspace.update(cx, |workspace, cx| {
            workspace.soft_wrap = false;
            workspace.generation = 1;
            assert!(workspace.apply_load_result(1, Ok(loaded), cx));
            assert!(workspace.soft_wrap);
            assert!(
                workspace
                    .editor(crate::app::PaneSide::Left)
                    .unwrap()
                    .read(cx)
                    .soft_wrap()
            );
        });

        std::fs::remove_file(path).unwrap();
    }

    #[gpui::test]
    fn opening_source_file_selects_editor_even_from_reading_mode(cx: &mut gpui::TestAppContext) {
        let path =
            std::env::temp_dir().join(format!("org-studio-source-file-{}.rs", std::process::id()));
        std::fs::write(&path, b"fn main() {}\n").unwrap();
        assert!(is_supported_document(&path));
        let loaded = load_workspace_document(path.clone(), false).unwrap();
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        workspace.update(cx, |workspace, cx| {
            workspace
                .document_workspace
                .set_surface(crate::app::PaneSide::Left, crate::app::PaneSurface::Reading);
            workspace.generation = 1;
            assert!(workspace.apply_load_result(1, Ok(loaded), cx));
            assert_eq!(
                workspace.document_workspace.active_surface(),
                crate::app::PaneSurface::Editor
            );
            assert!(workspace.editor(crate::app::PaneSide::Left).is_some());
            assert!(workspace.derived.latest.is_none());
            workspace.show_reading(cx);
            assert_eq!(
                workspace.document_workspace.active_surface(),
                crate::app::PaneSurface::Editor
            );
        });
        std::fs::remove_file(path).unwrap();
    }

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
