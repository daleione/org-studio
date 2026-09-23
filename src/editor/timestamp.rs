//! Editor host for the reusable timestamp picker: hit testing, hover delay and
//! optimistic, undoable source replacement. The picker itself has no session.
use super::*;
use crate::{
    components::timestamp_picker::{TimestampPicker, TimestampPickerEvent},
    document::DocumentFormat,
    org_semantic::{TimestampKind, timestamp_edit::timestamp_at},
};

#[derive(Default)]
pub(super) struct TimestampInteraction {
    pub(super) popup: Option<TimestampPopup>,
    pub(super) hover_range: Option<ByteRange>,
    pub(super) dismissed: Option<ByteRange>,
    pub(super) hover_task: Option<Task<()>>,
}

pub(super) struct TimestampPopup {
    picker: Entity<TimestampPicker>,
    _subscription: Subscription,
    range: ByteRange,
    revision: Revision,
    original: String,
    position: Point<Pixels>,
}

impl SemanticEditor {
    pub(super) fn set_timestamp_language(
        &mut self,
        language: crate::i18n::Language,
        cx: &mut Context<Self>,
    ) {
        if let Some(popup) = &self.timestamp.popup {
            popup
                .picker
                .update(cx, |picker, cx| picker.set_language(language, cx));
        }
    }

    pub(super) fn timestamp_highlight(&self) -> Option<ByteRange> {
        self.timestamp
            .popup
            .as_ref()
            .map(|popup| popup.range)
            .or(self
                .timestamp
                .hover_range
                .filter(|range| Some(*range) != self.timestamp.dismissed))
    }
    fn timestamp_hit(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<(ByteRange, String, TimestampKind, Point<Pixels>)> {
        if self.is_read_only(cx)
            || DocumentFormat::from_path(self.session.read(cx).syntax_path()) != DocumentFormat::Org
            || crate::syntax_highlighting::language_for_path(self.session.read(cx).syntax_path())
                .is_some()
        {
            return None;
        }
        let row = self
            .hit_rows
            .iter()
            .find(|row| position.y >= row.visible_top && position.y < row.visible_bottom)?;
        if row.inline_image_preview
            || position.x < row.text_origin_x
            || position.x > row.text_origin_x + row.visual_width()
        {
            return None;
        }
        let offset = self.hit_test(position);
        let snapshot = self.snapshot(cx);
        let range = snapshot.line_content_range(row.line).ok()?;
        // Avoid materializing pathological generated single-line buffers on hover.
        if range.len() > 64 * 1024 {
            return None;
        }
        let query = syntax::SparseEditorStyleSnapshot::query_lines(
            self.session.read(cx).syntax_path(),
            &snapshot,
            &[row.line.0],
            &self.syntax_service,
        );
        let style = query.snapshot.line(row.line.0)?;
        if matches!(
            style.id,
            syntax::EditorStyleId::Code
                | syntax::EditorStyleId::Comment
                | syntax::EditorStyleId::CodeBoundary
        ) {
            return None;
        }
        let text = snapshot.copy_range(range);
        let (local, kind) = timestamp_at(&text, offset.0.saturating_sub(range.start.0) as usize)?;
        if crate::org_syntax::inline::parse(&text)
            .spans
            .iter()
            .any(|span| {
                matches!(
                    span.kind,
                    crate::org_syntax::inline::InlineKind::Code
                        | crate::org_syntax::inline::InlineKind::Verbatim
                        | crate::org_syntax::inline::InlineKind::Link
                        | crate::org_syntax::inline::InlineKind::Target
                        | crate::org_syntax::inline::InlineKind::RadioTarget
                ) && span.source.start <= local.start
                    && span.source.end >= local.end
            })
        {
            return None;
        }
        let source = text[local.clone()].to_owned();
        let range = ByteRange::new(
            range.start.0 + local.start as u64,
            range.start.0 + local.end as u64,
        );
        Some((
            range,
            source,
            kind,
            gpui::point(position.x, row.visible_bottom + px(6.)),
        ))
    }

    pub(super) fn timestamp_hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.inline_actions.popup.is_some()
            || self.todo.popup.is_some()
            || self.timestamp.popup.is_some()
            || self.is_selecting
            || self.minimap.drag.is_some()
            || self.minimap.resizing.is_some()
        {
            return;
        }
        let hit = self.timestamp_hit(position, cx);
        let range = hit.as_ref().map(|h| h.0);
        if range != self.timestamp.dismissed {
            self.timestamp.dismissed = None;
        }
        if range == self.timestamp.hover_range {
            return;
        }
        self.timestamp.hover_task = None;
        self.timestamp.hover_range = range;
        cx.notify();
        let Some((range, source, kind, anchor)) = hit else {
            return;
        };
        if self.timestamp.dismissed == Some(range) {
            return;
        }
        let revision = self.snapshot(cx).revision();
        let executor = cx.background_executor().clone();
        self.timestamp.hover_task = Some(cx.spawn(async move |this, cx| {
            executor.timer(Duration::from_millis(350)).await;
            let _ = this.update(cx, |this, cx| {
                if this.timestamp.hover_range == Some(range)
                    && this.snapshot(cx).revision() == revision
                    && !this.is_selecting
                    && this.inline_actions.popup.is_none()
                    && this.todo.popup.is_none()
                {
                    this.open_timestamp_picker(range, source, kind, anchor, cx);
                }
            });
        }));
    }

