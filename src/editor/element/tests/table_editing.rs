use super::*;

#[gpui::test]
fn typing_past_table_padding_expands_the_source_row_without_wrapping(
    cx: &mut gpui::TestAppContext,
) {
    use crate::document::{ByteOffset, ByteRange, Selection};

    cx.update(crate::editor::init);
    // Source checked against Emacs 30.2 / Org 9.7.11: ordinary insertion does
    // not resize the separator or re-align the other rows until Tab is pressed.
    let source = "| Key | Description | Tail |\n|-----+-------------+------|\n| a   | note        | end  |\n";
    let caret = source.find("Description").unwrap() + "Description".len();
    let session = cx.new(|_| {
        DocumentSession::from_utf8("typing.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.simulate_resize(gpui::size(px(1400.), px(600.)));
    editor.update(view, |editor, cx| {
        editor.set_selection(Selection::caret(ByteOffset(caret as u64)), cx)
    });
    for _ in 0..12 {
        view.simulate_input("s");
    }
    view.run_until_parked();
    editor.read_with(view, |editor, cx| {
        let snapshot = editor.snapshot(cx);
        assert_eq!(
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
            source.replacen("Description", "Descriptionssssssssssss", 1)
        );
        let row = editor.hit_rows.iter().find(|row| row.line.0 == 0).unwrap();
        assert_eq!(row.table_layout.as_ref().unwrap().visual_rows, 1);
        let caret = row.position_for_display_index(caret + 12).unwrap();
        assert_eq!(caret.y, px(0.));
    });
    view.simulate_keystrokes("tab");
    view.run_until_parked();
    editor.read_with(view, |editor, cx| {
        let snapshot = editor.snapshot(cx);
        assert!(
            crate::editor::org_commands::table_columns(
                &snapshot,
                0,
                crate::document::DocumentFormat::Org,
            )
            .unwrap()
            .aligned
        );
        assert!(
            editor
                .hit_rows
                .iter()
                .filter_map(|row| row.table_layout.as_ref())
                .all(|layout| layout.visual_rows == 1)
        );
    });
}

#[gpui::test]
fn org_table_character_commands_match_emacs_source_and_caret(cx: &mut gpui::TestAppContext) {
    use crate::document::{ByteOffset, ByteRange, Selection};
    cx.update(crate::editor::init);
    // Buffer contents and point checked against Emacs 30.2 / Org 9.7.11.
    for (source, input, keys, expected) in [
        ("| B | 5¦   | tail |", "1", "", "| B | 51¦  | tail |"),
        ("| B | 5¦   | tail |", " ", "", "| B | 5 ¦  | tail |"),
        ("| B | 5¦   | tail |", "   ", "", "| B | 5   ¦ | tail |"),
        ("| B | 5¦   | tail |", "1 2", "", "| B | 51 2¦ | tail |"),
        (
            "| B | 5¦   | tail |",
            "",
            "backspace",
            "| B | ¦    | tail |",
        ),
        (
            "| B | 5 ¦  | tail |",
            "",
            "backspace",
            "| B | 5¦   | tail |",
        ),
        ("| B | ¦5   | tail |", "", "delete", "| B | ¦    | tail |"),
        ("| B | 5¦   | tail |", "", "delete", "| B | 5¦   | tail |"),
        ("| B |¦ 5   | tail |", "", "backspace", "| B ¦ 5   | tail |"),
        ("| B | 5¦   | tail |", "|", "", "| B | 5|¦  | tail |"),
        ("| 中 | 5¦   | tail |", "文", "", "| 中 | 5文¦  | tail |"),
        ("| 名 |    5¦ |", "1", "", "| 名 |    51¦ |"),
        (
            "| B | 5   ¦| tail |",
            "",
            "backspace",
            "| B | 5  ¦ | tail |",
        ),
        ("| B | 5  ¦ | tail |", "", "delete", "| B | 5  ¦ | tail |"),
    ] {
        let offset = source.find('¦').unwrap();
        let source = source.replace('¦', "");
        let expected_caret = expected.find('¦').unwrap();
        let expected = expected.replace('¦', "");
        let session = cx.new(|_| {
            DocumentSession::from_utf8("typing.org".into(), source.as_bytes().to_vec()).unwrap()
        });
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        view.simulate_resize(gpui::size(px(1500.), px(800.)));
        editor.update(view, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(offset as u64)), cx);
        });
        for ch in input.chars() {
            view.simulate_input(&ch.to_string());
        }
        if !keys.is_empty() {
            view.simulate_keystrokes(keys);
        }
        view.run_until_parked();
        editor.read_with(view, |editor, cx| {
            let snapshot = editor.snapshot(cx);
            assert_eq!(
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
                expected,
                "source={source:?}, input={input:?}, keys={keys}"
            );
            assert_eq!(editor.selection().head().0 as usize, expected_caret);
            let utf16 = expected[..expected_caret].encode_utf16().count();
            assert_eq!(editor.selection_utf16, utf16..utf16);
            let row = editor.hit_rows.iter().find(|row| row.line.0 == 0).unwrap();
            let table = row.table_layout.as_ref().unwrap();
            assert_eq!(table.metrics.font, crate::typography::table_font());
            assert!(table.fragments.iter().all(|fragment| {
                fragment
                    .preview_runs
                    .iter()
                    .all(|run| run.font.family == table.metrics.font.family)
            }));
            for index in row
                .display
                .text
                .char_indices()
                .map(|(index, _)| index)
                .chain(std::iter::once(row.display.text.len()))
            {
                let actual = row.position_for_display_index(index).unwrap();
                let native = row
                    .layout
                    .position_for_index(index, row.line_height)
                    .unwrap();
                assert!(
                    (f32::from(actual.x - native.x)).abs() < 0.1,
                    "source whitespace must use native positions: {expected:?}, index={index}, actual={actual:?}, native={native:?}, widths={:?}, fragments={:?}",
                    table.column_widths,
                    table.fragments.iter().map(|fragment| (&fragment.layout.text, fragment.layout.width(), fragment.layout.wrap_boundaries().len())).collect::<Vec<_>>()
                );
                assert_eq!(row.closest_display_index(actual), index);
            }
        });
        view.simulate_keystrokes("cmd-z");
        view.run_until_parked();
        editor.read_with(view, |editor, cx| {
            let snapshot = editor.snapshot(cx);
            assert_eq!(
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
                source
            );
            assert_eq!(editor.selection().head().0 as usize, offset);
        });
    }

    let source = "| Name | Value |\n|------+-------|\n| B    |     5 |\n| C    |    25 |\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("navigation.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    editor.update(view, |editor, cx| {
        editor.set_selection(
            Selection::caret(ByteOffset(source.find("| B").unwrap() as u64 + 3)),
            cx,
        );
    });
    view.simulate_keystrokes("tab");
    view.simulate_input("7");
    view.run_until_parked();
    editor.read_with(view, |editor, cx| {
        let snapshot = editor.snapshot(cx);
        assert_eq!(
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
            source.replace("| B    |     5 |", "| B    | 7     |")
        );
    });
    view.simulate_keystrokes("cmd-z");
    view.run_until_parked();
    editor.read_with(view, |editor, cx| {
        let snapshot = editor.snapshot(cx);
        assert_eq!(
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
            source
        );
    });
    let source = "| Name|Value|\n|---+---|\n| A|5|\n| B|25|\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("align.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    editor.update(view, |editor, cx| {
        editor.set_selection(
            Selection::caret(ByteOffset(source.find('5').unwrap() as u64 + 1)),
            cx,
        );
    });
    view.simulate_input("1");
    view.simulate_keystrokes("tab");
    view.run_until_parked();
    editor.read_with(view, |editor, cx| {
        let snapshot = editor.snapshot(cx);
        assert_eq!(
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
            "| Name | Value |\n|------+-------|\n| A    |    51 |\n| B    |    25 |\n"
        );
        assert_eq!(editor.selection().head().0, 53);
    });
    view.simulate_keystrokes("tab enter");
    view.run_until_parked();
    editor.read_with(view, |editor, cx| {
        let snapshot = editor.snapshot(cx);
        assert!(
            snapshot
                .copy_range(ByteRange::new(
                    editor.selection().head().0,
                    snapshot.len_bytes(),
                ))
                .starts_with("      |")
        );
    });
}

