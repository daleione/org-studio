use super::*;
use crate::{
    app::{PanePair, ReadyDocument, WorkspaceLoadState},
    document::{
        ByteOffset, ByteRange, DocumentCommand, DocumentSession, EditOrigin, EditTransaction,
        HistoryOutcome, TextEdit, TextSnapshot,
    },
};
use gpui::{EntityInputHandler, prelude::*};
use std::sync::Arc;

fn install(
    w: &mut WorkspaceWindow,
    text: &str,
    cx: &mut Context<WorkspaceWindow>,
) -> Entity<DocumentSession> {
    let session = cx.new(|_| {
        DocumentSession::from_utf8("/tmp/command-test.org".into(), text.as_bytes().to_vec())
            .unwrap()
    });
    w.state = WorkspaceLoadState::Ready {
        document: ReadyDocument {
            session: session.clone(),
            editor_syntax: Arc::default(),
            editors: PanePair {
                left: None,
                right: None,
            },
            readers: PanePair {
                left: None,
                right: None,
            },
        },
    };
    w.ensure_editor_for(PaneSide::Left, cx);
    w.request_document_focus(cx);
    cx.notify();
    session
}

fn source(doc: &DocumentSession) -> String {
    let snapshot = doc.snapshot();
    snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
}

#[gpui::test]
fn command_line_number_shortcuts_follow_visible_rows_and_preserve_digit_input(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let original = "|a|long|\n\n|second|x|\n";
    let doc = w.update(cx, |w, cx| install(w, original, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("alt-x");
    cx.simulate_input("123");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert_eq!(w.command_line.session.as_ref().unwrap().query, "123");
        assert_eq!(source(doc.read(cx)), original);
    });
    cx.simulate_keystrokes("cmd-a backspace ctrl-9");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        let s = w.command_line.session.as_mut().unwrap();
        assert!(s.query.is_empty());
        assert!(!s.execute_pending);
        // Scroll the first candidate out of view: visible row 1 now means current table.
        s.selected = s.visible_limit;
        assert_eq!(s.candidates[s.visible_start()].input, "table-align");
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("command-candidate-0").is_none());
    assert!(cx.debug_bounds("command-candidate-number-1").is_some());
    cx.simulate_keystrokes("ctrl-1");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert_eq!(source(doc.read(cx)), "| a | long |\n\n|second|x|\n");
        assert!(matches!(
            w.command_line.session.as_ref().unwrap().phase,
            Phase::Result { .. }
        ));
    });
}

#[gpui::test]
fn command_line_keyboard_focus_completion_and_search_handoff(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let doc = w.update(cx, |w, cx| install(w, "", cx));
    cx.simulate_resize(gpui::size(gpui::px(1200.), gpui::px(800.)));
    cx.run_until_parked();
    let status = cx.debug_bounds("floating-status-line").unwrap();
    cx.simulate_keystrokes("cmd-shift-p");
    cx.run_until_parked();
    let expanded = cx.debug_bounds("command-line-shell").unwrap();
    assert_eq!(expanded.bottom(), status.bottom());
    assert_eq!(expanded.center().x, status.center().x);
    assert!(expanded.size.width <= gpui::px(600.));
    assert!(expanded.size.width < status.size.width);
    assert_eq!(
        cx.debug_bounds("command-candidate-0").unwrap().size.height,
        gpui::px(32.)
    );
    assert!(expanded.size.height > status.size.height);
    cx.simulate_input("table-align --a");
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        let s = w.command_line.session.as_ref().unwrap();
        assert_eq!(s.query, "table-align --all");
        assert_eq!(s.input.read(cx).text, s.query);
        assert!(!doc.read(cx).is_dirty());
    });
    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    cx.simulate_input("needle");
    w.update(cx, |_, cx| {
        assert!(!doc.read(cx).is_dirty());
    });
    w.update(cx, |w, _| {
        assert!(w.search_is_open());
        assert!(!w.command_line_is_open());
    });
    cx.simulate_keystrokes("alt-x");
    cx.run_until_parked();
    w.update(cx, |w, _| {
        assert!(!w.search_is_open());
        assert!(w.command_line_is_open());
    });
    let focus = w.update(cx, |w, cx| {
        w.command_line
            .session
            .as_ref()
            .unwrap()
            .input
            .read(cx)
            .focus
            .clone()
    });
    cx.update(|window, _| {
        assert!(
            focus.is_focused(window),
            "command input must own focus after leaving search"
        )
    });
    cx.simulate_keystrokes("ctrl-g");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(
            !w.command_line_is_open(),
            "after C-g: input={:?} composing={} surface={:?}",
            w.command_line
                .session
                .as_ref()
                .map(|s| s.input.read(cx).text.clone()),
            w.command_line
                .session
                .as_ref()
                .is_some_and(|s| s.input.read(cx).is_composing()),
            w.document_workspace.active_surface()
        )
    });
    cx.simulate_input(":");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(!w.command_line_is_open());
        assert_eq!(source(doc.read(cx)), ":");
    });
    w.update(cx, |w, cx| w.show_reading(cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("shift-;");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(w.command_line_is_open());
        assert_eq!(source(doc.read(cx)), ":");
    });
}

