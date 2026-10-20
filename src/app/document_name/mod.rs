//! Title naming UI. File transactions belong to the workspace persistence adapter.
mod view;

use super::{
    ContentRoute, WorkspaceLoadState, WorkspaceWindow,
    save::{AfterRename, SaveInteraction},
};
use crate::{
    components::native_input::{InputConfig, InputEvent, NativeInput},
    document::{DocumentNameError, DocumentSession},
};
use gpui::{App, AppContext, Context, Entity, FocusHandle, KeyDownEvent, Subscription, Window};

#[derive(Default)]
pub(super) struct NameHost {
    session: Option<Session>,
    pub(super) anchor: gpui::Bounds<gpui::Pixels>,
}

impl NameHost {
    pub(super) fn is_open(&self) -> bool {
        self.session.is_some()
    }

    pub(super) fn is_for(&self, document: &Entity<DocumentSession>) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| &s.document == document)
    }
}

struct Session {
    document: Entity<DocumentSession>,
    input: Entity<NativeInput>,
    extension: String,
    cancel_focus: FocusHandle,
    done_focus: FocusHandle,
    error: Option<String>,
    _subscription: Subscription,
}

impl WorkspaceWindow {
    pub(super) fn document_name_available(&self, cx: &App) -> bool {
        self.document_name_context_available(cx)
            && matches!(self.save.interaction, SaveInteraction::Idle)
    }

    fn document_name_context_available(&self, cx: &App) -> bool {
        self.content_route == ContentRoute::Document
            && matches!(self.state, WorkspaceLoadState::Ready { .. })
            && !self.file_overview_is_open()
            && !self.export.is_open()
            && !self.buffer_busy()
            && !self.file_manager.operation_busy()
            && self
                .document_session()
                .is_some_and(|doc| !doc.read(cx).is_generated())
    }

