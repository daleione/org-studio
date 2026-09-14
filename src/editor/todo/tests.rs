use super::geometry::quick_bar_bounds;
use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext};

fn point(
    editor: &Entity<SemanticEditor>,
    line: u64,
    local: usize,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    cx.read(|cx| {
        let row = editor
            .read(cx)
            .hit_rows
            .iter()
            .find(|r| r.line.0 == line)
            .unwrap();
        let position = row
            .position_for_display_index(row.display.source_to_display(local))
            .unwrap();
        gpui::point(
            row.text_origin_x + position.x + px(2.),
            row.origin_y + position.y + px(5.),
        )
    })
}
fn hover(editor: &Entity<SemanticEditor>, line: u64, local: usize, cx: &mut VisualTestContext) {
    // Leave the previous token so deliberate reopening is possible after undo.
    cx.simulate_mouse_move(gpui::point(px(650.), px(600.)), None, Modifiers::default());
    let position = point(editor, line, local, cx);
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
}
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let p = cx.debug_bounds(selector).unwrap().center();
    cx.simulate_click(p, Modifiers::default());
    cx.run_until_parked();
}
fn source(editor: &Entity<SemanticEditor>, cx: &mut VisualTestContext) -> String {
    cx.read(|cx| {
        let s = editor.read(cx).snapshot(cx);
        s.copy_range(ByteRange::new(0, s.len_bytes()))
    })
}

#[gpui::test]
fn todo_menu_follows_another_heading_without_closing_first(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "#+TODO: TODO(t) DOING(i) | DONE(d)\n* TODO First\n* DOING Second\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("follow.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(gpui::size(px(800.), px(1000.)));
    cx.run_until_parked();
    hover(&editor, 1, 3, cx);
    let first_menu = cx.debug_bounds("todo-picker").unwrap();
    let first_range = cx.read(|cx| editor.read(cx).todo_highlight().unwrap());
    assert!(cx.debug_bounds("todo-current-TODO").is_some());
    assert!(cx.debug_bounds("todo-current-DOING").is_none());

    // Entering the options must retain the current heading.
    let option = cx.debug_bounds("todo-option-DONE").unwrap().center();
    cx.simulate_mouse_move(option, None, Modifiers::default());
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(editor.read(cx).todo_highlight(), Some(first_range)));

    let second = point(&editor, 2, 3, cx) + gpui::point(px(0.), px(8.));
    assert!(!first_menu.contains(&second));
    cx.simulate_mouse_move(second, None, Modifiers::default());
    cx.run_until_parked();
    cx.read(|cx| {
        let e = editor.read(cx);
        let popup = e.todo.popup.as_ref().unwrap();
        assert_ne!(popup.hit.range, first_range);
        assert_eq!(popup.hit.keyword.as_ref(), "DOING");
    });

    assert!(cx.debug_bounds("todo-current-DOING").is_some());
    assert!(cx.debug_bounds("todo-current-TODO").is_none());

    // Repeated switching must not leave a delayed callback aimed at an old heading.
    let first = point(&editor, 1, 3, cx);
    cx.simulate_mouse_move(first, None, Modifiers::default());
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(editor.read(cx).todo_highlight(), Some(first_range)));
    cx.simulate_mouse_move(second, None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    click(cx, "todo-option-DONE");
    assert_eq!(
        source(&editor, cx),
        original.replace("* DOING Second", "* DONE Second")
    );
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(source(&editor, cx), original);
}

#[gpui::test]
fn todo_menu_custom_states_click_remove_and_undo(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "#+TODO: TODO(t) DOING(i) WAITING(w) | DONE(d) CANCELLED(c)\r\n* TODO [#A] 完成日历 :work:\r\n<2026-09-15 Tue 14:00>\r\n正文保持不变\r\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("states.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.run_until_parked();
    hover(&editor, 1, 3, cx);
    assert!(cx.debug_bounds("todo-picker").is_some());
    assert!(cx.debug_bounds("timestamp-picker").is_none());
    assert!(cx.debug_bounds("todo-option-WAITING").is_some());
    click(cx, "todo-option-DOING");
    assert_eq!(source(&editor, cx), original.replace("* TODO", "* DOING"));
    assert!(cx.debug_bounds("todo-picker").is_none());
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(source(&editor, cx), original);
    hover(&editor, 1, 3, cx);
    assert!(cx.debug_bounds("todo-remove").is_none());
    click(cx, "todo-more");
    click(cx, "todo-remove");
    assert_eq!(source(&editor, cx), original.replace("* TODO ", "* "));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(source(&editor, cx), original);
    hover(&editor, 1, 3, cx);
    click(cx, "todo-option-TODO");
    assert_eq!(source(&editor, cx), original);
}