#[gpui::test]
fn command_line_align_all_is_one_undo_and_keeps_caret(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let original = "|a|中文|\n|long|b|\n\n* Folded\n|c|d|\n|e|length|\n";
    let caret = ByteOffset(original.find('文').unwrap() as u64);
    let doc = w.update(cx, |w, cx| {
        let doc = install(w, original, cx);
        w.editor(PaneSide::Left)
            .unwrap()
            .update(cx, |e, cx| e.set_selection(Selection::caret(caret), cx));
        doc
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input("table-align --all");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(matches!(
            w.command_line.session.as_ref().unwrap().phase,
            Phase::Result { undo: Some(_), .. }
        ));
        let formatted = source(doc.read(cx));
        assert!(formatted.contains("| a    | 中文 |"), "{formatted}");
        assert!(formatted.contains("| c | d      |"));
        let caret = w
            .editor(PaneSide::Left)
            .unwrap()
            .read(cx)
            .selection()
            .head()
            .0 as usize;
        assert!(formatted[caret..].starts_with('文'));
        assert!(doc.read(cx).is_dirty());
        w.undo_command_result(cx);
        assert_eq!(source(doc.read(cx)), original);
        assert_eq!(
            w.editor(PaneSide::Left).unwrap().read(cx).selection(),
            Selection::caret(ByteOffset(original.find('文').unwrap() as u64))
        );
        assert!(matches!(
            doc.update(cx, |d, cx| d.undo(cx)).unwrap(),
            HistoryOutcome::Empty
        ));
    });
    // Result feedback must not capture the next document shortcut.
    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    w.update(cx, |w, _| assert!(w.search_is_open()));
    cx.simulate_keystrokes("alt-x");
    cx.run_until_parked();
    w.update(cx, |w, _| {
        assert_eq!(
            w.command_line.session.as_ref().unwrap().candidates[0].input,
            "table-align --all"
        )
    });
}

#[gpui::test]
fn command_line_invalid_flags_and_ime_never_execute(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let doc = w.update(cx, |w, cx| install(w, "|a|b|\n", cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("alt-x");
    cx.simulate_input("table-align --wrong");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let input = w.update(cx, |w, cx| {
        let s = w.command_line.session.as_ref().unwrap();
        assert!(matches!(s.phase, Phase::Input));
        assert!(s.error.is_some());
        assert!(!doc.read(cx).is_dirty());
        s.input.clone()
    });
    cx.simulate_keystrokes("cmd-a backspace");
    cx.run_until_parked();
    w.update(cx, |_, cx| {
        assert_eq!(input.read(cx).text, "", "cleared input")
    });
    cx.update(|window, app| {
        input.update(app, |i, cx| {
            i.replace_and_mark_text_in_range(None, "对齐", Some(2..2), window, cx)
        })
    });
    // IMEs own the platform's Enter event while composing. Exercise the command dispatch guard.
    w.update(cx, |w, cx| {
        w.command_input_key("enter", Modifiers::default(), cx)
    });
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        let s = w.command_line.session.as_ref().unwrap();
        assert!(matches!(s.phase, Phase::Input));
        assert!(!s.execute_pending);
        assert!(!doc.read(cx).is_dirty());
    });
    cx.update(|window, app| {
        input.update(app, |i, cx| {
            i.replace_text_in_range(None, "对齐", window, cx)
        })
    });
    cx.run_until_parked();
    w.update(cx, |w, _| {
        assert_eq!(w.command_line.session.as_ref().unwrap().query, "对齐")
    });
}

