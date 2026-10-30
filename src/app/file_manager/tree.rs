//! Outline host. File data and mutations belong to the shared file manager.
mod drag;
mod input;
mod menu;
#[cfg(test)]
mod tests;
mod view;
#[cfg(feature = "visual-check")]
pub(super) mod visual_check;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use gpui::{
    AppContext, Context, Entity, FocusHandle, ListAlignment, ListOffset, ListState, Pixels,
    Subscription, Window, px,
};

use menu::{MenuKind, TreeMenu};

use crate::{
    app::{ContentRoute, WorkspaceWindow},
    command::FileTreeCommand,
    components::native_input::{InputConfig, InputEvent, NativeInput},
    file_manager::{
        EntryId, FileEntry, FileTree, SortDirection, SortKey, TreeMemory, TreeSelection,
    },
};

const TOOLBAR_COMMANDS: [FileTreeCommand; 4] = [
    FileTreeCommand::ParentRoot,
    FileTreeCommand::EnterRoot,
    FileTreeCommand::NewMenu,
    FileTreeCommand::MoreMenu,
];

pub(super) struct NameEdit {
    pub(super) input: Entity<NativeInput>,
    pub(super) directory: PathBuf,
    pub(super) source: Option<Arc<Path>>,
    pub(super) folder: bool,
    pub(super) error: Option<String>,
    pub(super) _subscription: Subscription,
}

pub(super) struct TreeHost {
    pub(super) model: FileTree,
    pub(super) list: ListState,
    pub(super) input: Option<Entity<NativeInput>>,
    subscription: Option<Subscription>,
    pub(super) toolbar: Option<[FocusHandle; TOOLBAR_COMMANDS.len()]>,
    pub(super) toolbar_bounds: Rc<Cell<[gpui::Bounds<Pixels>; TOOLBAR_COMMANDS.len()]>>,
    pub(super) menu: Option<TreeMenu>,
    pub(super) edit: Option<NameEdit>,
    pub(super) focus_requested: bool,
    pub(super) type_ahead_task: Option<gpui::Task<()>>,
    pub(super) bounds: Rc<Cell<gpui::Bounds<Pixels>>>,
    pub(super) row_bounds: Rc<RefCell<HashMap<crate::file_manager::EntryId, gpui::Bounds<Pixels>>>>,
    pub(super) horizontal_scroll: gpui::ScrollHandle,
    pub(super) retry_focus: HashMap<PathBuf, FocusHandle>,
    observed_document: Option<PathBuf>,
    pending_open: Option<(PathBuf, bool)>,
    drag: drag::TreeDrag,
    rendered_root: Option<PathBuf>,
    pub(super) persist: bool,
}

impl Default for TreeHost {
    fn default() -> Self {
        Self {
            model: FileTree::default(),
            list: ListState::new(0, ListAlignment::Top, px(56.)),
            input: None,
            subscription: None,
            toolbar: None,
            toolbar_bounds: Default::default(),
            menu: None,
            edit: None,
            focus_requested: false,
            type_ahead_task: None,
            bounds: Default::default(),
            row_bounds: Default::default(),
            horizontal_scroll: Default::default(),
            retry_focus: Default::default(),
            observed_document: None,
            pending_open: None,
            drag: Default::default(),
            rendered_root: None,
            persist: false,
        }
    }
}

impl TreeHost {
    fn directory_root_target(&self, targets: Option<&[Arc<Path>]>) -> Option<&Path> {
        let entry = match targets {
            Some([path]) => self.model.entry(EntryId::for_path(path)),
            Some(_) => None,
            None => self.model.single_selected(),
        }?;
        (entry.is_directory() && self.model.root() != Some(entry.path.as_ref()))
            .then_some(entry.path.as_ref())
    }

    fn operation_entry(&self) -> Option<&Arc<FileEntry>> {
        let id = match &self.menu {
            Some(menu) => EntryId::for_path(menu.targets.first()?),
            None => self.model.cursor()?,
        };
        self.model.entry(id)
    }
}

impl WorkspaceWindow {
    pub(crate) fn restore_file_tree(&mut self, memories: Vec<TreeMemory>) {
        self.file_manager.tree.model = FileTree::restore(memories);
        self.file_manager.tree.persist = true;
    }

