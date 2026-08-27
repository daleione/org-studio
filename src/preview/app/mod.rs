use super::{
    Arc, BlockId, BlockKind, BuiltinCommand, CapabilitySet, CommandDispatcher,
    CommandImplementation, CommandKey, ContentRoute, Context, DocumentFormat, Duration,
    EmacsOutcome, HashMap, HashSet, InitialDocumentLoad, Instant, InvocationOrigin,
    KEY_FEEDBACK_DURATION, KeyDownEvent, KeyStroke, ListAlignment, ListState,
    MAX_EXACT_SCROLL_LAYOUT_ROWS, PathBuf, PathPromptOptions, PrefixArgument, PreviewApp,
    PreviewDocument, PreviewLoadState, Window, accept_generation, built_in_contexts, changed_range,
    command_count, compile_input_profile, configured_minimap_visible, current_theme,
    dired_bindings, load_document, minimap, preview_bindings, preview_input, px, render_document,
    render_home, schedule_document_prewarm, visible_markdown_row_indices, visible_row_indices,
};
use gpui::{div, prelude::*, rgb};

mod benchmark;
mod commands;
mod document_lifecycle;
pub(super) use benchmark::ScrollBenchmark;

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
            picker_task: None,
            list_state: ListState::new(0, ListAlignment::Top, px(list_overdraw)),
            folded: Arc::new(HashSet::new()),
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
            minimap_visible,
            minimap_thumb_visibility: crate::settings::initial_minimap_thumb_visibility(
                preview_settings.minimap_thumb_visibility,
            ),
            minimap_width: crate::settings::initial_minimap_width(preview_settings.minimap_width),
            minimap_resize_preview: None,
            presentation_revision: 0,
            viewport_revision_key: None,
            minimap_pending_seek: None,
            minimap_seek_scheduled: false,
            dired: None,
            dired_error: None,
            dired_task: None,
            dired_list_state: ListState::new(0, ListAlignment::Top, px(80.0)),
            sidebar_list_state: ListState::new(0, ListAlignment::Top, px(60.0)),
            dired_pending_presentation: None,
            sidebar_pending_presentation: None,
            dired_presentation_scheduled: false,
            dired_viewport_memory: HashMap::new(),
            sidebar_viewport_memory: HashMap::new(),
        }
    }

    pub(super) fn save_preview_settings(&self) {
        crate::settings::PreviewSettings {
            minimap_enabled: self.minimap_visible,
            minimap_thumb_visibility: self.minimap_thumb_visibility,
            minimap_width: self.minimap_width,
        }
        .save_async();
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

    pub(super) fn body(&self, entity: gpui::Entity<Self>, editor_width: f32) -> gpui::Div {
        let theme = current_theme();
        let minimap_width = minimap::width_for_viewport(editor_width, self.minimap_width);
        match &self.state {
            PreviewLoadState::Empty => render_home(
                entity,
                &self.recent_documents,
                self.home_error.as_deref(),
                None,
            ),
            PreviewLoadState::Loading { path } => {
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
                                .py_2()
                                .bg(rgb(0xf3f8fd))
                                .border_b_1()
                                .border_color(rgb(0xd5e4f2))
                                .font_family(".SystemUIFont")
                                .text_size(px(12.0))
                                .text_color(rgb(0x55728d))
                                .child(format!("Opening {}", path.display())),
                        )
                        .child(render_document(
                            document.clone(),
                            self.list_state.clone(),
                            self.visible_rows.clone(),
                            self.folded.clone(),
                            entity,
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
                        entity,
                        &self.recent_documents,
                        self.home_error.as_deref(),
                        Some(path),
                    )
                }
            }
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
                            self.folded.clone(),
                            entity,
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
                    render_home(entity, &self.recent_documents, Some(&error), None)
                }
            }
            PreviewLoadState::Ready {
                generation,
                document,
            } => render_document(
                document.clone(),
                self.list_state.clone(),
                self.visible_rows.clone(),
                self.folded.clone(),
                entity,
                self.minimap_visible,
                editor_width,
                minimap_width,
                self.minimap_resize_preview,
                self.minimap_thumb_visibility,
                *generation,
                self.presentation_revision,
                self.opened_at.unwrap_or_else(Instant::now),
            ),
        }
    }

    pub(super) fn toggle_fold(&mut self, block_id: BlockId, document: &Arc<PreviewDocument>) {
        let is_heading = match document.format {
            DocumentFormat::Org => document
                .blocks
                .nodes()
                .get(block_id as usize)
                .is_some_and(|block| matches!(block.kind, BlockKind::Heading { .. })),
            DocumentFormat::Markdown => document
                .markdown_blocks
                .get(block_id as usize)
                .is_some_and(|block| {
                    matches!(block.kind, super::markdown::MarkdownKind::Heading { .. })
                }),
        };
        if !is_heading {
            return;
        }
        self.cancel_minimap_interaction();
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        let folded = Arc::make_mut(&mut self.folded);
        if !folded.remove(&block_id) {
            folded.insert(block_id);
        }
        let new_visible = Arc::new(match document.format {
            DocumentFormat::Org => {
                visible_row_indices(&document.projection.rows, &document.blocks, &self.folded)
            }
            DocumentFormat::Markdown => visible_markdown_row_indices(
                &document.projection.rows,
                &document.markdown_blocks,
                &self.folded,
            ),
        });
        let (old_range, new_count) = changed_range(&self.visible_rows, &new_visible);
        self.list_state.splice(old_range, new_count);
        if new_visible.len() <= MAX_EXACT_SCROLL_LAYOUT_ROWS {
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
