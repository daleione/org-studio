use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui::{AppContext, Context, PathPromptOptions, PromptLevel, Window};

use super::{ContentRoute, DiredStatus, WorkspaceWindow};
use crate::{
    file_manager::{
        ConflictPolicy, DiredSession, OperationPlan, OperationReport, execute_operation,
    },
    navigation::NavigationCause,
};

impl WorkspaceWindow {
    fn file_targets_include_root(&self, sources: &[Arc<Path>]) -> bool {
        self.content_route == ContentRoute::Document
            && self
                .file_manager
                .tree
                .model
                .root()
                .is_some_and(|root| sources.iter().any(|source| root.starts_with(source)))
    }
    fn file_operation_summary(&self, report: &OperationReport) -> Arc<str> {
        let mut message = self
            .language
            .text("files.completed")
            .replace("{count}", &report.completed.len().to_string());
        if !report.skipped.is_empty() {
            message.push_str(&format!(
                " · {}",
                self.language
                    .text("files.skipped")
                    .replace("{count}", &report.skipped.len().to_string())
            ));
        }
        if let Some(failure) = report.failures.first() {
            message.push_str(&format!(
                " · {}: {} — {}",
                self.language
                    .text("files.failed")
                    .replace("{count}", &report.failures.len().to_string()),
                failure.path.display(),
                failure.message
            ));
        }
        message.into()
    }
    pub(super) fn file_operation_targets(&self) -> Arc<[Arc<Path>]> {
        if self.content_route == ContentRoute::Document && self.file_manager.sidebar_focused {
            self.file_manager
                .tree
                .menu
                .as_ref()
                .map(|menu| menu.targets.clone())
                .unwrap_or_else(|| self.file_manager.tree.model.operation_targets())
        } else {
            self.file_manager
                .session
                .as_ref()
                .map(DiredSession::operation_targets)
                .unwrap_or_else(|| Arc::from([]))
        }
    }

    pub(crate) fn dired_prepare_execute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(crate) fn dired_create_file(&mut self, cx: &mut Context<Self>) {
        if !self.dired_operation_available(cx) {
            return;
        }
        self.prompt_dired_creation(false, cx);
    }

