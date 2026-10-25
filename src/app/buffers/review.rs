use super::*;
use crate::document::{SaveState, TextSnapshot};
use gpui::Window;

impl SaveReview {
    pub(crate) fn required_for(document: &DocumentSession) -> bool {
        if document.is_generated() {
            return false;
        }
        let empty_draft = document.is_draft() && document.snapshot().len_bytes() == 0;
        document.is_dirty() && !empty_draft || !matches!(document.save_state(), SaveState::Idle)
    }
}

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
                    .controls()
                    .iter()
                    .any(|&control| review.focus(control).is_focused(window))
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
        } else if !matches!(review.kind, ReviewKind::Close(_))
            && (key == "escape" || modifiers.control && key == "g")
        {
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
        if key == "escape" || m.control && key == "g" {
            self.cancel_buffer_panel(cx);
            return;
        }
        let Some(review) = self.buffers.review() else {
            return;
        };
        let control = review.focused_control(window);
        if key == "tab" {
            let controls = review.controls();
            let index = controls.iter().position(|&c| c == control).unwrap();
            let step = if m.shift { controls.len() - 1 } else { 1 };
            review
                .focus(controls[(index + step) % controls.len()])
                .focus(window, cx);
            cx.notify();
        } else if matches!(key, "enter" | "space") {
            if control == ReviewControl::List && key == "space" {
                if let Some(entry) = review.entries.get(review.selected) {
                    self.set_buffer_review_choice(entry.id, !entry.save, cx);
                }
            } else {
                self.buffer_review_action(control, window, cx);
            }
        } else if control == ReviewControl::List
            && let Some(r) = self.buffers.review_mut()
            && !r.entries.is_empty()
        {
            let len = r.entries.len();
            match key {
                "up" => r.selected = (r.selected + len - 1) % len,
                "down" => r.selected = (r.selected + 1) % len,
                "p" if m.control => r.selected = (r.selected + len - 1) % len,
                "n" if m.control => r.selected = (r.selected + 1) % len,
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
        if SaveReview::required_for(document) {
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
        self.close_command_line(cx);
        self.end_prefix(cx);
        self.close_search(false, cx);
        self.dismiss_buffer_panel(cx);
        self.cancel_pending_document_focus(cx);
        let entries = self
            .buffer_sessions()
            .filter_map(|s| {
                let s = s.read(cx);
                if matches!(kind, ReviewKind::Close(id) if id != s.id())
                    || !SaveReview::required_for(s)
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
            focus: std::array::from_fn(|_| cx.focus_handle()),
        }));
        self.buffers.pane = self.document_workspace.active_pane;
        self.buffers.outgoing = None;
        self.buffers.scroll = gpui::ScrollHandle::default();
        self.buffers.focus_pending = true;
        cx.notify();
    }

    pub(super) fn buffer_review_action(
        &mut self,
        control: ReviewControl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(review) = self.buffers.review() else {
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
        review.focus(control).focus(window, cx);
        match control {
            ReviewControl::Discard => self.discard_buffer_review(window, cx),
            ReviewControl::Submit => {
                if review.single_confirmation() {
                    let id = review.entries[0].id;
                    self.set_buffer_review_choice(id, true, cx);
                }
                self.process_buffer_review(window, cx);
            }
            ReviewControl::All => self.toggle_buffer_review_all(cx),
            ReviewControl::List => {
                if let Some(entry) = review.entries.get(review.selected) {
                    self.view_buffer_review_document(entry.id, window, cx);
                }
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
            if review.destructive_submit() {
                self.buffers.focus_pending = true;
            }
            cx.notify();
        }
    }

    fn toggle_buffer_review_all(&mut self, cx: &mut Context<Self>) {
        let Some(review) = self.buffers.review_mut().filter(|r| !r.running) else {
            return;
        };
        let save = review.selection_counts().1 > 0;
        for entry in &mut review.entries {
            if !entry.done {
                entry.save = save;
            }
        }
        if review.destructive_submit() {
            self.buffers.focus_pending = true;
        }
        cx.notify();
    }

    pub(super) fn view_buffer_review_document(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .buffers
            .review()
            .is_some_and(|r| !r.running && r.entries.iter().any(|e| e.id == id))
            || !matches!(
                self.save.interaction,
                crate::app::save::SaveInteraction::Idle
            )
        {
            return;
        }
        self.cancel_buffer_panel(cx);
        if self.file_overview_is_open() {
            self.close_file_overview_now(window, cx);
        }
        self.activate_buffer(id, cx);
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
                SaveReview::required_for(s) && !review.entries.iter().any(|e| e.id == s.id())
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
