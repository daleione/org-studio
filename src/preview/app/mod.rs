use super::{
    Arc, BuiltinCommand, CapabilitySet, CommandDispatcher, CommandImplementation, CommandKey,
    ContentRoute, Context, Duration, EmacsOutcome, InitialDocumentLoad, Instant, InvocationOrigin,
    KEY_FEEDBACK_DURATION, KeyDownEvent, KeyStroke, PathBuf, PathPromptOptions, PrefixArgument,
    PreviewLoadState, PreviewRenderOptions, Window, WorkspaceLoadedDocument, WorkspaceWindow,
    accept_generation, built_in_contexts, command_count, compile_input_profile,
    configured_minimap_visible, current_theme, dired_bindings, document_input,
    load_workspace_document, minimap, preview_bindings, px, render_document, render_home,
    render_loading, right_preview, workspace_bindings,
};
use crate::app::{DocumentViewPreferences, DocumentViewState};
use gpui::{div, prelude::*, rgb};

mod benchmark;
mod commands;
mod document_lifecycle;
mod save;
pub(crate) use benchmark::ScrollBenchmark;

fn preview_snapshot_is_renderable(
    panel_document: crate::document::DocumentId,
    panel_revision: crate::document::Revision,
    session_document: crate::document::DocumentId,
    session_revision: crate::document::Revision,
) -> bool {
    panel_document == session_document && panel_revision <= session_revision
}

impl Default for WorkspaceWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceWindow {
    pub fn new() -> Self {
        let settings = crate::settings::PreviewSettings::load();
        Self::with_settings(settings)
    }

    #[cfg(test)]
    pub(crate) fn with_right_preview(right_preview_open: bool) -> Self {
        let settings = crate::settings::PreviewSettings {
            right_preview_open,
            ..crate::settings::PreviewSettings::default()
        };
        Self::with_settings(settings)
    }

