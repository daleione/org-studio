use super::{
    Arc, BlockId, BuiltinCommand, CapabilitySet, CommandDispatcher, CommandImplementation,
    CommandKey, ContentRoute, Context, DocumentFormat, Duration, EmacsOutcome, FoldMeasurement,
    FoldTransitionInput, FoldTransitionPlan, HashMap, HashSet, InitialDocumentLoad, Instant,
    InvocationOrigin, KEY_FEEDBACK_DURATION, KeyDownEvent, KeyStroke,
    LOCAL_FOLD_ANIMATION_DURATION, ListAlignment, ListState, LocalCycleProjection, PathBuf,
    PathPromptOptions, PrefixArgument, PreviewApp, PreviewDocument, PreviewLoadState, RefCell,
    Window, accept_generation, built_in_contexts, changed_range, command_count,
    compile_input_profile, configured_minimap_visible, current_theme,
    cycle_markdown_subtree_visibility, cycle_org_subtree_visibility, dired_bindings,
    global_markdown_visibility, global_org_visibility, load_document, minimap, preview_bindings,
    preview_input, px, render_document, render_home, render_loading, should_eagerly_measure_rows,
};
use gpui::{div, prelude::*, rgb};

mod benchmark;
mod commands;
mod document_lifecycle;
pub(super) use benchmark::ScrollBenchmark;

struct GlobalVisibilityProjection {
    document: Arc<PreviewDocument>,
    visibility: super::GlobalVisibility,
    visible_rows: Vec<usize>,
    fold_markers: HashSet<BlockId>,
}

