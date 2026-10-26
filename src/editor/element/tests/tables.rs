use super::*;

#[gpui::test]
fn unformatted_tables_keep_source_spacing_without_wrapping(cx: &mut gpui::TestAppContext) {
    use crate::document::ByteRange;

    cx.update(crate::editor::init);
    for path in ["catalog.org", "catalog.md"] {
        let rule = if path.ends_with("org") {
            "|-----+-----+-----|"
        } else {
            "|-----|-----|-----|"
        };
        let source = format!(
            "| Key | Name | Link |\n{rule}\n| one | 星际观测记录 | [[https://example.net/guide][Help]] |\n| two | 草木调查手册 | Index |\n"
        );
        let session = cx
            .new(|_| DocumentSession::from_utf8(path.into(), source.as_bytes().to_vec()).unwrap());
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session.clone(), cx));
        for width in [1200., 470.] {
            view.simulate_resize(gpui::size(px(width), px(600.)));
            view.run_until_parked();
            view.read(|cx| {
                let editor = editor.read(cx);
                let mut boundaries = Vec::new();
                for row in editor.hit_rows.iter().filter(|row| row.line.0 < 4) {
                    let table = row.table_layout.as_ref().unwrap();
                    assert_eq!(table.visual_rows, 1);
                    let mut x = px(0.);
                    for fragment in &table.fragments {
                        assert_eq!(fragment.x, x, "native spacing on row {}", row.line.0);
                        x += fragment.layout.width();
                    }
                    boundaries.push(table.column_widths.clone());
                }
                assert_eq!(boundaries.len(), 4);
                assert_ne!(boundaries[0], boundaries[2]);
                let document = session.read(cx);
                assert!(!document.is_dirty());
                assert_eq!(
                    document
                        .snapshot()
                        .copy_range(ByteRange::new(0, source.len() as u64)),
                    source
                );
            });
        }
    }
}

#[gpui::test]
fn table_metrics_cache_tracks_display_options_before_the_next_frame(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let session = cx.new(|_| {
        DocumentSession::from_utf8("metrics.org".into(), b"| *Label* | Value |\n".to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.run_until_parked();
    view.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let snapshot = editor.snapshot(cx);
            let options = editor.display_options(editor.session.read(cx).syntax_path(), &snapshot);
            let columns = editor
                .table_columns(
                    &snapshot,
                    crate::document::LineIndex(0),
                    crate::document::DocumentFormat::Org,
                )
                .unwrap();
            let measure = |options| {
                editor.table_column_metrics(
                    &snapshot,
                    &columns,
                    crate::typography::table_font(),
                    px(editor.font_size_px()),
                    window.text_system(),
                    options,
                )
            };
            let hidden = measure(options);
            let mut shown_options = options;
            shown_options.hide_emphasis_markers = false;
            let shown = measure(shown_options);
            assert!(shown.natural_widths[0] > hidden.natural_widths[0]);
        });
    });
}

