use super::{
    Arc, BuiltinCommand, CapabilitySet, CommandDispatcher, CommandImplementation, CommandKey,
    ContentRoute, Context, Duration, EmacsOutcome, InitialDocumentLoad, Instant, InvocationOrigin,
    KEY_FEEDBACK_DURATION, KeyDownEvent, KeyStroke, LoadedDocument, PathBuf, PathPromptOptions,
    PrefixArgument, PreviewLoadState, PreviewRenderOptions, Window, WorkspaceWindow,
    accept_generation, built_in_contexts, command_count, compile_input_profile,
    configured_minimap_visible, current_theme, dired_bindings, load_document, minimap,
    preview_bindings, preview_input, px, render_document, render_home, render_loading,
};
use gpui::{div, prelude::*, rgb};

mod benchmark;
mod commands;
mod document_lifecycle;
pub(crate) use benchmark::ScrollBenchmark;

impl Default for WorkspaceWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceWindow {
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
            file_manager: super::file_manager_host::FileManagerHost::new(
                preview_settings.sidebar_width,
            ),
            picker_task: None,
            export: super::export_ui::ExportHost::default(),
            list_overdraw,
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
            content_route: ContentRoute::Document,
            minimap_visible,
            minimap_thumb_visibility: crate::settings::initial_minimap_thumb_visibility(
                preview_settings.minimap_thumb_visibility,
            ),
            minimap_width: crate::settings::initial_minimap_width(preview_settings.minimap_width),
            minimap_resize_preview: None,
            status: super::status_line::StatusLineHost::new(preview_settings.status_line),
        }
    }

    pub fn document_session(&self) -> Option<&gpui::Entity<crate::document::DocumentSession>> {
        self.state.ready().map(|document| &document.session)
    }

    pub(super) fn preview_panel(&self) -> Option<gpui::Entity<super::PreviewPanel>> {
        self.state.ready().map(|document| document.panel.clone())
    }

    pub(super) fn bump_preview_revision(&self, cx: &mut Context<Self>) {
        if let Some(panel) = self.preview_panel() {
            panel.update(cx, |panel, _| panel.bump_presentation_revision());
        }
    }

    pub(super) fn cancel_minimap_interaction(&mut self, cx: &mut Context<Self>) -> bool {
        self.minimap_resize_preview = None;
        self.preview_panel()
            .is_some_and(|panel| panel.update(cx, |panel, _| panel.cancel_minimap_interaction()))
    }

    pub(super) fn save_preview_settings(&self) {
        crate::settings::PreviewSettings {
            language: self.language,
            minimap_enabled: self.minimap_visible,
            minimap_thumb_visibility: self.minimap_thumb_visibility,
            minimap_width: self.minimap_width,
            sidebar_width: self.file_manager.sidebar_width(),
            status_line: self.status.settings(),
        }
        .save_async();
    }

    pub fn language(&self) -> crate::i18n::Language {
        self.language
    }

    pub(super) fn set_language(&mut self, language: crate::i18n::Language, cx: &mut Context<Self>) {
        if self.language != language {
            self.language = language;
            self.export.clear_status();
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
                    self.bump_preview_revision(cx);
                    self.cancel_minimap_interaction(cx);
                    self.save_preview_settings();
                }
                cx.notify();
            }
            minimap::MinimapWidthChange::Reset => {
                self.minimap_resize_preview = None;
                if self.minimap_width.take().is_some() {
                    self.bump_preview_revision(cx);
                    self.cancel_minimap_interaction(cx);
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
        self.file_manager.sidebar_visible()
    }

    pub fn current_document_path<'a>(&'a self, cx: &'a gpui::App) -> Option<&'a std::path::Path> {
        match &self.state {
            PreviewLoadState::Loading { path, .. } | PreviewLoadState::Failed { path, .. } => {
                Some(path)
            }
            PreviewLoadState::Ready { document } => Some(document.session.read(cx).path()),
            PreviewLoadState::Empty => None,
        }
    }

    pub(super) fn body(
        &self,
        entity: gpui::Entity<Self>,
        editor_width: f32,
        window: &Window,
        cx: &gpui::App,
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
            PreviewLoadState::Loading { path, .. } => render_loading(path, self.language),
            PreviewLoadState::Failed {
                path,
                message,
                previous,
            } => {
                let error = format!("{}: {message}", path.display());
                if let Some(panel_entity) = previous.as_ref().map(|document| &document.panel) {
                    let panel = panel_entity.read(cx);
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
                            panel.render_state(),
                            panel_entity.clone(),
                            entity.clone(),
                            PreviewRenderOptions {
                                minimap_visible: self.minimap_visible,
                                editor_width,
                                minimap_width,
                                minimap_resize_preview: self.minimap_resize_preview,
                                minimap_thumb_visibility: self.minimap_thumb_visibility,
                                generation: self.generation,
                                opened_at: self.opened_at.unwrap_or_else(Instant::now),
                            },
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
            PreviewLoadState::Ready { document: ready } => {
                let panel_entity = &ready.panel;
                let reload_error = &ready.reload_error;
                let panel = panel_entity.read(cx);
                let document = render_document(
                    panel.render_state(),
                    panel_entity.clone(),
                    entity.clone(),
                    PreviewRenderOptions {
                        minimap_visible: self.minimap_visible,
                        editor_width,
                        minimap_width,
                        minimap_resize_preview: self.minimap_resize_preview,
                        minimap_thumb_visibility: self.minimap_thumb_visibility,
                        generation: self.generation,
                        opened_at: self.opened_at.unwrap_or_else(Instant::now),
                    },
                );
                if let Some(error) = reload_error {
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
                                .child(error.to_string()),
                        )
                        .child(document)
                } else {
                    document
                }
            }
        };
        let Some(snapshot) = self.status_snapshot(cx) else {
            return content;
        };
        let layout = self.status_layout(&snapshot, editor_width, window);
        let status_popover = self.status.popover_for(snapshot.pane);
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
                    self.status.settings(),
                    entity,
                    self.language,
                ))
            })
    }

    pub(super) fn cycle_global_visibility_animated(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.preview_panel() else {
            return;
        };
        let viewport = window.viewport_size();
        let viewport_width = f32::from(viewport.width);
        let editor_width = if self.file_manager.sidebar_visible() {
            (viewport_width
                - self.rendered_sidebar_width(viewport_width)
                - super::file_manager_host::sidebar::RESIZE_HANDLE_PX)
                .max(super::file_manager_host::sidebar::MIN_DOCUMENT_WIDTH_PX)
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
        panel.update(cx, |panel, cx| {
            panel.cycle_global_visibility_animated(
                f32::from(viewport.height),
                available_width,
                window,
                cx,
            )
        });
    }

    pub(super) fn window_title(&self, cx: &gpui::App) -> String {
        if self.content_route == ContentRoute::FileManager
            && let Some(session) = self.file_manager.session()
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
        let path: Option<&std::path::Path> = match &self.state {
            PreviewLoadState::Loading { path, .. } | PreviewLoadState::Failed { path, .. } => {
                Some(path.as_path())
            }
            PreviewLoadState::Ready { document } => Some(document.session.read(cx).path()),
            PreviewLoadState::Empty => None,
        };
        path.and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("Org Studio")
            .to_owned()
    }
}
