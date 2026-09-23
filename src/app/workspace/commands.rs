use gpui::{ListOffset, px};

use gpui::{ClipboardItem, Context, KeyDownEvent, Window};

use crate::document::TextSnapshot;
use crate::{
    app::{ContentRoute, KeyFocusRestore, PaneSide, SurfaceAnchor, WorkspaceWindow},
    command::{
        BuiltinCommand, CapabilitySet, CommandDispatcher, CommandImplementation, CommandKey,
        InvocationOrigin, PrefixArgument,
    },
    input::{EmacsOutcome, compile_input_profile},
    keymap::KeyStroke,
    preview::{
        KEY_FEEDBACK_DURATION, PreviewStyleId, built_in_contexts, command_count, dired_bindings,
        preview_bindings, preview_style, source_bindings,
    },
};

impl WorkspaceWindow {
    pub fn intercept_fullscreen_escape(
        event: &gpui::KeystrokeEvent,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        let modifiers = event.keystroke.modifiers;
        if !window.is_fullscreen()
            || event.keystroke.key != "escape"
            || modifiers.control
            || modifiers.alt
            || modifiers.shift
            || modifiers.platform
        {
            return;
        }
        // Transient inputs must receive Escape themselves, including IME cancellation.
        // Only ordinary editing uses this interceptor to bypass the Escape/Meta prefix.
        if window.root::<Self>().flatten().is_some_and(|workspace| {
            let workspace = workspace.read(cx);
            workspace.search_is_open()
                || workspace.command_line_is_open()
                || workspace
                    .editor(workspace.document_workspace.active_pane)
                    .is_some_and(|editor| editor.read(cx).table_menu_is_open())
                || workspace.buffers.panel.is_some()
                || workspace.keyboard.pending_keys().is_some()
        }) {
            return;
        }
        window.toggle_fullscreen();
        cx.stop_propagation();
    }

