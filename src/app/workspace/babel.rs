use gpui::{Context, Window};

use crate::{
    app::{PaneSurface, WorkspaceWindow, echo_area::EchoMessage},
    document::{DocumentCommand, EditOrigin, EditTransaction},
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
        let Some(session) = self.document_session().cloned() else {
            return;
        };
        let selection = editor.read(cx).selection();
        let (snapshot, path) = {
            let session = session.read(cx);
            (session.snapshot(), session.path().to_path_buf())
        };
        self.babel_request = self.babel_request.wrapping_add(1);
        let request =
            match crate::babel::prepare_source_block_execution(&snapshot, &path, selection) {
                Ok(request) => request,
                Err(message) => {
                    self.show_echo_message(EchoMessage::error(message), cx);
                    return;
                }
            };

        let request_id = self.babel_request;
        self.set_echo_message(Some(EchoMessage::working(
            "Executing PlantUML source block…",
        )));
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
                this.apply_babel_result(session, result, cx);
            });
        }));
        cx.notify();
    }

    fn apply_babel_result(
        &mut self,
        session: gpui::Entity<crate::document::DocumentSession>,
        result: Result<crate::babel::PreparedBabelOutput, String>,
        cx: &mut Context<Self>,
    ) {
        let mut output = match result {
            Ok(output) => output,
            Err(message) => {
                self.show_echo_message(
                    EchoMessage::error(format!("PlantUML failed: {message}")),
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
            self.show_echo_message(
                EchoMessage::warning("PlantUML result was discarded because the source changed"),
                cx,
            );
            return;
        }
        output = match output.publish() {
            Ok(output) => output,
            Err(message) => {
                self.show_echo_message(EchoMessage::error(message), cx);
                return;
            }
        };
        let target = output.target.to_string_lossy().into_owned();
        let result_link = output.result_link.clone();
        let warning = output.warnings.first().cloned();
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
                self.show_echo_message(
                    EchoMessage::warning(
                        "The image was written, but #+RESULTS could not be updated",
                    ),
                    cx,
                );
                return;
            }
        }

        let message = warning.map_or_else(
            || EchoMessage::success(format!("Generated {result_link} ({target})")),
            |warning| EchoMessage::warning(format!("Generated {result_link}; warning: {warning}")),
        );
        self.show_echo_message(message, cx);
    }
}
