use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};

use gpui::{
    Context, Entity, InteractiveElement, ParentElement, PathPromptOptions, Styled, div, list,
    prelude::*, px, rgb,
};

use super::{ContentRoute, PreviewApp, PreviewLoadState};
use crate::{
    file_manager::{DiredSession, EntryKind, Mark, scan_directory_cancellable},
    navigation::{NavigationCause, SelectionIntent},
    theme::current_theme,
};

impl PreviewApp {
    fn document_path(&self) -> Option<&std::path::Path> {
        match &self.state {
            PreviewLoadState::Loading { path } | PreviewLoadState::Failed { path, .. } => {
                Some(path)
            }
            PreviewLoadState::Ready { document, .. } => Some(&document.path),
            PreviewLoadState::Empty => None,
        }
    }

    pub(super) fn choose_directory(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open File Manager".into()),
        });
        self.dired_task = Some(cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.open_file_manager(path, cx));
        }));
    }

    pub(super) fn open_default_dired(&mut self, cx: &mut Context<Self>) {
        let directory = self
            .dired
            .as_ref()
            .map(|session| session.directory().to_path_buf())
            .or_else(|| {
                self.document_path()
                    .and_then(|path| path.parent().map(PathBuf::from))
            })
            .or_else(|| std::env::current_dir().ok());
        if let Some(directory) = directory {
            self.open_file_manager(directory, cx);
        }
    }

    pub(super) fn open_file_manager(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        let directory = absolute_directory(directory);
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
        if let Some(current) = self
            .dired
            .as_ref()
            .map(|session| session.directory().to_path_buf())
        {
            let full = self.dired_list_state.logical_scroll_top();
            let sidebar = self.sidebar_list_state.logical_scroll_top();
            self.dired_viewport_memory.insert(
                current.clone(),
                (full.item_ix, full.offset_in_item.to_f64() as f32),
            );
            self.sidebar_viewport_memory.insert(
                current,
                (sidebar.item_ix, sidebar.offset_in_item.to_f64() as f32),
            );
        }
        self.content_route = ContentRoute::FileManager;
        let session = self
            .dired
            .get_or_insert_with(|| DiredSession::empty(directory.clone()));
        let viewport = self.dired_list_state.logical_scroll_top();
        session.capture_viewport(viewport.item_ix, viewport.offset_in_item.to_f64() as f32);
        let load = session.begin_navigation(intent);
        self.start_file_manager_load(load, cx);
    }

    fn start_file_manager_load(
        &mut self,
        load: crate::file_manager::NavigationLoad,
        cx: &mut Context<Self>,
    ) {
        let directory = load.intent.target.clone();
        self.content_route = ContentRoute::FileManager;
        self.dired_error = None;
        self.install_dired_keymap();
        let scan_directory = directory.clone();
        let cancellation = load.cancellation.clone();
        let scan = cx.background_spawn(async move {
            scan_directory_cancellable(scan_directory, &cancellation)
        });
        self.dired_task = Some(cx.spawn(async move |this, cx| {
            let result = scan.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(result) => {
                        if let Some(session) = this.dired.as_mut()
                            && let Some(commit) = session.apply_scan(&load, result)
                        {
                            this.dired_list_state.reset(commit.item_count);
                            this.sidebar_list_state.reset(commit.item_count);
                            this.dired_pending_presentation =
                                commit.presentation_rank.map(|rank| {
                                    (
                                        commit.transaction_id,
                                        commit.view_revision,
                                        rank,
                                        commit.presentation_offset,
                                    )
                                });
                            this.sidebar_pending_presentation = this
                                .sidebar_viewport_memory
                                .get(&directory)
                                .copied()
                                .and_then(|(rank, offset)| {
                                    (commit.item_count > 0)
                                        .then_some((rank.min(commit.item_count - 1), offset))
                                })
                                .or_else(|| commit.presentation_rank.map(|rank| (rank, 0.0)))
                                .map(|(rank, offset)| {
                                    (commit.transaction_id, commit.view_revision, rank, offset)
                                });
                            this.dired_presentation_scheduled = false;
                        }
                    }
                    Err(error) => {
                        if error.source.kind() != std::io::ErrorKind::Interrupted
                            && this.dired.as_ref().is_some_and(|session| {
                                session.accepts_transaction(load.transaction_id)
                            })
                        {
                            this.dired_error = Some(Arc::from(format!(
                                "{}: {}",
                                error.directory.display(),
                                error.source
                            )));
                        }
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn dired_history(&mut self, forward: bool, cx: &mut Context<Self>) {
        let viewport = self.dired_list_state.logical_scroll_top();
        if let Some(current) = self
            .dired
            .as_ref()
            .map(|session| session.directory().to_path_buf())
        {
            let sidebar = self.sidebar_list_state.logical_scroll_top();
            self.dired_viewport_memory.insert(
                current.clone(),
                (viewport.item_ix, viewport.offset_in_item.to_f64() as f32),
            );
            self.sidebar_viewport_memory.insert(
                current,
                (sidebar.item_ix, sidebar.offset_in_item.to_f64() as f32),
            );
        }
        let load = self.dired.as_mut().and_then(|session| {
            session.capture_viewport(viewport.item_ix, viewport.offset_in_item.to_f64() as f32);
            session.begin_history_navigation(forward)
        });
        if let Some(load) = load {
            self.start_file_manager_load(load, cx);
        }
    }

    pub(super) fn return_to_document(&mut self, cx: &mut Context<Self>) {
        self.content_route = ContentRoute::Document;
        self.install_preview_keymap();
        cx.notify();
    }

    pub(super) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        if self.sidebar_visible
            && self.dired.is_none()
            && let Some(path) = self
                .document_path()
                .and_then(|path| path.parent().map(PathBuf::from))
        {
            self.open_file_manager(path, cx);
            self.content_route = ContentRoute::Document;
            self.install_preview_keymap();
        }
        cx.notify();
    }

    pub(super) fn dired_move(&mut self, delta: i64, cx: &mut Context<Self>) {
        if let Some(session) = self.dired.as_mut() {
            session.move_cursor(delta);
            if let Some(id) = session.cursor()
                && let Some(index) = session.entries().iter().position(|entry| entry.id == id)
            {
                self.dired_list_state.scroll_to_reveal_item(index);
            }
            cx.notify();
        }
    }

    pub(super) fn dired_open_selected(&mut self, cx: &mut Context<Self>) {
        let current_directory = self
            .dired
            .as_ref()
            .map(|session| session.directory().to_path_buf());
        let selected = self
            .dired
            .as_ref()
            .and_then(|session| session.selected())
            .map(|entry| (entry.path.to_path_buf(), entry.kind));
        match selected {
            Some((path, EntryKind::Parent)) => {
                let anchor = current_directory.as_deref().and_then(|current| {
                    self.dired
                        .as_ref()
                        .map(|session| session.anchor_for_path(current))
                });
                let selection = anchor
                    .map(SelectionIntent::Explicit)
                    .unwrap_or(SelectionIntent::Restore);
                let intent = DiredSession::intent_for(path, NavigationCause::Up, selection);
                self.navigate_file_manager(intent, cx);
            }
            Some((path, EntryKind::Directory)) => self.open_file_manager(path, cx),
            Some((path, EntryKind::OrgFile | EntryKind::Markdown)) => {
                self.content_route = ContentRoute::Document;
                self.install_preview_keymap();
                self.open(path, cx);
            }
            _ => {}
        }
    }

    pub(super) fn dired_up(&mut self, cx: &mut Context<Self>) {
        let current = self
            .dired
            .as_ref()
            .map(|session| session.directory().to_path_buf());
        let parent = current
            .as_deref()
            .and_then(std::path::Path::parent)
            .map(PathBuf::from);
        if let (Some(parent), Some(current)) = (parent, current) {
            let anchor = self
                .dired
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

    pub(super) fn reload_file_manager(&mut self, cx: &mut Context<Self>) {
        if let Some(directory) = self
            .dired
            .as_ref()
            .map(|session| session.directory().to_path_buf())
        {
            let intent = DiredSession::intent_for(
                directory,
                NavigationCause::Refresh,
                SelectionIntent::Preserve,
            );
            self.navigate_file_manager(intent, cx);
        }
    }

    pub(super) fn dired_mark(&mut self, mark: Mark, cx: &mut Context<Self>) {
        if let Some(session) = self.dired.as_mut() {
            session.mark_selected(mark);
            cx.notify();
        }
    }
    pub(super) fn dired_unmark(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.dired.as_mut() {
            session.unmark_selected();
            cx.notify();
        }
    }
    pub(super) fn dired_unmark_all(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.dired.as_mut() {
            session.unmark_all();
            cx.notify();
        }
    }
    pub(super) fn dired_invert_marks(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.dired.as_mut() {
            session.invert_marks();
            cx.notify();
        }
    }
    pub(super) fn dired_prepare_execute(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.dired.as_ref() {
            let count = session.deletion_plan().delete.len();
            self.dired_error = Some(if count == 0 {
                Arc::from("No files are flagged for deletion")
            } else {
                Arc::from(format!(
                    "{count} file(s) flagged; destructive execution requires confirmation UI"
                ))
            });
            cx.notify();
        }
    }

    pub(super) fn show_dired_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.which_key_request = self.which_key_request.wrapping_add(1);
        self.which_key_items = Arc::new(super::dired_command_items(&self.commands));
        self.dired_help_visible = true;
        cx.notify();
    }

    pub(super) fn workspace_body(&self, entity: Entity<Self>) -> gpui::Div {
        match self.content_route {
            ContentRoute::FileManager => self.full_page_file_manager(entity),
            ContentRoute::Document if self.sidebar_visible => div()
                .size_full()
                .flex()
                .child(self.file_sidebar(entity.clone()))
                .child(
                    div()
                        .w(px(5.0))
                        .h_full()
                        .border_l_1()
                        .border_r_1()
                        .border_color(rgb(current_theme().border))
                        .bg(rgb(current_theme().background_alt)),
                )
                .child(div().flex_1().h_full().child(self.body(entity))),
            ContentRoute::Document => self.body(entity),
        }
    }

    fn full_page_file_manager(&self, entity: Entity<Self>) -> gpui::Div {
        let Some(session) = self.dired.as_ref() else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child("Choose a folder to begin");
        };
        let entries = session.entries().clone();
        let marks = Arc::new(session.visible_marks());
        let cursor = session.cursor();
        let directory = session.directory().display().to_string();
        let status = self.dired_error.clone().unwrap_or_else(|| {
            Arc::from(format!(
                "{} entries  |  {} marked",
                entries.len(),
                session.marked_count()
            ))
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(current_theme().background))
            .child(
                div()
                    .h(px(50.0))
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(current_theme().border))
                    .child(
                        div()
                            .w(px(24.0))
                            .h(px(24.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_sm()
                            .bg(rgb(current_theme().heading[0]))
                            .text_color(rgb(0xffffff))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_size(px(11.0))
                            .child("F"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(13.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(directory),
                    )
                    .child(
                        div()
                            .px_2()
                            .h(px(22.0))
                            .flex()
                            .items_center()
                            .rounded_sm()
                            .bg(rgb(current_theme().background_alt))
                            .text_color(rgb(current_theme().foreground_dim))
                            .text_size(px(10.5))
                            .child(format!("{} items", entries.len())),
                    ),
            )
            .child(
                div()
                    .h(px(30.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(current_theme().border))
                    .bg(rgb(current_theme().background_alt))
                    .text_color(rgb(current_theme().foreground_dim))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_size(px(10.0))
                    .child(div().w(px(38.0)).child("MARK"))
                    .child(div().w(px(36.0)).child("TYPE"))
                    .child(div().flex_1().child("NAME"))
                    .child(div().w(px(104.0)).child("MODIFIED"))
                    .child(div().w(px(84.0)).flex().justify_end().child("SIZE")),
            )
            .child(
                div().flex_1().min_h(px(0.0)).child(
                    list(self.dired_list_state.clone(), move |index, _, _| {
                        dired_row(
                            entity.clone(),
                            &entries[index],
                            marks.get(&entries[index].id).copied(),
                            cursor == Some(entries[index].id),
                        )
                    })
                    .size_full(),
                ),
            )
            .child(
                div()
                    .h(px(30.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(current_theme().border))
                    .bg(rgb(current_theme().background_alt))
                    .text_color(rgb(current_theme().foreground_dim))
                    .text_size(px(10.5))
                    .child(status.to_string()),
            )
    }

    fn file_sidebar(&self, entity: Entity<Self>) -> gpui::Div {
        let entries = self
            .dired
            .as_ref()
            .map(|session| session.entries().clone())
            .unwrap_or_default();
        let cursor = self.dired.as_ref().and_then(|session| session.cursor());
        div()
            .w(px(232.0))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(current_theme().background))
            .child(
                div()
                    .h(px(42.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(current_theme().border))
                    .child(
                        div()
                            .w(px(20.0))
                            .h(px(20.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_sm()
                            .bg(rgb(current_theme().heading[0]))
                            .text_color(rgb(0xffffff))
                            .text_size(px(11.0))
                            .child("F"),
                    )
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_size(px(12.5))
                            .child("Files"),
                    ),
            )
            .child(
                div().flex_1().min_h(px(0.0)).child(
                    list(self.sidebar_list_state.clone(), move |index, _, _| {
                        sidebar_row(
                            entity.clone(),
                            &entries[index],
                            cursor == Some(entries[index].id),
                        )
                    })
                    .size_full(),
                ),
            )
    }
}

fn dired_row(
    entity: Entity<PreviewApp>,
    entry: &crate::file_manager::FileEntry,
    mark: Option<Mark>,
    selected: bool,
) -> gpui::AnyElement {
    let id = entry.id;
    let mark_text = match mark {
        Some(Mark::Selected) => "*",
        Some(Mark::Delete) => "D",
        None => "",
    };
    let (icon, icon_color) = sidebar_icon(entry.kind);
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
    div()
        .id(("dired-row", id.0 as usize))
        .w_full()
        .h(px(34.0))
        .px_3()
        .flex()
        .items_center()
        .cursor_pointer()
        .border_b_1()
        .border_color(rgb(current_theme().background_alt))
        .text_size(px(12.0))
        .when(selected, |row| {
            row.bg(rgb(current_theme().code_boundary_background))
        })
        .hover(|style| style.bg(rgb(current_theme().background_alt)))
        .on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| {
                if let Some(session) = this.dired.as_mut() {
                    session.set_cursor(id);
                    cx.notify();
                }
            })
        })
        .child(
            div()
                .w(px(38.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(if mark == Some(Mark::Delete) {
                    current_theme().type_name
                } else {
                    current_theme().heading[3]
                }))
                .child(mark_text),
        )
        .child(
            div().w(px(36.0)).child(
                div()
                    .w(px(20.0))
                    .h(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .bg(rgb(icon_color))
                    .text_color(rgb(0xffffff))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_size(px(9.5))
                    .child(icon),
            ),
        )
        .child(
            div()
                .flex_1()
                .text_size(px(12.5))
                .child(entry.display_name.to_string()),
        )
        .child(
            div()
                .w(px(104.0))
                .text_color(rgb(current_theme().foreground_dim))
                .child(modified),
        )
        .child(
            div()
                .w(px(84.0))
                .flex()
                .justify_end()
                .text_color(rgb(current_theme().foreground_dim))
                .child(size),
        )
        .into_any_element()
}

fn sidebar_row(
    entity: Entity<PreviewApp>,
    entry: &crate::file_manager::FileEntry,
    selected: bool,
) -> gpui::AnyElement {
    let id = entry.id;
    let (icon, color) = sidebar_icon(entry.kind);
    div()
        .id(("sidebar-row", id.0 as usize))
        .mx_1()
        .my(px(1.0))
        .h(px(30.0))
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .rounded_sm()
        .cursor_pointer()
        .text_size(px(11.5))
        .when(selected, |row| {
            row.bg(rgb(current_theme().code_boundary_background))
        })
        .hover(|style| style.bg(rgb(current_theme().background_alt)))
        .on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| {
                if let Some(session) = this.dired.as_mut() {
                    session.set_cursor(id);
                }
                cx.notify();
                this.dired_open_selected(cx);
            })
        })
        .child(
            div()
                .w(px(20.0))
                .h(px(20.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .bg(rgb(color))
                .text_color(rgb(0xffffff))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_size(px(10.0))
                .child(icon),
        )
        .child(div().flex_1().child(entry.display_name.to_string()))
        .into_any_element()
}

fn sidebar_icon(kind: EntryKind) -> (&'static str, u32) {
    match kind {
        EntryKind::Parent => ("..", current_theme().foreground_dim),
        EntryKind::Directory => ("D", current_theme().heading[3]),
        EntryKind::OrgFile => ("O", current_theme().heading[1]),
        EntryKind::Markdown => ("M", current_theme().heading[0]),
        EntryKind::Image => ("I", current_theme().heading[2]),
        EntryKind::Symlink => ("@", current_theme().meta),
        EntryKind::RegularFile => ("F", current_theme().foreground_dim),
    }
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
