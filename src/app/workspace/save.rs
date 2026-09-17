use crate::{
    app::{WorkspaceWindow, buffers::ReviewKind, save::SaveInteraction},
    document::{DocumentSession, SaveError, SaveStartError, write_atomic},
};
use gpui::{Context, Entity, PromptButton, PromptLevel, Window};
use std::{path::PathBuf, sync::Arc};

impl WorkspaceWindow {
    pub(crate) fn save_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            self.prompt_buffer_path(session, window, cx);
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
    fn prompt_buffer_path(
        &mut self,
        session: Entity<DocumentSession>,
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
            format!("{name}.org")
        } else {
            name
        };
        let picker = cx.prompt_for_new_path(&directory, Some(&suggested));
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
                let Ok(Ok(Some(path))) = result else {
                    if let Some(r) = this.buffers.review_mut() {
                        r.running = false;
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
                if let Err(error) = this.start_buffer_save(session, Some(path), false, window, cx) {
                    this.save_failed(
                        format!(
                            "{}: {error:?}",
                            this.buffer_text("无法保存", "Could not save")
                        ),
                        cx,
                    );
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
                    Some(1) => this.prompt_buffer_path(session, window, cx),
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
                        this.save_failed(
                            format!("{}: {error}", this.buffer_text("保存失败", "Save failed")),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
        Ok(())
    }
    fn save_failed(&mut self, message: String, cx: &mut Context<Self>) {
        self.save.error = Some(message.clone().into());
        if self.buffer_busy() {
            self.fail_buffer_review(message, cx);
        }
        cx.notify();
    }
    pub(crate) fn request_open_at(
        &mut self,
        path: PathBuf,
        anchor: Arc<str>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open(path, cx);
        self.pending_navigation = Some((self.generation, anchor));
        self.apply_pending_navigation(self.generation, cx);
    }
    pub(crate) fn request_home(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if !self.buffer_busy() {
            self.show_home_now(cx);
        }
    }
    pub(crate) fn request_quit(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.buffer_sessions().any(|s| s.read(cx).is_dirty())
            || !matches!(self.save.interaction, SaveInteraction::Idle)
        {
            self.begin_buffer_review(ReviewKind::Quit, cx);
        } else {
            cx.quit();
        }
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
                if w.buffer_sessions().any(|s| s.read(cx).is_dirty())
                    || !matches!(w.save.interaction, SaveInteraction::Idle)
                {
                    w.begin_buffer_review(ReviewKind::Window, cx);
                    false
                } else {
                    true
                }
            })
        });
    }
}
