use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext};

fn source(editor: &Entity<SemanticEditor>, cx: &VisualTestContext) -> String {
    cx.read(|cx| {
        let snapshot = editor.read(cx).snapshot(cx);
        snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
    })
}

fn cell_point(
    editor: &Entity<SemanticEditor>,
    line: u64,
    column: usize,
    cx: &VisualTestContext,
) -> Point<Pixels> {
    cx.read(|cx| {
        let editor = editor.read(cx);
        let row = editor
            .hit_rows
            .iter()
            .find(|row| row.line.0 == line)
            .unwrap();
        let snapshot = editor.snapshot(cx);
        let range = snapshot.line_content_range(row.line).unwrap();
        let text = snapshot.copy_range(range);
        let format = DocumentFormat::from_path(editor.session.read(cx).syntax_path());
        let parsed = source_table::parse_line(&text, format);
        let p = row
            .position_for_display_index(
                row.display
                    .source_to_display(parsed.cells[column].text_range.start),
            )
            .unwrap();
        point(
            row.text_origin_x + p.x + px(2.),
            row.origin_y + p.y + px(5.),
        )
    })
}

fn hover(editor: &Entity<SemanticEditor>, line: u64, column: usize, cx: &mut VisualTestContext) {
    let position = cell_point(editor, line, column, cx);
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.run_until_parked();
}

fn click_button(cx: &mut VisualTestContext) {
    let button = cx.debug_bounds("table-cell-menu-button").unwrap();
    cx.simulate_click(button.center(), Modifiers::default());
    cx.run_until_parked();
}

fn choose(editor: &Entity<SemanticEditor>, action: MenuAction, cx: &mut VisualTestContext) {
    let index = cx.read(|cx| {
        editor
            .read(cx)
            .table_actions
            .popup
            .as_ref()
            .unwrap()
            .items
            .iter()
            .position(|item| item.action == action)
            .unwrap()
    });
    let selector = Box::leak(format!("table-menu-item-{index}").into_boxed_str());
    let bounds = cx.debug_bounds(selector).unwrap();
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
fn table_menu_hover_and_open_preserve_selection_and_edit_the_clicked_cell(cx: &mut TestAppContext) {
    cx.update(init);
    let original =
        "* Inventory\n\n| Item | Count |\n|------+-------|\n| Pencil | 12 |\n| Paper | 3 |\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("inventory.org".into(), original.as_bytes().to_vec()).unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.run_until_parked();
    let original_selection = Selection::new(ByteOffset(8), ByteOffset(2));
    editor.update(cx, |e, cx| e.set_selection(original_selection, cx));
    hover(&editor, 4, 1, cx);
    assert!(cx.debug_bounds("table-cell-menu-button").is_some());
    cx.read(|cx| {
        let e = editor.read(cx);
        let hit = e.table_actions.hover.as_ref().unwrap();
        assert!(hit.button.left() >= hit.bounds.left());
        assert!(
            hit.button.right() < hit.bounds.right(),
            "the right delimiter must stay outside the button"
        );
    });
    click_button(cx);
    cx.read(|cx| {
        let e = editor.read(cx);
        assert_eq!(e.selection, original_selection);
        assert_eq!(e.table_actions.popup.as_ref().unwrap().target.column, 1);
    });
    assert_eq!(source(&editor, cx), original);
    let other_cell = cell_point(&editor, 5, 0, cx);
    cx.simulate_mouse_move(other_cell, None, Modifiers::default());
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(
            editor
                .read(cx)
                .table_actions
                .popup
                .as_ref()
                .unwrap()
                .target
                .line,
            LineIndex(4)
        )
    });
    choose(&editor, MenuAction::Edit(TableEdit::InsertColumn), cx);
    let changed = source(&editor, cx);
    let row = changed
        .lines()
        .find(|line| line.contains("Pencil"))
        .unwrap();
    let parsed = source_table::parse_line(row, DocumentFormat::Org);
    assert_eq!(parsed.cells.len(), 3);
    assert_eq!(&row[parsed.cells[1].text_range.clone()], "");
    assert_eq!(&row[parsed.cells[2].text_range.clone()], "12");
    cx.simulate_keystrokes("cmd-z");
    cx.run_until_parked();
    assert_eq!(source(&editor, cx), original);
    cx.read(|cx| assert_eq!(editor.read(cx).selection, original_selection));
}

#[gpui::test]
fn table_menu_right_click_sort_keyboard_and_escape(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "| Item | Count |\n|------+-------|\n| Pencil | 12 |\n| Paper | 3 |\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("inventory.org".into(), original.as_bytes().to_vec()).unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.run_until_parked();
    let position = cell_point(&editor, 2, 1, cx);
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("table-cell-menu").is_some());
    choose(&editor, MenuAction::Sort, cx);
    cx.simulate_keystrokes("down down enter");
    cx.run_until_parked();
    let changed = source(&editor, cx);
    assert!(changed.find("Paper").unwrap() < changed.find("Pencil").unwrap());
    cx.simulate_keystrokes("cmd-z");
    cx.run_until_parked();
    assert_eq!(source(&editor, cx), original);
    hover(&editor, 2, 0, cx);
    click_button(cx);
    cx.simulate_input("accidental typing");
    assert_eq!(source(&editor, cx), original);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("table-cell-menu").is_none());
}

