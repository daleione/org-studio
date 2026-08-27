use super::{
    Arc, Context, DocumentFormat, Duration, HashSet, InitialDocumentLoad, Instant,
    MAX_EXACT_SCROLL_LAYOUT_ROWS, PathBuf, PathPromptOptions, PreviewApp, PreviewDocument,
    PreviewLoadState, accept_generation, load_document, minimap, schedule_document_prewarm,
    visible_markdown_row_indices, visible_row_indices,
};
use gpui::AppContext;

impl PreviewApp {
    pub(in crate::preview) fn begin_open(
        &mut self,
        path: PathBuf,
        opened_at: Instant,
        cx: &mut Context<Self>,
    ) -> u64 {
        crate::settings::remember_last_document(&path);
        self.cancel_minimap_interaction();
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        self.generation += 1;
        self.opened_at = Some(opened_at);
        self.first_frame_scheduled = None;
        let generation = self.generation;
        self.state = PreviewLoadState::Loading { path: path.clone() };
        let watch_started = Instant::now();
        self.watch_document(path.clone(), cx);
        if minimap::minimap_perf_enabled() {
            eprintln!(
                "org_preview_file_watch_ready generation={} elapsed_ms={:.3} since_open_ms={:.3}",
                generation,
                watch_started.elapsed().as_secs_f64() * 1000.0,
                self.opened_at
                    .map_or(0.0, |opened_at| opened_at.elapsed().as_secs_f64() * 1000.0),
            );
        }
        generation
    }

    pub fn open_initial(&mut self, load: InitialDocumentLoad, cx: &mut Context<Self>) {
        let InitialDocumentLoad {
            path,
            started_at,
            minimap_prewarm_scheduled,
            receiver,
        } = load;
        let generation = self.begin_open(path.clone(), started_at, cx);
        let prewarm_minimap = self.minimap_visible;

        match receiver.try_recv() {
            Ok(result) => {
                if !minimap_prewarm_scheduled {
                    schedule_document_prewarm(prewarm_minimap, &result, cx.background_executor());
                }
                if self.apply_load_result(generation, result) {
                    cx.notify();
                }
            }
            Err(async_channel::TryRecvError::Empty) => {
                let prewarm_executor = cx.background_executor().clone();
                self.load_task = Some(cx.spawn(async move |this, cx| {
                    let result = receiver.recv().await.unwrap_or_else(|_| {
                        Err((path, "initial document loader stopped".to_owned()))
                    });
                    if !minimap_prewarm_scheduled {
                        schedule_document_prewarm(prewarm_minimap, &result, &prewarm_executor);
                    }
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
        let generation = self.begin_open(path.clone(), Instant::now(), cx);

        // Opening the requested document is user-visible latency. Submit it before minimap
        // prewarming and at GPUI's high background priority so platform/font startup work cannot
        // leave a sub-millisecond file load queued for hundreds of milliseconds.
        let opened_at = self.opened_at.unwrap_or_else(Instant::now);
        let prewarm_minimap = self.minimap_visible;
        let prewarm_executor = cx.background_executor().clone();
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
                    schedule_document_prewarm(prewarm_minimap, &result, &prewarm_executor);
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

        cx.notify();
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
                let document = Arc::new(document);
                self.folded = Arc::new(HashSet::new());
                self.visible_rows = Arc::new(match document.format {
                    DocumentFormat::Org => visible_row_indices(
                        &document.projection.rows,
                        &document.blocks,
                        &self.folded,
                    ),
                    DocumentFormat::Markdown => visible_markdown_row_indices(
                        &document.projection.rows,
                        &document.markdown_blocks,
                        &self.folded,
                    ),
                });
                self.list_state.reset(self.visible_rows.len());
                if self.visible_rows.len() <= MAX_EXACT_SCROLL_LAYOUT_ROWS {
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

    pub(in crate::preview) fn watch_document(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.file_watch_request = self.file_watch_request.wrapping_add(1);
        let request = self.file_watch_request;
        if let Ok(watch) = crate::file_watcher::FileWatch::new(path.clone()) {
            self.file_watch_task = Some(cx.spawn(async move |this, cx| {
                if !watch.changed().await {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                watch.drain();
                let _ = this.update(cx, |this, cx| {
                    if this.file_watch_request == request {
                        this.open(path, cx);
                    }
                });
            }));
            return;
        }

        self.file_watch_task = None;
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
