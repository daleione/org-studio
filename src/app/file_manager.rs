use crate::components::UiTextExt;
use crate::typography::ui_px;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use gpui::{
    Context, CursorStyle, Entity, InteractiveElement, ListAlignment, ListOffset, ListState,
    MouseButton, ParentElement, PathPromptOptions, Styled, Window, div, list, prelude::*, px, rgb,
};

use super::{ContentRoute, DiredStatus, WorkspaceLoadState, WorkspaceWindow};
use crate::{
    file_manager::{
        ConflictPolicy, DirectoryStore, DiredSession, EntryKind, Mark, NavigationLoad,
        OperationPlan, scan_directory_cancellable,
    },
    motion::{Easing, MotionSpec, Tween},
    navigation::{NavigationCause, SelectionIntent, TransactionId, ViewRevision},
    theme::current_theme,
};

mod breadcrumb;
mod operations;
pub(crate) mod sidebar;
mod tree;
mod watch;
#[cfg(feature = "visual-check")]
pub use tree::visual_check::run_visual_check;

#[derive(Clone, Copy)]
pub(crate) struct DiredContextMenu {
    position: gpui::Point<gpui::Pixels>,
}

#[derive(Clone)]
struct DiredDrag {
    sources: Arc<[Arc<std::path::Path>]>,
}

#[derive(Clone, Copy)]
enum DiredContextAction {
    Open,
    Rename,
    Copy,
    Move,
    Trash,
}

const SIDEBAR_MOTION: MotionSpec =
    MotionSpec::new(Duration::from_millis(170), Easing::EaseOutCubic);

pub(crate) struct FileManagerHost {
    directory_watch: Option<watch::DirectoryWatch>,
    directories: DirectoryStore,
    directory_tasks: HashMap<PathBuf, gpui::Task<()>>,
    dired_load: Option<NavigationLoad>,
    tree: tree::TreeHost,
    help_visible: bool,
    sidebar_visible: bool,
    sidebar_focused: bool,
    sidebar_width: u16,
    sidebar_resize: Option<sidebar::ResizeSession>,
    sidebar_swipe: crate::components::sidebar_swipe::SidebarSwipe,
    sidebar_visibility_animation: Option<Tween>,
    session: Option<DiredSession>,
    status: Option<super::DiredStatus>,
    watch_error: Option<Arc<str>>,
    task: Option<gpui::Task<()>>,
    refresh_pending: bool,
    operation_task: Option<gpui::Task<()>>,
    operation_busy: bool,
    context_menu: Option<DiredContextMenu>,
    list_state: ListState,
    pending_presentation: Option<(TransactionId, ViewRevision, usize, f32)>,
    presentation_scheduled: bool,
}

impl FileManagerHost {
    pub(crate) fn new(sidebar_width: u16) -> Self {
        Self {
            directory_watch: None,
            directories: DirectoryStore::default(),
            directory_tasks: HashMap::new(),
            dired_load: None,
            tree: tree::TreeHost::default(),
            help_visible: false,
            sidebar_visible: false,
            sidebar_focused: false,
            sidebar_width: crate::settings::initial_sidebar_width(sidebar_width),
            sidebar_resize: None,
            sidebar_swipe: Default::default(),
            sidebar_visibility_animation: None,
            session: None,
            status: None,
            watch_error: None,
            task: None,
            refresh_pending: false,
            operation_task: None,
            operation_busy: false,
            context_menu: None,
            list_state: ListState::new(0, ListAlignment::Top, px(80.0)),
            pending_presentation: None,
            presentation_scheduled: false,
        }
    }

    pub(crate) fn reset_for_document(&mut self) {
        self.sidebar_focused = false;
        self.task = None;
        self.dired_load = None;
        self.refresh_pending = false;
    }

    pub(crate) fn sidebar_width(&self) -> u16 {
        self.sidebar_width
    }

    pub(crate) fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    pub(crate) fn sidebar_focused(&self) -> bool {
        self.sidebar_focused
    }

