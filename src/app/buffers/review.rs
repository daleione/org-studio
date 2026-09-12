use super::*;
use gpui::Window;

impl WorkspaceWindow {
    pub(crate) fn buffer_review_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let m = event.keystroke.modifiers;
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
        if session.read(cx).is_dirty()
            || !matches!(
                session.read(cx).save_state(),
                crate::document::SaveState::Idle
            )
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
        if self.buffers.review().is_some() {
            return;
        }
        self.end_prefix(cx);
        self.close_search(false, cx);
        self.dismiss_buffer_panel(cx);
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
        }));
        self.buffers.pane = self.document_workspace.active_pane;
        self.buffers.returning = false;
        self.buffers.focus_pending = true;
        cx.notify();
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
        self.dismiss_buffer_panel(cx);
        match kind {
            ReviewKind::Save => self.request_document_focus(cx),
            ReviewKind::Close(id) => self.remove_buffer(id, cx),
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
        }
        cx.notify();
    }
}
