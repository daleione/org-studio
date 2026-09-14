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
        let p = row
            .position_for_display_index(row.display.source_to_display(local))
            .unwrap();
        gpui::point(
            row.text_origin_x + p.x + px(2.),
            row.origin_y + p.y + px(6.),
        )
    })
}
fn source(editor: &Entity<SemanticEditor>, cx: &mut VisualTestContext) -> String {
    cx.read(|cx| {
        let s = editor.read(cx).snapshot(cx);
        s.copy_range(ByteRange::new(0, s.len_bytes()))
    })
}
fn click(cx: &mut VisualTestContext, id: &'static str) {
    let p = cx.debug_bounds(id).unwrap().center();
    cx.simulate_click(p, Modifiers::default());
    cx.run_until_parked();
}
fn hover(editor: &Entity<SemanticEditor>, line: u64, local: usize, cx: &mut VisualTestContext) {
    let p = point(editor, line, local, cx);
    cx.simulate_mouse_move(p, None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
}
#[gpui::test]
fn checkbox_click_updates_statistics_and_undo_preserves_selection(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "* Checklist [0/2]\n- [ ] first\n- [ ] second\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("check.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.run_until_parked();
    editor.update(cx, |e, cx| {
        e.selection = Selection::new(ByteOffset(9), ByteOffset(3));
        e.sync_selection_utf16(&e.snapshot(cx));
    });
    let p = point(&editor, 1, 3, cx);
    cx.simulate_click(p, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        source(&editor, cx),
        original
            .replace("[0/2]", "[1/2]")
            .replace("[ ] first", "[X] first")
    );
    cx.read(|cx| {
        assert_eq!(
            editor.read(cx).selection,
            Selection::new(ByteOffset(9), ByteOffset(3))
        )
    });
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(source(&editor, cx), original);
}
#[gpui::test]
fn checkbox_drag_never_toggles_even_when_pointer_returns(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "* List\n- [ ] first task\n#+begin_src org\n- [ ] code\n#+end_src\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("drag.org"), original.as_bytes().to_vec()).unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.run_until_parked();
    let p = point(&editor, 1, 3, cx);
    let end = point(&editor, 1, 13, cx);
    cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
    cx.read(|cx| {
        assert_ne!(
            editor.read(cx).selection.anchor(),
            editor.read(cx).selection.head(),
            "dragging must still select text"
        )
    });
    cx.simulate_mouse_move(p, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(p, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(source(&editor, cx), original);
    let p = point(&editor, 3, 3, cx);
    cx.simulate_click(p, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(source(&editor, cx), original);
}

#[gpui::test]
fn checkbox_press_keeps_caret_viewport_and_hover_stable(cx: &mut TestAppContext) {
    cx.update(init);
    let original = format!(
        "* Checklist [9/10]\n{}- [ ] final item\n{}",
        "- [X] done\n".repeat(9),
        "body\n".repeat(60)
    );
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("steady.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(gpui::size(px(800.), px(600.)));
    cx.run_until_parked();
    let selection = Selection::new(ByteOffset(9), ByteOffset(3));
    editor.update(cx, |e, cx| {
        e.selection = selection;
        e.sync_selection_utf16(&e.snapshot(cx));
    });
    let p = point(&editor, 10, 3, cx);
    cx.simulate_mouse_move(p, None, Modifiers::default());
    cx.run_until_parked();
    let before = cx.read(|cx| {
        let e = editor.read(cx);
        (
            e.scroll_y,
            e.hit_rows
                .iter()
                .map(|r| (r.line, r.origin_y))
                .collect::<Vec<_>>(),
        )
    });
    cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    cx.read(|cx| {
        let e = editor.read(cx);
        assert_eq!(
            e.selection, selection,
            "press must not move the caret to the checkbox"
        );
        assert!(!e.is_selecting);
        assert_eq!(e.scroll_y, before.0);
    });
    let slightly_moved = p + gpui::point(px(2.), px(0.));
    cx.simulate_mouse_move(
        slightly_moved,
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_up(slightly_moved, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        source(&editor, cx),
        original.replace("[9/10]", "[10/10]").replace("[ ]", "[X]")
    );
    cx.read(|cx| {
        let e = editor.read(cx);
        assert_eq!(e.selection, selection);
        assert_eq!(e.scroll_y, before.0);
        assert_eq!(
            e.hit_rows
                .iter()
                .map(|r| (r.line, r.origin_y))
                .collect::<Vec<_>>(),
            before.1
        );
        let range = e
            .inline_background_highlight()
            .expect("hover must not blink off after the edit");
        assert_eq!(e.snapshot(cx).copy_range(range), "[X]");
    });
}

#[gpui::test]
fn checkbox_statistics_transaction_preserves_other_measured_lines(cx: &mut TestAppContext) {
    let original = format!(
        "* Checklist [0/1]\n- [ ] item\n{}",
        "long body line\n".repeat(80)
    );
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("measured.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let editor = cx.new(|cx| SemanticEditor::new(session, cx));
    editor.update(cx, |e, cx| {
        let snapshot = e.snapshot(cx);
        e.display_map.configure(snapshot.len_lines(), 500.);
        e.display_map.update_line_layout(50, 3, 22., 0., 0.);
        let start = original.find("[ ]").unwrap() as u64;
        let edits = crate::org_syntax::command::checkbox_transaction(
            &snapshot,
            ByteRange::new(start, start + 3),
        )
        .unwrap();
        assert!(edits.len() > 1);
        e.apply_inline_edits(snapshot.revision(), edits, None, cx);
    });
    cx.run_until_parked();
    editor.update(cx, |e, _| {
        assert_eq!(
            e.display_map.line_height_px(50),
            66.,
            "a checkbox and progress cookie must not discard unrelated line geometry"
        )
    });
}
#[gpui::test]
fn priority_bar_grace_keyboard_remove_and_undo(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "* TODO [#A] 中文任务\n\n* TODO [#B] Second\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("priority.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.run_until_parked();
    hover(&editor, 0, 9, cx);
    assert!(cx.debug_bounds("priority-B").is_some());
    let bar = cx.debug_bounds("inline-picker").unwrap();
    let remove = cx.debug_bounds("priority-remove").unwrap();
    assert!(
        bar.right() - remove.right() <= px(7.),
        "quick bar must hug its controls"
    );
    let p = cx.debug_bounds("priority-B").unwrap().center();
    cx.simulate_mouse_move(p, None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    click(cx, "priority-B");
    assert_eq!(source(&editor, cx), original.replacen("[#A]", "[#B]", 1));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(source(&editor, cx), original);
    cx.simulate_mouse_move(gpui::point(px(600.), px(400.)), None, Modifiers::default());
    hover(&editor, 0, 9, cx);
    cx.simulate_keystrokes("c");
    cx.run_until_parked();
    assert!(source(&editor, cx).starts_with("* TODO [#C] 中文任务"));
    cx.simulate_mouse_move(gpui::point(px(600.), px(400.)), None, Modifiers::default());
    hover(&editor, 0, 9, cx);
    click(cx, "priority-remove");
    assert!(source(&editor, cx).starts_with("* TODO 中文任务"));
}
#[gpui::test]
fn tags_distinguish_inheritance_search_add_apply_and_undo(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "#+FILETAGS: :global:\n#+TAGS: other(o)\n* Parent :work:\n** Child :home:\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("tags.org"), original.as_bytes().to_vec()).unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.run_until_parked();
    let p = point(&editor, 3, 11, cx);
    cx.simulate_click(p, Modifiers::default());
    cx.run_until_parked();
    cx.read(|cx| {
        let popup = editor.read(cx).inline_actions.popup.as_ref().unwrap();
        let InlineValue::Tags(TagsValue {
            local,
            inherited,
            available,
        }) = &popup.picker.read(cx).value
        else {
            panic!()
        };
        assert_eq!(local, &["home"]);
        assert_eq!(inherited, &["global", "work"]);
        assert!(available.contains(&"other".into()));
    });
    cx.simulate_keystrokes("new");
    cx.run_until_parked();
    click(cx, "tag-add");
    click(cx, "tag-home");
    click(cx, "inline-apply");
    assert_eq!(
        source(&editor, cx),
        original.replace("Child :home:", "Child :new:")
    );
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(source(&editor, cx), original);
}
#[gpui::test]
fn link_card_previews_copies_and_edits_only_destination(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "* Links\n[[#target][跳转]]\n[[https://example.com][网站]]\n* Target\n:PROPERTIES:\n:CUSTOM_ID: target\n:END:\nA preview line\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("links.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.run_until_parked();
    hover(&editor, 1, 4, cx);
    cx.read(|cx| {
        let popup = editor.read(cx).inline_actions.popup.as_ref().unwrap();
        let InlineValue::Link(LinkValue {
            preview, can_open, ..
        }) = &popup.picker.read(cx).value
        else {
            panic!()
        };
        assert!(*can_open);
        assert_eq!(preview, &["A preview line"]);
        assert!(matches!(&popup.picker.read(cx).value, InlineValue::Link(LinkValue { title: Some(title),.. }) if title == "Target"));
    });
    click(cx, "link-copy");
    cx.read(|cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "#target"));
    hover(&editor, 2, 6, cx);
    let card = cx.debug_bounds("inline-picker").unwrap();
    let actions = cx.debug_bounds("link-actions").unwrap();
    assert!(
        card.bottom() - actions.bottom() <= px(2.),
        "no empty block below link actions"
    );
    assert!(card.size.height < px(85.));
    click(cx, "link-edit");
    cx.simulate_keystrokes("cmd-a https://orgmode.org");
    click(cx, "inline-apply");
    assert_eq!(
        source(&editor, cx),
        original.replace("https://example.com", "https://orgmode.org")
    );
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(source(&editor, cx), original);
}
#[gpui::test]
fn links_have_continuous_hover_hits_and_leave_the_next_link_reachable(cx: &mut TestAppContext) {
    cx.update(init);
    let original =
        "* Links\n[[https://example.com][中文链接预览]]\n[[https://orgmode.org][下一条链接]]\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("hover.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(gpui::size(px(800.), px(600.)));
    cx.run_until_parked();
    hover(&editor, 1, 3, cx);
    let (identity, bounds) = cx.read(|cx| {
        let p = editor.read(cx).inline_actions.popup.as_ref().unwrap();
        (p.picker.entity_id(), p.bounds.unwrap())
    });
    let start = point(&editor, 1, 0, cx);
    let end = point(&editor, 1, original.lines().nth(1).unwrap().len(), cx);
    for step in 0..((f32::from(end.x - start.x) - 3.) / 2.) as usize {
        let position = gpui::point(start.x + px(step as f32 * 2.), start.y);
        cx.simulate_mouse_move(position, None, Modifiers::default());
        cx.executor().advance_clock(Duration::from_millis(360));
        cx.run_until_parked();
        cx.read(|cx| {
            let e = editor.read(cx);
            assert!(e.inline_hit(position, cx).is_some(), "hole at {position:?}");
            assert_eq!(
                e.inline_actions.popup.as_ref().unwrap().picker.entity_id(),
                identity
            );
        });
    }
    let next = point(&editor, 2, 3, cx);
    assert!(
        !bounds.contains(&next),
        "preview must leave the downward mouse path open"
    );
    cx.simulate_mouse_move(next, None, Modifiers::default());
    cx.run_until_parked();
    cx.read(|cx| {
        let p = editor.read(cx).inline_actions.popup.as_ref().unwrap();
        assert!(p.hit.source.contains("orgmode.org"));
    });
    let card = cx.debug_bounds("link-copy").unwrap().center();
    cx.simulate_mouse_move(card, None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    assert!(cx.debug_bounds("link-copy").is_some());
}

#[gpui::test]
fn inherited_tags_are_visible_above_footer_and_hover_uses_existing_pills(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "#+FILETAGS: :studio:\n#+TAGS: design home review work\n* Parent :work:\n** Child :home:review:\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(
            PathBuf::from("tag-layout.org"),
            original.as_bytes().to_vec(),
        )
        .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(gpui::size(px(700.), px(450.)));
    cx.run_until_parked();
    let p = point(&editor, 3, 12, cx);
    cx.simulate_mouse_move(p, None, Modifiers::default());
    cx.run_until_parked();
    cx.read(|cx| {
        let e = editor.read(cx);
        assert!(e.inline_tag_highlight().is_some());
        assert!(e.inline_background_highlight().is_none());
    });
    cx.simulate_click(p, Modifiers::default());
    cx.run_until_parked();
    let list = cx.debug_bounds("tag-options").unwrap();
    let last = cx.debug_bounds("tag-review").unwrap();
    assert!(
        last.bottom() <= list.bottom(),
        "last local tag should also fit without a tiny scroll"
    );
    let footer = cx.debug_bounds("inline-apply").unwrap();
    let inherited = cx.debug_bounds("inherited-tags").unwrap();
    for tag in ["inherited-tag-studio", "inherited-tag-work"] {
        let chip = cx.debug_bounds(tag).unwrap();
        assert!(chip.bottom() <= inherited.bottom() && chip.top() >= inherited.top());
        assert!(chip.bottom() < footer.top());
    }
    cx.read(|cx| assert!(editor.read(cx).inline_background_highlight().is_none()));
}

#[gpui::test]
fn custom_priority_bounds_and_tag_cancel_leave_source_intact(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "#+PRIORITIES: A E C\n* TODO [#D] Task :home:\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("custom.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(gpui::size(px(280.), px(250.)));
    cx.run_until_parked();
    hover(&editor, 1, 9, cx);
    cx.read(|cx| {
        let e = editor.read(cx);
        let popup = e.inline_actions.popup.as_ref().unwrap();
        let InlineValue::Priority(PriorityValue { current, choices }) =
            &popup.picker.read(cx).value
        else {
            panic!()
        };
        assert_eq!(current, "D");
        assert_eq!(choices, &["A", "B", "C", "D", "E"]);
        let bounds = popup.bounds.unwrap();
        let viewport = e.viewport.unwrap();
        assert!(bounds.left() >= viewport.left() && bounds.right() <= viewport.right());
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(source(&editor, cx), original);
    let p = point(&editor, 1, 20, cx);
    cx.simulate_click(p, Modifiers::default());
    cx.run_until_parked();
    click(cx, "tag-home");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(source(&editor, cx), original);
    cx.read(|cx| assert!(editor.read(cx).inline_actions.popup.is_none()));
}

#[gpui::test]
fn wrapped_link_hit_anchors_to_visual_row_and_rejects_stale_edit(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "* Links\n[[https://example.com/long/path][This is a long link description that wraps across several visual rows]]\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(PathBuf::from("wrapped.org"), original.as_bytes().to_vec())
            .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(gpui::size(px(340.), px(500.)));
    cx.run_until_parked();
    hover(&editor, 1, 65, cx);
    cx.read(|cx| {
        let e = editor.read(cx);
        let popup = e.inline_actions.popup.as_ref().unwrap();
        assert!(
            popup.hit.bounds.top() > e.hit_rows.iter().find(|r| r.line.0 == 1).unwrap().origin_y
        );
        assert_eq!(popup.hit.source, original.lines().nth(1).unwrap());
    });
    click(cx, "link-edit");
    editor.update(cx, |e, cx| {
        let revision = e.snapshot(cx).revision();
        e.apply_inline_edits(
            revision,
            vec![TextEdit::new(ByteRange::new(0, 0), "New\n".to_owned())],
            None,
            cx,
        );
        e.inline_event(&InlineEvent::Link("https://wrong.invalid".into()), cx);
    });
    cx.run_until_parked();
    assert_eq!(source(&editor, cx), format!("New\n{original}"));
    cx.read(|cx| assert!(editor.read(cx).inline_actions.popup.is_none()));
}