#[gpui::test]
fn org_table_compensation_excludes_literals_paste_and_selected_text(cx: &mut gpui::TestAppContext) {
    use crate::document::{ByteOffset, ByteRange, EditOrigin, Selection};
    cx.update(crate::editor::init);
    for (path, source) in [
        (
            "literal.org",
            "#+begin_src text\n| B | 5   | tail |\n#+end_src\n",
        ),
        (
            "literal.org",
            "#+begin_example\n| B | 5   | tail |\n#+end_example\n",
        ),
        ("table.md", "| B | 5   | tail |\n|---|-----|------|\n"),
    ] {
        let offset = source.find('5').unwrap() + 1;
        let session = cx
            .new(|_| DocumentSession::from_utf8(path.into(), source.as_bytes().to_vec()).unwrap());
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        editor.update(view, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(offset as u64)), cx);
        });
        view.simulate_input("1");
        view.run_until_parked();
        editor.read_with(view, |editor, cx| {
            let snapshot = editor.snapshot(cx);
            assert_eq!(
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
                source.replace('5', "51")
            );
        });
    }
    let source = "| B | 5   | tail |";
    for selected in [false, true] {
        let session = cx.new(|_| {
            DocumentSession::from_utf8("table.org".into(), source.as_bytes().to_vec()).unwrap()
        });
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        editor.update(view, |editor, cx| {
            let caret = ByteOffset(source.find('5').unwrap() as u64 + 1);
            editor.set_selection(
                if selected {
                    Selection::new(ByteOffset(caret.0 - 1), caret)
                } else {
                    Selection::caret(caret)
                },
                cx,
            );
            if !selected {
                editor.replace_selection("1", EditOrigin::Paste, cx);
            }
        });
        if selected {
            view.simulate_keystrokes("backspace");
        }
        view.run_until_parked();
        editor.read_with(view, |editor, cx| {
            let snapshot = editor.snapshot(cx);
            assert_eq!(
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
                source.replace('5', if selected { "" } else { "51" })
            );
        });
    }
}

