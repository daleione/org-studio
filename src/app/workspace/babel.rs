use gpui::{Context, Focusable, Window};

use crate::{
    app::{ContentRoute, PaneSurface, WorkspaceWindow, echo_area::EchoMessage},
    document::{DocumentCommand, EditOrigin, EditTransaction},
    editor::SourceRunPhase,
};

impl WorkspaceWindow {
    pub(crate) fn tangle_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        self.begin_babel_request(cx);
        let request =
            match crate::babel::prepare_source_block_execution(&snapshot, &path, selection) {
                Ok(request) => request,
                Err(message) => {
                    editor.update(cx, |editor, cx| {
                        editor.finish_source_run_feedback(SourceRunPhase::Failure, cx)
                    });
                    self.show_echo_message(EchoMessage::error(message), cx);
                    return;
                }
            };

        if let Some(program) = request.external_program() {
            let trusted_path = std::fs::canonicalize(&path).unwrap_or(path);
            if !self.babel_trusted_documents.contains(&trusted_path) {
                let answer = window.prompt(
                    gpui::PromptLevel::Warning,
                    &format!("Run {} code from this document?", request.language_name()),
                    Some(&format!(
                        "This runs {program} in {} and may change files.",
                        trusted_path.parent().unwrap_or(&trusted_path).display()
                    )),
                    &[
                        gpui::PromptButton::ok("Run Code"),
                        gpui::PromptButton::cancel("Cancel"),
                    ],
                    cx,
                );
                let request_id = self.babel_request;
                self.babel_task = Some(cx.spawn_in(window, async move |this, cx| {
                    let Ok(0) = answer.await else { return };
                    let _ = this.update_in(cx, |this, window, cx| {
                        if this.babel_request != request_id
                            || !this.document_session().is_some_and(|current| {
                                current == &session
                                    && current.read(cx).revision() == request.revision
                            })
                        {
                            return;
                        }
                        this.babel_trusted_documents.insert(trusted_path);
                        this.launch_source_execution(request, session, editor, window, cx);
                    });
                }));
                return;
            }
        }
        self.launch_source_execution(request, session, editor, window, cx);
    }

    fn launch_source_execution(
        &mut self,
        request: crate::babel::BabelExecutionRequest,
        session: gpui::Entity<crate::document::DocumentSession>,
        editor: gpui::Entity<crate::editor::SemanticEditor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source_block_start = request.source_block_start;
        editor.update(cx, |editor, cx| {
            editor.show_source_run_feedback(source_block_start, SourceRunPhase::Running, cx)
        });
        self.babel_editor = Some(editor.clone());
        let request_id = self.babel_request;
        let language_name = request.language_name();
        self.set_echo_message(Some(EchoMessage::working(format!(
            "Executing {language_name} source block…"
        ))));
        let background = cx
            .background_executor()
            .spawn(async move { crate::babel::execute_source_block(request) });
        self.babel_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = background.await;
            let _ = this.update_in(cx, |this, _window, cx| {
                if this.babel_request != request_id {
                    return;
                }
                this.babel_task = None;
                this.babel_editor = None;
                this.apply_babel_result(session, editor, language_name, result, cx);
            });
        }));
        cx.notify();
    }

    fn begin_babel_request(&mut self, cx: &mut Context<Self>) -> u64 {
        self.babel_task = None;
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
        let source_block_start = output.source_block_start;
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
                        selection,
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
        let message = match file {
            Some((target, link, None)) => {
                EchoMessage::success(format!("Generated {link} ({})", target.display()))
            }
            Some((_, link, Some(warning))) => {
                EchoMessage::warning(format!("Generated {link}; warning: {warning}"))
            }
            None => EchoMessage::success(format!("Executed {language_name} source block")),
        };
        editor.update(cx, |editor, cx| {
            editor.finish_source_run_feedback_at(source_block_start, SourceRunPhase::Success, cx)
        });
        self.show_echo_message(message, cx);
    }
}