    pub(crate) fn save_file_tree_memory(&mut self) {
        if self.file_manager.tree.persist && self.file_manager.tree.drag.temporary.is_empty() {
            let offset = self.file_manager.tree.list.logical_scroll_top();
            self.file_manager
                .tree
                .model
                .capture_viewport(offset.item_ix + 1, f32::from(offset.offset_in_item));
            crate::settings::save_file_tree_state(self.file_manager.tree.model.memories());
        }
    }

    pub(super) fn sync_file_tree_document(&mut self, cx: &mut Context<Self>) -> bool {
        let document = self.current_document_path(cx).map(Path::to_path_buf);
        // Follow document changes once; manual root navigation must survive
        // redraws and sidebar toggles while the same document stays active.
        if self.file_manager.tree.observed_document == document {
            return false;
        }
        self.file_manager.tree.observed_document = document.clone();
        let Some(document) = document else {
            return false;
        };
        let changes_root = !self
            .file_manager
            .tree
            .model
            .root()
            .is_some_and(|root| document.starts_with(root));
        if changes_root {
            let Some(parent) = document.parent() else {
                return false;
            };
            self.update_file_tree_root(parent.to_path_buf());
        }
        (changes_root || self.file_manager.tree.model.auto_reveal)
            && self.file_manager.tree.model.reveal(&document)
    }

    pub(super) fn rebuild_file_tree(&mut self, cx: &mut Context<Self>) {
        let tree = &mut self.file_manager.tree;
        if tree.rendered_root.as_deref() == tree.model.root() {
            let offset = tree.list.logical_scroll_top();
            tree.model
                .capture_viewport(offset.item_ix + 1, f32::from(offset.offset_in_item));
        }
        tree.model.rebuild(&self.file_manager.directories);
        tree.rendered_root = tree.model.root().map(Path::to_path_buf);
        // Restore before painting. Height hints also let cursor reveal work
        // immediately, while offscreen rows have not been measured yet.
        tree.list.reset_with_uniform_height(
            tree.model.rows().len().saturating_sub(1),
            crate::typography::ui_px(28.),
        );
        if let Some((rank, offset)) = tree.model.viewport() {
            tree.list.scroll_to(ListOffset {
                item_ix: rank.saturating_sub(1),
                offset_in_item: px(offset),
            });
        }
        let directories = tree.model.active_directories();
        self.sync_file_tree_name_row(cx);
        if self.file_manager.sidebar_visible && self.content_route == ContentRoute::Document {
            for directory in directories {
                self.request_file_directory(directory, false, cx);
            }
        }
        self.save_file_tree_memory();
        cx.notify();
    }

