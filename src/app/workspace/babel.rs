use gpui::{Context, Focusable, Window};

use crate::{
    app::{PaneSurface, WorkspaceWindow, echo_area::EchoMessage},
    document::{DocumentCommand, EditOrigin, EditTransaction},
    editor::SourceRunPhase,
};

impl WorkspaceWindow {
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
        self.babel_task = None;
        if let Some(previous_editor) = self.babel_editor.take() {
            previous_editor.update(cx, |editor, cx| {
                editor.clear_source_run_feedback();
                cx.notify();
            });
        }
        self.babel_request = self.babel_request.wrapping_add(1);
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
        let target = output.target.to_string_lossy().into_owned();
        let result_link = output.result_link.clone();
        let warning = output.warnings.first().cloned();
        session.update(cx, |session, cx| {
            session.resource_changed(output.target.clone(), cx)
        });
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
                self.show_echo_message(
                    EchoMessage::warning(format!(
                        "The {language_name} output was written, but #+RESULTS could not be updated"
                    )),
                    cx,
                );
                return;
            }
        }
        let message = warning.map_or_else(
            || EchoMessage::success(format!("Generated {result_link} ({target})")),
            |warning| EchoMessage::warning(format!("Generated {result_link}; warning: {warning}")),
        );
        editor.update(cx, |editor, cx| {
            editor.finish_source_run_feedback_at(source_block_start, SourceRunPhase::Success, cx)
        });
        self.show_echo_message(message, cx);
    }
}
