use std::{sync::Arc, time::Instant};

use gpui::{Context, Window, div, prelude::*, px, rgb};

use crate::app::home::{render_home, render_loading};
use crate::{
    app::{
        ContentRoute, DocumentViewPreferences, DocumentWorkspaceState, PanePair, PaneSide,
        PaneSurface, ReadyDocument, WorkspaceLoadState, WorkspaceWindow, split_layout,
    },
    preview::{
        ReadingPreviewPanel, ReadingRenderOptions, configured_minimap_visible, document_input,
        minimap, preview_style, render_reading_document,
    },
    theme::current_theme,
};

mod actions;
mod babel;
mod benchmark;
mod commands;
mod document_lifecycle;
mod save;
pub(crate) use benchmark::ScrollBenchmark;

#[derive(Clone, Copy)]
struct PaneRenderContext<'a> {
    width: f32,
    minimap_width: f32,
    minimap_reveal: f32,
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
        Self::with_settings(crate::settings::WorkspaceSettings::load())
    }

    #[cfg(test)]
    pub(crate) fn with_split_layout(split: bool) -> Self {
        let mut workspace = Self::with_settings(crate::settings::WorkspaceSettings::default());
        if split {
            workspace.document_workspace.enter_split();
        }
        workspace
    }

    fn with_settings(preview_settings: crate::settings::WorkspaceSettings) -> Self {
        Self::with_settings_state(preview_settings)
    }

    fn with_settings_state(preview_settings: crate::settings::WorkspaceSettings) -> Self {
        let list_overdraw = std::env::var("ORG_STUDIO_LIST_OVERDRAW")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(80.0);
        let (commands, keyboard, key_context) = document_input();
        let minimap_visible = configured_minimap_visible(preview_settings.minimap_enabled);
        let mut document_workspace = DocumentWorkspaceState::default();
        let benchmark_reading =
            std::env::var("ORG_STUDIO_SCROLL_BENCH_SURFACE").as_deref() == Ok("reading");
        if benchmark_reading {
            document_workspace.set_surface(PaneSide::Left, PaneSurface::Reading);
        }
        if cfg!(feature = "benchmarks")
            && std::env::var_os("ORG_STUDIO_EDITOR_BENCH_SPLIT").is_some()
        {
            document_workspace.enter_split();
            document_workspace.set_surface(PaneSide::Left, PaneSurface::Editor);
            document_workspace.set_surface(PaneSide::Right, PaneSurface::Editor);
        }
        if cfg!(feature = "benchmarks")
            && let Ok(layout) = std::env::var("ORG_STUDIO_MINIMAP_VISUAL_LAYOUT")
        {
            let surfaces = match layout.as_str() {
                "editor-reading" => Some((PaneSurface::Editor, PaneSurface::Reading)),
                "editor-editor" => Some((PaneSurface::Editor, PaneSurface::Editor)),
                "reading-reading" => Some((PaneSurface::Reading, PaneSurface::Reading)),
                _ => None,
            };
            if let Some((left, right)) = surfaces {
                document_workspace.enter_split();
                document_workspace.set_surface(PaneSide::Left, left);
                document_workspace.set_surface(PaneSide::Right, right);
            } else {
                eprintln!(
                    "org_studio_minimap_visual_layout invalid={layout:?} expected=editor-reading|editor-editor|reading-reading"
                );
            }
        }
        let benchmark_reading_style = benchmark_reading
            .then(|| std::env::var("ORG_STUDIO_SCROLL_BENCH_STYLE").ok())
            .flatten()
            .and_then(|style| crate::preview::PreviewStyleId::parse(&style));
        Self {
            language: preview_settings.language,
            focus_handle: None,
            focus_workspace_on_render: false,
            focus_lost_subscription: None,
            commands,
            keyboard,
            key_context,
            state: WorkspaceLoadState::Empty,
            document_subscription: None,
            editor_minimap_width_subscriptions: Vec::new(),
            subscribed_document: None,
            recent_documents: crate::recent_documents::load(),
            home_error: None,
            generation: 0,
            pending_navigation: None,
            pending_surface_anchors: PanePair {
                left: None,
                right: None,
            },
            load_task: None,
            babel_task: None,
            babel_editor: None,
            babel_request: 0,
            derived: crate::app::DerivedHost::default(),
            file_watch_task: None,
            file_watch_request: 0,
            file_watch_directory: None,
            file_watch_target: None,
            file_manager: crate::app::file_manager::FileManagerHost::new(
                preview_settings.sidebar_width,
            ),
            agenda: crate::app::agenda::AgendaHost::new(),
            picker_task: None,
            export: crate::app::export_ui::ExportHost::default(),
            save: crate::app::save::SaveHost::default(),
            list_overdraw,
            opened_at: None,
            first_frame_scheduled: None,
            scroll_benchmark: std::env::var("ORG_STUDIO_SCROLL_BENCH_FRAMES")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|frames| *frames > 0)
                .map(|target_frames| {
                    let style_switches = std::env::var("ORG_STUDIO_STYLE_BENCH_SWITCHES")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0);
                    ScrollBenchmark {
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
                        style_switches_remaining: style_switches,
                        // Leave one interval after the last switch so the final
                        // frame measures the settled style instead of quitting
                        // during the resize transaction.
                        style_switch_interval: target_frames
                            .checked_div(style_switches.saturating_add(1))
                            .unwrap_or(target_frames)
                            .max(1),
                        style_switch_count: 0,
                        resize_narrow_width: std::env::var("ORG_STUDIO_STYLE_BENCH_NARROW_WIDTH")
                            .ok()
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(760.0),
                        resize_wide_width: std::env::var("ORG_STUDIO_STYLE_BENCH_WIDE_WIDTH")
                            .ok()
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(1400.0),
                        // The style gate starts at its configured wide width;
                        // the first switch must exercise the narrow layout.
                        resize_to_wide: true,
                    }
                }),
            which_key_task: None,
            which_key_request: 0,
            key_feedback_task: None,
            key_feedback_request: 0,
            which_key_items: Arc::new(Vec::new()),
            echo: crate::app::echo_area::EchoAreaHost::default(),
            content_route: if std::env::var_os("ORG_STUDIO_AGENDA_TEXT").is_some() {
                ContentRoute::AgendaText
            } else if std::env::var_os("ORG_STUDIO_AGENDA").is_some() {
                ContentRoute::Agenda
            } else {
                ContentRoute::Document
            },
            agenda_text_return: None,
            agenda_text_open: false,
            document_workspace,
            document_view_preferences: DocumentViewPreferences {
                split_ratio: preview_settings.split_ratio,
            },
            content_font_sizes: PanePair {
                left: crate::typography::ContentFontSize::default(),
                right: crate::typography::ContentFontSize::default(),
            },
            split_resize: None,
            soft_wrap: true,
            minimap_visible,
            minimap_visibility_animation: None,
            minimap_thumb_visibility: crate::settings::initial_minimap_thumb_visibility(
                preview_settings.minimap_thumb_visibility,
            ),
            minimap_width: crate::settings::initial_minimap_width(preview_settings.minimap_width),
            minimap_resize_preview: None,
            reading_style: benchmark_reading_style.unwrap_or(preview_settings.reading_style),
            status: crate::app::status_line::StatusLineHost::new(preview_settings.status_line),
        }
    }

    pub fn document_session(&self) -> Option<&gpui::Entity<crate::document::DocumentSession>> {
        self.state.ready().map(|document| &document.session)
    }

    pub(crate) fn request_document_focus(&mut self, cx: &mut Context<Self>) {
        self.install_document_keymap();
        self.focus_active_surface(cx);
    }

    pub(crate) fn editor(
        &self,
        pane: PaneSide,
    ) -> Option<gpui::Entity<crate::editor::SemanticEditor>> {
        self.state
            .ready()
            .and_then(|document| document.editors.get(pane).clone())
    }

    pub(crate) fn visible_panes(&self) -> impl Iterator<Item = PaneSide> + use<> {
        let workspace = self.document_workspace;
        [PaneSide::Left, PaneSide::Right]
            .into_iter()
            .filter(move |pane| workspace.pane_is_visible(*pane))
    }

    pub(crate) fn ensure_editor_for(&mut self, pane: PaneSide, cx: &mut Context<Self>) {
        let Some(ready) = self.state.ready() else {
            return;
        };
        if ready.editors.get(pane).is_some() {
            return;
        }
        let session = ready.session.clone();
        let editor_syntax = ready.editor_syntax.clone();
        let soft_wrap = self.soft_wrap;
        let minimap_visible = self.minimap_visible;
        let minimap_width = self.minimap_width;
        let content_font_size = *self.content_font_sizes.get(pane);
        let editor = cx.new(move |cx| {
            let mut editor = crate::editor::SemanticEditor::new_with_syntax_service(
                session,
                false,
                editor_syntax,
                cx,
            );
            editor.set_content_font_size(content_font_size, cx);
            editor.set_soft_wrap(soft_wrap, cx);
            editor.set_minimap(minimap_visible, minimap_width, cx);
            editor
        });
        if let Some(ready) = self.state.ready_mut() {
            *ready.editors.get_mut(pane) = Some(editor);
        }
    }

    pub(crate) fn reconcile_visible_editor_panes(&mut self, cx: &mut Context<Self>) {
        let panes = self
            .visible_panes()
            .filter(|pane| matches!(self.document_workspace.surface(*pane), PaneSurface::Editor))
            .collect::<Vec<_>>();
        for pane in panes {
            self.ensure_editor_for(pane, cx);
        }
    }

    pub(crate) fn reading_panel(&self) -> Option<gpui::Entity<ReadingPreviewPanel>> {
        let active = self.document_workspace.active_pane;
        if matches!(
            self.document_workspace.active_surface(),
            PaneSurface::Reading
        ) {
            return self.reading_panel_for(active);
        }
        let other = active.other();
        (self.document_workspace.is_split()
            && matches!(self.document_workspace.surface(other), PaneSurface::Reading))
        .then(|| self.reading_panel_for(other))
        .flatten()
    }

    pub(crate) fn reading_panel_for(
        &self,
        pane: PaneSide,
    ) -> Option<gpui::Entity<ReadingPreviewPanel>> {
        self.state
            .ready()
            .and_then(|document| document.readers.get(pane).clone())
    }

    pub(crate) fn latest_preview_is_current(&self, cx: &gpui::App) -> bool {
        self.derived.latest.as_ref().is_some_and(|preview| {
            self.document_session().is_some_and(|session| {
                let session = session.read(cx);
                preview.document_id == session.id()
                    && preview.revision == session.revision()
                    && preview.path == session.path()
            })
        })
    }

    pub(crate) fn reconcile_visible_reading_panes(&mut self, cx: &mut Context<Self>) -> bool {
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
        let content_font_sizes = self.content_font_sizes.clone();
        let Some(ready) = self.state.ready_mut() else {
            return false;
        };
        for &pane in &panes {
            if let Some(panel) = ready.readers.get(pane).clone() {
                let current = panel.read(cx).document().clone();
                if !Arc::ptr_eq(&current, &document)
                    || current.document_id != document.document_id
                    || current.revision != document.revision
                    || current.path != document.path
                {
                    let document = document.clone();
                    let style = *preview_style(self.reading_style);
                    panel.update(cx, |panel, cx| {
                        panel.replace_document_with_style(document, style, cx)
                    });
                }
            } else {
                let document = document.clone();
                let content_font_size = *content_font_sizes.get(pane);
                *ready.readers.get_mut(pane) = Some(cx.new(move |_| {
                    let mut panel = ReadingPreviewPanel::new(document, list_overdraw);
                    panel.set_content_font_size(content_font_size);
                    panel
                }));
            }
        }
        for pane in panes {
            self.apply_pending_surface_anchor(pane, cx);
        }
        self.apply_pending_navigation(self.generation, cx);
        true
    }

    pub(crate) fn visible_reading_panes_are_current(&self, cx: &gpui::App) -> bool {
        self.latest_preview_is_current(cx)
            && self
                .visible_panes()
                .filter(|pane| {
                    matches!(self.document_workspace.surface(*pane), PaneSurface::Reading)
                })
                .all(|pane| {
                    self.reading_panel_for(pane).is_some_and(|panel| {
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

    pub(crate) fn bump_preview_revision(&self, cx: &mut Context<Self>) {
        for panel in self
            .visible_panes()
            .filter_map(|pane| self.reading_panel_for(pane))
        {
            panel.update(cx, |panel, _| panel.bump_geometry_revision());
        }
    }

    pub(crate) fn cancel_minimap_interaction(&mut self, cx: &mut Context<Self>) -> bool {
        self.minimap_resize_preview = None;
        self.reading_panel()
            .is_some_and(|panel| panel.update(cx, |panel, _| panel.cancel_minimap_interaction()))
    }

    pub(crate) fn save_preview_settings(&self) {
        let _settings = crate::settings::WorkspaceSettings {
            split_ratio: self.document_view_preferences.split_ratio,
            language: self.language,
            minimap_enabled: self.minimap_visible,
            minimap_thumb_visibility: self.minimap_thumb_visibility,
            minimap_width: self.minimap_width,
            sidebar_width: self.file_manager.sidebar_width(),
            reading_style: self.reading_style,
            status_line: self.status.settings(),
        };
        #[cfg(not(test))]
        _settings.save_async();
    }

    pub fn language(&self) -> crate::i18n::Language {
        self.language
    }

    pub(crate) fn set_language(&mut self, language: crate::i18n::Language, cx: &mut Context<Self>) {
        if self.language != language {
            self.language = language;
            self.export.clear_status();
            self.save_preview_settings();
            cx.notify();
        }
    }

    pub(crate) fn change_minimap_width(
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
            WorkspaceLoadState::Loading { path, .. } | WorkspaceLoadState::Failed { path, .. } => {
                Some(path)
            }
            WorkspaceLoadState::Ready { document } => Some(document.session.read(cx).path()),
            WorkspaceLoadState::Empty => None,
        }
    }

    pub(crate) fn body(
        &self,
        entity: gpui::Entity<Self>,
        editor_width: f32,
        minimap_reveal: f32,
        window: &Window,
        cx: &gpui::App,
    ) -> gpui::Div {
        let minimap_width = minimap::width_for_viewport(editor_width, self.minimap_width);
        let content = match &self.state {
            WorkspaceLoadState::Empty => render_home(
                entity.clone(),
                &self.recent_documents,
                self.home_error.as_deref(),
                None,
                self.language,
            ),
            WorkspaceLoadState::Loading { path, .. } => render_loading(path, self.language),
            WorkspaceLoadState::Failed {
                path,
                message,
                previous,
            } => {
                let error = format!("{}: {message}", path.display());
                if let Some(previous) = previous.as_ref() {
                    self.render_document_layout(
                        previous,
                        entity.clone(),
                        PaneRenderContext {
                            width: editor_width,
                            minimap_width,
                            minimap_reveal,
                            window,
                            cx,
                        },
                    )
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
            WorkspaceLoadState::Ready { document: ready } => self.render_document_layout(
                ready,
                entity.clone(),
                PaneRenderContext {
                    width: editor_width,
                    minimap_width,
                    minimap_reveal,
                    window,
                    cx,
                },
            ),
        };
        if self.state.ready().is_some() {
            return content;
        }
        let Some(snapshot) = self.status_snapshot(cx) else {
            return content;
        };
        let layout = self.status_layout(&snapshot, editor_width, window);
        let style_popover_left =
            crate::app::status_line::reading_style_popover_left(&snapshot, &layout, window);
        let status_popover = self.status.popover_for(snapshot.pane);
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(content))
            .child(crate::app::status_line::render_status_line(
                &snapshot,
                layout,
                entity.clone(),
                window,
            ))
            .when_some(status_popover, |view, popover| {
                view.child(crate::app::status_line::render_status_popover(
                    popover,
                    Some(&snapshot),
                    self.status.settings(),
                    entity,
                    self.language,
                    editor_width,
                    style_popover_left,
                ))
            })
    }

    fn render_document_layout(
        &self,
        ready: &ReadyDocument,
        entity: gpui::Entity<Self>,
        render: PaneRenderContext<'_>,
    ) -> gpui::Div {
        let PaneRenderContext {
            width: editor_width,
            minimap_width,
            minimap_reveal,
            window,
            cx,
        } = render;
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
                        minimap_reveal,
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
                        minimap_reveal,
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
                    minimap_reveal,
                    window,
                    cx,
                },
            )
        }
    }

    fn render_pane_content(
        &self,
        ready: &ReadyDocument,
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
            PaneSurface::Reading => self.render_reading(ready, entity.clone(), pane, render),
        };
        let Some(snapshot) = self.document_status_snapshot(pane, render.cx) else {
            return div().w(px(render.width)).h_full().min_w_0().child(content);
        };
        let layout = self.status_layout(&snapshot, render.width, render.window);
        let style_popover_left =
            crate::app::status_line::reading_style_popover_left(&snapshot, &layout, render.window);
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
            .child(crate::app::status_line::render_status_line(
                &snapshot,
                layout,
                entity.clone(),
                render.window,
            ))
            .when_some(status_popover, |view, popover| {
                view.child(crate::app::status_line::render_status_popover(
                    popover,
                    Some(&snapshot),
                    self.status.settings(),
                    entity,
                    self.language,
                    render.width,
                    style_popover_left,
                ))
            })
    }

    fn render_reading(
        &self,
        ready: &ReadyDocument,
        entity: gpui::Entity<Self>,
        pane: PaneSide,
        render: PaneRenderContext<'_>,
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
            ReadingRenderOptions {
                minimap_visible: self.minimap_visible,
                minimap_reveal: render.minimap_reveal,
                pane_width: render.width,
                minimap_width: render.minimap_width,
                minimap_resize_preview: self.minimap_resize_preview,
                minimap_thumb_visibility: self.minimap_thumb_visibility,
                generation: self.generation,
                opened_at: self.opened_at.unwrap_or_else(Instant::now),
                style: *preview_style(self.reading_style),
                dispatch_action: {
                    let workspace = entity.clone();
                    Arc::new(move |action, panel, window, cx| {
                        workspace.update(cx, |workspace, cx| {
                            workspace.dispatch_preview_action(action, panel, window, cx);
                        });
                    })
                },
                change_minimap_width: {
                    let workspace = entity;
                    Arc::new(move |change, cx| {
                        workspace.update(cx, |workspace, cx| {
                            workspace.change_minimap_width(change, cx);
                        });
                    })
                },
            },
        )
    }

    pub(crate) fn cycle_global_visibility_animated(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.reading_panel() else {
            return;
        };
        let viewport = window.viewport_size();
        let viewport_width = f32::from(viewport.width);
        let editor_width = if self.file_manager.sidebar_visible() {
            (viewport_width
                - self.rendered_sidebar_width(viewport_width)
                - crate::app::file_manager::sidebar::RESIZE_HANDLE_PX)
                .max(crate::app::file_manager::sidebar::MIN_DOCUMENT_WIDTH_PX)
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
        let available_width = crate::preview::layout::reading_content_width(
            pane_width,
            minimap_space,
            *preview_style(self.reading_style),
        );
        panel.update(cx, |panel, cx| {
            panel.cycle_global_visibility_animated(
                f32::from(viewport.height),
                available_width,
                *preview_style(self.reading_style),
                window,
                cx,
            )
        });
    }

    pub(crate) fn window_title(&self, cx: &gpui::App) -> String {
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
            WorkspaceLoadState::Loading { path, .. } => Some(path.as_path()),
            WorkspaceLoadState::Failed {
                previous: Some(document),
                ..
            } => Some(document.session.read(cx).path()),
            WorkspaceLoadState::Failed {
                path,
                previous: None,
                ..
            } => Some(path.as_path()),
            WorkspaceLoadState::Ready { document } => Some(document.session.read(cx).path()),
            WorkspaceLoadState::Empty => None,
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
    use super::{WorkspaceWindow, preview_snapshot_is_renderable};
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

    #[test]
    fn every_workspace_starts_with_soft_wrap_enabled() {
        let workspace =
            WorkspaceWindow::with_settings(crate::settings::WorkspaceSettings::default());
        assert!(workspace.soft_wrap);
    }
}