#[gpui::test]
fn editor_table_entities_display_data_and_preserve_columns_caret_and_undo(
    cx: &mut gpui::TestAppContext,
) {
    use crate::document::{ByteOffset, ByteRange, Selection};
    cx.update(crate::editor::init);
    let cases = [
        (r"a\vert{}b", "a|b"),
        (r"\nbsp{}padded\nbsp{}", " padded "),
        (r"《示例：A12》\nbsp{}", "《示例：A12》 "),
        (
            "用于测试列宽的较长中文示例文本",
            "用于测试列宽的较长中文示例文本",
        ),
        (r"=\nbsp{}= \unknown{}", r"\nbsp{} \unknown{}"),
        (
            r"\ast{}粗体\ast{} \under{}下划线\under{} \equal{}原样\equal{} \tilde{}代码\tilde{}",
            "*粗体* _下划线_ =原样= ~代码~",
        ),
        (
            r"[\zwnj{}[\zwnj{}file:demo]\zwnj{}[\zwnj{}链接形态]\zwnj{}]\zwnj{}",
            "[[file:demo][链接形态]]",
        ),
    ];
    let mut source = "| value | next |\n|-+-|\n".to_owned();
    for (encoded, _) in cases {
        source.push_str(&format!("| {encoded} | tail |\n"));
    }
    let session = cx.new(|_| {
        DocumentSession::from_utf8("results.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.simulate_resize(gpui::size(px(1500.), px(1000.)));
    view.run_until_parked();
    view.read(|cx| {
        let editor = editor.read(cx);
        for (index, (_, expected)) in cases.iter().enumerate() {
            let row = editor
                .hit_rows
                .iter()
                .find(|row| row.line.0 == index as u64 + 2)
                .unwrap();
            let table = row
                .table_layout
                .as_ref()
                .expect("entities must use source-aware table shaping");
            let cells = table
                .fragments
                .iter()
                .filter(|fragment| !fragment.delimiter)
                .collect::<Vec<_>>();
            assert_eq!(cells.len(), 2);
            assert_eq!(
                cells[0]
                    .layout
                    .text
                    .replace('\u{200c}', "")
                    .replace('\u{a0}', " "),
                format!(" {expected} ")
            );
            assert_eq!(cells[1].layout.text.as_ref(), " tail ");
        }
        assert!(
            editor.link_hits.is_empty(),
            "entity display must not activate data-shaped links"
        );
        let row = editor.hit_rows.iter().find(|row| row.line.0 == 2).unwrap();
        let local = row.display.text.find(r"\vert{}").unwrap() + r"\vert{}".len();
        let table = row.table_layout.as_ref().unwrap();
        let position = table.position_for_index(local, row.line_height);
        assert_eq!(
            table.closest_index_for_position(position, row.line_height),
            local
        );
        let snapshot = editor.snapshot(cx);
        assert_eq!(
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
            source
        );
    });
    let offset = source.find(r"\vert{}").unwrap() + r"\vert{}".len();
    editor.update(view, |editor, cx| {
        editor.set_selection(Selection::caret(ByteOffset(offset as u64)), cx)
    });
    view.simulate_keystrokes("x");
    view.run_until_parked();
    view.read(|cx| {
        let snapshot = editor.read(cx).snapshot(cx);
        assert_eq!(
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
            source.replacen(r"a\vert{}b", r"a\vert{}xb", 1)
        );
    });
    view.simulate_keystrokes("cmd-z");
    view.run_until_parked();
    view.read(|cx| {
        let snapshot = editor.read(cx).snapshot(cx);
        assert_eq!(
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
            source
        );
    });
    editor.update(view, |editor, cx| {
        assert!(editor.align_table_at_selection(cx));
    });
    view.run_until_parked();
    let aligned = view.read(|cx| {
        let editor = editor.read(cx);
        let snapshot = editor.snapshot(cx);
        let source = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()));
        for (line, (encoded, _)) in source.lines().skip(2).zip(cases) {
            let parsed =
                crate::document::table::parse_line(line, crate::document::DocumentFormat::Org);
            assert_eq!(&line[parsed.cells[0].text_range.clone()], encoded);
        }
        let mut expected = None;
        for row in editor.hit_rows.iter() {
            let Some(table) = &row.table_layout else {
                continue;
            };
            let widths = table
                .fragments
                .iter()
                .filter(|fragment| fragment.delimiter)
                .map(|fragment| fragment.x)
                .collect::<Vec<_>>();
            if let Some(expected) = &expected {
                assert_eq!(
                    &widths, expected,
                    "displayed columns must align: {:?}",
                    row.display.text
                );
            } else {
                expected = Some(widths);
            }
        }
        source
    });
    view.simulate_keystrokes("tab");
    view.run_until_parked();
    view.read(|cx| {
        let snapshot = editor.read(cx).snapshot(cx);
        assert_eq!(
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
            aligned
        );
    });
}

#[gpui::test]
fn projected_table_cells_share_column_boundaries_across_rows(cx: &mut gpui::TestAppContext) {
    use crate::command::OrgDisplayFeature;
    use crate::document::ByteRange;

    cx.update(crate::editor::init);
    for (original, expected_cells) in [
        (
            "| Kind | Notation | End |\n|------+----------+-----|\n| Orbit | \\beta{} | stop |\n| Ratio | m\\vert{}n | tail |\n| Plain | =\\beta{}= | text |\n",
            ["Ratio", "m|n", "tail"],
        ),
        (
            "| Reference | Count | Comment |\n|-----------+-------+---------|\n| [[https://example.net/catalog/handbook][目录😀]] | 9 | One |\n| [[https://example.net/catalog/appendix][Appendix]] | \\alpha{} | Another |\n",
            ["Appendix", "α", "Another"],
        ),
    ] {
        let session = cx.new(|_| {
            DocumentSession::from_utf8("columns.org".into(), original.as_bytes().to_vec()).unwrap()
        });
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session.clone(), cx));
        view.simulate_resize(gpui::size(px(1800.), px(900.)));
        view.run_until_parked();
        view.read(|cx| {
            let row = editor
                .read(cx)
                .hit_rows
                .iter()
                .find(|row| row.line.0 == 3)
                .unwrap();
            let table = row.table_layout.as_ref().unwrap();
            let cells = table
                .fragments
                .iter()
                .filter(|fragment| !fragment.delimiter)
                .map(|fragment| fragment.layout.text.trim())
                .collect::<Vec<_>>();
            assert_eq!(cells, expected_cells);
        });

        let assert_columns = |view: &gpui::VisualTestContext| {
            view.read(|cx| {
                let editor = editor.read(cx);
                let mut expected = None;
                for row in editor
                    .hit_rows
                    .iter()
                    .filter(|row| row.table_layout.is_some())
                {
                    let table = row.table_layout.as_ref().unwrap();
                    let delimiters = table
                        .fragments
                        .iter()
                        .filter(|fragment| fragment.delimiter)
                        .map(|fragment| fragment.x)
                        .collect::<Vec<_>>();
                    assert_eq!(delimiters.len(), 4);
                    if let Some((positions, widths)) = &expected {
                        assert_eq!(
                            &delimiters, positions,
                            "table borders must align on source row {}",
                            row.line.0
                        );
                        assert_eq!(&table.column_widths, widths);
                    } else {
                        expected = Some((delimiters, table.column_widths.clone()));
                    }
                }
                assert!(expected.is_some());
            });
        };
        editor.update(view, |editor, cx| {
            assert!(editor.align_table_at_selection(cx))
        });
        view.run_until_parked();
        assert_columns(view);
        let aligned = view.read(|cx| {
            let snapshot = session.read(cx).snapshot();
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
        });
        for feature in [
            OrgDisplayFeature::Entities,
            OrgDisplayFeature::Entities,
            OrgDisplayFeature::DescriptiveLinks,
            OrgDisplayFeature::DescriptiveLinks,
            OrgDisplayFeature::EmphasisMarkers,
            OrgDisplayFeature::EmphasisMarkers,
        ] {
            editor.update(view, |editor, cx| editor.toggle_org_display(feature, cx));
            view.run_until_parked();
            assert_columns(view);
        }
        let entity = aligned
            .find("\\beta{}")
            .or_else(|| aligned.find("\\alpha{}"))
            .unwrap();
        editor.update(view, |editor, cx| {
            editor.set_selection(
                crate::document::Selection::caret(crate::document::ByteOffset(entity as u64 + 2)),
                cx,
            )
        });
        view.run_until_parked();
        assert_columns(view);
        if let Some(target) = aligned.find("catalog/handbook") {
            let range = ByteRange::new(
                target as u64,
                target as u64 + "catalog/handbook".len() as u64,
            );
            editor.update(view, |editor, cx| {
                editor.search_preview(std::sync::Arc::from([range]), Some(range), cx)
            });
            view.run_until_parked();
            assert_columns(view);
            editor.update(view, |editor, cx| editor.search_finish(true, cx));
            view.run_until_parked();
            assert_columns(view);
        }
        view.simulate_resize(gpui::size(px(470.), px(900.)));
        view.run_until_parked();
        assert_columns(view);
        view.read(|cx| {
            let snapshot = session.read(cx).snapshot();
            assert_eq!(
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
                aligned
            );
        });
    }
}