    pub(crate) fn dispatch_command(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.keyboard.pending_keys().is_some() {
            self.cancel_prefix_input(window, cx);
        }
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

    pub(crate) fn open_agenda(&mut self, cx: &mut Context<Self>) {
        self.agenda.text_query.hide();
        self.agenda.text_view.hide();
        self.content_route = ContentRoute::Agenda;
        self.agenda.requery();
        self.focus_workspace_on_render = true;
        cx.notify();
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
            CommandImplementation::Builtin(BuiltinCommand::ExecuteCommand) => {
                self.open_command_line(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::AlignTables) => {
                self.start_table_alignment(crate::command::TableScope::Current, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::EditTable(edit)) => {
                if matches!(edit, crate::command::TableEdit::Sort { .. }) {
                    self.prompt_command("table-sort ", cx);
                    return;
                }
                let edit = match (edit, prefix != PrefixArgument::None) {
                    (crate::command::TableEdit::InsertRow, true) => {
                        crate::command::TableEdit::InsertRowBelow
                    }
                    (crate::command::TableEdit::InsertHline, true) => {
                        crate::command::TableEdit::InsertHlineAbove
                    }
                    _ => edit,
                };
                if self.document_workspace.active_surface() == crate::app::PaneSurface::Editor
                    && let Some(editor) = self.editor(self.document_workspace.active_pane)
                {
                    editor.update(cx, |editor, cx| {
                        if let Err(error) = editor.edit_table_at_selection(edit, cx) {
                            editor.show_command_feedback(error, cx);
                        }
                    });
                }
            }
            CommandImplementation::Builtin(BuiltinCommand::GotoLine) => self.prompt_goto_line(cx),
            CommandImplementation::Builtin(BuiltinCommand::FindDocument) => {
                self.open_search(false, false, false, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::IsearchForward) => {
                self.open_search(true, false, false, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::IsearchBackward) => {
                self.open_search(true, true, false, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::QueryReplace) => {
                self.open_search(false, false, true, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::OpenDocument) => {
                self.open_buffer_picker(crate::app::buffers::PickerIntent::File, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::ShowHome) => {
                self.request_home(window, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::OpenAgenda) => self.open_agenda(cx),
            CommandImplementation::Builtin(BuiltinCommand::OpenAgendaText) => {
                if !self.agenda_text_open {
                    self.agenda_text_return = Some(self.content_route);
                    self.agenda_text_open = true;
                    self.agenda.text_view.reopen();
                    self.agenda
                        .query_runtime
                        .reopen(&mut self.agenda.text_query);
                } else {
                    self.agenda.text_view.mount();
                }
                self.content_route = ContentRoute::AgendaText;
                self.agenda.state.projection = crate::app::agenda::state::AgendaProjection::Source;
                if self.agenda.text_query.resume() {
                    self.agenda.requery_independent_text();
                }
                self.sync_agenda_text_buffer(cx);
                if let Some(editor) = &self.agenda.text_editor {
                    editor.update(cx, |editor, cx| editor.request_focus(cx));
                }
                cx.notify();
            }
            CommandImplementation::Builtin(BuiltinCommand::ReloadDocument) => {
                if self.refresh_generated_result(cx) {
                    return;
                }
                if self.content_route == ContentRoute::FileManager
                    || self.file_manager.sidebar_focused()
                {
                    self.reload_file_manager(cx);
                } else {
                    self.reload(window, cx);
                }
            }
            CommandImplementation::Builtin(BuiltinCommand::SaveDocument) => {
                if self.generated_command_disposition(crate::editor::GeneratedCommand::Save)
                    == crate::editor::CommandDisposition::Disabled
                {
                    return;
                }
                self.save_document(window, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::SwitchBuffer) => {
                self.open_buffer_picker(crate::app::buffers::PickerIntent::Switch, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::NavigateBack) => {
                self.navigate_history(false, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::NavigateForward) => {
                self.navigate_history(true, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::CloseBuffer) => {
                self.open_buffer_picker(crate::app::buffers::PickerIntent::Close, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::SaveBuffers) => {
                self.begin_buffer_review(crate::app::buffers::ReviewKind::Save, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::NextBuffer) => {
                self.cycle_buffer(true, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::PreviousBuffer) => {
                self.cycle_buffer(false, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::SaveDocumentAs) => {
                if self.generated_command_disposition(crate::editor::GeneratedCommand::Save)
                    == crate::editor::CommandDisposition::Disabled
                {
                    return;
                }
                self.save_document_as(window, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::OrgContextCommand) => {
                self.execute_org_context_command(window, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::ExecuteSourceBlock) => {
                self.execute_source_block(window, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::OpenLinkAt) => {
                // Resolve via the active pane: a pending `C-c` prefix moves focus
                // to the workspace root, so a focus-based lookup would drop the
                // command. `execute_source_block` takes the same route.
                if let Some(editor) = self.editor(self.document_workspace.active_pane) {
                    editor.update(cx, |editor, cx| editor.open_link_at_caret(cx));
                }
            }
            CommandImplementation::Builtin(BuiltinCommand::ToggleInlineImagePreviews) => {
                window.dispatch_action(Box::new(crate::editor::ToggleInlineImagePreviews), cx);
            }
            CommandImplementation::Builtin(BuiltinCommand::UndoDocument) => {
                if self.generated_command_disposition(crate::editor::GeneratedCommand::Undo)
                    == crate::editor::CommandDisposition::Disabled
                {
                    return;
                }
                if let Some(session) = self.document_session().cloned() {
                    let _ = session.update(cx, |session, cx| session.undo(cx));
                    cx.notify();
                }
            }
            CommandImplementation::Builtin(BuiltinCommand::RedoDocument) => {
                if self.generated_command_disposition(crate::editor::GeneratedCommand::Redo)
                    == crate::editor::CommandDisposition::Disabled
                {
                    return;
                }
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
                if self.content_route == ContentRoute::AgendaText {
                    self.close_agenda_text(cx)
                } else {
                    self.return_to_document(cx)
                }
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
            CommandImplementation::Builtin(BuiltinCommand::IncreaseContentFontSize) => {
                self.increase_content_font_size(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DecreaseContentFontSize) => {
                self.decrease_content_font_size(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::ResetContentFontSize) => {
                self.reset_content_font_size(cx)
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

    pub(crate) fn show_editor(&mut self, cx: &mut Context<Self>) {
        self.document_workspace.layout = crate::app::WorkspaceLayout::Single;
        self.set_active_surface(crate::app::PaneSurface::Editor, cx);
    }

    pub(crate) fn show_reading(&mut self, cx: &mut Context<Self>) {
        if self
            .current_document_path(cx)
            .is_some_and(crate::preview::is_editor_only_document)
        {
            self.show_echo_message(
                crate::app::echo_area::EchoMessage::warning(self.buffer_text(
                    "文本和代码文件仅在编辑区显示",
                    "Text and source files are shown in the editor",
                )),
                cx,
            );
            return;
        }
        self.document_workspace.layout = crate::app::WorkspaceLayout::Single;
        self.set_active_surface(crate::app::PaneSurface::Reading, cx);
    }

    pub(crate) fn show_split(&mut self, cx: &mut Context<Self>) {
        self.end_prefix(cx);
        if let Some((source, target)) = self.document_workspace.enter_split() {
            let inherited = *self.content_font_sizes.get(source);
            *self.content_font_sizes.get_mut(target) = inherited;
        }
        self.cancel_split_resize();
        self.reconcile_visible_editor_panes(cx);
        self.reconcile_visible_reading_panes(cx);
        self.reconcile_derived_preview(cx);
        self.install_document_keymap();
        self.focus_active_surface(cx);
        cx.notify();
    }

    pub(crate) fn increase_content_font_size(&mut self, cx: &mut Context<Self>) {
        if !self.content_font_size_command_available() {
            return;
        }
        let pane = self.document_workspace.active_pane;
        let next = self.content_font_sizes.get(pane).increase();
        self.set_content_font_size(pane, next, cx);
    }

    pub(crate) fn decrease_content_font_size(&mut self, cx: &mut Context<Self>) {
        if !self.content_font_size_command_available() {
            return;
        }
        let pane = self.document_workspace.active_pane;
        let next = self.content_font_sizes.get(pane).decrease();
        self.set_content_font_size(pane, next, cx);
    }

    pub(crate) fn reset_content_font_size(&mut self, cx: &mut Context<Self>) {
        if !self.content_font_size_command_available() {
            return;
        }
        self.set_content_font_size(
            self.document_workspace.active_pane,
            crate::typography::ContentFontSize::reset(),
            cx,
        );
    }

    pub(crate) fn content_font_size_command_available(&self) -> bool {
        matches!(self.content_route, crate::app::ContentRoute::Document)
            && self.state.ready().is_some()
            && self.export.panel().is_none()
    }

    fn set_content_font_size(
        &mut self,
        pane: crate::app::PaneSide,
        font_size: crate::typography::ContentFontSize,
        cx: &mut Context<Self>,
    ) -> bool {
        if *self.content_font_sizes.get(pane) == font_size {
            return false;
        }
        *self.content_font_sizes.get_mut(pane) = font_size;
        let editor = self.editor(pane);
        let reader = self.reading_panel_for(pane);
        if let Some(editor) = editor {
            editor.update(cx, |editor, cx| {
                editor.set_content_font_size(font_size, cx);
            });
        }
        if let Some(reader) = reader {
            reader.update(cx, |reader, _| {
                reader.set_content_font_size(font_size);
            });
        }
        cx.notify();
        true
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
        self.close_command_line(cx);
        self.close_search(false, cx);
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
        if matches!(surface, crate::app::PaneSurface::Reading)
            && self
                .current_document_path(cx)
                .is_some_and(crate::preview::is_editor_only_document)
        {
            self.show_echo_message(
                crate::app::echo_area::EchoMessage::warning(self.buffer_text(
                    "文本和代码文件仅在编辑区显示",
                    "Text and source files are shown in the editor",
                )),
                cx,
            );
            return;
        }
        self.end_prefix(cx);
        self.close_command_line(cx);
        self.close_search(false, cx);
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

    /// A transient input owns focus even when the previous overlay requested editor focus.
    pub(crate) fn cancel_pending_document_focus(&mut self, cx: &mut Context<Self>) {
        self.focus_workspace_on_render = false;
        self.key_focus_restore = None;
        for pane in [PaneSide::Left, PaneSide::Right] {
            if let Some(editor) = self.editor(pane) {
                editor.update(cx, |editor, _| editor.cancel_pending_focus());
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
        if matches!(self.content_route, ContentRoute::Agenda)
            && self.keyboard.pending_keys().is_none()
        {
            let key = event.keystroke.key.as_str();
            if event.keystroke.modifiers.platform
                && (key.eq_ignore_ascii_case("k") || key.eq_ignore_ascii_case("f"))
            {
                self.agenda.state.search_expanded = true;
                cx.notify();
                if let Some(input) = &self.agenda.search_input {
                    let focus = input.read(cx).focus.clone();
                    window.focus(&focus, cx);
                }
                cx.stop_propagation();
                return;
            }
            if self
                .agenda
                .search_input
                .as_ref()
                .is_some_and(|input| input.read(cx).focus.is_focused(window))
            {
                return;
            }
            if self.agenda.state.projection == crate::app::agenda::state::AgendaProjection::Source
                && self.agenda.state.overlay == crate::app::agenda::state::AgendaOverlay::None
                && self.agenda.text_editor.as_ref().is_some_and(|editor| {
                    gpui::Focusable::focus_handle(editor.read(cx), cx).is_focused(window)
                })
            {
                // Let the embedded editor own navigation, selection and clipboard keys.
                return;
            }
            if key.eq_ignore_ascii_case("q")
                && self.agenda.state.overlay == crate::app::agenda::state::AgendaOverlay::None
            {
                self.return_to_document(cx);
                cx.stop_propagation();
                return;
            }
            if event.keystroke.modifiers.platform && key.eq_ignore_ascii_case("n") {
                self.dispatch_agenda_intent(crate::app::agenda::UiIntent::OpenCapture, cx);
                cx.stop_propagation();
                return;
            }
            if self.agenda.state.overlay == crate::app::agenda::state::AgendaOverlay::Capture {
                match key {
                    "escape" => {
                        self.agenda.close_top_layer();
                    }
                    "enter" => {
                        self.dispatch_agenda_intent(crate::app::agenda::UiIntent::SubmitCapture, cx)
                    }
                    "backspace" => {
                        self.agenda.state.capture.title.pop();
                    }
                    _ if !event.keystroke.modifiers.platform
                        && !event.keystroke.modifiers.control
                        && !event.keystroke.modifiers.alt =>
                    {
                        if let Some(text) = event.keystroke.key_char.as_deref() {
                            self.agenda.state.capture.title.push_str(text);
                        }
                    }
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
                return;
            }
            if self.agenda.state.overlay == crate::app::agenda::state::AgendaOverlay::Refile {
                match key {
                    "escape" => {
                        self.agenda.close_top_layer();
                    }
                    "enter" => {
                        self.dispatch_agenda_intent(crate::app::agenda::UiIntent::SubmitRefile, cx)
                    }
                    "up" => self
                        .dispatch_agenda_intent(crate::app::agenda::UiIntent::RefileNext(-1), cx),
                    "down" => {
                        self.dispatch_agenda_intent(crate::app::agenda::UiIntent::RefileNext(1), cx)
                    }
                    "backspace" => {
                        self.agenda.state.refile_search.pop();
                        self.agenda.state.refile_selected = 0;
                    }
                    _ if !event.keystroke.modifiers.platform
                        && !event.keystroke.modifiers.control
                        && !event.keystroke.modifiers.alt =>
                    {
                        if let Some(text) = event.keystroke.key_char.as_deref() {
                            self.agenda.state.refile_search.push_str(text);
                            self.agenda.state.refile_selected = 0;
                        }
                    }
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
                return;
            }
            if self.agenda.state.overlay == crate::app::agenda::state::AgendaOverlay::Repeat {
                let action = match key {
                    "1" | "enter" => Some(crate::agenda::RepeatCompletionAction::Occurrence),
                    "2" => Some(crate::agenda::RepeatCompletionAction::Series),
                    "escape" | "3" => Some(crate::agenda::RepeatCompletionAction::Cancel),
                    _ => None,
                };
                if let Some(action) = action {
                    self.dispatch_agenda_intent(
                        crate::app::agenda::UiIntent::ResolveRepeat(action),
                        cx,
                    );
                }
                cx.stop_propagation();
                cx.notify();
                return;
            }
            if key.eq_ignore_ascii_case("c")
                && !event.keystroke.modifiers.platform
                && !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.alt
            {
                if let Some(key) = self.agenda.selected_task_key() {
                    self.dispatch_agenda_intent(crate::app::agenda::UiIntent::ToggleClock(key), cx);
                }
                cx.stop_propagation();
                return;
            }
            if key == "escape" && self.agenda.close_top_layer() {
                cx.stop_propagation();
                cx.notify();
                return;
            }
            if matches!(key, "up" | "k" | "down" | "j") {
                self.agenda
                    .move_selection(if matches!(key, "up" | "k") { -1 } else { 1 });
                cx.stop_propagation();
                cx.notify();
                return;
            }
            if key == "enter"
                && let Some(task) = self.agenda.selected_task()
            {
                self.dispatch_agenda_intent(crate::app::agenda::UiIntent::OpenSource(task), cx);
                cx.stop_propagation();
                return;
            }
        }
        if event.keystroke.key.eq_ignore_ascii_case("c")
            && event.keystroke.modifiers.platform
            && matches!(
                self.document_workspace.active_surface(),
                crate::app::PaneSurface::Reading
            )
            && self.copy_reading_selection(cx)
        {
            cx.stop_propagation();
            return;
        }
        if event.keystroke.key == "escape"
            && (self.status.dismiss_popover()
                || self.file_manager.dismiss_context_menu()
                || self.cancel_minimap_interaction(cx)
                || self.cancel_sidebar_resize()
                || self.cancel_split_resize()
                || self.dismiss_echo_message())
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
        self.route_stroke(stroke, window, cx);
    }

    pub(crate) fn route_stroke(
        &mut self,
        stroke: KeyStroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor_surface = matches!(
            self.document_workspace.active_surface(),
            crate::app::PaneSurface::Editor
        );
        if editor_surface
            && self.keyboard.pending_keys().is_none()
            && stroke.meta()
            && !stroke.control()
            && !stroke.command()
            && matches!(stroke.key(), "up" | "down" | "left" | "right")
            && !self
                .editor(self.document_workspace.active_pane)
                .is_some_and(|editor| editor.read(cx).available_table_edits(cx).is_some())
        {
            // Outside tables, Option-arrows retain the editor's native word navigation.
            return;
        }
        let editor_escape_prefix = editor_surface
            && stroke.key() == "escape"
            && !stroke.control()
            && !stroke.meta()
            && !stroke.shift()
            && !stroke.command();
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
                // Restore after the command ran, so the state check sees the
                // post-command session/route (e.g. Home drops the session).
                self.restore_key_focus_after_command(window, cx);
            }
            EmacsOutcome::Pending if editor_escape_prefix => {}
            EmacsOutcome::Cancelled if editor_surface => {
                cx.stop_propagation();
                self.restore_key_focus(window, cx);
                window.dispatch_action(Box::new(crate::editor::KeyboardQuit), cx);
            }
            EmacsOutcome::Pending => {
                cx.stop_propagation();
                self.move_key_focus_to_command_area(window, cx);
            }
            EmacsOutcome::Disabled | EmacsOutcome::Cancelled => {
                cx.stop_propagation();
                self.restore_key_focus(window, cx);
            }
            EmacsOutcome::Undefined if has_feedback => {
                cx.stop_propagation();
                self.restore_key_focus(window, cx);
            }
            EmacsOutcome::PassThrough | EmacsOutcome::Undefined => {
                self.restore_key_focus(window, cx);
            }
        }
    }

    /// While a key prefix is capturing (for example `C-x`), move focus to the
    /// workspace root so the editor's GPUI bindings (`C-b`, `C-f`, `C-d`, …)
    /// no longer intercept the stroke that completes the chord. The editor is
    /// not editable during the wait, matching the "focus is on the key area"
    /// model.
    fn move_key_focus_to_command_area(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.key_focus_restore.is_some() {
            return;
        }
        let Some(handle) = window.focused(cx) else {
            return;
        };
        self.key_focus_restore = Some(KeyFocusRestore {
            handle,
            session: self.document_session().map(|session| session.entity_id()),
            route: self.content_route,
        });
        let target = self
            .focus_handle
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        window.focus(&target, cx);
    }

    /// Restores the pre-prefix focus, but only when nothing else claimed focus
    /// while the prefix was pending (commands that switch surfaces focus their
    /// own content and must keep it).
    fn restore_key_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(restore) = self.key_focus_restore.take() else {
            return;
        };
        let still_on_workspace_root = window.focused(cx).is_some_and(|current| {
            self.focus_handle
                .as_ref()
                .is_some_and(|root| current == *root)
        });
        if still_on_workspace_root {
            window.focus(&restore.handle, cx);
        }
    }

    /// Command variant of [`Self::restore_key_focus`]: additionally refuses to
    /// restore when the command replaced the document session or switched the
    /// route, since the captured focus then belongs to stale content.
    pub(crate) fn restore_key_focus_after_command(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let unchanged = self.key_focus_restore.as_ref().is_some_and(|restore| {
            self.content_route == restore.route
                && self.document_session().map(|session| session.entity_id()) == restore.session
        });
        if unchanged {
            self.restore_key_focus(window, cx);
        } else {
            self.key_focus_restore = None;
        }
    }

    pub(crate) fn copy_reading_selection(&mut self, cx: &mut Context<Self>) -> bool {
        if !matches!(
            self.document_workspace.active_surface(),
            crate::app::PaneSurface::Reading
        ) {
            return false;
        }
        let Some(panel) = self.reading_panel_for(self.document_workspace.active_pane) else {
            return false;
        };
        let Some(text) = panel.read(cx).selected_text() else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
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
            &source_bindings(),
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

    pub(crate) fn cancel_prefix_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.end_prefix(cx);
        self.restore_key_focus(window, cx);
        cx.notify();
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
