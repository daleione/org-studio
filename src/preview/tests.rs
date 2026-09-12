use super::{
    GLOBAL_VISIBILITY_CYCLE_COMMAND, GlobalVisibility, MAX_EAGER_LAYOUT_ROWS, ORG_CONTEXT_COMMAND,
    REDO_DOCUMENT_COMMAND, TOGGLE_INLINE_IMAGE_PREVIEWS_COMMAND, UNDO_DOCUMENT_COMMAND,
    accept_generation, dired_command_items, preview_input, should_eagerly_measure_rows,
};

use crate::{
    app::{WorkspaceLoadState, WorkspaceWindow},
    document::{
        ByteOffset, ByteRange, DocumentCommand, EditOrigin, EditTransaction, Selection, TextEdit,
        TextSnapshot,
    },
    input::EmacsOutcome,
    keymap::KeyStroke,
};
use gpui::{AppContext, Focusable};

fn reading_semantic_golden(document: &super::PreviewSnapshot) -> String {
    use super::{
        CodeRowRole, DocumentFormat,
        markdown::MarkdownKind,
        projection::{ReadingCodeRow, VisualRowKind},
    };

    let display_map = document.display_map.as_ref().unwrap();
    let mut output = String::new();
    for (index, visual) in document.projection.rows.iter().enumerate() {
        let runs = display_map.runs(index);
        let text = runs.text.as_ref();
        let line = match &visual.kind {
            VisualRowKind::Blank | VisualRowKind::Hidden | VisualRowKind::Rule => continue,
            VisualRowKind::Heading(level) => format!("heading-{level}:{text}"),
            VisualRowKind::List(marker) => {
                let state = match marker.checkbox {
                    Some(super::org_line::CheckboxState::Empty) => "empty",
                    Some(super::org_line::CheckboxState::Partial) => "partial",
                    Some(super::org_line::CheckboxState::Checked) => "checked",
                    None => "none",
                };
                format!("list:{state}:{text}")
            }
            VisualRowKind::Caption => format!("caption:{text}"),
            VisualRowKind::Quote => format!("quote:{text}"),
            VisualRowKind::Code(ReadingCodeRow::End) => continue,
            VisualRowKind::Code(_) => {
                let language = visual.code_language.as_deref().unwrap_or("code");
                if document.format == DocumentFormat::Markdown {
                    match document.markdown_blocks[visual.block_id as usize].kind {
                        MarkdownKind::Code {
                            role: CodeRowRole::Open,
                            ..
                        } => format!("code-open:{language}"),
                        MarkdownKind::Code {
                            role: CodeRowRole::Close,
                            ..
                        } => "code-close".to_owned(),
                        _ => format!("code-body:{language}:{text}"),
                    }
                } else {
                    format!("code-body:{language}:{text}")
                }
            }
            VisualRowKind::Table(table) if table.is_separator() => "table-separator".to_owned(),
            VisualRowKind::Table(table) => {
                let cells = table
                    .cells()
                    .iter()
                    .map(|cell| cell.text(text))
                    .collect::<Vec<_>>()
                    .join("|");
                format!(
                    "table-{}:{cells}",
                    if table.is_header() { "header" } else { "body" }
                )
            }
            VisualRowKind::Image { .. } => "image".to_owned(),
            VisualRowKind::Diagram(_) => "diagram".to_owned(),
            VisualRowKind::Text => format!("text:{text}"),
        };
        output.push_str(&line);
        output.push('\n');
    }
    output
}

#[test]
fn org_reading_semantics_match_golden() {
    let document = loaded_document(
        "reading-basic.org",
        include_str!("../../tests/fixtures/reading-basic.org"),
    )
    .into_preview();
    assert_eq!(
        reading_semantic_golden(&document),
        include_str!("../../tests/goldens/reading-basic-org.txt")
    );
}

#[test]
fn markdown_reading_semantics_match_golden() {
    let document = loaded_document(
        "reading-basic.md",
        include_str!("../../tests/fixtures/reading-basic.md"),
    )
    .into_preview();
    assert_eq!(
        reading_semantic_golden(&document),
        include_str!("../../tests/goldens/reading-basic-md.txt")
    );
}

#[test]
fn reading_component_fixtures_have_stable_layout_contracts_for_both_styles_and_widths() {
    use super::{
        PreviewStyleId, layout::reading_content_width, projection::VisualRowKind,
        style::RowStyleKind,
    };

    for (path, source) in [
        (
            "reading-basic.org",
            include_str!("../../tests/fixtures/reading-basic.org"),
        ),
        (
            "reading-basic.md",
            include_str!("../../tests/fixtures/reading-basic.md"),
        ),
    ] {
        let document = loaded_document(path, source).into_preview();
        let display_map = document.display_map.as_deref().unwrap();
        let rows = &document.projection.rows;
        assert!(
            rows.iter()
                .any(|row| matches!(row.kind, VisualRowKind::Heading(_)))
        );
        assert!(
            rows.iter()
                .any(|row| matches!(row.kind, VisualRowKind::Text))
        );
        assert!(
            rows.iter()
                .any(|row| matches!(row.kind, VisualRowKind::List(_)))
        );
        assert!(
            rows.iter()
                .any(|row| matches!(row.kind, VisualRowKind::Table(_)))
        );
        assert!(
            rows.iter()
                .any(|row| matches!(row.kind, VisualRowKind::Quote))
        );
        assert!(
            rows.iter()
                .any(|row| matches!(row.kind, VisualRowKind::Code(_)))
        );
        assert!(
            rows.iter()
                .any(|row| matches!(row.kind, VisualRowKind::Image { .. }))
        );

        for style_id in PreviewStyleId::ALL {
            let style = *super::preview_style(style_id);
            for pane_width in [720.0, 1_400.0] {
                let content_width = reading_content_width(pane_width, 120.0, style);
                assert!(content_width >= 120.0);
                assert!(content_width <= pane_width - 120.0);
                for row in 0..document.projection.rows.len() {
                    let measure = display_map.estimated_measure(row, content_width, 1.0, style);
                    assert!(
                        measure.pixels.is_finite(),
                        "{path} row {row} has invalid height"
                    );
                    assert!(measure.display_lines >= 1);
                    if document
                        .projection
                        .rows
                        .get(row)
                        .expect("fixture row exists")
                        .style_kind
                        != RowStyleKind::Hidden
                    {
                        assert!(
                            measure.pixels > 0.0,
                            "{path} row {row} collapsed unexpectedly"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn markdown_closing_fence_is_zero_height_in_reading() {
    let document = loaded_document("code.md", "```rust\nlet value = 1;\n```\n").into_preview();
    let closing = document
        .projection
        .rows
        .iter()
        .find(|row| {
            matches!(
                row.kind,
                super::projection::VisualRowKind::Code(super::projection::ReadingCodeRow::End)
            )
        })
        .expect("fixture has a closing fence");
    assert_eq!(closing.style_kind, super::style::RowStyleKind::Hidden);
}

#[test]
fn markdown_plantuml_fence_becomes_one_in_memory_diagram_row() {
    use super::projection::VisualRowKind;

    let document = loaded_document(
        "diagram.md",
        "before\n```plantuml\n@startuml\nAlice -> Bob: hello\n@enduml\n```\nafter\n",
    )
    .into_preview();

    assert_eq!(document.projection.rows.len(), 7);
    assert!(matches!(
        document.projection.rows.get(1).unwrap().kind,
        VisualRowKind::Diagram(super::diagram::DiagramProjection::Ready { .. })
    ));
    assert_eq!(
        document.display_map.as_ref().unwrap().runs(1).text.as_ref(),
        ""
    );
    assert_eq!(
        document
            .projection
            .rows
            .iter()
            .filter(|row| matches!(row.kind, VisualRowKind::Hidden))
            .count(),
        4
    );
}

#[test]
fn org_plantuml_source_block_stays_source_until_babel_execution() {
    use super::projection::VisualRowKind;

    let document = loaded_document(
        "diagram.org",
        "before\n#+begin_src plantuml :file diagram.svg\n@startuml\nAlice -> Bob: hello\n@enduml\n#+end_src\nafter\n",
    )
    .into_preview();

    assert_eq!(document.projection.rows.len(), 5);
    assert!(
        document
            .projection
            .rows
            .iter()
            .any(|row| matches!(row.kind, VisualRowKind::Code(_)))
    );
    assert!(
        !document
            .projection
            .rows
            .iter()
            .any(|row| matches!(row.kind, VisualRowKind::Diagram(_)))
    );
}

#[test]
fn org_plantuml_result_image_is_rendered_below_the_source_block() {
    use super::projection::VisualRowKind;

    let document = loaded_document(
        "diagram.org",
        "#+begin_src plantuml :file diagram.svg\n@startuml\nAlice -> Bob: hello\n@enduml\n#+end_src\n\n#+RESULTS:\n[[file:diagram.svg]]\n",
    )
    .into_preview();
    let code = document
        .projection
        .rows
        .iter()
        .position(|row| matches!(row.kind, VisualRowKind::Code(_)))
        .expect("source code remains visible");
    let image = document
        .projection
        .rows
        .iter()
        .position(|row| matches!(row.kind, VisualRowKind::Image { .. }))
        .expect("the file result is projected as an image");

    assert!(code < image);
    assert!(
        !document
            .projection
            .rows
            .iter()
            .any(|row| matches!(row.kind, VisualRowKind::Diagram(_)))
    );
}

#[test]
fn ordinary_org_source_block_keeps_its_physical_code_rows() {
    use super::projection::VisualRowKind;

    let document = loaded_document(
        "code.org",
        "#+begin_src rust\nfn main() {\n    println!(\"hello\");\n}\n#+end_src\n",
    )
    .into_preview();

    assert_eq!(document.projection.rows.len(), 3);
    assert!(
        document
            .projection
            .rows
            .iter()
            .all(|row| matches!(row.kind, VisualRowKind::Code(_)))
    );
}

#[test]
fn reading_checkbox_action_only_targets_the_structural_prefix() {
    let document =
        loaded_document("checkbox.org", "- [ ] task\n- literal [ ] text\n").into_preview();
    let actions = document
        .projection
        .rows
        .iter()
        .map(|row| super::checkbox_action(&document, row).is_some())
        .collect::<Vec<_>>();
    assert_eq!(actions, vec![true, false]);
}

#[test]
fn markdown_code_action_uses_the_precomputed_body_range() {
    let source = "```rust\nlet value = 1;\n```\n";
    let document = loaded_document("code.md", source).into_preview();
    let action = document
        .projection
        .rows
        .iter()
        .position(|row| row.code_action_range.is_some())
        .and_then(|row| super::code_action(&document, row))
        .expect("opening fence exposes a copy action");
    assert_eq!(
        document.text.copy_range(action.target().source_range),
        "let value = 1;\n"
    );
}

#[test]
fn malformed_reading_input_falls_back_without_building_a_second_document_model() {
    let document = loaded_document(
        "malformed.org",
        "* Heading\n| unfinished table\n#+begin_src rust\nlet value = 1;\n",
    )
    .into_preview();
    let display_map = document.display_map.as_ref().unwrap();

    assert_eq!(display_map.projection.revision, document.revision);
    assert_eq!(
        display_map.projection.rows.len(),
        document.projection.rows.len()
    );
    for row in 0..document.projection.rows.len() {
        let _ = display_map.runs(row);
    }
}

#[test]
fn large_reading_projection_keeps_display_materialization_viewport_bounded() {
    let source = (0..20_000)
        .map(|index| format!("* Heading {index}\nbody {index}\n"))
        .collect::<String>();
    let document = loaded_document("large-reading.org", &source).into_preview();
    let display_map = document.display_map.as_ref().unwrap();

    assert!(document.projection.chunk_count() > 100);
    assert!(display_map.display_runs.lock().unwrap().entries.is_empty());
    for row in 10_000..10_032 {
        let _ = display_map.runs(row);
    }
    assert_eq!(display_map.display_runs.lock().unwrap().entries.len(), 32);
}

fn visible_source_lines(
    app: &WorkspaceWindow,
    document: &super::PreviewSnapshot,
    cx: &gpui::App,
) -> Vec<u64> {
    app.reading_panel()
        .unwrap()
        .read(cx)
        .visible_rows()
        .iter()
        .map(|index| {
            document.text.line_of_byte(
                document
                    .projection
                    .rows
                    .get(*index)
                    .unwrap()
                    .source
                    .range
                    .start,
            ) + 1
        })
        .collect()
}

#[test]
fn eager_layout_is_limited_to_small_documents() {
    assert!(should_eagerly_measure_rows(MAX_EAGER_LAYOUT_ROWS));
    assert!(!should_eagerly_measure_rows(MAX_EAGER_LAYOUT_ROWS + 1));
}

#[test]
fn folded_tail_keeps_reading_bottom_padding_in_minimap_geometry() {
    let document =
        loaded_document("folded-tail.org", "* Heading\nbody\n* Hidden tail\nlast\n").into_preview();
    let display_map = document.display_map.as_ref().unwrap();
    let style = *super::preview_style(super::PreviewStyleId::WarmClay);
    let measure = display_map.estimated_measure(0, 760.0, 1.0, style);
    let padded = display_map.with_presentation_tail_padding(0, 0, 1, 1.0, style, measure);

    assert_eq!(
        padded.pixels - measure.pixels,
        style.spacing.content_padding_bottom
    );
    assert_eq!(padded.display_lines, measure.display_lines);
}

fn loaded_document(path: &str, source: &str) -> super::LoadedDocument {
    let path = std::path::PathBuf::from(path);
    let session =
        crate::document::DocumentSession::from_utf8(path.clone(), source.as_bytes().to_vec())
            .unwrap();
    let preview = super::loading::derive_preview(path, session.snapshot());
    super::LoadedDocument::new(session, preview).unwrap()
}

#[gpui::test]
fn opening_split_from_single_reading_reuses_the_current_snapshot(cx: &mut gpui::TestAppContext) {
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    workspace.update(cx, |workspace, cx| {
        workspace.show_reading(cx);
        assert!(workspace.apply_load_result(
            0,
            Ok(loaded_document("single-reading.org", "* Heading\nbody\n")),
            cx,
        ));
        let ready = workspace.state.ready().unwrap();
        assert!(ready.readers.left.is_some());
        assert!(ready.readers.right.is_none());

        workspace.show_split(cx);
        let ready = workspace.state.ready().unwrap();
        let left = ready
            .readers
            .left
            .as_ref()
            .unwrap()
            .read(cx)
            .document()
            .clone();
        let right = ready
            .readers
            .right
            .as_ref()
            .unwrap()
            .read(cx)
            .document()
            .clone();
        assert!(std::sync::Arc::ptr_eq(&left, &right));
    });
}

#[gpui::test]
fn source_action_uses_the_focused_editor_instead_of_the_previous_active_pane(
    cx: &mut gpui::TestAppContext,
) {
    let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(true));
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert!(workspace.apply_load_result(
                0,
                Ok(loaded_document("source-origin.org", "* Heading\n")),
                cx,
            ));
            workspace.toggle_pane_surface(crate::app::PaneSide::Right, cx);
            workspace.document_workspace.active_pane = crate::app::PaneSide::Left;
        });
        let right = workspace
            .read(cx)
            .editor(crate::app::PaneSide::Right)
            .unwrap();
        window.focus(&right.read(cx).focus_handle(cx), cx);
        let (pane, editor) = workspace
            .read(cx)
            .source_editor_for_action(window, cx)
            .unwrap();
        assert_eq!(pane, crate::app::PaneSide::Right);
        assert_eq!(editor.entity_id(), right.entity_id());
    });
}

#[gpui::test]
fn pane_reading_zoom_survives_surface_switch_and_is_not_shared(cx: &mut gpui::TestAppContext) {
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(true));
    workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(
            0,
            Ok(loaded_document("pane-zoom.org", "* Heading\nbody\n")),
            cx,
        ));
        workspace.toggle_pane_surface(crate::app::PaneSide::Left, cx);
        let ready = workspace.state.ready().unwrap();
        ready.readers.left.as_ref().unwrap().update(cx, |panel, _| {
            assert!(panel.set_content_font_size(crate::typography::ContentFontSize::new(18)));
        });
        ready
            .readers
            .right
            .as_ref()
            .unwrap()
            .update(cx, |panel, _| {
                assert!(panel.set_content_font_size(crate::typography::ContentFontSize::new(14)));
            });

        workspace.toggle_pane_surface(crate::app::PaneSide::Left, cx);
        workspace.toggle_pane_surface(crate::app::PaneSide::Left, cx);
        let ready = workspace.state.ready().unwrap();
        assert_eq!(
            ready.readers.left.as_ref().unwrap().read(cx).zoom(),
            18.0 / 15.0
        );
        assert_eq!(
            ready.readers.right.as_ref().unwrap().read(cx).zoom(),
            14.0 / 15.0
        );
    });
}