    pub(crate) fn session(&self) -> Option<&DiredSession> {
        self.session.as_ref()
    }

    pub(crate) fn status(&self) -> Option<&DiredStatus> {
        self.status.as_ref()
    }

    pub(crate) fn watch_error(&self) -> Option<&Arc<str>> {
        self.watch_error.as_ref()
    }

    pub(super) fn operation_busy(&self) -> bool {
        self.operation_busy
    }

    pub(crate) fn dismiss_context_menu(&mut self) -> bool {
        self.context_menu.take().is_some()
    }

    pub(crate) fn set_help_visible(&mut self, visible: bool) {
        self.help_visible = visible;
    }

    pub(crate) fn help_visible(&self) -> bool {
        self.help_visible
    }

    pub(crate) fn is_resizing_sidebar(&self) -> bool {
        self.sidebar_resize.is_some()
    }
}

impl WorkspaceWindow {
    pub(crate) fn schedule_file_manager_presentation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_manager.pending_presentation.is_some()
            && !self.file_manager.presentation_scheduled
        {
            self.file_manager.presentation_scheduled = true;
            cx.on_next_frame(window, |this, _, cx| {
                this.file_manager.presentation_scheduled = false;
                if let Some((transaction, revision, rank, offset)) =
                    this.file_manager.pending_presentation.take()
                    && this.file_manager.session.as_ref().is_some_and(|session| {
                        session.presentation_is_current(transaction, revision)
                    })
                {
                    this.file_manager.list_state.scroll_to(ListOffset {
                        item_ix: rank,
                        offset_in_item: px(offset),
                    });
                }
                cx.notify();
            });
        }
        self.prepare_file_tree(window, cx);
    }

    pub(crate) fn document_path<'a>(&'a self, cx: &'a gpui::App) -> Option<&'a std::path::Path> {
        match &self.state {
            WorkspaceLoadState::Loading { path, .. } | WorkspaceLoadState::Failed { path, .. } => {
                Some(path)
            }
            WorkspaceLoadState::Ready { document } => Some(document.session.read(cx).path()),
            WorkspaceLoadState::Empty => None,
        }
    }

    pub(crate) fn choose_directory(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open File Manager".into()),
        });
        self.file_manager.task = Some(cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.open_file_manager(path, cx));
        }));
    }

    pub(crate) fn open_default_dired(&mut self, cx: &mut Context<Self>) {
        let directory = self
            .file_manager
            .session
            .as_ref()
            .map(|session| session.directory().to_path_buf())
            .or_else(|| {
                self.document_path(cx)
                    .and_then(|path| path.parent().map(PathBuf::from))
            })
            .or_else(|| std::env::current_dir().ok());
        if let Some(directory) = directory {
            self.open_file_manager(directory, cx);
        }
    }

    pub(crate) fn open_file_manager(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        let directory = absolute_directory(directory);
        self.content_route = ContentRoute::FileManager;
        self.file_manager.sidebar_focused = false;
        self.install_dired_keymap();
        let intent =
            DiredSession::intent_for(directory, NavigationCause::Enter, SelectionIntent::Restore);
        self.navigate_file_manager(intent, cx);
    }

    fn navigate_file_manager(
        &mut self,
        intent: crate::navigation::NavigationIntent<PathBuf, crate::file_manager::FileAnchor>,
        cx: &mut Context<Self>,
    ) {
        let directory = intent.target.clone();
        let session = self
            .file_manager
            .session
            .get_or_insert_with(|| DiredSession::empty(directory));
        let viewport = self.file_manager.list_state.logical_scroll_top();
        session.capture_viewport(viewport.item_ix, f32::from(viewport.offset_in_item));
        let load = session.begin_navigation(intent);
        self.start_file_manager_load(load, cx);
    }

    fn start_file_manager_load(&mut self, load: NavigationLoad, cx: &mut Context<Self>) {
        let directory = load.intent.target.clone();
        let refresh = matches!(
            load.intent.cause,
            NavigationCause::Refresh | NavigationCause::FileSystemDelta
        );
        self.file_manager.status = None;
        self.file_manager.dired_load = Some(load);
        self.request_file_directory(directory.clone(), refresh, cx);
        self.commit_dired_directory(&directory, cx);
        self.sync_file_directory_watches(cx);
        cx.notify();
    }

    fn request_file_directory(
        &mut self,
        directory: PathBuf,
        refresh: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self
            .file_manager
            .directories
            .request(directory.clone(), refresh)
        else {
            return;
        };
        let work = request.clone();
        let scan = cx.background_spawn(async move {
            scan_directory_cancellable(work.directory, &work.cancellation)
        });
        let task = cx.spawn(async move |this, cx| {
            let result = scan.await;
            let _ = this.update(cx, |this, cx| {
                if !this.file_manager.directories.commit(&request, result) {
                    return;
                }
                this.file_manager.directory_tasks.remove(&request.directory);
                this.commit_dired_directory(&request.directory, cx);
                this.rebuild_file_tree(cx);
                this.sync_file_directory_watches(cx);
                if this
                    .file_manager
                    .directories
                    .needs_refresh(&request.directory)
                {
                    this.request_file_directory(request.directory, true, cx);
                }
                cx.notify();
            });
        });
        self.file_manager.directory_tasks.insert(directory, task);
    }

    fn commit_dired_directory(&mut self, directory: &Path, cx: &mut Context<Self>) {
        if self.file_manager.directories.is_loading(directory) {
            return;
        }
        if self
            .file_manager
            .dired_load
            .as_ref()
            .is_none_or(|load| load.intent.target != directory)
        {
            return;
        }
        if let Some(error) = self.file_manager.directories.error(directory) {
            self.file_manager.status = Some(DiredStatus::Error(
                format!("{}: {error}", directory.display()).into(),
            ));
            self.file_manager.dired_load = None;
        } else if let Some(snapshot) = self.file_manager.directories.snapshot(directory).cloned() {
            let load = self
                .file_manager
                .dired_load
                .take()
                .expect("pending directory navigation");
            if let Some(commit) = self
                .file_manager
                .session
                .as_mut()
                .and_then(|session| session.apply_directory(&load, &snapshot))
            {
                self.file_manager.list_state.reset(commit.item_count);
                self.file_manager.pending_presentation = commit.presentation_rank.map(|rank| {
                    (
                        commit.transaction_id,
                        commit.view_revision,
                        rank,
                        commit.presentation_offset,
                    )
                });
            }
        }
        if std::mem::take(&mut self.file_manager.refresh_pending) {
            self.refresh_file_manager(NavigationCause::FileSystemDelta, cx);
        }
    }

    pub(crate) fn dired_history(&mut self, forward: bool, cx: &mut Context<Self>) {
        let viewport = self.file_manager.list_state.logical_scroll_top();
        let load = self.file_manager.session.as_mut().and_then(|session| {
            session.capture_viewport(viewport.item_ix, viewport.offset_in_item.to_f64() as f32);
            session.begin_history_navigation(forward)
        });
        if let Some(load) = load {
            self.start_file_manager_load(load, cx);
        }
    }

    pub(crate) fn return_to_document(&mut self, cx: &mut Context<Self>) {
        self.content_route = ContentRoute::Document;
        self.file_manager.sidebar_focused = false;
        self.sync_file_directory_watches(cx);
        self.request_document_focus(cx);
        cx.notify();
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.end_prefix(cx);
        self.save_file_tree_memory();
        let now = Instant::now();
        let from = self.sidebar_reveal_at(now).0;
        self.file_manager.sidebar_visible = !self.file_manager.sidebar_visible;
        self.file_manager.sidebar_resize = None;
        self.file_manager.sidebar_visibility_animation =
            (!cx.reduce_motion()).then_some(Tween::new(
                now,
                from,
                if self.file_manager.sidebar_visible {
                    1.0
                } else {
                    0.0
                },
                SIDEBAR_MOTION,
            ));
        if self.file_manager.sidebar_visible {
            if self.file_manager.watch_error.is_some() {
                self.file_manager.directory_watch = None;
            }
            self.ensure_file_tree_root(cx);
            self.rebuild_file_tree(cx);
        }
        self.sync_file_directory_watches(cx);
        if !self.file_manager.sidebar_visible && self.file_manager.sidebar_focused {
            self.focus_document(cx);
        }
        cx.notify();
    }

    pub(crate) fn dired_move(&mut self, delta: i64, cx: &mut Context<Self>) {
        if let Some(session) = self.file_manager.session.as_mut() {
            session.move_cursor(delta);
            if let Some(id) = session.cursor()
                && let Some(index) = session.entries().iter().position(|entry| entry.id == id)
                && self.content_route == ContentRoute::FileManager
            {
                self.file_manager.list_state.scroll_to_reveal_item(index);
            }
            cx.notify();
        }
    }

    pub(crate) fn dired_open_selected(&mut self, cx: &mut Context<Self>) {
        let opened_from_full_page = self.content_route == ContentRoute::FileManager;
        let current_directory = self
            .file_manager
            .session
            .as_ref()
            .map(|session| session.directory().to_path_buf());
        let selected = self
            .file_manager
            .session
            .as_ref()
            .and_then(|session| session.selected())
            .map(|entry| (entry.path.to_path_buf(), entry.kind));
        match selected {
            Some((path, EntryKind::Parent)) => {
                let anchor = current_directory.as_deref().and_then(|current| {
                    self.file_manager
                        .session
                        .as_ref()
                        .map(|session| session.anchor_for_path(current))
                });
                let selection = anchor
                    .map(SelectionIntent::Explicit)
                    .unwrap_or(SelectionIntent::Restore);
                let intent = DiredSession::intent_for(path, NavigationCause::Up, selection);
                self.navigate_file_manager(intent, cx);
            }
            Some((path, EntryKind::Directory)) => {
                self.open_file_manager(path, cx);
            }
            Some((path, EntryKind::Image)) => self.open(path, cx),
            Some((path, EntryKind::OrgFile | EntryKind::Markdown | EntryKind::CodeFile)) => {
                let already_open = matches!(
                    &self.state,
                    WorkspaceLoadState::Loading { path: current, .. }
                        if current == &path
                ) || self.document_path(cx) == Some(path.as_path());
                self.content_route = ContentRoute::Document;
                if opened_from_full_page {
                    self.file_manager.sidebar_focused = false;
                    self.request_document_focus(cx);
                }
                if !self.file_manager.sidebar_visible {
                    self.sync_file_directory_watches(cx);
                }
                if already_open {
                    cx.notify();
                } else {
                    self.open(path, cx);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn dired_up(&mut self, cx: &mut Context<Self>) {
        let current = self
            .file_manager
            .session
            .as_ref()
            .map(|session| session.directory().to_path_buf());
        let parent = current
            .as_deref()
            .and_then(std::path::Path::parent)
            .map(PathBuf::from);
        if let (Some(parent), Some(current)) = (parent, current) {
            let anchor = self
                .file_manager
                .session
                .as_ref()
                .expect("dired session exists")
                .anchor_for_path(&current);
            let intent = DiredSession::intent_for(
                parent,
                NavigationCause::Up,
                SelectionIntent::Explicit(anchor),
            );
            self.navigate_file_manager(intent, cx);
        }
    }

    pub(crate) fn reload_file_manager(&mut self, cx: &mut Context<Self>) {
        self.refresh_file_manager(NavigationCause::Refresh, cx);
    }

    pub(crate) fn refresh_file_manager(&mut self, cause: NavigationCause, cx: &mut Context<Self>) {
        if self.file_manager.dired_load.is_some() {
            self.file_manager.refresh_pending = true;
            return;
        }
        if let Some(directory) = self
            .file_manager
            .session
            .as_ref()
            .map(|session| session.directory().to_path_buf())
        {
            let intent = DiredSession::intent_for(directory, cause, SelectionIntent::Preserve);
            self.navigate_file_manager(intent, cx);
        }
    }

    pub(crate) fn dired_mark(&mut self, mark: Mark, cx: &mut Context<Self>) {
        if let Some(session) = self.file_manager.session.as_mut() {
            session.mark_selected(mark);
            self.reveal_dired_cursor();
            cx.notify();
        }
    }
    pub(crate) fn dired_unmark(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.file_manager.session.as_mut() {
            session.unmark_selected();
            self.reveal_dired_cursor();
            cx.notify();
        }
    }
    pub(crate) fn dired_unmark_all(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.file_manager.session.as_mut() {
            session.unmark_all();
            cx.notify();
        }
    }
    pub(crate) fn dired_invert_marks(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.file_manager.session.as_mut() {
            session.invert_marks();
            cx.notify();
        }
    }

    fn reveal_dired_cursor(&self) {
        let Some(session) = self.file_manager.session.as_ref() else {
            return;
        };
        let Some(id) = session.cursor() else {
            return;
        };
        let Some(index) = session.entries().iter().position(|entry| entry.id == id) else {
            return;
        };
        if self.content_route == ContentRoute::FileManager {
            self.file_manager.list_state.scroll_to_reveal_item(index);
        }
    }
    pub(crate) fn show_dired_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.end_prefix(cx);
        self.file_manager.help_visible = true;
        cx.notify();
    }

    pub(crate) fn workspace_body(
        &self,
        entity: Entity<Self>,
        viewport_width: f32,
        minimap_reveal: f32,
        window: &Window,
        cx: &gpui::App,
        titlebar_inset: f32,
    ) -> gpui::Div {
        let (sidebar_reveal, sidebar_animating) = if self.content_route == ContentRoute::Document {
            self.sidebar_reveal_at(Instant::now())
        } else {
            (0.0, false)
        };
        if sidebar_animating {
            window.request_animation_frame();
        }
        let body = match self.content_route {
            ContentRoute::Image => div().size_full().when_some(
                self.image_viewer.path.as_ref().zip(self.image_viewer.size),
                |body, (path, size)| {
                    body.child(super::image_viewer::render_image_viewer(
                        entity.clone(),
                        path,
                        size,
                        &self.image_viewer,
                        titlebar_inset,
                        window,
                    ))
                },
            ),
            ContentRoute::Agenda => div().size_full().child(self.agenda.render(
                entity.clone(),
                viewport_width,
                window,
                !cx.reduce_motion(),
                titlebar_inset,
            )),
            ContentRoute::AgendaText => div()
                .size_full()
                .p(px(12.0))
                .when_some(self.agenda.text_editor.clone(), |body, editor| {
                    body.child(editor)
                }),
            ContentRoute::FileManager => {
                self.full_page_file_manager(entity.clone(), viewport_width, window, cx)
            }
            ContentRoute::Document if sidebar_reveal > 0.0 => {
                let full_sidebar_width = self.rendered_sidebar_width(viewport_width);
                let sidebar_width = full_sidebar_width * sidebar_reveal;
                let resize_handle_width = sidebar::RESIZE_HANDLE_PX * sidebar_reveal;
                let editor_width = (viewport_width - sidebar_width - resize_handle_width)
                    .max(sidebar::MIN_DOCUMENT_WIDTH_PX);
                let resize_entity = entity.clone();
                let document_entity = entity.clone();
                div()
                    .size_full()
                    .flex()
                    .child(
                        div()
                            .flex_none()
                            .w(px(sidebar_width))
                            .h_full()
                            .overflow_hidden()
                            .child(
                                self.file_sidebar(entity.clone(), full_sidebar_width, window, cx)
                                    .relative()
                                    .left(px(-(1.0 - sidebar_reveal) * full_sidebar_width)),
                            ),
                    )
                    .child(
                        div()
                            .id("file-sidebar-resize-handle")
                            .w(px(resize_handle_width))
                            .h_full()
                            .flex_none()
                            .flex()
                            .justify_center()
                            .cursor(CursorStyle::ResizeLeftRight)
                            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                                cx.stop_propagation();
                                resize_entity.update(cx, |this, cx| {
                                    if event.click_count >= 2 {
                                        this.reset_sidebar_width(cx);
                                    } else {
                                        this.begin_sidebar_resize(
                                            f32::from(event.position.x),
                                            viewport_width,
                                            cx,
                                        );
                                    }
                                });
                            })
                            .child(div().w(px(1.0)).h_full().bg(rgb(current_theme().border))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                document_entity.update(cx, |this, cx| {
                                    this.file_manager.context_menu = None;
                                    this.focus_document(cx);
                                });
                            })
                            .child(self.body(
                                entity.clone(),
                                editor_width,
                                minimap_reveal,
                                window,
                                cx,
                            )),
                    )
            }
            ContentRoute::Document => {
                self.body(entity.clone(), viewport_width, minimap_reveal, window, cx)
            }
        };
        let gesture_entity = entity.clone();
        body.relative()
            .when(self.content_route == ContentRoute::Document, |body| {
                body.on_scroll_wheel(move |event, _, cx| {
                    gesture_entity.update(cx, |this, cx| this.file_sidebar_scroll(event, cx));
                })
            })
            .when_some(self.file_manager.context_menu, |body, menu| {
                body.child(dired_context_menu(entity.clone(), menu))
            })
            .when_some(self.file_manager.tree.menu.as_ref(), |body, menu| {
                body.child(self.render_file_tree_menu(entity, menu, window))
            })
    }

    fn full_page_file_manager(
        &self,
        entity: Entity<Self>,
        viewport_width: f32,
        window: &Window,
        cx: &gpui::App,
    ) -> gpui::Div {
        let Some(session) = self.file_manager.session.as_ref() else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child("Choose a folder to begin");
        };
        let entries = session.entries().clone();
        let marks = session.visible_marks();
        let operation_targets = session.operation_targets();
        let cursor = session.cursor();
        let directory = session.directory();
        let status_snapshot = self.status_snapshot(cx);
        let status_line = status_snapshot.as_ref().map(|snapshot| {
            let layout = self.status_layout(snapshot, viewport_width, window);
            super::status_line::render_status_line(snapshot, layout, entity.clone(), window)
        });
        let status_popover = status_snapshot.as_ref().and_then(|snapshot| {
            self.status.popover_for(snapshot.pane).map(|popover| {
                let layout = self.status_layout(snapshot, viewport_width, window);
                let popover_anchors =
                    super::status_line::status_popover_anchors(snapshot, &layout, window, None);
                super::status_line::render_status_popover(
                    popover,
                    Some(snapshot),
                    self.status.settings(),
                    entity.clone(),
                    self.language,
                    viewport_width,
                    popover_anchors,
                )
            })
        });
        let dismiss_entity = entity.clone();
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(current_theme().background))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                dismiss_entity.update(cx, |this, cx| {
                    if this.file_manager.context_menu.take().is_some() {
                        cx.notify();
                    }
                });
            })
            .child(
                div()
                    .h(ui_px(50.0))
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(current_theme().border))
                    .child(
                        div()
                            .w(ui_px(24.0))
                            .h(ui_px(24.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_sm()
                            .bg(rgb(current_theme().heading[0]))
                            .text_color(rgb(0xffffff))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_size(ui_px(11.0))
                            .child("F"),
                    )
                    .child(breadcrumb::file_manager_breadcrumb(
                        entity.clone(),
                        directory,
                    ))
                    .child(
                        div()
                            .px_2()
                            .h(ui_px(22.0))
                            .flex()
                            .items_center()
                            .rounded_sm()
                            .bg(rgb(current_theme().background_alt))
                            .text_color(rgb(current_theme().foreground_dim))
                            .text_size(ui_px(10.5))
                            .child_text(format!("{} items", entries.len())),
                    ),
            )
            .child(
                div()
                    .h(ui_px(30.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(current_theme().border))
                    .bg(rgb(current_theme().background_alt))
                    .text_color(rgb(current_theme().foreground_dim))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_size(ui_px(10.0))
                    .child(div().w(ui_px(38.0)).child("MARK"))
                    .child(div().w(ui_px(36.0)).child("TYPE"))
                    .child(div().flex_1().child("NAME"))
                    .child(div().w(px(104.0)).child("MODIFIED"))
                    .child(div().w(px(84.0)).flex().justify_end().child("SIZE")),
            )
            .child(
                div().flex_1().min_h(px(0.0)).child(
                    list(
                        self.file_manager.list_state.clone(),
                        move |index, window, _| {
                            let drag_sources = if marks.contains_key(&entries[index].id) {
                                operation_targets.clone()
                            } else if matches!(entries[index].kind, EntryKind::Parent) {
                                Arc::from([])
                            } else {
                                vec![entries[index].path.clone()].into()
                            };
                            dired_row(
                                entity.clone(),
                                &entries[index],
                                marks.get(&entries[index].id).copied(),
                                cursor == Some(entries[index].id),
                                drag_sources,
                                window,
                            )
                        },
                    )
                    .size_full(),
                ),
            )
            .when_some(status_line, |manager, status_line| {
                manager.child(status_line)
            })
            .when_some(status_popover, |manager, popover| manager.child(popover))
    }
}