#[gpui::test]
fn todo_menu_keys_and_dismissal_preserve_document(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "#+SEQ_TODO: PLAN(p) WAIT(w) | SHIPPED(s)\n* PLAN Build\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("keys.org"), original.as_bytes().to_vec()).unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.run_until_parked();
    hover(&editor, 1, 3, cx);
    for language in [
        crate::i18n::Language::English,
        crate::i18n::Language::Chinese,
    ] {
        editor.update(cx, |e, cx| e.set_ui_language(language, cx));
        cx.run_until_parked();
        assert!(cx.debug_bounds("todo-option-SHIPPED").is_some());
    }
    cx.simulate_keystrokes("s");
    assert_eq!(source(&editor, cx), original.replace("* PLAN", "* SHIPPED"));
    cx.simulate_keystrokes("cmd-z");
    hover(&editor, 1, 3, cx);
    cx.simulate_keystrokes("down enter");
    assert_eq!(source(&editor, cx), original.replace("* PLAN", "* WAIT"));
    cx.simulate_keystrokes("cmd-z");
    for keys in ["escape", "ctrl-g"] {
        hover(&editor, 1, 3, cx);
        cx.simulate_keystrokes(keys);
        cx.run_until_parked();
        assert!(cx.debug_bounds("todo-picker").is_none());
        assert_eq!(source(&editor, cx), original);
    }
    hover(&editor, 1, 3, cx);
    cx.simulate_click(gpui::point(px(650.), px(600.)), Modifiers::default());
    assert!(cx.debug_bounds("todo-picker").is_none());
    assert_eq!(source(&editor, cx), original);
}