#[gpui::test]
fn content_font_size_is_per_pane_and_shared_by_editor_and_reading(cx: &mut gpui::TestAppContext) {
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(true));
    workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(
            0,
            Ok(loaded_document("pane-font-size.org", "* Heading\nbody\n",)),
            cx,
        ));

        workspace.increase_content_font_size(cx);
        assert_eq!(workspace.content_font_sizes.left.get(), 16);
        assert_eq!(workspace.content_font_sizes.right.get(), 15);
        let ready = workspace.state.ready().unwrap();
        assert_eq!(
            ready
                .editors
                .left
                .as_ref()
                .unwrap()
                .read(cx)
                .content_font_size()
                .get(),
            16
        );
        workspace.toggle_pane_surface(crate::app::PaneSide::Left, cx);
        let ready = workspace.state.ready().unwrap();
        assert_eq!(
            ready.readers.left.as_ref().unwrap().read(cx).zoom(),
            16.0 / 15.0
        );

        workspace.activate_pane(crate::app::PaneSide::Right, cx);
        workspace.decrease_content_font_size(cx);
        assert_eq!(workspace.content_font_sizes.left.get(), 16);
        assert_eq!(workspace.content_font_sizes.right.get(), 14);
        let ready = workspace.state.ready().unwrap();
        assert_eq!(
            ready.readers.right.as_ref().unwrap().read(cx).zoom(),
            14.0 / 15.0
        );
        workspace.toggle_pane_surface(crate::app::PaneSide::Right, cx);
        let ready = workspace.state.ready().unwrap();
        assert_eq!(
            ready
                .editors
                .right
                .as_ref()
                .unwrap()
                .read(cx)
                .content_font_size()
                .get(),
            14
        );

        workspace.reset_content_font_size(cx);
        assert_eq!(workspace.content_font_sizes.left.get(), 16);
        assert_eq!(workspace.content_font_sizes.right.get(), 15);
    });
}

#[gpui::test]
fn reading_font_size_preserves_source_anchor_across_extreme_values(cx: &mut gpui::TestAppContext) {
    let source = (0..120)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    let preview = loaded_document("reading-font-anchor.org", &source).into_preview();
    let panel = cx.new(|_| super::ReadingPreviewPanel::new(std::sync::Arc::new(preview), 80.0));
    panel.update(cx, |panel, _| {
        panel.scroll_to(gpui::ListOffset {
            item_ix: 40,
            offset_in_item: gpui::px(7.0),
        });
        let source_anchor = panel.top_source_offset();
        for size in [96, 5, 15] {
            assert!(panel.set_content_font_size(crate::typography::ContentFontSize::new(size),));
            assert_eq!(panel.top_source_offset(), source_anchor);
            assert_eq!(panel.list_state().logical_scroll_top().item_ix, 40);
            assert_eq!(
                panel.list_state().logical_scroll_top().offset_in_item,
                gpui::px(7.0)
            );
            assert_eq!(panel.zoom(), size as f32 / 15.0);
        }
    });
}

#[gpui::test]
fn content_font_size_shortcut_context_follows_document_availability(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "cmd-=",
            super::IncreaseContentFontSize,
            Some(super::DOCUMENT_WORKSPACE_KEY_CONTEXT),
        )]);
    });
    let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));

    cx.simulate_keystrokes("cmd-=");
    assert_eq!(
        cx.read(|cx| workspace.read(cx).content_font_sizes.left.get()),
        15
    );

    let source = (0..120)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    cx.update(|_, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.show_reading(cx);
            assert!(workspace.apply_load_result(
                0,
                Ok(loaded_document("shortcut-font-size.org", &source)),
                cx,
            ));
        });
    });
    cx.simulate_keystrokes("cmd-=");
    assert_eq!(
        cx.read(|cx| workspace.read(cx).content_font_sizes.left.get()),
        16
    );

    cx.update(|_, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.show_editor(cx);
        });
    });
    cx.simulate_keystrokes("cmd-=");
    assert_eq!(
        cx.read(|cx| workspace.read(cx).content_font_sizes.left.get()),
        17
    );

    cx.update(|_, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.show_reading(cx);
            for _ in 17..96 {
                workspace.increase_content_font_size(cx);
            }
        });
    });
    cx.run_until_parked();
    let reader = cx.read(|cx| {
        workspace
            .read(cx)
            .reading_panel_for(crate::app::PaneSide::Left)
            .unwrap()
    });
    cx.read(|cx| {
        let reader = reader.read(cx);
        assert_eq!(reader.zoom(), 96.0 / 15.0);
        let first = reader
            .list_state()
            .bounds_for_item(0)
            .expect("the first reading row is laid out at 96 px");
        assert!(first.size.height > gpui::px(0.0));
        assert!(reader.list_state().viewport_bounds().size.height > gpui::px(0.0));
    });

    cx.update(|_, cx| {
        reader.update(cx, |reader, _| reader.scroll_to_end());
        workspace.update(cx, |_, cx| cx.notify());
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| {
        super::minimap::list_viewport_reaches_document_end(reader.read(cx).list_state())
    }));

    cx.update(|_, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.reset_content_font_size(cx);
            for _ in 5..15 {
                workspace.decrease_content_font_size(cx);
            }
        });
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let reader = reader.read(cx);
        assert_eq!(reader.zoom(), 5.0 / 15.0);
        assert!(super::minimap::list_viewport_reaches_document_end(
            reader.list_state()
        ));
    });

    cx.update(|_, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.show_export_panel(cx);
        });
    });
    cx.simulate_keystrokes("cmd-=");
    assert_eq!(
        cx.read(|cx| workspace.read(cx).content_font_sizes.left.get()),
        5
    );

    cx.update(|_, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.close_export_panel(cx);
            workspace.content_route = crate::app::ContentRoute::FileManager;
            cx.notify();
        });
    });
    cx.simulate_keystrokes("cmd-=");
    assert_eq!(
        cx.read(|cx| workspace.read(cx).content_font_sizes.left.get()),
        5
    );
}

#[gpui::test]
fn opening_a_split_inherits_the_active_pane_font_size(cx: &mut gpui::TestAppContext) {
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(
            0,
            Ok(loaded_document("split-font-size.org", "* Heading\nbody\n",)),
            cx,
        ));
        for _ in 0..5 {
            workspace.increase_content_font_size(cx);
        }
        workspace.show_split(cx);

        assert_eq!(workspace.content_font_sizes.left.get(), 20);
        assert_eq!(workspace.content_font_sizes.right.get(), 20);
        workspace.toggle_pane_surface(crate::app::PaneSide::Right, cx);
        let ready = workspace.state.ready().unwrap();
        assert_eq!(
            ready
                .editors
                .right
                .as_ref()
                .unwrap()
                .read(cx)
                .content_font_size()
                .get(),
            20
        );
    });
}

#[gpui::test]
fn reopening_a_split_after_loading_another_document_preserves_independent_font_sizes(
    cx: &mut gpui::TestAppContext,
) {
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(
            0,
            Ok(loaded_document("first-font-size.org", "* First\nbody\n")),
            cx,
        ));
        workspace.increase_content_font_size(cx);
        workspace.show_split(cx);
        assert_eq!(workspace.content_font_sizes.left.get(), 16);
        assert_eq!(workspace.content_font_sizes.right.get(), 16);

        workspace.activate_pane(crate::app::PaneSide::Right, cx);
        for _ in 0..4 {
            workspace.increase_content_font_size(cx);
        }
        workspace.activate_pane(crate::app::PaneSide::Left, cx);
        workspace.show_editor(cx);
        assert!(workspace.apply_load_result(
            0,
            Ok(loaded_document("second-font-size.org", "* Second\nbody\n")),
            cx,
        ));
        let ready = workspace.state.ready().unwrap();
        assert!(ready.editors.right.is_none());
        assert!(ready.readers.right.is_none());

        workspace.show_split(cx);
        assert_eq!(workspace.content_font_sizes.left.get(), 16);
        assert_eq!(workspace.content_font_sizes.right.get(), 20);
    });
}

#[gpui::test]
fn surface_switch_preserves_the_top_source_line(cx: &mut gpui::TestAppContext) {
    let source = (0..80)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(true));
    workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(
            0,
            Ok(loaded_document("surface-anchor.org", &source)),
            cx,
        ));

        let ready = workspace.state.ready().unwrap();
        let session = ready.session.clone();
        let snapshot = ready.session.read(cx).snapshot();
        let editor_anchor = snapshot
            .line_content_range(crate::document::LineIndex(24))
            .unwrap()
            .start;
        ready
            .editors
            .left
            .as_ref()
            .unwrap()
            .update(cx, |editor, cx| {
                assert!(editor.scroll_to_source_offset(editor_anchor, cx));
            });

        workspace.toggle_pane_surface(crate::app::PaneSide::Left, cx);
        let reader = workspace
            .reading_panel_for(crate::app::PaneSide::Left)
            .expect("reading panel exists after switching");
        assert_eq!(reader.read(cx).top_source_offset(), Some(editor_anchor));

        let reading_anchor = snapshot
            .line_content_range(crate::document::LineIndex(48))
            .unwrap()
            .start;
        reader.update(cx, |reader, _| {
            assert!(reader.scroll_to_source_offset(reading_anchor));
        });

        let inserted = "new first line\n";
        session.update(cx, |session, cx| {
            session
                .edit(
                    DocumentCommand::new(
                        EditTransaction::new(
                            session.revision(),
                            vec![TextEdit::new(ByteRange::new(0, 0), inserted)],
                        ),
                        Selection::caret(ByteOffset(0)),
                        Selection::caret(ByteOffset(inserted.len() as u64)),
                        EditOrigin::Typing,
                    ),
                    cx,
                )
                .unwrap();
        });

        workspace.toggle_pane_surface(crate::app::PaneSide::Left, cx);
        let editor = workspace.editor(crate::app::PaneSide::Left).unwrap();
        let current = session.read(cx).snapshot();
        let (restored, _) = editor.read(cx).top_source_anchor(&current);
        assert_eq!(
            restored,
            ByteOffset(reading_anchor.0 + inserted.len() as u64)
        );
    });
}

