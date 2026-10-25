use super::*;

fn block_row(edge: EditorBlockEdge, top: f32) -> EditorBlockPaintRow {
    EditorBlockPaintRow {
        block: Some(EditorBlockDecoration {
            kind: EditorBlockKind::Source,
            edge,
            body_line: (edge == EditorBlockEdge::Body).then_some(1),
        }),
        top,
        bottom: top + 28.0,
        active: edge == EditorBlockEdge::Body,
        folded: false,
    }
}

#[test]
fn source_block_chrome_covers_only_the_body() {
    let segments = editor_block_segments([
        block_row(EditorBlockEdge::Open, 0.0),
        block_row(EditorBlockEdge::Body, 28.0),
        block_row(EditorBlockEdge::Close, 56.0),
    ]);

    assert_eq!(segments.len(), 1);
    assert!(segments[0].open);
    assert!(segments[0].close);
    assert!(segments[0].active);
    assert_eq!(segments[0].top, 28.0);
    assert_eq!(segments[0].bottom, 56.0);
}

#[test]
fn empty_and_folded_source_blocks_do_not_frame_their_declarations() {
    assert!(
        editor_block_segments([
            block_row(EditorBlockEdge::Open, 0.0),
            block_row(EditorBlockEdge::Close, 28.0),
        ])
        .is_empty()
    );
    let mut folded = block_row(EditorBlockEdge::Open, 0.0);
    folded.folded = true;
    assert!(editor_block_segments([folded]).is_empty());

    let mut middle = block_row(EditorBlockEdge::Body, 0.0);
    middle.block.as_mut().unwrap().body_line = Some(8);
    let segments = editor_block_segments([middle, block_row(EditorBlockEdge::Close, 28.0)]);
    assert!(
        !segments[0].open,
        "a scrolled continuation has no top border"
    );
    assert!(segments[0].close);
    assert_eq!(segments[0].bottom, 28.0);
}

