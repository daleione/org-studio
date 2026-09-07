//! Generated text uses the normal editor's shaping, hit testing and selection.
use super::*;
use gpui::{HighlightStyle, TextRun};

pub(crate) type LineHighlights = Vec<(Range<usize>, HighlightStyle)>;

impl SemanticEditor {
    pub(crate) fn new_read_only(
        text: String,
        highlights: Vec<LineHighlights>,
        cx: &mut Context<Self>,
    ) -> Self {
        let session = cx.new(|_| DocumentSession::read_only_text(text));
        let mut editor = Self::new_with_autofocus(session, false, cx);
        editor.generated_highlights = Some(highlights);
        editor.display_map.set_soft_wrap(false);
        editor.minimap.visible = false;
        editor.inline_image_previews = false;
        editor
    }

    pub(crate) fn update_read_only(
        &mut self,
        text: String,
        highlights: Vec<LineHighlights>,
        cx: &mut Context<Self>,
    ) {
        let snapshot = self.snapshot(cx);
        let changed = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())) != text;
        self.generated_highlights = Some(highlights);
        self.shape_cache.clear();
        if changed {
            self.session
                .update(cx, |session, cx| session.replace_read_only_text(text, cx));
            self.set_selection(Selection::default(), cx);
            self.scroll_x = 0.0;
            self.scroll_y = 0.0;
        }
        cx.notify();
    }

    pub(crate) fn is_read_only(&self, cx: &App) -> bool {
        self.session.read(cx).is_read_only()
    }

    pub(crate) fn selected_line(&self, cx: &App) -> u64 {
        self.snapshot(cx)
            .line_index_at(self.selection.head())
            .map_or(0, |line| line.0)
    }
}

pub(super) fn highlighted_runs(base: TextRun, highlights: &LineHighlights) -> Vec<TextRun> {
    let mut boundaries = vec![0, base.len];
    for (range, _) in highlights {
        boundaries.extend([range.start.min(base.len), range.end.min(base.len)]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
        .windows(2)
        .map(|bounds| {
            let mut run = base.clone();
            run.len = bounds[1] - bounds[0];
            for (range, style) in highlights {
                if range.start <= bounds[0] && range.end >= bounds[1] {
                    if let Some(color) = style.color {
                        run.color = color;
                    }
                    if let Some(weight) = style.font_weight {
                        run.font.weight = weight;
                    }
                }
            }
            run
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::EntityInputHandler;

    #[gpui::test]
    fn generated_buffer_selects_and_copies_but_rejects_input_and_saving(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(super::super::init);
        let text = "Monday 7 September 2026 W37\n  工作: Scheduled: TODO 中文任务\n";
        let (editor, cx) =
            cx.add_window_view(|_, cx| SemanticEditor::new_read_only(text.into(), vec![], cx));
        cx.update(|window, app| {
            let focus = editor.read(app).focus_handle.clone();
            window.focus(&focus, app);
        });
        cx.simulate_keystrokes("cmd-a cmd-c");
        cx.run_until_parked();
        cx.update(|_, app| assert_eq!(app.read_from_clipboard().unwrap().text().unwrap(), text));
        cx.simulate_keystrokes("x backspace delete cmd-v cmd-x cmd-z cmd-shift-z tab shift-tab");
        cx.update(|window, app| {
            editor.update(app, |editor, cx| {
                editor.replace_and_mark_text_in_range(None, "输入", Some(2..2), window, cx);
                editor.replace_text_in_range(None, "替换", window, cx);
                let session = editor.session.read(cx);
                assert_eq!(
                    session
                        .snapshot()
                        .copy_range(ByteRange::new(0, session.snapshot().len_bytes())),
                    text
                );
                assert!(!session.is_dirty());
                assert!(matches!(
                    session.save_request(None),
                    Err(crate::document::SaveStartError::ReadOnly)
                ));
                assert!(matches!(
                    session.save_request(Some("/tmp/should-not-save.org".into())),
                    Err(crate::document::SaveStartError::ReadOnly)
                ));
                assert!(editor.marked.is_none());
                assert!(!editor.soft_wrap());
                assert!(!editor.minimap.visible);
            });
        });
        cx.simulate_keystrokes("cmd-up down");
        cx.run_until_parked();
        editor.update(cx, |editor, cx| assert_eq!(editor.selected_line(cx), 1));
        editor.update(cx, |editor, cx| {
            editor.update_read_only(text.into(), vec![], cx)
        });
        cx.run_until_parked();
        editor.update(cx, |editor, cx| assert_eq!(editor.selected_line(cx), 1));
        editor.update(cx, |editor, cx| {
            editor.update_read_only("Changed\n".into(), vec![], cx)
        });
        cx.run_until_parked();
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.selected_line(cx), 0);
            assert!(!editor.session.read(cx).is_dirty());
        });
    }

    #[gpui::test]
    fn generated_session_rejects_direct_edits(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| DocumentSession::read_only_text("original".into()));
        session.update(cx, |session, cx| {
            let transaction = EditTransaction::new(
                session.revision(),
                vec![TextEdit::new(ByteRange::new(0, 0), "bad")],
            );
            assert!(matches!(
                session.apply_transient_edit(transaction.clone(), cx),
                Err(crate::document::EditError::ReadOnly)
            ));
            let command = DocumentCommand::new(
                transaction,
                Selection::default(),
                Selection::default(),
                EditOrigin::Typing,
            );
            assert!(matches!(
                session.edit(command, cx),
                Err(crate::document::EditError::ReadOnly)
            ));
            assert!(session.undo(cx).is_err());
            assert!(session.redo(cx).is_err());
            assert!(matches!(
                session.begin_force_save(),
                Err(crate::document::SaveStartError::ReadOnly)
            ));
        });
    }

    #[gpui::test]
    fn enter_activates_the_caret_line_without_inserting_a_newline(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let activated = std::rc::Rc::new(std::cell::Cell::new(None));
        let editor =
            cx.new(|cx| SemanticEditor::new_read_only("Header\nTask\n".into(), vec![], cx));
        struct Harness(
            Entity<SemanticEditor>,
            std::rc::Rc<std::cell::Cell<Option<u64>>>,
        );
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let activated = self.1.clone();
                div()
                    .size_full()
                    .on_action(move |action: &ActivateReadOnlyLine, _, _| {
                        activated.set(Some(action.line))
                    })
                    .child(self.0.clone())
            }
        }
        let (_, cx) = cx.add_window_view(|_, _| Harness(editor.clone(), activated.clone()));
        cx.update(|window, app| {
            let focus = editor.read(app).focus_handle.clone();
            window.focus(&focus, app);
        });
        cx.simulate_keystrokes("down enter");
        cx.run_until_parked();
        assert_eq!(activated.get(), Some(1));
        editor.update(cx, |editor, cx| {
            assert!(!editor.session.read(cx).is_dirty())
        });
    }
}
