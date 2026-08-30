use std::sync::Arc;

use crate::{
    command::{
        ArgumentSpec, Availability, BuiltinCommand, BuiltinCommandSpec, CapabilitySet,
        CommandRegistry, CommandRegistryBuilder, CommandRole, PrefixArgument, RedactionPolicy,
        RepeatPolicy, SideEffectClass, UndoPolicy,
    },
    input::{
        BindingBehavior, BindingSpec, ContextRegistryBuilder, ContextSet, KeyboardRouter,
        compile_input_profile,
    },
};

use super::{
    BEGINNING_COMMAND, DIRED_BACK_COMMAND, DIRED_COPY_COMMAND, DIRED_CREATE_DIRECTORY_COMMAND,
    DIRED_CREATE_FILE_COMMAND, DIRED_DELETE_COMMAND, DIRED_EXECUTE_COMMAND, DIRED_FORWARD_COMMAND,
    DIRED_HELP_COMMAND, DIRED_INVERT_COMMAND, DIRED_MARK_COMMAND, DIRED_MOVE_COMMAND,
    DIRED_NEXT_COMMAND, DIRED_OPEN_COMMAND, DIRED_PREVIOUS_COMMAND, DIRED_RENAME_COMMAND,
    DIRED_TRASH_COMMAND, DIRED_UNMARK_ALL_COMMAND, DIRED_UNMARK_COMMAND, DIRED_UP_COMMAND,
    END_COMMAND, EXPORT_DOCUMENT_COMMAND, GLOBAL_VISIBILITY_CYCLE_COMMAND,
    OPEN_DEFAULT_DIRED_COMMAND, OPEN_DOCUMENT_COMMAND, OPEN_FILE_MANAGER_COMMAND,
    QUIT_APPLICATION_COMMAND, RELOAD_DOCUMENT_COMMAND, RETURN_DOCUMENT_COMMAND,
    SAVE_DOCUMENT_AS_COMMAND, SAVE_DOCUMENT_COMMAND, SCROLL_BACKWARD_COMMAND,
    SCROLL_FORWARD_COMMAND, SHOW_HOME_COMMAND, TOGGLE_MINIMAP_COMMAND, TOGGLE_SIDEBAR_COMMAND,
};

#[cfg(test)]
pub(super) fn preview_input() -> (Arc<CommandRegistry>, KeyboardRouter, ContextSet) {
    document_input(crate::app::DocumentMode::Preview)
}