#[gpui::test]
fn command_line_background_plan_rejects_a_changed_revision_and_cancellation(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(crate::editor::init);
    let w = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    let doc = w.update(cx, |w, cx| install(w, "|a|long|\n", cx));
    w.update(cx, |w, cx| {
        w.open_command_line(cx);
        w.start_table_alignment(crate::command::TableScope::Document, cx);
        doc.update(cx, |d, cx| {
            d.edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        d.revision(),
                        vec![TextEdit::new(ByteRange::new(0, 0), "changed\n")],
                    ),
                    Selection::default(),
                    Selection::default(),
                    EditOrigin::Other,
                ),
                cx,
            )
            .unwrap();
        });
    });
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(w.command_line.session.as_ref().unwrap().error.is_some());
        assert_eq!(source(doc.read(cx)), "changed\n|a|long|\n");
        w.start_table_alignment(crate::command::TableScope::Document, cx);
        w.close_command_line(cx);
    });
    cx.run_until_parked();
    w.update(cx, |_, cx| {
        assert_eq!(source(doc.read(cx)), "changed\n|a|long|\n")
    });
}

#[test]
fn command_line_catalog_resolves_aliases_scopes_and_availability() {
    let (commands, _, _) = crate::preview::document_input();
    let all = catalog::entries(
        &commands,
        crate::i18n::Language::Chinese,
        false,
        true,
        false,
    );
    for (query, scope) in [
        ("table-align", crate::command::TableScope::Current),
        (":table-align --all", crate::command::TableScope::Document),
        (
            "org-studio.table.align",
            crate::command::TableScope::Current,
        ),
    ] {
        assert_eq!(catalog::candidates(&all, query, &[])[0].scope, Some(scope));
    }
    assert_eq!(catalog::candidates(&all, "w", &[])[0].input, "save");
    assert_eq!(
        catalog::candidates(&all, "对齐 所有", &[])[0].input,
        "table-align --all"
    );
    assert!(catalog::candidates(&all, "--selection", &[]).is_empty());
    let selected = catalog::entries(&commands, crate::i18n::Language::Chinese, false, true, true);
    assert_eq!(
        catalog::candidates(&selected, "--selection", &[])[0].input,
        "table-align --selection"
    );
    let reading = catalog::entries(
        &commands,
        crate::i18n::Language::Chinese,
        false,
        false,
        false,
    );
    assert!(!reading.iter().any(|entry| matches!(
        entry.scope,
        Some(crate::command::TableScope::Current | crate::command::TableScope::Selection)
    )));
    assert!(
        reading
            .iter()
            .any(|entry| entry.input == "table-align --all")
    );
    // Typing the unavailable current-table command must not execute a wider scope.
    assert!(catalog::candidates(&reading, "table-align", &[]).is_empty());
    let read_only = catalog::entries(&commands, crate::i18n::Language::Chinese, true, true, true);
    assert!(read_only.iter().all(|entry| entry.scope.is_none()
        && !matches!(entry.input.as_str(), "save" | "reload" | "undo" | "redo")));
    assert!(
        catalog::candidates(&read_only, "", &["save".into()])
            .iter()
            .all(|entry| entry.input != "save")
    );
    assert!(catalog::validate("table-align --all --selection").is_err());
    assert!(catalog::validate("save --all").is_err());
}

#[gpui::test]
fn command_line_expiry_and_outgoing_controls_cannot_execute_or_undo(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let w = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    let doc = w.update(cx, |w, cx| {
        let doc = install(w, "|a|long|\n", cx);
        w.open_command_line(cx);
        w.start_table_alignment(crate::command::TableScope::Document, cx);
        doc
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    w.update(cx, |w, _| assert!(w.command_line_is_open()));
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(!w.command_line_is_open());
        let revision = doc.read(cx).revision();
        // Outgoing content stays mounted during the closing animation.
        w.undo_command_result(cx);
        assert_eq!(doc.read(cx).revision(), revision);
        w.open_command_line(cx);
    });
    cx.executor().advance_clock(Duration::from_secs(4));
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(
            w.command_line_is_open(),
            "an old result timer must not dismiss a new input"
        );
        w.close_command_line(cx);
        w.activate_command_candidate(0, cx);
        w.command_input_key("enter", Modifiers::default(), cx);
        assert!(!w.command_line.session.as_ref().unwrap().execute_pending);
        assert_eq!(source(doc.read(cx)), "| a | long |\n");
    });
}

