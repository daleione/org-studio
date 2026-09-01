use super::{
    Arc, BuiltinCommand, CapabilitySet, CommandDispatcher, CommandImplementation, CommandKey,
    ContentRoute, Context, Duration, EmacsOutcome, InitialDocumentLoad, Instant, InvocationOrigin,
    KEY_FEEDBACK_DURATION, KeyDownEvent, KeyStroke, PathBuf, PathPromptOptions, PrefixArgument,
    PreviewLoadState, ReadingRenderOptions, Window, WorkspaceLoadedDocument, WorkspaceWindow,
    accept_generation, built_in_contexts, command_count, compile_input_profile,
    configured_minimap_visible, current_theme, dired_bindings, document_input,
    load_workspace_document, minimap, preview_bindings, px, render_home, render_loading,
    render_reading_document, split_layout, workspace_bindings,
};
use crate::app::{DocumentViewPreferences, DocumentWorkspaceState, PaneSide, PaneSurface};
use gpui::{div, prelude::*, rgb};

mod actions;
mod benchmark;
mod commands;
mod document_lifecycle;
mod save;
pub(crate) use benchmark::ScrollBenchmark;

#[derive(Clone, Copy)]
struct PaneRenderContext<'a> {
    width: f32,
    minimap_width: f32,
    window: &'a Window,
    cx: &'a gpui::App,
}

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
    pub(crate) fn with_split_layout(split: bool) -> Self {
        let mut workspace = Self::with_settings(crate::settings::PreviewSettings::default());
        if split {
            workspace.document_workspace.layout = crate::app::WorkspaceLayout::Split;
        }
        workspace
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
            editor_minimap_width_subscriptions: Vec::new(),
            subscribed_document: None,
            recent_documents: crate::recent_documents::load(),
            home_error: None,
            generation: 0,
            pending_navigation: None,
            load_task: None,
            derived: crate::app::DerivedHost::default(),
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
            document_workspace: DocumentWorkspaceState::default(),
            document_view_preferences: DocumentViewPreferences {
                split_ratio: preview_settings.split_ratio,
            },
            split_resize: None,
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

    pub(super) fn request_document_focus(&mut self, cx: &mut Context<Self>) {
        self.install_document_keymap();
        self.focus_active_surface(cx);
    }

    pub(super) fn editor(
        &self,
        pane: PaneSide,
    ) -> Option<gpui::Entity<crate::editor::SemanticEditor>> {
        self.state
            .ready()
            .and_then(|document| document.editors.get(pane).clone())
    }

    pub(super) fn visible_panes(&self) -> impl Iterator<Item = PaneSide> + use<> {
        let workspace = self.document_workspace;
        [PaneSide::Left, PaneSide::Right]
            .into_iter()
            .filter(move |pane| workspace.pane_is_visible(*pane))
    }

    pub(super) fn ensure_editor_for(&mut self, pane: PaneSide, cx: &mut Context<Self>) {
        let Some(ready) = self.state.ready() else {
            return;
        };
        if ready.editors.get(pane).is_some() {
            return;
        }
        let session = ready.session.clone();
        let soft_wrap = self.soft_wrap;
        let minimap_visible = self.minimap_visible;
        let minimap_width = self.minimap_width;
        let editor = cx.new(move |cx| {
            let mut editor = crate::editor::SemanticEditor::new_with_autofocus(session, false, cx);
            editor.set_soft_wrap(soft_wrap, cx);
            editor.set_minimap(minimap_visible, minimap_width, cx);
            editor
        });
        if let Some(ready) = self.state.ready_mut() {
            *ready.editors.get_mut(pane) = Some(editor);
        }
    }

    pub(super) fn reconcile_visible_editor_panes(&mut self, cx: &mut Context<Self>) {
        let panes = self
            .visible_panes()
            .filter(|pane| matches!(self.document_workspace.surface(*pane), PaneSurface::Editor))
            .collect::<Vec<_>>();
        for pane in panes {
            self.ensure_editor_for(pane, cx);
        }
    }

    pub(super) fn preview_panel(&self) -> Option<gpui::Entity<super::ReadingPreviewPanel>> {
        let active = self.document_workspace.active_pane;
        if matches!(
            self.document_workspace.active_surface(),
            PaneSurface::Reading
        ) {
            return self.preview_panel_for(active);
        }
        let other = active.other();
        (self.document_workspace.is_split()
            && matches!(self.document_workspace.surface(other), PaneSurface::Reading))
        .then(|| self.preview_panel_for(other))
        .flatten()
    }

    pub(super) fn preview_panel_for(
        &self,
        pane: PaneSide,
    ) -> Option<gpui::Entity<super::ReadingPreviewPanel>> {
        self.state
            .ready()
            .and_then(|document| document.readers.get(pane).clone())
    }

    pub(super) fn latest_preview_is_current(&self, cx: &gpui::App) -> bool {
        self.derived.latest.as_ref().is_some_and(|preview| {
            self.document_session().is_some_and(|session| {
                let session = session.read(cx);
                preview.document_id == session.id()
                    && preview.revision == session.revision()
                    && preview.path == session.path()
            })
        })
    }

    pub(super) fn reconcile_visible_reading_panes(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.latest_preview_is_current(cx) {
            return false;
        }
        let document = self
            .derived
            .latest
            .as_ref()
            .expect("current preview exists")
            .clone();
        let panes = self
            .visible_panes()
            .filter(|pane| matches!(self.document_workspace.surface(*pane), PaneSurface::Reading))
            .collect::<Vec<_>>();
        let list_overdraw = self.list_overdraw;
        let Some(ready) = self.state.ready_mut() else {
            return false;
        };
        for pane in panes {
            if let Some(panel) = ready.readers.get(pane).clone() {
                let current = panel.read(cx).document().clone();
                if current.document_id != document.document_id
                    || current.revision != document.revision
                    || current.path != document.path
                {
                    let document = document.clone();
                    panel.update(cx, |panel, cx| panel.replace_document(document, cx));
                }
            } else {
                let document = document.clone();
                *ready.readers.get_mut(pane) =
                    Some(cx.new(move |_| super::ReadingPreviewPanel::new(document, list_overdraw)));
            }
        }
        self.apply_pending_navigation(self.generation, cx);
        true
    }

    pub(super) fn visible_reading_panes_are_current(&self, cx: &gpui::App) -> bool {
        self.latest_preview_is_current(cx)
            && self
                .visible_panes()
                .filter(|pane| {
                    matches!(self.document_workspace.surface(*pane), PaneSurface::Reading)
                })
                .all(|pane| {
                    self.preview_panel_for(pane).is_some_and(|panel| {
                        let preview = panel.read(cx);
                        let latest = self
                            .derived
                            .latest
                            .as_ref()
                            .expect("current preview exists");
                        preview.document().document_id == latest.document_id
                            && preview.document().revision == latest.revision
                            && preview.document().path == latest.path
                    })
                })
    }

    pub(super) fn bump_preview_revision(&self, cx: &mut Context<Self>) {
        for panel in self
            .visible_panes()
            .filter_map(|pane| self.preview_panel_for(pane))
        {
            panel.update(cx, |panel, _| panel.bump_geometry_revision());
        }
    }

    pub(super) fn cancel_minimap_interaction(&mut self, cx: &mut Context<Self>) -> bool {
        self.minimap_resize_preview = None;
        self.preview_panel()
            .is_some_and(|panel| panel.update(cx, |panel, _| panel.cancel_minimap_interaction()))
    }

    pub(super) fn save_preview_settings(&self) {
        crate::settings::PreviewSettings {
            split_ratio: self.document_view_preferences.split_ratio,
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
                    self.propagate_editor_minimap_settings(cx);
                    self.bump_preview_revision(cx);
                    self.cancel_minimap_interaction(cx);
                    self.save_preview_settings();
                }
                cx.notify();
            }
            minimap::MinimapWidthChange::Reset => {
                self.minimap_resize_preview = None;
                if self.minimap_width.take().is_some() {
                    self.propagate_editor_minimap_settings(cx);
                    self.bump_preview_revision(cx);
                    self.cancel_minimap_interaction(cx);
                    self.save_preview_settings();
                }
                cx.notify();
            }
        }
    }

    fn propagate_editor_minimap_settings(&self, cx: &mut Context<Self>) {
        let Some(document) = self.state.ready() else {
            return;
        };
        for editor in [&document.editors.left, &document.editors.right]
            .into_iter()
            .flatten()
        {
            editor.update(cx, |editor, cx| {
                editor.set_minimap(self.minimap_visible, self.minimap_width, cx)
            });
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
                        window,
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
                    window,
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
        if self.state.ready().is_some() {
            return content;
        }
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
        window: &Window,
        cx: &gpui::App,
    ) -> gpui::Div {
        if self.document_workspace.is_split() {
            let left_width = self.rendered_left_pane_width(editor_width);
            let right_width = (editor_width - left_width - split_layout::RESIZE_HANDLE_PX).max(0.0);
            let resize_entity = entity.clone();
            div()
                .size_full()
                .flex()
                .child(self.render_pane_content(
                    ready,
                    entity.clone(),
                    PaneSide::Left,
                    PaneRenderContext {
                        width: left_width,
                        minimap_width: minimap::width_for_viewport(left_width, self.minimap_width),
                        window,
                        cx,
                    },
                ))
                .child(
                    div()
                        .id("split-resize-handle")
                        .w(px(split_layout::RESIZE_HANDLE_PX))
                        .h_full()
                        .flex_none()
                        .cursor(gpui::CursorStyle::ResizeLeftRight)
                        .border_l_1()
                        .border_color(rgb(current_theme().border))
                        .on_mouse_down(gpui::MouseButton::Left, move |event, _, cx| {
                            resize_entity.update(cx, |this, cx| {
                                this.begin_split_resize(
                                    f32::from(event.position.x),
                                    editor_width,
                                    cx,
                                );
                            });
                        }),
                )
                .child(self.render_pane_content(
                    ready,
                    entity,
                    PaneSide::Right,
                    PaneRenderContext {
                        width: right_width,
                        minimap_width: minimap::width_for_viewport(right_width, self.minimap_width),
                        window,
                        cx,
                    },
                ))
        } else {
            self.render_pane_content(
                ready,
                entity.clone(),
                self.document_workspace.active_pane,
                PaneRenderContext {
                    width: editor_width,
                    minimap_width,
                    window,
                    cx,
                },
            )
        }
    }

    fn render_pane_content(
        &self,
        ready: &super::ReadyDocument,
        entity: gpui::Entity<Self>,
        pane: PaneSide,
        render: PaneRenderContext<'_>,
    ) -> gpui::Div {
        let activate_entity = entity.clone();
        let content = match self.document_workspace.surface(pane) {
            PaneSurface::Editor => ready.editors.get(pane).as_ref().map_or_else(
                || {
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(current_theme().foreground_dim))
                        .child("Opening Editor…")
                },
                |editor| div().size_full().child(editor.clone()),
            ),
            PaneSurface::Reading => {
                self.render_reading(ready, entity.clone(), pane, render, self.minimap_visible)
            }
        };
        let Some(snapshot) = self.document_status_snapshot(pane, render.cx) else {
            return div().w(px(render.width)).h_full().min_w_0().child(content);
        };
        let layout = self.status_layout(&snapshot, render.width, render.window);
        let status_popover = self.status.popover_for(snapshot.pane);
        div()
            .w(px(render.width))
            .h_full()
            .min_w_0()
            .relative()
            .flex()
            .flex_col()
            .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                activate_entity.update(cx, |this, cx| this.activate_pane(pane, cx));
            })
            .child(div().flex_1().min_h_0().child(content))
            .child(super::status_line::render_status_line(
                &snapshot,
                layout,
                entity.clone(),
                render.window,
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

    fn render_reading(
        &self,
        ready: &super::ReadyDocument,
        entity: gpui::Entity<Self>,
        pane: PaneSide,
        render: PaneRenderContext<'_>,
        minimap_visible: bool,
    ) -> gpui::Div {
        let Some(panel_entity) = ready.readers.get(pane).as_ref() else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(current_theme().foreground_dim))
                .child("Preparing Reading…");
        };
        let panel = panel_entity.read(render.cx);
        let session = ready.session.read(render.cx);
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
                .child("Preparing Reading…");
        }
        // A previous revision is still a complete, internally coherent immutable snapshot.
        // Keep painting it until the derived projection is atomically replaced. Each pane owns
        // its viewport, so publication never needs to coordinate scroll state across panes.
        render_reading_document(
            panel.render_state(),
            panel_entity.clone(),
            entity,
            ReadingRenderOptions {
                minimap_visible,
                pane_width: render.width,
                minimap_width: render.minimap_width,
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
        let pane_width = if self.document_workspace.is_split() {
            let left_width = self.rendered_left_pane_width(editor_width);
            match self.document_workspace.active_pane {
                PaneSide::Left => left_width,
                PaneSide::Right => {
                    (editor_width - left_width - split_layout::RESIZE_HANDLE_PX).max(0.0)
                }
            }
        } else {
            editor_width
        };
        let minimap_width = minimap::width_for_viewport(pane_width, self.minimap_width);
        let minimap_space = if self.minimap_visible {
            minimap_width
        } else {
            0.0
        };
        let available_width = super::layout::reading_content_width(pane_width, minimap_space);
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