#[gpui::test]
fn table_menu_hides_invalid_markdown_actions_and_ignores_literal_tables(cx: &mut TestAppContext) {
    cx.update(init);
    let original =
        "```\n| literal | text |\n```\n\n| Item | Count |\n| --- | ---: |\n| Pencil | 12 |\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("inventory.md".into(), original.as_bytes().to_vec()).unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.run_until_parked();
    hover(&editor, 1, 0, cx);
    assert!(cx.debug_bounds("table-cell-menu-button").is_none());
    hover(&editor, 4, 1, cx);
    click_button(cx);
    cx.read(|cx| {
        let menu = editor.read(cx).table_actions.popup.as_ref().unwrap();
        for edit in [
            TableEdit::KillRow,
            TableEdit::MoveRowUp,
            TableEdit::MoveRowDown,
            TableEdit::MoveColumnRight,
            TableEdit::InsertHline,
        ] {
            assert!(
                !menu
                    .items
                    .iter()
                    .any(|item| item.action == MenuAction::Edit(edit))
            );
        }
    });
    editor.update(cx, |e, cx| e.scroll(0., -30., cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("table-cell-menu").is_none());
    assert_eq!(source(&editor, cx), original);
}

#[test]
fn table_menu_position_stays_in_viewport_without_changing_anchor() {
    for (width, height) in [(800., 600.), (300., 240.), (180., 100.)] {
        let viewport = Bounds::new(point(px(50.), px(40.)), size(px(width), px(height)));
        let anchor = Bounds::new(
            point(viewport.right() - px(22.), viewport.bottom() - px(28.)),
            size(px(20.), px(20.)),
        );
        let menu = super::view::menu_bounds(viewport, anchor, 450.);
        assert!(menu.left() >= viewport.left());
        assert!(menu.right() <= viewport.right());
        assert_eq!(menu.top(), anchor.bottom() + px(6.));
        assert!(menu.bottom() <= viewport.bottom());
    }
}

#[gpui::test]
fn table_menu_wrap_geometry_scroll_and_resize_are_independent(cx: &mut TestAppContext) {
    cx.update(init);
    let note = "记录不同季节的叶片形态并整理观察笔记以便下次对照".repeat(3);
    let original = format!(
        "| 名称 | 状态 | 说明 |\n|---+---+---|\n| 北坡 | 完成 | {note} |\n| 南岸 | 计划 | 周末继续收集样本 |\n\n后续记录\n"
    );
    let session = cx.new(|_| {
        DocumentSession::from_utf8("field-notes.org".into(), original.as_bytes().to_vec()).unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(size(px(520.), px(360.)));
    cx.run_until_parked();
    editor.update(cx, |e, cx| {
        e.set_soft_wrap(true, cx);
        assert!(e.align_table_at_selection(cx));
    });
    cx.run_until_parked();
    let before = source(&editor, cx);
    let geometry = cx.read(|cx| {
        editor
            .read(cx)
            .hit_rows
            .iter()
            .map(|row| (row.line, row.origin_y, row.visible_bottom))
            .collect::<Vec<_>>()
    });
    let position = cx.read(|cx| {
        let e = editor.read(cx);
        let row = e
            .hit_rows
            .iter()
            .find(|row| row.line == LineIndex(2))
            .unwrap();
        let table = row.table_layout.as_ref().unwrap();
        assert!(table.visual_rows > 1);
        let fragment = table.fragments.iter().rfind(|f| !f.delimiter).unwrap();
        point(
            row.text_origin_x + fragment.x + px(5.),
            row.origin_y + row.line_height + px(5.),
        )
    });
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.run_until_parked();
    cx.read(|cx| {
        let e = editor.read(cx);
        let hit = e.table_actions.hover.as_ref().unwrap();
        assert!(hit.button.left() >= hit.bounds.left());
        assert!(
            hit.button.right() < hit.bounds.right(),
            "wrapped cells must also keep their right delimiter visible"
        );
        let row = e.hit_rows.iter().find(|row| row.line == hit.line).unwrap();
        let fragment = row
            .table_layout
            .as_ref()
            .unwrap()
            .fragments
            .iter()
            .rfind(|f| !f.delimiter)
            .unwrap();
        assert!(
            hit.button.left() >= row.text_origin_x + fragment.x + fragment.width,
            "the button must stay in the padding outside every wrapped text line"
        );
    });
    click_button(cx);
    let bounds = cx.debug_bounds("table-cell-menu").unwrap();
    assert!(bounds.right() <= px(520.) && bounds.bottom() <= px(360.));
    let button = cx.debug_bounds("table-cell-menu-button").unwrap();
    assert_eq!(bounds.top(), button.bottom() + px(6.));
    cx.read(|cx| {
        let e = editor.read(cx);
        assert_eq!(e.table_actions.popup.as_ref().unwrap().target.column, 2);
        assert_eq!(
            e.hit_rows
                .iter()
                .map(|row| (row.line, row.origin_y, row.visible_bottom))
                .collect::<Vec<_>>(),
            geometry
        );
    });
    let scroll_y = cx.read(|cx| editor.read(cx).scroll_y);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-60.))),
        modifiers: Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let e = editor.read(cx);
        assert_eq!(e.scroll_y, scroll_y);
        assert!(e.table_actions.popup.as_ref().unwrap().scroll.offset().y < px(0.));
    });
    assert_eq!(source(&editor, cx), before);
    cx.simulate_resize(size(px(430.), px(360.)));
    cx.run_until_parked();
    assert!(cx.debug_bounds("table-cell-menu").is_none());
    assert!(cx.debug_bounds("table-cell-menu-button").is_none());
    assert_eq!(source(&editor, cx), before);
}