#[gpui::test]
fn editor_table_hover_uses_rows_for_data_and_columns_for_headers(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    for (path, separator) in [
        ("tables.org", "|---+---+---|"),
        ("tables.md", "|---|---|---|"),
    ] {
        let source = format!(
            "| A | B | C |\n{separator}\n| 1 | 2 | 3 |\n\n| D | E | F |\n{separator}\n| 4 | 5 | 6 |\n"
        );
        let session = cx
            .new(|_| DocumentSession::from_utf8(path.into(), source.as_bytes().to_vec()).unwrap());
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        view.run_until_parked();
        let body_cell = table_cell_center(&editor, 2, 1, view);
        view.simulate_mouse_move(body_cell, None, Modifiers::default());
        view.run_until_parked();
        view.read(|cx| {
            let editor = editor.read(cx);
            let (line, column, header) = editor.hovered_table_cell().unwrap();
            assert_eq!((line.0, column, header), (2, 1, false));
            let snapshot = editor.snapshot(cx);
            let format =
                crate::document::DocumentFormat::from_path(editor.session.read(cx).syntax_path());
            let quads = table_hover_quads(
                editor.hit_rows.iter(),
                &snapshot,
                format,
                line,
                column,
                header,
                0x4689d8,
            );
            assert_eq!(quads.len(), 1);
            assert!(quads[0].bounds.contains(&body_cell));
        });
        let header_cell = table_cell_center(&editor, 0, 1, view);
        view.simulate_mouse_move(header_cell, None, Modifiers::default());
        view.run_until_parked();
        view.read(|cx| {
            let editor = editor.read(cx);
            let (line, column, header) = editor.hovered_table_cell().unwrap();
            assert_eq!((line.0, column, header), (0, 1, true));
            let snapshot = editor.snapshot(cx);
            let format =
                crate::document::DocumentFormat::from_path(editor.session.read(cx).syntax_path());
            let quads = table_hover_quads(
                editor.hit_rows.iter(),
                &snapshot,
                format,
                line,
                column,
                header,
                0x4689d8,
            );
            assert_eq!(quads.len(), 3);
            let second_table_top = editor
                .hit_rows
                .iter()
                .find(|row| row.line.0 == 4)
                .unwrap()
                .origin_y;
            assert!(
                quads
                    .iter()
                    .all(|quad| quad.bounds.bottom() < second_table_top)
            );
            assert_eq!(
                quads
                    .iter()
                    .filter(|quad| quad.bounds.contains(&header_cell))
                    .count(),
                1,
            );
        });
        let blank = view.read(|cx| {
            let editor = editor.read(cx);
            let row = editor.hit_rows.iter().find(|row| row.line.0 == 3).unwrap();
            gpui::point(row.text_origin_x + px(10.), row.origin_y + px(5.))
        });
        view.simulate_mouse_move(blank, None, Modifiers::default());
        view.run_until_parked();
        view.read(|cx| assert!(editor.read(cx).hovered_table_cell().is_none()));
    }
}

