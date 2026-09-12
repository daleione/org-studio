use crate::app::WorkspaceWindow;
use gpui::AppContext;

impl WorkspaceWindow {
    pub(crate) fn generated_command_disposition(
        &self,
        command: crate::editor::GeneratedCommand,
    ) -> crate::editor::CommandDisposition {
        if self.content_route == crate::app::ContentRoute::AgendaText
            || (self.content_route == crate::app::ContentRoute::Agenda
                && self.agenda.state.projection == super::state::AgendaProjection::Source)
        {
            self.agenda.text_view.command(command)
        } else {
            crate::editor::CommandDisposition::Unhandled
        }
    }

    pub(crate) fn close_agenda_text(&mut self, cx: &mut gpui::Context<Self>) {
        self.agenda.pending_text_task = None;
        self.agenda.pending_text_generation = None;
        self.agenda.text_view.close();
        self.agenda.text_query.close();
        self.agenda.text_projection_version = None;
        self.agenda.text_generation = None;
        self.agenda_text_open = false;
        self.content_route = self
            .agenda_text_return
            .take()
            .unwrap_or(crate::app::ContentRoute::Document);
        if self.content_route == crate::app::ContentRoute::Document {
            self.request_document_focus(cx);
        } else {
            self.focus_workspace_on_render = true;
        }
        cx.notify();
    }

    pub(crate) fn refresh_generated_result(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        if self.generated_command_disposition(crate::editor::GeneratedCommand::Refresh)
            != crate::editor::CommandDisposition::Enabled
        {
            return false;
        }
        if self.content_route == crate::app::ContentRoute::AgendaText {
            self.agenda.requery_independent_text();
        } else {
            self.agenda.requery();
        }
        self.sync_agenda_text_buffer(cx);
        true
    }

    pub(crate) fn activate_generated_line(&mut self, line: u64, cx: &mut gpui::Context<Self>) {
        if self.generated_command_disposition(crate::editor::GeneratedCommand::Activate)
            == crate::editor::CommandDisposition::Enabled
            || (self.content_route == crate::app::ContentRoute::Agenda
                && self.agenda.state.projection == super::state::AgendaProjection::Source)
        {
            self.open_agenda_text_line(line, cx);
        }
    }

    pub(crate) fn sync_agenda_text_buffer(&mut self, cx: &mut gpui::Context<Self>) {
        if !matches!(
            self.content_route,
            crate::app::ContentRoute::Agenda | crate::app::ContentRoute::AgendaText
        ) || (matches!(self.content_route, crate::app::ContentRoute::Agenda)
            && self.agenda.state.projection != super::state::AgendaProjection::Source)
        {
            return;
        }
        let result = if matches!(self.content_route, crate::app::ContentRoute::AgendaText) {
            self.agenda.text_query.result.as_ref()
        } else {
            self.agenda.page_query.result.as_ref()
        };
        let Some(result) = result else {
            return;
        };
        let today = jiff::Zoned::now().date();
        let generation = (
            result.query_id,
            result.query_generation,
            self.language,
            today,
        );
        if self.agenda.text_generation == Some(generation) {
            return;
        }
        let window = if matches!(self.content_route, crate::app::ContentRoute::AgendaText) {
            result.query.window
        } else {
            self.agenda
                .state
                .browses_dates()
                .then(|| self.agenda.state.calendar_window(today))
        };
        let buffer = super::view::agenda_text(result, self.language, window);
        let logical_anchor = self.agenda.text_editor.as_ref().and_then(|editor| {
            let line = editor.read(cx).selected_line(cx) as usize;
            self.agenda
                .text_projection_version
                .and_then(|version| self.agenda.text_view.resolve(version, line))
                .map(|target| (target.entry.entry, target.placement))
        });
        let restored_line = logical_anchor.and_then(|anchor| {
            buffer.targets.iter().position(|target| {
                target
                    .as_ref()
                    .is_some_and(|target| (target.entry.entry, target.placement) == anchor)
            })
        });
        self.agenda.text_view.mount();
        let Some(projection) =
            self.agenda
                .text_view
                .publish(buffer.text, buffer.highlights, buffer.targets)
        else {
            return;
        };
        if let Some(editor) = &self.agenda.text_editor {
            editor.update(cx, |editor, cx| {
                editor.update_read_only(projection.text.clone(), projection.highlights.clone(), cx);
                if let Some(line) = restored_line {
                    editor.select_read_only_line(line, cx);
                }
            });
        } else {
            let projection = projection.clone();
            self.agenda.text_editor = Some(cx.new(|cx| {
                crate::editor::SemanticEditor::new_activatable_read_only(
                    projection.text.clone(),
                    projection.highlights.clone(),
                    "Agenda",
                    "agenda.text-projection",
                    cx,
                )
            }));
        }
        self.agenda.text_projection_version = Some(projection.version);
        self.agenda.text_generation = Some(generation);
    }