#[gpui::test]
fn org_source_block_actions_follow_first_body_line_and_hover(cx: &mut gpui::TestAppContext) {
    use crate::document::ByteOffset;
    use crate::editor::SourceRunPhase;

    cx.update(crate::editor::init);
    let source = concat!(
        "plain\n",
        "#+name: mysql-empty\n",
        "#+begin_src sql\n",
        "SELECT id, code, name, region, email, phone, address, created_at, updated_at\n",
        "FROM customers\n",
        "WHERE id = 999999;\n",
        "#+end_src\n",
        "\n",
        "#+begin_src rust\n",
        "fn main() {}\n",
        "#+end_src\n",
        "after\n",
    );
    let offset = ByteOffset(source.find("#+begin_src sql").unwrap() as u64);
    let session = cx.new(|_| {
        DocumentSession::from_utf8("actions.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.simulate_resize(gpui::size(px(420.0), px(800.0)));
    view.run_until_parked();
    let (run, body_point, declaration_point) = view.read(|cx| {
        let editor = editor.read(cx);
        let row = |line| {
            editor
                .hit_rows
                .iter()
                .find(|row| row.line.0 == line)
                .unwrap()
        };
        let document_left = row(0).text_origin_x;
        for line in [1, 2, 6, 8, 10, 11] {
            assert_eq!(row(line).text_origin_x, document_left);
        }
        assert!(row(3).text_origin_x > document_left);
        assert!(!row(3).layout.wrap_boundaries().is_empty());
        assert_eq!(editor.source_run_buttons.len(), 2);
        assert_eq!(editor.source_copy.buttons.len(), 2);
        assert!(
            editor
                .source_run_buttons
                .iter()
                .all(|button| !button.visible)
        );
        let run = editor.source_run_buttons[0];
        let copy = editor.source_copy.buttons[0];
        assert_eq!(run.source_offset, offset);
        assert_eq!(copy.offset, offset);
        for bounds in [run.bounds, copy.bounds] {
            assert!(bounds.top() >= row(3).origin_y);
            assert!(bounds.bottom() <= row(3).origin_y + row(3).line_height);
        }
        assert!(run.bounds.right() < row(3).text_origin_x);
        assert!(copy.bounds.left() > row(3).text_origin_x + row(3).visual_width());
        assert_eq!(run.body_bounds.left(), document_left);
        assert!(run.body_bounds.top() >= row(3).visible_top);
        assert!(run.body_bounds.bottom() <= row(6).visible_top);
        (
            run,
            gpui::point(row(5).text_origin_x + px(30.0), row(5).origin_y + px(10.0)),
            gpui::point(document_left + px(4.0), row(2).origin_y + px(10.0)),
        )
    });

    // Entering a later code line reveals the first line's action, without
    // revealing the neighboring block's action.
    view.simulate_mouse_move(body_point, None, Modifiers::default());
    view.run_until_parked();
    view.read(|cx| {
        let editor = editor.read(cx);
        assert_eq!(
            editor.source_run_hover,
            Some(crate::editor::SourceRunHover::Body(offset))
        );
        assert!(editor.source_run_buttons[0].visible);
        assert!(!editor.source_run_buttons[1].visible);
    });
    view.simulate_mouse_move(declaration_point, None, Modifiers::default());
    view.run_until_parked();
    view.read(|cx| {
        let editor = editor.read(cx);
        assert!(editor.source_run_hover.is_none());
        assert!(
            editor
                .source_run_buttons
                .iter()
                .all(|button| !button.visible)
        );
    });

    view.simulate_mouse_move(run.bounds.center(), None, Modifiers::default());
    view.run_until_parked();
    view.simulate_click(run.bounds.center(), Modifiers::default());
    view.run_until_parked();
    view.read(|cx| {
        let feedback = editor.read(cx).source_run_feedback.unwrap();
        assert_eq!(feedback.source_offset, offset);
        assert_eq!(feedback.phase, SourceRunPhase::Running);
    });
    view.simulate_mouse_move(declaration_point, None, Modifiers::default());
    view.run_until_parked();
    view.read(|cx| assert!(editor.read(cx).source_run_buttons[0].visible));

    for phase in [SourceRunPhase::Success, SourceRunPhase::Failure] {
        editor.update(view, |editor, cx| {
            editor.show_source_run_feedback(offset, phase, cx)
        });
        view.run_until_parked();
        view.read(|cx| {
            let editor = editor.read(cx);
            assert!(editor.source_run_buttons[0].visible);
            assert!(!editor.source_run_buttons[1].visible);
            assert_eq!(editor.source_run_feedback.unwrap().phase, phase);
        });
    }
    editor.update(view, |editor, cx| {
        editor.clear_source_run_feedback();
        cx.notify();
    });
    view.run_until_parked();
    view.read(|cx| {
        let editor = editor.read(cx);
        assert!(
            editor
                .source_run_buttons
                .iter()
                .all(|button| !button.visible)
        );
        assert_eq!(
            editor
                .snapshot(cx)
                .copy_range(crate::document::ByteRange::new(0, source.len() as u64)),
            source
        );
    });
}

#[gpui::test]
fn tblfm_run_action_recalculates_the_independent_summary_in_one_undo_step(
    cx: &mut gpui::TestAppContext,
) {
    use crate::document::{ByteOffset, ByteRange};
    use crate::editor::{SourceRunAction, SourceRunPhase};

    cx.update(crate::editor::init);
    let report = include_str!("../../../../tests/fixtures/sql-report-summary.org");
    let source = format!(
        "{report}\n#+begin_example\n| 1 | 0 |\n#+TBLFM: $2=$1*2\n#+end_example\n\n#+TBLFM: $2=99\n"
    );
    let session = cx.new(|_| {
        DocumentSession::from_utf8("report.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session.clone(), cx));
    view.simulate_resize(gpui::size(px(1100.), px(1200.)));
    view.run_until_parked();
    view.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    view.run_until_parked();
    let (button, detail) = view.read(|cx| {
        let e = editor.read(cx);
        let formulas = e
            .source_run_buttons
            .iter()
            .filter(|button| matches!(button.action, SourceRunAction::TableFormula))
            .collect::<Vec<_>>();
        assert_eq!(
            formulas.len(),
            1,
            "literal and orphan formulas have no action"
        );
        let button = *formulas[0];
        assert!(button.visible);
        let row = e
            .hit_rows
            .iter()
            .find(|row| row.range.start == button.source_offset)
            .unwrap();
        assert!(button.bounds.left() >= row.text_origin_x - px(super::super::GUTTER_PADDING));
        assert!(button.bounds.right() <= row.text_origin_x);
        let plot = e.plots.lines.values().next().unwrap();
        assert!(plot.entry.source.formulas.is_none());
        (button, e.snapshot(cx).copy_range(plot.entry.source.table))
    });
    view.simulate_click(button.bounds.center(), Modifiers::default());
    view.run_until_parked();
    view.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    view.run_until_parked();
    view.read(|cx| {
        let e = editor.read(cx);
        let snapshot = e.snapshot(cx);
        let text = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()));
        let summary = text.lines().find(|line| line.contains("Total")).unwrap();
        let cells =
            crate::document::table::parse_line(summary, crate::document::DocumentFormat::Org).cells;
        for (cell, expected) in cells.iter().skip(1).zip(["12", "7.50", "60"]) {
            assert_eq!(&summary[cell.text_range.clone()], expected);
        }
        let feedback = e.source_run_feedback.unwrap();
        assert_eq!(feedback.phase, SourceRunPhase::Success);
        assert!(text[feedback.source_offset.0 as usize..].starts_with("#+TBLFM:"));
        assert_eq!(e.plots.lines.len(), 1);
        let (&plot_line, plot) = e.plots.lines.iter().next().unwrap();
        assert!(matches!(&plot.entry.prepared,
            crate::plot::PlotPreparation::Ready(plot) if plot.plan.rows == 3));
        assert_eq!(snapshot.copy_range(plot.entry.source.table), detail);
        assert!(e.display_map.trailing_height(plot_line) > 200.);
        let formula_line = snapshot
            .line_index_at(ByteOffset(text.find("#+TBLFM:").unwrap() as u64))
            .unwrap()
            .0;
        assert_eq!(e.display_map.trailing_height(formula_line), 0.);
    });
    session.update(view, |session, cx| {
        session.undo(cx).unwrap();
    });
    view.run_until_parked();
    view.read(|cx| {
        let snapshot = session.read(cx).snapshot();
        assert_eq!(
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
            source
        );
    });

    let markdown = cx.new(|_| {
        DocumentSession::from_utf8("report.md".into(), report.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(markdown, cx));
    view.run_until_parked();
    view.read(|cx| assert!(editor.read(cx).source_run_buttons.is_empty()));
}

#[gpui::test]
fn tblfm_run_errors_preserve_the_table_and_show_failure(cx: &mut gpui::TestAppContext) {
    use crate::document::ByteRange;
    cx.update(crate::editor::init);
    let source = "| 2 | 0 |\n#+TBLFM: $2=unknown($1)\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("formulas.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.run_until_parked();
    let position = view.read(|cx| editor.read(cx).source_run_buttons[0].bounds.center());
    view.simulate_click(position, Modifiers::default());
    view.run_until_parked();
    view.read(|cx| {
        let e = editor.read(cx);
        assert_eq!(
            e.source_run_feedback.unwrap().phase,
            crate::editor::SourceRunPhase::Failure
        );
        assert!(e.command_feedback.as_ref().unwrap().contains("Unsupported"));
        let snapshot = e.snapshot(cx);
        assert_eq!(
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
            source
        );
    });
}

#[gpui::test]
fn org_call_run_actions_use_the_line_gutter_and_ignore_literal_calls(
    cx: &mut gpui::TestAppContext,
) {
    use crate::document::{ByteOffset, ByteRange};
    use crate::editor::SourceRunPhase;

    cx.update(crate::editor::init);
    let source = "#+name: calculated\n#+CALL: double(n=6)\n#+call: double(n=7)\n\n#+begin_example\n#+CALL: literal()\n#+end_example\n\n#+begin_src python\ntext = 'example'\n#+CALL: literal()\nreturn text\n#+end_src\n";
    let offset = ByteOffset(source.find("#+CALL:").unwrap() as u64);
    let session = cx.new(|_| {
        DocumentSession::from_utf8("run-actions.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.simulate_resize(gpui::size(px(640.), px(800.)));
    view.run_until_parked();
    let button = view.read(|cx| {
        let e = editor.read(cx);
        assert_eq!(
            e.source_run_buttons.len(),
            3,
            "two real CALLs and one source-body action"
        );
        assert_eq!(
            e.source_copy.buttons.len(),
            1,
            "CALL does not introduce a source-copy action"
        );
        let row = e.hit_rows.iter().find(|row| row.line.0 == 1).unwrap();
        let button = *e
            .source_run_buttons
            .iter()
            .find(|b| b.source_offset == offset)
            .unwrap();
        assert!(button.visible);
        assert!(button.bounds.left() >= row.text_origin_x - px(super::super::GUTTER_PADDING));
        assert!(button.bounds.right() <= row.text_origin_x);
        assert!(button.bounds.top() >= row.origin_y);
        assert!(button.bounds.bottom() <= row.origin_y + row.line_height);
        button
    });
    view.simulate_mouse_move(button.bounds.center(), None, Modifiers::default());
    view.run_until_parked();
    view.simulate_click(button.bounds.center(), Modifiers::default());
    view.run_until_parked();
    view.read(|cx| {
        let feedback = editor.read(cx).source_run_feedback.unwrap();
        assert_eq!(feedback.source_offset, offset);
        assert_eq!(feedback.phase, SourceRunPhase::Running);
    });
    view.read(|cx| {
        assert_eq!(
            editor
                .read(cx)
                .snapshot(cx)
                .copy_range(ByteRange::new(0, source.len() as u64)),
            source
        )
    });

    let markdown = cx.new(|_| {
        DocumentSession::from_utf8("run-actions.md".into(), b"#+CALL: literal()\n".to_vec())
            .unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(markdown, cx));
    view.run_until_parked();
    view.read(|cx| assert!(editor.read(cx).source_run_buttons.is_empty()));
}

#[gpui::test]
fn folded_source_run_action_moves_to_the_declaration_and_preserves_feedback(
    cx: &mut gpui::TestAppContext,
) {
    use crate::document::{ByteOffset, ByteRange, Selection};
    use crate::editor::SourceRunPhase;

    cx.update(crate::editor::init);
    let source = "#+name: double\n#+begin_src python :var n=2\nvalue = n * 2\nreturn value\n#+end_src\nafter\n";
    let offset = ByteOffset(source.find("#+begin_src").unwrap() as u64);
    let session = cx.new(|_| {
        DocumentSession::from_utf8("run-folded.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.simulate_resize(gpui::size(px(640.), px(600.)));
    view.run_until_parked();
    editor.update(view, |e, cx| {
        e.set_selection(Selection::caret(offset), cx);
        e.show_source_run_feedback(offset, SourceRunPhase::Running, cx);
    });
    // Focus the source declaration, then use the same Tab fold command as the editor.
    let header = view.read(|cx| {
        let e = editor.read(cx);
        let row = e.hit_rows.iter().find(|r| r.line.0 == 1).unwrap();
        gpui::point(
            row.text_origin_x + px(1.),
            row.origin_y + row.line_height / 2.,
        )
    });
    view.simulate_click(header, Modifiers::default());
    view.simulate_keystrokes("tab");
    view.run_until_parked();
    editor.update(view, |e, cx| {
        e.finish_fold_animation();
        cx.notify();
    });
    view.run_until_parked();
    let button = view.read(|cx| {
        let e = editor.read(cx);
        assert!(e.fold_markers.contains(&1));
        assert_eq!(e.source_run_buttons.len(), 1);
        assert_eq!(e.source_copy.buttons.len(), 1);
        let button = e.source_run_buttons[0];
        let row = e.hit_rows.iter().find(|r| r.line.0 == 1).unwrap();
        assert!(button.visible);
        assert_eq!(button.source_offset, offset);
        assert!(button.bounds.right() <= row.text_origin_x);
        assert!(button.bounds.top() >= row.origin_y);
        assert!(button.bounds.bottom() <= row.origin_y + row.line_height);
        assert_eq!(
            e.source_run_feedback.unwrap().phase,
            SourceRunPhase::Running
        );
        button
    });
    view.simulate_click(button.bounds.center(), Modifiers::default());
    view.run_until_parked();
    view.read(|cx| {
        assert!(
            editor.read(cx).fold_markers.contains(&1),
            "run must not expand the block"
        )
    });
    view.simulate_keystrokes("tab");
    view.run_until_parked();
    editor.update(view, |e, cx| {
        e.finish_fold_animation();
        cx.notify();
    });
    view.run_until_parked();
    view.read(|cx| {
        let e = editor.read(cx);
        assert!(!e.fold_markers.contains(&1));
        assert_eq!(e.source_run_buttons.len(), 1);
        let row = e.hit_rows.iter().find(|r| r.line.0 == 2).unwrap();
        let button = e.source_run_buttons[0];
        assert_eq!(button.source_offset, offset);
        assert!(button.bounds.left() >= e.hit_rows[0].text_origin_x);
        assert!(button.bounds.bottom() <= row.origin_y + row.line_height);
        assert!(button.bounds.top() >= row.origin_y);
        assert_eq!(
            e.snapshot(cx)
                .copy_range(ByteRange::new(0, source.len() as u64)),
            source
        );
    });
}
