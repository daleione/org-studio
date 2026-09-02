use gpui::{ListOffset, px};
use std::{sync::Arc, time::Duration};

use gpui::{Context, KeyDownEvent, Window};

use crate::document::TextSnapshot;
use crate::{
    app::{ContentRoute, SurfaceAnchor, WorkspaceWindow},
    command::{
        BuiltinCommand, CapabilitySet, CommandDispatcher, CommandImplementation, CommandKey,
        InvocationOrigin, PrefixArgument,
    },
    input::{EmacsOutcome, compile_input_profile},
    keymap::KeyStroke,
    preview::{
        KEY_FEEDBACK_DURATION, PreviewStyleId, built_in_contexts, command_count, dired_bindings,
        preview_bindings, preview_style, workspace_bindings,
    },
};

impl WorkspaceWindow {
    pub(crate) fn dispatch_command(
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

    pub(crate) fn dispatch_command_key(
        &mut self,
        command: CommandKey,
        prefix: PrefixArgument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prepared = CommandDispatcher::prepare_key(
            &self.commands,
            command,
            InvocationOrigin::Keyboard,
            CapabilitySet::READ_FILE_SYSTEM
                .union(CapabilitySet::WRITE_FILE_SYSTEM)
                .union(CapabilitySet::CONFIGURATION),
            prefix,
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

    pub(crate) fn execute_command(
        &mut self,
        implementation: CommandImplementation,
        prefix: PrefixArgument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            implementation,
            CommandImplementation::Builtin(BuiltinCommand::GlobalVisibilityCycle)
        ) && let Some(panel) = self.reading_panel()
        {
            panel.update(cx, |panel, _| panel.reset_cycle_continuation());
        }
        match implementation {
            CommandImplementation::Builtin(BuiltinCommand::OpenDocument) => {
                self.choose_file(window, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::ShowHome) => {
                self.request_home(window, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::ReloadDocument) => {
                if self.content_route == ContentRoute::FileManager
                    || self.file_manager.sidebar_focused()
                {
                    self.reload_file_manager(cx);
                } else {
                    self.reload(window, cx);
                }
            }
            CommandImplementation::Builtin(BuiltinCommand::SaveDocument) => {
                self.save_document(window, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::SaveDocumentAs) => {
                self.save_document_as(window, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::UndoDocument) => {
                if let Some(session) = self.document_session().cloned() {
                    let _ = session.update(cx, |session, cx| session.undo(cx));
                    cx.notify();
                }
            }
            CommandImplementation::Builtin(BuiltinCommand::RedoDocument) => {
                if let Some(session) = self.document_session().cloned() {
                    let _ = session.update(cx, |session, cx| session.redo(cx));
                    cx.notify();
                }
            }
            CommandImplementation::Builtin(BuiltinCommand::ExportDocument) => {
                self.show_export_panel(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::QuitApplication) => {
                self.request_quit(window, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::ScrollForward) => {
                if let Some(panel) = self.reading_panel() {
                    panel.update(cx, |panel, _| {
                        panel.scroll_by(px(640.0 * command_count(prefix)));
                    });
                }
                cx.notify();
            }
            CommandImplementation::Builtin(BuiltinCommand::ScrollBackward) => {
                if let Some(panel) = self.reading_panel() {
                    panel.update(cx, |panel, _| {
                        panel.scroll_by(px(-640.0 * command_count(prefix)));
                    });
                }
                cx.notify();
            }
            CommandImplementation::Builtin(BuiltinCommand::BeginningOfDocument) => {
                if let Some(panel) = self.reading_panel() {
                    panel.update(cx, |panel, _| {
                        panel.scroll_to(ListOffset::default());
                    });
                }
                cx.notify();
            }
            CommandImplementation::Builtin(BuiltinCommand::EndOfDocument) => {
                if let Some(panel) = self.reading_panel() {
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
            CommandImplementation::Builtin(BuiltinCommand::ShowEditor) => self.show_editor(cx),
            CommandImplementation::Builtin(BuiltinCommand::ShowReading) => self.show_reading(cx),
            CommandImplementation::Builtin(BuiltinCommand::ShowSplit) => self.show_split(cx),
            CommandImplementation::Builtin(BuiltinCommand::ToggleSoftWrap) => {
                self.toggle_soft_wrap(cx)
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
                self.dired_open_selected(window, cx)
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

    pub(crate) fn show_editor(&mut self, cx: &mut Context<Self>) {
        self.document_workspace.layout = crate::app::WorkspaceLayout::Single;
        self.set_active_surface(crate::app::PaneSurface::Editor, cx);
    }

    pub(crate) fn show_reading(&mut self, cx: &mut Context<Self>) {
        self.document_workspace.layout = crate::app::WorkspaceLayout::Single;
        self.set_active_surface(crate::app::PaneSurface::Reading, cx);
    }

    pub(crate) fn show_split(&mut self, cx: &mut Context<Self>) {
        self.document_workspace.layout = crate::app::WorkspaceLayout::Split;
        self.cancel_split_resize();
        self.reconcile_visible_editor_panes(cx);
        self.reconcile_visible_reading_panes(cx);
        self.reconcile_derived_preview(cx);
        self.install_document_keymap();
        self.focus_active_surface(cx);
        cx.notify();
    }

    pub(crate) fn toggle_pane_surface(
        &mut self,
        pane: crate::app::PaneSide,
        cx: &mut Context<Self>,
    ) {
        self.document_workspace.active_pane = pane;
        let surface = match self.document_workspace.surface(pane) {
            crate::app::PaneSurface::Editor => crate::app::PaneSurface::Reading,
            crate::app::PaneSurface::Reading => crate::app::PaneSurface::Editor,
        };
        self.set_active_surface(surface, cx);
    }

    pub(crate) fn activate_pane(&mut self, pane: crate::app::PaneSide, cx: &mut Context<Self>) {
        let dismissed_popover = self.status.dismiss_popover();
        if self.document_workspace.active_pane == pane {
            if dismissed_popover {
                cx.notify();
            }
            return;
        }
        let previous = self.document_workspace.active_pane;
        if matches!(
            self.document_workspace.surface(previous),
            crate::app::PaneSurface::Editor
        ) && let Some(editor) = self.editor(previous)
        {
            editor.update(cx, |editor, cx| editor.finish_composition(cx));
        }
        self.document_workspace.active_pane = pane;
        if matches!(
            self.document_workspace.surface(pane),
            crate::app::PaneSurface::Editor
        ) {
            self.ensure_editor_for(pane, cx);
        }
        self.install_document_keymap();
        self.focus_active_surface(cx);
        cx.notify();
    }

    pub(crate) fn set_active_surface(
        &mut self,
        surface: crate::app::PaneSurface,
        cx: &mut Context<Self>,
    ) {
        let pane = self.document_workspace.active_pane;
        let previous_surface = self.document_workspace.surface(pane);
        if matches!(surface, crate::app::PaneSurface::Reading)
            && let Some(editor) = self.editor(pane)
        {
            editor.update(cx, |editor, cx| editor.finish_composition(cx));
        }
        if previous_surface != surface {
            *self.pending_surface_anchors.get_mut(pane) =
                self.surface_top_source_anchor(pane, previous_surface, cx);
        }
        self.document_workspace.set_surface(pane, surface);
        match surface {
            crate::app::PaneSurface::Editor => self.ensure_editor_for(pane, cx),
            crate::app::PaneSurface::Reading => {
                self.reconcile_visible_reading_panes(cx);
            }
        }
        self.apply_pending_surface_anchor(pane, cx);
        self.cancel_split_resize();
        self.reconcile_derived_preview(cx);
        self.install_document_keymap();
        self.focus_active_surface(cx);
        cx.notify();
    }

    fn surface_top_source_anchor(
        &self,
        pane: crate::app::PaneSide,
        surface: crate::app::PaneSurface,
        cx: &gpui::App,
    ) -> Option<SurfaceAnchor> {
        match surface {
            crate::app::PaneSurface::Editor => self.editor(pane).and_then(|editor| {
                editor.read_with(cx, |editor, cx| {
                    let snapshot = editor.snapshot(cx);
                    let source = editor.top_source_anchor(&snapshot).0;
                    let line = snapshot.line_index_at(source).ok()?;
                    let range = snapshot.line_range(line).ok()?;
                    Some(SurfaceAnchor {
                        document_id: snapshot.document_id(),
                        source: snapshot.revision_range(range),
                    })
                })
            }),
            crate::app::PaneSurface::Reading => self.reading_panel_for(pane).and_then(|panel| {
                let panel = panel.read(cx);
                Some(SurfaceAnchor {
                    document_id: panel.document().document_id,
                    source: panel.top_source_revision_range()?,
                })
            }),
        }
    }

    pub(crate) fn apply_pending_surface_anchor(
        &mut self,
        pane: crate::app::PaneSide,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(anchor) = *self.pending_surface_anchors.get(pane) else {
            return false;
        };
        let Some(session) = self.document_session().cloned() else {
            return false;
        };
        let mapped = {
            let session = session.read(cx);
            if session.id() != anchor.document_id {
                *self.pending_surface_anchors.get_mut(pane) = None;
                return false;
            }
            session.map_range_to_current(anchor.source)
        };
        let Ok(mapped) = mapped else {
            *self.pending_surface_anchors.get_mut(pane) = None;
            return false;
        };
        *self.pending_surface_anchors.get_mut(pane) = Some(SurfaceAnchor {
            document_id: anchor.document_id,
            source: mapped,
        });
        let source = mapped.range.start;
        let applied = match self.document_workspace.surface(pane) {
            crate::app::PaneSurface::Editor => self.editor(pane).is_some_and(|editor| {
                editor.update(cx, |editor, cx| editor.scroll_to_source_offset(source, cx))
            }),
            crate::app::PaneSurface::Reading => {
                if !self.latest_preview_is_current(cx) {
                    return false;
                }
                self.reading_panel_for(pane).is_some_and(|panel| {
                    panel.update(cx, |panel, _| panel.scroll_to_source_offset(source))
                })
            }
        };
        if applied {
            *self.pending_surface_anchors.get_mut(pane) = None;
        }
        applied
    }

    pub(crate) fn focus_active_surface(&mut self, cx: &mut Context<Self>) {
        match self.document_workspace.active_surface() {
            crate::app::PaneSurface::Editor => {
                if let Some(editor) = self.editor(self.document_workspace.active_pane) {
                    editor.update(cx, |editor, cx| editor.request_focus(cx));
                }
            }
            crate::app::PaneSurface::Reading => {
                self.focus_workspace_on_render = true;
            }
        }
    }

    pub(crate) fn toggle_soft_wrap(&mut self, cx: &mut Context<Self>) {
        self.soft_wrap = !self.soft_wrap;
        if let Some(ready) = self.state.ready() {
            for editor in [&ready.editors.left, &ready.editors.right]
                .into_iter()
                .flatten()
            {
                editor.update(cx, |editor, cx| editor.set_soft_wrap(self.soft_wrap, cx));
            }
        }
        self.save_preview_settings();
        cx.notify();
    }

    pub(crate) fn key_down(
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
            && (self.status.dismiss_popover()
                || self.file_manager.dismiss_context_menu()
                || self.cancel_minimap_interaction(cx)
                || self.cancel_sidebar_resize()
                || self.cancel_split_resize())
        {
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if event.keystroke.key == "escape"
            && matches!(
                self.document_workspace.active_surface(),
                crate::app::PaneSurface::Reading
            )
        {
            cx.stop_propagation();
            let other = self.document_workspace.active_pane.other();
            if self.document_workspace.is_split()
                && matches!(
                    self.document_workspace.surface(other),
                    crate::app::PaneSurface::Editor
                )
            {
                self.activate_pane(other, cx);
            } else {
                self.set_active_surface(crate::app::PaneSurface::Editor, cx);
            }
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

    pub(crate) fn install_document_keymap(&mut self) {
        if matches!(
            self.document_workspace.active_surface(),
            crate::app::PaneSurface::Reading
        ) {
            self.install_route_keymap(false, false);
        } else {
            self.install_source_keymap();
        }
    }

    fn install_source_keymap(&mut self) {
        let contexts = built_in_contexts();
        let generation = self.keyboard.generation().wrapping_add(1);
        let active_contexts = vec!["workspace", "editor"];
        let Ok(configuration) = compile_input_profile(
            generation,
            &workspace_bindings(),
            &self.commands,
            &contexts,
            &active_contexts,
            &["prompt"],
        ) else {
            return;
        };
        self.key_context = contexts
            .set(active_contexts)
            .expect("registered source editor contexts");
        self.keyboard.replace_configuration(configuration);
    }

    pub(crate) fn install_dired_keymap(&mut self) {
        self.install_route_keymap(true, false);
    }

    pub(crate) fn install_sidebar_keymap(&mut self) {
        self.install_route_keymap(true, true);
    }

    pub(crate) fn install_route_keymap(&mut self, dired: bool, sidebar: bool) {
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

    pub(crate) fn schedule_which_key(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn cancel_which_key(&mut self, cx: &mut Context<Self>) {
        self.which_key_request = self.which_key_request.wrapping_add(1);
        self.which_key_task = None;
        if !self.which_key_items.is_empty() || self.file_manager.help_visible() {
            self.which_key_items = Arc::new(Vec::new());
            self.file_manager.set_help_visible(false);
            cx.notify();
        }
    }

    pub(crate) fn schedule_key_feedback_clear(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn cancel_key_feedback(&mut self) {
        self.key_feedback_request = self.key_feedback_request.wrapping_add(1);
        self.key_feedback_task = None;
    }

    pub(crate) fn select_reading_style(
        &mut self,
        style_id: PreviewStyleId,
        cx: &mut Context<Self>,
    ) {
        let changed = self.apply_reading_style(style_id, cx);
        self.status.dismiss_popover();
        if changed {
            self.save_preview_settings();
        }
        cx.notify();
    }

    pub(crate) fn apply_reading_style(
        &mut self,
        style_id: PreviewStyleId,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.reading_style == style_id {
            return false;
        }
        let previous = *preview_style(self.reading_style);
        let next = *preview_style(style_id);
        self.reading_style = style_id;
        self.status.clear_layout_cache();
        if let Some(ready) = self.state.ready() {
            for pane in [crate::app::PaneSide::Left, crate::app::PaneSide::Right] {
                if let Some(panel) = ready.readers.get(pane).as_ref() {
                    panel.update(cx, |panel, cx| panel.change_style(previous, next, cx));
                }
            }
        }
        true
    }
}