fn dired_context_menu(entity: Entity<WorkspaceWindow>, menu: DiredContextMenu) -> gpui::AnyElement {
    let items = [
        ("Open", "RET", DiredContextAction::Open),
        ("Rename…", "R", DiredContextAction::Rename),
        ("Copy to…", "C", DiredContextAction::Copy),
        ("Move to…", "M", DiredContextAction::Move),
        ("Move to Trash…", "D", DiredContextAction::Trash),
    ];
    div()
        .id("dired-context-menu")
        .absolute()
        .left(menu.position.x)
        .top(menu.position.y)
        .w(px(184.0))
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(rgb(current_theme().border))
        .bg(rgb(current_theme().background_alt))
        .shadow_lg()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .children(items.into_iter().map(|(label, key, action)| {
            let entity = entity.clone();
            div()
                .id(("dired-context-action", action as usize))
                .h(ui_px(28.0))
                .px_2()
                .flex()
                .items_center()
                .cursor_pointer()
                .text_size(ui_px(11.5))
                .hover(|style| style.bg(rgb(current_theme().code_boundary_background)))
                .child(div().flex_1().child_text(label))
                .child(
                    div()
                        .text_size(ui_px(10.0))
                        .text_color(rgb(current_theme().foreground_dim))
                        .child(key),
                )
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    entity.update(cx, |this, cx| {
                        this.file_manager.context_menu = None;
                        match action {
                            DiredContextAction::Open => this.dired_open_selected(cx),
                            DiredContextAction::Rename => this.dired_rename(cx),
                            DiredContextAction::Copy => this.dired_copy(cx),
                            DiredContextAction::Move => this.dired_move_to(cx),
                            DiredContextAction::Trash => this.dired_trash(window, cx),
                        }
                        cx.notify();
                    });
                })
        }))
        .into_any_element()
}

