use super::*;
use crate::document::{LineIndex, TextSnapshot};
use crate::{
    command::{
        BuiltinCommand, CapabilitySet, CommandDispatcher, CommandImplementation, InvocationOrigin,
        PrefixArgument, TableScope,
    },
    document::{DocumentCommand, EditOrigin, EditTransaction, HistoryOutcome},
    editor::TableAlignmentPlan,
};

impl WorkspaceWindow {
    pub(super) fn execute_command_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let no_match = self
            .command_text("没有匹配的命令", "No matching command")
            .to_owned();
        let Some(s) = self.command_line.session.as_mut() else {
            return;
        };
        if !s.accepts_input(cx) {
            return;
        }
        if let Err(usage) = catalog::validate(&s.query) {
            s.error = Some(usage.into());
            cx.notify();
            return;
        }
        let Some(entry) = s.candidates.get(s.selected).cloned() else {
            s.error = Some(no_match);
            cx.notify();
            return;
        };
        let prepared = CommandDispatcher::prepare_key(
            &self.commands,
            entry.key,
            InvocationOrigin::CommandPalette,
            CapabilitySet::READ_FILE_SYSTEM
                .union(CapabilitySet::WRITE_FILE_SYSTEM)
                .union(CapabilitySet::CONFIGURATION),
            PrefixArgument::None,
        );
        let Ok(prepared) = prepared else {
            self.command_failure(
                self.command_text("命令当前不可用", "Command is unavailable"),
                cx,
            );
            return;
        };
        let implementation = prepared.implementation;
        if let CommandImplementation::Builtin(BuiltinCommand::EditTable(edit)) = implementation {
            let edit = if matches!(edit, crate::command::TableEdit::Sort { .. }) {
                match catalog::sort_edit(&entry.input) {
                    Ok(edit) => edit,
                    Err(error) => {
                        self.command_failure(error, cx);
                        return;
                    }
                }
            } else {
                edit
            };
            let pane = self.document_workspace.active_pane;
            let result = self
                .editor(pane)
                .ok_or("No active editor")
                .and_then(|editor| {
                    editor.update(cx, |editor, cx| editor.edit_table_at_selection(edit, cx))
                });
            if let Err(error) = result {
                self.command_failure(error, cx);
                return;
            }
            self.command_line.remember(entry.input);
            self.close_command_line(cx);
            return;
        }
        let target = if implementation == CommandImplementation::Builtin(BuiltinCommand::GotoLine) {
            if entry.input == "goto-line" {
                self.prompt_goto_line(cx);
                return;
            }
            let line = match catalog::line_number(&entry.input) {
                Ok(line) => line,
                Err(usage) => {
                    self.command_failure(usage, cx);
                    return;
                }
            };
            let Some(doc) = self.document_session() else {
                return;
            };
            let snapshot = doc.read(cx).snapshot();
            let Ok(range) = snapshot.line_content_range(LineIndex(line - 1)) else {
                let message = if self.language == crate::i18n::Language::Chinese {
                    format!("行号超出范围：1–{}", snapshot.len_lines())
                } else {
                    format!("Line number out of range: 1–{}", snapshot.len_lines())
                };
                self.command_failure(&message, cx);
                return;
            };
            // Wait for the current preview before mapping a source offset to a rendered row.
            if self.document_workspace.active_surface() == PaneSurface::Reading
                && !self.latest_preview_is_current(cx)
            {
                self.command_line.session.as_mut().unwrap().execute_pending = true;
                return;
            }
            Some(range.start)
        } else {
            None
        };
        self.command_line.remember(entry.input);
        if let Some(target) = target {
            let pane = self.document_workspace.active_pane;
            *self.pending_surface_anchors.get_mut(pane) = None;
            match self.document_workspace.active_surface() {
                PaneSurface::Editor => {
                    if let Some(editor) = self.editor(pane) {
                        editor.update(cx, |editor, cx| editor.jump_to_source_offset(target, cx));
                    }
                }
                PaneSurface::Reading => {
                    if let Some(panel) = self.reading_panel_for(pane) {
                        panel.update(cx, |panel, cx| {
                            panel.reveal_source_offset(target);
                            cx.notify();
                        });
                    }
                }
            }
            self.close_command_line(cx);
        } else if let (CommandImplementation::Builtin(BuiltinCommand::AlignTables), Some(scope)) =
            (implementation, entry.scope)
        {
            self.start_table_alignment(scope, cx);
        } else {
            self.close_command_line(cx);
            self.execute_command(implementation, PrefixArgument::None, window, cx);
        }
    }

    pub(crate) fn start_table_alignment(&mut self, scope: TableScope, cx: &mut Context<Self>) {
        if !self.command_line_is_open() {
            self.open_command_line(cx);
        }
        let Some(doc) = self.document_session().cloned() else {
            return;
        };
        if doc.read(cx).is_read_only() {
            return;
        }
        let Some(s) = self.command_line.session.as_ref() else {
            return;
        };
        if !s.accepts_input(cx) {
            return;
        }
        let id = s.id;
        let selection = self
            .editor(s.pane)
            .filter(|_| s.surface == PaneSurface::Editor)
            .map_or(Selection::default(), |e| e.read(cx).selection());
        if scope != TableScope::Document
            && (s.surface != PaneSurface::Editor
                || (scope == TableScope::Selection && selection.is_empty()))
        {
            return;
        }
        let snapshot = doc.read(cx).snapshot();
        let revision = snapshot.revision();
        let path = doc.read(cx).syntax_path().to_path_buf();
        let work = cx.background_executor().spawn(async move {
            crate::editor::plan_table_alignment(&path, &snapshot, scope, selection)
        });
        let s = self.command_line.session.as_mut().unwrap();
        s.phase = Phase::Running(Instant::now());
        s.focus_pending = false;
        self.focus_workspace_on_render = true;
        s.input.update(cx, |input, cx| {
            input.enabled = false;
            cx.notify();
        });
        s.timer = Some(cx.spawn(async move |weak, cx| {
            cx.background_executor().timer(RUNNING_DELAY).await;
            let _ = weak.update(cx, |_, cx| cx.notify());
        }));
        s.task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                let Some(s) =
                    this.command_line.session.as_ref().filter(|s| {
                        s.id == id && !s.returning && matches!(s.phase, Phase::Running(_))
                    })
                else {
                    return;
                };
                let valid = this.command_context_is_current(s, cx)
                    && this
                        .document_session()
                        .is_some_and(|doc| doc.read(cx).revision() == revision);
                if !valid {
                    this.command_failure("文档已改变，请重试 / Document changed; run again", cx);
                    return;
                }
                if let Some(plan) = result {
                    this.apply_table_alignment(plan, revision, selection, cx);
                } else {
                    this.command_failure(
                        "当前文档格式不支持表格对齐 / Unsupported document format",
                        cx,
                    );
                }
            });
        }));
        cx.notify();
    }

    pub(super) fn apply_table_alignment(
        &mut self,
        mut plan: TableAlignmentPlan,
        revision: Revision,
        before: Selection,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.document_session().cloned() else {
            return;
        };
        let mut undo = None;
        if !plan.edits.is_empty() {
            let editors = [PaneSide::Left, PaneSide::Right]
                .into_iter()
                .filter_map(|pane| {
                    let editor = self.editor(pane)?;
                    let mapped = plan.map_selection(editor.read(cx).selection());
                    Some((editor, mapped))
                })
                .collect::<Vec<_>>();
            let after = plan.map_selection(before);
            let edits = std::mem::take(&mut plan.edits);
            let applied = doc.update(cx, |doc, cx| {
                doc.edit(
                    DocumentCommand::new(
                        EditTransaction::new(revision, edits),
                        before,
                        after,
                        EditOrigin::Other,
                    ),
                    cx,
                )
            });
            match applied {
                Ok(delta) => {
                    undo = Some(delta.after);
                    for (editor, selection) in editors {
                        editor.update(cx, |editor, cx| {
                            editor.restore_command_selection(selection, delta.after, cx)
                        });
                    }
                    // DocumentEvent::Edited updates both preview panes through the workspace
                    // subscription. Scheduling here as well would enqueue the same delta twice.
                }
                Err(error) => {
                    self.command_failure(&error.to_string(), cx);
                    return;
                }
            }
        }
        let chinese = self.language == crate::i18n::Language::Chinese;
        let message = if plan.tables == 0 {
            self.command_text("当前范围没有可对齐的表格", "No tables in this scope")
                .to_owned()
        } else if chinese {
            format!(
                "已对齐 {} 个表格 · {} 个无需修改{}",
                plan.changed,
                plan.tables - plan.changed - plan.skipped,
                if plan.skipped > 0 {
                    format!(" · {} 个嵌套表格暂不支持", plan.skipped)
                } else {
                    String::new()
                }
            )
        } else {
            format!(
                "Aligned {} tables · {} unchanged · {} skipped",
                plan.changed,
                plan.tables - plan.changed - plan.skipped,
                plan.skipped
            )
        };
        self.command_result(message, undo, cx);
    }

    fn command_failure(&mut self, message: &str, cx: &mut Context<Self>) {
        if let Some(s) = self.command_line.session.as_mut() {
            s.phase = Phase::Input;
            s.error = Some(message.into());
            s.focus_pending = true;
            s.input.update(cx, |input, cx| {
                input.enabled = true;
                cx.notify();
            });
        }
        cx.notify();
    }

    fn command_result(&mut self, message: String, undo: Option<Revision>, cx: &mut Context<Self>) {
        let Some(s) = self.command_line.session.as_mut() else {
            return;
        };
        s.phase = Phase::Result { message, undo };
        let id = s.id;
        s.timer = Some(cx.spawn(async move |weak, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let _ = weak.update(cx, |this, cx| {
                if this.command_line.session.as_ref().is_some_and(|s| {
                    s.id == id && !s.returning && matches!(s.phase, Phase::Result { .. })
                }) {
                    this.close_command_line(cx);
                }
            });
        }));
        self.focus_active_surface(cx);
        cx.notify();
    }

    pub(super) fn undo_command_result(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.command_line.session.as_ref().filter(|s| !s.returning) else {
            return;
        };
        let Phase::Result {
            undo: Some(revision),
            ..
        } = s.phase
        else {
            return;
        };
        let Some(doc) = self.document_session().cloned() else {
            return;
        };
        if doc.read(cx).id() != s.document || doc.read(cx).revision() != revision {
            return;
        }
        if let Ok(HistoryOutcome::Applied(selection)) = doc.update(cx, |doc, cx| doc.undo(cx)) {
            if let Some(editor) = self.editor(self.document_workspace.active_pane) {
                let revision = doc.read(cx).revision();
                editor.update(cx, |editor, cx| {
                    editor.restore_command_selection(selection, revision, cx)
                });
            }
            self.command_result(
                self.command_text("已撤销表格对齐", "Table alignment undone")
                    .into(),
                None,
                cx,
            );
        }
    }
}