    pub(super) fn prepare_file_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.file_manager.sidebar_visible || self.content_route != ContentRoute::Document {
            self.file_manager.tree.pending_open = None;
            return;
        }
        if let Some((path, focus_editor)) = self.file_manager.tree.pending_open.clone() {
            match &self.state {
                super::WorkspaceLoadState::Ready { document }
                    if document.session.read(cx).path() == path =>
                {
                    self.file_manager.tree.pending_open = None;
                    if focus_editor {
                        self.focus_document(cx);
                    }
                }
                super::WorkspaceLoadState::Failed { path: failed, .. } if *failed == path => {
                    self.file_manager.tree.pending_open = None;
                    self.focus_sidebar(cx);
                }
                _ => {}
            }
        }
        if self.file_manager.tree.input.is_none() {
            let input = cx.new(|cx| {
                NativeInput::new(
                    InputConfig {
                        id: "file-tree-type-ahead",
                        font_size: 13.,
                        command: |key, modifiers, _| {
                            modifiers.control
                                || modifiers.platform
                                || matches!(
                                    key,
                                    "up" | "down"
                                        | "left"
                                        | "right"
                                        | "home"
                                        | "end"
                                        | "pageup"
                                        | "pagedown"
                                        | "enter"
                                        | "escape"
                                        | "tab"
                                        | "space"
                                        | "f2"
                                        | "f6"
                                        | "f10"
                                )
                        },
                        ..Default::default()
                    },
                    cx,
                )
            });
            self.file_manager.tree.toolbar = Some(std::array::from_fn(|_| cx.focus_handle()));
            self.file_manager.tree.subscription = Some(cx.subscribe_in(
                &input,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::Changed(text) => this.file_tree_type_ahead(input, text, cx),
                    InputEvent::Command { key, modifiers } => {
                        this.file_tree_input_command(key, *modifiers, window, cx)
                    }
                    InputEvent::MetricsChanged => {}
                },
            ));
            self.file_manager.tree.input = Some(input);
        }
        if self.sync_file_tree_document(cx) {
            self.rebuild_file_tree(cx);
            self.sync_file_directory_watches(cx);
            self.reveal_file_tree_cursor(cx);
        }
        let errors = self
            .file_manager
            .tree
            .model
            .rows()
            .iter()
            .filter(|row| matches!(row.kind, crate::file_manager::TreeRowKind::Error(_)))
            .map(|row| row.parent.clone())
            .collect::<std::collections::HashSet<_>>();
        let tree = &mut self.file_manager.tree;
        let retry_disappeared = tree
            .retry_focus
            .iter()
            .any(|(path, focus)| !errors.contains(path) && focus.is_focused(window));
        tree.retry_focus.retain(|path, _| errors.contains(path));
        for path in errors {
            tree.retry_focus
                .entry(path)
                .or_insert_with(|| cx.focus_handle());
        }
        tree.focus_requested |= retry_disappeared;
        if self.file_manager.tree.model.rows().is_empty()
            && self.file_manager.tree.model.root().is_some()
        {
            self.rebuild_file_tree(cx);
            self.sync_file_directory_watches(cx);
        }
        if std::mem::take(&mut self.file_manager.tree.focus_requested)
            && let Some(input) = &self.file_manager.tree.input
        {
            let focus = input.read(cx).focus.clone();
            window.focus(&focus, cx);
            self.install_sidebar_keymap();
        }
    }

    pub(super) fn reveal_file_tree_cursor(&mut self, cx: &mut Context<Self>) {
        if let Some(rank) = self
            .file_manager
            .tree
            .model
            .cursor()
            .and_then(|id| self.file_manager.tree.model.rank(id))
            && rank > 0
        {
            self.file_manager.tree.list.scroll_to_reveal_item(rank - 1);
            let depth = self.file_manager.tree.model.rows()[rank].depth as f32;
            let scroll = &self.file_manager.tree.horizontal_scroll;
            let mut offset = scroll.offset();
            let start = f32::from(crate::typography::ui_px(depth * 16.));
            let viewport = f32::from(self.file_manager.tree.bounds.get().size.width);
            let left = -f32::from(offset.x);
            if start < left {
                offset.x = px(-start);
            } else if start + f32::from(crate::typography::ui_px(100.)) > left + viewport {
                offset.x =
                    px(-(start + f32::from(crate::typography::ui_px(100.)) - viewport).max(0.));
            }
            scroll.set_offset(offset);
        }
        self.save_file_tree_memory();
        cx.notify();
    }

    pub(crate) fn dispatch_file_tree_command(
        &mut self,
        command: FileTreeCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use FileTreeCommand as Command;
        if command == Command::Focus {
            if self.file_manager.sidebar_focused {
                self.focus_document(cx);
            } else {
                if !self.file_manager.sidebar_visible {
                    self.toggle_sidebar(cx);
                }
                self.focus_sidebar(cx);
                if self.file_manager.tree.model.cursor().is_none() {
                    let path = self.document_path(cx).map(Path::to_path_buf);
                    if let Some(path) = path {
                        self.file_manager.tree.model.reveal(&path);
                    }
                    self.rebuild_file_tree(cx);
                }
            }
            return;
        }
        if !self.file_manager.sidebar_visible || self.content_route != ContentRoute::Document {
            return;
        }
        match command {
            Command::Focus => unreachable!(),
            Command::Next
            | Command::Previous
            | Command::ExtendNext
            | Command::ExtendPrevious
            | Command::First
            | Command::Last
            | Command::PageDown
            | Command::PageUp => {
                let page = (f32::from(self.file_manager.tree.bounds.get().size.height)
                    / f32::from(crate::typography::ui_px(28.)))
                .floor()
                .max(1.) as i64;
                let delta = match command {
                    Command::Next | Command::ExtendNext => 1,
                    Command::Previous | Command::ExtendPrevious => -1,
                    Command::First => i64::MIN,
                    Command::Last => i64::MAX,
                    Command::PageDown => page,
                    _ => -page,
                };
                self.file_manager.tree.model.move_cursor(
                    delta,
                    matches!(command, Command::ExtendNext | Command::ExtendPrevious),
                );
            }
            Command::Expand | Command::Collapse => {
                if self
                    .file_manager
                    .tree
                    .model
                    .move_hierarchy(command == Command::Expand)
                {
                    self.rebuild_file_tree(cx);
                }
            }
            Command::Open => self.open_file_tree_selection(true, cx),
            Command::ToggleSelection => {
                if let Some(id) = self.file_manager.tree.model.cursor() {
                    self.file_manager
                        .tree
                        .model
                        .select(id, TreeSelection::Toggle);
                }
            }
            Command::FocusNext | Command::ReturnToEditor => {
                self.file_manager.tree.menu = None;
                let retry = self
                    .file_manager
                    .tree
                    .model
                    .current()
                    .and_then(|entry| self.file_manager.tree.retry_focus.get(entry.path.as_ref()))
                    .cloned();
                if command == Command::FocusNext
                    && let Some(retry) = retry
                {
                    window.focus(&retry, cx);
                    return;
                }
                self.focus_document(cx);
            }
            Command::FocusToolbar => {
                if let Some(focus) = self
                    .file_manager
                    .tree
                    .toolbar
                    .as_ref()
                    .and_then(|toolbar| toolbar.last())
                {
                    window.focus(focus, cx);
                }
            }
            Command::ContextMenu => self.open_file_tree_menu(MenuKind::Context, None, window, cx),
            Command::NewMenu => self.open_file_tree_menu(MenuKind::New, None, window, cx),
            Command::MoreMenu => self.open_file_tree_menu(MenuKind::More, None, window, cx),
            Command::NewFile | Command::NewFolder | Command::Rename => {
                self.begin_file_tree_name(command, window, cx)
            }
            Command::Copy => self.dired_copy(cx),
            Command::Move => self.dired_move_to(cx),
            Command::Trash => self.dired_trash(window, cx),
            Command::ChangeRoot => self.choose_file_tree_root(cx),
            Command::ParentRoot => {
                if let Some(parent) = self
                    .file_manager
                    .tree
                    .model
                    .parent_root()
                    .map(Path::to_path_buf)
                {
                    let child = self.file_manager.tree.model.root().unwrap().to_path_buf();
                    self.set_file_tree_root(parent, cx);
                    self.file_manager.tree.model.reveal(&child);
                    self.rebuild_file_tree(cx);
                    self.focus_sidebar(cx);
                }
            }
            Command::EnterRoot => {
                let tree = &self.file_manager.tree;
                if let Some(directory) = tree
                    .directory_root_target(tree.menu.as_ref().map(|menu| menu.targets.as_ref()))
                    .map(Path::to_path_buf)
                {
                    self.set_file_tree_root(directory, cx);
                    self.focus_sidebar(cx);
                }
            }
            Command::Reveal => {
                if let Some(path) = self.document_path(cx).map(Path::to_path_buf) {
                    if self.file_manager.tree.model.reveal(&path) {
                        self.rebuild_file_tree(cx);
                    } else {
                        self.choose_file_tree_root_at_document(cx);
                    }
                }
            }
            Command::Refresh => {
                self.file_manager.directory_watch = None;
                for directory in self.file_manager.tree.model.active_directories() {
                    self.request_file_directory(directory, true, cx);
                }
            }
            Command::CollapseAll => {
                self.file_manager.tree.model.collapse_all();
                self.rebuild_file_tree(cx);
            }
            Command::Hidden => {
                self.file_manager.tree.model.show_hidden ^= true;
                self.rebuild_file_tree(cx);
            }
            Command::AutoReveal => {
                self.file_manager.tree.model.auto_reveal ^= true;
                if self.file_manager.tree.model.auto_reveal
                    && let Some(path) = self.document_path(cx).map(Path::to_path_buf)
                    && self.file_manager.tree.model.reveal(&path)
                {
                    self.rebuild_file_tree(cx);
                }
                self.save_file_tree_memory();
            }
            Command::SortName | Command::SortModified => {
                self.file_manager.tree.model.sort.key = if command == Command::SortName {
                    SortKey::Name
                } else {
                    SortKey::Modified
                };
                self.rebuild_file_tree(cx);
            }
            Command::ReverseSort => {
                self.file_manager.tree.model.sort.direction =
                    match self.file_manager.tree.model.sort.direction {
                        SortDirection::Ascending => SortDirection::Descending,
                        SortDirection::Descending => SortDirection::Ascending,
                    };
                self.rebuild_file_tree(cx);
            }
            Command::FoldersFirst => {
                self.file_manager.tree.model.sort.directories_first ^= true;
                self.rebuild_file_tree(cx);
            }
            Command::RevealInFinder => {
                if let Some(path) = self
                    .file_manager
                    .tree
                    .operation_entry()
                    .map(|entry| entry.path.to_path_buf())
                {
                    cx.reveal_path(&path);
                }
            }
        }
        if !matches!(
            command,
            Command::ContextMenu | Command::NewMenu | Command::MoreMenu
        ) {
            self.file_manager.tree.menu = None;
            if command != Command::FocusToolbar
                && self.file_manager.sidebar_focused
                && self.file_manager.tree.edit.is_none()
            {
                self.file_manager.tree.focus_requested = true;
            }
        }
        self.sync_file_directory_watches(cx);
        self.reveal_file_tree_cursor(cx);
    }

    pub(super) fn set_file_tree_root(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        self.update_file_tree_root(directory);
        self.rebuild_file_tree(cx);
        self.sync_file_directory_watches(cx);
    }

    fn update_file_tree_root(&mut self, directory: PathBuf) {
        self.save_file_tree_memory();
        self.file_manager.tree.edit = None;
        self.file_manager.tree.pending_open = None;
        self.file_manager
            .tree
            .model
            .set_root(super::absolute_directory(directory));
        self.file_manager.tree.menu = None;
    }

    fn choose_file_tree_root(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(self.language.text("files.open_folder").into()),
        });
        self.file_manager.task = Some(cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| this.set_file_tree_root(path, cx));
            }
        }));
    }

    fn choose_file_tree_root_at_document(&mut self, cx: &mut Context<Self>) {
        let Some(parent) = self
            .document_path(cx)
            .and_then(Path::parent)
            .map(Path::to_path_buf)
        else {
            return;
        };
        // This entry is explicitly labelled as changing the root in the menu.
        self.set_file_tree_root(parent, cx);
        if let Some(path) = self.document_path(cx).map(Path::to_path_buf) {
            self.file_manager.tree.model.reveal(&path);
            self.rebuild_file_tree(cx);
        }
    }

    fn open_file_tree_selection(&mut self, focus_editor: bool, cx: &mut Context<Self>) {
        let Some(entry) = self.file_manager.tree.operation_entry().cloned() else {
            if self.file_manager.tree.model.root().is_none() {
                self.choose_file_tree_root(cx);
            }
            return;
        };
        if entry.is_directory() {
            self.file_manager.tree.model.toggle(entry.id);
            self.rebuild_file_tree(cx);
        } else if crate::preview::supports_document_path(&entry.path)
            || crate::preview::supports_image_path(&entry.path)
        {
            self.file_manager.tree.pending_open = Some((entry.path.to_path_buf(), focus_editor));
            self.open(entry.path.to_path_buf(), cx);
        } else if focus_editor {
            let url = objc2_foundation::NSURL::fileURLWithPath(
                &objc2_foundation::NSString::from_str(&entry.path.to_string_lossy()),
            );
            if !objc2_app_kit::NSWorkspace::sharedWorkspace().openURL(&url) {
                self.show_dired_operation_error(self.language.text("files.open_failed"), cx);
            }
        }
    }
}
