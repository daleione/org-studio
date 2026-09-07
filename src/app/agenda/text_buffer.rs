use crate::app::WorkspaceWindow;
use gpui::AppContext;

impl WorkspaceWindow {
    pub(super) fn sync_agenda_text_buffer(&mut self, cx: &mut gpui::Context<Self>) {
        if !matches!(self.content_route, crate::app::ContentRoute::Agenda)
            || self.agenda.state.projection != super::state::AgendaProjection::Source
        {
            return;
        }
        let Some(result) = &self.agenda.result else {
            return;
        };
        let today = jiff::Zoned::now().date();
        let generation = (result.query_generation, self.language, today);
        if self.agenda.text_generation == Some(generation) {
            return;
        }
        let buffer = super::view::agenda_text(
            result,
            self.language,
            self.agenda
                .state
                .browses_dates()
                .then(|| self.agenda.state.calendar_window(today)),
        );
        if let Some(editor) = &self.agenda.text_editor {
            editor.update(cx, |editor, cx| {
                editor.update_read_only(buffer.text, buffer.highlights, cx)
            });
        } else {
            self.agenda.text_editor = Some(cx.new(|cx| {
                crate::editor::SemanticEditor::new_read_only(buffer.text, buffer.highlights, cx)
            }));
        }
        self.agenda.text_targets = buffer.targets;
        self.agenda.text_generation = Some(generation);
    }

    pub(crate) fn open_agenda_text_line(
        &mut self,
        line: u64,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(key) = self
            .agenda
            .text_targets
            .get(line as usize)
            .copied()
            .flatten()
        else {
            return;
        };
        let Some(task) = self.agenda.task(key) else {
            return;
        };
        let path = task.source.path.as_ref().clone();
        let already_open = self
            .document_path(cx)
            .is_some_and(|current| same_path(current, &path));
        self.content_route = crate::app::ContentRoute::Document;
        self.request_document_focus(cx);
        self.agenda.pending_text_task = Some(task);
        if already_open {
            self.reveal_agenda_text_target(cx);
        } else {
            self.request_open(path, window, cx);
        }
        cx.notify();
    }

    pub(super) fn reveal_agenda_text_target(&mut self, cx: &mut gpui::Context<Self>) {
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
        match heading {
            Ok(heading) => {
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
        cx.update(|window, app| {
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
                workspace.agenda.index.replace(shard);
                workspace.agenda.text_targets = vec![None, Some(key)];
                workspace.content_route = crate::app::ContentRoute::Agenda;
                workspace.open_agenda_text_line(0, window, cx);
                assert!(matches!(
                    workspace.content_route,
                    crate::app::ContentRoute::Agenda
                ));
                workspace.open_agenda_text_line(1, window, cx);
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
}