#[gpui::test]
fn narrow_tables_wrap_cells_with_shared_columns_and_source_mapping(cx: &mut gpui::TestAppContext) {
    use crate::document::{ByteOffset, Selection};
    use gpui::EntityInputHandler;
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    for path in ["library.org", "library.md"] {
        let literal = if path.ends_with(".md") {
            "`plant|tree`"
        } else {
            "=plant|tree="
        };
        let prose = "这是一段用于检查自动换行的普通说明文字。".repeat(8);
        let source = format!(
            "| 书架 | 类别 | 说明 |\n|---|---|---|\n| 松林 | Astronomy | 每周更新观测笔记并整理适合初学者的阅读目录 |\n| 海湾🙂 | {literal} | 保存植物标本照片与季节变化记录 e\u{301} |\n\n{prose}\n"
        );
        let session =
            cx.new(|_| DocumentSession::from_utf8(path.into(), source.into_bytes()).unwrap());
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        view.simulate_resize(gpui::size(px(1500.), px(800.)));
        view.run_until_parked();
        editor.update(view, |editor, cx| {
            editor.set_soft_wrap(true, cx);
            editor.set_selection(Selection::caret(ByteOffset(2)), cx);
            assert!(editor.align_table_at_selection(cx));
        });
        view.run_until_parked();
        let original = editor.update(view, |e, cx| e.snapshot(cx));
        let wide_columns = editor.update(view, |e, _| {
            e.hit_rows[0]
                .table_layout
                .as_ref()
                .unwrap()
                .fragments
                .iter()
                .filter(|f| &e.hit_rows[0].display.text[f.display_range.clone()] == "|")
                .map(|f| f.x)
                .collect::<Vec<_>>()
        });
        view.simulate_resize(gpui::size(px(520.), px(800.)));
        view.run_until_parked();
        let prose_x = editor.update(view, |e, _| {
            assert_eq!(e.max_horizontal_scroll(), 0.);
            let mut narrow_columns = None;
            for row in e.hit_rows.iter().filter(|row| row.line.0 < 4) {
                let table = row
                    .table_layout
                    .as_ref()
                    .expect("narrow tables retain pixel alignment");
                let columns = table
                    .fragments
                    .iter()
                    .filter(|f| matches!(&row.display.text[f.display_range.clone()], "|" | "+"))
                    .map(|f| f.x)
                    .collect::<Vec<_>>();
                assert!(columns.last() < wide_columns.last());
                assert!(f32::from(table.width) <= e.display_map.wrap_width() + 0.5);
                if let Some(expected) = &narrow_columns {
                    assert_eq!(&columns, expected);
                } else {
                    narrow_columns = Some(columns);
                }
                let height = e
                    .minimap
                    .prepared_layout()
                    .unwrap()
                    .line_height_px(row.line.0);
                assert!((height - f32::from(row.visible_bottom - row.visible_top)).abs() < 0.5);
                assert!(
                    e.minimap
                        .prepared_layout()
                        .unwrap()
                        .line_wrap_starts(row.line.0)
                        .is_empty()
                );
            }
            let prose = e.hit_rows.iter().find(|row| row.line.0 == 5).unwrap();
            assert!(!prose.layout.wrap_boundaries().is_empty());
            prose.text_origin_x
        });
        editor.update(view, |e, cx| e.scroll(-180., 0., cx));
        view.run_until_parked();
        editor.update(view, |e, cx| {
            assert_eq!(e.scroll_x, 0.);
            let prose = e.hit_rows.iter().find(|row| row.line.0 == 5).unwrap();
            assert_eq!(prose.text_origin_x, prose_x);
            let row = &e.hit_rows[2];
            assert!((f32::from(prose_x - row.text_origin_x) - e.scroll_x).abs() < 0.5);
            // Hit testing and selection follow the cell continuation, not the source line width.
            let index = row.display.text.find("目录").unwrap();
            let point = row.position_for_display_index(index).unwrap();
            assert!(
                point.y > px(0.),
                "cell continuation: point={point:?}, text={}, fragments={:?}",
                row.display.text,
                row.table_layout
                    .as_ref()
                    .unwrap()
                    .fragments
                    .iter()
                    .map(|f| (
                        &f.content_range,
                        f.width,
                        f.layout.len(),
                        f.layout.wrap_boundaries().len()
                    ))
                    .collect::<Vec<_>>()
            );
            let table = row.table_layout.as_ref().unwrap();
            let content = table
                .fragments
                .iter()
                .find(|f| f.content_range.contains(&index))
                .unwrap();
            let highlights = table.range_bounds(content.content_range.clone(), row.line_height);
            assert!(highlights.len() > 1);
            assert!(
                highlights
                    .iter()
                    .all(|b| b.left() >= content.x
                        && b.right() <= content.x + content.width + px(0.5))
            );
            let hit = e.hit_test(gpui::point(
                row.text_origin_x + point.x,
                row.origin_y + point.y + px(1.),
            ));
            assert_eq!(hit.0, row.range.start.0 + index as u64);
            e.set_selection(Selection::caret(hit), cx);
        });
        view.run_until_parked();
        view.update(|window, cx| {
            editor.update(cx, |e, cx| {
                let caret = e.selection().head();
                let utf16 = e.snapshot(cx).byte_to_utf16(caret).unwrap().0 as usize;
                let bounds = e
                    .bounds_for_range(utf16..utf16, gpui::Bounds::default(), window, cx)
                    .unwrap();
                let row = e.hit_rows.iter().find(|row| row.line.0 == 2).unwrap();
                let position = row
                    .position_for_display_index((caret.0 - row.range.start.0) as usize)
                    .unwrap();
                assert_eq!(
                    bounds.origin,
                    gpui::point(row.text_origin_x + position.x, row.origin_y + position.y)
                );
            });
        });
        editor.update(view, |e, _| {
            let row = e.hit_rows.iter().find(|row| row.line.0 == 2).unwrap();
            let caret = row
                .position_for_display_index(row.display.text.len())
                .unwrap();
            let right = e.minimap.bounds.unwrap().left();
            assert!(row.text_origin_x + caret.x < right);
        });
        editor.update(view, |e, cx| {
            let row = e.hit_rows.iter().find(|row| row.line.0 == 2).unwrap();
            let short_cell =
                ByteOffset(row.range.start.0 + row.display.text.find("Astronomy").unwrap() as u64);
            e.set_selection(Selection::caret(short_cell), cx);
            e.move_vertical(1, false, cx);
            assert_eq!(
                e.snapshot(cx)
                    .line_index_at(e.selection().head())
                    .unwrap()
                    .0,
                3
            );
            let row = e.hit_rows.iter().find(|row| row.line.0 == 2).unwrap();
            let target =
                ByteOffset(row.range.start.0 + row.display.text.find("目录").unwrap() as u64);
            e.set_selection(Selection::caret(target), cx);
        });
        editor.update(view, |e, cx| e.set_soft_wrap(false, cx));
        view.run_until_parked();
        editor.update(view, |e, cx| {
            assert!(
                e.hit_rows
                    .iter()
                    .filter_map(|row| row.table_layout.as_ref())
                    .all(|table| table.visual_rows == 1)
            );
            assert!(e.max_horizontal_scroll() > 0.);
            e.scroll(-180., 0., cx);
            assert!(e.scroll_x > 0.);
            e.set_soft_wrap(true, cx);
        });
        view.simulate_resize(gpui::size(px(1500.), px(800.)));
        view.run_until_parked();
        editor.update(view, |e, cx| {
            assert_eq!(e.scroll_x, 0.);
            assert_eq!(e.snapshot(cx).revision(), original.revision());
        });
        view.simulate_resize(gpui::size(px(520.), px(800.)));
        view.run_until_parked();
        view.simulate_input("新增");
        view.run_until_parked();
        editor.update(view, |e, cx| {
            assert_ne!(e.snapshot(cx).revision(), original.revision());
            let row = e.hit_rows.iter().find(|row| row.line.0 == 2).unwrap();
            assert_eq!(row.table_layout.as_ref().unwrap().visual_rows, 1);
            let caret = e.selection().head();
            let source_local = (caret.0 - row.range.start.0) as usize;
            let before = row
                .position_for_display_index(row.display.source_to_display(source_local))
                .unwrap();
            assert_eq!(before.y, px(0.));
            assert!(e.scroll_x > 0.);
            assert!(row.text_origin_x + before.x < e.minimap.bounds.unwrap().left());
            let prose = e.hit_rows.iter().find(|row| row.line.0 == 5).unwrap();
            assert_eq!(prose.text_origin_x, prose_x);
            e.move_vertical(-1, false, cx);
            assert_ne!(e.selection().head(), caret);
        });
    }
}
