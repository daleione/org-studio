//! Single command and key source for the outline, menus, and command palette.

pub(crate) struct FileTreeCommandSpec {
    pub(crate) command: FileTreeCommand,
    pub(crate) name: &'static str,
    pub(crate) label: &'static str,
    pub(crate) keys: &'static [&'static str],
}

impl FileTreeCommandSpec {
    pub(crate) fn shortcut_label(&self) -> String {
        let Some(stroke) = self
            .keys
            .first()
            .and_then(|key| crate::keymap::KeyStroke::parse(key).ok())
        else {
            return String::new();
        };
        let mut label = String::new();
        for (present, symbol) in [
            (stroke.control(), "⌃"),
            (stroke.meta(), "⌥"),
            (stroke.shift(), "⇧"),
            (stroke.command(), "⌘"),
        ] {
            if present {
                label.push_str(symbol);
            }
        }
        label.push_str(&match stroke.key() {
            "enter" => "↩".into(),
            "backspace" => "⌫".into(),
            "escape" => "⎋".into(),
            "tab" => "⇥".into(),
            "space" => "Space".into(),
            "up" => "↑".into(),
            "down" => "↓".into(),
            "left" => "←".into(),
            "right" => "→".into(),
            key => key.to_uppercase(),
        });
        label
    }
}

macro_rules! commands {
    ($(($variant:ident, $name:literal, $label:literal, [$($key:literal),*])),* $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum FileTreeCommand {
            $($variant,)*
        }

        pub(crate) const FILE_TREE_COMMANDS: &[FileTreeCommandSpec] = &[$(
            FileTreeCommandSpec { command: FileTreeCommand::$variant,
                name: concat!("org-studio.files.", $name), label: concat!("files.", $label), keys: &[$($key),*] },
        )*];
    };
}

commands! {
    (Focus, "focus", "focus", ["cmd-alt-b", "f6", "S-f6"]),
    (Next, "next", "next", ["down", "C-n"]),
    (Previous, "previous", "previous", ["up", "C-p"]),
    (ExtendNext, "extend-next", "extend_next", ["S-down"]),
    (ExtendPrevious, "extend-previous", "extend_previous", ["S-up"]),
    (Expand, "expand", "expand", ["right", "C-f"]),
    (Collapse, "collapse", "collapse", ["left", "C-b"]),
    (First, "first", "first", ["home"]),
    (Last, "last", "last", ["end"]),
    (PageDown, "page-down", "page_down", ["pagedown"]),
    (PageUp, "page-up", "page_up", ["pageup"]),
    (Open, "open", "open", ["RET"]),
    (ToggleSelection, "toggle-selection", "toggle_selection", ["SPC"]),
    (ContextMenu, "context-menu", "context_menu", ["S-f10"]),
    (MoreMenu, "more-menu", "more", []),
    (NewMenu, "new-menu", "new", []),
    (NewFile, "new-file", "new_file", ["cmd-alt-n"]),
    (NewFolder, "new-folder", "new_folder", ["cmd-shift-n"]),
    (Rename, "rename", "rename", ["f2"]),
    (Copy, "copy", "copy", []),
    (Move, "move", "move", []),
    (Trash, "trash", "trash", ["cmd-backspace"]),
    (Hidden, "hidden", "hidden", ["cmd-shift-."]),
    (CollapseAll, "collapse-all", "collapse_all", []),
    (Reveal, "reveal", "reveal", []),
    (ChangeRoot, "change-root", "change_root", []),
    (ParentRoot, "parent-root", "parent_root", ["cmd-up"]),
    (EnterRoot, "enter-root", "enter_root", ["cmd-down"]),
    (Refresh, "refresh", "refresh", []),
    (SortName, "sort-name", "sort_name", []),
    (SortModified, "sort-modified", "sort_modified", []),
    (ReverseSort, "reverse-sort", "reverse_sort", []),
    (FoldersFirst, "folders-first", "folders_first", []),
    (AutoReveal, "auto-reveal", "auto_reveal", []),
    (FocusToolbar, "focus-toolbar", "focus_toolbar", ["S-tab"]),
    (FocusNext, "focus-next", "focus_next", ["tab"]),
    (ReturnToEditor, "return-to-editor", "return_to_editor", ["escape", "C-g"]),
    (RevealInFinder, "reveal-in-finder", "reveal_in_finder", []),
}

pub(crate) fn file_tree_command(command: FileTreeCommand) -> &'static FileTreeCommandSpec {
    FILE_TREE_COMMANDS
        .iter()
        .find(|spec| spec.command == command)
        .expect("registered file tree command")
}
