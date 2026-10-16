use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext};

fn sample(
    editor: &Entity<SemanticEditor>,
    view: &VisualTestContext,
    line: u64,
) -> (Vec<f32>, usize) {
    view.read(|cx| {
        let editor = editor.read(cx);
        let row = editor
            .hit_rows
            .iter()
            .find(|row| row.line.0 == line)
            .unwrap();
        let layout = row.table_layout.as_ref().unwrap();
        (
            layout
                .column_widths
                .iter()
                .copied()
                .map(f32::from)
                .collect(),
            layout.visual_rows,
        )
    })
}

fn boundary(
    editor: &Entity<SemanticEditor>,
    view: &VisualTestContext,
    line: u64,
    column: usize,
) -> Point<Pixels> {
    view.read(|cx| {
        let editor = editor.read(cx);
        let row = editor
            .hit_rows
            .iter()
            .find(|row| row.line.0 == line)
            .unwrap();
        let snapshot = editor.snapshot(cx);
        let format = DocumentFormat::from_path(editor.session.read(cx).syntax_path());
        let (x, width) = delimiter_positions(row, &snapshot, format)[column + 1];
        gpui::point(row.text_origin_x + x + width / 2., row.origin_y + px(5.))
    })
}

fn start_drag(view: &mut VisualTestContext, start: Point<Pixels>, delta: f32) {
    view.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    view.simulate_mouse_move(
        gpui::point(start.x + px(delta), start.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    view.run_until_parked();
}

fn drag(view: &mut VisualTestContext, start: Point<Pixels>, delta: f32) {
    start_drag(view, start, delta);
    view.simulate_mouse_up(
        gpui::point(start.x + px(delta), start.y),
        MouseButton::Left,
        Modifiers::default(),
    );
    view.run_until_parked();
}

fn text(editor: &Entity<SemanticEditor>, view: &VisualTestContext) -> String {
    view.read(|cx| {
        let snapshot = editor.read(cx).snapshot(cx);
        snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
    })
}

fn fixture<'a>(
    cx: &'a mut TestAppContext,
    extension: &str,
) -> (Entity<SemanticEditor>, &'a mut VisualTestContext) {
    let separator = if extension == "org" {
        "|---+---+---|"
    } else {
        "|---|---|---|"
    };
    let source = format!(
        "| Left | Right | Third |\n{separator}\n| alpha beta gamma delta epsilon | one two three four five six | x |\n\nbody\n"
    );
    let session = cx.new(|_| {
        DocumentSession::from_utf8(format!("resize.{extension}").into(), source.into_bytes())
            .unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    editor.update(view, |editor, cx| editor.set_minimap(false, None, cx));
    view.simulate_resize(gpui::size(px(900.), px(500.)));
    view.run_until_parked();
    view.simulate_keystrokes("ctrl-shift-a");
    view.run_until_parked();
    (editor, view)
}

fn mixed_width_fixture<'a>(
    cx: &'a mut TestAppContext,
    extension: &str,
) -> (Entity<SemanticEditor>, &'a mut VisualTestContext) {
    let rule = if extension == "org" {
        "|---+---|"
    } else {
        "|---|---|"
    };
    let source =
        format!("| Labels | Value |\n{rule}\n| 中文示例 | sample text |\n| item | demo |\n");
    let session = cx.new(|_| {
        DocumentSession::from_utf8(format!("mixed.{extension}").into(), source.into_bytes())
            .unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    editor.update(view, |editor, cx| editor.set_minimap(false, None, cx));
    view.simulate_resize(gpui::size(px(900.), px(500.)));
    view.run_until_parked();
    view.simulate_keystrokes("tab");
    view.run_until_parked();
    (editor, view)
}

#[gpui::test]
fn tab_fits_mixed_width_columns_to_visible_content(cx: &mut TestAppContext) {
    cx.update(init);
    for extension in ["org", "md"] {
        let (editor, view) = mixed_width_fixture(cx, extension);
        for size in [15, 14] {
            editor.update(view, |editor, cx| {
                editor.set_content_font_size(crate::typography::ContentFontSize::new(size), cx);
            });
            view.simulate_keystrokes("tab");
            view.run_until_parked();
            view.read(|cx| {
                let editor = editor.read(cx);
                let mut longest = [px(0.); 2];
                for row in editor.hit_rows.iter().filter(|row| row.line.0 != 1) {
                    let Some(layout) = &row.table_layout else {
                        continue;
                    };
                    assert_eq!(layout.visual_rows, 1, "{extension}, font size {size}");
                    for (width, fragment) in longest.iter_mut().zip(
                        layout
                            .fragments
                            .iter()
                            .filter(|fragment| !fragment.delimiter),
                    ) {
                        *width = (*width).max(fragment.layout.width());
                    }
                }
                let layout = editor.hit_rows[0].table_layout.as_ref().unwrap();
                for (width, longest) in layout.column_widths.iter().zip(longest) {
                    assert!(
                        (f32::from(*width - longest - layout.metrics.space_width * 2.)).abs() < 0.1
                    );
                }
            });
        }
    }
}

#[gpui::test]
fn measured_columns_preserve_authored_right_alignment(cx: &mut TestAppContext) {
    cx.update(init);
    let source = "| 名     | v        |\n|--------+----------|\n|      x |    value |\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("aligned.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.run_until_parked();
    view.read(|cx| {
        let editor = editor.read(cx);
        let row = editor.hit_rows.iter().find(|row| row.line.0 == 2).unwrap();
        let table = row.table_layout.as_ref().unwrap();
        for cell in table
            .fragments
            .windows(2)
            .filter(|pair| !pair[0].delimiter && pair[1].delimiter)
        {
            assert!(
                (f32::from(
                    cell[1].x - cell[0].x - cell[0].layout.width() - table.metrics.space_width
                ))
                .abs()
                    < 0.1
            );
        }
    });
    assert_eq!(text(&editor, view), source);
}

#[gpui::test]
fn drag_writes_source_and_wraps_then_format_repairs_narrow_columns(cx: &mut TestAppContext) {
    cx.update(init);
    for extension in ["org", "md"] {
        let (editor, view) = fixture(cx, extension);
        let original = text(&editor, view);
        let selection = view.read(|cx| editor.read(cx).selection);
        let revision = view.read(|cx| editor.read(cx).snapshot(cx).revision());
        let (before, _) = sample(&editor, view, 2);
        let start = boundary(&editor, view, 2, 0);
        view.simulate_mouse_move(start, None, Modifiers::default());
        view.run_until_parked();
        view.read(|cx| {
            let editor = editor.read(cx);
            assert!(editor.table_resize_is_hovered());
            assert!(editor.hovered_table_cell().is_none());
        });
        drag(view, start, -80.);
        let (after, rows) = sample(&editor, view, 2);
        assert!((after[0] - (before[0] - 80.)).abs() < 6.);
        assert!((after[1] - (before[1] + 80.)).abs() < 6.);
        assert!((after[2] - before[2]).abs() < 0.1);
        assert!((after.iter().sum::<f32>() - before.iter().sum::<f32>()).abs() < 6.);
        assert!(rows > 1);
        for line in [0, 1] {
            assert_eq!(sample(&editor, view, line).0, after);
        }
        let resized = text(&editor, view);
        assert_ne!(resized, original);
        view.read(|cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.selection, selection);
            assert_ne!(editor.snapshot(cx).revision(), revision);
            assert!(editor.session.read(cx).is_dirty());
            let columns = editor
                .table_columns(
                    &editor.snapshot(cx),
                    LineIndex(0),
                    DocumentFormat::from_path(editor.session.read(cx).syntax_path()),
                )
                .unwrap();
            assert!(!columns.aligned);
            assert!(!editor.is_selecting);
            assert!(!editor.table_resize_is_dragging());
            assert!(
                editor.minimap.prepared_layout().is_some(),
                "complete layout also reflows with the minimap hidden"
            );
        });
        view.simulate_keystrokes("ctrl-shift-a");
        view.run_until_parked();
        assert!(sample(&editor, view, 2).0[0] > after[0]);
        view.read(|cx| {
            let editor = editor.read(cx);
            let columns = editor
                .table_columns(
                    &editor.snapshot(cx),
                    LineIndex(0),
                    DocumentFormat::from_path(editor.session.read(cx).syntax_path()),
                )
                .unwrap();
            assert!(columns.aligned);
        });
        view.simulate_keystrokes("cmd-z");
        view.run_until_parked();
        assert_eq!(text(&editor, view), resized);
        view.simulate_keystrokes("cmd-z");
        view.run_until_parked();
        assert_eq!(text(&editor, view), original);
        view.simulate_keystrokes("cmd-shift-z");
        view.run_until_parked();
        assert_eq!(text(&editor, view), resized);
    }
}

#[gpui::test]
fn escape_cancels_drag_and_double_click_restores_auto_widths(cx: &mut TestAppContext) {
    cx.update(init);
    let (editor, view) = fixture(cx, "org");
    let original = text(&editor, view);
    let selection = Selection::new(ByteOffset(5), ByteOffset(10));
    editor.update(view, |editor, cx| editor.set_selection(selection, cx));
    let before = sample(&editor, view, 2).0;
    let start = boundary(&editor, view, 2, 0);
    start_drag(view, start, -70.);
    view.simulate_keystrokes("escape");
    view.simulate_mouse_up(start, MouseButton::Left, Modifiers::default());
    view.run_until_parked();
    assert_eq!(sample(&editor, view, 2).0, before);
    view.read(|cx| assert_eq!(editor.read(cx).selection, selection));
    assert_eq!(text(&editor, view), original);

    let start = boundary(&editor, view, 2, 0);
    drag(view, start, -70.);
    let custom = sample(&editor, view, 2).0;
    let start = boundary(&editor, view, 2, 0);
    start_drag(view, start, 30.);
    view.simulate_keystrokes("escape");
    view.simulate_mouse_up(start, MouseButton::Left, Modifiers::default());
    view.run_until_parked();
    assert_eq!(
        sample(&editor, view, 2).0,
        custom,
        "cancel restores the previously saved widths"
    );
    let start = boundary(&editor, view, 2, 0);
    view.simulate_event(MouseDownEvent {
        position: start,
        button: MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    view.run_until_parked();
    assert_eq!(sample(&editor, view, 2).0, before);
    assert_eq!(text(&editor, view), original);
}

#[gpui::test]
fn drag_clamps_minimum_width_and_plain_click_does_not_save(cx: &mut TestAppContext) {
    cx.update(init);
    let (editor, view) = fixture(cx, "md");
    let original = text(&editor, view);
    let before = sample(&editor, view, 2).0;
    let start = boundary(&editor, view, 2, 0);
    view.simulate_click(start, Modifiers::default());
    view.run_until_parked();
    assert_eq!(text(&editor, view), original);
    drag(view, start, -10_000.);
    let after = sample(&editor, view, 2).0;
    view.read(|cx| assert!((after[0] - editor.read(cx).font_size_px() * 3.).abs() < 6.));
    assert!((before.iter().sum::<f32>() - after.iter().sum::<f32>()).abs() < 6.);
}

#[gpui::test]
fn custom_widths_adapt_to_window_resize_and_respect_disabled_wrapping(cx: &mut TestAppContext) {
    cx.update(init);
    let (editor, view) = fixture(cx, "org");
    let start = boundary(&editor, view, 2, 0);
    drag(view, start, -80.);
    let before = sample(&editor, view, 2).0;
    view.simulate_resize(gpui::size(px(450.), px(500.)));
    view.run_until_parked();
    let narrowed = sample(&editor, view, 2).0;
    assert!(narrowed.iter().sum::<f32>() < before.iter().sum::<f32>());
    assert!(
        narrowed
            .iter()
            .zip(&before)
            .all(|(narrow, wide)| narrow <= wide)
    );
    editor.update(view, |editor, cx| editor.set_soft_wrap(false, cx));
    view.run_until_parked();
    let (unwrapped, rows) = sample(&editor, view, 2);
    assert_eq!(rows, 1);
    assert!(unwrapped[0] > narrowed[0]);
    view.read(|cx| {
        let editor = editor.read(cx);
        let layout = editor
            .hit_rows
            .iter()
            .find(|row| row.line.0 == 2)
            .unwrap()
            .table_layout
            .as_ref()
            .unwrap();
        for fragment in layout
            .fragments
            .iter()
            .filter(|fragment| !fragment.delimiter)
        {
            assert!(
                fragment.layout.width() <= fragment.width + px(1.),
                "unwrapped text must not overlap the next column"
            );
        }
    });
}

#[gpui::test]
fn source_edits_above_table_keep_widths_until_explicit_alignment(cx: &mut TestAppContext) {
    cx.update(init);
    let (editor, view) = fixture(cx, "md");
    let start = boundary(&editor, view, 2, 0);
    drag(view, start, -60.);
    let widths = sample(&editor, view, 2).0;
    view.simulate_keystrokes("cmd-up");
    editor.update(view, |editor, cx| {
        editor.replace_selection("intro\n", EditOrigin::Other, cx)
    });
    view.run_until_parked();
    assert_eq!(sample(&editor, view, 3).0, widths);
    editor.update(view, |editor, cx| {
        let snapshot = editor.snapshot(cx);
        editor.set_selection(
            Selection::caret(snapshot.line_content_range(LineIndex(3)).unwrap().start),
            cx,
        );
    });
    view.simulate_keystrokes("ctrl-shift-a");
    view.run_until_parked();
    assert!(sample(&editor, view, 3).0[0] > widths[0]);
    view.simulate_keystrokes("cmd-z");
    view.run_until_parked();
    assert_eq!(sample(&editor, view, 3).0, widths);
    view.simulate_keystrokes("cmd-z");
    view.run_until_parked();
    assert_eq!(sample(&editor, view, 2).0, widths);
}

#[gpui::test]
fn raw_table_can_resize_but_pipes_in_code_cannot(cx: &mut TestAppContext) {
    cx.update(init);
    for (path, source, line, resizable) in [
        (
            "raw.org",
            "| Left column | Right column |\n|---+---|\n| alpha | beta |\n",
            0,
            true,
        ),
        (
            "code.org",
            "#+begin_src text\n| A | B |\n#+end_src\n",
            1,
            false,
        ),
        ("code.md", "```text\n| A | B |\n|---|---|\n```\n", 1, false),
    ] {
        let session = cx
            .new(|_| DocumentSession::from_utf8(path.into(), source.as_bytes().to_vec()).unwrap());
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        view.run_until_parked();
        let start = boundary(&editor, view, line, 0);
        view.simulate_mouse_move(start, None, Modifiers::default());
        view.run_until_parked();
        view.read(|cx| assert_eq!(editor.read(cx).table_resize_is_hovered(), resizable));
        if resizable {
            drag(view, start, 8.);
            assert!(sample(&editor, view, line).0.len() == 2);
            assert_ne!(text(&editor, view), source);
        }
    }
}

#[gpui::test]
fn right_edge_writes_width_and_format_preserves_an_aligned_table(cx: &mut TestAppContext) {
    cx.update(init);
    for extension in ["org", "md"] {
        let (editor, view) = fixture(cx, extension);
        let original = text(&editor, view);
        let before = sample(&editor, view, 2).0;
        let start = boundary(&editor, view, 2, 2);
        view.simulate_mouse_move(start, None, Modifiers::default());
        view.run_until_parked();
        view.read(|cx| {
            assert_eq!(editor.read(cx).table_resize.hover.unwrap().column, 2);
            assert!(editor.read(cx).hovered_table_cell().is_none());
        });
        drag(view, start, 60.);
        let expanded = sample(&editor, view, 2).0;
        for column in 0..2 {
            assert!((expanded[column] - before[column]).abs() < 0.1);
        }
        assert!((expanded[2] - before[2] - 60.).abs() < 6.);
        for line in [0, 1] {
            assert_eq!(sample(&editor, view, line).0, expanded);
        }
        let start = boundary(&editor, view, 2, 2);
        drag(view, start, -30.);
        let narrowed = sample(&editor, view, 2).0;
        assert!((narrowed[2] - expanded[2] + 30.).abs() < 6.);
        assert_eq!(&narrowed[..2], &expanded[..2]);
        let resized = text(&editor, view);
        assert_ne!(resized, original);
        let revision = view.read(|cx| editor.read(cx).snapshot(cx).revision());
        view.simulate_keystrokes("ctrl-shift-a");
        view.run_until_parked();
        assert_eq!(text(&editor, view), resized);
        view.simulate_keystrokes("tab");
        view.run_until_parked();
        assert_eq!(text(&editor, view), resized);
        view.read(|cx| assert_eq!(editor.read(cx).snapshot(cx).revision(), revision));
    }
}

#[gpui::test]
fn right_edge_clamps_to_viewport_and_supports_cancel_and_reset(cx: &mut TestAppContext) {
    cx.update(init);
    let (editor, view) = fixture(cx, "org");
    let before = sample(&editor, view, 2).0;
    let start = boundary(&editor, view, 2, 2);
    start_drag(view, start, 40.);
    view.simulate_keystrokes("escape");
    view.simulate_mouse_up(start, MouseButton::Left, Modifiers::default());
    view.run_until_parked();
    assert_eq!(sample(&editor, view, 2).0, before);

    let start = boundary(&editor, view, 2, 2);
    drag(view, start, -10_000.);
    let minimum = sample(&editor, view, 2).0;
    view.read(|cx| assert!((minimum[2] - editor.read(cx).font_size_px() * 3.).abs() < 6.));

    let start = boundary(&editor, view, 2, 2);
    drag(view, start, 10_000.);
    let maximum = sample(&editor, view, 2).0;
    for column in 0..2 {
        assert!((maximum[column] - before[column]).abs() < 0.1);
    }
    view.read(|cx| {
        let editor = editor.read(cx);
        let layout = editor
            .hit_rows
            .iter()
            .find(|row| row.line.0 == 2)
            .unwrap()
            .table_layout
            .as_ref()
            .unwrap();
        assert!((f32::from(layout.width) - (editor.display_map.wrap_width() - 2.)).abs() < 6.);
    });
    let start = boundary(&editor, view, 2, 2);
    view.simulate_event(MouseDownEvent {
        position: start,
        button: MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    view.run_until_parked();
    assert_eq!(sample(&editor, view, 2).0, before);
}

#[gpui::test]
fn right_edge_can_resize_a_single_column_table(cx: &mut TestAppContext) {
    cx.update(init);
    for (path, source) in [
        (
            "single.org",
            "| Single column |\n|---------------|\n| value |\n",
        ),
        (
            "single.md",
            "| Single column |\n|---------------|\n| value |\n",
        ),
    ] {
        let session = cx
            .new(|_| DocumentSession::from_utf8(path.into(), source.as_bytes().to_vec()).unwrap());
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        view.run_until_parked();
        view.simulate_keystrokes("ctrl-shift-a");
        view.run_until_parked();
        let original = text(&editor, view);
        let before = sample(&editor, view, 2).0;
        let start = boundary(&editor, view, 2, 0);
        drag(view, start, 40.);
        let after = sample(&editor, view, 2).0;
        assert_eq!(after.len(), 1);
        assert!((after[0] - before[0] - 40.).abs() < 6.);
        assert_ne!(text(&editor, view), original);
    }
}

#[gpui::test]
fn resized_widths_survive_save_and_reopen_with_normal_dirty_and_undo_state(
    cx: &mut TestAppContext,
) {
    cx.update(init);
    let directory = tempfile::tempdir().unwrap();
    for extension in ["org", "md"] {
        for (column, delta) in [(0, -18.), (1, 36.)] {
            let (editor, view) = mixed_width_fixture(cx, extension);
            let path = directory.path().join(format!("table-{column}.{extension}"));
            let session = view.read(|cx| editor.read(cx).session.clone());
            let request = session.update(view, |session, _| {
                session.begin_save(Some(path.clone())).unwrap()
            });
            let outcome = crate::document::write_atomic(request).unwrap();
            session.update(view, |session, cx| {
                session.finish_save(outcome, cx).unwrap()
            });
            view.run_until_parked();
            let original = text(&editor, view);
            let start = boundary(&editor, view, 2, column);
            start_drag(view, start, delta);
            assert_eq!(
                text(&editor, view),
                original,
                "movement only previews the edit"
            );
            view.read(|cx| assert!(!session.read(cx).is_dirty()));
            view.simulate_mouse_up(
                gpui::point(start.x + px(delta), start.y),
                MouseButton::Left,
                Modifiers::default(),
            );
            view.run_until_parked();
            let resized = text(&editor, view);
            let (widths, rows) = sample(&editor, view, 2);
            if column == 0 {
                assert!(rows > 1);
            }
            view.read(|cx| assert!(session.read(cx).is_dirty()));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            view.simulate_keystrokes("cmd-z");
            view.run_until_parked();
            assert_eq!(text(&editor, view), original);
            view.read(|cx| assert!(!session.read(cx).is_dirty()));
            view.simulate_keystrokes("cmd-shift-z");
            view.run_until_parked();
            assert_eq!(text(&editor, view), resized);

            let request = session.update(view, |session, _| session.begin_save(None).unwrap());
            let outcome = crate::document::write_atomic(request).unwrap();
            session.update(view, |session, cx| {
                session.finish_save(outcome, cx).unwrap()
            });
            view.run_until_parked();
            view.read(|cx| assert!(!session.read(cx).is_dirty()));
            let saved = std::fs::read(&path).unwrap();
            assert_eq!(saved, resized.as_bytes());

            let reopened = cx.new(|_| DocumentSession::from_utf8(path.clone(), saved).unwrap());
            let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(reopened, cx));
            editor.update(view, |editor, cx| editor.set_minimap(false, None, cx));
            view.simulate_resize(gpui::size(px(900.), px(500.)));
            view.run_until_parked();
            assert_eq!(text(&editor, view), resized);
            assert_eq!(sample(&editor, view, 2), (widths, rows));
            view.read(|cx| assert!(!editor.read(cx).session.read(cx).is_dirty()));
        }
    }
}
