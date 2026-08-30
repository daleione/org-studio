use gpui::{ListOffset, px};
use std::{sync::Arc, time::Duration};

use super::{
    BuiltinCommand, CapabilitySet, CommandDispatcher, CommandImplementation, CommandKey,
    ContentRoute, Context, EmacsOutcome, InvocationOrigin, KEY_FEEDBACK_DURATION, KeyDownEvent,
    KeyStroke, PrefixArgument, Window, WorkspaceWindow, built_in_contexts, command_count,
    compile_input_profile, dired_bindings, preview_bindings,
};

impl WorkspaceWindow {
    pub(in crate::preview) fn dispatch_command(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prepared = CommandDispatcher::prepare(
            &self.commands,
            name,
            InvocationOrigin::PlatformAction,
            CapabilitySet::READ_FILE_SYSTEM
                .union(CapabilitySet::WRITE_FILE_SYSTEM)
                .union(CapabilitySet::CONFIGURATION),
        );
        let Ok(prepared) = prepared else {
            return;
        };
        self.execute_command(
            prepared.implementation,
            prepared.invocation.prefix,
            window,
            cx,
        );
    }

    pub(in crate::preview) fn dispatch_command_key(
        &mut self,
        command: CommandKey,
        prefix: PrefixArgument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(prepared) = CommandDispatcher::prepare_key(
            &self.commands,
            command,
            InvocationOrigin::Keyboard,
            CapabilitySet::READ_FILE_SYSTEM.union(CapabilitySet::WRITE_FILE_SYSTEM),
            prefix,
        ) else {
            return;
        };
        self.execute_command(
            prepared.implementation,
            prepared.invocation.prefix,
            window,
            cx,
        );
    }

