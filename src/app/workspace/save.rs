use crate::{
    app::{
        WorkspaceWindow,
        buffers::{ReviewKind, SaveReview},
        save::{AfterRename, SaveInteraction},
    },
    document::{
        DocumentSession, SaveError, SaveRequest, SaveStartError, TargetExpectation, write_atomic,
    },
};
use gpui::{AppContext, Context, Entity, PromptButton, PromptLevel, Window};
use std::{future::Future, path::PathBuf, pin::Pin};

impl WorkspaceWindow {
    pub(crate) fn save_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.document_name.is_open()
            && !self.confirm_document_name(AfterRename::Save, window, cx)
        {
            return;
        }
        if self.generated_command_disposition(crate::editor::GeneratedCommand::Save)
            == crate::editor::CommandDisposition::Disabled
        {
            return;
        }
        if let Some(session) = self.document_session().cloned() {
            self.save_buffer(session, false, window, cx);
        }
    }
    pub(crate) fn save_document_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.document_name.is_open()
            && !self.confirm_document_name(AfterRename::SaveAs, window, cx)
        {
            return;
        }
        if self.generated_command_disposition(crate::editor::GeneratedCommand::Save)
            == crate::editor::CommandDisposition::Disabled
        {
            return;
        }
        if let Some(session) = self.document_session().cloned() {
            self.save_buffer(session, true, window, cx);
        }
    }
    pub(crate) fn save_buffer(
        &mut self,
        session: Entity<DocumentSession>,
        save_as: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.save.interaction, SaveInteraction::Idle) {
            return;
        }
        if save_as || session.read(cx).file_path().is_none() {
            self.prompt_buffer_path(session, save_as, window, cx);
            return;
        }
        match self.start_buffer_save(session.clone(), None, false, window, cx) {
            Ok(()) => {}
            Err(SaveStartError::Conflict) => self.prompt_buffer_conflict(session, window, cx),
            Err(error) => self.save_failed(
                format!(
                    "{}: {error:?}",
                    self.buffer_text("无法保存", "Could not save")
                ),
                cx,
            ),
        }
    }

    /// Rename only the file identity. Unsaved text remains in the same session;
    /// a Save/Save As request continues after the rename has completed.
    pub(crate) fn rename_document(
        &mut self,
        document: Entity<DocumentSession>,
        target: PathBuf,
        after: AfterRename,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.save.interaction, SaveInteraction::Idle) {
            return;
        }
        if self
            .buffer_for_path(&target, cx)
            .is_some_and(|other| other != document)
        {
            self.document_name_failed(
                self.buffer_text(
                    "目标文件已经打开，请使用其他名称",
                    "Target file is already open; choose another name",
                )
                .to_owned(),
                cx,
            );
            return;
        }
        let source = document.read(cx).path().to_path_buf();
        self.save.interaction = SaveInteraction::Renaming {
            document: document.read(cx).id(),
            target: target.clone(),
        };
        let operation = cx.background_spawn({
            let source = source.clone();
            let target = target.clone();
            async move { crate::file_manager::rename_exclusive(&source, &target) }
        });
        self.save.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = operation.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.save.task = None;
                this.save.interaction = SaveInteraction::Idle;
                let result = result.and_then(|()| {
                    document.update(cx, |doc, cx| {
                        doc.finish_file_rename(&source, target.clone(), cx)
                    })
                });
                match result {
                    Ok(()) => {
                        this.recent_documents.retain(|d| d.path != source);
                        crate::recent_documents::record_success(&mut this.recent_documents, target);
                        this.invalidate_file_overview_frames();
                        if this.document_session() == Some(&document) {
                            this.sync_document_watch(cx);
                        }
                        if this.document_name.is_for(&document) {
                            this.cancel_document_name(cx);
                        }
                        match after {
                            AfterRename::Nothing => {}
                            AfterRename::Save => this.save_buffer(document, false, window, cx),
                            AfterRename::SaveAs => this.save_buffer(document, true, window, cx),
                        }
                    }
                    Err(error) => {
                        let message = if error.kind() == std::io::ErrorKind::AlreadyExists {
                            this.buffer_text(
                                "已有同名文件，请使用其他名称",
                                "A file with this name already exists; choose another name",
                            )
                            .to_owned()
                        } else {
                            format!(
                                "{}: {error}",
                                this.buffer_text("无法重命名", "Could not rename")
                            )
                        };
                        if this.document_name.is_for(&document) {
                            this.document_name_failed(message, cx);
                        } else {
                            this.save_failed(message, cx);
                        }
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn prompt_buffer_path(
        &mut self,
        session: Entity<DocumentSession>,
        save_as: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let directory = session
            .read(cx)
            .path()
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Documents")))
            .unwrap_or_else(|| PathBuf::from("."));
        let name = session.read(cx).display_name();
        let suggested = if std::path::Path::new(&name).extension().is_none() {
            let extension = session
                .read(cx)
                .syntax_path()
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("org");
            format!("{name}.{extension}")
        } else {
            name
        };
        let directory_only = !save_as && session.read(cx).is_named_draft();
        let picker: Pin<Box<dyn Future<Output = Option<PathBuf>>>> = if directory_only {
            let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some(
                    format!(
                        "{}“{suggested}”{}",
                        self.buffer_text("选择", "Choose a folder for "),
                        self.buffer_text("的保存位置", "")
                    )
                    .into(),
                ),
            });
            Box::pin(async move {
                let Ok(Ok(Some(paths))) = picker.await else {
                    return None;
                };
                paths
                    .into_iter()
                    .next()
                    .map(|folder| folder.join(suggested))
            })
        } else {
            let picker = cx.prompt_for_new_path(&directory, Some(&suggested));
            Box::pin(async move {
                let Ok(Ok(path)) = picker.await else {
                    return None;
                };
                path
            })
        };
        let from_review = self.buffer_busy();
        self.save.interaction = SaveInteraction::Prompt;
        self.save.dialog_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = picker.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.save.dialog_task = None;
                this.save.interaction = SaveInteraction::Idle;
                // Saves are serialized: a replacement review cannot be running while this
                // prompt is open. Cancellation revokes the pending write, not completed writes.
                if from_review && !this.buffer_busy() {
                    cx.notify();
                    return;
                }
                let Some(path) = result else {
                    if let Some(r) = this.buffers.review_mut() {
                        r.running = false;
                        this.buffers.focus_pending = true;
                    }
                    cx.notify();
                    return;
                };
                if this
                    .buffer_for_path(&path, cx)
                    .is_some_and(|other| other.entity_id() != session.entity_id())
                {
                    this.save_failed(
                        this.buffer_text(
                            "目标文件已经打开，请选择其他路径",
                            "Target file is already open; choose another path",
                        )
                        .to_owned(),
                        cx,
                    );
                    return;
                }
                let request = session.update(cx, |doc, _| doc.begin_save(Some(path)));
                match request {
                    Ok(mut request) => {
                        // A folder choice does not grant permission to replace a
                        // same-named file. Check this again in the background write.
                        if directory_only {
                            request.expected_target = TargetExpectation::Exact(None);
                        }
                        this.write_buffer_save(session, request, window, cx);
                    }
                    Err(error) => this.save_failed(
                        format!(
                            "{}: {error:?}",
                            this.buffer_text("无法保存", "Could not save")
                        ),
                        cx,
                    ),
                }
            });
        }));
    }
    fn prompt_buffer_conflict(
        &mut self,
        session: Entity<DocumentSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = window.prompt(
            PromptLevel::Warning,
            self.buffer_text("文件已被其他程序修改", "The file changed on disk"),
            Some(self.buffer_text(
                "覆盖磁盘版本、另存为，或取消。",
                "Overwrite the disk version, save under another name, or cancel.",
            )),
            &[
                PromptButton::ok(self.buffer_text("覆盖", "Overwrite")),
                PromptButton::new(self.buffer_text("另存为…", "Save As…")),
                PromptButton::cancel(self.buffer_text("取消", "Cancel")),
            ],
            cx,
        );
        let from_review = self.buffer_busy();
        self.save.interaction = SaveInteraction::Prompt;
        self.save.dialog_task = Some(cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await.ok();
            let _ = this.update_in(cx, |this, window, cx| {
                this.save.dialog_task = None;
                this.save.interaction = SaveInteraction::Idle;
                if from_review && !this.buffer_busy() {
                    cx.notify();
                    return;
                }
                match answer {
                    Some(0) => {
                        if let Err(error) = this.start_buffer_save(session, None, true, window, cx)
                        {
                            this.save_failed(
                                format!(
                                    "{}: {error:?}",
                                    this.buffer_text("无法保存", "Could not save")
                                ),
                                cx,
                            );
                        }
                    }
                    Some(1) => this.prompt_buffer_path(session, true, window, cx),
                    _ => {
                        if let Some(r) = this.buffers.review_mut() {
                            r.running = false;
                        }
                        cx.notify();
                    }
                }
            });
        }));
    }
    fn start_buffer_save(
        &mut self,
        session: Entity<DocumentSession>,
        target: Option<PathBuf>,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), SaveStartError> {
        let request = session.update(cx, |s, _| {
            if force {
                s.begin_force_save()
            } else {
                s.begin_save(target)
            }
        })?;
        self.write_buffer_save(session, request, window, cx);
        Ok(())
    }

    fn write_buffer_save(
        &mut self,
        session: Entity<DocumentSession>,
        request: SaveRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let revision = request.revision();
        let from_review = self.buffer_busy();
        self.save.error = None;
        self.save.interaction = SaveInteraction::Saving;
        let background = cx
            .background_executor()
            .spawn(async move { write_atomic(request) });
        self.save.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = background.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.save.task = None;
                this.save.interaction = SaveInteraction::Idle;
                match result {
                    Ok(outcome) => {
                        let path = outcome.target_path().to_path_buf();
                        let warning = outcome.warning().cloned();
                        if let Err(error) = session.update(cx, |s, cx| s.finish_save(outcome, cx)) {
                            session.update(cx, |s, _| s.cancel_save(revision));
                            this.save_failed(
                                format!(
                                    "{}: {error:?}",
                                    this.buffer_text(
                                        "保存结果未应用",
                                        "Save result was not applied"
                                    )
                                ),
                                cx,
                            );
                            return;
                        }
                        crate::recent_documents::record_success(&mut this.recent_documents, path);
                        if this
                            .document_session()
                            .is_some_and(|s| s.entity_id() == session.entity_id())
                        {
                            this.sync_document_watch(cx);
                        }
                        if let Some(warning) = warning {
                            this.save_failed(warning.to_string(), cx);
                            return;
                        }
                        let id = session.read(cx).id();
                        this.save.error = None;
                        if from_review
                            && let Some(review) = this.buffers.review_mut()
                            && review.running
                        {
                            if let Some(entry) = review.entries.iter_mut().find(|e| e.id == id) {
                                entry.done = !session.read(cx).is_dirty();
                                entry.revision = revision;
                            }
                            if session.read(cx).is_dirty() {
                                this.fail_buffer_review(
                                    this.buffer_text(
                                        "文档有新修改，请重新审阅",
                                        "Document has new edits; review again",
                                    )
                                    .to_owned(),
                                    cx,
                                );
                            } else {
                                this.process_buffer_review(window, cx);
                            }
                        }
                    }
                    Err(error) => {
                        session.update(cx, |s, _| {
                            s.cancel_save(revision);
                            if let SaveError::Conflict { external, .. } = &error {
                                s.observe_disk(external.clone());
                            }
                        });
                        let message = if session.read(cx).is_draft()
                            && matches!(error, SaveError::Conflict { .. })
                        {
                            this.buffer_text(
                                "已有同名文件，请选择其他保存位置",
                                "A file with this name already exists; choose another folder",
                            )
                            .to_owned()
                        } else {
                            format!("{}: {error}", this.buffer_text("保存失败", "Save failed"))
                        };
                        this.save_failed(message, cx);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn save_failed(&mut self, message: String, cx: &mut Context<Self>) {
        self.save.error = Some(message.clone().into());
        if self.buffer_busy() {
            self.fail_buffer_review(message, cx);
        }
        cx.notify();
    }
    pub(crate) fn request_home(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if !self.buffer_busy() {
            self.show_home_now(cx);
        }
    }
    pub(crate) fn request_quit(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.exit_needs_save_review(cx) {
            self.begin_buffer_review(ReviewKind::Quit, cx);
        } else {
            cx.quit();
        }
    }
    fn exit_needs_save_review(&self, cx: &gpui::App) -> bool {
        !matches!(self.save.interaction, SaveInteraction::Idle)
            || self
                .buffer_sessions()
                .any(|s| SaveReview::required_for(s.read(cx)))
    }
    pub(crate) fn install_close_guard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.save.close_hook_installed {
            return;
        }
        self.save.close_hook_installed = true;
        window.on_window_should_close(cx, |window, cx| {
            let Some(workspace) = window.root::<WorkspaceWindow>().flatten() else {
                return true;
            };
            workspace.update(cx, |w, cx| {
                if matches!(w.save.interaction, SaveInteraction::AllowCloseOnce) {
                    w.save.interaction = SaveInteraction::Idle;
                    return true;
                }
                if w.exit_needs_save_review(cx) {
                    w.begin_buffer_review(ReviewKind::Window, cx);
                    false
                } else {
                    true
                }
            })
        });
    }
}