    pub(crate) fn dired_create_directory(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn dired_rename(&mut self, cx: &mut Context<Self>) {
        if !self.dired_operation_available(cx) {
            return;
        }
        let targets = self.file_operation_targets();
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

    pub(crate) fn dired_copy(&mut self, cx: &mut Context<Self>) {
        if !self.dired_operation_available(cx) {
            return;
        }
        self.prompt_dired_transfer(false, cx);
    }

    pub(crate) fn dired_move_to(&mut self, cx: &mut Context<Self>) {
        if !self.dired_operation_available(cx) {
            return;
        }
        self.prompt_dired_transfer(true, cx);
    }

    fn prompt_dired_transfer(&mut self, move_items: bool, cx: &mut Context<Self>) {
        let targets = self.file_operation_targets();
        if targets.is_empty() {
            self.show_dired_operation_error(self.language.text("files.no_selection"), cx);
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

    pub(crate) fn dired_trash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dired_operation_available(cx) {
            return;
        }
        let targets = self.file_operation_targets();
        if targets.is_empty() {
            self.show_dired_operation_error(self.language.text("files.no_selection"), cx);
            return;
        }
        if self.file_targets_include_root(&targets) {
            self.show_dired_operation_error(self.language.text("files.root_protected"), cx);
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
            detail.push_str(&format!(
                "\n{}",
                self.language
                    .text("files.more_items")
                    .replace("{count}", &(count - 8).to_string())
            ));
        }
        let dirty = self
            .buffer_sessions()
            .filter(|session| {
                let document = session.read(cx);
                document.is_dirty()
                    && document
                        .file_path()
                        .is_some_and(|path| sources.iter().any(|source| path.starts_with(source)))
            })
            .count();
        if dirty > 0 {
            detail.push_str(&format!(
                "\n\n{}",
                self.language
                    .text("files.trash_unsaved")
                    .replace("{count}", &dirty.to_string())
            ));
        }
        let prompt = window.prompt(
            PromptLevel::Warning,
            &self
                .language
                .text("files.confirm_trash")
                .replace("{count}", &count.to_string()),
            Some(&detail),
            &[
                self.language.text("files.move_to_trash"),
                self.language.text("files.cancel"),
            ],
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

    pub(crate) fn execute_dired_operation(&mut self, plan: OperationPlan, cx: &mut Context<Self>) {
        if !self.dired_operation_available(cx) {
            return;
        }
        let destructive_sources: &[Arc<Path>] = match &plan {
            OperationPlan::Rename { source, .. } => std::slice::from_ref(source),
            OperationPlan::Move { sources, .. } | OperationPlan::Trash { sources } => sources,
            _ => &[],
        };
        if self.file_targets_include_root(destructive_sources) {
            self.show_dired_operation_error(self.language.text("files.root_protected"), cx);
            return;
        }
        self.file_manager.operation_busy = true;
        self.file_manager.status = Some(DiredStatus::Working(
            self.language.text("files.working").into(),
        ));
        cx.notify();
        let moving = matches!(
            &plan,
            OperationPlan::Rename { .. } | OperationPlan::Move { .. }
        );
        let documents = if moving {
            self.buffer_sessions()
                .filter_map(|session| {
                    let document = session.read(cx);
                    Some((document.id(), document.file_path()?.to_path_buf()))
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let edited_name = self
            .file_manager
            .tree
            .edit
            .as_ref()
            .map(|edit| edit.input.clone());
        let created = match &plan {
            OperationPlan::CreateFile { path } | OperationPlan::CreateDirectory { path } => {
                Some(path.clone())
            }
            OperationPlan::Rename { destination, .. } => Some(destination.clone()),
            _ => None,
        };
        let operation = cx.background_spawn(async move {
            let report = execute_operation(&plan);
            let retargets = documents
                .into_iter()
                .filter_map(|(id, path)| {
                    report
                        .destinations
                        .iter()
                        .find_map(|(source, destination)| {
                            let relative = path.strip_prefix(source).ok()?;
                            let destination = destination.join(relative);
                            let stamp = crate::document::FileStamp::read(&destination);
                            Some((id, path.clone(), destination, stamp))
                        })
                })
                .collect::<Vec<_>>();
            (report, retargets)
        });
        self.file_manager.operation_task = Some(cx.spawn(async move |this, cx| {
            let (report, retargets) = operation.await;
            let _ = this.update(cx, |this, cx| {
                let summary = this.file_operation_summary(&report);
                this.file_manager.operation_busy = false;
                if let Some(session) = this.file_manager.session.as_mut() {
                    session.clear_completed_operations(&report.completed, &report.destinations);
                }
                let mut coordination_error = None;
                let mut reload_active = false;
                for (id, source, destination, observed) in retargets {
                    if let Some(document) = this.buffer_session(id, cx) {
                        let stamp = observed.as_ref().ok().cloned();
                        if let Err(error) = document.update(cx, |session, cx| {
                            session.finish_file_move(&source, destination.clone(), stamp, cx)
                        }) {
                            coordination_error = Some(format!(
                                "{}: {error}",
                                this.language.text("files.follow_move_failed")
                            ));
                        } else {
                            let action = match observed {
                                Ok(stamp) => Some(
                                    document
                                        .update(cx, |session, _| session.observe_disk(Some(stamp))),
                                ),
                                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(
                                    document.update(cx, |session, _| session.observe_disk(None)),
                                ),
                                Err(error) => {
                                    coordination_error = Some(format!(
                                        "{}: {error}",
                                        this.language.text("files.follow_move_failed")
                                    ));
                                    None
                                }
                            };
                            reload_active |= action
                                == Some(crate::document::DiskChangeAction::Reload)
                                && this.document_session() == Some(&document);
                            this.recent_documents.retain(|d| d.path != source);
                            crate::recent_documents::record_success(
                                &mut this.recent_documents,
                                destination,
                            );
                        }
                    }
                }
                for source in report.completed.iter() {
                    if let Some(parent) = source.parent() {
                        this.file_manager.directories.invalidate(parent);
                    }
                }
                for (source, destination) in report.destinations.iter() {
                    if moving {
                        this.file_manager.tree.model.relocate(source, destination);
                    }
                    if let Some(parent) = destination.parent() {
                        this.file_manager.directories.invalidate(parent);
                    }
                }
                if let Some(input) = edited_name
                    && this
                        .file_manager
                        .tree
                        .edit
                        .as_ref()
                        .is_some_and(|edit| edit.input == input)
                {
                    if report.succeeded() {
                        this.file_manager.tree.edit = None;
                        this.file_manager.tree.focus_requested = this.file_manager.sidebar_focused;
                        if let Some(path) = created {
                            this.file_manager.tree.model.reveal(&path);
                        }
                    } else {
                        if let Some(edit) = &mut this.file_manager.tree.edit {
                            edit.error = Some(summary.to_string());
                        }
                        input.update(cx, |input, cx| {
                            input.enabled = true;
                            cx.notify();
                        });
                    }
                }
                this.rebuild_file_tree(cx);
                this.sync_file_directory_watches(cx);
                if this.content_route == ContentRoute::FileManager {
                    this.refresh_file_manager(NavigationCause::FileSystemDelta, cx);
                }
                this.invalidate_file_overview_frames();
                this.sync_document_watch(cx);
                if reload_active {
                    this.reload_current(cx);
                }
                this.file_manager.status = Some(if let Some(error) = coordination_error {
                    DiredStatus::Error(error.into())
                } else if report.succeeded() {
                    DiredStatus::Success(summary)
                } else {
                    DiredStatus::Error(summary)
                });
                cx.notify();
            });
        }));
    }

    pub(super) fn show_dired_operation_error(
        &mut self,
        message: impl Into<Arc<str>>,
        cx: &mut Context<Self>,
    ) {
        self.file_manager.status = Some(DiredStatus::Error(message.into()));
        cx.notify();
    }

    pub(super) fn dired_operation_available(&mut self, cx: &mut Context<Self>) -> bool {
        if self.file_manager.operation_busy
            || self.buffer_busy()
            || !matches!(
                self.save.interaction,
                crate::app::save::SaveInteraction::Idle
            )
            || self.buffer_sessions().any(|session| {
                !matches!(
                    session.read(cx).save_state(),
                    crate::document::SaveState::Idle
                )
            })
        {
            self.show_dired_operation_error(self.language.text("files.operation_busy"), cx);
            false
        } else {
            true
        }
    }
}
