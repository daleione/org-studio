use std::{path::PathBuf, sync::Arc};

use gpui::{AppContext, Context, PathPromptOptions, PromptLevel, Window};

use super::{ContentRoute, DiredStatus, WorkspaceWindow};
use crate::{
    file_manager::{ConflictPolicy, DiredSession, OperationPlan, execute_operation},
    navigation::NavigationCause,
};

impl WorkspaceWindow {
    pub(in crate::preview) fn dired_prepare_execute(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dired_operation_available(cx) {
            return;
        }
        let Some(plan) = self
            .file_manager
            .session
            .as_ref()
            .map(DiredSession::deletion_plan)
        else {
            return;
        };
        if plan.item_count() == 0 {
            self.show_dired_operation_error("No files are flagged for deletion", cx);
            return;
        }
        self.confirm_dired_trash(plan, window, cx);
    }

    pub(in crate::preview) fn dired_create_file(&mut self, cx: &mut Context<Self>) {
        if !self.dired_operation_available(cx) {
            return;
        }
        self.prompt_dired_creation(false, cx);
    }

    pub(in crate::preview) fn dired_create_directory(&mut self, cx: &mut Context<Self>) {
        if !self.dired_operation_available(cx) {
            return;
        }
        self.prompt_dired_creation(true, cx);
    }

    fn prompt_dired_creation(&mut self, directory: bool, cx: &mut Context<Self>) {
        let Some(parent) = self
            .file_manager
            .session
            .as_ref()
            .map(|session| session.directory().to_path_buf())
        else {
            return;
        };
        let suggested_name = if directory {
            "New Folder"
        } else {
            "untitled.org"
        };
        let prompt = cx.prompt_for_new_path(&parent, Some(suggested_name));
        self.file_manager.operation_task = Some(cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = prompt.await else {
                return;
            };
            let plan = if directory {
                OperationPlan::create_directory(path)
            } else {
                OperationPlan::create_file(path)
            };
            let _ = this.update(cx, |this, cx| this.execute_dired_operation(plan, cx));
        }));
    }

    pub(in crate::preview) fn dired_rename(&mut self, cx: &mut Context<Self>) {
        if !self.dired_operation_available(cx) {
            return;
        }
        let Some(targets) = self
            .file_manager
            .session
            .as_ref()
            .map(DiredSession::operation_targets)
        else {
            return;
        };
        if targets.len() != 1 {
            self.show_dired_operation_error("Rename requires exactly one selected item", cx);
            return;
        }
        let source = targets[0].clone();
        let Some(parent) = source.parent().map(PathBuf::from) else {
            return;
        };
        let prompt =
            cx.prompt_for_new_path(&parent, source.file_name().and_then(|name| name.to_str()));
        self.file_manager.operation_task = Some(cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(destination))) = prompt.await else {
                return;
            };
            let plan = OperationPlan::rename(source, destination);
            let _ = this.update(cx, |this, cx| this.execute_dired_operation(plan, cx));
        }));
    }

    pub(in crate::preview) fn dired_copy(&mut self, cx: &mut Context<Self>) {
        if !self.dired_operation_available(cx) {
            return;
        }
        self.prompt_dired_transfer(false, cx);
    }

    pub(in crate::preview) fn dired_move_to(&mut self, cx: &mut Context<Self>) {
        if !self.dired_operation_available(cx) {
            return;
        }
        self.prompt_dired_transfer(true, cx);
    }

    fn prompt_dired_transfer(&mut self, move_items: bool, cx: &mut Context<Self>) {
        let Some(targets) = self
            .file_manager
            .session
            .as_ref()
            .map(DiredSession::operation_targets)
        else {
            return;
        };
        if targets.is_empty() {
            self.show_dired_operation_error("No file selected", cx);
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(if move_items {
                "Move to Folder".into()
            } else {
                "Copy to Folder".into()
            }),
        });
        self.file_manager.operation_task = Some(cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            let Some(destination) = paths.into_iter().next() else {
                return;
            };
            let plan = if move_items {
                OperationPlan::move_to(targets, destination, ConflictPolicy::Error)
            } else {
                OperationPlan::copy(targets, destination, ConflictPolicy::Error)
            };
            let _ = this.update(cx, |this, cx| this.execute_dired_operation(plan, cx));
        }));
    }

    pub(in crate::preview) fn dired_trash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dired_operation_available(cx) {
            return;
        }
        let Some(targets) = self
            .file_manager
            .session
            .as_ref()
            .map(DiredSession::operation_targets)
        else {
            return;
        };
        if targets.is_empty() {
            self.show_dired_operation_error("No file selected", cx);
            return;
        }
        self.confirm_dired_trash(OperationPlan::trash(targets), window, cx);
    }

    fn confirm_dired_trash(
        &mut self,
        plan: OperationPlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let OperationPlan::Trash { sources } = &plan else {
            return;
        };
        let count = sources.len();
        let mut detail = sources
            .iter()
            .take(8)
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        if count > 8 {
            detail.push_str(&format!("\n…and {} more", count - 8));
        }
        let prompt = window.prompt(
            PromptLevel::Warning,
            &format!("Move {count} item(s) to Trash?"),
            Some(&detail),
            &["Move to Trash", "Cancel"],
            cx,
        );
        self.file_manager.operation_task = Some(cx.spawn(async move |this, cx| {
            let Ok(answer) = prompt.await else {
                return;
            };
            if answer == 0 {
                let _ = this.update(cx, |this, cx| this.execute_dired_operation(plan, cx));
            }
        }));
    }

    pub(in crate::preview) fn execute_dired_operation(
        &mut self,
        plan: OperationPlan,
        cx: &mut Context<Self>,
    ) {
        if !self.dired_operation_available(cx) {
            return;
        }
        self.file_manager.operation_busy = true;
        self.file_manager.status = Some(DiredStatus::Working(
            format!("Working on {} item(s)…", plan.item_count()).into(),
        ));
        cx.notify();
        let operation = cx.background_spawn(async move { execute_operation(&plan) });
        self.file_manager.operation_task = Some(cx.spawn(async move |this, cx| {
            let report = operation.await;
            let _ = this.update(cx, |this, cx| {
                this.file_manager.operation_busy = false;
                let current_document = this.document_path(cx).map(PathBuf::from);
                if let Some(session) = this.file_manager.session.as_mut() {
                    session.clear_completed_operations(&report.completed, &report.destinations);
                }
                this.refresh_file_manager(NavigationCause::FileSystemDelta, cx);
                this.file_manager.status = Some(if report.succeeded() {
                    DiredStatus::Success(report.summary())
                } else {
                    DiredStatus::Error(report.summary())
                });
                if this.content_route == ContentRoute::Document
                    && let Some(current_document) = current_document
                    && let Some((_, destination)) = report
                        .destinations
                        .iter()
                        .find(|(source, _)| source.as_ref() == current_document)
                    && crate::preview::is_supported_document(destination)
                {
                    this.open(destination.to_path_buf(), cx);
                }
                cx.notify();
            });
        }));
    }

    fn show_dired_operation_error(&mut self, message: impl Into<Arc<str>>, cx: &mut Context<Self>) {
        self.file_manager.status = Some(DiredStatus::Error(message.into()));
        cx.notify();
    }

    fn dired_operation_available(&mut self, cx: &mut Context<Self>) -> bool {
        if self.file_manager.operation_busy {
            self.show_dired_operation_error("A file operation is already in progress", cx);
            false
        } else {
            true
        }
    }
}