pub(super) fn document_input(
    mode: crate::app::DocumentMode,
) -> (Arc<CommandRegistry>, KeyboardRouter, ContextSet) {
    let mut builder = CommandRegistryBuilder::default();
    builder
        .register_builtin(BuiltinCommandSpec {
            name: SAVE_DOCUMENT_COMMAND.into(),
            aliases: &["save-buffer"],
            title: "Save Document",
            description: "Safely save the current document",
            command: BuiltinCommand::SaveDocument,
            role: CommandRole::Action,
            argument_spec: ArgumentSpec::None,
            repeat: RepeatPolicy::Never,
            undo: UndoPolicy::None,
            availability: Availability::FocusedView,
            side_effect: SideEffectClass::WriteFileSystem,
            required_capabilities: CapabilitySet::WRITE_FILE_SYSTEM,
            redaction: RedactionPolicy::RedactArguments,
        })
        .expect("valid built-in save command");
    builder
        .register_builtin(BuiltinCommandSpec {
            name: SAVE_DOCUMENT_AS_COMMAND.into(),
            aliases: &["write-file"],
            title: "Save Document As",
            description: "Safely save the current document at another path",
            command: BuiltinCommand::SaveDocumentAs,
            role: CommandRole::Action,
            argument_spec: ArgumentSpec::None,
            repeat: RepeatPolicy::Never,
            undo: UndoPolicy::None,
            availability: Availability::FocusedView,
            side_effect: SideEffectClass::WriteFileSystem,
            required_capabilities: CapabilitySet::WRITE_FILE_SYSTEM,
            redaction: RedactionPolicy::RedactArguments,
        })
        .expect("valid built-in save-as command");
    builder
        .register_builtin(BuiltinCommandSpec {
            name: OPEN_DOCUMENT_COMMAND.into(),
            aliases: &["find-file"],
            title: "Open File",
            description: "Open a local Org document",
            command: BuiltinCommand::OpenDocument,
            role: CommandRole::Action,
            argument_spec: ArgumentSpec::None,
            repeat: RepeatPolicy::Never,
            undo: UndoPolicy::None,
            availability: Availability::FocusedView,
            side_effect: SideEffectClass::ReadFileSystem,
            required_capabilities: CapabilitySet::READ_FILE_SYSTEM,
            redaction: RedactionPolicy::RedactArguments,
        })
        .expect("valid built-in open command");
    builder
        .register_builtin(BuiltinCommandSpec {
            name: SHOW_HOME_COMMAND.into(),
            aliases: &["list-buffers"],
            title: "Home",
            description: "Show the document home and recent files",
            command: BuiltinCommand::ShowHome,
            role: CommandRole::Action,
            argument_spec: ArgumentSpec::None,
            repeat: RepeatPolicy::Never,
            undo: UndoPolicy::None,
            availability: Availability::FocusedView,
            side_effect: SideEffectClass::None,
            required_capabilities: CapabilitySet::empty(),
            redaction: RedactionPolicy::None,
        })
        .expect("valid built-in home command");
    builder
        .register_builtin(BuiltinCommandSpec {
            name: RELOAD_DOCUMENT_COMMAND.into(),
            aliases: &["revert-buffer"],
            title: "Reload Document",
            description: "Reload the current document from disk",
            command: BuiltinCommand::ReloadDocument,
            role: CommandRole::Action,
            argument_spec: ArgumentSpec::None,
            repeat: RepeatPolicy::Never,
            undo: UndoPolicy::None,
            availability: Availability::FocusedView,
            side_effect: SideEffectClass::ReadFileSystem,
            required_capabilities: CapabilitySet::READ_FILE_SYSTEM,
            redaction: RedactionPolicy::None,
        })
        .expect("valid built-in reload command");
    builder
        .register_builtin(BuiltinCommandSpec {
            name: EXPORT_DOCUMENT_COMMAND.into(),
            aliases: &["export-document"],
            title: "Export Document",
            description: "Export the current document as PDF, PNG, or SVG",
            command: BuiltinCommand::ExportDocument,
            role: CommandRole::Action,
            argument_spec: ArgumentSpec::None,
            repeat: RepeatPolicy::Never,
            undo: UndoPolicy::None,
            availability: Availability::FocusedView,
            side_effect: SideEffectClass::WriteFileSystem,
            required_capabilities: CapabilitySet::WRITE_FILE_SYSTEM,
            redaction: RedactionPolicy::RedactArguments,
        })
        .expect("valid built-in export command");
    for (name, title, command, argument_spec) in [
        (
            QUIT_APPLICATION_COMMAND,
            "Quit",
            BuiltinCommand::QuitApplication,
            ArgumentSpec::None,
        ),
        (
            SCROLL_FORWARD_COMMAND,
            "Scroll Forward",
            BuiltinCommand::ScrollForward,
            ArgumentSpec::Count,
        ),
        (
            SCROLL_BACKWARD_COMMAND,
            "Scroll Backward",
            BuiltinCommand::ScrollBackward,
            ArgumentSpec::Count,
        ),
        (
            BEGINNING_COMMAND,
            "Beginning of Document",
            BuiltinCommand::BeginningOfDocument,
            ArgumentSpec::None,
        ),
        (
            END_COMMAND,
            "End of Document",
            BuiltinCommand::EndOfDocument,
            ArgumentSpec::None,
        ),
    ] {
        builder
            .register_builtin(BuiltinCommandSpec {
                name: name.into(),
                aliases: &[],
                title,
                description: title,
                command,
                role: CommandRole::Action,
                argument_spec,
                repeat: RepeatPolicy::Repeatable,
                undo: UndoPolicy::None,
                availability: Availability::FocusedView,
                side_effect: SideEffectClass::None,
                required_capabilities: CapabilitySet::empty(),
                redaction: RedactionPolicy::None,
            })
            .expect("valid built-in preview command");
    }
    for (name, title, command, argument_spec) in [
        (
            OPEN_FILE_MANAGER_COMMAND,
            "Open File Manager",
            BuiltinCommand::OpenFileManager,
            ArgumentSpec::None,
        ),
        (
            OPEN_DEFAULT_DIRED_COMMAND,
            "Open Dired",
            BuiltinCommand::OpenDefaultDired,
            ArgumentSpec::None,
        ),
        (
            RETURN_DOCUMENT_COMMAND,
            "Return to Document",
            BuiltinCommand::ReturnToDocument,
            ArgumentSpec::None,
        ),
        (
            TOGGLE_SIDEBAR_COMMAND,
            "Toggle Sidebar",
            BuiltinCommand::ToggleSidebar,
            ArgumentSpec::None,
        ),
        (
            TOGGLE_MINIMAP_COMMAND,
            "Toggle Minimap",
            BuiltinCommand::ToggleMinimap,
            ArgumentSpec::None,
        ),
        (
            GLOBAL_VISIBILITY_CYCLE_COMMAND,
            "Cycle Document Visibility",
            BuiltinCommand::GlobalVisibilityCycle,
            ArgumentSpec::None,
        ),
        (
            DIRED_NEXT_COMMAND,
            "Next Line",
            BuiltinCommand::DiredNext,
            ArgumentSpec::Count,
        ),
        (
            DIRED_PREVIOUS_COMMAND,
            "Previous Line",
            BuiltinCommand::DiredPrevious,
            ArgumentSpec::Count,
        ),
        (
            DIRED_OPEN_COMMAND,
            "Open",
            BuiltinCommand::DiredOpen,
            ArgumentSpec::None,
        ),
        (
            DIRED_UP_COMMAND,
            "Up Directory",
            BuiltinCommand::DiredUp,
            ArgumentSpec::None,
        ),
        (
            DIRED_BACK_COMMAND,
            "History Back",
            BuiltinCommand::DiredBack,
            ArgumentSpec::None,
        ),
        (
            DIRED_FORWARD_COMMAND,
            "History Forward",
            BuiltinCommand::DiredForward,
            ArgumentSpec::None,
        ),
        (
            DIRED_MARK_COMMAND,
            "Mark",
            BuiltinCommand::DiredMark,
            ArgumentSpec::None,
        ),
        (
            DIRED_UNMARK_COMMAND,
            "Unmark",
            BuiltinCommand::DiredUnmark,
            ArgumentSpec::None,
        ),
        (
            DIRED_UNMARK_ALL_COMMAND,
            "Unmark All",
            BuiltinCommand::DiredUnmarkAll,
            ArgumentSpec::None,
        ),
        (
            DIRED_INVERT_COMMAND,
            "Invert Marks",
            BuiltinCommand::DiredInvertMarks,
            ArgumentSpec::None,
        ),
        (
            DIRED_DELETE_COMMAND,
            "Flag Delete",
            BuiltinCommand::DiredFlagDelete,
            ArgumentSpec::None,
        ),
        (
            DIRED_EXECUTE_COMMAND,
            "Execute",
            BuiltinCommand::DiredExecute,
            ArgumentSpec::None,
        ),
        (
            DIRED_HELP_COMMAND,
            "Dired Help",
            BuiltinCommand::DiredHelp,
            ArgumentSpec::None,
        ),
    ] {
        let configuration = command == BuiltinCommand::ToggleMinimap;
        builder
            .register_builtin(BuiltinCommandSpec {
                name: name.into(),
                aliases: &[],
                title,
                description: title,
                command,
                role: CommandRole::Action,
                argument_spec,
                repeat: RepeatPolicy::Repeatable,
                undo: UndoPolicy::None,
                availability: Availability::FocusedView,
                side_effect: if configuration {
                    SideEffectClass::Configuration
                } else {
                    SideEffectClass::None
                },
                required_capabilities: if configuration {
                    CapabilitySet::CONFIGURATION
                } else {
                    CapabilitySet::empty()
                },
                redaction: RedactionPolicy::None,
            })
            .expect("valid built-in file manager command");
    }
    for (name, title, command) in [
        (
            DIRED_CREATE_FILE_COMMAND,
            "Create File",
            BuiltinCommand::DiredCreateFile,
        ),
        (
            DIRED_CREATE_DIRECTORY_COMMAND,
            "Create Directory",
            BuiltinCommand::DiredCreateDirectory,
        ),
        (DIRED_RENAME_COMMAND, "Rename", BuiltinCommand::DiredRename),
        (DIRED_COPY_COMMAND, "Copy", BuiltinCommand::DiredCopy),
        (DIRED_MOVE_COMMAND, "Move", BuiltinCommand::DiredMove),
        (
            DIRED_TRASH_COMMAND,
            "Move to Trash",
            BuiltinCommand::DiredTrash,
        ),
    ] {
        builder
            .register_builtin(BuiltinCommandSpec {
                name: name.into(),
                aliases: &[],
                title,
                description: title,
                command,
                role: CommandRole::Action,
                argument_spec: ArgumentSpec::None,
                repeat: RepeatPolicy::Never,
                // Filesystem undo is not journaled yet; do not advertise a
                // transaction that the executor cannot actually reverse.
                undo: UndoPolicy::None,
                availability: Availability::FocusedView,
                side_effect: SideEffectClass::WriteFileSystem,
                required_capabilities: CapabilitySet::WRITE_FILE_SYSTEM,
                redaction: RedactionPolicy::RedactArguments,
            })
            .expect("valid built-in file operation command");
    }
    let commands = Arc::new(builder.build());
    let contexts = built_in_contexts();
    let (bindings, active_contexts) = match mode {
        crate::app::DocumentMode::Source => (workspace_bindings(), ["workspace", "editor"]),
        crate::app::DocumentMode::Preview => (preview_bindings(), ["workspace", "preview"]),
    };
    let active_context = contexts
        .set(active_contexts)
        .expect("registered built-in contexts");
    let configuration = compile_input_profile(
        1,
        &bindings,
        &commands,
        &contexts,
        &active_contexts,
        &["prompt"],
    )
    .expect("built-in input profile is valid");
    let keyboard = KeyboardRouter::new(
        configuration.generation,
        configuration.interner,
        configuration.grammar,
        configuration.enabled_when,
    );
    (commands, keyboard, active_context)
}

