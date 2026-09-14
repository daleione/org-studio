//! Revision-guarded edits and picker commands; keep selection and layout stable.
use super::*;
use crate::links::LiteralDestination;

impl SemanticEditor {
    pub(super) fn apply_inline_edits(
        &mut self,
        revision: Revision,
        edits: Vec<TextEdit>,
        checkbox_hover: Option<ByteRange>,
        cx: &mut Context<Self>,
    ) {
        if self.is_read_only(cx) || self.snapshot(cx).revision() != revision {
            return;
        }
        let snapshot = self.snapshot(cx);
        let stable_lines = edits.iter().all(|edit| {
            !edit.replacement.contains(['\n', '\r'])
                && !snapshot.copy_range(edit.range).contains(['\n', '\r'])
        });
        let map = |offset| crate::editor::commands::map_offset_through_edits(offset, &edits);
        let after = Selection::new(map(self.selection.anchor()), map(self.selection.head()));
        let checkbox_hover = checkbox_hover
            .filter(|_| stable_lines)
            .map(|range| ByteRange::new(map(range.start).0, map(range.end).0));
        let command = DocumentCommand::new(
            EditTransaction::new(revision, edits),
            self.selection,
            after,
            EditOrigin::Other,
        );
        if self.session.update(cx, |s, cx| s.edit(command, cx)).is_ok() {
            self.selection = after;
            self.sync_selection_revision(cx);
            self.inline_actions.commit = stable_lines.then(|| InlineCommit {
                revision: self.snapshot(cx).revision(),
                checkbox_hover,
            });
            self.sync_selection_utf16(&self.snapshot(cx));
            self.hit_rows = Arc::from([]);
            self.pending_reveal_caret = false;
            self.inline_actions.hovered = checkbox_hover.map(|range| Hovered {
                range,
                kind: HighlightKind::Token,
            });
        }
        cx.notify();
    }
    pub(super) fn inline_event(&mut self, event: &InlineEvent, cx: &mut Context<Self>) {
        let Some(popup) = self.inline_actions.popup.as_ref() else {
            return;
        };
        let revision = popup.revision;
        let hit = popup.hit.clone();
        if self.snapshot(cx).revision() != revision
            || self.snapshot(cx).copy_range(hit.range) != hit.source
        {
            self.dismiss_inline(cx);
            return;
        }
        let mut range = hit.range;
        let replacement = match event {
            InlineEvent::Priority(value) if matches!(hit.kind, Kind::Priority) => {
                if let Some(value) = value {
                    Some(format!("[#{value}]"))
                } else {
                    let snapshot = self.snapshot(cx);
                    let Some(line) = snapshot
                        .line_index_at(range.end)
                        .ok()
                        .and_then(|line| snapshot.line_content_range(line).ok())
                    else {
                        return;
                    };
                    let tail = snapshot.copy_range(ByteRange::new(range.end.0, line.end.0));
                    range.end.0 += (tail.len() - tail.trim_start_matches([' ', '\t']).len()) as u64;
                    Some(String::new())
                }
            }
            InlineEvent::Tags(tags) if matches!(hit.kind, Kind::Tags) => Some(if tags.is_empty() {
                String::new()
            } else {
                format!(":{}:", tags.join(":"))
            }),
            InlineEvent::Link(target) => {
                let Kind::Link(link) = &hit.kind else {
                    return;
                };
                let Some(destination) = LiteralDestination::parse(&hit.source, &link.meta.raw)
                else {
                    return;
                };
                if !destination.accepts(target) {
                    if let Some(popup) = &self.inline_actions.popup {
                        popup.picker.update(cx, |p, cx| p.reject_input(cx));
                    }
                    return;
                }
                let local = destination.range;
                range = ByteRange::new(
                    hit.range.start.0 + local.start as u64,
                    hit.range.start.0 + local.end as u64,
                );
                Some(target.clone())
            }
            InlineEvent::Open => {
                if let Kind::Link(link) = &hit.kind {
                    self.open_link(link, cx);
                }
                None
            }
            InlineEvent::Copy => {
                if let Kind::Link(link) = &hit.kind {
                    cx.write_to_clipboard(ClipboardItem::new_string(link.meta.raw.to_string()));
                }
                None
            }
            _ => None,
        };
        self.dismiss_inline(cx);
        if let Some(replacement) = replacement
            && self.snapshot(cx).copy_range(range) != replacement
        {
            self.apply_inline_edits(revision, vec![TextEdit::new(range, replacement)], None, cx);
        }
    }
}
