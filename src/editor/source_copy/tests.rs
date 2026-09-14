use super::*;

#[gpui::test]
fn copies_org_and_markdown_code_without_moving_selection(cx: &mut gpui::TestAppContext) {
    cx.update(init);
    for (path, source, expected) in [
        (
            "copy.org",
            "* Code\n#+begin_src rust\n  中文();\n\n#+end_src\n",
            "  中文();\n\n",
        ),
        (
            "copy.md",
            "# Code\n````rust\n  中文();\n```\n````\n",
            "  中文();\n```\n",
        ),
        ("copy.md", "# Code\n~~~text\nlast line", "last line"),
        ("copy.org", "* Code\n#+begin_src text\n#+end_src\n", ""),
    ] {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from(path), source.as_bytes().to_vec()).unwrap()
        });
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        view.simulate_resize(gpui::size(px(640.), px(400.)));
        view.run_until_parked();
        editor.update(view, |e, cx| {
            e.selection = Selection::new(ByteOffset(4), ByteOffset(0));
            e.sync_selection_utf16(&e.snapshot(cx));
        });
        let (hit, selection, scroll) = view.read(|cx| {
            let e = editor.read(cx);
            assert!(
                !e.source_copy.buttons.is_empty(),
                "missing copy icon in {path}: {source}"
            );
            (e.source_copy.buttons[0], e.selection, e.scroll_y)
        });
        assert!(hit.bounds.right() <= px(640. - 26.));
        assert_eq!(hit.bounds.size, gpui::size(px(26.), px(26.)));
        view.simulate_mouse_move(hit.bounds.center(), None, gpui::Modifiers::default());
        view.run_until_parked();
        assert!(view.debug_bounds("source-copy-tooltip").is_none());
        view.executor().advance_clock(Duration::from_millis(501));
        view.run_until_parked();
        assert!(view.debug_bounds("source-copy-tooltip").is_some());
        view.simulate_click(hit.bounds.center(), gpui::Modifiers::default());
        view.run_until_parked();
        view.read(|cx| {
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .unwrap_or_default(),
                expected,
                "wrong copied body for {path}: {source}"
            );
            let e = editor.read(cx);
            assert_eq!(e.selection, selection);
            assert_eq!(e.scroll_y, scroll);
            assert_eq!(e.source_copy.feedback, Some((hit.revision, hit.offset)));
            assert_eq!(
                e.snapshot(cx)
                    .copy_range(ByteRange::new(0, source.len() as u64)),
                source
            );
        });
        view.executor().advance_clock(Duration::from_millis(1601));
        view.run_until_parked();
        view.read(|cx| assert!(editor.read(cx).source_copy.feedback.is_none()));
        view.simulate_mouse_move(
            gpui::point(px(10.), px(10.)),
            None,
            gpui::Modifiers::default(),
        );
        view.run_until_parked();
        assert!(view.debug_bounds("source-copy-tooltip").is_none());
    }
}