impl Default for PreviewApp {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewApp {
    pub fn new() -> Self {
        let list_overdraw = std::env::var("ORG_STUDIO_LIST_OVERDRAW")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(80.0);
        let (commands, keyboard, key_context) = preview_input();
        let preview_settings = crate::settings::PreviewSettings::load();
        let minimap_visible = configured_minimap_visible();
        Self {
            language: preview_settings.language,
            focus_handle: None,
            focus_lost_subscription: None,
            commands,
            keyboard,
            key_context,
            state: PreviewLoadState::Empty,
            recent_documents: crate::recent_documents::load(),
            home_error: None,
            generation: 0,
            load_task: None,
            file_watch_task: None,
            file_watch_request: 0,
            file_watch_directory: None,
            file_watch_target: None,
            dired_watch_task: None,
            dired_watch_request: 0,
            dired_watch_directory: None,
            picker_task: None,
            export_task: None,
            export_cancel: None,
            export_request: 0,
            export_panel: None,
            export_status: None,
            list_state: ListState::new(0, ListAlignment::Top, px(list_overdraw)),
            fold_markers: Arc::new(HashSet::new()),
            visible_rows: Arc::new(Vec::new()),
            last_ready: None,
            opened_at: None,
            first_frame_scheduled: None,
            scroll_benchmark: std::env::var("ORG_STUDIO_SCROLL_BENCH_FRAMES")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|frames| *frames > 0)
                .map(|target_frames| ScrollBenchmark {
                    target_frames,
                    warmup_remaining: std::env::var("ORG_STUDIO_SCROLL_BENCH_WARMUP_FRAMES")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0),
                    sampling_started: false,
                    scroll_pixels: std::env::var("ORG_STUDIO_SCROLL_BENCH_PIXELS")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(640.0),
                    samples: Vec::with_capacity(target_frames),
                    last_frame: Instant::now(),
                }),
            which_key_task: None,
            which_key_request: 0,
            key_feedback_task: None,
            key_feedback_request: 0,
            which_key_items: Arc::new(Vec::new()),
            dired_help_visible: false,
            content_route: ContentRoute::Document,
            sidebar_visible: false,
            sidebar_focused: false,
            sidebar_width: crate::settings::initial_sidebar_width(preview_settings.sidebar_width),
            sidebar_resize: None,
            minimap_visible,
            minimap_thumb_visibility: crate::settings::initial_minimap_thumb_visibility(
                preview_settings.minimap_thumb_visibility,
            ),
            minimap_width: crate::settings::initial_minimap_width(preview_settings.minimap_width),
            minimap_resize_preview: None,
            global_visibility: super::GlobalVisibility::All,
            global_cycle_contiguous: false,
            local_cycle_continuation: None,
            fold_animation_revision: 0,
            fold_animation: None,
            presentation_revision: 0,
            viewport_revision_key: None,
            minimap_pending_seek: None,
            minimap_seek_scheduled: false,
            dired: None,
            dired_status: None,
            dired_task: None,
            dired_scan_transaction: None,
            dired_refresh_pending: false,
            dired_operation_task: None,
            dired_operation_busy: false,
            dired_context_menu: None,
            dired_list_state: ListState::new(0, ListAlignment::Top, px(80.0)),
            sidebar_list_state: ListState::new(0, ListAlignment::Top, px(60.0)),
            dired_pending_presentation: None,
            sidebar_pending_presentation: None,
            dired_presentation_scheduled: false,
            dired_viewport_memory: HashMap::new(),
            sidebar_viewport_memory: HashMap::new(),
            status_line_settings: preview_settings.status_line,
            status_popover: None,
            status_layout_cache: RefCell::new(HashMap::new()),
        }
    }

    pub(super) fn save_preview_settings(&self) {
        crate::settings::PreviewSettings {
            language: self.language,
            minimap_enabled: self.minimap_visible,
            minimap_thumb_visibility: self.minimap_thumb_visibility,
            minimap_width: self.minimap_width,
            sidebar_width: self.sidebar_width,
            status_line: self.status_line_settings,
        }
        .save_async();
    }

    pub fn language(&self) -> crate::i18n::Language {
        self.language
    }

    pub(super) fn set_language(&mut self, language: crate::i18n::Language, cx: &mut Context<Self>) {
        if self.language != language {
            self.language = language;
            self.export_status = None;
            self.save_preview_settings();
            cx.notify();
        }
    }

    pub(super) fn change_minimap_width(
        &mut self,
        change: minimap::MinimapWidthChange,
        cx: &mut Context<Self>,
    ) {
        match change {
            minimap::MinimapWidthChange::Preview(width) => {
                if self.minimap_resize_preview != Some(width) {
                    self.minimap_resize_preview = Some(width);
                    cx.notify();
                }
            }
            minimap::MinimapWidthChange::Commit(width) => {
                self.minimap_resize_preview = None;
                let width = width.round().clamp(48.0, minimap::MINIMAP_MANUAL_MAX_PX) as u16;
                if self.minimap_width != Some(width) {
                    self.minimap_width = Some(width);
                    self.presentation_revision = self.presentation_revision.wrapping_add(1);
                    self.cancel_minimap_interaction();
                    self.save_preview_settings();
                }
                cx.notify();
            }
            minimap::MinimapWidthChange::Reset => {
                self.minimap_resize_preview = None;
                if self.minimap_width.take().is_some() {
                    self.presentation_revision = self.presentation_revision.wrapping_add(1);
                    self.cancel_minimap_interaction();
                    self.save_preview_settings();
                }
                cx.notify();
            }
        }
    }

    pub fn minimap_visible(&self) -> bool {
        self.minimap_visible
    }

    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    pub fn current_document_path(&self) -> Option<&std::path::Path> {
        match &self.state {
            PreviewLoadState::Loading { path } | PreviewLoadState::Failed { path, .. } => {
                Some(path)
            }
            PreviewLoadState::Ready { document, .. } => Some(&document.path),
            PreviewLoadState::Empty => None,
        }
    }

    pub(super) fn body(
        &self,
        entity: gpui::Entity<Self>,
        editor_width: f32,
        window: &Window,
    ) -> gpui::Div {
        let theme = current_theme();
        let minimap_width = minimap::width_for_viewport(editor_width, self.minimap_width);
        let content = match &self.state {
            PreviewLoadState::Empty => render_home(
                entity.clone(),
                &self.recent_documents,
                self.home_error.as_deref(),
                None,
                self.language,
            ),
            PreviewLoadState::Loading { path } => render_loading(path, self.language),
            PreviewLoadState::Failed { path, message } => {
                let error = format!("{}: {message}", path.display());
                if let Some((generation, document)) = &self.last_ready {
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .bg(rgb(theme.background))
                        .child(
                            div()
                                .flex_none()
                                .px_6()
                                .py_3()
                                .bg(rgb(0xfff2f0))
                                .border_b_1()
                                .border_color(rgb(0xf2c8c2))
                                .text_size(px(13.0))
                                .text_color(rgb(0xa12b1f))
                                .child(format!(
                                    "Could not open document. Showing the previous file. {error}"
                                )),
                        )
                        .child(render_document(
                            document.clone(),
                            self.list_state.clone(),
                            self.visible_rows.clone(),
                            self.fold_markers.clone(),
                            self.fold_animation.clone(),
                            entity.clone(),
                            self.minimap_visible,
                            editor_width,
                            minimap_width,
                            self.minimap_resize_preview,
                            self.minimap_thumb_visibility,
                            *generation,
                            self.presentation_revision,
                            self.opened_at.unwrap_or_else(Instant::now),
                        ))
                } else {
                    render_home(
                        entity.clone(),
                        &self.recent_documents,
                        Some(&error),
                        None,
                        self.language,
                    )
                }
            }
            PreviewLoadState::Ready {
                generation,
                document,
            } => render_document(
                document.clone(),
                self.list_state.clone(),
                self.visible_rows.clone(),
                self.fold_markers.clone(),
                self.fold_animation.clone(),
                entity.clone(),
                self.minimap_visible,
                editor_width,
                minimap_width,
                self.minimap_resize_preview,
                self.minimap_thumb_visibility,
                *generation,
                self.presentation_revision,
                self.opened_at.unwrap_or_else(Instant::now),
            ),
        };
        let Some(snapshot) = self.status_snapshot() else {
            return content;
        };
        let layout = self.status_layout(&snapshot, editor_width, window);
        let status_popover = self
            .status_popover
            .as_ref()
            .filter(|popover| popover.pane == snapshot.pane)
            .cloned();
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(content))
            .child(super::status_line::render_status_line(
                &snapshot,
                layout,
                entity.clone(),
                window,
            ))
            .when_some(status_popover, |view, popover| {
                view.child(super::status_line::render_status_popover(
                    popover,
                    Some(&snapshot),
                    self.status_line_settings,
                    entity,
                    self.language,
                ))
            })
    }

    #[cfg(test)]
    pub(super) fn toggle_fold(&mut self, block_id: BlockId, document: &Arc<PreviewDocument>) {
        self.discard_fold_animation();
        let Some(projection) = self.local_fold_projection(block_id, document) else {
            return;
        };
        self.remember_local_fold(block_id, projection.visibility);
        if projection.visibility == super::LocalVisibility::Empty {
            return;
        }
        self.apply_local_fold_projection(projection);
    }

    pub(super) fn toggle_fold_animated(
        &mut self,
        block_id: BlockId,
        document: &Arc<PreviewDocument>,
        viewport_height: f32,
        available_width: f32,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        self.discard_fold_animation();
        let Some(projection) = self.local_fold_projection(block_id, document) else {
            return;
        };
        self.remember_local_fold(block_id, projection.visibility);
        if projection.visibility == super::LocalVisibility::Empty {
            return;
        }
        self.apply_fold_projection_animated(
            projection.visible_rows,
            projection.fold_markers,
            document,
            viewport_height,
            available_width,
            window,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_fold_projection_animated(
        &mut self,
        visible_rows: Vec<usize>,
        fold_markers: HashSet<BlockId>,
        document: &PreviewDocument,
        viewport_height: f32,
        available_width: f32,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        if cx.reduce_motion() {
            self.apply_fold_projection(visible_rows, fold_markers);
            return;
        }
        let measurement = window
            .as_deref()
            .map_or(FoldMeasurement::Estimated, FoldMeasurement::Rendered);
        let Some(plan) = FoldTransitionPlan::build(FoldTransitionInput {
            current_rows: &self.visible_rows,
            target_rows: &visible_rows,
            document,
            list_state: &self.list_state,
            viewport_height,
            available_width,
            measurement,
        }) else {
            self.apply_fold_projection(visible_rows, fold_markers);
            return;
        };

        self.fold_animation_revision = self.fold_animation_revision.wrapping_add(1);
        let revision = self.fold_animation_revision;
        let suppressed_markers = fold_markers
            .difference(&self.fold_markers)
            .copied()
            .collect();
        let transition = plan.into_transition(revision, suppressed_markers);

        self.cancel_minimap_interaction();
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        self.fold_markers = Arc::new(fold_markers);
        self.visible_rows = Arc::new(visible_rows);
        for edit in transition.initial_edits.iter() {
            self.list_state.splice(edit.range.clone(), edit.new_count);
        }
        self.fold_animation = Some(transition);
        if let Some(window) = window {
            self.schedule_fold_animation_frame(revision, window, cx);
        }
    }

    fn local_fold_projection(
        &self,
        block_id: BlockId,
        document: &Arc<PreviewDocument>,
    ) -> Option<LocalCycleProjection> {
        let continue_from_children =
            self.local_cycle_continuation == Some((block_id, super::LocalVisibility::Children));
        match document.format {
            DocumentFormat::Org => cycle_org_subtree_visibility(
                &document.projection.rows,
                &document.blocks,
                &self.visible_rows,
                &self.fold_markers,
                block_id,
                continue_from_children,
            ),
            DocumentFormat::Markdown => cycle_markdown_subtree_visibility(
                &document.projection.rows,
                &document.markdown_blocks,
                &self.visible_rows,
                &self.fold_markers,
                block_id,
                continue_from_children,
            ),
        }
    }

    fn remember_local_fold(&mut self, block_id: BlockId, visibility: super::LocalVisibility) {
        self.global_cycle_contiguous = false;
        self.local_cycle_continuation = match visibility {
            super::LocalVisibility::Empty => None,
            visibility => Some((block_id, visibility)),
        };
    }

    #[cfg(test)]
    fn apply_local_fold_projection(&mut self, projection: LocalCycleProjection) {
        self.apply_fold_projection(projection.visible_rows, projection.fold_markers);
    }

    fn apply_fold_projection(&mut self, visible_rows: Vec<usize>, fold_markers: HashSet<BlockId>) {
        self.cancel_minimap_interaction();
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        self.fold_markers = Arc::new(fold_markers);
        self.apply_visible_rows(Arc::new(visible_rows));
    }

    pub(super) fn discard_fold_animation(&mut self) {
        self.fold_animation_revision = self.fold_animation_revision.wrapping_add(1);
        self.finish_fold_transition();
    }

    fn finish_fold_transition(&mut self) {
        let Some(animation) = self.fold_animation.take() else {
            return;
        };
        let actual_count = self.list_state.item_count();
        let expected_count = animation.transition_item_count();
        debug_assert_eq!(actual_count, expected_count);
        if actual_count != expected_count {
            // Recover to the semantic projection instead of leaving temporary segments behind.
            self.list_state
                .splice(0..actual_count, self.visible_rows.len());
            return;
        }
        for edit in animation.completion_edits() {
            self.list_state.splice(edit.range, edit.new_count);
        }
    }

    fn schedule_fold_animation_frame(
        &mut self,
        revision: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.on_next_frame(window, move |this, window, cx| {
            let Some(animation) = this.fold_animation.as_mut() else {
                return;
            };
            if animation.revision != revision {
                return;
            }
            let Some(started_at) = animation.started_at else {
                // Establish time zero only after the full-height Shell has completed its first
                // frame. Otherwise a slow initial layout consumes the whole animation duration.
                animation.started_at = Some(Instant::now());
                this.schedule_fold_animation_frame(revision, window, cx);
                return;
            };
            let progress = (started_at.elapsed().as_secs_f32()
                / LOCAL_FOLD_ANIMATION_DURATION.as_secs_f32())
            .clamp(0.0, 1.0);
            animation.progress = progress;
            // ListState caches item heights. Invalidating only the visible transition Shells makes
            // the outer list physically reflow following rows without remeasuring the document.
            for shell in animation.segments.iter() {
                this.list_state
                    .remeasure_items(shell.transition_index..shell.transition_index + 1);
            }
            cx.notify();

            if progress < 1.0 {
                this.schedule_fold_animation_frame(revision, window, cx);
            } else {
                cx.on_next_frame(window, move |this, _, cx| {
                    if this.fold_animation.as_ref().is_some_and(|animation| {
                        animation.revision == revision && animation.progress >= 1.0
                    }) {
                        this.finish_fold_transition();
                        cx.notify();
                    }
                });
            }
        });
    }

    #[cfg(test)]
    pub(super) fn cycle_global_visibility(&mut self) {
        self.discard_fold_animation();
        let Some(projection) = self.next_global_visibility_projection() else {
            return;
        };
        self.remember_global_visibility(projection.visibility);
        self.apply_fold_projection(projection.visible_rows, projection.fold_markers);
    }

    pub(super) fn cycle_global_visibility_animated(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.discard_fold_animation();
        let Some(projection) = self.next_global_visibility_projection() else {
            return;
        };
        self.remember_global_visibility(projection.visibility);
        let viewport = window.viewport_size();
        let viewport_width = f32::from(viewport.width);
        let editor_width = if self.sidebar_visible {
            (viewport_width
                - self.rendered_sidebar_width(viewport_width)
                - super::sidebar::RESIZE_HANDLE_PX)
                .max(super::sidebar::MIN_DOCUMENT_WIDTH_PX)
        } else {
            viewport_width
        };
        let minimap_width = minimap::width_for_viewport(editor_width, self.minimap_width);
        let minimap_space = if self.minimap_visible {
            minimap_width
        } else {
            0.0
        };
        let available_width = (editor_width - 110.0 - minimap_space).max(120.0);
        self.apply_fold_projection_animated(
            projection.visible_rows,
            projection.fold_markers,
            &projection.document,
            f32::from(viewport.height),
            available_width,
            Some(window),
            cx,
        );
    }

    fn next_global_visibility_projection(&self) -> Option<GlobalVisibilityProjection> {
        let document = match &self.state {
            PreviewLoadState::Ready { document, .. } => document.clone(),
            _ => return None,
        };
        let next = if self.global_cycle_contiguous {
            self.global_visibility.next()
        } else {
            super::GlobalVisibility::Overview
        };
        let (new_visible, new_markers) = match document.format {
            DocumentFormat::Org => {
                global_org_visibility(&document.projection.rows, &document.blocks, next)
            }
            DocumentFormat::Markdown => global_markdown_visibility(
                &document.projection.rows,
                &document.markdown_blocks,
                next,
            ),
        };
        if new_visible.is_empty() && next != super::GlobalVisibility::All {
            return None;
        }
        Some(GlobalVisibilityProjection {
            document,
            visibility: next,
            visible_rows: new_visible,
            fold_markers: new_markers,
        })
    }

    fn remember_global_visibility(&mut self, next: super::GlobalVisibility) {
        self.global_visibility = next;
        self.global_cycle_contiguous = true;
        self.local_cycle_continuation = None;
    }

    fn apply_visible_rows(&mut self, new_visible: Arc<Vec<usize>>) {
        let (old_range, new_count) = changed_range(&self.visible_rows, &new_visible);
        self.list_state.splice(old_range, new_count);
        if should_eagerly_measure_rows(new_visible.len()) {
            self.list_state.clone().measure_all();
        }
        self.visible_rows = new_visible;
    }

    pub(super) fn cancel_minimap_interaction(&mut self) -> bool {
        self.minimap_pending_seek = None;
        self.minimap_seek_scheduled = false;
        self.minimap_resize_preview = None;
        let was_dragging = match &self.state {
            PreviewLoadState::Ready { document, .. } => document.minimap.cancel_interaction(),
            _ => self
                .last_ready
                .as_ref()
                .is_some_and(|(_, document)| document.minimap.cancel_interaction()),
        };
        self.list_state.scrollbar_drag_ended();
        was_dragging
    }

    pub(super) fn window_title(&self) -> String {
        if self.content_route == ContentRoute::FileManager
            && let Some(session) = self.dired.as_ref()
        {
            return format!(
                "{} - Files",
                session
                    .directory()
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            );
        }
        let path = match &self.state {
            PreviewLoadState::Loading { path } | PreviewLoadState::Failed { path, .. } => {
                Some(path)
            }
            PreviewLoadState::Ready { document, .. } => Some(&document.path),
            PreviewLoadState::Empty => None,
        };
        path.and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("Org Studio")
            .to_owned()
    }
}