pub(super) fn built_in_contexts() -> crate::input::ContextRegistry {
    let mut builder = ContextRegistryBuilder::default();
    for name in [
        "workspace",
        "preview",
        "prompt",
        "sidebar",
        "dired",
        "editor",
    ] {
        builder
            .register(name)
            .expect("valid unique built-in context");
    }
    builder.build()
}

pub(super) fn preview_bindings() -> Vec<BindingSpec<'static>> {
    let mut bindings = workspace_bindings();
    bindings.extend([
        BindingSpec {
            keys: "g",
            behavior: BindingBehavior::Command(RELOAD_DOCUMENT_COMMAND),
        },
        BindingSpec {
            keys: "q",
            behavior: BindingBehavior::Command(QUIT_APPLICATION_COMMAND),
        },
        BindingSpec {
            keys: "SPC",
            behavior: BindingBehavior::Command(SCROLL_FORWARD_COMMAND),
        },
        BindingSpec {
            keys: "backspace",
            behavior: BindingBehavior::Command(SCROLL_BACKWARD_COMMAND),
        },
        BindingSpec {
            keys: "M-<",
            behavior: BindingBehavior::Command(BEGINNING_COMMAND),
        },
        BindingSpec {
            keys: "M->",
            behavior: BindingBehavior::Command(END_COMMAND),
        },
        BindingSpec {
            keys: "S-tab",
            behavior: BindingBehavior::Command(GLOBAL_VISIBILITY_CYCLE_COMMAND),
        },
    ]);
    bindings
}

