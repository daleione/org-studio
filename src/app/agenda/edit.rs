use std::sync::Arc;

use gpui::{AppContext, Context};

use crate::{
    agenda::{AgendaCommand, SourceVersion, TaskRecord, prepare_edit, shard_from_live},
    app::WorkspaceWindow,
    document::{DocumentSession, Revision, write_atomic},
};

impl WorkspaceWindow {
    pub(super) fn capture_agenda(
        &mut self,
        path: &std::path::Path,
        heading: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Result<bool, crate::agenda::WorkflowError> {
        use crate::document::{
            ByteRange, DocumentCommand, EditOrigin, EditTransaction, Selection, TextEdit,
            TextSnapshot,
        };
        let session = self.buffer_for_path(path, cx);
        if let Some(session) = session {
            let snapshot = session.read(cx).snapshot();
            let range = ByteRange::new(0, snapshot.len_bytes());
            let text = crate::agenda::capture_text(
                snapshot.copy_range(range),
                heading,
                &self.agenda.state.capture,
            )?;
            let command = DocumentCommand::new(
                EditTransaction::new(snapshot.revision(), vec![TextEdit::new(range, text)]),
                Selection::default(),
                Selection::default(),
                EditOrigin::Other,
            );
            session
                .update(cx, |session, cx| session.edit(command, cx))
                .map_err(|error| {
                    crate::agenda::WorkflowError::Io(Arc::from(format!("{error:?}")))
                })?;
            self.sync_agenda_document(cx);
            Ok(false)
        } else {
            crate::agenda::append_capture(path, heading, &self.agenda.state.capture)?;
            Ok(true)
        }
    }

    pub(super) fn agenda_disk_task(
        &self,
        task: &TaskRecord,
        cx: &Context<Self>,
    ) -> Result<TaskRecord, crate::agenda::WorkflowError> {
        let mut task = task.clone();
        if let Some(session) = self.buffer_for_path(&task.source.path, cx) {
            let session = session.read(cx);
            let path = session.path();
            if path == task.source.path.as_path()
                || std::fs::canonicalize(path).is_ok_and(|path| path == *task.source.path)
            {
                if session.is_dirty() {
                    return Err(crate::agenda::WorkflowError::InvalidInput(
                        "跨文件移动前请先保存打开的源文件和目标文件",
                    ));
                }
                if matches!(task.source.version, SourceVersion::Live { document, revision } if document != session.id() || revision != session.revision())
                {
                    return Err(crate::agenda::WorkflowError::SourceChanged);
                }
                let stamp = crate::document::FileStamp::read(path)?;
                if stamp != *session.sync_state().base() {
                    return Err(crate::agenda::WorkflowError::SourceChanged);
                }
                task.source.version = SourceVersion::Disk(Arc::new(stamp));
            }
        }
        Ok(task)
    }

    /// Open documents retain their undo history and normal save policy. Other
    /// files use the same command validation and conflict-aware atomic writer.
    pub(super) fn apply_agenda_command(
        &mut self,
        task: &TaskRecord,
        operation: &AgendaCommand,
        cx: &mut Context<Self>,
    ) -> Result<(), Arc<str>> {
        let open = self.buffer_for_path(&task.source.path, cx);
        let is_open = open.is_some();
        let session = match open {
            Some(session) => session,
            None => {
                let bytes = std::fs::read(task.source.path.as_path())
                    .map_err(|error| Arc::from(error.to_string()))?;
                let session = DocumentSession::from_utf8(task.source.path.as_ref().clone(), bytes)
                    .map_err(|error| Arc::from(format!("{error:?}")))?;
                cx.new(|_| session)
            }
        };
        let mut task = task.clone();
        let indexed_path = task.source.path.clone();
        task.source.path = Arc::new(session.read(cx).path().to_path_buf());
        if is_open && let SourceVersion::Disk(expected) = &task.source.version {
            let current = session.read(cx);
            if current.sync_state().base() != expected.as_ref() {
                return Err(Arc::from("文件版本已改变，请刷新后重试"));
            }
            task.source.version = SourceVersion::Live {
                document: current.id(),
                revision: Revision::INITIAL,
            };
        }
        let prepared = prepare_edit(session.read(cx), &task, operation)
            .map_err(|error| Arc::from(format!("{error:?}")))?;
        session
            .update(cx, |session, cx| session.edit(prepared.command, cx))
            .map_err(|error| Arc::from(format!("{error:?}")))?;
        if !is_open {
            let request = session
                .update(cx, |session, _| session.begin_save(None))
                .map_err(|error| Arc::from(format!("{error:?}")))?;
            let outcome = write_atomic(request).map_err(|error| Arc::from(format!("{error:?}")))?;
            session
                .update(cx, |session, cx| session.finish_save(outcome, cx))
                .map_err(|error| Arc::from(format!("{error:?}")))?;
            self.agenda.refresh_sources();
        } else {
            let snapshot = session.read(cx).snapshot();
            let analysis = crate::org_semantic::analyze(
                &snapshot,
                Arc::new(crate::org_syntax::parse(&snapshot)),
            );
            self.agenda.analysis_generation += 1;
            self.agenda.runtime.index.replace(shard_from_live(
                task.key.file,
                self.agenda.analysis_generation,
                indexed_path,
                &analysis,
            ));
            self.agenda.requery();
            self.sync_agenda_document(cx);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agenda::{FileId, shard_from_disk},
        app::{PanePair, ReadyDocument, WorkspaceLoadState},
        document::{ByteRange, TextSnapshot},
    };

    #[gpui::test]
    fn same_file_refile_preserves_unterminated_unicode_heading(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                "/tmp/agenda-refile-session.org".into(),
                "* TODO Source\n* TODO 目标".as_bytes().to_vec(),
            )
            .unwrap()
        });
        let tasks = cx.read(|cx| {
            let snapshot = session.read(cx).snapshot();
            let analysis = crate::org_semantic::analyze(
                &snapshot,
                Arc::new(crate::org_syntax::parse(&snapshot)),
            );
            shard_from_live(
                FileId(1),
                1,
                Arc::new(session.read(cx).path().to_path_buf()),
                &analysis,
            )
            .tasks
        });
        let prepared = cx.read(|cx| {
            prepare_edit(
                session.read(cx),
                &tasks[0],
                &AgendaCommand::RefileSameFile(Box::new(tasks[1].clone())),
            )
            .unwrap()
        });
        session.update(cx, |session, cx| {
            session.edit(prepared.command, cx).unwrap();
            let snapshot = session.snapshot();
            assert_eq!(
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
                "* TODO 目标\n** TODO Source\n"
            );
        });
    }

    #[gpui::test]
    fn unopened_task_uses_atomic_save_and_rejects_stale_second_write(
        cx: &mut gpui::TestAppContext,
    ) {
        let path = std::env::temp_dir().join(format!("agenda-gateway-{}.org", std::process::id()));
        std::fs::write(&path, "* TODO Original\n").unwrap();
        let task = shard_from_disk(FileId(1), 1, path.clone()).unwrap().tasks[0].clone();
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        workspace.update(cx, |workspace, cx| {
            workspace
                .apply_agenda_command(&task, &AgendaCommand::SetPriority(Some('B')), cx)
                .unwrap();
            assert!(
                workspace
                    .apply_agenda_command(&task, &AgendaCommand::DeleteSubtree, cx)
                    .is_err()
            );
        });
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "* TODO [#B] Original\n"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[gpui::test]
    fn consecutive_live_edits_publish_new_tasks_and_remain_undoable(cx: &mut gpui::TestAppContext) {
        let path =
            std::env::temp_dir().join(format!("agenda-live-gateway-{}.org", std::process::id()));
        std::fs::write(&path, "* TODO Original\n").unwrap();
        let shard = shard_from_disk(FileId(1), 1, path.clone()).unwrap();
        let original = shard.tasks[0].clone();
        let session = cx.new(|_| {
            DocumentSession::from_utf8(path.clone(), std::fs::read(&path).unwrap()).unwrap()
        });
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        workspace.update(cx, |workspace, cx| {
            workspace.agenda.runtime.index.replace(shard);
            workspace.state = WorkspaceLoadState::Ready {
                document: ReadyDocument {
                    session: session.clone(),
                    editor_syntax: Arc::default(),
                    editors: PanePair {
                        left: None,
                        right: None,
                    },
                    readers: PanePair {
                        left: None,
                        right: None,
                    },
                },
            };
            workspace
                .apply_agenda_command(&original, &AgendaCommand::SetPriority(Some('B')), cx)
                .unwrap();
            let current = workspace.agenda.runtime.index.snapshot().files[0].tasks[0].clone();
            assert!(matches!(current.source.version, SourceVersion::Live { .. }));
            workspace
                .apply_agenda_command(&current, &AgendaCommand::SetPriority(Some('A')), cx)
                .unwrap();
            assert!(matches!(
                workspace.agenda_disk_task(&current, cx),
                Err(crate::agenda::WorkflowError::InvalidInput(_))
            ));
        });
        session.update(cx, |session, cx| {
            session.undo(cx).unwrap();
            let snapshot = session.snapshot();
            assert_eq!(
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
                "* TODO [#B] Original\n"
            );
        });
        workspace.update(cx, |workspace, cx| {
            workspace.agenda.state.capture.title = "Captured".into();
            assert!(!workspace.capture_agenda(&path, None, cx).unwrap());
        });
        session.update(cx, |session, cx| {
            let snapshot = session.snapshot();
            assert_eq!(
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
                "* TODO [#B] Original\n* TODO Captured\n"
            );
            session.undo(cx).unwrap();
        });
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "* TODO Original\n");
        std::fs::remove_file(path).unwrap();
    }
}
