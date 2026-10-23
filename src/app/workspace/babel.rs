use gpui::{Context, Focusable, Window};

use crate::{
    app::{ContentRoute, PaneSurface, WorkspaceWindow, echo_area::EchoMessage},
    document::{DocumentCommand, EditOrigin, EditTransaction},
    editor::SourceRunPhase,
};

impl WorkspaceWindow {
    pub(crate) fn tangle_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.babel_run.is_some() {
            self.show_echo_message(
                EchoMessage::warning("Wait for or stop the current execution before tangling"),
                cx,
            );
            return;
        }
        let Some(session) = self.document_session().cloned() else {
            return;
        };
        let (snapshot, path) = {
            let session = session.read(cx);
            (session.snapshot(), session.path().to_path_buf())
        };
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("org"))
        {
            self.show_echo_message(EchoMessage::error("Tangling requires an Org document"), cx);
            return;
        }
        let request = match crate::babel::prepare_tangle(&snapshot, &path) {
            Ok(request) => request,
            Err(message) => {
                self.show_echo_message(EchoMessage::error(message), cx);
                return;
            }
        };
        let request_id = self.begin_babel_request(cx);
        self.set_echo_message(Some(EchoMessage::working("Tangling Org source blocks…")));
        let background = cx
            .background_executor()
            .spawn(async move { crate::babel::tangle_document(request) });
        self.babel_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = background.await;
            let _ = this.update_in(cx, |this, _window, cx| {
                if this.babel_request != request_id {
                    return;
                }
                this.babel_task = None;
                match result {
                    Ok(output) => {
                        let current = this.document_session().is_some_and(|current| {
                            current == &session
                                && current.read(cx).id() == output.document_id
                                && current.read(cx).revision() == output.revision
                        });
                        if !current {
                            this.show_echo_message(
                                EchoMessage::warning(
                                    "Tangle cancelled because the document changed",
                                ),
                                cx,
                            );
                            return;
                        }
                        match output.publish() {
                            Ok(paths) => {
                                for path in &paths {
                                    session.update(cx, |session, cx| {
                                        session.resource_changed(path.clone(), cx)
                                    });
                                }
                                let message = if let [path] = paths.as_slice() {
                                    format!("Tangled {}", path.display())
                                } else {
                                    format!("Tangled {} files", paths.len())
                                };
                                this.show_echo_message(EchoMessage::success(message), cx);
                            }
                            Err(message) => this.show_echo_message(EchoMessage::error(message), cx),
                        }
                    }
                    Err(message) => this.show_echo_message(EchoMessage::error(message), cx),
                }
            });
        }));
    }

    pub(crate) fn execute_org_context_command(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.content_route != ContentRoute::Document {
            return;
        }
        if matches!(
            self.document_workspace.active_surface(),
            PaneSurface::Editor
        ) && let Some(editor) = self.editor(self.document_workspace.active_pane)
        {
            let handled = editor.update(cx, |editor, cx| {
                match editor.recalculate_table_at_selection(false, cx) {
                    Ok(true) => true,
                    Err(error) => {
                        editor.show_command_feedback(&error, cx);
                        true
                    }
                    Ok(false) => editor.align_table_at_selection(cx),
                }
            });
            if handled {
                return;
            }
        }
        self.execute_source_block(window, cx);
    }

    pub(crate) fn execute_source_block(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(
            self.document_workspace.active_surface(),
            PaneSurface::Editor
        ) {
            self.show_echo_message(
                EchoMessage::error("Source blocks can only be executed from the editor"),
                cx,
            );
            return;
        }
        let Some(editor) = self.editor(self.document_workspace.active_pane) else {
            return;
        };
        let selection = editor.read(cx).selection();
        self.execute_source_block_for(editor, selection, window, cx);
    }

    pub(crate) fn execute_source_block_at(
        &mut self,
        source_offset: crate::document::ByteOffset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let origin = self.source_editor_for_action(window, cx);
        let Some((pane, editor)) = origin else {
            self.show_echo_message(
                EchoMessage::error("The source editor is no longer focused"),
                cx,
            );
            return;
        };
        self.document_workspace.active_pane = pane;
        self.execute_source_block_for(
            editor,
            crate::document::Selection::caret(source_offset),
            window,
            cx,
        );
    }

    pub(crate) fn source_editor_for_action(
        &self,
        window: &Window,
        cx: &gpui::App,
    ) -> Option<(
        crate::app::PaneSide,
        gpui::Entity<crate::editor::SemanticEditor>,
    )> {
        [crate::app::PaneSide::Left, crate::app::PaneSide::Right]
            .into_iter()
            .filter_map(|pane| self.editor(pane).map(|editor| (pane, editor)))
            .find(|(_, editor)| editor.read(cx).focus_handle(cx).is_focused(window))
    }

    fn execute_source_block_for(
        &mut self,
        editor: gpui::Entity<crate::editor::SemanticEditor>,
        selection: crate::document::Selection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.document_session().cloned() else {
            return;
        };
        let (snapshot, path) = {
            let session = session.read(cx);
            (session.snapshot(), session.path().to_path_buf())
        };
        let open = self
            .buffer_sessions()
            .map(|session| {
                let session = session.read(cx);
                (session.path().to_owned(), session.snapshot())
            })
            .collect();
        let request = match crate::babel::prepare_execution(&snapshot, &path, selection, open) {
            Ok(request) => request,
            Err(message) => {
                editor.update(cx, |editor, cx| {
                    editor.finish_source_run_feedback(SourceRunPhase::Failure, cx)
                });
                self.show_echo_message(EchoMessage::error(message), cx);
                return;
            }
        };

        if let Some(run) = &self.babel_run {
            if run.source == request.source_block_start && run.document == snapshot.document_id() {
                self.stop_babel_execution(self.babel_request, cx);
            } else {
                self.show_echo_message(
                    EchoMessage::warning("Wait for or stop the current execution"),
                    cx,
                );
            }
            return;
        }
        self.begin_babel_request(cx);
        if request
            .requests()
            .iter()
            .any(|(_, r)| r.mysql.as_ref().is_some_and(|mysql| !mysql.ready()))
        {
            editor.update(cx, |editor, cx| {
                editor.finish_source_run_feedback(SourceRunPhase::Failure, cx)
            });
            self.show_echo_message(EchoMessage::error(crate::babel::mysql::CONNECTION_HELP), cx);
            return;
        }
        self.confirm_source_execution(request, session, editor, window, cx);
    }

    pub(crate) fn confirm_source_execution(
        &mut self,
        request: crate::babel::ExecutionPlan,
        session: gpui::Entity<crate::document::DocumentSession>,
        editor: gpui::Entity<crate::editor::SemanticEditor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pending = request
            .requests()
            .into_iter()
            .filter_map(|(path, invocation)| {
                let sql_target = invocation.mysql.as_ref().map(|mysql| mysql.identity());
                let trusted_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
                let trusted = if let Some(target) = &sql_target {
                    self.mysql
                        .trusted
                        .contains(&(trusted_path.clone(), target.clone()))
                        && !invocation.mysql.as_ref().unwrap().ask_each_time
                } else {
                    invocation.external_program().is_none()
                        || self.babel_trusted_documents.contains(&trusted_path)
                            && !invocation.headers_eval_query()
                };
                (!trusted).then_some((trusted_path, sql_target, invocation))
            })
            .collect::<Vec<_>>();
        if !pending.is_empty() {
            editor.update(cx, |editor, cx| {
                editor.clear_source_run_feedback();
                cx.notify();
            });
            let targets = pending
                .iter()
                .map(|(path, target, _)| (path.clone(), target.clone()))
                .collect::<std::collections::HashSet<_>>();
            let answer = if targets.len() == 1
                && let Some(mysql) = &pending[0].2.mysql
            {
                self.prompt_mysql_execution(mysql, &session, window, cx)
            } else {
                self.prompt_code_execution(&request, &session, window, cx)
            };
            let approvals = pending
                .into_iter()
                .map(|(path, target, _)| (path, target))
                .collect::<Vec<_>>();
            let request_id = self.babel_request;
            self.babel_task = Some(cx.spawn_in(window, async move |this, cx| {
                let approved = answer.recv().await.unwrap_or(false);
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.babel_request != request_id {
                        return;
                    }
                    this.babel_task = None;
                    if !approved || !this.babel_versions_current(&request.versions, cx) {
                        return;
                    }
                    for (trusted_path, sql_target) in approvals {
                        if let Some(target) = sql_target {
                            this.mysql.trusted.insert((trusted_path, target));
                        } else {
                            this.babel_trusted_documents.insert(trusted_path);
                        }
                    }
                    this.launch_source_execution(request, session, editor, window, cx);
                });
            }));
            return;
        }
        self.launch_source_execution(request, session, editor, window, cx);
    }

    fn launch_source_execution(
        &mut self,
        request: crate::babel::ExecutionPlan,
        session: gpui::Entity<crate::document::DocumentSession>,
        editor: gpui::Entity<crate::editor::SemanticEditor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source_block_start = request.source_block_start;
        if !self.babel_versions_current(&request.versions, cx) {
            self.show_echo_message(
                EchoMessage::warning("Babel execution cancelled because a dependency changed"),
                cx,
            );
            return;
        }
        self.start_babel_run(
            request.document_id,
            source_block_start,
            request.cancel.clone(),
            cx,
        );
        if let Some(run) = &mut self.babel_run {
            run.documents = request.versions.iter().map(|v| v.document_id()).collect();
            run.progress = request.progress.clone();
        }
        editor.update(cx, |editor, cx| {
            editor.show_source_run_feedback(source_block_start, SourceRunPhase::Running, cx)
        });
        self.babel_editor = Some(editor.clone());
        let request_id = self.babel_request;
        let language_name = request.language_name();
        let versions = request.versions.clone();
        let background = cx
            .background_executor()
            .spawn(async move { crate::babel::execute_plan(request) });
        self.babel_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = background.await;
            let _ = this.update_in(cx, |this, _window, cx| {
                if this.babel_request != request_id {
                    return;
                }
                this.babel_task = None;
                this.babel_editor = None;
                let cancelled = this
                    .babel_run
                    .as_ref()
                    .is_some_and(|run| run.cancel.cancelled());
                this.babel_run = None;
                let result = if cancelled {
                    Err("Babel execution cancelled; previous results were preserved".into())
                } else if this.babel_versions_current(&versions, cx) {
                    result
                } else {
                    Err("Babel result was discarded because a dependency changed".into())
                };
                this.apply_babel_result(session, editor, language_name, result, cx);
            });
        }));
        cx.notify();
    }

    fn babel_versions_current(
        &self,
        versions: &[crate::babel::DocumentVersion],
        cx: &gpui::App,
    ) -> bool {
        versions.iter().all(|version| {
            let session = self
                .buffer_session(version.document_id(), cx)
                .or_else(|| self.buffer_for_path(&version.path, cx));
            let snapshot = session.as_ref().map(|s| s.read(cx).snapshot());
            version.matches(snapshot.as_ref())
                && session
                    .as_ref()
                    .is_none_or(|s| version.path_matches(s.read(cx).path()))
        })
    }

    fn begin_babel_request(&mut self, cx: &mut Context<Self>) -> u64 {
        self.babel_task = None;
        let code_prompt = self.babel_confirmation.take().is_some();
        if self.mysql.confirmation.take().is_some() || code_prompt {
            self.request_document_focus(cx);
        }
        self.babel_run = None;
        if let Some(previous_editor) = self.babel_editor.take() {
            previous_editor.update(cx, |editor, cx| {
                editor.clear_source_run_feedback();
                cx.notify();
            });
        }
        self.babel_request = self.babel_request.wrapping_add(1);
        self.babel_request
    }

    fn apply_babel_result(
        &mut self,
        session: gpui::Entity<crate::document::DocumentSession>,
        editor: gpui::Entity<crate::editor::SemanticEditor>,
        language_name: &'static str,
        result: Result<crate::babel::PreparedBabelOutput, String>,
        cx: &mut Context<Self>,
    ) {
        let mut output = match result {
            Ok(output) => output,
            Err(message) => {
                editor.update(cx, |editor, cx| {
                    editor.finish_source_run_feedback(SourceRunPhase::Failure, cx)
                });
                self.show_echo_message(
                    EchoMessage::error(format!("{language_name} failed: {message}")),
                    cx,
                );
                return;
            }
        };
        let current = self.document_session().is_some_and(|current| {
            current.read(cx).id() == output.document_id
                && current.read(cx).revision() == output.revision
                && current == &session
        });
        if !current {
            editor.update(cx, |editor, cx| {
                editor.finish_source_run_feedback(SourceRunPhase::Failure, cx)
            });
            self.show_echo_message(
                EchoMessage::warning(format!(
                    "{language_name} result was discarded because the source changed"
                )),
                cx,
            );
            return;
        }
        output = match output.publish() {
            Ok(output) => output,
            Err(message) => {
                editor.update(cx, |editor, cx| {
                    editor.finish_source_run_feedback(SourceRunPhase::Failure, cx)
                });
                self.show_echo_message(EchoMessage::error(message), cx);
                return;
            }
        };
        debug_assert_eq!(output.language_name, language_name);
        let source_block_start = output.offset_after_result_edit(output.source_block_start);
        let after_selection = crate::document::Selection::new(
            output.offset_after_result_edit(output.selection.anchor()),
            output.offset_after_result_edit(output.selection.head()),
        );
        let table = output.table.take();
        let file = output.file().map(|(target, link, warning)| {
            (
                target.to_path_buf(),
                link.to_owned(),
                warning.map(str::to_owned),
            )
        });
        if let Some((target, _, _)) = &file {
            session.update(cx, |session, cx| {
                session.resource_changed(target.clone(), cx)
            });
        }
        if let Some(edit) = output.result_edit.take() {
            let selection = output.selection;
            let revision = output.revision;
            let result = session.update(cx, |session, cx| {
                session.edit(
                    DocumentCommand::new(
                        EditTransaction::new(revision, vec![edit]),
                        selection,
                        after_selection,
                        EditOrigin::Other,
                    ),
                    cx,
                )
            });
            if result.is_err() {
                editor.update(cx, |editor, cx| {
                    editor.finish_source_run_feedback(SourceRunPhase::Failure, cx)
                });
                let message = if file.is_some() {
                    format!(
                        "The {language_name} output was written, but #+RESULTS could not be updated"
                    )
                } else {
                    format!("The {language_name} result could not be inserted")
                };
                self.show_echo_message(EchoMessage::warning(message), cx);
                return;
            }
        }
        let message = if let Some(table) = table {
            EchoMessage::success(match self.language {
                crate::i18n::Language::Chinese => format!(
                    "查询完成 · {} 行 × {} 列 · {} ms",
                    table.rows.len(),
                    table.columns.len(),
                    table.elapsed.as_millis()
                ),
                crate::i18n::Language::English => format!(
                    "Query complete · {} rows × {} columns · {} ms",
                    table.rows.len(),
                    table.columns.len(),
                    table.elapsed.as_millis()
                ),
            })
        } else {
            match file {
                Some((target, link, None)) => {
                    EchoMessage::success(format!("Generated {link} ({})", target.display()))
                }
                Some((_, link, Some(warning))) => {
                    EchoMessage::warning(format!("Generated {link}; warning: {warning}"))
                }
                None => EchoMessage::success(format!("Executed {language_name} source block")),
            }
        };
        editor.update(cx, |editor, cx| {
            editor.finish_source_run_feedback_at(source_block_start, SourceRunPhase::Success, cx)
        });
        self.show_echo_message(message, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ByteOffset, ByteRange, Selection, TextEdit, TextSnapshot};

    fn replace(
        session: &gpui::Entity<crate::document::DocumentSession>,
        source: &str,
        cx: &mut gpui::App,
    ) {
        session.update(cx, |session, cx| {
            session
                .edit(
                    DocumentCommand::new(
                        EditTransaction::new(
                            session.revision(),
                            vec![TextEdit::new(
                                ByteRange::new(0, session.snapshot().len_bytes()),
                                source,
                            )],
                        ),
                        Selection::default(),
                        Selection::default(),
                        EditOrigin::Other,
                    ),
                    cx,
                )
                .unwrap();
        });
    }

    #[gpui::test]
    fn call_results_work_in_a_draft_and_one_undo_restores_the_source(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(crate::editor::init);
        let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
        workspace.update(cx, |w, cx| {
            w.create_buffer("analysis.org".into(), None, cx);
            w.show_editor(cx);
            let session = w.document_session().unwrap().clone();
            let source = "#+name: double\n#+begin_src python :var n=2\nreturn n*2\n#+end_src\n\n#+CALL: double(n=6)\n\n#+RESULTS:\n: previous\n";
            replace(&session, source, cx);
            let snapshot = session.read(cx).snapshot();
            let plan = crate::babel::prepare_execution(&snapshot, session.read(cx).path(), Selection::caret(ByteOffset(source.find("#+CALL").unwrap() as u64)), Vec::new()).unwrap();
            assert!(w.babel_versions_current(&plan.versions, cx), "drafts use their open buffer, not a missing disk file");
            let output = crate::babel::execute_plan(plan).unwrap();
            let editor = w.editor(crate::app::PaneSide::Left).unwrap();
            w.apply_babel_result(session.clone(), editor, "Python", Ok(output), cx);
            let snapshot = session.read(cx).snapshot();
            assert!(snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())).contains("#+RESULTS:\n: 12\n"));
            session.update(cx, |s,cx| s.undo(cx).unwrap());
            let snapshot = session.read(cx).snapshot();
            assert_eq!(snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())), source);
        });
    }

    #[gpui::test]
    fn changing_an_open_unsaved_library_invalidates_the_whole_plan(cx: &mut gpui::TestAppContext) {
        cx.update(crate::editor::init);
        let directory = tempfile::tempdir().unwrap();
        let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
        workspace.update(cx, |w, cx| {
            w.create_buffer(
                "library.org".into(),
                Some(directory.path().join("library.org")),
                cx,
            );
            let library = w.document_session().unwrap().clone();
            replace(
                &library,
                "#+name: double\n#+begin_src python :var n=2\nreturn n*2\n#+end_src\n",
                cx,
            );
            w.create_buffer(
                "analysis.org".into(),
                Some(directory.path().join("analysis.org")),
                cx,
            );
            let caller = w.document_session().unwrap().clone();
            replace(&caller, "#+CALL: library.org:double(n=6)\n", cx);
            let open = w
                .buffer_sessions()
                .map(|s| (s.read(cx).path().to_owned(), s.read(cx).snapshot()))
                .collect();
            let plan = crate::babel::prepare_execution(
                &caller.read(cx).snapshot(),
                caller.read(cx).path(),
                Selection::caret(ByteOffset(0)),
                open,
            )
            .unwrap();
            assert!(w.babel_versions_current(&plan.versions, cx));
            replace(&library, "Changed\n", cx);
            assert!(!w.babel_versions_current(&plan.versions, cx));
            assert!(!directory.path().join("library.org").exists());
        });
    }
}