    pub(super) fn open_document_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.document_name.session.is_some() {
            self.cancel_document_name(cx);
            return;
        }
        if !self.document_name_available(cx) {
            return;
        }
        let document = self.document_session().unwrap().clone();
        let (name, extension) = {
            let doc = document.read(cx);
            let name = doc.display_name();
            let extension = doc
                .syntax_path()
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_owned();
            let stem = if extension.is_empty() {
                name.clone()
            } else {
                name.strip_suffix(&format!(".{extension}"))
                    .unwrap_or(&name)
                    .to_owned()
            };
            (stem, extension)
        };
        self.close_search(false, cx);
        self.close_command_line(cx);
        self.dismiss_buffer_panel(cx);
        self.end_prefix(cx);
        self.cancel_pending_document_focus(cx);
        self.status.dismiss_popover();
        let input = cx.new(|cx| {
            let mut input = NativeInput::new(
                InputConfig {
                    id: "document-name-input",
                    outlined: true,
                    font_size: 13.,
                    ..Default::default()
                },
                cx,
            );
            input.sync(&name, cx);
            input.selection = 0..name.len();
            input
        });
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            match event {
                InputEvent::Command { key, .. } if key == "enter" => {
                    this.confirm_document_name(AfterRename::Nothing, window, cx);
                }
                InputEvent::Command { key, .. } if key == "escape" => this.cancel_document_name(cx),
                InputEvent::Changed(_) => {
                    if let Some(prompt) = &mut this.document_name.session {
                        prompt.error = None;
                        prompt.input.update(cx, |input, cx| {
                            input.invalid = false;
                            cx.notify();
                        });
                    }
                }
                _ => {}
            }
            cx.notify();
        });
        let focus = input.read(cx).focus.clone();
        window.focus(&focus, cx);
        self.document_name.session = Some(Session {
            document,
            input,
            extension,
            cancel_focus: cx.focus_handle(),
            done_focus: cx.focus_handle(),
            error: None,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn document_name_tick(&mut self, cx: &mut Context<Self>) {
        if self.document_name.session.as_ref().is_some_and(|prompt| {
            !self.document_name_context_available(cx)
                || !matches!(self.save.interaction, SaveInteraction::Idle)
                    && !self.document_name_busy()
                || self.document_session() != Some(&prompt.document)
                || self.search_is_open()
                || self.command_line_is_open()
                || self.buffers.panel.is_some()
        }) {
            // A new document or another tool owns focus now. Do not revive the
            // previous editor or carry its unconfirmed name into that document.
            self.document_name.session = None;
        }
    }

    pub(super) fn cancel_document_name(&mut self, cx: &mut Context<Self>) {
        if self.document_name_busy() {
            return;
        }
        let Some(prompt) = self.document_name.session.take() else {
            return;
        };
        if self.document_session() == Some(&prompt.document)
            && self.content_route == ContentRoute::Document
        {
            self.request_document_focus(cx);
        }
        cx.notify();
    }

    fn document_name_busy(&self) -> bool {
        matches!(self.save.interaction, SaveInteraction::Renaming { .. })
    }

    pub(super) fn confirm_document_name(
        &mut self,
        after: AfterRename,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(prompt) = self.document_name.session.as_ref() else {
            return true;
        };
        if !self.document_name_available(cx)
            || self.document_session() != Some(&prompt.document)
            || prompt.input.read(cx).is_composing()
        {
            return false;
        }
        let value = prompt.input.read(cx).text.clone();
        let document = prompt.document.clone();
        let result = if document.read(cx).is_draft() {
            document
                .update(cx, |doc, cx| doc.rename_draft(&value, cx))
                .map(|()| None)
        } else {
            let doc = document.read(cx);
            doc.rename_candidate(&value)
                .map(|name| Some(doc.path().with_file_name(name)))
        };
        match result {
            Ok(Some(target)) if target != document.read(cx).path() => {
                self.rename_document(document, target, after, window, cx);
                false
            }
            Ok(_) => {
                self.cancel_document_name(cx);
                true
            }
            Err(error) => {
                let message = match error {
                    DocumentNameError::Empty => self.buffer_text("请输入名称", "Enter a name"),
                    DocumentNameError::Invalid => self.buffer_text(
                        "名称不能包含路径或控制字符",
                        "Use a filename without path separators or control characters",
                    ),
                    DocumentNameError::DifferentFormat => self.buffer_text(
                        "命名不会更改文档格式，请保留当前后缀",
                        "Keep the current extension; naming does not change the format",
                    ),
                    DocumentNameError::NotDraft => {
                        self.buffer_text("当前文档已不是草稿", "This document is no longer a draft")
                    }
                };
                self.document_name_failed(message.to_owned(), cx);
                false
            }
        }
    }

    pub(super) fn document_name_failed(&mut self, error: String, cx: &mut Context<Self>) {
        let Some(prompt) = &mut self.document_name.session else {
            return;
        };
        prompt.error = Some(error);
        prompt.input.update(cx, |input, cx| {
            input.invalid = true;
            cx.notify();
        });
        cx.notify();
    }

    pub(super) fn document_name_capture_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(prompt) = self.document_name.session.as_ref() else {
            return false;
        };
        if self.document_name_busy() {
            cx.stop_propagation();
            return true;
        }
        if prompt.input.read(cx).is_composing() {
            return true;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if key == "enter" && prompt.done_focus.is_focused(window) {
            self.confirm_document_name(AfterRename::Nothing, window, cx);
            cx.stop_propagation();
            return true;
        }
        if key == "escape" || key == "g" && modifiers.control {
            self.cancel_document_name(cx);
            cx.stop_propagation();
        } else if key == "enter" && prompt.cancel_focus.is_focused(window) {
            self.cancel_document_name(cx);
            cx.stop_propagation();
        } else if key == "tab" {
            let focuses = [
                prompt.input.read(cx).focus.clone(),
                prompt.cancel_focus.clone(),
                prompt.done_focus.clone(),
            ];
            let index = focuses
                .iter()
                .position(|f| f.is_focused(window))
                .unwrap_or(0);
            window.focus(
                &focuses[(index + if modifiers.shift { 2 } else { 1 }) % 3],
                cx,
            );
            cx.stop_propagation();
        }
        true
    }
}

#[cfg(test)]
mod tests;
