//! The picker edits values; only this revision-guarded host writes the source document.
use super::super::*;
use crate::components::link_picker::{LinkPicker, LinkPickerEvent};
use crate::document::{DocumentFormat, RevisionRange};
use crate::org_syntax::inline::{BracketLink, InlineKind, LinkValue, unescape_link_target};
use gpui::point;

pub(in crate::editor) struct LinkEditor {
    picker: Entity<LinkPicker>,
    source: RevisionRange,
    original: String,
    position: Point<Pixels>,
    _subscription: Subscription,
}

impl SemanticEditor {
    pub(crate) fn link_editor_is_open(&self) -> bool {
        self.link_editor.is_some()
    }

    pub(crate) fn edit_link_at_caret(&mut self, cx: &mut Context<Self>) {
        if self.is_read_only(cx)
            || DocumentFormat::detect(self.session.read(cx).syntax_path())
                != Some(DocumentFormat::Org)
        {
            return;
        }
        self.finish_composition(cx);
        let snapshot = self.snapshot(cx);
        let head = self.selection.head();
        let Ok(line) = snapshot.line_index_at(head) else {
            return;
        };
        let Ok(range) = snapshot.line_content_range(line) else {
            return;
        };
        let styles = syntax::SparseEditorStyleSnapshot::query_lines(
            self.session.read(cx).syntax_path(),
            &snapshot,
            &[line.0],
            &self.syntax_service,
        );
        if styles.snapshot.line(line.0).is_none_or(|style| {
            matches!(
                style.id,
                syntax::EditorStyleId::Code | syntax::EditorStyleId::CodeBoundary
            )
        }) {
            return;
        }
        let text = snapshot.copy_range(range);
        let local = (head.0 - range.start.0) as usize;
        let parsed = crate::org_syntax::inline::parse(&text);
        let links = parsed
            .spans
            .iter()
            .filter(|span| span.kind == InlineKind::Link)
            .collect::<Vec<_>>();
        let link = links
            .iter()
            .find(|span| span.source.contains(&local))
            .or_else(|| links.iter().find(|span| span.source.end == local))
            .and_then(|span| BracketLink::at(&text, span.source.start));
        let (source, value) = if let Some(link) = link {
            (
                ByteRange::new(
                    range.start.0 + link.source.start as u64,
                    range.start.0 + link.source.end as u64,
                ),
                LinkValue {
                    target: unescape_link_target(&text[link.target]),
                    description: link
                        .description
                        .map_or_else(String::new, |range| text[range].to_owned()),
                },
            )
        } else {
            (
                self.selection.range(),
                LinkValue {
                    target: String::new(),
                    description: snapshot.copy_range(self.selection.range()),
                },
            )
        };
        let position = self
            .hit_rows
            .iter()
            .find(|row| row.line == line)
            .and_then(|row| {
                let local = row
                    .display
                    .source_to_display(head.0.saturating_sub(row.range.start.0) as usize);
                row.position_for_display_index(local).map(|position| {
                    point(
                        row.text_origin_x + position.x,
                        row.origin_y + position.y + row.line_height,
                    )
                })
            })
            .or_else(|| self.viewport.map(|viewport| viewport.origin))
            .unwrap_or_default();
        self.dismiss_inline(cx);
        self.dismiss_todo(cx);
        self.dismiss_timestamp(cx);
        self.dismiss_table_actions(cx);
        self.hovered_link = None;
        let picker = cx.new(|cx| LinkPicker::new(value, self.ui_language, cx));
        let subscription = cx.subscribe(&picker, |this, _, event, cx| match event {
            LinkPickerEvent::Applied(source) => this.apply_link_edit(source, cx),
            LinkPickerEvent::Cancelled => this.dismiss_link_editor(cx),
        });
        self.link_editor = Some(LinkEditor {
            picker,
            source: snapshot.revision_range(source),
            original: snapshot.copy_range(source),
            position,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn apply_link_edit(&mut self, source: &str, cx: &mut Context<Self>) {
        let Some(popup) = self.link_editor.take() else {
            return;
        };
        self.autofocus = true;
        let snapshot = self.snapshot(cx);
        if self.is_read_only(cx)
            || snapshot.revision() != popup.source.revision
            || snapshot.copy_range(popup.source.range) != popup.original
        {
            self.command_feedback = Some(self.ui_language.text("link.source_changed").into());
            cx.notify();
            return;
        }
        if source == popup.original {
            cx.notify();
            return;
        }
        let after = Selection::caret(ByteOffset(popup.source.range.start.0 + source.len() as u64));
        let command = DocumentCommand::new(
            EditTransaction::new(
                snapshot.revision(),
                vec![TextEdit::new(popup.source.range, source.to_owned())],
            ),
            self.selection,
            after,
            EditOrigin::Other,
        );
        if self
            .session
            .update(cx, |session, cx| session.edit(command, cx))
            .is_ok()
        {
            self.selection = after;
            self.sync_selection_revision(cx);
            self.sync_selection_utf16(&self.snapshot(cx));
            self.hit_rows = Arc::from([]);
            self.pending_reveal_caret = true;
        }
        cx.notify();
    }

    pub(in crate::editor) fn dismiss_link_editor(&mut self, cx: &mut Context<Self>) {
        if self.link_editor.take().is_some() {
            self.autofocus = true;
            cx.notify();
        }
    }

    pub(in crate::editor) fn link_editor_overlay(
        &self,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let popup = self.link_editor.as_ref()?;
        Some(
            gpui::deferred(
                gpui::anchored()
                    .position(popup.position)
                    .snap_to_window_with_margin(px(10.))
                    .child(
                        div()
                            .id("editor-link-popup")
                            .on_mouse_down_out(
                                cx.listener(|this, _, _, cx| this.dismiss_link_editor(cx)),
                            )
                            .child(popup.picker.clone()),
                    ),
            )
            .with_priority(10)
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests;
