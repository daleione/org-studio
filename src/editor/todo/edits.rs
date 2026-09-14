//! State changes and native Org directive customization.
use super::*;

impl SemanticEditor {
    pub(super) fn apply_todo(&mut self, state: Option<&str>, cx: &mut Context<Self>) {
        let Some(popup) = self.todo.popup.take() else {
            return;
        };
        self.todo.dismissed = Some(popup.hit.range);
        self.todo.hover_task = None;
        self.todo.dismiss_task = None;
        let snapshot = self.snapshot(cx);
        if self.is_read_only(cx)
            || snapshot.revision() != popup.revision
            || snapshot.copy_range(popup.hit.range) != popup.hit.keyword.as_ref()
        {
            self.command_feedback = Some(self.ui_language.text("todo.source_changed").into());
            cx.notify();
            return;
        }
        if state == Some(popup.hit.keyword.as_ref()) {
            cx.notify();
            return;
        }
        // Only states from the revision-coherent menu may be written.
        if state.is_some_and(|s| {
            self.todo
                .config
                .as_ref()
                .is_none_or(|(r, c)| *r != popup.revision || c.todo_state(s).is_none())
        }) {
            cx.notify();
            return;
        }
        self.finish_composition(cx);
        let range = if state.is_some() {
            popup.hit.range
        } else {
            ByteRange::new(popup.hit.range.start.0, popup.hit.remove_end.0)
        };
        let replacement = state.unwrap_or("");
        // Keep both endpoints attached to the same text, including reversed selections.
        let map = |offset: ByteOffset| {
            let value = if offset.0 <= range.start.0 {
                offset.0
            } else if offset.0 >= range.end.0 {
                offset.0 - range.len() + replacement.len() as u64
            } else {
                let mut local = ((offset.0 - range.start.0) as usize).min(replacement.len());
                while !replacement.is_char_boundary(local) {
                    local -= 1;
                }
                range.start.0 + local as u64
            };
            ByteOffset(value)
        };
        let after = Selection::new(map(self.selection.anchor()), map(self.selection.head()));
        let result = self.session.update(cx, |session, cx| {
            session.edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        popup.revision,
                        vec![TextEdit::new(range, replacement.to_owned())],
                    ),
                    self.selection,
                    after,
                    EditOrigin::Other,
                ),
                cx,
            )
        });
        if result.is_ok() {
            self.selection = after;
            self.sync_selection_revision(cx);
            self.sync_selection_utf16(&self.snapshot(cx));
            self.hit_rows = Arc::from([]);
            self.pending_reveal_caret = false;
            self.command_feedback = None;
        } else {
            self.command_feedback = Some(self.ui_language.text("todo.failed").into());
        }
        cx.notify();
    }
    pub(super) fn customize_todo(&mut self, cx: &mut Context<Self>) {
        let Some(popup) = self.todo.popup.as_ref() else {
            return;
        };
        let revision = popup.revision;
        let keyword = popup.hit.keyword.clone();
        let snapshot = self.snapshot(cx);
        self.dismiss_todo(cx);
        if self.is_read_only(cx) || snapshot.revision() != revision {
            return;
        }
        // Use the file's native Org directive as the customization surface. Scan off-thread.
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let target = executor
                .spawn(async move {
                    let mut first = None;
                    let mut lines = crate::document::LineCursor::within(
                        &snapshot,
                        ByteRange::new(0, snapshot.len_bytes()),
                    )?;
                    while let Some(line) = lines.next_line() {
                        if let Some(sequence) =
                            crate::org_semantic::parse_todo_directive(line.text.trim_start())
                        {
                            let start = line.text.find(':')? + 1;
                            let end = line.text.trim_end_matches(['\r', '\n']).len();
                            let start = start + line.text[start..end].len()
                                - line.text[start..end].trim_start().len();
                            let range = ByteRange::new(
                                line.range.start.0 + start as u64,
                                line.range.start.0 + end as u64,
                            );
                            first.get_or_insert(range);
                            if sequence.states.iter().any(|s| s.keyword == keyword) {
                                return Some(range);
                            }
                        }
                    }
                    first
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.is_read_only(cx) || this.snapshot(cx).revision() != revision {
                    return;
                }
                if let Some(range) = target {
                    this.selection = Selection::new(range.start, range.end);
                    this.sync_selection_revision(cx);
                    this.sync_selection_utf16(&this.snapshot(cx));
                } else {
                    let newline = this.session.read(cx).newline_sequence();
                    let directive = "#+TODO: TODO | DONE";
                    let text = format!("{directive}{newline}");
                    let after = Selection::new(ByteOffset(8), ByteOffset(directive.len() as u64));
                    let result = this.session.update(cx, |session, cx| {
                        session.edit(
                            DocumentCommand::new(
                                EditTransaction::new(
                                    revision,
                                    vec![TextEdit::new(ByteRange::new(0, 0), text)],
                                ),
                                this.selection,
                                after,
                                EditOrigin::Other,
                            ),
                            cx,
                        )
                    });
                    if result.is_err() {
                        this.command_feedback = Some(this.ui_language.text("todo.failed").into());
                        return;
                    }
                    this.selection = after;
                    this.sync_selection_revision(cx);
                    this.sync_selection_utf16(&this.snapshot(cx));
                }
                this.pending_reveal_caret = true;
                this.autofocus = true;
                cx.notify();
            });
        })
        .detach();
    }
}