fn dired_row(
    entity: Entity<WorkspaceWindow>,
    entry: &crate::file_manager::FileEntry,
    mark: Option<Mark>,
    selected: bool,
    drag_sources: Arc<[Arc<std::path::Path>]>,
    window: &Window,
) -> gpui::AnyElement {
    let id = entry.id;
    let mark_text = match mark {
        Some(Mark::Selected) => "*",
        Some(Mark::Delete) => "D",
        None => "",
    };
    let icon_color = if entry.is_directory() {
        crate::theme::navigation_colors().accent
    } else {
        crate::theme::navigation_colors().secondary
    };
    let modified = entry
        .metadata
        .modified
        .map(format_modified)
        .unwrap_or_default();
    let size = entry
        .metadata
        .byte_len
        .map(format_bytes)
        .unwrap_or_default();
    let row = div()
        .id(("dired-row", id.0 as usize))
        .w_full()
        .h(ui_px(34.0))
        .px_3()
        .flex()
        .items_center()
        .cursor_pointer()
        .border_b_1()
        .border_color(rgb(current_theme().background_alt))
        .text_size(ui_px(12.0))
        .when(selected, |row| {
            row.bg(rgb(current_theme().code_boundary_background))
        })
        .hover(|style| style.bg(rgb(current_theme().background_alt)));
    with_dired_row_interactions(
        row,
        entity,
        id,
        entry.path.clone(),
        entry.is_directory(),
        drag_sources,
    )
    .child(
        div()
            .w(ui_px(38.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgb(if mark == Some(Mark::Delete) {
                current_theme().type_name
            } else {
                current_theme().heading[3]
            }))
            .child(mark_text),
    )
    .child(
        div()
            .w(ui_px(36.0))
            .child(crate::components::icon::file_icon(
                entry.kind,
                ui_px(16.0),
                icon_color,
                window,
            )),
    )
    .child(
        div()
            .flex_1()
            .text_size(ui_px(12.5))
            .child_text(entry.display_name.to_string()),
    )
    .child(
        div()
            .w(px(104.0))
            .text_color(rgb(current_theme().foreground_dim))
            .child_text(modified),
    )
    .child(
        div()
            .w(px(84.0))
            .flex()
            .justify_end()
            .text_color(rgb(current_theme().foreground_dim))
            .child_text(size),
    )
    .into_any_element()
}

fn with_dired_row_interactions(
    row: gpui::Stateful<gpui::Div>,
    entity: Entity<WorkspaceWindow>,
    id: crate::file_manager::EntryId,
    drop_directory: Arc<Path>,
    is_directory: bool,
    drag_sources: Arc<[Arc<Path>]>,
) -> gpui::Stateful<gpui::Div> {
    let click_entity = entity.clone();
    let menu_entity = entity.clone();
    let drop_entity = entity;
    let can_drag = !drag_sources.is_empty();
    row.on_click(move |event, _, cx| {
        if event.is_right_click() {
            return;
        }
        click_entity.update(cx, |this, cx| {
            this.file_manager.context_menu = None;
            if let Some(session) = this.file_manager.session.as_mut() {
                session.set_cursor(id);
            }
            if event.click_count() >= 2 {
                this.dired_open_selected(cx);
            } else {
                cx.notify();
            }
        });
    })
    .on_mouse_down(MouseButton::Right, move |event, _, cx| {
        cx.stop_propagation();
        menu_entity.update(cx, |this, cx| {
            if let Some(session) = this.file_manager.session.as_mut() {
                session.set_cursor(id);
            }
            this.file_manager.context_menu = Some(DiredContextMenu {
                position: event.position,
            });
            cx.notify();
        });
    })
    .when(can_drag, |row| {
        row.on_drag(
            DiredDrag {
                sources: drag_sources,
            },
            |_, _, _, cx| cx.new(|_| gpui::Empty),
        )
    })
    .when(is_directory, |row| {
        row.on_drop::<DiredDrag>(move |drag, _, cx| {
            cx.stop_propagation();
            let plan = OperationPlan::move_to(
                drag.sources.clone(),
                drop_directory.to_path_buf(),
                ConflictPolicy::Error,
            );
            drop_entity.update(cx, |this, cx| this.execute_dired_operation(plan, cx));
        })
    })
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn format_modified(modified: SystemTime) -> String {
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO);
    if age.as_secs() < 60 {
        "now".to_string()
    } else if age.as_secs() < 3_600 {
        format!("{}m ago", age.as_secs() / 60)
    } else if age.as_secs() < 86_400 {
        format!("{}h ago", age.as_secs() / 3_600)
    } else {
        format!("{}d ago", age.as_secs() / 86_400)
    }
}

fn absolute_directory(directory: PathBuf) -> PathBuf {
    if directory.as_os_str().is_empty() {
        return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    }
    if directory.is_absolute() {
        directory
    } else {
        std::env::current_dir()
            .map(|current| current.join(&directory))
            .unwrap_or(directory)
    }
}
