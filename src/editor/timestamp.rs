//! Editor host for the reusable timestamp picker: hit testing, hover delay and
//! optimistic, undoable source replacement. The picker itself has no session.
use super::*;
use crate::{
    components::timestamp_picker::{TimestampPicker, TimestampPickerEvent},
    document::DocumentFormat,
    org_semantic::{TimestampKind, timestamp_edit::timestamp_at},
};

pub(super) struct TimestampPopup {
    picker: Entity<TimestampPicker>,
    _subscription: Subscription,
    range: ByteRange,
    revision: Revision,
    original: String,
    position: Point<Pixels>,
}

impl SemanticEditor {
    pub(crate) fn set_ui_language(
        &mut self,
        language: crate::i18n::Language,
        cx: &mut Context<Self>,
    ) {
        if self.ui_language != language {
            self.ui_language = language;
            if let Some(popup) = &self.timestamp_popup {
                popup
                    .picker
                    .update(cx, |picker, cx| picker.set_language(language, cx));
            }
            cx.notify();
        }
    }

    pub(super) fn timestamp_highlight(&self) -> Option<ByteRange> {
        self.timestamp_popup
            .as_ref()
            .map(|popup| popup.range)
            .or(self
                .timestamp_hover_range
                .filter(|range| Some(*range) != self.timestamp_dismissed))
    }
    fn timestamp_hit(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<(ByteRange, String, TimestampKind, Point<Pixels>)> {
        if self.is_read_only(cx)
            || DocumentFormat::from_path(self.session.read(cx).syntax_path()) != DocumentFormat::Org
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
        if self.timestamp_popup.is_some()
            || self.is_selecting
            || self.minimap.drag.is_some()
            || self.minimap.resizing.is_some()
        {
            return;
        }
        let hit = self.timestamp_hit(position, cx);
        let range = hit.as_ref().map(|h| h.0);
        if range != self.timestamp_dismissed {
            self.timestamp_dismissed = None;
        }
        if range == self.timestamp_hover_range {
            return;
        }
        self.timestamp_hover_task = None;
        self.timestamp_hover_range = range;
        cx.notify();
        let Some((range, source, kind, anchor)) = hit else {
            return;
        };
        if self.timestamp_dismissed == Some(range) {
            return;
        }
        let revision = self.snapshot(cx).revision();
        let executor = cx.background_executor().clone();
        self.timestamp_hover_task = Some(cx.spawn(async move |this, cx| {
            executor.timer(Duration::from_millis(350)).await;
            let _ = this.update(cx, |this, cx| {
                if this.timestamp_hover_range == Some(range)
                    && this.snapshot(cx).revision() == revision
                    && !this.is_selecting
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
        self.timestamp_popup = Some(TimestampPopup {
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
        if let Some(popup) = self.timestamp_popup.take() {
            self.timestamp_dismissed = Some(popup.range);
        }
        self.timestamp_hover_task = None;
        // Retain the hovered identity until the pointer leaves, preventing reopen.
        cx.notify();
    }

    fn apply_timestamp(&mut self, source: &str, cx: &mut Context<Self>) {
        let Some(popup) = self.timestamp_popup.take() else {
            return;
        };
        self.timestamp_dismissed = Some(popup.range);
        self.timestamp_hover_task = None;
        let snapshot = self.snapshot(cx);
        if self.is_read_only(cx)
            || snapshot.revision() != popup.revision
            || snapshot.copy_range(popup.range) != popup.original
        {
            self.command_feedback = Some("日期所在文本已变化，请重新打开日历。".into());
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
            self.command_feedback = Some("未能修改日期，请重新打开日历。".into());
        }
        cx.notify();
    }

    pub(super) fn timestamp_overlay(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        let popup = self.timestamp_popup.as_ref()?;
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
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext};

    #[gpui::test]
    fn timestamp_popup_click_edit_apply_and_undo(cx: &mut TestAppContext) {
        cx.update(init);
        let source = "* TODO 评审\n<2026-09-15 Tue 14:00>\n保留正文\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("calendar.org"), source.as_bytes().to_vec())
                .unwrap()
        });
        let view_session = session.clone();
        let (editor, cx) = cx.add_window_view(move |_, cx| SemanticEditor::new(view_session, cx));
        cx.run_until_parked();
        editor.update(cx, |editor, cx| {
            let snapshot = editor.snapshot(cx);
            let range = snapshot.line_content_range(LineIndex(1)).unwrap();
            editor.open_timestamp_picker(
                range,
                snapshot.copy_range(range),
                TimestampKind::Plain,
                gpui::point(px(80.), px(80.)),
                cx,
            );
        });
        cx.run_until_parked();
        for (language, width) in [
            (crate::i18n::Language::Chinese, 360.),
            (crate::i18n::Language::English, 400.),
        ] {
            editor.update(cx, |editor, cx| editor.set_ui_language(language, cx));
            cx.run_until_parked();
            assert_eq!(
                cx.debug_bounds("timestamp-picker").unwrap().size.width,
                px(width)
            );
        }
        let popup = cx
            .debug_bounds("timestamp-picker")
            .expect("picker is painted");
        let repeat = cx.debug_bounds("repeat-page").unwrap();
        assert!(popup.contains(&repeat.center()));
        cx.simulate_click(repeat.center(), Modifiers::default());
        cx.run_until_parked();
        let restart = cx.debug_bounds("restart").expect("repeat subview");
        cx.simulate_click(restart.center(), Modifiers::default());
        let done = cx.debug_bounds("repeat-done").unwrap().center();
        cx.simulate_click(done, Modifiers::default());
        cx.run_until_parked();
        let apply = cx.debug_bounds("apply").unwrap();
        assert!(
            cx.debug_bounds("timestamp-picker")
                .unwrap()
                .contains(&apply.center())
        );
        cx.simulate_click(apply.center(), Modifiers::default());
        cx.run_until_parked();
        cx.read(|cx| {
            let snapshot = session.read(cx).snapshot();
            assert_eq!(
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
                "* TODO 评审\n<2026-09-15 Tue 14:00 .+1w>\n保留正文\n"
            );
            assert!(editor.read(cx).timestamp_popup.is_none());
        });
        cx.simulate_keystrokes("cmd-z");
        cx.read(|cx| {
            let snapshot = session.read(cx).snapshot();
            assert_eq!(
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
                source
            );
        });
    }

    #[gpui::test]
    fn timestamp_hover_uses_source_geometry_and_ignores_code(cx: &mut TestAppContext) {
        cx.update(init);
        let session=cx.new(|_|DocumentSession::from_utf8(PathBuf::from("calendar.org"),"* 日程\n<2026-09-15 Tue 14:00>\n#+begin_src org\n<2026-09-15 Tue>\n#+end_src\n=<2026-09-15 Tue>=\n".as_bytes().to_vec()).unwrap());
        let (editor, cx) = cx.add_window_view(move |_, cx| SemanticEditor::new(session, cx));
        cx.run_until_parked();
        editor.update(cx, |editor, cx| {
            for (line, expected) in [(1, true), (3, false), (5, false)] {
                let row = editor.hit_rows.iter().find(|r| r.line.0 == line).unwrap();
                let point = gpui::point(row.text_origin_x + px(35.), row.visible_top + px(5.));
                assert_eq!(
                    editor.timestamp_hit(point, cx).is_some(),
                    expected,
                    "line {line}"
                );
                if expected {
                    editor.timestamp_hover(point, cx);
                }
            }
        });
        cx.executor().advance_clock(Duration::from_millis(400));
        cx.run_until_parked();
        assert!(cx.debug_bounds("timestamp-picker").is_some());
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(cx.debug_bounds("timestamp-picker").is_none());
    }

    #[gpui::test]
    fn timestamp_apply_rejects_stale_source(cx: &mut TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("calendar.org"), b"<2026-09-15 Tue>".to_vec())
                .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session.clone(), cx));
        editor.update(cx, |editor, cx| {
            editor.open_timestamp_picker(
                ByteRange::new(0, 16),
                "<2026-09-15 Tue>".into(),
                TimestampKind::Plain,
                Point::default(),
                cx,
            );
            // Corrupt the captured revision to exercise the last write guard.
            editor.timestamp_popup.as_mut().unwrap().revision = Revision(u64::MAX);
            editor.apply_timestamp("<2026-09-16 Wed>", cx);
            assert_eq!(
                editor.snapshot(cx).copy_range(ByteRange::new(0, 16)),
                "<2026-09-15 Tue>"
            );
            assert!(editor.command_feedback.is_some());
        });
    }
}