pub(super) fn workspace_bindings() -> Vec<BindingSpec<'static>> {
    vec![
        BindingSpec {
            keys: "C-x C-f",
            behavior: BindingBehavior::Command(OPEN_DOCUMENT_COMMAND),
        },
        BindingSpec {
            keys: "C-x C-b",
            behavior: BindingBehavior::Command(SHOW_HOME_COMMAND),
        },
        BindingSpec {
            keys: "C-x C-r",
            behavior: BindingBehavior::Command(RELOAD_DOCUMENT_COMMAND),
        },
        BindingSpec {
            keys: "C-x C-c",
            behavior: BindingBehavior::Command(QUIT_APPLICATION_COMMAND),
        },
        BindingSpec {
            keys: "C-x d",
            behavior: BindingBehavior::Command(OPEN_DEFAULT_DIRED_COMMAND),
        },
        BindingSpec {
            keys: "C-x C-d",
            behavior: BindingBehavior::Command(TOGGLE_SIDEBAR_COMMAND),
        },
    ]
}

pub(super) fn dired_bindings() -> Vec<BindingSpec<'static>> {
    vec![
        BindingSpec {
            keys: "C-x C-b",
            behavior: BindingBehavior::Command(SHOW_HOME_COMMAND),
        },
        BindingSpec {
            keys: "n",
            behavior: BindingBehavior::Command(DIRED_NEXT_COMMAND),
        },
        BindingSpec {
            keys: "j",
            behavior: BindingBehavior::Command(DIRED_NEXT_COMMAND),
        },
        BindingSpec {
            keys: "p",
            behavior: BindingBehavior::Command(DIRED_PREVIOUS_COMMAND),
        },
        BindingSpec {
            keys: "k",
            behavior: BindingBehavior::Command(DIRED_PREVIOUS_COMMAND),
        },
        BindingSpec {
            keys: "RET",
            behavior: BindingBehavior::Command(DIRED_OPEN_COMMAND),
        },
        BindingSpec {
            keys: "l",
            behavior: BindingBehavior::Command(DIRED_OPEN_COMMAND),
        },
        BindingSpec {
            keys: "S-6",
            behavior: BindingBehavior::Command(DIRED_UP_COMMAND),
        },
        BindingSpec {
            keys: "h",
            behavior: BindingBehavior::Command(DIRED_UP_COMMAND),
        },
        BindingSpec {
            keys: "S-h",
            behavior: BindingBehavior::Command(DIRED_BACK_COMMAND),
        },
        BindingSpec {
            keys: "S-l",
            behavior: BindingBehavior::Command(DIRED_FORWARD_COMMAND),
        },
        BindingSpec {
            keys: "g",
            behavior: BindingBehavior::Command(RELOAD_DOCUMENT_COMMAND),
        },
        BindingSpec {
            keys: "m",
            behavior: BindingBehavior::Command(DIRED_MARK_COMMAND),
        },
        BindingSpec {
            keys: "u",
            behavior: BindingBehavior::Command(DIRED_UNMARK_COMMAND),
        },
        BindingSpec {
            keys: "S-u",
            behavior: BindingBehavior::Command(DIRED_UNMARK_ALL_COMMAND),
        },
        BindingSpec {
            keys: "t",
            behavior: BindingBehavior::Command(DIRED_INVERT_COMMAND),
        },
        BindingSpec {
            keys: "d",
            behavior: BindingBehavior::Command(DIRED_DELETE_COMMAND),
        },
        BindingSpec {
            keys: "x",
            behavior: BindingBehavior::Command(DIRED_EXECUTE_COMMAND),
        },
        BindingSpec {
            keys: "S-n",
            behavior: BindingBehavior::Command(DIRED_CREATE_FILE_COMMAND),
        },
        BindingSpec {
            keys: "S-=",
            behavior: BindingBehavior::Command(DIRED_CREATE_DIRECTORY_COMMAND),
        },
        BindingSpec {
            keys: "S-r",
            behavior: BindingBehavior::Command(DIRED_RENAME_COMMAND),
        },
        BindingSpec {
            keys: "S-c",
            behavior: BindingBehavior::Command(DIRED_COPY_COMMAND),
        },
        BindingSpec {
            keys: "S-m",
            behavior: BindingBehavior::Command(DIRED_MOVE_COMMAND),
        },
        BindingSpec {
            keys: "S-d",
            behavior: BindingBehavior::Command(DIRED_TRASH_COMMAND),
        },
        BindingSpec {
            keys: "q",
            behavior: BindingBehavior::Command(RETURN_DOCUMENT_COMMAND),
        },
        BindingSpec {
            keys: "S-/",
            behavior: BindingBehavior::Command(DIRED_HELP_COMMAND),
        },
        BindingSpec {
            keys: "C-x d",
            behavior: BindingBehavior::Command(OPEN_DEFAULT_DIRED_COMMAND),
        },
        BindingSpec {
            keys: "C-x C-d",
            behavior: BindingBehavior::Command(TOGGLE_SIDEBAR_COMMAND),
        },
    ]
}