    fn open_timestamp_picker(
        &mut self,
        range: ByteRange,
        source: String,
        kind: TimestampKind,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let picker = cx.new(|cx| {
            TimestampPicker::new(
                source.clone(),
                kind,
                jiff::Zoned::now().date(),
                self.ui_language,
                cx,
            )
        });
        let subscription = cx.subscribe(&picker, |this, _, event: &TimestampPickerEvent, cx| {
            match event {
                TimestampPickerEvent::Applied(source) => this.apply_timestamp(source, cx),
                TimestampPickerEvent::Cancelled => this.dismiss_timestamp(cx),
            }
            this.autofocus = true;
            cx.notify();
        });
        self.timestamp.popup = Some(TimestampPopup {
            picker,
            _subscription: subscription,
            range,
            revision: self.snapshot(cx).revision(),
            original: source,
            position,
        });
        self.hovered_link = None;
        self.hover_position = None;
        cx.notify();
    }

    pub(super) fn dismiss_timestamp(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = self.timestamp.popup.take() {
            self.timestamp.dismissed = Some(popup.range);
        }
        self.timestamp.hover_task = None;
        // Retain the hovered identity until the pointer leaves, preventing reopen.
        cx.notify();
    }

    fn apply_timestamp(&mut self, source: &str, cx: &mut Context<Self>) {
        let Some(popup) = self.timestamp.popup.take() else {
            return;
        };
        self.timestamp.dismissed = Some(popup.range);
        self.timestamp.hover_task = None;
        let snapshot = self.snapshot(cx);
        if self.is_read_only(cx)
            || snapshot.revision() != popup.revision
            || snapshot.copy_range(popup.range) != popup.original
        {
            self.command_feedback = Some(self.ui_language.text("timestamp.source_changed").into());
            cx.notify();
            return;
        }
        if source == popup.original {
            cx.notify();
            return;
        }
        self.finish_composition(cx);
        let after = Selection::caret(ByteOffset(popup.range.start.0 + source.len() as u64));
        let result = self.session.update(cx, |session, cx| {
            session.edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        popup.revision,
                        vec![TextEdit::new(popup.range, source.to_owned())],
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
            self.pending_reveal_caret = true;
            self.command_feedback = None;
        } else {
            self.command_feedback = Some(self.ui_language.text("timestamp.failed").into());
        }
        cx.notify();
    }

    pub(super) fn timestamp_overlay(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        let popup = self.timestamp.popup.as_ref()?;
        Some(
            gpui::deferred(
                gpui::anchored()
                    .position(popup.position)
                    .snap_to_window_with_margin(px(10.))
                    .child(
                        div()
                            .id("editor-timestamp-popup")
                            .on_mouse_down_out(
                                cx.listener(|this, _, _, cx| this.dismiss_timestamp(cx)),
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