#[gpui::test]
fn table_menu_button_uses_only_the_space_before_the_pipe(cx: &mut TestAppContext) {
    cx.update(init);
    for path in ["minerals.org", "minerals.md"] {
        let original = "| Mineral | Amount |\n| ------- | ------ |\n| Quartz! | 123456 |\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8(path.into(), original.as_bytes().to_vec()).unwrap()
        });
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        view.run_until_parked();
        editor.update(view, |e, cx| assert!(e.align_table_at_selection(cx)));
        view.run_until_parked();
        let before = source(&editor, view);
        for line in [0, 2] {
            for column in [0, 1] {
                hover(&editor, line, column, view);
                let button = view.debug_bounds("table-cell-menu-button").unwrap();
                let icon = view.debug_bounds("table-cell-menu-icon").unwrap();
                view.read(|cx| {
                    let e = editor.read(cx);
                    let row = e.hit_rows.iter().find(|row| row.line.0 == line).unwrap();
                    let snapshot = e.snapshot(cx);
                    let text = snapshot.copy_range(snapshot.line_content_range(row.line).unwrap());
                    let parsed = source_table::parse_line(
                        &text,
                        DocumentFormat::from_path(e.session.read(cx).syntax_path()),
                    );
                    let cell = &parsed.cells[column];
                    let at = |offset| {
                        row.text_origin_x
                            + row
                                .position_for_display_index(row.display.source_to_display(offset))
                                .unwrap()
                                .x
                    };
                    assert!(
                        button.left() >= at(cell.text_range.end),
                        "button overlaps the last letter"
                    );
                    assert!(
                        button.left() >= at(cell.raw_range.end - 1),
                        "button must occupy only the final space"
                    );
                    assert!(
                        button.right() < at(cell.raw_range.end),
                        "button overlaps the pipe"
                    );
                    let space_center = (at(cell.raw_range.end - 1) + at(cell.raw_range.end)) / 2.;
                    assert!((button.center().x - space_center).abs() < px(0.01));
                    assert!(
                        (button.center().y - row.origin_y - row.line_height / 2.).abs() < px(0.01),
                        "button must be vertically centered in the text line"
                    );
                    assert!((icon.center().x - button.center().x).abs() <= px(1.));
                    assert!((icon.center().y - button.center().y).abs() <= px(1.));
                });
            }
        }
        assert_eq!(source(&editor, view), before);
    }
}

#[gpui::test]
fn table_menu_document_changes_invalidate_target_and_read_only_has_no_entry(
    cx: &mut TestAppContext,
) {
    cx.update(init);
    let original = "| Item | Count |\n|------+-------|\n| Ink | 2 |\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("supplies.org".into(), original.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session.clone(), cx));
    view.run_until_parked();
    hover(&editor, 2, 0, view);
    click_button(view);
    session.update(view, |session, cx| {
        session
            .edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        session.snapshot().revision(),
                        vec![TextEdit::new(ByteRange::new(0, 0), "Notes\n\n")],
                    ),
                    Selection::default(),
                    Selection::default(),
                    EditOrigin::Other,
                ),
                cx,
            )
            .unwrap();
    });
    view.run_until_parked();
    assert!(view.debug_bounds("table-cell-menu").is_none());
    editor.update(view, |e, cx| {
        e.table_menu_choose(MenuAction::Edit(TableEdit::KillRow), cx)
    });
    assert_eq!(source(&editor, view), format!("Notes\n\n{original}"));
    let session = cx.new(|_| DocumentSession::read_only_text(original.into()));
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.run_until_parked();
    hover(&editor, 2, 0, view);
    assert!(view.debug_bounds("table-cell-menu-button").is_none());
}