    fn with_settings(preview_settings: crate::settings::PreviewSettings) -> Self {
        let list_overdraw = std::env::var("ORG_STUDIO_LIST_OVERDRAW")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(80.0);
        let (commands, keyboard, key_context) = document_input();
        let minimap_visible = configured_minimap_visible();
        Self {
            language: preview_settings.language,
            focus_handle: None,
            focus_workspace_on_render: false,
            focus_lost_subscription: None,
            commands,
            keyboard,
            key_context,
            state: PreviewLoadState::Empty,
            document_subscription: None,
            editor_minimap_width_subscription: None,
            subscribed_document: None,
            recent_documents: crate::recent_documents::load(),
            home_error: None,
            generation: 0,
            load_task: None,
            derived: crate::app::DerivedHost::default(),
            right_preview_scroll: crate::app::RightPreviewScrollHost::default(),
            file_watch_task: None,
            file_watch_request: 0,
            file_watch_directory: None,
            file_watch_target: None,
            file_manager: super::file_manager_host::FileManagerHost::new(
                preview_settings.sidebar_width,
            ),
            picker_task: None,
            export: super::export_ui::ExportHost::default(),
            save: super::SaveHost::default(),
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
            document_view: DocumentViewState::editing(preview_settings.right_preview_open),
            document_view_preferences: DocumentViewPreferences {
                right_preview_width: preview_settings.right_preview_width,
            },
            right_preview_resize: None,
            soft_wrap: preview_settings.soft_wrap,
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

    pub(super) fn request_editor_focus(&mut self, cx: &mut Context<Self>) {
        self.focus_editor_surface(self.document_view.right_preview_open(), cx);
    }

    pub(super) fn focus_editor_surface(
        &mut self,
        right_preview_open: bool,
        cx: &mut Context<Self>,
    ) {
        let keymap_changed = !self.document_view.editor_focused();
        self.document_view = DocumentViewState::editing(right_preview_open);
        if keymap_changed {
            self.install_document_keymap();
        }
        if let Some(document) = self.state.ready() {
            document
                .editor
                .update(cx, |editor, cx| editor.request_focus(cx));
        }
    }

    pub(super) fn preview_panel(&self) -> Option<gpui::Entity<super::PreviewPanel>> {
        self.state
            .ready()
            .and_then(|document| document.panel.clone())
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
            right_preview_open: self.document_view.right_preview_open(),
            right_preview_width: self.document_view_preferences.right_preview_width,
            soft_wrap: self.soft_wrap,
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
                if let Some(previous) = previous.as_ref() {
                    let document = self.render_document_layout(
                        previous,
                        entity.clone(),
                        editor_width,
                        minimap_width,
                        cx,
                    );
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
                        .child(document)
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
                let notice = &ready.notice;
                let document = self.render_document_layout(
                    ready,
                    entity.clone(),
                    editor_width,
                    minimap_width,
                    cx,
                );
                if let Some(error) = notice {
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

    fn render_document_layout(
        &self,
        ready: &super::ReadyDocument,
        entity: gpui::Entity<Self>,
        editor_width: f32,
        minimap_width: f32,
        cx: &gpui::App,
    ) -> gpui::Div {
        if self.document_view.right_preview_open() {
            let pane_width = self.rendered_right_preview_width(editor_width);
            let resize_entity = entity.clone();
            div()
                .size_full()
                .flex()
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .min_w_0()
                        .child(ready.editor.clone()),
                )
                .child(
                    div()
                        .id("right-preview-resize-handle")
                        .w(px(right_preview::RESIZE_HANDLE_PX))
                        .h_full()
                        .flex_none()
                        .cursor(gpui::CursorStyle::ResizeLeftRight)
                        .border_l_1()
                        .border_color(rgb(current_theme().border))
                        .on_mouse_down(gpui::MouseButton::Left, move |event, _, cx| {
                            resize_entity.update(cx, |this, cx| {
                                this.begin_right_preview_resize(
                                    f32::from(event.position.x),
                                    editor_width,
                                    cx,
                                );
                            });
                        }),
                )
                .child({
                    let focus_entity = entity.clone();
                    div()
                        .w(px(pane_width))
                        .h_full()
                        .min_w_0()
                        .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                            focus_entity.update(cx, |this, cx| this.focus_right_preview(cx));
                        })
                        .child(self.render_preview(
                            ready,
                            entity,
                            pane_width,
                            minimap_width,
                            false,
                            cx,
                        ))
                })
        } else {
            div().size_full().child(ready.editor.clone())
        }
    }

    fn render_preview(
        &self,
        ready: &super::ReadyDocument,
        entity: gpui::Entity<Self>,
        editor_width: f32,
        minimap_width: f32,
        minimap_visible: bool,
        cx: &gpui::App,
    ) -> gpui::Div {
        let Some(panel_entity) = ready.panel.as_ref() else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(current_theme().foreground_dim))
                .child("Updating Preview…");
        };
        let panel = panel_entity.read(cx);
        let session = ready.session.read(cx);
        if !preview_snapshot_is_renderable(
            panel.document().document_id,
            panel.document().revision,
            session.id(),
            session.revision(),
        ) {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(current_theme().foreground_dim))
                .child("Updating Preview…");
        }
        // A previous revision is still a complete, internally coherent immutable snapshot.
        // Keep painting it until the derived projection is atomically replaced; source-mapped
        // cross-pane interactions already wait for matching revisions in `derived`.
        render_document(
            panel.render_state(),
            panel_entity.clone(),
            entity,
            PreviewRenderOptions {
                minimap_visible,
                editor_width,
                minimap_width,
                minimap_resize_preview: self.minimap_resize_preview,
                minimap_thumb_visibility: self.minimap_thumb_visibility,
                generation: self.generation,
                opened_at: self.opened_at.unwrap_or_else(Instant::now),
            },
        )
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
            PreviewLoadState::Loading { path, .. } => Some(path.as_path()),
            PreviewLoadState::Failed {
                previous: Some(document),
                ..
            } => Some(document.session.read(cx).path()),
            PreviewLoadState::Failed {
                path,
                previous: None,
                ..
            } => Some(path.as_path()),
            PreviewLoadState::Ready { document } => Some(document.session.read(cx).path()),
            PreviewLoadState::Empty => None,
        };
        let title = path
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("Org Studio")
            .to_owned();
        let Some(session) = self.document_session() else {
            return title;
        };
        let session = session.read(cx);
        let marker = match session.sync_state() {
            crate::document::SyncState::Conflict { .. } => " ⚠",
            crate::document::SyncState::Missing { .. } => " ?",
            _ if !matches!(session.save_state(), crate::document::SaveState::Idle) => " ↻",
            _ if session.is_dirty() => " •",
            _ => "",
        };
        format!("{title}{marker}")
    }
}

#[cfg(test)]
mod render_coherence_tests {
    use super::preview_snapshot_is_renderable;
    use crate::document::{DocumentSnapshot, Revision};

    #[test]
    fn previous_revision_remains_visible_until_atomic_preview_replacement() {
        let document = DocumentSnapshot::from_utf8(b"text".to_vec()).unwrap();
        assert!(preview_snapshot_is_renderable(
            document.document_id(),
            Revision(3),
            document.document_id(),
            Revision(4),
        ));
        let other = DocumentSnapshot::from_utf8(b"other".to_vec()).unwrap();
        assert!(!preview_snapshot_is_renderable(
            document.document_id(),
            Revision(3),
            other.document_id(),
            Revision(4),
        ));
    }
}
