use std::path::{Path, PathBuf};

use crate::{
    app::{ContentRoute, WorkspaceWindow},
    babel::ExecutionPlan,
    document::{DocumentId, DocumentSession, Revision},
};
use gpui::{Context, Entity, FocusHandle, Focusable, Window};

mod view;

pub(crate) struct CodeConfirmation {
    document: DocumentId,
    revision: Revision,
    document_name: String,
    document_path: PathBuf,
    origin: PathBuf,
    name: Option<String>,
    language: &'static str,
    directory: PathBuf,
    preview: Vec<String>,
    line_count: usize,
    dependencies: Vec<Dependency>,
    ask_each_time: bool,
    has_scripts: bool,
    writes_result: bool,
    focuses: [FocusHandle; 2],
    answer: async_channel::Sender<bool>,
}

struct Dependency {
    name: String,
    detail: String,
    hint: String,
    count: usize,
}

impl WorkspaceWindow {
    pub(crate) fn prompt_code_execution(
        &mut self,
        plan: &ExecutionPlan,
        session: &Entity<DocumentSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> async_channel::Receiver<bool> {
        let (answer, receiver) = async_channel::bounded(1);
        let requests = plan.requests();
        let mut dependencies: Vec<Dependency> = Vec::new();
        for (path, request) in requests.iter().take(requests.len().saturating_sub(1)) {
            let name = request
                .block_name()
                .unwrap_or(request.language_name())
                .to_owned();
            let detail =
                if let Some(target) = request.mysql.as_ref().and_then(|mysql| mysql.target()) {
                    let database = if target.database.is_empty() {
                        self.buffer_text("未指定数据库", "No default database")
                    } else {
                        &target.database
                    };
                    format!(
                        "MySQL · {} · {}@{}:{}",
                        database, target.user, target.host, target.port
                    )
                } else {
                    format!(
                        "{} · {}",
                        request.language_name(),
                        path.file_name().unwrap_or_default().to_string_lossy()
                    )
                };
            // The complete origin is available on hover, without repeating it in the dialog.
            let hint = format!("{}\n{detail}", path.display());
            if let Some(dependency) = dependencies
                .iter_mut()
                .find(|d| d.name == name && d.hint == hint)
            {
                dependency.count += 1;
            } else {
                dependencies.push(Dependency {
                    name,
                    detail,
                    hint,
                    count: 1,
                });
            }
        }
        let source = plan.source_preview();
        let preview = source
            .lines()
            .take(5)
            .map(|line| {
                let text = line.chars().take(160).collect::<String>();
                if text.len() < line.len() {
                    format!("{text}…")
                } else {
                    text
                }
            })
            .collect();
        let document = session.read(cx);
        let prompt = CodeConfirmation {
            document: document.id(),
            revision: document.revision(),
            document_name: document.display_name(),
            document_path: document.path().to_owned(),
            origin: requests
                .last()
                .map(|(path, _)| path.to_path_buf())
                .unwrap_or_else(|| document.path().to_owned()),
            name: plan.block_name().map(str::to_owned),
            language: plan.language_name(),
            directory: plan.execution_directory().to_owned(),
            preview,
            line_count: source.lines().count(),
            dependencies,
            ask_each_time: requests.iter().any(|(_, r)| r.headers_eval_query()),
            has_scripts: requests.iter().any(|(_, r)| r.external_program().is_some()),
            writes_result: plan.writes_result(),
            focuses: std::array::from_fn(|_| cx.focus_handle()),
            answer,
        };
        self.cancel_pending_document_focus(cx);
        window.focus(&prompt.focuses[1], cx);
        self.babel_confirmation = Some(prompt);
        cx.notify();
        receiver
    }

    fn answer_code_confirmation(
        &mut self,
        approved: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.babel_confirmation.take() else {
            return;
        };
        let _ = prompt.answer.try_send(approved);
        if self.content_route == ContentRoute::Document
            && let Some(editor) = self.editor(self.document_workspace.active_pane)
        {
            window.focus(&editor.read(cx).focus_handle(cx), cx);
        }
        cx.notify();
    }

    pub(crate) fn code_confirmation_capture_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(prompt) = self.babel_confirmation.as_ref() else {
            return false;
        };
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if key == "escape" || modifiers.control && key == "g" {
            self.answer_code_confirmation(false, window, cx);
        } else if key == "tab" {
            let index = usize::from(prompt.focuses[0].is_focused(window));
            window.focus(&prompt.focuses[index], cx);
        } else if matches!(key, "enter" | "space") {
            self.answer_code_confirmation(prompt.focuses[1].is_focused(window), window, cx);
        }
        cx.stop_propagation();
        true
    }
}

fn short_directory(directory: &Path, document: &Path, current_label: &'static str) -> String {
    let parent = document.parent().unwrap_or(Path::new("."));
    let same = directory == parent
        || std::fs::canonicalize(directory)
            .ok()
            .zip(std::fs::canonicalize(parent).ok())
            .is_some_and(|(a, b)| a == b);
    if same {
        current_label.to_owned()
    } else {
        directory
            .file_name()
            .unwrap_or(directory.as_os_str())
            .to_string_lossy()
            .into_owned()
    }
}