#[gpui::test]
fn command_line_alignment_preserves_both_split_viewports(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(true));
    let original = (0..160)
        .map(|i| format!("* Section {i}\n|a|long value|\n|long name|x|\n\nText {i}\n\n"))
        .collect::<String>();
    let doc = w.update(cx, |w, cx| {
        let doc = install(w, &original, cx);
        w.document_workspace
            .set_surface(PaneSide::Right, PaneSurface::Reading);
        w.schedule_derived_update(cx);
        doc
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(25));
    cx.run_until_parked();
    let (editor, reader) = w.update(cx, |w, _| {
        (
            w.editor(PaneSide::Left).unwrap(),
            w.reading_panel_for(PaneSide::Right).unwrap(),
        )
    });
    editor.update(cx, |e, cx| {
        e.scroll_to_source_offset(
            ByteOffset(original.find("* Section 70\n").unwrap() as u64),
            cx,
        );
    });
    reader.update(cx, |r, cx| {
        r.scroll_to_source_offset_with_offset(
            ByteOffset(original.find("* Section 90\n").unwrap() as u64),
            gpui::px(7.),
        );
        cx.notify();
    });
    cx.run_until_parked();
    let editor_anchor = editor.update(cx, |e, cx| e.top_source_anchor(&e.snapshot(cx)));
    let reader_anchor = reader.update(cx, |r, _| r.top_source_anchor().unwrap());
    let expected = w.update(cx, |_, cx| {
        crate::editor::plan_table_alignment(
            std::path::Path::new("notes.org"),
            &doc.read(cx).snapshot(),
            crate::command::TableScope::Document,
            Selection::default(),
        )
        .unwrap()
    });
    cx.simulate_keystrokes("alt-x");
    cx.simulate_input("table-align --all");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let actual_editor = editor.update(cx, |e, cx| e.top_source_anchor(&e.snapshot(cx)));
    cx.executor().advance_clock(Duration::from_millis(25));
    cx.run_until_parked();
    let actual_reader = reader.update(cx, |r, _| r.top_source_anchor().unwrap());
    assert_eq!(actual_editor.0, expected.map_offset(editor_anchor.0));
    assert!((actual_editor.1 - editor_anchor.1).abs() < 0.05);
    assert_eq!(
        actual_reader,
        (expected.map_offset(reader_anchor.0), reader_anchor.1)
    );
}

#[gpui::test]
fn command_line_preview_mapping_respects_scroll_during_background_update(
    cx: &mut gpui::TestAppContext,
) {
    let original = (0..100)
        .map(|i| format!("* Heading {i}\nbody {i}\n"))
        .collect::<String>();
    let mut buffer = crate::document::DocumentBuffer::from_utf8(original.into_bytes()).unwrap();
    let path = std::path::PathBuf::from("pending-preview.org");
    let preview =
        crate::preview::derive_preview_incremental(path.clone(), buffer.snapshot(), None, &[]);
    let panel = cx.new(|_| crate::preview::ReadingPreviewPanel::new(Arc::new(preview), 80.));
    panel.update(cx, |p, _| {
        p.scroll_to(gpui::ListOffset {
            item_ix: 50,
            offset_in_item: gpui::px(3.),
        })
    });
    let delta = buffer
        .commit(EditTransaction::new(
            buffer.snapshot().revision(),
            vec![TextEdit::new(
                ByteRange::new(0, 0),
                "Inserted paragraph\n\n",
            )],
        ))
        .unwrap();
    panel.update(cx, |p, _| {
        p.map_viewport_through_delta(&delta);
        p.map_viewport_through_delta(&delta); // Duplicate derived notifications must be harmless.
        p.scroll_to(gpui::ListOffset {
            item_ix: 80,
            offset_in_item: gpui::px(9.),
        });
    });
    let anchor = panel.update(cx, |p, _| p.top_source_anchor().unwrap());
    let next = crate::preview::derive_preview_incremental(path, buffer.snapshot(), None, &[]);
    panel.update(cx, |p, cx| {
        p.replace_document_with_style(
            Arc::new(next),
            *crate::preview::preview_style(crate::preview::PreviewStyleId::Base),
            cx,
        );
        assert_eq!(
            p.top_source_anchor().unwrap(),
            (
                ByteOffset(anchor.0.0 + "Inserted paragraph\n\n".len() as u64),
                anchor.1
            )
        );
    });
}