    pub(crate) fn open_agenda_text_line(&mut self, line: u64, cx: &mut gpui::Context<Self>) {
        let Some(reference) = self.agenda.text_projection_version.and_then(|version| {
            self.agenda
                .text_view
                .resolve(version, line as usize)
                .copied()
        }) else {
            return;
        };
        let result = if matches!(self.content_route, crate::app::ContentRoute::AgendaText) {
            self.agenda.text_query.result.as_ref()
        } else {
            self.agenda.page_query.result.as_ref()
        };
        let Some(entry) = result.and_then(|result| result.resolve_entry(reference.entry)) else {
            return;
        };
        let Some(task) = self.agenda.task(entry.row.task) else {
            return;
        };
        let path = task.source.path.as_ref().clone();
        let already_open = self
            .document_path(cx)
            .is_some_and(|current| same_path(current, &path));
        self.agenda.pending_text_task = Some(task);
        if already_open {
            self.agenda.pending_text_generation = None;
            self.content_route = crate::app::ContentRoute::Document;
            self.agenda_text_return = Some(crate::app::ContentRoute::Document);
            self.request_document_focus(cx);
            self.reveal_agenda_text_target(cx);
        } else {
            self.open(path, cx);
            if self.agenda.pending_text_task.is_some() {
                self.agenda.pending_text_generation = Some(self.generation);
            }
        }
        cx.notify();
    }

    pub(super) fn reveal_agenda_text_target(&mut self, cx: &mut gpui::Context<Self>) {
        if self
            .agenda
            .pending_text_generation
            .is_some_and(|generation| generation != self.generation)
        {
            self.agenda.pending_text_task = None;
            self.agenda.pending_text_generation = None;
            return;
        }
        let Some(task) = self.agenda.pending_text_task.as_ref() else {
            return;
        };
        let Some(session) = self.document_session() else {
            return;
        };
        if !same_path(session.read(cx).path(), task.source.path.as_path()) {
            if matches!(
                self.save.interaction,
                crate::app::save::SaveInteraction::Idle
            ) {
                self.agenda.pending_text_task = None;
            }
            return;
        }
        let heading = crate::agenda::resolve_heading(session.read(cx), task);
        self.agenda.pending_text_task = None;
        self.agenda.pending_text_generation = None;
        match heading {
            Ok(heading) => {
                if self.content_route == crate::app::ContentRoute::AgendaText {
                    self.agenda.text_view.hide();
                    self.agenda_text_return = Some(crate::app::ContentRoute::Document);
                    self.content_route = crate::app::ContentRoute::Document;
                }
                self.show_editor(cx);
                if let Some(editor) = self.editor(self.document_workspace.active_pane) {
                    editor.update(cx, |editor, cx| {
                        editor.set_selection(
                            crate::document::Selection::caret(heading.source.start),
                            cx,
                        );
                        editor.request_focus(cx);
                    });
                }
            }
            Err(_) => self.set_document_notice(Some(
                self.language.text("agenda.task_location_changed").into(),
            )),
        }
    }
}