#[gpui::test]
fn editor_to_reading_keeps_the_top_line_across_async_projection(cx: &mut gpui::TestAppContext) {
    let source = (0..80)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    let session = crate::document::DocumentSession::from_utf8(
        std::path::PathBuf::from("async-surface-anchor.org"),
        source.into_bytes(),
    )
    .unwrap();
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    let (anchor, session) = workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(
            0,
            Ok(super::WorkspaceLoadedDocument::Source(Box::new(session))),
            cx,
        ));
        let ready = workspace.state.ready().unwrap();
        let session = ready.session.clone();
        let snapshot = ready.session.read(cx).snapshot();
        let anchor = snapshot
            .line_content_range(crate::document::LineIndex(32))
            .unwrap()
            .start;
        ready
            .editors
            .left
            .as_ref()
            .unwrap()
            .update(cx, |editor, cx| {
                assert!(editor.scroll_to_source_offset(anchor, cx));
            });
        workspace.show_reading(cx);
        assert!(
            workspace
                .reading_panel_for(crate::app::PaneSide::Left)
                .is_none()
        );
        (anchor, session)
    });

    let inserted = "inserted above\n";
    let delta = session.update(cx, |session, cx| {
        session
            .edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        session.revision(),
                        vec![TextEdit::new(ByteRange::new(0, 0), inserted)],
                    ),
                    Selection::caret(ByteOffset(0)),
                    Selection::caret(ByteOffset(inserted.len() as u64)),
                    EditOrigin::Typing,
                ),
                cx,
            )
            .unwrap()
    });
    workspace.update(cx, |workspace, cx| {
        workspace.schedule_derived_update_with_delta(Some(delta), cx)
    });

    cx.executor()
        .advance_clock(std::time::Duration::from_millis(25));
    cx.run_until_parked();

    cx.read(|cx| {
        let reader = workspace
            .read(cx)
            .reading_panel_for(crate::app::PaneSide::Left)
            .expect("async reading panel is published");
        assert_eq!(
            reader.read(cx).top_source_offset(),
            Some(ByteOffset(anchor.0 + inserted.len() as u64))
        );
    });
}

#[gpui::test]
fn reading_style_switch_reuses_projection_and_invalidates_geometry_once(
    cx: &mut gpui::TestAppContext,
) {
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(true));
    workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(
            0,
            Ok(loaded_document("style-switch.org", "* Heading\nbody\n")),
            cx,
        ));
        let panel = workspace
            .state
            .ready()
            .unwrap()
            .readers
            .right
            .as_ref()
            .unwrap()
            .clone();
        let original = panel.read(cx).document().clone();
        let initial = panel.read(cx).render_state().geometry_revision;
        let base = *super::preview_style(super::PreviewStyleId::Base);
        let warm = *super::preview_style(super::PreviewStyleId::WarmClay);

        panel.update(cx, |panel, cx| panel.change_style(base, base, cx));
        assert_eq!(panel.read(cx).render_state().geometry_revision, initial);

        let mut paint_only = base;
        paint_only.id = super::PreviewStyleId::WarmClay;
        paint_only.palette.background ^= 0x00010101;
        assert_eq!(base.layout_key(), paint_only.layout_key());
        assert_ne!(base.paint_key(), paint_only.paint_key());
        panel.update(cx, |panel, cx| panel.change_style(base, paint_only, cx));
        let paint_state = panel.read(cx).render_state();
        assert_eq!(paint_state.geometry_revision, initial);
        assert_eq!(
            paint_state
                .minimap_state
                .raster_epoch
                .load(std::sync::atomic::Ordering::Acquire),
            1,
        );

        panel.update(cx, |panel, cx| panel.change_style(paint_only, warm, cx));
        let state = panel.read(cx).render_state();
        assert_eq!(state.geometry_revision, initial.wrapping_add(1));
        assert_eq!(
            state
                .minimap_state
                .raster_epoch
                .load(std::sync::atomic::Ordering::Acquire),
            2,
        );
        assert!(std::sync::Arc::ptr_eq(&original, &state.document));
        assert!(std::sync::Arc::ptr_eq(
            &original.projection,
            &state.document.projection,
        ));
    });
}

#[gpui::test]
fn reopening_a_hidden_reading_pane_catches_it_up_without_losing_its_viewport(
    cx: &mut gpui::TestAppContext,
) {
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(true));
    let session = workspace.update(cx, |workspace, cx| {
        workspace.toggle_pane_surface(crate::app::PaneSide::Left, cx);
        assert!(
            workspace.apply_load_result(
                0,
                Ok(loaded_document(
                    "hidden-reading.org",
                    &(0..300)
                        .map(|index| format!("* Heading {index}\nbody {index}\n"))
                        .collect::<String>(),
                )),
                cx,
            )
        );
        let ready = workspace.state.ready().unwrap();
        let hidden = ready.readers.right.as_ref().unwrap();
        hidden.update(cx, |panel, _| {
            panel.scroll_to(gpui::ListOffset {
                item_ix: 120,
                offset_in_item: gpui::px(6.0),
            });
        });
        assert_eq!(
            hidden.read(cx).list_state().logical_scroll_top().item_ix,
            120
        );
        let session = ready.session.clone();
        workspace.show_reading(cx);
        session
    });
    let delta = session.update(cx, |session, cx| {
        let before = session.snapshot();
        session
            .edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        before.revision(),
                        vec![TextEdit::new(ByteRange::new(0, 0), "preamble\n")],
                    ),
                    Selection::caret(ByteOffset(0)),
                    Selection::caret(ByteOffset(9)),
                    EditOrigin::Typing,
                ),
                cx,
            )
            .unwrap()
    });
    workspace.update(cx, |workspace, cx| {
        workspace.schedule_derived_update_with_delta(Some(delta), cx)
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(25));
    cx.run_until_parked();

    workspace.update(cx, |workspace, cx| {
        let latest_revision = workspace.document_session().unwrap().read(cx).revision();
        let hidden_revision = workspace
            .state
            .ready()
            .unwrap()
            .readers
            .right
            .as_ref()
            .unwrap()
            .read(cx)
            .document()
            .revision;
        assert_ne!(hidden_revision, latest_revision);

        workspace.show_split(cx);
        let hidden = workspace
            .state
            .ready()
            .unwrap()
            .readers
            .right
            .as_ref()
            .unwrap();
        assert_eq!(hidden.read(cx).document().revision, latest_revision);
        let offset = hidden.read(cx).list_state().logical_scroll_top();
        assert!(offset.item_ix > 100, "restored offset: {offset:?}");
        assert_eq!(offset.offset_in_item, gpui::px(6.0));
    });
}

#[gpui::test]
fn editors_are_created_on_demand(cx: &mut gpui::TestAppContext) {
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    workspace.update(cx, |workspace, cx| {
        assert!(
            workspace.apply_load_result(
                0,
                Ok(super::WorkspaceLoadedDocument::Source(Box::new(
                    crate::document::DocumentSession::from_utf8(
                        std::path::PathBuf::from("lazy-editor.org"),
                        b"* Heading\nbody\n".to_vec(),
                    )
                    .unwrap(),
                ))),
                cx,
            )
        );
        let ready = workspace.state.ready().unwrap();
        assert!(
            ready
                .editors
                .left
                .as_ref()
                .unwrap()
                .read(cx)
                .autofocus_pending()
        );
        assert!(ready.editors.right.is_none());

        workspace.show_split(cx);
        assert!(workspace.state.ready().unwrap().editors.right.is_none());
        workspace.toggle_pane_surface(crate::app::PaneSide::Right, cx);
        let ready = workspace.state.ready().unwrap();
        assert!(
            ready
                .editors
                .right
                .as_ref()
                .unwrap()
                .read(cx)
                .autofocus_pending()
        );
    });
}

#[gpui::test]
fn loading_two_editor_panes_only_autofocuses_the_active_one(cx: &mut gpui::TestAppContext) {
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(true));
    workspace.update(cx, |workspace, cx| {
        workspace.toggle_pane_surface(crate::app::PaneSide::Right, cx);
        workspace.activate_pane(crate::app::PaneSide::Left, cx);
        assert!(
            workspace.apply_load_result(
                0,
                Ok(super::WorkspaceLoadedDocument::Source(Box::new(
                    crate::document::DocumentSession::from_utf8(
                        std::path::PathBuf::from("two-editors.org"),
                        b"* Heading\nbody\n".to_vec(),
                    )
                    .unwrap(),
                ))),
                cx,
            )
        );
        let ready = workspace.state.ready().unwrap();
        assert!(
            ready
                .editors
                .left
                .as_ref()
                .unwrap()
                .read(cx)
                .autofocus_pending()
        );
        assert!(
            !ready
                .editors
                .right
                .as_ref()
                .unwrap()
                .read(cx)
                .autofocus_pending()
        );
    });
}