#[gpui::test]
fn command_line_narrow_shell_scroll_and_noop_are_safe(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    cx.simulate_resize(gpui::size(gpui::px(390.), gpui::px(600.)));
    let doc = w.update(cx, |w, cx| install(w, &"ordinary text\n".repeat(200), cx));
    cx.run_until_parked();
    let (editor, before_revision) = w.update(cx, |w, cx| {
        (w.editor(PaneSide::Left).unwrap(), doc.read(cx).revision())
    });
    let anchor = editor.update(cx, |e, cx| e.top_source_anchor(&e.snapshot(cx)));
    let trigger = cx.debug_bounds("status-command").unwrap();
    cx.simulate_click(trigger.center(), Modifiers::default());
    cx.run_until_parked();
    let bounds = cx.debug_bounds("command-line-shell").unwrap();
    assert!(bounds.size.width <= gpui::px(350.));
    assert!(bounds.top() >= gpui::px(0.));
    let first_row = cx.debug_bounds("command-candidate-0").unwrap();
    assert!(first_row.top() >= bounds.top() + gpui::px(6.));
    assert!(first_row.left() >= bounds.left() + gpui::px(6.));
    assert!(first_row.right() <= bounds.right() - gpui::px(6.));
    assert!(
        (f32::from(first_row.top() - bounds.top()) - f32::from(first_row.left() - bounds.left()))
            .abs()
            <= 1.,
        "shell={bounds:?}, row={first_row:?}"
    );
    let input = cx.debug_bounds("command-line-input").unwrap();
    assert!(input.size.width > gpui::px(120.));
    // Equal distance must select the same row regardless of trackpad event frequency.
    for deltas in [
        vec![-1.; (2. * CANDIDATE_ROW_HEIGHT) as usize],
        vec![-CANDIDATE_ROW_HEIGHT / 2.; 4],
        vec![-2. * CANDIDATE_ROW_HEIGHT],
    ] {
        w.update(cx, |w, cx| w.command_query_changed(String::new(), cx));
        for delta in deltas {
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: bounds.center(),
                delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(delta))),
                touch_phase: gpui::TouchPhase::Moved,
                ..Default::default()
            });
        }
        cx.run_until_parked();
        w.update(cx, |w, _| {
            assert_eq!(w.command_line.session.as_ref().unwrap().selected, 2)
        });
    }
    // Discard unused distance at an edge, so reversing direction responds immediately.
    w.update(cx, |w, _| {
        let s = w.command_line.session.as_mut().unwrap();
        s.scroll_candidates(100.75 * CANDIDATE_ROW_HEIGHT, gpui::TouchPhase::Moved);
        let last = s.selected;
        s.scroll_candidates(-CANDIDATE_ROW_HEIGHT, gpui::TouchPhase::Moved);
        assert_eq!(s.selected, last - 1);
    });
    for _ in 0..20 {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: bounds.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-80.))),
            touch_phase: gpui::TouchPhase::Moved,
            ..Default::default()
        });
    }
    cx.run_until_parked();
    assert_eq!(
        editor.update(cx, |e, cx| e.top_source_anchor(&e.snapshot(cx))),
        anchor
    );
    cx.simulate_input("table-align --a");
    cx.run_until_parked();
    let candidate = cx.debug_bounds("command-candidate-0").unwrap();
    cx.simulate_click(candidate.center(), Modifiers::default());
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(matches!(
            w.command_line.session.as_ref().unwrap().phase,
            Phase::Result { undo: None, .. }
        ));
        assert_eq!(doc.read(cx).revision(), before_revision);
        assert!(!doc.read(cx).is_dirty());
    });
}