fn same_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    left == right
        || left
            .canonicalize()
            .ok()
            .zip(right.canonicalize().ok())
            .is_some_and(|(left, right)| left == right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ByteRange, EditTransaction, TextEdit};

    #[gpui::test]
    fn text_task_jump_preserves_unsaved_session_and_disambiguates_duplicate_titles(
        cx: &mut gpui::TestAppContext,
    ) {
        let path =
            std::env::temp_dir().join(format!("agenda-buffer-jump-{}.org", std::process::id()));
        std::fs::write(&path, "* TODO Same\n* TODO Same\n").unwrap();
        let loaded = crate::preview::load_workspace_document(path.clone(), false).unwrap();
        let shard =
            crate::agenda::shard_from_disk(crate::agenda::FileId(991), 1, path.clone()).unwrap();
        let key = shard.tasks[1].key;
        let expected = shard.tasks[1].source.heading_range.start.0 + 8;
        let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
        cx.update(|_, app| {
            workspace.update(app, |workspace, cx| {
                workspace.generation = 1;
                assert!(workspace.apply_load_result(1, Ok(loaded), cx));
                let session = workspace.document_session().unwrap().clone();
                session.update(cx, |session, cx| {
                    session
                        .apply_transient_edit(
                            EditTransaction::new(
                                session.revision(),
                                vec![TextEdit::new(ByteRange::new(0, 0), "unsaved\n")],
                            ),
                            cx,
                        )
                        .unwrap();
                });
                let index = workspace.agenda.runtime.index.replace(shard);
                let result = workspace.agenda.page_query.execute(
                    index,
                    &crate::agenda::AgendaQuery::builtin(
                        crate::agenda::BuiltinQuery::Unscheduled,
                        "2026-09-07".parse().unwrap(),
                    ),
                );
                let target = result
                    .placements
                    .iter()
                    .find(|placement| result.entries[placement.entry.0 as usize].row.task == key)
                    .and_then(|placement| result.placement_ref(placement.key))
                    .unwrap();
                workspace.agenda.page_query.result = Some(result);
                workspace.agenda.text_view.mount();
                let projection = workspace
                    .agenda
                    .text_view
                    .publish("Header\nTask\n".into(), vec![], vec![None, Some(target)])
                    .unwrap();
                workspace.agenda.text_projection_version = Some(projection.version);
                workspace.content_route = crate::app::ContentRoute::Agenda;
                workspace.open_agenda_text_line(0, cx);
                assert!(matches!(
                    workspace.content_route,
                    crate::app::ContentRoute::Agenda
                ));
                workspace.open_agenda_text_line(1, cx);
                assert!(matches!(
                    workspace.content_route,
                    crate::app::ContentRoute::Document
                ));
                assert_eq!(workspace.document_session().unwrap(), &session);
                assert!(session.read(cx).is_dirty());
                let editor = workspace
                    .editor(workspace.document_workspace.active_pane)
                    .unwrap();
                assert_eq!(editor.read(cx).selection().head().0, expected);
                assert!(workspace.agenda.pending_text_task.is_none());
            })
        });
        std::fs::remove_file(path).unwrap();
    }

    #[gpui::test]
    fn closed_independent_text_can_be_remounted_by_agenda_source_projection(
        cx: &mut gpui::TestAppContext,
    ) {
        let source = crate::document::DocumentSnapshot::from_utf8(
            b"* TODO Reopen\n<2026-09-07 Mon>\n".to_vec(),
        )
        .unwrap();
        let analysis = crate::org_semantic::analyze(
            &source,
            std::sync::Arc::new(crate::org_syntax::parse(&source)),
        );
        let shard = crate::agenda::shard_from_live(
            crate::agenda::FileId(992),
            1,
            std::sync::Arc::new(std::path::PathBuf::from("/tmp/reopen.org")),
            &analysis,
        );
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        cx.update(|app| {
            workspace.update(app, |workspace, cx| {
                let index = workspace.agenda.runtime.index.replace(shard);
                let result = workspace.agenda.page_query.execute(
                    index,
                    &crate::agenda::AgendaQuery::builtin(
                        crate::agenda::BuiltinQuery::Today,
                        "2026-09-07".parse().unwrap(),
                    ),
                );
                workspace.agenda.page_query.result = Some(result);
                workspace.content_route = crate::app::ContentRoute::AgendaText;
                workspace.agenda.text_view.mount();
                workspace.agenda_text_open = true;
                workspace.close_agenda_text(cx);

                workspace.content_route = crate::app::ContentRoute::Agenda;
                workspace.agenda.state.projection = super::super::state::AgendaProjection::Source;
                workspace.sync_agenda_text_buffer(cx);
                assert!(workspace.agenda.text_projection_version.is_some());
            });
        });
    }
}