pub(super) fn dired_command_items(commands: &CommandRegistry) -> Vec<(Arc<str>, Arc<str>)> {
    let mut items: Vec<(crate::command::CommandKey, Vec<&'static str>)> = Vec::new();
    for binding in dired_bindings() {
        let BindingBehavior::Command(name) = binding.behavior else {
            continue;
        };
        let Some(command) = commands.key(name) else {
            continue;
        };
        if let Some((_, keys)) = items.iter_mut().find(|(key, _)| *key == command) {
            keys.push(binding.keys);
        } else {
            items.push((command, vec![binding.keys]));
        }
    }

    let mut result = items
        .into_iter()
        .filter_map(|(key, keys)| {
            let descriptor = commands.descriptor(key)?;
            let keys = keys
                .into_iter()
                .map(display_dired_key)
                .collect::<Vec<_>>()
                .join(" / ");
            let title = if descriptor.name.as_str() == RELOAD_DOCUMENT_COMMAND {
                Arc::from("Refresh Directory")
            } else {
                descriptor.title.clone()
            };
            Some((Arc::from(keys), title))
        })
        .collect::<Vec<_>>();
    result.push((Arc::from("C-g"), Arc::from("Close command list")));
    result
}

pub(super) fn display_dired_key(key: &str) -> &str {
    match key {
        "S-6" => "^",
        "S-/" => "?",
        "S-u" => "U",
        "S-h" => "H",
        "S-l" => "L",
        "S-n" => "N",
        "S-=" => "+",
        "S-r" => "R",
        "S-c" => "C",
        "S-m" => "M",
        "S-d" => "D",
        key => key,
    }
}

pub(super) fn command_count(prefix: PrefixArgument) -> f32 {
    prefix.effective_count().unwrap_or(1).clamp(-1_000, 1_000) as f32
}