    pub(in crate::preview) fn execute_command(
        &mut self,
        implementation: CommandImplementation,
        prefix: PrefixArgument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            implementation,
            CommandImplementation::Builtin(BuiltinCommand::GlobalVisibilityCycle)
        ) && let Some(panel) = self.preview_panel()
        {
            panel.update(cx, |panel, _| panel.reset_cycle_continuation());
        }
        match implementation {
            CommandImplementation::Builtin(BuiltinCommand::OpenDocument) => self.choose_file(cx),
            CommandImplementation::Builtin(BuiltinCommand::ShowHome) => self.show_home(cx),
            CommandImplementation::Builtin(BuiltinCommand::ReloadDocument) => {
                if self.content_route == ContentRoute::FileManager
                    || self.file_manager.sidebar_focused()
                {
                    self.reload_file_manager(cx);
                } else {
                    self.reload(cx);
                }
            }
            CommandImplementation::Builtin(BuiltinCommand::ExportDocument) => {
                self.show_export_panel(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::QuitApplication) => cx.quit(),
            CommandImplementation::Builtin(BuiltinCommand::ScrollForward) => {
                if let Some(panel) = self.preview_panel() {
                    panel.update(cx, |panel, _| {
                        panel.scroll_by(px(640.0 * command_count(prefix)));
                    });
                }
                cx.notify();
            }
            CommandImplementation::Builtin(BuiltinCommand::ScrollBackward) => {
                if let Some(panel) = self.preview_panel() {
                    panel.update(cx, |panel, _| {
                        panel.scroll_by(px(-640.0 * command_count(prefix)));
                    });
                }
                cx.notify();
            }
            CommandImplementation::Builtin(BuiltinCommand::BeginningOfDocument) => {
                if let Some(panel) = self.preview_panel() {
                    panel.update(cx, |panel, _| {
                        panel.scroll_to(ListOffset::default());
                    });
                }
                cx.notify();
            }
            CommandImplementation::Builtin(BuiltinCommand::EndOfDocument) => {
                if let Some(panel) = self.preview_panel() {
                    panel.update(cx, |panel, _| {
                        panel.scroll_to_end();
                    });
                }
                cx.notify();
            }
            CommandImplementation::Builtin(BuiltinCommand::OpenFileManager) => {
                self.choose_directory(cx);
            }
            CommandImplementation::Builtin(BuiltinCommand::OpenDefaultDired) => {
                self.open_default_dired(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::ReturnToDocument) => {
                self.return_to_document(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::ToggleSidebar) => {
                self.toggle_sidebar(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::ToggleMinimap) => {
                self.toggle_minimap(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::GlobalVisibilityCycle) => {
                self.cycle_global_visibility_animated(window, cx);
                cx.notify();
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredNext) => {
                self.dired_move(command_count(prefix) as i64, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredPrevious) => {
                self.dired_move(-(command_count(prefix) as i64), cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredOpen) => {
                self.dired_open_selected(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredUp) => self.dired_up(cx),
            CommandImplementation::Builtin(BuiltinCommand::DiredBack) => {
                self.dired_history(false, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredForward) => {
                self.dired_history(true, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredMark) => {
                self.dired_mark(crate::file_manager::Mark::Selected, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredUnmark) => self.dired_unmark(cx),
            CommandImplementation::Builtin(BuiltinCommand::DiredUnmarkAll) => {
                self.dired_unmark_all(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredInvertMarks) => {
                self.dired_invert_marks(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredFlagDelete) => {
                self.dired_mark(crate::file_manager::Mark::Delete, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredExecute) => {
                self.dired_prepare_execute(window, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredCreateFile) => {
                self.dired_create_file(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredCreateDirectory) => {
                self.dired_create_directory(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredRename) => self.dired_rename(cx),
            CommandImplementation::Builtin(BuiltinCommand::DiredCopy) => self.dired_copy(cx),
            CommandImplementation::Builtin(BuiltinCommand::DiredMove) => self.dired_move_to(cx),
            CommandImplementation::Builtin(BuiltinCommand::DiredTrash) => {
                self.dired_trash(window, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredHelp) => {
                self.show_dired_shortcuts(cx)
            }
        }
    }

    pub(in crate::preview) fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.export.is_open() {
            cx.stop_propagation();
            if event.keystroke.key == "escape" {
                self.close_export_panel(cx);
            }
            return;
        }
        if event.keystroke.key == "escape"
            && (self.file_manager.dismiss_context_menu()
                || self.cancel_minimap_interaction(cx)
                || self.cancel_sidebar_resize())
        {
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let modifiers = event.keystroke.modifiers;
        let stroke = KeyStroke::new(
            event.keystroke.key.as_str(),
            modifiers.control,
            modifiers.alt,
            modifiers.shift,
            modifiers.platform,
        );
        let previous_status = self.keyboard.status().map(str::to_owned);
        let outcome = self.keyboard.route(stroke, self.key_context);
        let has_feedback = self.keyboard.status().is_some();
        if previous_status.as_deref() != self.keyboard.status() {
            cx.notify();
        }
        if matches!(outcome, EmacsOutcome::Pending) {
            self.cancel_key_feedback();
            self.schedule_which_key(cx);
        } else {
            self.cancel_which_key(cx);
            if has_feedback {
                self.schedule_key_feedback_clear(cx);
            } else {
                self.cancel_key_feedback();
            }
        }
        match outcome {
            EmacsOutcome::Command { command, prefix } => {
                cx.stop_propagation();
                self.dispatch_command_key(command, prefix, window, cx);
            }
            EmacsOutcome::Pending | EmacsOutcome::Disabled | EmacsOutcome::Cancelled => {
                cx.stop_propagation();
            }
            EmacsOutcome::Undefined if has_feedback => cx.stop_propagation(),
            EmacsOutcome::PassThrough | EmacsOutcome::Undefined => {}
        }
    }

    pub(in crate::preview) fn install_preview_keymap(&mut self) {
        self.install_route_keymap(false, false);
    }

    pub(in crate::preview) fn install_dired_keymap(&mut self) {
        self.install_route_keymap(true, false);
    }

    pub(in crate::preview) fn install_sidebar_keymap(&mut self) {
        self.install_route_keymap(true, true);
    }

    pub(in crate::preview) fn install_route_keymap(&mut self, dired: bool, sidebar: bool) {
        let contexts = built_in_contexts();
        let generation = self.keyboard.generation().wrapping_add(1);
        let bindings = if dired {
            dired_bindings()
        } else {
            preview_bindings()
        };
        let route_context = if dired { "dired" } else { "preview" };
        let active_contexts = if sidebar {
            vec!["workspace", "dired", "sidebar"]
        } else {
            vec!["workspace", route_context]
        };
        let Ok(configuration) = compile_input_profile(
            generation,
            &bindings,
            &self.commands,
            &contexts,
            &active_contexts,
            &["prompt"],
        ) else {
            return;
        };
        self.key_context = contexts
            .set(active_contexts)
            .expect("registered route contexts");
        self.keyboard.replace_configuration(configuration);
    }

    pub(in crate::preview) fn schedule_which_key(&mut self, cx: &mut Context<Self>) {
        self.which_key_request = self.which_key_request.wrapping_add(1);
        let request = self.which_key_request;
        self.file_manager.set_help_visible(false);
        self.which_key_items = Arc::new(Vec::new());
        let delay = cx.background_executor().timer(Duration::from_millis(400));
        self.which_key_task = Some(cx.spawn(async move |this, cx| {
            delay.await;
            let _ = this.update(cx, |this, cx| {
                if this.which_key_request != request || this.keyboard.status().is_none() {
                    return;
                }
                let items = this
                    .keyboard
                    .which_key_candidates()
                    .into_iter()
                    .take(24)
                    .map(|candidate| {
                        let title: Arc<str> = if candidate.disabled {
                            Arc::from("Disabled")
                        } else if let Some(command) = candidate.command {
                            this.commands
                                .descriptor(command)
                                .map(|descriptor| descriptor.title.clone())
                                .unwrap_or_else(|| Arc::from("Unknown command"))
                        } else if candidate.is_prefix {
                            Arc::from("Prefix")
                        } else {
                            Arc::from("Pass through")
                        };
                        (candidate.key, title)
                    })
                    .collect();
                this.which_key_items = Arc::new(items);
                cx.notify();
            });
        }));
    }

    pub(in crate::preview) fn cancel_which_key(&mut self, cx: &mut Context<Self>) {
        self.which_key_request = self.which_key_request.wrapping_add(1);
        self.which_key_task = None;
        if !self.which_key_items.is_empty() || self.file_manager.help_visible() {
            self.which_key_items = Arc::new(Vec::new());
            self.file_manager.set_help_visible(false);
            cx.notify();
        }
    }

    pub(in crate::preview) fn schedule_key_feedback_clear(&mut self, cx: &mut Context<Self>) {
        self.key_feedback_request = self.key_feedback_request.wrapping_add(1);
        let request = self.key_feedback_request;
        let delay = cx.background_executor().timer(KEY_FEEDBACK_DURATION);
        self.key_feedback_task = Some(cx.spawn(async move |this, cx| {
            delay.await;
            let _ = this.update(cx, |this, cx| {
                if this.key_feedback_request != request {
                    return;
                }
                this.key_feedback_task = None;
                if this.keyboard.dismiss_status() {
                    cx.notify();
                }
            });
        }));
    }

    pub(in crate::preview) fn cancel_key_feedback(&mut self) {
        self.key_feedback_request = self.key_feedback_request.wrapping_add(1);
        self.key_feedback_task = None;
    }
}