#[gpui::test]
fn a_path_change_invalidates_and_rebuilds_the_shared_reading_snapshot(
    cx: &mut gpui::TestAppContext,
) {
    let root = std::env::temp_dir().join(format!(
        "org-studio-reading-path-change-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let old_path = root.join("old.org");
    let new_path = root.join("new.org");
    std::fs::write(&old_path, "* Heading\nbody\n").unwrap();
    std::fs::write(&new_path, "* Heading\nbody\n").unwrap();
    let loaded = super::load_document(old_path.clone()).unwrap();
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(true));
    let session = workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(0, Ok(loaded), cx));
        assert!(workspace.latest_preview_is_current(cx));
        workspace.document_session().unwrap().clone()
    });
    let stamp = crate::document::FileStamp::read(&new_path).unwrap();
    session.update(cx, |session, cx| {
        session
            .retarget_moved_file(new_path.clone(), stamp, None, cx)
            .unwrap();
    });
    workspace.update(cx, |workspace, cx| {
        assert!(!workspace.latest_preview_is_current(cx));
        workspace.schedule_derived_update(cx);
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(25));
    cx.run_until_parked();

    workspace.read_with(cx, |workspace, cx| {
        assert!(workspace.latest_preview_is_current(cx));
        assert_eq!(workspace.derived.latest.as_ref().unwrap().path, new_path);
        assert_eq!(
            workspace
                .reading_panel_for(crate::app::PaneSide::Right)
                .unwrap()
                .read(cx)
                .document()
                .path,
            new_path
        );
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn a_resource_change_refreshes_every_editor_and_rebuilds_reading_at_the_same_revision(
    cx: &mut gpui::TestAppContext,
) {
    let root = std::env::temp_dir().join(format!(
        "org-studio-reading-resource-change-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let document_path = root.join("notes.org");
    let image_path = root.join("result.svg");
    std::fs::write(&document_path, "#+RESULTS:\n[[file:result.svg]]\n").unwrap();
    std::fs::write(
        &image_path,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"></svg>"#,
    )
    .unwrap();
    let loaded = super::load_document(document_path).unwrap();
    let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(true));
    let (session, before) = cx.update(|_, cx| {
        workspace.update(cx, |workspace, cx| {
            assert!(workspace.apply_load_result(0, Ok(loaded), cx));
            workspace.toggle_pane_surface(crate::app::PaneSide::Right, cx);
            workspace.toggle_pane_surface(crate::app::PaneSide::Right, cx);
            (
                workspace.document_session().unwrap().clone(),
                workspace.derived.latest.as_ref().unwrap().clone(),
            )
        })
    });
    cx.run_until_parked();

    cx.update(|_, cx| {
        session.update(cx, |session, cx| {
            session.resource_changed(image_path.clone(), cx)
        })
    });
    cx.read(|cx| {
        let workspace = workspace.read(cx);
        let ready = workspace.state.ready().unwrap();
        for editor in [&ready.editors.left, &ready.editors.right]
            .into_iter()
            .flatten()
        {
            assert_eq!(editor.read(cx).inline_image_resource_generation(), 1);
        }
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(25));
    cx.run_until_parked();

    cx.read(|cx| {
        let workspace = workspace.read(cx);
        let latest = workspace.derived.latest.as_ref().unwrap();
        assert_eq!(latest.revision, before.revision);
        assert!(!std::sync::Arc::ptr_eq(latest, &before));
        let panel = workspace
            .reading_panel_for(crate::app::PaneSide::Right)
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(panel.read(cx).document(), latest));
    });
    std::fs::remove_file(image_path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn minimap_width_changes_reach_every_materialized_editor(cx: &mut gpui::TestAppContext) {
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    workspace.update(cx, |workspace, cx| {
        assert!(
            workspace.apply_load_result(
                0,
                Ok(super::WorkspaceLoadedDocument::Source(Box::new(
                    crate::document::DocumentSession::from_utf8(
                        std::path::PathBuf::from("minimap-width.org"),
                        b"body\n".to_vec(),
                    )
                    .unwrap(),
                ))),
                cx,
            )
        );
        workspace.show_split(cx);
        workspace.toggle_pane_surface(crate::app::PaneSide::Right, cx);
        workspace.change_minimap_width(super::minimap::MinimapWidthChange::Commit(144.0), cx);
        let ready = workspace.state.ready().unwrap();
        assert_eq!(
            ready
                .editors
                .left
                .as_ref()
                .unwrap()
                .read(cx)
                .minimap_width(),
            144.0
        );
        assert_eq!(
            ready
                .editors
                .right
                .as_ref()
                .unwrap()
                .read(cx)
                .minimap_width(),
            144.0
        );
    });
}

#[gpui::test]
fn incremental_document_replacement_preserves_list_and_fold_state(cx: &mut gpui::TestAppContext) {
    let source = (0..400)
        .map(|index| format!("* Heading {index}\nbody {index}\n"))
        .collect::<String>();
    let mut buffer = crate::document::DocumentBuffer::from_utf8(source.into_bytes()).unwrap();
    let before = buffer.snapshot();
    let previous =
        super::loading::derive_preview(std::path::PathBuf::from("panel-state.org"), before.clone());
    let folded = previous
        .blocks
        .nodes()
        .iter()
        .position(|block| {
            matches!(block.kind, crate::org_syntax::BlockKind::Heading { .. })
                && previous.text.copy_range(block.content) == "Heading 200"
        })
        .unwrap() as crate::org_syntax::BlockId;
    let panel = cx.new(|_| super::ReadingPreviewPanel::new(std::sync::Arc::new(previous), 80.0));
    panel.update(cx, |panel, _| {
        panel.toggle_fold(folded);
        panel.list_state().scrollbar_drag_started();
        panel.scroll_to(gpui::ListOffset {
            item_ix: 200,
            offset_in_item: gpui::px(7.0),
        });
    });

    let edit = before
        .copy_range(ByteRange::new(0, before.len_bytes()))
        .find("body 100")
        .unwrap() as u64
        + 5;
    let delta = buffer
        .commit(EditTransaction::new(
            before.revision(),
            vec![TextEdit::new(ByteRange::new(edit, edit + 3), "one hundred")],
        ))
        .unwrap();
    let next = cx.read(|cx| {
        let previous = panel.read(cx).document().clone();
        super::loading::derive_preview_incremental(
            std::path::PathBuf::from("panel-state.org"),
            buffer.snapshot(),
            Some(&previous),
            &[delta],
        )
    });
    panel.update(cx, |panel, cx| {
        panel.replace_document_with_style(
            std::sync::Arc::new(next),
            *super::preview_style(super::PreviewStyleId::Base),
            cx,
        );
        assert!(panel.list_state().is_scrollbar_dragging());
        assert_eq!(panel.fold_markers().len(), 1);
        assert!(panel.fold_markers().contains(&folded));
        let scroll = panel.list_state().logical_scroll_top();
        assert_eq!(scroll.item_ix, 200);
        assert_eq!(scroll.offset_in_item, gpui::px(7.0));
    });
}

#[gpui::test]
fn code_copy_feedback_is_pane_local_and_clears_after_the_confirmation_window(
    cx: &mut gpui::TestAppContext,
) {
    let preview = loaded_document(
        "copy-feedback.org",
        "#+begin_src rust\nfn main() {}\n#+end_src\n",
    )
    .into_preview();
    let panel = cx.new(|_| super::ReadingPreviewPanel::new(std::sync::Arc::new(preview), 80.0));
    let range = ByteRange::new(17, 29);
    panel.update(cx, |panel, cx| {
        panel.show_copy_feedback(range, super::CopyFeedbackState::Succeeded, cx);
        assert_eq!(
            panel.render_state().copy_feedback,
            Some((range, super::CopyFeedbackState::Succeeded))
        );
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(1_601));
    cx.run_until_parked();
    assert_eq!(
        panel.read_with(cx, |panel, _| panel.render_state().copy_feedback),
        None
    );
}

#[gpui::test]
fn independent_preview_actions_can_be_pending_together(cx: &mut gpui::TestAppContext) {
    let preview = loaded_document("actions.org", "- [ ] one\n- [ ] two\n").into_preview();
    let panel = cx.new(|_| super::ReadingPreviewPanel::new(std::sync::Arc::new(preview), 80.0));
    panel.update(cx, |panel, _| {
        let first = super::PreviewActionIdentity::Source(ByteRange::new(2, 5));
        let second = super::PreviewActionIdentity::Source(ByteRange::new(12, 15));
        assert!(panel.begin_action(first).is_some());
        assert!(panel.begin_action(second).is_some());
        assert!(panel.begin_action(first).is_none());
        assert_eq!(panel.render_state().action_states.len(), 2);
    });
}

#[gpui::test]
fn trailing_space_update_does_not_rebind_or_jump_split_reading(cx: &mut gpui::TestAppContext) {
    let source = (0..400)
        .map(|index| format!("* Heading {index}\nbody {index}\n"))
        .collect::<String>();
    let session = crate::document::DocumentSession::from_utf8(
        std::path::PathBuf::from("stable-right-preview.org"),
        source.into_bytes(),
    )
    .unwrap();
    let before = session.snapshot();
    let preview = super::loading::derive_preview(
        std::path::PathBuf::from("stable-right-preview.org"),
        before.clone(),
    );
    let loaded = super::LoadedDocument::new(session, preview).unwrap();
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(true));
    workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(
            0,
            Ok(super::WorkspaceLoadedDocument::Preview(Box::new(loaded))),
            cx,
        ));
    });
    let (session, panel) = cx.read(|cx| {
        let ready = workspace.read(cx).state.ready().unwrap();
        (ready.session.clone(), ready.readers.right.clone().unwrap())
    });
    let expected = gpui::ListOffset {
        item_ix: 200,
        offset_in_item: gpui::px(7.0),
    };
    panel.update(cx, |panel, _| panel.scroll_to(expected));
    let edit = before
        .copy_range(ByteRange::new(0, before.len_bytes()))
        .find("body 100\n")
        .unwrap() as u64
        + "body 100".len() as u64;
    let delta = session.update(cx, |session, cx| {
        session
            .edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        before.revision(),
                        vec![TextEdit::new(ByteRange::new(edit, edit), " ")],
                    ),
                    Selection::caret(ByteOffset(edit)),
                    Selection::caret(ByteOffset(edit + 1)),
                    EditOrigin::Typing,
                ),
                cx,
            )
            .unwrap()
    });
    workspace.update(cx, |workspace, cx| {
        workspace.schedule_derived_update_with_delta(Some(delta), cx)
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(25));
    cx.run_until_parked();

    let actual = cx.read(|cx| panel.read(cx).list_state().logical_scroll_top());
    assert_eq!(actual.item_ix, expected.item_ix);
    assert_eq!(actual.offset_in_item, expected.offset_in_item);
}

#[gpui::test]
fn pane_layout_changes_preserve_editor_state_and_publish_only_latest_revision(
    cx: &mut gpui::TestAppContext,
) {
    let session = crate::document::DocumentSession::from_utf8(
        std::path::PathBuf::from("phase-d.org"),
        b"* Heading\nbody\n".to_vec(),
    )
    .unwrap();
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(
            0,
            Ok(super::WorkspaceLoadedDocument::Source(Box::new(session))),
            cx,
        ));
    });
    let (session, editor) = cx.read(|cx| {
        let ready = workspace.read(cx).state.ready().unwrap();
        (
            ready.session.clone(),
            ready.editors.left.clone().expect("left editor exists"),
        )
    });
    editor.update(cx, |editor, cx| {
        editor.set_selection(Selection::caret(ByteOffset(2)), cx)
    });
    let original = session.read_with(cx, |session, _| session.snapshot());

    workspace.update(cx, |workspace, cx| {
        workspace.show_split(cx);
        assert!(workspace.reading_panel().is_none());
        assert!(
            workspace
                .document_status_snapshot(crate::app::PaneSide::Left, cx)
                .is_some()
        );
        workspace.show_editor(cx);
    });

    workspace.update(cx, |workspace, cx| {
        workspace.show_split(cx);
        workspace.toggle_soft_wrap(cx);
    });
    assert!(!cx.read(|cx| editor.read(cx).soft_wrap()));
    assert_eq!(
        session.read_with(cx, |session, _| session.revision()),
        original.revision()
    );
    assert_eq!(
        cx.read(|cx| editor.read(cx).selection()),
        Selection::caret(ByteOffset(2))
    );

    let revision = session.read_with(cx, |session, _| session.revision());
    let end = original.len_bytes();
    session.update(cx, |session, cx| {
        session
            .edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        revision,
                        vec![TextEdit::new(ByteRange::new(end, end), "latest")],
                    ),
                    Selection::caret(ByteOffset(end)),
                    Selection::caret(ByteOffset(end + 6)),
                    EditOrigin::Typing,
                ),
                cx,
            )
            .unwrap();
    });
    workspace.update(cx, |workspace, cx| workspace.schedule_derived_update(cx));
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(25));
    cx.run_until_parked();

    let latest_revision = session.read_with(cx, |session, _| session.revision());
    let panel_revision = cx.read(|cx| {
        workspace
            .read(cx)
            .reading_panel()
            .unwrap()
            .read(cx)
            .document()
            .revision
    });
    assert_eq!(panel_revision, latest_revision);

    workspace.update(cx, |workspace, cx| {
        assert_eq!(
            workspace.document_workspace.active_surface(),
            crate::app::PaneSurface::Editor
        );
        workspace.activate_pane(crate::app::PaneSide::Right, cx);
        assert_eq!(
            workspace.document_workspace.active_surface(),
            crate::app::PaneSurface::Reading
        );
        assert!(
            !workspace
                .document_status_snapshot(crate::app::PaneSide::Right, cx)
                .unwrap()
                .uses_editor_viewport()
        );
        workspace.show_editor(cx);
        assert_eq!(
            workspace.document_workspace.active_surface(),
            crate::app::PaneSurface::Editor
        );
        assert!(!workspace.document_workspace.is_split());
        assert!(
            workspace
                .document_status_snapshot(crate::app::PaneSide::Right, cx)
                .unwrap()
                .uses_editor_viewport()
        );
    });

    workspace.update(cx, |workspace, cx| {
        workspace.show_split(cx);
        workspace.show_editor(cx);
    });
    assert_eq!(
        session.read_with(cx, |session, _| session.revision()),
        latest_revision
    );
    assert_eq!(
        cx.read(|cx| editor.read(cx).selection()),
        Selection::caret(ByteOffset(2))
    );
}

#[gpui::test]
fn each_split_pane_switches_surface_independently(cx: &mut gpui::TestAppContext) {
    use crate::app::{PaneSide, PaneSurface};

    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    workspace.update(cx, |workspace, cx| {
        workspace.show_split(cx);
        assert!(workspace.document_workspace.is_split());
        assert_eq!(
            workspace.document_workspace.surface(PaneSide::Left),
            PaneSurface::Editor
        );
        assert_eq!(
            workspace.document_workspace.surface(PaneSide::Right),
            PaneSurface::Reading
        );

        workspace.toggle_pane_surface(PaneSide::Left, cx);
        assert_eq!(
            workspace.document_workspace.surface(PaneSide::Left),
            PaneSurface::Reading
        );
        assert_eq!(
            workspace.document_workspace.surface(PaneSide::Right),
            PaneSurface::Reading
        );

        workspace.activate_pane(PaneSide::Right, cx);
        workspace.toggle_pane_surface(PaneSide::Right, cx);
        assert_eq!(
            workspace.document_workspace.surface(PaneSide::Left),
            PaneSurface::Reading
        );
        assert_eq!(
            workspace.document_workspace.surface(PaneSide::Right),
            PaneSurface::Editor
        );
    });
}

#[gpui::test]
fn opening_split_preserves_ime_until_reading_receives_focus(cx: &mut gpui::TestAppContext) {
    let loaded = super::load_document(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/preview-basics.org"),
    )
    .unwrap();
    let window = cx.open_window(gpui::size(gpui::px(900.0), gpui::px(700.0)), |_, _| {
        WorkspaceWindow::with_split_layout(false)
    });
    let editor = window
        .update(cx, |workspace, _, cx| {
            assert!(workspace.apply_load_result(0, Ok(loaded), cx));
            workspace
                .state
                .ready()
                .unwrap()
                .editors
                .left
                .clone()
                .expect("left editor exists")
        })
        .unwrap();
    let root = window.entity(cx).unwrap();
    cx.update(|cx| {
        cx.with_window(root.entity_id(), |window, cx| {
            editor.update(cx, |editor, cx| {
                <crate::editor::SemanticEditor as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                    editor,
                    None,
                    "ni",
                    Some(2..2),
                    window,
                    cx,
                );
            });
        })
        .unwrap();
    });
    assert!(cx.read(|cx| editor.read(cx).has_active_composition()));

    window
        .update(cx, |workspace, _, cx| workspace.show_split(cx))
        .unwrap();
    assert!(cx.read(|cx| editor.read(cx).has_active_composition()));

    window
        .update(cx, |workspace, _, cx| {
            workspace.activate_pane(crate::app::PaneSide::Right, cx)
        })
        .unwrap();
    assert!(!cx.read(|cx| editor.read(cx).has_active_composition()));
}

