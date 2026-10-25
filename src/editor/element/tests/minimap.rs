use super::*;

#[gpui::test]
fn tab_alignment_holds_minimap_pixels_until_complete_layout(cx: &mut gpui::TestAppContext) {
    use crate::document::ByteOffset;
    use crate::editor::org_commands::{EditorCommandContext, TableNavigation};
    use std::sync::Arc;
    cx.update(crate::editor::init);
    for (path, table) in [
        ("align.org", "| a|bbb|\n|---+---|\n|长字段| x|\n"),
        ("align.md", "| a|bbb|\n|---|---|\n|长字段| x|\n"),
    ] {
        let path = std::path::PathBuf::from(path);
        let source = format!("{table}{}", "body text\n".repeat(200));
        let session =
            cx.new(|_| DocumentSession::from_utf8(path.clone(), source.into_bytes()).unwrap());
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        view.simulate_resize(gpui::size(px(800.), px(500.)));
        view.run_until_parked();
        editor.update(view, |editor, cx| {
            let previous = editor
                .minimap
                .active_frame()
                .expect("initial minimap is painted");
            assert!(editor.minimap.active_frame_has_complete_layout());
            let bounds = editor.minimap.bounds.unwrap();
            // Retain a stale bootstrap anchor while the complete frame is scrolled.
            let bootstrap = editor.minimap_viewport_geometry(bounds);
            let (total, top, bottom) =
                editor.minimap_source_viewport(f32::from(bounds.size.height));
            editor.scroll_y = 1_200.0;
            editor.minimap.note_viewport_scrolled(1_200.0);
            editor.minimap.stabilize_viewport(
                bootstrap,
                total,
                top,
                bottom,
                f32::from(bounds.size.height),
                crate::minimap::Density::for_width(f32::from(bounds.size.width)),
            );
            let original_geometry = editor.minimap_viewport_geometry(bounds);
            let snapshot = editor.snapshot(cx);
            let context = EditorCommandContext::at(&path, &snapshot, ByteOffset(2)).unwrap();
            assert!(editor.align_table_from_context(
                &snapshot,
                &context,
                TableNavigation::NextCell,
                cx
            ));
            let snapshot = editor.snapshot(cx);
            assert_eq!(snapshot.revision().0, 1);
            editor.minimap.update_snapshot(&snapshot);
            // Reserve, but deliberately hold back the complete-layout job. This
            // exercises the intermediate frame regardless of executor timing.
            let preparation = prepare_minimap_layout_request(
                editor,
                &path,
                &snapshot,
                editor.display_map.wrap_width(),
                gpui::font(".SystemUIFont"),
                px(15.),
                cx.text_system().clone(),
            )
            .unwrap();
            let geometry = editor.minimap_viewport_geometry(bounds);
            assert!(
                (geometry.content_top - original_geometry.content_top).abs() < 0.001,
                "pending layout switched the active image to a stale bootstrap camera: {} -> {}",
                original_geometry.content_top,
                geometry.content_top,
            );
            let (paint, request) = build_minimap(
                editor,
                &path,
                &snapshot,
                false,
                bounds,
                geometry,
                1.,
                crate::theme::current_theme(),
            );
            assert!(Arc::ptr_eq(&paint.image.unwrap().0, &previous.raster.image));
            assert!(
                request.is_none(),
                "Tab must not rasterize new table text using an unfinished layout"
            );
            // Once the first Tab has aligned the table, the next Tab only
            // navigates: neither the document nor raster generation changes.
            let generation = editor.minimap.generation;
            let context =
                EditorCommandContext::at(&path, &snapshot, editor.selection().head()).unwrap();
            assert!(editor.align_table_from_context(
                &snapshot,
                &context,
                TableNavigation::NextCell,
                cx
            ));
            assert_eq!(editor.snapshot(cx).revision(), snapshot.revision());
            assert_eq!(editor.minimap.generation, generation);
            let complete = Arc::new(editor.display_map.clone());
            assert!(editor.minimap.publish_prepared_layout(
                &preparation.key,
                preparation.epoch,
                complete.clone()
            ));
            editor.minimap.invalidate_raster();
            let (_, request) = build_minimap(
                editor,
                &path,
                &snapshot,
                false,
                bounds,
                geometry,
                1.,
                crate::theme::current_theme(),
            );
            assert!(Arc::ptr_eq(
                &request
                    .expect("minimap refresh resumes with complete geometry")
                    .layout,
                &complete
            ));
        });
    }
}

#[test]
fn minimap_rows_adapt_the_canonical_editor_semantics() {
    let source = "plain\n* heading\n- list\n| table |\n> quote\n:KEY: value\n#+title: title\n# comment\n#+begin_src rust\nlet x = 1;\n#+end_src\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let lines = (0..11).collect::<Vec<_>>();
    let styles = SparseEditorStyleSnapshot::for_lines(
        Path::new("contract.org"),
        &snapshot,
        &lines,
        &EditorSyntaxService::default(),
    );
    let theme = crate::theme::current_theme();
    let expected = [
        EditorStyleId::Plain,
        EditorStyleId::Heading(1),
        EditorStyleId::List,
        EditorStyleId::Table,
        EditorStyleId::Quote,
        EditorStyleId::Property,
        EditorStyleId::Meta,
        EditorStyleId::Comment,
        EditorStyleId::CodeBoundary,
        EditorStyleId::Code,
        EditorStyleId::CodeBoundary,
    ];
    let mut rich_span_budget = crate::editor::minimap::RichSpanBudget::default();

    for (line, expected_style) in expected.into_iter().enumerate() {
        let style = styles.line(line as u64).expect("canonical line semantics");
        assert_eq!(style.id, expected_style);
        let text = snapshot.copy_range(style.source_range);
        let spans = semantic_spans(Path::new("contract.org"), &text, style);
        let row = minimap_text_row(text, Some(style), spans, &mut rich_span_budget, theme);
        if matches!(style.id, EditorStyleId::Property | EditorStyleId::Meta) {
            assert_eq!(
                row.color, theme.foreground,
                "property and keyword values inherit the editor body face"
            );
        }
        assert_eq!(
            matches!(row.font_role, crate::typography::FontRole::Table),
            expected_style == EditorStyleId::Table
        );
        assert_eq!(row.block_edge, style.block.as_ref().map(|block| block.edge));
        if line == 9 {
            assert!(row.spans.iter().any(|span| span.color
                == Some(cosmic_text::Color::rgb(
                    (theme.keyword >> 16) as u8,
                    (theme.keyword >> 8) as u8,
                    theme.keyword as u8,
                ))));
        }
    }

    for line in 8..=10 {
        let style = styles.line(line).unwrap();
        let row = minimap_text_row(
            String::new(),
            Some(style),
            Vec::new(),
            &mut rich_span_budget,
            theme,
        );
        assert!(row.block_background.is_some());
        assert!(row.block_accent.is_some());
    }
}
