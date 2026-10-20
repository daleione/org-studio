use super::*;
use crate::document::TextSnapshot;
use gpui::Window;

impl WorkspaceWindow {
    pub(crate) fn buffer_review_capture_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(review) = self.buffers.review() else {
            return false;
        };
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if review.inline {
            if !self.file_overview_is_open()
                || !review
                    .close_focus
                    .as_ref()
                    .is_some_and(|focuses| focuses.iter().any(|focus| focus.is_focused(window)))
            {
                return false;
            }
            // Navigation withdraws only an unsubmitted card confirmation.
            if modifiers.platform && key.eq_ignore_ascii_case("n")
                || modifiers.control && key == "x"
            {
                if !review.running {
                    self.cancel_buffer_panel(cx);
                }
                return false;
            }
        }
        if !review.running {
            self.buffer_review_key(event, window, cx);
        } else if !matches!(review.kind, ReviewKind::Close(_)) && modifiers.control && key == "g" {
            self.cancel_buffer_panel(cx);
        }
        cx.stop_propagation();
        true
    }

    fn buffer_review_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let m = event.keystroke.modifiers;
        if let Some(focuses) = self.buffers.review().and_then(|r| r.close_focus.as_ref()) {
            let index = focuses
                .iter()
                .position(|f| f.is_focused(window))
                .unwrap_or(focuses.len() - 1);
            if key == "escape" || m.control && key == "g" {
                self.close_review_action(CloseAction::Cancel, window, cx);
            } else if key == "tab" {
                let step = if m.shift { focuses.len() - 1 } else { 1 };
                window.focus(&focuses[(index + step) % focuses.len()], cx);
            } else if matches!(key, "enter" | "space") {
                self.close_review_action(CloseAction::ALL[index], window, cx);
            }
            return;
        }
        if key == "escape" || m.control && key == "g" {
            self.cancel_buffer_panel(cx);
        } else if key == "enter" {
            self.process_buffer_review(window, cx);
        } else if let Some(r) = self.buffers.review_mut()
            && !r.entries.is_empty()
        {
            let len = r.entries.len();
            match key {
                "up" => r.selected = (r.selected + len - 1) % len,
                "tab" if m.shift => r.selected = (r.selected + len - 1) % len,
                "down" | "tab" => r.selected = (r.selected + 1) % len,
                "p" if m.control => r.selected = (r.selected + len - 1) % len,
                "n" if m.control => r.selected = (r.selected + 1) % len,
                "space" if !r.entries[r.selected].done => {
                    r.entries[r.selected].save = !r.entries[r.selected].save
                }
                _ => {}
            }
            let selected = r.selected;
            self.buffers.scroll.scroll_to_item(selected);
            cx.notify();
        }
    }
    pub(crate) fn request_close_buffer(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        if self.buffer_busy() {
            return;
        }
        let Some(session) = self.buffer_session(id, cx) else {
            return;
        };
        let document = session.read(cx);
        let empty_draft = document.is_draft() && document.snapshot().len_bytes() == 0;
        if document.is_dirty() && !empty_draft
            || !matches!(document.save_state(), crate::document::SaveState::Idle)
        {
            self.begin_buffer_review(ReviewKind::Close(id), cx);
        } else {
            self.remove_buffer(id, cx);
            if matches!(&self.buffers.panel, Some(Panel::Picker(p)) if p.intent == PickerIntent::Close)
            {
                self.cancel_buffer_panel(cx);
            }
        }
    }

    pub(crate) fn begin_buffer_review(&mut self, kind: ReviewKind, cx: &mut Context<Self>) {
        self.cancel_overview_close(cx);
        if self.buffers.review().is_some() {
            return;
        }
        self.end_prefix(cx);
        self.close_search(false, cx);
        self.dismiss_buffer_panel(cx);
        self.cancel_pending_document_focus(cx);
        let entries = self
            .buffer_sessions()
            .filter_map(|s| {
                let s = s.read(cx);
                if matches!(kind, ReviewKind::Close(id) if id != s.id())
                    || !s.is_dirty() && matches!(s.save_state(), crate::document::SaveState::Idle)
                {
                    return None;
                }
                Some(ReviewEntry {
                    id: s.id(),
                    revision: s.revision(),
                    save: true,
                    done: false,
                })
            })
            .collect();
        self.buffers.panel = Some(Panel::Review(SaveReview {
            kind,
            entries,
            running: false,
            error: None,
            selected: 0,
            inline: matches!(kind, ReviewKind::Close(_)) && self.file_overview_is_open(),
            close_focus: matches!(kind, ReviewKind::Close(_))
                .then(|| std::array::from_fn(|_| cx.focus_handle())),
        }));
        self.buffers.pane = self.document_workspace.active_pane;
        self.buffers.returning = false;
        self.buffers.focus_pending = true;
        cx.notify();
    }

    pub(super) fn close_review_action(
        &mut self,
        action: CloseAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(review) = self.buffers.review() else {
            return;
        };
        let ReviewKind::Close(id) = review.kind else {
            return;
        };
        if review.running
            || !matches!(
                self.save.interaction,
                crate::app::save::SaveInteraction::Idle
            )
        {
            return;
        }
        match action {
            CloseAction::Discard => self.discard_buffer_review(window, cx),
            CloseAction::Cancel => self.cancel_buffer_panel(cx),
            CloseAction::Save => {
                self.set_buffer_review_choice(id, true, cx);
                self.process_buffer_review(window, cx);
            }
        }
    }

    pub(crate) fn set_buffer_review_choice(
        &mut self,
        id: DocumentId,
        save: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(review) = self.buffers.review_mut()
            && !review.running
            && let Some((index, entry)) = review
                .entries
                .iter_mut()
                .enumerate()
                .find(|(_, entry)| entry.id == id && !entry.done)
        {
            entry.save = save;
            review.selected = index;
            cx.notify();
        }
    }

    pub(crate) fn discard_buffer_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(
            self.save.interaction,
            crate::app::save::SaveInteraction::Idle
        ) {
            return;
        }
        let Some(review) = self.buffers.review_mut() else {
            return;
        };
        if review.running || review.kind == ReviewKind::Save {
            return;
        }
        for entry in &mut review.entries {
            if !entry.done {
                entry.save = false;
            }
        }
        // Keep captured revisions and the existing final validation. This path skips
        // saving, but must not discard a newer edit or an unreviewed hidden document.
        self.process_buffer_review(window, cx);
    }

    pub(crate) fn process_buffer_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(
            self.save.interaction,
            crate::app::save::SaveInteraction::Idle
        ) {
            return;
        }
        let Some(review) = self.buffers.review_mut() else {
            return;
        };
        review.running = true;
        review.error = None;
        let next = review
            .entries
            .iter()
            .find(|e| e.save && !e.done)
            .map(|e| e.id);
        if let Some(id) = next {
            if let Some(session) = self.buffer_session(id, cx) {
                self.save_buffer(session, false, window, cx);
                return;
            }
            self.fail_buffer_review(
                self.buffer_text(
                    "文档已关闭，请取消后重试",
                    "Document closed; cancel and retry",
                )
                .to_owned(),
                cx,
            );
            return;
        }
        let review = self.buffers.review().unwrap();
        // Decisions are bound to a revision, including discard decisions. A later edit is never
        // silently covered by an earlier confirmation, even when it came from a background task.
        let changed = review.entries.iter().any(|e| {
            self.buffer_session(e.id, cx).is_some_and(|s| {
                let s = s.read(cx);
                if e.save {
                    s.is_dirty()
                } else {
                    s.revision() != e.revision
                }
            })
        });
        let new_dirty = matches!(review.kind, ReviewKind::Quit | ReviewKind::Window)
            && self.buffer_sessions().any(|s| {
                let s = s.read(cx);
                s.is_dirty() && !review.entries.iter().any(|e| e.id == s.id())
            });
        if changed || new_dirty {
            self.fail_buffer_review(
                self.buffer_text(
                    "保存期间文档有新修改，请取消并重新审阅",
                    "Documents changed while saving; cancel and review again",
                )
                .to_owned(),
                cx,
            );
            return;
        }
        let kind = review.kind;
        self.buffers.review_mut().unwrap().running = false;
        self.dismiss_buffer_panel(cx);
        match kind {
            ReviewKind::Save => self.request_document_focus(cx),
            ReviewKind::Close(id) => {
                self.remove_buffer(id, cx);
                self.focus_file_overview(cx);
            }
            ReviewKind::Quit => {
                self.save.interaction = crate::app::save::SaveInteraction::AllowCloseOnce;
                cx.quit();
            }
            ReviewKind::Window => {
                self.save.interaction = crate::app::save::SaveInteraction::AllowCloseOnce;
                window.remove_window();
            }
        }
    }

    pub(crate) fn fail_buffer_review(&mut self, error: String, cx: &mut Context<Self>) {
        if let Some(review) = self.buffers.review_mut() {
            review.running = false;
            review.error = Some(error);
            self.buffers.focus_pending = true;
        }
        cx.notify();
    }
}