fn simulate_next_frame<V: gpui::Render + 'static>(
    window: &gpui::WindowHandle<V>,
    cx: &mut gpui::TestAppContext,
) -> usize {
    let root = window.entity(cx).unwrap();
    let callback_count = cx.update(|cx| {
        cx.with_window(root.entity_id(), |window, cx| {
            window.simulate_next_frame(cx)
        })
        .unwrap()
    });
    cx.run_until_parked();
    callback_count
}

fn finish_fold_animation<V: gpui::Render + 'static>(
    window: &gpui::WindowHandle<V>,
    cx: &mut gpui::TestAppContext,
) {
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION + std::time::Duration::from_millis(20));
    assert!(simulate_next_frame(window, cx) > 0);
    assert!(simulate_next_frame(window, cx) > 0);
}

fn ready_document(app: &WorkspaceWindow, cx: &gpui::App) -> std::sync::Arc<super::PreviewSnapshot> {
    match &app.state {
        WorkspaceLoadState::Ready { document } => document
            .readers
            .right
            .as_ref()
            .expect("preview panel should be published")
            .read(cx)
            .document()
            .clone(),
        _ => panic!("document should be ready"),
    }
}

fn apply_load_result(
    app: &mut WorkspaceWindow,
    generation: u64,
    result: Result<super::LoadedDocument, (std::path::PathBuf, String)>,
    cx: &mut gpui::TestAppContext,
) -> bool {
    cx.update(|cx| app.apply_load_result(generation, result, cx))
}

fn cycle_panel_global_visibility(app: &WorkspaceWindow, cx: &mut gpui::App) {
    app.reading_panel()
        .unwrap()
        .update(cx, |panel, _| panel.cycle_global_visibility());
}

fn toggle_panel_fold(
    app: &WorkspaceWindow,
    block_id: crate::org_syntax::BlockId,
    cx: &mut gpui::App,
) {
    app.reading_panel()
        .unwrap()
        .update(cx, |panel, _| panel.toggle_fold(block_id));
}

fn toggle_panel_fold_animated(
    app: &WorkspaceWindow,
    block_id: crate::org_syntax::BlockId,
    viewport_height: f32,
    available_width: f32,
    window: Option<&mut gpui::Window>,
    cx: &mut gpui::App,
) {
    app.reading_panel().unwrap().update(cx, |panel, cx| {
        panel.toggle_fold_animated(
            block_id,
            viewport_height,
            available_width,
            *super::preview_style(super::PreviewStyleId::Base),
            window,
            cx,
        );
    });
}

fn heading_id(
    document: &super::PreviewSnapshot,
    level: u16,
    ordinal: usize,
) -> crate::org_syntax::BlockId {
    document
        .blocks
        .nodes()
        .iter()
        .enumerate()
        .filter(|(_, block)| {
            matches!(block.kind, crate::org_syntax::BlockKind::Heading { level: actual } if actual == level)
        })
        .nth(ordinal)
        .map(|(index, _)| index as crate::org_syntax::BlockId)
        .expect("requested heading should exist")
}

#[test]
fn stale_generations_are_rejected() {
    assert!(accept_generation(7, 7));
    assert!(!accept_generation(8, 7));
}

#[test]
fn shift_tab_dispatches_the_global_visibility_cycle() {
    let (commands, mut keyboard, context) = preview_input();
    let expected = commands.key(GLOBAL_VISIBILITY_CYCLE_COMMAND).unwrap();
    assert_eq!(
        keyboard.route(KeyStroke::new("tab", false, false, true, false), context),
        EmacsOutcome::Command {
            command: expected,
            prefix: crate::command::PrefixArgument::None,
        }
    );
}

#[test]
fn control_c_control_c_dispatches_the_org_context_command() {
    let (commands, mut keyboard, context) = super::document_input();
    assert_eq!(
        keyboard.route(KeyStroke::new("c", true, false, false, false), context),
        EmacsOutcome::Pending
    );
    assert_eq!(
        keyboard.route(KeyStroke::new("c", true, false, false, false), context),
        EmacsOutcome::Command {
            command: commands.key(ORG_CONTEXT_COMMAND).unwrap(),
            prefix: crate::command::PrefixArgument::None,
        }
    );
}

#[gpui::test]
fn control_c_control_c_realigns_the_table_at_point(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = cx.add_window_view(|_, cx| {
        crate::editor::init(cx);
        WorkspaceWindow::with_split_layout(false)
    });
    let session = cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert!(workspace.apply_load_result(
                0,
                Ok(loaded_document(
                    "context-table.org",
                    "| a|long |\n| wider|b|\n",
                )),
                cx,
            ));
            let editor = workspace.editor(crate::app::PaneSide::Left).unwrap();
            editor.update(cx, |editor, cx| {
                editor.set_selection(Selection::caret(ByteOffset(2)), cx);
                editor.request_focus(cx);
            });
            window.focus(&editor.read(cx).focus_handle(cx), cx);
            workspace.document_session().unwrap().clone()
        })
    });

    cx.simulate_keystrokes("ctrl-c ctrl-c");

    cx.read(|cx| {
        let snapshot = session.read(cx).snapshot();
        assert_eq!(
            snapshot.copy_range(
                snapshot
                    .line_content_range(crate::document::LineIndex(0))
                    .unwrap()
            ),
            "| a     | long |"
        );
        assert_eq!(snapshot.revision().0, 1);
    });
}

#[gpui::test]
fn ctrl_x_ctrl_b_with_the_editor_focused_lists_buffers_without_moving_the_cursor(
    cx: &mut gpui::TestAppContext,
) {
    let (workspace, cx) = cx.add_window_view(|_, cx| {
        crate::editor::init(cx);
        WorkspaceWindow::with_split_layout(false)
    });
    let session = cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert!(workspace.apply_load_result(
                0,
                Ok(loaded_document("probe.md", "| a | value |\n")),
                cx,
            ));
            let editor = workspace.editor(crate::app::PaneSide::Left).unwrap();
            editor.update(cx, |editor, cx| {
                editor.set_selection(Selection::caret(ByteOffset(2)), cx);
                editor.request_focus(cx);
            });
            window.focus(&editor.read(cx).focus_handle(cx), cx);
            workspace.document_session().unwrap().clone()
        })
    });

    cx.simulate_keystrokes("ctrl-x ctrl-b");

    cx.read(|cx| {
        let snapshot = session.read(cx).snapshot();
        assert_eq!(
            snapshot.copy_range(crate::document::ByteRange::new(0, snapshot.len_bytes())),
            "| a | value |\n",
            "the completing Ctrl-B must not edit or move through the document"
        );
    });
    workspace.update(cx, |workspace, cx| {
        assert!(
            matches!(
                workspace.buffers.panel,
                Some(crate::app::buffers::Panel::Picker(_))
            ),
            "Ctrl-X Ctrl-B should open the buffer list, not the line-editing binding"
        );
        assert_eq!(workspace.document_session(), Some(&session));
        assert_eq!(
            workspace
                .editor(crate::app::PaneSide::Left)
                .unwrap()
                .read(cx)
                .selection(),
            Selection::caret(ByteOffset(2))
        );
    });
}

#[gpui::test]
fn escape_cancels_a_pending_prefix_and_restores_editor_focus(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = cx.add_window_view(|_, cx| {
        crate::editor::init(cx);
        WorkspaceWindow::with_split_layout(false)
    });
    let session = cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert!(workspace.apply_load_result(
                0,
                Ok(loaded_document("probe.md", "| a | value |\n")),
                cx,
            ));
            let editor = workspace.editor(crate::app::PaneSide::Left).unwrap();
            editor.update(cx, |editor, cx| {
                editor.set_selection(Selection::caret(ByteOffset(2)), cx);
                editor.request_focus(cx);
            });
            window.focus(&editor.read(cx).focus_handle(cx), cx);
            workspace.document_session().unwrap().clone()
        })
    });

    cx.simulate_keystrokes("ctrl-x escape");

    cx.read(|cx| {
        let snapshot = session.read(cx).snapshot();
        assert_eq!(
            snapshot.copy_range(crate::document::ByteRange::new(0, snapshot.len_bytes())),
            "| a | value |\n",
            "Escape must cancel the prefix without editing the document"
        );
    });
    workspace.update(cx, |workspace, _| {
        assert!(
            matches!(workspace.state, WorkspaceLoadState::Ready { .. }),
            "Escape must not navigate away from the document"
        );
    });
    cx.update(|window, cx| {
        let editor = workspace
            .read(cx)
            .editor(crate::app::PaneSide::Left)
            .unwrap();
        let editor_handle = editor.update(cx, |editor, cx| editor.focus_handle(cx));
        assert_eq!(
            window.focused(cx),
            Some(editor_handle),
            "focus must return to the editor after Escape cancels the prefix"
        );
    });
}

#[gpui::test]
fn keys_while_a_prefix_is_pending_never_edit_the_document(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = cx.add_window_view(|_, cx| {
        crate::editor::init(cx);
        WorkspaceWindow::with_split_layout(false)
    });
    let session = cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert!(workspace.apply_load_result(
                0,
                Ok(loaded_document("probe.md", "| a | value |\n")),
                cx,
            ));
            let editor = workspace.editor(crate::app::PaneSide::Left).unwrap();
            editor.update(cx, |editor, cx| {
                editor.set_selection(Selection::caret(ByteOffset(2)), cx);
                editor.request_focus(cx);
            });
            window.focus(&editor.read(cx).focus_handle(cx), cx);
            workspace.document_session().unwrap().clone()
        })
    });

    cx.simulate_keystrokes("ctrl-x a");

    cx.read(|cx| {
        let snapshot = session.read(cx).snapshot();
        assert_eq!(
            snapshot.copy_range(crate::document::ByteRange::new(0, snapshot.len_bytes())),
            "| a | value |\n",
            "an unresolved key while the prefix is pending must not type into the document"
        );
        assert_eq!(snapshot.revision().0, 0);
    });
}

#[gpui::test]
fn editor_ctrl_b_still_moves_the_cursor_when_no_prefix_is_pending(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = cx.add_window_view(|_, cx| {
        crate::editor::init(cx);
        WorkspaceWindow::with_split_layout(false)
    });
    let session = cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert!(workspace.apply_load_result(
                0,
                Ok(loaded_document("probe.md", "| a | value |\n")),
                cx,
            ));
            let editor = workspace.editor(crate::app::PaneSide::Left).unwrap();
            editor.update(cx, |editor, cx| {
                editor.set_selection(Selection::caret(ByteOffset(2)), cx);
                editor.request_focus(cx);
            });
            window.focus(&editor.read(cx).focus_handle(cx), cx);
            workspace.document_session().unwrap().clone()
        })
    });

    cx.simulate_keystrokes("ctrl-b");

    cx.read(|cx| {
        let snapshot = session.read(cx).snapshot();
        assert_eq!(
            snapshot.copy_range(crate::document::ByteRange::new(0, snapshot.len_bytes())),
            "| a | value |\n"
        );
    });
    workspace.update(cx, |workspace, cx| {
        let editor = workspace.editor(crate::app::PaneSide::Left).unwrap();
        editor.update(cx, |editor, _| {
            assert_ne!(
                editor.selection().head(),
                ByteOffset(2),
                "Ctrl-B alone must keep moving the cursor (line editing stays intact)"
            );
        });
    });
}

#[test]
fn org_inline_image_preview_uses_the_official_key_sequence() {
    let (commands, mut keyboard, context) = super::document_input();
    for key in ["c", "x"] {
        assert_eq!(
            keyboard.route(KeyStroke::new(key, true, false, false, false), context),
            EmacsOutcome::Pending
        );
    }
    assert_eq!(
        keyboard.route(KeyStroke::new("v", true, false, false, false), context),
        EmacsOutcome::Command {
            command: commands.key(TOGGLE_INLINE_IMAGE_PREVIEWS_COMMAND).unwrap(),
            prefix: crate::command::PrefixArgument::None,
        }
    );
}

#[test]
fn reading_keymap_routes_platform_undo_and_redo_to_document_history() {
    let (commands, mut keyboard, context) = preview_input();
    for (stroke, name) in [
        (
            KeyStroke::new("z", false, false, false, true),
            UNDO_DOCUMENT_COMMAND,
        ),
        (
            KeyStroke::new("z", false, false, true, true),
            REDO_DOCUMENT_COMMAND,
        ),
    ] {
        assert_eq!(
            keyboard.route(stroke, context),
            EmacsOutcome::Command {
                command: commands.key(name).unwrap(),
                prefix: crate::command::PrefixArgument::None,
            }
        );
    }
}

