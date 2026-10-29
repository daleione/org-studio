use std::{
    path::{Component, Path},
    sync::Arc,
    time::Duration,
};

use gpui::{AppContext, Context, Entity, KeyDownEvent, Modifiers, Window};

use super::{NameEdit, WorkspaceWindow};
use crate::{
    command::FileTreeCommand,
    components::native_input::{InputConfig, InputEvent, NativeInput},
    file_manager::{EntryId, OperationPlan},
};

impl WorkspaceWindow {
    pub(crate) fn file_tree_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.file_manager.sidebar_focused
            || self.content_route != crate::app::ContentRoute::Document
        {
            return false;
        }
        if self.file_manager.tree.menu.is_some() {
            self.file_tree_menu_key(event, window, cx);
            cx.stop_propagation();
            return true;
        }
        if self
            .file_manager
            .tree
            .edit
            .as_ref()
            .is_some_and(|edit| !edit.input.read(cx).is_composing())
            && (event.keystroke.key == "escape"
                || (event.keystroke.key == "g" && event.keystroke.modifiers.control))
        {
            self.cancel_file_tree_name(cx);
            cx.stop_propagation();
            return true;
        }
        if let Some(edit) = self
            .file_manager
            .tree
            .edit
            .as_ref()
            .filter(|edit| edit.input.read(cx).focus.is_focused(window))
        {
            // The name field consumes its own commands before bubbling here.
            // Unhandled navigation must not move the outline behind the field.
            let key = &event.keystroke.key;
            let printable = (key.chars().count() == 1 || key == "space")
                && !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.platform;
            if !printable && !edit.input.read(cx).is_composing() {
                cx.stop_propagation();
            }
            return true;
        }
        let Some(input) = self
            .file_manager
            .tree
            .input
            .clone()
            .filter(|input| input.read(cx).focus.is_focused(window))
        else {
            return false;
        };
        input.update(cx, |input, cx| input.key_down(event, window, cx));
        true
    }

    pub(super) fn file_tree_input_command(
        &mut self,
        key: &str,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let stroke = crate::keymap::KeyStroke::new(
            key,
            modifiers.control,
            modifiers.alt,
            modifiers.shift,
            modifiers.platform,
        );
        self.route_stroke(stroke, window, cx);
    }

    pub(in crate::app::file_manager) fn cancel_file_tree_name(&mut self, cx: &mut Context<Self>) {
        if !self.file_manager.operation_busy && self.file_manager.tree.edit.take().is_some() {
            self.file_manager.tree.focus_requested = self.file_manager.sidebar_focused;
            cx.notify();
        }
    }

    pub(super) fn file_tree_type_ahead(
        &mut self,
        input: &Entity<NativeInput>,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        if !text.is_empty() {
            let mut characters = text.chars();
            let first = characters.next().unwrap();
            let repeated =
                text.chars().count() > 1 && characters.all(|character| character == first);
            let query = if repeated {
                first.to_string()
            } else {
                text.to_owned()
            };
            if repeated {
                input.update(cx, |input, cx| input.sync(&query, cx));
            }
            self.file_manager.tree.model.type_ahead(&query);
            self.reveal_file_tree_cursor(cx);
        }
        let input = input.clone();
        self.file_manager.tree.type_ahead_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(800))
                .await;
            let _ = this.update(cx, |_, cx| {
                if !input.read(cx).is_composing() {
                    input.update(cx, |input, cx| input.sync("", cx));
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn begin_file_tree_name(
        &mut self,
        command: FileTreeCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dired_operation_available(cx) {
            return;
        }
        let source = if command == FileTreeCommand::Rename {
            let targets = self.file_operation_targets();
            if targets.len() != 1 {
                self.show_dired_operation_error(self.language.text("files.single_selection"), cx);
                return;
            }
            if self.file_manager.tree.model.root() == Some(targets[0].as_ref()) {
                self.show_dired_operation_error(self.language.text("files.root_protected"), cx);
                return;
            }
            Some(targets[0].clone())
        } else {
            None
        };
        let Some(directory) = source
            .as_ref()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .or_else(|| {
                self.file_manager
                    .tree
                    .menu
                    .as_ref()
                    .and_then(|menu| menu.creation_directory.clone())
            })
            .or_else(|| self.file_manager.tree.model.creation_directory())
        else {
            return;
        };
        let folder = command == FileTreeCommand::NewFolder
            || (command == FileTreeCommand::Rename
                && self
                    .file_manager
                    .tree
                    .operation_entry()
                    .is_some_and(|entry| entry.is_directory()));
        let text = source
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                self.language
                    .text(if folder {
                        "files.untitled_folder"
                    } else {
                        "files.untitled_file"
                    })
                    .to_owned()
            });
        let input = cx.new(|cx| {
            NativeInput::new(
                InputConfig {
                    id: "file-tree-entry-name",
                    placeholder: self
                        .language
                        .text(crate::command::file_tree_command(command).label)
                        .into(),
                    font_size: 13.,
                    command: |key, modifiers, _| {
                        matches!(key, "enter" | "escape") || (modifiers.control && key == "g")
                    },
                    ..Default::default()
                },
                cx,
            )
        });
        input.update(cx, |input, cx| {
            input.sync(&text, cx);
            let stem_end = if folder || source.is_none() {
                text.len()
            } else {
                Path::new(&text)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(str::len)
                    .unwrap_or(text.len())
            };
            input.selection = 0..stem_end;
        });
        let subscription = cx.subscribe(&input, |this, _, event, cx| match event {
            InputEvent::Command { key, .. } if key == "enter" => this.commit_file_tree_name(cx),
            InputEvent::Command { .. } => {
                this.cancel_file_tree_name(cx);
            }
            InputEvent::Changed(_) => {
                if let Some(edit) = &mut this.file_manager.tree.edit {
                    edit.error = None;
                }
                cx.notify();
            }
            _ => {}
        });
        self.file_manager.tree.edit = Some(NameEdit {
            input: input.clone(),
            directory: directory.clone(),
            source,
            folder,
            error: None,
            _subscription: subscription,
        });
        let id = EntryId::for_path(&directory);
        if self.file_manager.tree.model.entry(id).is_some()
            && !self.file_manager.tree.model.expanded(&directory)
        {
            self.file_manager.tree.model.toggle(id);
            self.rebuild_file_tree(cx);
        }
        let focus = input.read(cx).focus.clone();
        window.focus(&focus, cx);
        self.file_manager.tree.focus_requested = false;
        self.sync_file_tree_name_row(cx);
        cx.notify();
    }

    pub(super) fn sync_file_tree_name_row(&self, cx: &gpui::App) {
        if let Some(edit) = &self.file_manager.tree.edit {
            let path = edit.source.as_deref().unwrap_or(&edit.directory);
            if let Some(rank) = self
                .file_manager
                .tree
                .model
                .rank(crate::file_manager::EntryId::for_path(path))
                && rank > 0
            {
                self.file_manager
                    .tree
                    .list
                    .splice_focusable(rank - 1..rank, [Some(edit.input.read(cx).focus.clone())]);
            }
        }
    }

    fn commit_file_tree_name(&mut self, cx: &mut Context<Self>) {
        if self.file_manager.operation_busy {
            return;
        }
        let Some(edit) = &self.file_manager.tree.edit else {
            return;
        };
        let name = edit.input.read(cx).text.trim().to_owned();
        let mut components = Path::new(&name).components();
        if name.is_empty()
            || name.chars().any(char::is_control)
            || !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
        {
            if let Some(edit) = &mut self.file_manager.tree.edit {
                edit.error = Some(self.language.text("files.invalid_name").to_owned());
            }
            cx.notify();
            return;
        }
        let target = edit.directory.join(&name);
        let plan = if let Some(source) = &edit.source {
            if source.as_ref() == target {
                self.file_manager.tree.edit = None;
                self.file_manager.tree.focus_requested = true;
                cx.notify();
                return;
            }
            OperationPlan::rename(Arc::clone(source), target)
        } else if edit.folder {
            OperationPlan::create_directory(target)
        } else {
            OperationPlan::create_file(target)
        };
        let input = edit.input.clone();
        input.update(cx, |input, cx| {
            input.enabled = false;
            cx.notify();
        });
        self.execute_dired_operation(plan, cx);
    }
}
