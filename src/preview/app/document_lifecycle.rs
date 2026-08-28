use super::{
    Arc, Context, Duration, HashSet, InitialDocumentLoad, Instant, PathBuf, PathPromptOptions,
    PreviewApp, PreviewDocument, PreviewLoadState, accept_generation, load_document, minimap,
    should_eagerly_measure_rows,
};
use gpui::AppContext;

impl PreviewApp {
    pub(in crate::preview) fn show_home(&mut self, cx: &mut Context<Self>) {
        self.discard_fold_animation();
        self.generation = self.generation.wrapping_add(1);
        self.load_task = None;
        self.file_watch_request = self.file_watch_request.wrapping_add(1);
        self.file_watch_task = None;
        self.file_watch_directory = None;
        self.file_watch_target = None;
        self.cancel_minimap_interaction();
        self.state = PreviewLoadState::Empty;
        self.last_ready = None;
        self.opened_at = None;
        self.first_frame_scheduled = None;
        self.home_error = None;
        self.content_route = super::super::ContentRoute::Document;
        self.sidebar_focused = false;
        self.dired_task = None;
        self.dired_scan_transaction = None;
        self.dired_refresh_pending = false;
        self.stop_dired_directory_watch();
        self.install_preview_keymap();
        cx.notify();
    }

    pub(in crate::preview) fn begin_open(&mut self, path: PathBuf, opened_at: Instant) -> u64 {
        self.discard_fold_animation();
        self.home_error = None;
        self.cancel_minimap_interaction();
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        self.generation += 1;
        self.opened_at = Some(opened_at);
        self.first_frame_scheduled = None;
        let generation = self.generation;
        self.state = PreviewLoadState::Loading { path: path.clone() };
        generation
    }

    pub fn open_initial(&mut self, load: InitialDocumentLoad, cx: &mut Context<Self>) {
        let InitialDocumentLoad {
            path,
            started_at,
            receiver,
        } = load;
        let generation = self.begin_open(path.clone(), started_at);
        self.watch_document_profiled(path.clone(), generation, cx);

        match receiver.try_recv() {
            Ok(result) => {
                if self.apply_load_result(generation, result) {
                    cx.notify();
                }
            }
            Err(async_channel::TryRecvError::Empty) => {
                self.load_task = Some(cx.spawn(async move |this, cx| {
                    let result = receiver.recv().await.unwrap_or_else(|_| {
                        Err((path, "initial document loader stopped".to_owned()))
                    });
                    let _ = this.update(cx, |this, cx| {
                        if this.apply_load_result(generation, result) {
                            cx.notify();
                        }
                    });
                }));
            }
            Err(async_channel::TryRecvError::Closed) => {
                let result = Err((path, "initial document loader stopped".to_owned()));
                if self.apply_load_result(generation, result) {
                    cx.notify();
                }
            }
        }
    }

    pub fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let generation = self.begin_open(path.clone(), Instant::now());