#[test]
fn sidebar_keymap_activates_both_sidebar_and_dired_contexts() {
    let mut app = WorkspaceWindow::with_split_layout(true);
    app.install_sidebar_keymap();
    let contexts = super::built_in_contexts();
    assert!(app.key_context.contains(contexts.key("workspace").unwrap()));
    assert!(app.key_context.contains(contexts.key("sidebar").unwrap()));
    assert!(app.key_context.contains(contexts.key("dired").unwrap()));
    assert!(!app.key_context.contains(contexts.key("preview").unwrap()));
}

#[test]
fn source_keymap_passes_text_keys_to_the_editor() {
    let mut app = WorkspaceWindow::with_split_layout(false);
    for key in ["g", "q", "space", "backspace"] {
        assert!(
            matches!(
                app.keyboard.route(
                    KeyStroke::new(key, false, false, false, false),
                    app.key_context,
                ),
                EmacsOutcome::PassThrough | EmacsOutcome::Undefined
            ),
            "{key} must remain available to the editor"
        );
    }
    assert!(matches!(
        app.keyboard.route(
            KeyStroke::new("tab", false, false, true, false),
            app.key_context,
        ),
        EmacsOutcome::PassThrough | EmacsOutcome::Undefined
    ));
    let contexts = super::built_in_contexts();
    assert!(app.key_context.contains(contexts.key("editor").unwrap()));
    assert!(!app.key_context.contains(contexts.key("preview").unwrap()));
}

#[gpui::test]
fn editor_only_workspace_load_does_not_build_a_hidden_preview(cx: &mut gpui::TestAppContext) {
    let path =
        std::env::temp_dir().join(format!("org-studio-source-load-{}.org", std::process::id()));
    std::fs::write(&path, "* Heading\nbody\n").unwrap();
    let loaded = super::load_workspace_document(path.clone(), false).unwrap();
    let _ = std::fs::remove_file(&path);
    let mut app = WorkspaceWindow::with_split_layout(false);
    app.generation = 1;
    assert!(cx.update(|cx| app.apply_load_result(1, Ok(loaded), cx)));
    assert!(app.document_session().is_some());
    assert!(app.reading_panel().is_none());
}

#[gpui::test]
fn opening_preview_while_a_source_load_finishes_schedules_the_new_document(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::path::PathBuf::from("open-during-load.org");
    let session = crate::document::DocumentSession::from_utf8(
        path.clone(),
        b"* Current document\nbody\n".to_vec(),
    )
    .unwrap();
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    workspace.update(cx, |workspace, cx| {
        let generation = workspace.begin_open(path, std::time::Instant::now());
        workspace.show_split(cx);
        assert!(workspace.apply_load_result(
            generation,
            Ok(super::WorkspaceLoadedDocument::Source(Box::new(session))),
            cx,
        ));
        workspace.reconcile_derived_preview(cx);
    });

    cx.executor()
        .advance_clock(std::time::Duration::from_millis(25));
    cx.run_until_parked();

    cx.read(|cx| {
        let workspace = workspace.read(cx);
        let ready = workspace.state.ready().unwrap();
        let panel = ready
            .readers
            .right
            .as_ref()
            .expect("right preview should be published");
        assert_eq!(
            panel.read(cx).document().revision,
            ready.session.read(cx).revision()
        );
    });
}

#[gpui::test]
fn hiding_reading_preserves_its_view_state_but_rejects_a_new_hidden_projection(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-close-preview-{}.org",
        std::process::id()
    ));
    std::fs::write(&path, "* Heading\nbody\n").unwrap();
    let loaded = super::load_document(path.clone()).unwrap();
    let late_loaded = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(true));

    workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(0, Ok(loaded), cx));
        assert!(workspace.reading_panel().is_some());
        workspace.show_editor(cx);
        assert!(workspace.reading_panel().is_none());
        assert!(workspace.derived.latest.is_some());
        assert!(
            workspace
                .derived
                .pending
                .lock()
                .expect("derived request slot")
                .is_none()
        );

        workspace.generation = 1;
        assert!(workspace.apply_load_result(1, Ok(late_loaded), cx));
        assert!(workspace.reading_panel().is_none());
        assert!(workspace.derived.latest.is_none());
    });
}

#[gpui::test]
fn lagging_reading_keeps_preview_status_even_when_the_pane_retains_an_editor(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-lagging-preview-status-{}.org",
        std::process::id()
    ));
    std::fs::write(&path, "* Heading\nbody\n").unwrap();
    let loaded = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(true));
    let session = workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(0, Ok(loaded), cx));
        workspace.activate_pane(crate::app::PaneSide::Right, cx);
        workspace.toggle_pane_surface(crate::app::PaneSide::Right, cx);
        workspace.toggle_pane_surface(crate::app::PaneSide::Right, cx);
        assert!(workspace.state.ready().unwrap().editors.right.is_some());
        workspace.document_session().unwrap().clone()
    });
    session.update(cx, |session, cx| {
        let end = session.snapshot().len_bytes();
        session
            .edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        session.revision(),
                        vec![TextEdit::new(ByteRange::new(end, end), "new")],
                    ),
                    Selection::caret(ByteOffset(end)),
                    Selection::caret(ByteOffset(end + 3)),
                    EditOrigin::Typing,
                ),
                cx,
            )
            .unwrap();
    });

    cx.read(|cx| {
        let status = workspace
            .read(cx)
            .document_status_snapshot(crate::app::PaneSide::Right, cx)
            .unwrap();
        assert!(!status.uses_editor_viewport());
        assert!(status.dirty);
        assert_eq!(status.transient_text(), None);
    });
}

#[gpui::test]
fn export_source_uses_the_live_edited_session(cx: &mut gpui::TestAppContext) {
    let path =
        std::env::temp_dir().join(format!("org-studio-live-export-{}.org", std::process::id()));
    std::fs::write(&path, "old").unwrap();
    let loaded = super::load_workspace_document(path.clone(), false).unwrap();
    let _ = std::fs::remove_file(&path);
    let mut app = WorkspaceWindow::with_split_layout(false);
    app.generation = 1;
    assert!(cx.update(|cx| app.apply_load_result(1, Ok(loaded), cx)));
    let session = app.document_session().unwrap().clone();
    cx.update(|cx| {
        session.update(cx, |session, cx| {
            session
                .edit(
                    DocumentCommand::new(
                        EditTransaction::new(
                            session.revision(),
                            vec![TextEdit::new(ByteRange::new(0, 3), "new")],
                        ),
                        Selection::new(ByteOffset(0), ByteOffset(3)),
                        Selection::caret(ByteOffset(3)),
                        EditOrigin::Other,
                    ),
                    cx,
                )
                .unwrap();
        });
    });
    let (snapshot, export_path, format) = cx.update(|cx| app.current_export_source(cx).unwrap());
    assert_eq!(snapshot.copy_range(ByteRange::new(0, 3)), "new");
    assert_eq!(export_path, path);
    assert_eq!(format, super::DocumentFormat::Org);
}

#[gpui::test]
fn unchanged_viewport_does_not_cancel_an_active_sidebar_resize(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-sidebar-resize-{}.org",
        std::process::id()
    ));
    std::fs::write(&path, "* Heading\nbody\n").unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let window = cx.open_window(gpui::size(gpui::px(900.0), gpui::px(700.0)), |_, _| {
        WorkspaceWindow::with_split_layout(true)
    });
    window
        .update(cx, |app, _, cx| {
            app.generation = 1;
            assert!(app.apply_load_result(1, Ok(document), cx));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |app, _, cx| {
            app.begin_sidebar_resize(240.0, 900.0, cx);
            assert!(app.file_manager.is_resizing_sidebar());
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |app, _, _| {
            assert!(app.file_manager.is_resizing_sidebar());
        })
        .unwrap();
}

#[gpui::test]
fn app_global_visibility_cycle_updates_list_fold_and_minimap_projection_together(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-global-visibility-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "preamble\n* One\nbody\n** Child\nchild body\n* Two\nvisible\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let app = cx.new(|_| WorkspaceWindow::with_split_layout(true));
    app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document), cx));
        cycle_panel_global_visibility(app, cx);
        assert_eq!(
            app.reading_panel().unwrap().read(cx).global_visibility(),
            GlobalVisibility::Overview
        );
        assert_eq!(
            app.reading_panel().unwrap().read(cx).visible_rows().len(),
            2
        );
        assert_eq!(
            app.reading_panel().unwrap().read(cx).fold_markers().len(),
            2
        );
        assert_eq!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .item_count(),
            app.reading_panel().unwrap().read(cx).visible_rows().len()
        );
        cycle_panel_global_visibility(app, cx);
        assert_eq!(
            app.reading_panel().unwrap().read(cx).global_visibility(),
            GlobalVisibility::Contents
        );
        assert_eq!(
            app.reading_panel().unwrap().read(cx).visible_rows().len(),
            3
        );
        assert_eq!(
            app.reading_panel().unwrap().read(cx).fold_markers().len(),
            2
        );
        assert_eq!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .item_count(),
            app.reading_panel().unwrap().read(cx).visible_rows().len()
        );
        cycle_panel_global_visibility(app, cx);
        assert_eq!(
            app.reading_panel().unwrap().read(cx).global_visibility(),
            GlobalVisibility::All
        );
        assert!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .is_empty()
        );
        assert_eq!(
            app.reading_panel().unwrap().read(cx).visible_rows().len(),
            7
        );
        assert_eq!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .item_count(),
            app.reading_panel().unwrap().read(cx).visible_rows().len()
        );
    });
}

#[gpui::test]
fn shift_tab_animates_all_three_global_visibility_transitions(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-animated-global-visibility-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "preamble\n* One\nbody\n** Child\nchild body\n* Two\nvisible\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let window = cx.open_window(gpui::size(gpui::px(900.0), gpui::px(700.0)), |_, _| {
        WorkspaceWindow::with_split_layout(true)
    });
    window
        .update(cx, |app, _, cx| {
            app.generation = 1;
            assert!(app.apply_load_result(1, Ok(document), cx));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    for (visibility, direction) in [
        (GlobalVisibility::Overview, super::FoldDirection::Collapse),
        (GlobalVisibility::Contents, super::FoldDirection::Expand),
        (GlobalVisibility::All, super::FoldDirection::Expand),
    ] {
        window
            .update(cx, |app, window, cx| {
                app.cycle_global_visibility_animated(window, cx);
                assert_eq!(
                    app.reading_panel().unwrap().read(cx).global_visibility(),
                    visibility
                );
                let animation = app
                    .reading_panel()
                    .unwrap()
                    .read(cx)
                    .fold_animation()
                    .expect("every Shift-Tab state should animate");
                assert_eq!(animation.direction, direction);
                if visibility == GlobalVisibility::Overview {
                    assert!(animation.segments.len() > 1);
                }
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        assert!(simulate_next_frame(&window, cx) > 0);
        finish_fold_animation(&window, cx);
        window
            .update(cx, |app, _, cx| {
                assert!(
                    app.reading_panel()
                        .unwrap()
                        .read(cx)
                        .fold_animation()
                        .is_none()
                );
                assert_eq!(
                    app.reading_panel()
                        .unwrap()
                        .read(cx)
                        .list_state()
                        .item_count(),
                    app.reading_panel().unwrap().read(cx).visible_rows().len()
                );
            })
            .unwrap();
    }
}

#[gpui::test]
fn expanding_a_child_from_contents_does_not_collapse_its_parent(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-contents-child-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "preamble\n* One\nparent body\n** Child\nchild body\n* Two\nsecond body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let app = cx.new(|_| WorkspaceWindow::with_split_layout(true));
    app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document), cx));
        cycle_panel_global_visibility(app, cx);
        cycle_panel_global_visibility(app, cx);

        let document = ready_document(app, cx);
        let parent = heading_id(&document, 1, 0);
        let child = heading_id(&document, 2, 0);

        assert!(
            !app.reading_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&parent)
        );
        assert!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&child)
        );

        toggle_panel_fold(app, child, cx);
        assert_eq!(
            app.reading_panel().unwrap().read(cx).global_visibility(),
            GlobalVisibility::Contents
        );
        assert!(
            !app.reading_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&parent)
        );
        assert!(
            !app.reading_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&child)
        );
        assert_eq!(visible_source_lines(app, &document, cx), vec![2, 4, 5, 6]);

        toggle_panel_fold(app, child, cx);
        assert_eq!(
            app.reading_panel().unwrap().read(cx).global_visibility(),
            GlobalVisibility::Contents
        );
        assert_eq!(
            app.reading_panel().unwrap().read(cx).visible_rows().len(),
            3
        );
        assert!(
            !app.reading_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&parent)
        );
        assert!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&child)
        );

        cycle_panel_global_visibility(app, cx);
        assert_eq!(
            app.reading_panel().unwrap().read(cx).global_visibility(),
            GlobalVisibility::Overview
        );
        assert_eq!(
            app.reading_panel().unwrap().read(cx).visible_rows().len(),
            2
        );
    });
}

