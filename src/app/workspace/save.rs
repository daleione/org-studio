use std::{path::PathBuf, sync::Arc};

use gpui::{Context, PromptButton, PromptLevel, Window};

use crate::document::{SaveError, SaveStartError, write_atomic};
use crate::{
    app::WorkspaceWindow,
    app::save::{PendingTransition, SaveInteraction, SaveStatus},
};

impl WorkspaceWindow {
    pub(crate) fn save_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.generated_command_disposition(crate::editor::GeneratedCommand::Save)
            == crate::editor::CommandDisposition::Disabled
        {
            return;
        }
        self.save_document_then(None, window, cx);
    }

    fn save_document_then(
        &mut self,
        transition: Option<PendingTransition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.save.interaction, SaveInteraction::Idle) {
            return;
        }
        match self.start_save(None, false, transition.clone(), window, cx) {
            Ok(()) | Err(SaveStartError::AlreadySaving | SaveStartError::ReadOnly) => {}
            Err(SaveStartError::Conflict) => self.prompt_conflict_save(transition, window, cx),
        }
    }

    pub(crate) fn save_document_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.generated_command_disposition(crate::editor::GeneratedCommand::Save)
            == crate::editor::CommandDisposition::Disabled
        {
            return;
        }
        self.save_document_as_then(None, window, cx);
    }

    fn save_document_as_then(
        &mut self,
        transition: Option<PendingTransition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.save.interaction, SaveInteraction::Idle) {
            return;
        }
        let Some(session) = self.document_session() else {
            return;
        };
        let path = session.read(cx).path();
        let directory = path.parent().unwrap_or_else(|| std::path::Path::new("."));
        let suggested = path.file_name().and_then(|name| name.to_str());
        let picker = cx.prompt_for_new_path(directory, suggested);
        self.save.interaction = SaveInteraction::SaveAsPrompt(transition);
        self.save.dialog_task = Some(cx.spawn_in(window, async move |this, cx| {
            let selection = picker.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.save.dialog_task = None;
                let SaveInteraction::SaveAsPrompt(transition) =
                    std::mem::take(&mut this.save.interaction)
                else {
                    return;
                };
                if let Ok(Ok(Some(path))) = selection
                    && let Err(error) = this.start_save(Some(path), false, transition, window, cx)
                {
                    this.save.status = Some(SaveStatus::Error(
                        format!("Could not start Save As: {error:?}").into(),
                    ));
                    this.save.interaction = SaveInteraction::Idle;
                }
                cx.notify();
            });
        }));
    }

    fn prompt_conflict_save(
        &mut self,
        transition: Option<PendingTransition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.save.interaction, SaveInteraction::Idle) {
            return;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            "The file changed on disk",
            Some("Overwrite the disk version, save your edits under another name, or cancel."),
            &[
                PromptButton::ok("Overwrite"),
                PromptButton::new("Save As…"),
                PromptButton::cancel("Cancel"),
            ],
            cx,
        );
        self.save.interaction = SaveInteraction::ConflictPrompt(transition);
        self.save.dialog_task = Some(cx.spawn_in(window, async move |this, cx| {
            let choice = answer.await.ok();
            let _ = this.update_in(cx, |this, window, cx| {
                this.save.dialog_task = None;
                let SaveInteraction::ConflictPrompt(transition) =
                    std::mem::take(&mut this.save.interaction)
                else {
                    return;
                };
                match choice {
                    Some(0) => {
                        let _ = this.start_save(None, true, transition, window, cx);
                    }
                    Some(1) => this.save_document_as_then(transition, window, cx),
                    _ => {}
                }
            });
        }));
    }

    fn start_save(
        &mut self,
        target: Option<PathBuf>,
        force: bool,
        transition: Option<PendingTransition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), SaveStartError> {
        let Some(session) = self.document_session().cloned() else {
            return Ok(());
        };
        let request = session.update(cx, |session, _| {
            if force {
                session.begin_force_save()
            } else {
                session.begin_save(target)
            }
        })?;
        let revision = request.revision();
        self.save.status = Some(SaveStatus::Saving);
        self.save.interaction = SaveInteraction::Saving(transition);
        self.set_document_notice(None);
        let background = cx
            .background_executor()
            .spawn(async move { write_atomic(request) });
        self.save.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = background.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.apply_save_result(session, revision, result, window, cx);
            });
        }));
        cx.notify();
        Ok(())
    }

    fn apply_save_result(
        &mut self,
        session: gpui::Entity<crate::document::DocumentSession>,
        revision: crate::document::Revision,
        result: Result<crate::document::SaveOutcome, SaveError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save.task = None;
        let transition = match std::mem::take(&mut self.save.interaction) {
            SaveInteraction::Saving(transition) => transition,
            _ => None,
        };
        match result {
            Ok(outcome) => {
                let path = outcome.target_path().to_path_buf();
                let warning = outcome.warning().cloned();
                match session.update(cx, |session, cx| session.finish_save(outcome, cx)) {
                    Ok(_) => {
                        crate::recent_documents::record_success(
                            &mut self.recent_documents,
                            path.clone(),
                        );
                        self.watch_document_profiled(path.clone(), self.generation, cx);
                        if let Some(warning) = warning {
                            let message: Arc<str> =
                                format!("Saved {}, but {warning}", path.display()).into();
                            self.save.status = Some(SaveStatus::Error(message.clone()));
                            self.set_document_notice(Some(message));
                        } else {
                            self.save.status = None;
                            self.set_document_notice(None);
                            if let Some(transition) = transition {
                                self.complete_transition(transition, false, window, cx);
                            }
                        }
                    }
                    Err(error) => {
                        session.update(cx, |session, _| session.cancel_save(revision));
                        let message: Arc<str> =
                            format!("Save result was not applied: {error:?}").into();
                        self.save.status = Some(SaveStatus::Error(message.clone()));
                        self.set_document_notice(Some(message));
                    }
                }
            }
            Err(error) => {
                session.update(cx, |session, _| {
                    session.cancel_save(revision);
                    if let SaveError::Conflict { external, .. } = &error {
                        session.observe_disk(external.clone());
                    }
                });
                let message: Arc<str> = format!("Save failed: {error}").into();
                self.save.status = Some(SaveStatus::Error(message.clone()));
                self.set_document_notice(Some(message));
            }
        }
        cx.notify();
    }

    fn complete_transition(
        &mut self,
        transition: PendingTransition,
        discard_current: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match transition {
            PendingTransition::Close => {
                self.save.interaction = SaveInteraction::AllowCloseOnce;
                window.remove_window();
            }
            PendingTransition::Quit => {
                if discard_current {
                    self.save.interaction = SaveInteraction::AllowCloseOnce;
                }
                cx.quit();
            }
            PendingTransition::Open { path, anchor } => {
                if discard_current {
                    self.open_discarding_current(path, cx);
                } else {
                    self.open(path, cx);
                }
                self.pending_navigation = anchor.map(|anchor| (self.generation, anchor));
            }
            PendingTransition::Home => self.show_home_now(cx),
        }
    }

    pub fn request_open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.request_transition(PendingTransition::Open { path, anchor: None }, window, cx);
    }

    pub(crate) fn request_open_at(
        &mut self,
        path: PathBuf,
        anchor: Arc<str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_transition(
            PendingTransition::Open {
                path,
                anchor: Some(anchor),
            },
            window,
            cx,
        );
    }

    pub(crate) fn request_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.request_transition(PendingTransition::Home, window, cx);
    }

    pub(crate) fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.request_transition(PendingTransition::Quit, window, cx);
    }

    fn request_transition(
        &mut self,
        transition: PendingTransition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &mut self.save.interaction {
            SaveInteraction::Idle => {}
            SaveInteraction::Saving(queued) => {
                if queued.is_none() {
                    *queued = Some(transition);
                }
                return;
            }
            SaveInteraction::GuardPrompt(_)
            | SaveInteraction::ConflictPrompt(_)
            | SaveInteraction::SaveAsPrompt(_)
            | SaveInteraction::AllowCloseOnce => return,
        }
        let Some(session) = self.document_session() else {
            self.complete_transition(transition, false, window, cx);
            return;
        };
        if !matches!(
            session.read(cx).save_state(),
            crate::document::SaveState::Idle
        ) {
            return;
        }
        if !session.read(cx).is_dirty() {
            self.complete_transition(transition, false, window, cx);
            return;
        }

        let file_name = session
            .read(cx)
            .path()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Save changes to {file_name}?"),
            Some("Your unsaved changes will be lost if you discard them."),
            &[
                PromptButton::ok("Save"),
                PromptButton::new("Discard"),
                PromptButton::cancel("Cancel"),
            ],
            cx,
        );
        self.save.interaction = SaveInteraction::GuardPrompt(transition);
        self.save.dialog_task = Some(cx.spawn_in(window, async move |this, cx| {
            let choice = answer.await.ok();
            let _ = this.update_in(cx, |this, window, cx| {
                this.save.dialog_task = None;
                let SaveInteraction::GuardPrompt(transition) =
                    std::mem::take(&mut this.save.interaction)
                else {
                    return;
                };
                match choice {
                    Some(0) => this.save_document_then(Some(transition), window, cx),
                    Some(1) => this.complete_transition(transition, true, window, cx),
                    _ => {
                        // A generated-view source jump is tied to this prompt.  Cancelling must
                        // invalidate it so a later save cannot unexpectedly perform the jump.
                        this.agenda.pending_text_task = None;
                        this.agenda.pending_text_generation = None;
                    }
                }
            });
        }));
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
            workspace.update(cx, |workspace, cx| {
                if matches!(workspace.save.interaction, SaveInteraction::AllowCloseOnce) {
                    workspace.save.interaction = SaveInteraction::Idle;
                    return true;
                }
                let needs_guard =
                    workspace.document_session().is_some_and(|session| {
                        let session = session.read(cx);
                        session.is_dirty()
                            || !matches!(session.save_state(), crate::document::SaveState::Idle)
                    }) || !matches!(workspace.save.interaction, SaveInteraction::Idle);
                if needs_guard {
                    workspace.request_transition(PendingTransition::Close, window, cx);
                    false
                } else {
                    true
                }
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ByteRange, EditTransaction, TextEdit};

    #[gpui::test]
    fn dirty_window_close_is_guarded_and_cancel_keeps_the_session(cx: &mut gpui::TestAppContext) {
        let path = std::env::temp_dir().join(format!(
            "org-studio-close-guard-{}-{}.org",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"base").unwrap();
        let loaded = crate::preview::load_workspace_document(path.clone(), false).unwrap();
        let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
        workspace.update(cx, |workspace, cx| {
            workspace.generation = 1;
            assert!(workspace.apply_load_result(1, Ok(loaded), cx));
            let session = workspace.document_session().unwrap().clone();
            session.update(cx, |session, cx| {
                session
                    .apply_transient_edit(
                        EditTransaction::new(
                            session.revision(),
                            vec![TextEdit::new(ByteRange::new(4, 4), " local")],
                        ),
                        cx,
                    )
                    .unwrap();
            });
        });
        cx.update(|window, app| {
            workspace.update(app, |workspace, cx| {
                workspace.install_close_guard(window, cx)
            });
        });

        assert!(!cx.simulate_close());
        assert!(cx.has_pending_prompt());
        assert!(!cx.simulate_close());
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert!(cx.cx.read(|app| {
            workspace
                .read(app)
                .document_session()
                .is_some_and(|session| session.read(app).is_dirty())
        }));
        std::fs::remove_file(path).unwrap();
    }
}
