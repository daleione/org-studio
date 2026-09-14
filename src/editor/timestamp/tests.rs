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
        assert!(editor.read(cx).timestamp.popup.is_none());
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
        editor.timestamp.popup.as_mut().unwrap().revision = Revision(u64::MAX);
        editor.apply_timestamp("<2026-09-16 Wed>", cx);
        assert_eq!(
            editor.snapshot(cx).copy_range(ByteRange::new(0, 16)),
            "<2026-09-15 Tue>"
        );
        assert_eq!(
            editor.command_feedback.as_deref(),
            Some(editor.ui_language.text("timestamp.source_changed"))
        );
    });
}