#[gpui::test]
fn clicking_a_second_level_heading_in_contents_never_folds_its_parent(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-contents-second-level-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "* Parent\nparent body\n** Child\nchild body\n*** Grandchild\ngrandchild body\n* Other\nother body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let app = cx.new(|_| WorkspaceWindow::with_split_layout(true));
    app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document), cx));
        cycle_panel_global_visibility(app, cx);
        cycle_panel_global_visibility(app, cx);

        let document = ready_document(app, cx);
        let parent = heading_id(&document, 1, 0);
        let child = heading_id(&document, 2, 0);

        assert_eq!(visible_source_lines(app, &document, cx), vec![1, 3, 5, 7]);
        assert!(
            !app.reading_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&parent)
        );

        toggle_panel_fold(app, child, cx);
        assert_eq!(visible_source_lines(app, &document, cx), vec![1, 3, 7]);
        assert!(
            !app.reading_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&parent)
        );
        assert!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&child)
        );

        toggle_panel_fold(app, child, cx);
        assert_eq!(
            visible_source_lines(app, &document, cx),
            vec![1, 3, 4, 5, 7]
        );
        assert!(
            !app.reading_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&parent)
        );
    });
}

#[gpui::test]
fn heading_click_cycles_only_its_subtree_through_official_local_states(
    cx: &mut gpui::TestAppContext,
) {
    let path =
        std::env::temp_dir().join(format!("org-studio-local-cycle-{}.org", std::process::id()));
    std::fs::write(
        &path,
        "* Parent\nparent body\n** Child\nchild body\n** Sibling\nsibling body\n* Other\nother body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let app = cx.new(|_| WorkspaceWindow::with_split_layout(true));
    app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document), cx));
        cycle_panel_global_visibility(app, cx);
        let document = ready_document(app, cx);
        let parent = heading_id(&document, 1, 0);

        toggle_panel_fold(app, parent, cx);
        assert_eq!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .local_cycle_continuation(),
            Some((parent, super::LocalVisibility::Children))
        );
        assert_eq!(
            visible_source_lines(app, &document, cx),
            vec![1, 2, 3, 5, 7]
        );

        toggle_panel_fold(app, parent, cx);
        assert_eq!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .local_cycle_continuation(),
            Some((parent, super::LocalVisibility::Subtree))
        );
        assert_eq!(
            visible_source_lines(app, &document, cx),
            vec![1, 2, 3, 4, 5, 6, 7]
        );

        toggle_panel_fold(app, parent, cx);
        assert_eq!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .local_cycle_continuation(),
            Some((parent, super::LocalVisibility::Folded))
        );
        assert_eq!(visible_source_lines(app, &document, cx), vec![1, 7]);
    });
}

#[gpui::test]
fn animated_collapse_inserts_one_flow_shell_and_fast_reclick_finishes_it(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-animated-local-cycle-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "* Parent\nparent body\n** Child\nchild body\n* Other\nother body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let app = cx.new(|_| WorkspaceWindow::with_split_layout(true));

    app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document), cx));
        let document = ready_document(app, cx);
        let parent = heading_id(&document, 1, 0);

        toggle_panel_fold_animated(app, parent, 700.0, 790.0, None, cx);
        assert_eq!(visible_source_lines(app, &document, cx), vec![1, 5, 6]);
        assert_eq!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .map(|animation| animation.segments[0].transition_index),
            Some(1)
        );
        assert_eq!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .item_count(),
            app.reading_panel().unwrap().read(cx).visible_rows().len() + 1
        );

        toggle_panel_fold_animated(app, parent, 700.0, 790.0, None, cx);
        assert_eq!(
            visible_source_lines(app, &document, cx),
            vec![1, 2, 3, 5, 6]
        );
        let expansion = app
            .reading_panel()
            .unwrap()
            .read(cx)
            .fold_animation()
            .unwrap();
        assert!(matches!(expansion.direction, super::FoldDirection::Expand));
        assert_eq!(expansion.segments[0].target_len, 2);
        assert_eq!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .item_count(),
            app.reading_panel().unwrap().read(cx).visible_rows().len() - 1
        );
        app.reading_panel()
            .unwrap()
            .update(cx, |panel, _| panel.discard_fold_animation());
    });

    app.update(cx, |app, cx| {
        assert!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .is_none()
        );
        assert_eq!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .item_count(),
            app.reading_panel().unwrap().read(cx).visible_rows().len()
        );
    });
}

#[gpui::test]
fn reduced_motion_applies_local_fold_without_a_delayed_projection(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let path = std::env::temp_dir().join(format!(
        "org-studio-reduced-motion-fold-{}.org",
        std::process::id()
    ));
    std::fs::write(&path, "* Parent\nbody\n** Child\nchild body\n").unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let app = cx.new(|_| WorkspaceWindow::with_split_layout(true));

    app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document), cx));
        let document = ready_document(app, cx);
        let parent = heading_id(&document, 1, 0);

        toggle_panel_fold_animated(app, parent, 700.0, 790.0, None, cx);
        assert_eq!(visible_source_lines(app, &document, cx), vec![1]);
        assert!(
            app.reading_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .is_none()
        );
    });
}

