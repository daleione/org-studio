use super::*;

#[gpui::test]
fn measured_layout_fits_both_languages_and_has_no_empty_columns(cx: &mut gpui::TestAppContext) {
    let (_, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    cx.update(|window, _| {
        for title in ["WWWWWWWWWWWW", "切换当前文档的阅读模式"] {
            let items = (0..4)
                .map(|index| labels::Item {
                    key: format!("C-{index}"),
                    title: title.into(),
                    group: labels::Group::Commands,
                    prefix: false,
                    disabled: false,
                })
                .collect();
            let layout = layout::Layout::new(items, 1200., 800., window);
            assert_eq!(layout.rows.len(), 1);
            assert_eq!(
                layout.rows[0].columns.len(),
                2,
                "four items use two filled columns"
            );
            let run = gpui::TextRun {
                len: title.len(),
                font: gpui::font(layout::FONT),
                color: gpui::rgb(0).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let text_width = f32::from(
                window
                    .text_system()
                    .shape_line(title.into(), gpui::px(layout::FONT_SIZE), &[run], None)
                    .width,
            );
            let space = layout.column_width
                - layout.key_width
                - layout::ITEM_GAP
                - 2. * layout::ROW_PADDING;
            assert!(
                space >= text_width,
                "localized titles must fit their allocated space"
            );
            assert_eq!(layout.rows[0].columns[0].items, 0..2);
            assert_eq!(layout.rows[0].columns[1].items, 2..4);
        }
    });
}

#[gpui::test]
fn save_menu_interrupts_both_delayed_and_visible_prefixes(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    w.update(cx, |w, cx| w.create_buffer("notes.org".into(), None, cx));
    cx.run_until_parked();
    for delay in [100, 401] {
        cx.simulate_keystrokes("ctrl-x");
        cx.executor().advance_clock(Duration::from_millis(delay));
        cx.run_until_parked();
        cx.update(|window, cx| window.dispatch_action(Box::new(crate::app::SaveBuffers), cx));
        cx.executor().advance_clock(Duration::from_millis(401));
        cx.run_until_parked();
        w.update(cx, |w, _| {
            assert!(w.keyboard.pending_keys().is_none());
            assert!(w.prefix_hint.task.is_none());
            assert!(!w.prefix_hint_visible());
            assert!(w.buffers.panel.is_some());
            assert!(w.status.shell_owns(ShellKind::Buffers, PaneSide::Left));
        });
        cx.simulate_keystrokes("ctrl-g");
        cx.run_until_parked();
        w.update(cx, |w, _| {
            assert!(w.buffers.panel.is_none(), "one C-g must close the review")
        });
    }
}

#[gpui::test]
fn view_menu_ends_the_prefix_and_restores_typing(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    w.update(cx, |w, cx| w.create_buffer("notes.org".into(), None, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-c");
    cx.executor().advance_clock(Duration::from_millis(401));
    cx.run_until_parked();
    cx.update(|window, cx| window.dispatch_action(Box::new(crate::preview::ShowEditor), cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("a");
    w.update(cx, |w, cx| {
        assert!(w.keyboard.pending_keys().is_none());
        assert!(w.prefix_hint.task.is_none());
        assert!(w.document_session().unwrap().read(cx).is_dirty());
    });
}

#[gpui::test]
fn prefix_scroll_does_not_move_the_document(cx: &mut gpui::TestAppContext) {
    use crate::document::{ByteOffset, ByteRange, EditTransaction, TextEdit};
    use gpui::{point, px, size};
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    cx.simulate_resize(size(px(390.), px(600.)));
    let editor = w.update(cx, |w, cx| {
        w.create_buffer("notes.org".into(), None, cx);
        w.document_session().unwrap().update(cx, |s, cx| {
            s.apply_transient_edit(
                EditTransaction::new(
                    s.revision(),
                    vec![TextEdit::new(
                        ByteRange::new(0, 0),
                        "Scrollable text\n".repeat(200),
                    )],
                ),
                cx,
            )
            .unwrap();
        });
        w.editor(PaneSide::Left).unwrap()
    });
    editor.update(cx, |e, cx| e.scroll_to_source_offset(ByteOffset(0), cx));
    cx.run_until_parked();
    let origin = editor.update(cx, |e, cx| e.top_source_anchor(&e.snapshot(cx)));
    cx.simulate_keystrokes("ctrl-x");
    cx.executor().advance_clock(Duration::from_millis(401));
    cx.run_until_parked();
    let bounds = cx.debug_bounds("floating-status-line").unwrap();
    for (position, delta) in [
        (bounds.center(), -80.),
        (bounds.center(), -10000.),
        (bounds.center(), -80.),
        (bounds.center(), 10000.),
        (point(bounds.center().x, bounds.bottom() - px(20.)), -80.),
    ] {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(delta))),
            touch_phase: gpui::TouchPhase::Moved,
            ..Default::default()
        });
        cx.run_until_parked();
        assert_eq!(
            editor.update(cx, |e, cx| e.top_source_anchor(&e.snapshot(cx))),
            origin
        );
    }
}

#[gpui::test]
fn prefix_discovery_waits_then_descends_and_returns_to_status(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    cx.simulate_resize(gpui::size(gpui::px(1200.), gpui::px(800.)));
    w.update(cx, |w, cx| w.create_buffer("notes.org".into(), None, cx));
    cx.run_until_parked();
    let status = cx.debug_bounds("floating-status-line").unwrap();
    cx.simulate_keystrokes("ctrl-c");
    cx.run_until_parked();
    w.update(cx, |w, _| assert!(!w.prefix_hint_visible()));
    cx.executor().advance_clock(Duration::from_millis(401));
    cx.run_until_parked();
    w.update(cx, |w, _| {
        let p = w.prefix_hint.presentation.as_ref().unwrap();
        assert_eq!(p.prefix, "C-c");
        assert_eq!(p.candidates.len(), w.keyboard.which_key_candidates().len());
    });
    let root = cx.debug_bounds("floating-status-line").unwrap();
    assert!(root.size.height > status.size.height);
    assert_eq!(root.bottom(), status.bottom());
    cx.simulate_keystrokes("v");
    cx.run_until_parked();
    w.update(cx, |w, _| {
        assert_eq!(w.prefix_hint.presentation.as_ref().unwrap().prefix, "C-c v")
    });
    assert!(cx.debug_bounds("floating-status-line").unwrap().size.height < root.size.height);
    cx.simulate_keystrokes("ctrl-g");
    cx.run_until_parked();
    assert_eq!(cx.debug_bounds("floating-status-line").unwrap(), status);
    w.update(cx, |w, cx| {
        assert!(!w.prefix_hint_visible());
        assert!(w.keyboard.status().is_none());
        assert!(!w.document_session().unwrap().read(cx).is_dirty());
    });
}

#[gpui::test]
fn prefix_hands_shell_to_buffer_picker_and_home_has_no_idle_bar(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    w.update(cx, |w, cx| w.create_buffer("notes.org".into(), None, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-x");
    cx.executor().advance_clock(Duration::from_millis(401));
    cx.run_until_parked();
    w.update(cx, |w, _| assert!(w.prefix_hint_visible()));
    cx.simulate_keystrokes("b");
    cx.run_until_parked();
    w.update(cx, |w, _| {
        assert!(!w.prefix_hint_visible());
        assert!(w.buffers.panel.is_some());
    });
    cx.simulate_keystrokes("ctrl-g");
    w.update(cx, |w, cx| w.show_home_now(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("floating-status-line").is_none());
    cx.simulate_keystrokes("ctrl-x");
    cx.executor().advance_clock(Duration::from_millis(401));
    cx.run_until_parked();
    assert!(cx.debug_bounds("prefix-hint-items").is_some());
    cx.simulate_keystrokes("ctrl-g");
    cx.run_until_parked();
    assert!(cx.debug_bounds("floating-status-line").is_none());
}

#[gpui::test]
fn localized_prefix_fits_narrow_windows_and_escape_preserves_fullscreen(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(crate::editor::init);
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        cx.intercept_keystrokes(WorkspaceWindow::intercept_fullscreen_escape)
            .detach();
    });
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    w.update(cx, |w, cx| w.create_buffer("notes.org".into(), None, cx));
    cx.update(|window, _| window.toggle_fullscreen());
    cx.simulate_resize(gpui::size(gpui::px(390.), gpui::px(600.)));
    for language in [
        crate::i18n::Language::Chinese,
        crate::i18n::Language::English,
    ] {
        w.update(cx, |w, _| w.language = language);
        cx.simulate_keystrokes("ctrl-x");
        cx.executor().advance_clock(Duration::from_millis(401));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("floating-status-line").unwrap();
        assert!(f32::from(bounds.left()) >= 20.);
        assert!(f32::from(bounds.right()) <= 370.);
        w.update(cx, |w, _| {
            let items = w.prefix_items(w.prefix_hint.presentation.as_ref().unwrap());
            assert!(
                items
                    .iter()
                    .any(|i| i.title == labels::text(language, "打开文件", "Open file"))
            );
            assert!(
                items.iter().any(|i| i.key == "C-s"
                    && i.title == labels::text(language, "保存文件", "Save file"))
            );
            assert!(
                items
                    .iter()
                    .any(|i| i.key == "C-w"
                        && i.title == labels::text(language, "另存为", "Save as"))
            );
        });
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        cx.update(|window, _| assert!(window.is_fullscreen()));
        w.update(cx, |w, _| assert!(w.keyboard.pending_keys().is_none()));
    }
}

#[gpui::test]
fn clicking_nested_hint_executes_the_actual_binding(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    w.update(cx, |w, cx| w.create_buffer("notes.org".into(), None, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-c");
    cx.executor().advance_clock(Duration::from_millis(401));
    cx.run_until_parked();
    let view_key = cx.debug_bounds("prefix-hint-key-v").unwrap();
    cx.simulate_click(view_key.center(), Default::default());
    cx.run_until_parked();
    let edit = cx.debug_bounds("prefix-hint-key-e").unwrap();
    let read = cx.debug_bounds("prefix-hint-key-r").unwrap();
    let split = cx.debug_bounds("prefix-hint-key-s").unwrap();
    assert_eq!(edit.top(), read.top());
    assert_eq!(
        read.top(),
        split.top(),
        "compact choices must stay in one row"
    );
    cx.simulate_click(split.center(), Default::default());
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(w.document_workspace.is_split());
        assert!(w.keyboard.pending_keys().is_none());
        assert!(!w.document_session().unwrap().read(cx).is_dirty());
    });
}