        // Opening the requested document is user-visible latency. Submit it before synchronous
        // file-watcher setup and before one-time font startup work so a small local file is not
        // queued behind either of them.
        let opened_at = self.opened_at.unwrap_or_else(Instant::now);
        let watch_path = path.clone();
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
                    let result = load_document(path);
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
                if this.apply_load_result(generation, result) {
                    cx.notify();
                }
            });
        }));
        self.watch_document_profiled(watch_path, generation, cx);

        cx.notify();
    }

    fn watch_document_profiled(&mut self, path: PathBuf, generation: u64, cx: &mut Context<Self>) {
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
            return;
        }
        self.file_watch_task = Some(cx.spawn(async move |this, cx| {
            let Ok(Ok(watch)) = setup_receiver.recv().await else {
                let _ = this.update(cx, |this, _| {
                    if this.file_watch_request == request {
                        this.file_watch_task = None;
                        this.file_watch_directory = None;
                        this.file_watch_target = None;
                    }
                });
                return;
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
                let keep_watching = this
                    .update(cx, |this, cx| {
                        if this.file_watch_request != request {
                            return false;
                        }
                        if this.current_document_path() == Some(changed_path.as_path()) {
                            this.open(changed_path, cx);
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

    pub(in crate::preview) fn apply_load_result(
        &mut self,
        generation: u64,
        result: Result<PreviewDocument, (PathBuf, String)>,
    ) -> bool {
        if !accept_generation(self.generation, generation) {
            return false;
        }
        self.state = match result {
            Ok(document) => {
                if minimap::minimap_perf_enabled() {
                    eprintln!(
                        "org_preview_document_ready generation={} bytes={} rows={} read_ms={:.3} rope_ms={:.3} parse_ms={:.3} display_map_ms={:.3} load_total_ms={:.3} since_open_ms={:.3}",
                        generation,
                        document.metrics.bytes,
                        document.projection.presentation.len(),
                        document.metrics.read.as_secs_f64() * 1000.0,
                        document.metrics.rope.as_secs_f64() * 1000.0,
                        document.metrics.parse.as_secs_f64() * 1000.0,
                        document.metrics.display_map.as_secs_f64() * 1000.0,
                        document.metrics.total.as_secs_f64() * 1000.0,
                        self.opened_at
                            .map_or(0.0, |opened_at| opened_at.elapsed().as_secs_f64() * 1000.0),
                    );
                }
                crate::recent_documents::record_success(
                    &mut self.recent_documents,
                    document.path.clone(),
                );
                self.home_error = None;
                let document = Arc::new(document);
                self.fold_markers = Arc::new(HashSet::new());
                self.global_visibility = super::super::GlobalVisibility::All;
                self.global_cycle_contiguous = false;
                self.local_cycle_continuation = None;
                self.discard_fold_animation();
                self.visible_rows = Arc::new((0..document.projection.rows.len()).collect());
                self.list_state.reset(self.visible_rows.len());
                if should_eagerly_measure_rows(self.visible_rows.len()) {
                    self.list_state.clone().measure_all();
                }
                self.last_ready = Some((generation, document.clone()));
                PreviewLoadState::Ready {
                    generation,
                    document,
                }
            }
            Err((path, message)) => PreviewLoadState::Failed { path, message },
        };
        true
    }

    pub(in crate::preview) fn choose_file(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open Org document".into()),
        });

        self.picker_task = Some(cx.spawn(async move |this, cx| {
            let selected = receiver.await;
            if let Ok(Ok(Some(paths))) = selected
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| this.open(path, cx));
            }
        }));
    }

    pub(in crate::preview) fn open_recent(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if super::super::is_supported_document(&path) {
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

    pub(in crate::preview) fn clear_recent_documents(&mut self, cx: &mut Context<Self>) {
        crate::recent_documents::clear(&mut self.recent_documents);
        self.home_error = None;
        cx.notify();
    }

    pub(in crate::preview) fn open_dropped_paths(
        &mut self,
        paths: &gpui::ExternalPaths,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = paths
            .paths()
            .iter()
            .find(|path| super::super::is_supported_document(path))
        {
            self.open(path.clone(), cx);
            return;
        }
        self.home_error = Some("Drop an Org or Markdown document to open it.".into());
        cx.notify();
    }

    pub(in crate::preview) fn reload(&mut self, cx: &mut Context<Self>) {
        let path = match &self.state {
            PreviewLoadState::Loading { path } | PreviewLoadState::Failed { path, .. } => {
                Some(path.clone())
            }
            PreviewLoadState::Ready { document, .. } => Some(document.path.clone()),
            PreviewLoadState::Empty => None,
        };
        if let Some(path) = path {
            self.open(path, cx);
        }
    }

    pub fn toggle_minimap(&mut self, cx: &mut Context<Self>) {
        self.minimap_visible = !self.minimap_visible;
        if self.minimap_visible {
            cx.background_spawn(async { minimap::prewarm_text_rasterizer() })
                .detach();
        }
        self.save_preview_settings();
        cx.notify();
    }
}