#[gpui::test]
fn measured_fold_travel_renders_through_real_list_animation_frames(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-rendered-fold-travel-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "* Parent\nparent body\n** Child One\nchild one body\n** Child Two\nchild two body\n* Other\nother body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let window = cx.open_window(gpui::size(gpui::px(900.0), gpui::px(700.0)), |_, _| {
        WorkspaceWindow::with_split_layout(true)
    });
    window
        .update(cx, |app, _window, cx| {
            app.generation = 1;
            assert!(app.apply_load_result(1, Ok(document), cx));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |app, window, cx| {
            let document = ready_document(app, cx);
            let parent = heading_id(&document, 1, 0);
            let first_hidden = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(1)
                .unwrap();
            let following = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(6)
                .unwrap();
            let expected_distance = f32::from(following.top() - first_hidden.top());

            toggle_panel_fold_animated(app, parent, 700.0, 790.0, Some(window), cx);
            let animation = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            let shell = &animation.segments[0];
            assert_eq!(shell.transition_index, 1);
            assert!((shell.distance - expected_distance).abs() < 0.01);
            assert_eq!(
                app.reading_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .item_count(),
                app.reading_panel().unwrap().read(cx).visible_rows().len() + 1
            );
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    // Even if preparing the first Shell frame exceeds the whole nominal duration, animation
    // time starts only after that frame is presented.
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION + std::time::Duration::from_millis(20));
    assert!(simulate_next_frame(&window, cx) > 0);
    let first_gap = window
        .update(cx, |app, _, cx| {
            let animation = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            assert_eq!(animation.progress, 0.0);
            assert!(animation.started_at.is_some());
            let heading = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap();
            let peer = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(2)
                .unwrap();
            f32::from(peer.top() - heading.bottom())
        })
        .unwrap();
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION / 3);
    assert!(simulate_next_frame(&window, cx) > 0);
    let later_gap = window
        .update(cx, |app, _, cx| {
            let animation = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            assert!(animation.progress > 0.0);
            let heading = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap();
            let shell = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(1)
                .unwrap();
            let peer = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(2)
                .unwrap();
            assert!((f32::from(shell.top() - heading.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - shell.bottom())).abs() < 0.01);
            let gap = f32::from(peer.top() - heading.bottom());
            let eased = crate::motion::FOLD_MOTION.ease(animation.progress);
            let expected_gap = animation.segments[0].distance * (1.0 - eased);
            assert!(
                (gap - expected_gap).abs() < 1.0,
                "gap={gap} expected_gap={expected_gap} progress={}",
                animation.progress
            );
            gap
        })
        .unwrap();
    assert!(later_gap < first_gap);

    finish_fold_animation(&window, cx);
    window
        .update(cx, |app, _, cx| {
            assert!(
                app.reading_panel()
                    .unwrap()
                    .read(cx)
                    .fold_animation()
                    .is_none()
            );
            assert_eq!(
                app.reading_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .item_count(),
                app.reading_panel().unwrap().read(cx).visible_rows().len()
            );
        })
        .unwrap();

    window
        .update(cx, |app, window, cx| {
            let document = ready_document(app, cx);
            let parent = heading_id(&document, 1, 0);
            toggle_panel_fold_animated(app, parent, 700.0, 790.0, Some(window), cx);
            let animation = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            assert!(matches!(animation.direction, super::FoldDirection::Expand));
            assert_eq!(animation.segments[0].transition_index, 1);
            assert_eq!(animation.segments[0].target_len, 3);
            assert_eq!(
                app.reading_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .item_count(),
                app.reading_panel().unwrap().read(cx).visible_rows().len() - 2
            );
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    assert!(simulate_next_frame(&window, cx) > 0);
    let first_gap = window
        .update(cx, |app, _, cx| {
            let animation = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            assert_eq!(animation.progress, 0.0);
            let heading = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap();
            let shell = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(1)
                .unwrap();
            let peer = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(2)
                .unwrap();
            assert!((f32::from(shell.top() - heading.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - shell.bottom())).abs() < 0.01);
            f32::from(peer.top() - heading.bottom())
        })
        .unwrap();
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION / 3);
    assert!(simulate_next_frame(&window, cx) > 0);
    let later_gap = window
        .update(cx, |app, _, cx| {
            let animation = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            assert!(animation.progress > 0.0);
            let heading = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap();
            let shell = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(1)
                .unwrap();
            let peer = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(2)
                .unwrap();
            assert!((f32::from(shell.top() - heading.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - shell.bottom())).abs() < 0.01);
            let gap = f32::from(peer.top() - heading.bottom());
            let eased = crate::motion::FOLD_MOTION.ease(animation.progress);
            let expected_gap = animation.segments[0].distance * eased;
            assert!(
                (gap - expected_gap).abs() < 1.0,
                "gap={gap} expected_gap={expected_gap} progress={}",
                animation.progress
            );
            gap
        })
        .unwrap();
    assert!(later_gap > first_gap);

    finish_fold_animation(&window, cx);
    window
        .update(cx, |app, _, cx| {
            assert!(
                app.reading_panel()
                    .unwrap()
                    .read(cx)
                    .fold_animation()
                    .is_none()
            );
            assert_eq!(
                app.reading_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .item_count(),
                app.reading_panel().unwrap().read(cx).visible_rows().len()
            );
            let heading = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap();
            let peer = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(4)
                .unwrap();
            assert!(peer.top() > heading.bottom());
        })
        .unwrap();

    window
        .update(cx, |app, window, cx| {
            let document = ready_document(app, cx);
            let parent = heading_id(&document, 1, 0);
            toggle_panel_fold_animated(app, parent, 700.0, 790.0, Some(window), cx);
            let animation = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            assert!(matches!(animation.direction, super::FoldDirection::Expand));
            assert_eq!(animation.segments.len(), 2);
            assert_eq!(animation.segments[0].transition_index, 3);
            assert_eq!(animation.segments[1].transition_index, 5);
            assert!(animation.segments.iter().all(|shell| shell.target_len == 1));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    assert!(simulate_next_frame(&window, cx) > 0);
    window
        .update(cx, |app, _, cx| {
            let first_shell = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(3)
                .unwrap();
            let second_shell = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(5)
                .unwrap();
            assert!(f32::from(first_shell.size.height).abs() < 0.01);
            assert!(f32::from(second_shell.size.height).abs() < 0.01);
        })
        .unwrap();
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION / 3);
    assert!(simulate_next_frame(&window, cx) > 0);
    window
        .update(cx, |app, _, cx| {
            let animation = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            assert!(animation.progress > 0.0);
            let first_child = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(2)
                .unwrap();
            let first_shell = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(3)
                .unwrap();
            let second_child = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(4)
                .unwrap();
            let second_shell = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(5)
                .unwrap();
            let peer = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(6)
                .unwrap();
            assert!((f32::from(first_shell.top() - first_child.bottom())).abs() < 0.01);
            assert!((f32::from(second_child.top() - first_shell.bottom())).abs() < 0.01);
            assert!((f32::from(second_shell.top() - second_child.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - second_shell.bottom())).abs() < 0.01);
            for (index, shell) in animation.segments.iter().enumerate() {
                let bounds = app
                    .reading_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .bounds_for_item(shell.transition_index)
                    .unwrap();
                let eased = crate::motion::FOLD_MOTION.ease(animation.progress);
                let expected = shell.distance * eased;
                assert!(
                    (f32::from(bounds.size.height) - expected).abs() < 1.0,
                    "shell {index} height did not share the expansion progress"
                );
            }
        })
        .unwrap();
    finish_fold_animation(&window, cx);
    window
        .update(cx, |app, _, cx| {
            assert!(
                app.reading_panel()
                    .unwrap()
                    .read(cx)
                    .fold_animation()
                    .is_none()
            );
            assert_eq!(
                app.reading_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .item_count(),
                app.reading_panel().unwrap().read(cx).visible_rows().len()
            );
            let first_child_body = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(3)
                .unwrap();
            let second_child = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(4)
                .unwrap();
            let second_child_body = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(5)
                .unwrap();
            let peer = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(6)
                .unwrap();
            assert!(first_child_body.size.height > gpui::px(0.0));
            assert!(second_child.top() >= first_child_body.bottom());
            assert!(second_child_body.size.height > gpui::px(0.0));
            assert!(peer.top() >= second_child_body.bottom());
        })
        .unwrap();
}

#[gpui::test]
fn collapse_keeps_an_offscreen_peer_behind_a_bounded_flow_shell(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-offscreen-fold-peer-{}.org",
        std::process::id()
    ));
    let mut source = String::from("* Parent\n");
    for index in 0..48 {
        source.push_str(&format!("** Child {index}\nbody {index}\n"));
    }
    source.push_str("* Other\nother body\n");
    std::fs::write(&path, source).unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let window = cx.open_window(gpui::size(gpui::px(900.0), gpui::px(320.0)), |_, _| {
        WorkspaceWindow::with_split_layout(true)
    });
    window
        .update(cx, |app, _window, cx| {
            app.generation = 1;
            assert!(app.apply_load_result(1, Ok(document), cx));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |app, window, cx| {
            let document = ready_document(app, cx);
            let parent = heading_id(&document, 1, 0);
            let peer = heading_id(&document, 1, 1);
            let peer_row = document
                .projection
                .rows
                .iter()
                .position(|row| row.block_id == peer)
                .unwrap();
            let old_peer_position = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .visible_rows()
                .binary_search(&peer_row)
                .unwrap();
            assert!(
                app.reading_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .bounds_for_item(old_peer_position)
                    .unwrap()
                    .top()
                    > gpui::px(320.0)
            );

            toggle_panel_fold_animated(app, parent, 320.0, 790.0, Some(window), cx);
            let animation = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            let shell = &animation.segments[0];
            assert_eq!(shell.transition_index, 1);
            assert!(shell.distance <= 320.0);
            assert!(shell.rendered_rows.len() < 20);
            assert_eq!(
                app.reading_panel().unwrap().read(cx).visible_rows()[1],
                peer_row
            );
            assert_eq!(
                app.reading_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .item_count(),
                app.reading_panel().unwrap().read(cx).visible_rows().len() + 1
            );
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    assert!(simulate_next_frame(&window, cx) > 0);

    window
        .update(cx, |app, _, cx| {
            let heading = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap();
            let shell = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(1)
                .unwrap();
            let peer = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(2)
                .unwrap();
            assert!((f32::from(shell.top() - heading.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - shell.bottom())).abs() < 0.01);
            assert!(f32::from(peer.top() - heading.bottom()) > 24.0);
            assert!(
                app.reading_panel()
                    .unwrap()
                    .read(cx)
                    .fold_animation()
                    .is_some()
            );
        })
        .unwrap();
    finish_fold_animation(&window, cx);
    window
        .update(cx, |app, _, cx| {
            let heading = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap();
            let peer = app
                .reading_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(1)
                .unwrap();
            assert!((f32::from(peer.top() - heading.bottom())).abs() < 0.01);
            assert_eq!(
                app.reading_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .item_count(),
                app.reading_panel().unwrap().read(cx).visible_rows().len()
            );
            assert!(
                app.reading_panel()
                    .unwrap()
                    .read(cx)
                    .fold_animation()
                    .is_none()
            );
        })
        .unwrap();
}

#[gpui::test]
fn open_failure_retains_previous_session_and_rejects_stale_completion(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-reload-state-{}.org",
        std::process::id()
    ));
    std::fs::write(&path, "* retained\n").unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(&path);

    let mut app = WorkspaceWindow::with_split_layout(true);
    app.generation = 1;
    assert!(apply_load_result(&mut app, 1, Ok(document), cx));
    assert!(app.state.ready().is_some());
    let session = app.document_session().unwrap().clone();

    app.generation = 2;
    assert!(apply_load_result(
        &mut app,
        2,
        Err((path.clone(), "reload failed".to_owned())),
        cx,
    ));
    assert!(matches!(app.state, WorkspaceLoadState::Failed { .. }));
    assert!(app.state.ready().is_some());
    assert_eq!(app.document_session(), Some(&session));

    let stale_path = path.with_extension("md");
    std::fs::write(&stale_path, "# stale\n").unwrap();
    let stale = super::load_document(stale_path.clone()).unwrap();
    let _ = std::fs::remove_file(stale_path);
    assert!(!apply_load_result(&mut app, 1, Ok(stale), cx));
    assert!(matches!(app.state, WorkspaceLoadState::Failed { .. }));
    assert!(app.state.ready().is_some());
    assert_eq!(app.document_session(), Some(&session));
}

#[gpui::test]
fn reload_keeps_session_identity_and_publishes_a_coherent_preview(cx: &mut gpui::TestAppContext) {
    use std::sync::{Arc, Mutex};

    let path = std::env::temp_dir().join(format!(
        "org-studio-stable-session-reload-{}.org",
        std::process::id()
    ));
    std::fs::write(&path, "* Before\nold\n").unwrap();
    let loaded = super::load_document(path.clone()).unwrap();
    let app = cx.new(|_| WorkspaceWindow::with_split_layout(true));
    let (session_before, document_id, panel, events) = app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(loaded), cx));
        let session = app.document_session().unwrap().clone();
        let panel = app.reading_panel().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        panel.update(cx, |_, cx| {
            let events = events.clone();
            cx.subscribe_self(move |_, event, _| events.lock().unwrap().push(*event))
                .detach();
        });
        let document_id = session.read(cx).id();
        (session, document_id, panel, events)
    });

    std::fs::write(&path, "* After\nnew\n").unwrap();
    app.update(cx, |app, cx| app.reload_current(cx));
    cx.run_until_parked();

    app.update(cx, |app, cx| {
        let session_after = app.document_session().unwrap();
        assert_eq!(session_after, &session_before);
        assert_eq!(session_after.read(cx).id(), document_id);
        assert_eq!(
            session_after.read(cx).revision(),
            crate::document::Revision(1)
        );
        let preview = panel.read(cx).document();
        assert_eq!(preview.document_id, document_id);
        assert_eq!(preview.revision, crate::document::Revision(1));
        assert_eq!(
            preview
                .text
                .copy_range(crate::document::ByteRange::new(0, preview.text.len_bytes())),
            "* After\nnew\n"
        );
    });
    assert_eq!(
        events.lock().unwrap().as_slice(),
        [super::DerivedEvent::Published {
            document_id,
            revision: crate::document::Revision(1),
        }]
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn dired_help_lists_every_command_and_groups_alias_keys() {
    let (commands, _, _) = preview_input();
    let items = dired_command_items(&commands);
    let labels = items
        .iter()
        .map(|(keys, title)| (keys.as_ref(), title.as_ref()))
        .collect::<Vec<_>>();

    assert_eq!(items.len(), 25);
    assert!(labels.contains(&("n / j", "Next Line")));
    assert!(labels.contains(&("p / k", "Previous Line")));
    assert!(labels.contains(&("^ / h", "Up Directory")));
    assert!(labels.contains(&("RET / l", "Open")));
    assert!(labels.contains(&("H", "History Back")));
    assert!(labels.contains(&("L", "History Forward")));
    assert!(labels.contains(&("g", "Refresh Directory")));
    assert!(labels.contains(&("C-x d", "Open Dired")));
    assert!(labels.contains(&("C-x C-b", "Home")));
    assert!(labels.contains(&("C-x C-d", "Toggle Sidebar")));
    assert!(labels.contains(&("?", "Dired Help")));
    assert!(labels.contains(&("N", "Create File")));
    assert!(labels.contains(&("+", "Create Directory")));
    assert!(labels.contains(&("R", "Rename")));
    assert!(labels.contains(&("C", "Copy")));
    assert!(labels.contains(&("M", "Move")));
    assert!(labels.contains(&("D", "Move to Trash")));
    assert!(labels.contains(&("C-g", "Close command list")));
}

#[test]
fn loads_markdown_without_sending_it_through_the_org_parser() {
    let path = std::env::temp_dir().join(format!("org-studio-markdown-{}.md", std::process::id()));
    std::fs::write(&path, "# Markdown\n\n- **native** preview\n").unwrap();
    let document = super::load_document(path.clone()).unwrap().into_preview();
    let _ = std::fs::remove_file(path);

    assert_eq!(document.format, super::DocumentFormat::Markdown);
    assert!(document.blocks.nodes().is_empty());
    assert!(!document.markdown_blocks.is_empty());
    assert_eq!(document.projection.rows.len(), 3);
}

#[test]
fn markdown_parent_and_minimap_share_identical_display_runs() {
    let path = std::env::temp_dir().join(format!(
        "org-studio-shared-display-runs-{}.md",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "# **标题** and *italic*\n\n```rust\nlet answer = 42;\n```\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap().into_preview();
    let _ = std::fs::remove_file(path);
    let display_map = document.display_map.as_ref().unwrap();
    let style = *super::preview_style(super::PreviewStyleId::Base);
    let heading_layout = display_map.layout(0, style);
    assert_eq!(heading_layout.font_size, style.typography.heading_sizes[0]);
    assert_eq!(
        heading_layout.line_height,
        style.typography.heading_line_heights[0]
    );
    let blank_layout = display_map.layout(1, style);
    assert_eq!(blank_layout.fixed_height, Some(style.spacing.block_gap));

    let heading =
        super::parse_document_inline(super::DocumentFormat::Markdown, "**标题** and *italic*");
    let heading_runs = display_map.runs(0);
    assert_eq!(heading_runs.text.as_ref(), heading.text);
    assert_eq!(heading_runs.inline_spans.as_ref(), heading.spans);

    let code_row = document
        .projection
        .rows
        .iter()
        .position(|row| {
            document
                .text
                .copy_range(row.source.range)
                .contains("answer")
        })
        .expect("code row");
    assert!(!display_map.runs(code_row).code_spans.is_empty());
    let code_layout = display_map.layout(code_row, style);
    assert_eq!(code_layout.padding_left, 16.0);
    assert_eq!(code_layout.padding_right, 16.0);
    assert_eq!(
        code_layout.min_height,
        style
            .row_layout(super::style::RowStyleKind::Code)
            .min_height
    );
}

#[test]
fn org_parent_and_minimap_share_identical_display_runs() {
    let path = std::env::temp_dir().join(format!(
        "org-studio-shared-display-runs-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "* *粗体* and /italic/\n\n#+begin_src rust\nlet n = 7;\n#+end_src\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap().into_preview();
    let _ = std::fs::remove_file(path);
    let display_map = document.display_map.as_ref().unwrap();
    let style = *super::preview_style(super::PreviewStyleId::Base);
    let heading_layout = display_map.layout(0, style);
    assert_eq!(heading_layout.font_size, style.typography.heading_sizes[0]);
    assert_eq!(
        heading_layout.line_height,
        style.typography.heading_line_heights[0]
    );
    let blank_layout = display_map.layout(1, style);
    assert_eq!(blank_layout.fixed_height, Some(style.spacing.block_gap));
    let source = document
        .text
        .copy_range(document.projection.rows.get(0).unwrap().source.range)
        .trim_end_matches(['\r', '\n'])
        .to_owned();
    let expected = super::parse_document_inline(super::DocumentFormat::Org, &source);
    let heading_runs = display_map.runs(0);
    assert_eq!(heading_runs.text.as_ref(), expected.text);
    assert_eq!(heading_runs.inline_spans.as_ref(), expected.spans);

    let code_row = document
        .projection
        .rows
        .iter()
        .position(|row| document.text.copy_range(row.source.range).contains("let n"))
        .expect("code row");
    assert!(!display_map.runs(code_row).code_spans.is_empty());
    let code_layout = display_map.layout(code_row, style);
    assert_eq!(code_layout.padding_left, 16.0);
    assert_eq!(code_layout.padding_right, 16.0);
    let expected_code_layout = style.row_layout(super::style::RowStyleKind::Code);
    assert_eq!(code_layout.font_size, expected_code_layout.font_size);
    assert_eq!(code_layout.line_height, expected_code_layout.line_height);
    assert_eq!(code_layout.min_height, expected_code_layout.min_height);
    assert_eq!(code_layout.padding_top, expected_code_layout.padding_top);
    assert_eq!(
        code_layout.padding_bottom,
        expected_code_layout.padding_bottom
    );
}

#[test]
fn outline_entries_preserve_titles_and_deep_levels() {
    for (extension, text) in [
        ("org", "* Parent / literal\nbody\n***** Child / detail\n"),
        ("md", "# Parent / literal\nbody\n##### Child / detail\n"),
    ] {
        let path = std::env::temp_dir().join(format!(
            "outline-levels-{}.{}",
            std::process::id(),
            extension
        ));
        std::fs::write(&path, text).unwrap();
        let preview = super::load_document(path.clone()).unwrap().into_preview();
        std::fs::remove_file(path).unwrap();
        let entries = preview.outline_entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title.as_ref(), "Parent / literal");
        assert_eq!(entries[1].title.as_ref(), "Child / detail");
        assert_eq!(entries[1].level, 5);
        assert_eq!(entries[1].line, 3);
    }
}