#[gpui::test]
fn todo_hover_default_states_code_guard_and_stale_revision(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "* TODO Title\nTODO is body text\n#+begin_src org\n* TODO Example\n#+end_src\n* Ordinary heading\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("guards.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.run_until_parked();
    for (line, local) in [(1, 1), (3, 3), (5, 3)] {
        hover(&editor, line, local, cx);
        assert!(cx.debug_bounds("todo-picker").is_none(), "line {line}");
    }
    hover(&editor, 0, 3, cx);
    assert!(cx.debug_bounds("todo-option-TODO").is_some());
    assert!(cx.debug_bounds("todo-option-DONE").is_some());
    assert!(cx.debug_bounds("todo-option-DOING").is_none());
    editor.update(cx, |e, cx| {
        e.todo.popup.as_mut().unwrap().revision = Revision(u64::MAX);
        e.apply_todo(Some("DONE"), cx);
        assert!(e.command_feedback.is_some());
    });
    assert_eq!(source(&editor, cx), original);
    hover(&editor, 0, 3, cx);
    editor.update(cx, |e, cx| e.scroll(0., -10., cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("todo-picker").is_none());
    assert_eq!(source(&editor, cx), original);
}
#[gpui::test]
fn todo_quick_bar_bridge_and_delayed_dismissal(cx: &mut TestAppContext) {
    cx.update(init);
    let session = cx.new(|_| {
        DocumentSession::from_utf8(
            PathBuf::from("bridge.org"),
            b"* TODO First\n\n\n* TODO Second\n".to_vec(),
        )
        .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.run_until_parked();
    let before = point(&editor, 3, 3, cx);
    hover(&editor, 0, 3, cx);
    let bar = cx.debug_bounds("todo-bar").unwrap();
    cx.read(|cx| {
        let token = editor.read(cx).todo.popup.as_ref().unwrap().hit.bounds;
        assert!(bar.top() >= token.bottom());
    });
    assert_eq!(
        before,
        point(&editor, 3, 3, cx),
        "floating UI must not relayout source"
    );
    // Traverse the transparent 3px bridge, then linger on a real option.
    let bridge = gpui::point(bar.left() + px(10.), bar.top() - px(2.));
    cx.simulate_mouse_move(bridge, None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert!(cx.debug_bounds("todo-bar").is_some());
    let option = cx.debug_bounds("todo-option-DONE").unwrap().center();
    cx.simulate_mouse_move(option, None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert!(cx.debug_bounds("todo-bar").is_some());
    let outside = gpui::point(px(600.), px(500.));
    cx.simulate_mouse_move(outside, None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    assert!(cx.debug_bounds("todo-bar").is_some());
    // Reentry cancels the pending timer, rather than leaving a stale dismissal.
    cx.simulate_mouse_move(option, None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    assert!(cx.debug_bounds("todo-bar").is_some());
    cx.simulate_mouse_move(outside, None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(350));
    cx.run_until_parked();
    assert!(cx.debug_bounds("todo-bar").is_none());
}

#[gpui::test]
fn todo_quick_bar_preserves_reversed_selection_and_scroll(cx: &mut TestAppContext) {
    cx.update(init);
    let original = format!(
        "#+TODO: TODO(t) DOING(i) | DONE(d)\n{}",
        "* TODO 中文任务\n".repeat(100)
    );
    let selected_start = original.rfind("中文任务").unwrap() as u64;
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("caret.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(gpui::size(px(800.), px(400.)));
    cx.run_until_parked();
    let before = Selection::new(ByteOffset(selected_start + 12), ByteOffset(selected_start));
    editor.update(cx, |e, cx| {
        e.selection = before;
        e.sync_selection_revision(cx);
        e.sync_selection_utf16(&e.snapshot(cx));
        e.scroll_y = 150.;
        e.pending_reveal_caret = false;
        cx.notify();
    });
    cx.run_until_parked();
    let (line, scroll) = cx.read(|cx| {
        let e = editor.read(cx);
        (
            e.hit_rows
                .iter()
                .find(|r| r.visible_top > e.viewport.unwrap().top() + px(20.))
                .unwrap()
                .line
                .0,
            (e.scroll_x, e.scroll_y),
        )
    });
    hover(&editor, line, 3, cx);
    click(cx, "todo-option-DOING");
    cx.read(|cx| {
        let e = editor.read(cx);
        assert_eq!(
            e.selection,
            Selection::new(
                ByteOffset(before.anchor().0 + 1),
                ByteOffset(before.head().0 + 1)
            )
        );
        assert_eq!((e.scroll_x, e.scroll_y), scroll);
        assert!(!e.pending_reveal_caret);
        assert_eq!(e.snapshot(cx).copy_range(e.selection.range()), "中文任务");
    });
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(source(&editor, cx), original);
    cx.read(|cx| assert_eq!(editor.read(cx).selection, before));
}

#[gpui::test]
fn todo_quick_bar_narrow_bottom_and_keyboard_overflow(cx: &mut TestAppContext) {
    cx.update(init);
    let original = format!(
        "#+TODO: TODO(t) DOING(i) WAITING(w) | DONE(d) CANCELLED(c)\n{}",
        "* TODO Task\n".repeat(20)
    );
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("narrow.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(gpui::size(px(320.), px(280.)));
    cx.run_until_parked();
    let line = cx.read(|cx| {
        let e = editor.read(cx);
        e.hit_rows
            .iter()
            .rev()
            .find(|r| r.origin_y + px(6.) < e.viewport.unwrap().bottom())
            .unwrap()
            .line
            .0
    });
    hover(&editor, line, 3, cx);
    let bar = cx.debug_bounds("todo-bar").unwrap();
    cx.read(|cx| {
        let e = editor.read(cx);
        let token = e.todo.popup.as_ref().unwrap().hit.bounds;
        let viewport = e.viewport.unwrap();
        assert!(bar.bottom() <= token.top());
        assert!(bar.left() >= viewport.left() && bar.right() <= viewport.right());
    });
    assert!(bar.contains(&cx.debug_bounds("todo-more").unwrap().center()));
    // Left goes to More; another Left reveals the last overflowed status.
    cx.simulate_keystrokes("left left");
    cx.run_until_parked();
    assert!(bar.contains(&cx.debug_bounds("todo-option-CANCELLED").unwrap().center()));
    cx.simulate_keystrokes("space");
    assert_eq!(source(&editor, cx).matches("* CANCELLED Task").count(), 1);
    cx.simulate_keystrokes("cmd-z");
    // Undo reveals the caret; reopen a visible heading to check the More submenu.
    hover(&editor, 1, 3, cx);
    click(cx, "todo-more");
    let menu = cx.debug_bounds("todo-more-menu").unwrap();
    assert!(menu.top() >= px(0.) && menu.bottom() <= px(280.));
    assert!(menu.right() <= px(320.));
    click(cx, "todo-remove");
    assert_eq!(source(&editor, cx).matches("* Task").count(), 1);
}

#[gpui::test]
fn todo_quick_bar_customize_edits_native_directive(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "#+TODO: TODO(t) | DONE(d)\r\n#+SEQ_TODO: PLAN(p!) WAIT(w@) | SHIPPED(s)\r\n* PLAN Build\r\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("customize.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.run_until_parked();
    hover(&editor, 2, 3, cx);
    click(cx, "todo-more");
    click(cx, "todo-customize");
    assert_eq!(source(&editor, cx), original);
    cx.read(|cx| {
        let e = editor.read(cx);
        assert_eq!(
            e.snapshot(cx).copy_range(e.selection.range()),
            "PLAN(p!) WAIT(w@) | SHIPPED(s)"
        );
    });
}

#[gpui::test]
fn todo_quick_bar_customize_defaults_is_undoable(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "* TODO Build\r\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("defaults.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.run_until_parked();
    hover(&editor, 0, 3, cx);
    click(cx, "todo-more");
    click(cx, "todo-customize");
    assert_eq!(
        source(&editor, cx),
        format!("#+TODO: TODO | DONE\r\n{original}")
    );
    cx.read(|cx| {
        let e = editor.read(cx);
        assert_eq!(
            e.snapshot(cx).copy_range(e.selection.range()),
            "TODO | DONE"
        );
    });
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(source(&editor, cx), original);
}

#[test]
fn todo_quick_bar_clamps_to_editor_pane_and_flips_at_bottom() {
    let viewport = Bounds::new(
        gpui::point(px(200.), px(100.)),
        gpui::size(px(600.), px(400.)),
    );
    let token = |x, y| Bounds::new(gpui::point(px(x), px(y)), gpui::size(px(50.), px(24.)));
    let top = quick_bar_bounds(token(730., 100.), viewport, 420.);
    assert_eq!(top.top(), px(127.));
    assert_eq!(top.right(), px(792.));
    let bottom = quick_bar_bounds(token(210., 470.), viewport, 420.);
    assert_eq!(bottom.bottom(), px(467.));
    assert!(bottom.left() >= viewport.left());
}
