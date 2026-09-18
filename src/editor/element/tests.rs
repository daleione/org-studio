use super::blocks::{
    EditorBlockPaintRow, editor_block_horizontal_bounds, editor_block_segments,
    editor_block_text_inset,
};
use super::cookie::{
    COOKIE_BAR_GAP_EM, COOKIE_BAR_MAX_HEIGHT, cookie_bar_bounds, cookie_bar_insets,
};
use super::minimap::build_minimap;
use super::minimap_layout::{prepare_minimap_layout_request, schedule_minimap_layout_preparation};
use super::minimap_raster::{
    apply_editor_minimap_media_dimensions, minimap_text_color, minimap_text_row,
};
use super::quads::push_swatch_quads;
use super::rows::{MAX_ANIMATED_PAINT_LINES, animated_paint_lines};
use super::scroll::{scroll_is_at_end, stabilized_scroll_y};
use super::text::folded_display_text;
use crate::document::{DocumentSession, DocumentSnapshot, TextSnapshot};
use crate::editor::{
    SemanticEditor,
    layout_map::EditorLayoutMap,
    minimap::RasterMedia,
    syntax::{
        EditorBlockDecoration, EditorBlockEdge, EditorBlockKind, EditorStyleId,
        EditorSyntaxService, SparseEditorStyleSnapshot, semantic_spans,
    },
};
use gpui::{AppContext, px};
use std::path::Path;

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
                crate::theme::current_theme(),
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
        assert_eq!(row.color, minimap_text_color(style.id, theme));
        assert_eq!(row.block_edge, style.block.as_ref().map(|block| block.edge));
        if line == 9 {
            assert!(
                row.spans
                    .iter()
                    .any(|span| span.color == Some(theme.keyword))
            );
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

#[test]
fn folded_heading_display_adds_an_ellipsis_without_changing_source_text() {
    assert_eq!(
        folded_display_text("* Heading".to_owned(), true),
        "* Heading..."
    );
    assert_eq!(
        folded_display_text("* Heading".to_owned(), false),
        "* Heading"
    );
}

#[test]
fn cookie_bar_hangs_below_the_baseline_inside_its_row() {
    // Level-two heading at the default content size: a 30px row of 20.1px type.
    let line_height = 30.0_f32;
    let font_size = 20.1_f32;
    let ascent = 18.65_f32;
    let descent = 4.74_f32;
    let leading = (line_height - ascent - descent) / 2.0;
    let baseline = leading + ascent;
    let (top, bottom) = cookie_bar_bounds(
        px(0.0),
        px(line_height),
        px(ascent),
        px(descent),
        px(font_size),
    );
    assert!(
        (f32::from(top) - (baseline + font_size * COOKIE_BAR_GAP_EM)).abs() < 0.01,
        "the bar hangs a gap below the digits: {top:?}"
    );
    // Clear of the brackets, whose glyphs descend below the baseline.
    assert!(f32::from(top) > baseline + 3.0);
    assert!(f32::from(bottom) <= line_height + leading + 0.001);
    assert!(f32::from(bottom - top) > 2.0);

    // Tight rows clamp the bar rather than let it reach the next row.
    for line_height in [10.0_f32, 12.0, 15.6, 18.0, 24.0, 40.0] {
        let leading = ((line_height - ascent - descent) / 2.0).max(0.0);
        let (top, bottom) = cookie_bar_bounds(
            px(40.0),
            px(line_height),
            px(ascent),
            px(descent),
            px(font_size),
        );
        assert!(f32::from(top) >= 40.0, "line_height {line_height}: {top:?}");
        assert!(
            f32::from(bottom) <= 40.0 + line_height + leading + 0.001,
            "line_height {line_height}: {bottom:?}"
        );
        let height = f32::from(bottom - top);
        assert!(height > 0.0 && height <= COOKIE_BAR_MAX_HEIGHT + 0.001);
    }
}

#[gpui::test]
fn cookie_bar_clears_the_real_heading_metrics(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let source = "** DONE Phase 3：颜色收敛与深色验收 [4/7]\nbody\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(
            Path::new("cookie.org").to_path_buf(),
            source.as_bytes().to_vec(),
        )
        .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(gpui::size(px(900.0), px(400.0)));
    cx.run_until_parked();
    let (line_height, font_size, ascent, descent) = cx.read(|cx| {
        let editor = editor.read(cx);
        let row = editor
            .hit_rows
            .iter()
            .find(|row| row.line.0 == 0)
            .expect("the heading row is rendered");
        (
            row.line_height,
            row.layout.font_size(),
            row.layout.ascent(),
            row.layout.descent(),
        )
    });
    let (top, bottom) = cookie_bar_bounds(px(0.0), line_height, ascent, descent, font_size);
    let line_height = f32::from(line_height);
    let ascent = f32::from(ascent);
    let descent = f32::from(descent);
    let leading = ((line_height - ascent - descent) / 2.0).max(0.0);
    let baseline = leading + ascent;
    // The real font must leave room for the gap under the digits.
    assert!(
        f32::from(top) > baseline + 3.0,
        "bar top {top:?} against baseline {baseline} (line height {line_height})"
    );
    assert!(f32::from(bottom) <= line_height + leading + 0.001);
    assert!(f32::from(bottom - top) >= 2.0);

    // Insets come from the real font's ink boxes and must never invert the bar.
    let (left_inset, right_inset, token_width) = cx.update(|window, cx| {
        let editor = editor.read(cx);
        let row = editor
            .hit_rows
            .iter()
            .find(|row| row.line.0 == 0)
            .expect("the heading row is rendered");
        let range = crate::org_syntax::cookie::trailing_progress(&row.layout.text)
            .expect("trailing cookie")
            .0;
        let start = row
            .position_for_display_index(range.start)
            .expect("cookie start");
        let end = row
            .position_for_display_index(range.end)
            .expect("cookie end");
        let width = end.x - start.x;
        let (left, right) = cookie_bar_insets(row, &range, width, window);
        (left, right, width)
    });
    assert!(f32::from(left_inset) > 0.0, "left inset {left_inset:?}");
    assert!(f32::from(right_inset) > 0.0, "right inset {right_inset:?}");
    assert!(left_inset + right_inset < token_width * 0.7);
}

#[gpui::test]
fn caret_blink_resets_on_movement_and_stops_when_hidden(cx: &mut gpui::TestAppContext) {
    let session = cx.new(|_| {
        DocumentSession::from_utf8(Path::new("caret.md").to_path_buf(), b"hello".to_vec()).unwrap()
    });
    let editor = cx.new(|cx| SemanticEditor::new(session, cx));
    editor.update(cx, |editor, cx| {
        assert_eq!(editor.caret_opacity(true, cx), 1.0);
        assert!(editor.caret_blink_task.is_some());
        editor.caret_blink.as_mut().unwrap().2 =
            std::time::Instant::now() - std::time::Duration::from_millis(800);
        assert_eq!(editor.caret_opacity(true, cx), 0.0);
        editor.selection = crate::editor::Selection::caret(crate::document::ByteOffset(1));
        assert_eq!(editor.caret_opacity(true, cx), 1.0);
        editor.caret_opacity(false, cx);
        assert!(editor.caret_blink.is_none());
        assert!(editor.caret_blink_task.is_none());
    });
}

#[gpui::test]
fn minimap_prefetch_commits_image_height_before_the_editor_reaches_it(
    cx: &mut gpui::TestAppContext,
) {
    let source = "line\n".repeat(100);
    let session = cx.new(|_| {
        DocumentSession::from_utf8(
            std::path::PathBuf::from("prefetched-image.org"),
            source.into_bytes(),
        )
        .unwrap()
    });
    let editor = cx.new(|cx| SemanticEditor::new(session, cx));

    editor.update(cx, |editor, _| {
        editor.display_map.configure(100, 800.0);
        let initial_height = editor.display_map.total_height();
        let media = [RasterMedia {
            line: 50,
            line_start: 250,
            row_offset_units: 50.0,
            row_height_units: 1.0,
            path: std::path::PathBuf::from("image.svg"),
            dimensions: Some((100, 400)),
        }];

        assert!(apply_editor_minimap_media_dimensions(editor, &media));
        let resolved_height = editor.display_map.total_height();
        assert!(resolved_height > initial_height + 300.0);
        assert_eq!(
            editor
                .inline_image_line_dimensions
                .borrow()
                .get(&50)
                .map(|metrics| (metrics.line_start, metrics.source)),
            Some((250, (100, 400)))
        );
        assert!(!apply_editor_minimap_media_dimensions(editor, &media));
        assert_eq!(editor.display_map.total_height(), resolved_height);
    });
}

#[test]
fn block_chrome_scrolls_horizontally_with_its_source_text() {
    let (left, right) = editor_block_horizontal_bounds(px(100.0), px(700.0), 180.0);
    assert_eq!(left, px(-80.0));
    assert_eq!(right, px(520.0));
}

#[gpui::test]
fn minimap_opened_after_scroll_keeps_visible_svg_dimensions_stable(cx: &mut gpui::TestAppContext) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview-basics.org");
    let source = std::fs::read(&path).unwrap();
    let session = cx.new(|_| DocumentSession::from_utf8(path.clone(), source).unwrap());
    let editor = cx.new(|cx| SemanticEditor::new(session.clone(), cx));
    let media = cx.update(|cx| {
        let snapshot = session.read(cx).snapshot();
        (0..snapshot.len_lines())
            .filter_map(|line| {
                let range = snapshot
                    .line_content_range(crate::document::LineIndex(line))
                    .ok()?;
                let text = snapshot.copy_range(range);
                let target = crate::org_syntax::standalone_image_path(&text)?;
                let path = crate::preview::resolve_image_path(&path, target);
                let image = crate::editor::image_loader::decode_svg(
                    &std::fs::read(&path).unwrap(),
                    &cx.svg_renderer(),
                )
                .unwrap();
                let dimensions = crate::preview::image_dimensions(&path).unwrap();
                Some((
                    RasterMedia {
                        line,
                        line_start: range.start.0,
                        row_offset_units: 0.0,
                        row_height_units: 1.0,
                        path,
                        dimensions: Some(dimensions),
                    },
                    image,
                ))
            })
            .collect::<Vec<_>>()
    });
    assert!(
        media.len() >= 2,
        "fixture must cover multiple SVGs so competing dimension writers are exercised"
    );
    editor.update(cx, |editor, cx| {
        editor.set_minimap(false, Some(144), cx);
        editor
            .display_map
            .configure(session.read(cx).snapshot().len_lines(), 1606.0);
        editor.viewport = Some(gpui::Bounds::new(
            gpui::point(px(0.0), px(0.0)),
            gpui::size(px(1800.0), px(1028.0)),
        ));
        editor.scroll_y = 748.0;
        editor.set_minimap(true, Some(144), cx);
        // Each frame measures visible body images, then the background minimap
        // publishes its discovered dimensions. These writers must converge without
        // another scroll moving the SVGs out of the body viewport.
        for _ in 0..3 {
            for (source, image) in &media {
                let (_, (width, height)) =
                    editor.accept_inline_image_render(&source.path, image.clone());
                editor.inline_image_line_dimensions.borrow_mut().insert(
                    source.line,
                    crate::editor::InlineImageMetrics {
                        line_start: source.line_start,
                        source: (width, height),
                        spec: Default::default(),
                    },
                );
                let (_, height) = crate::preview::fitted_image_size(width, height, 640.0);
                editor
                    .display_map
                    .update_line_layout(source.line, 1, height, 6.0, 6.0);
                assert!(
                    !apply_editor_minimap_media_dimensions(editor, std::slice::from_ref(source)),
                    "visible SVG must not repeatedly reject minimap publication: {:?}",
                    source.path,
                );
            }
            assert_eq!(editor.scroll_y, 748.0);
        }
    });
    let request = editor.update(cx, |editor, cx| {
        let bounds = gpui::Bounds::new(
            gpui::point(px(1656.0), px(0.0)),
            gpui::size(px(144.0), px(1028.0)),
        );
        let snapshot = session.read(cx).snapshot();
        editor.minimap.update_snapshot(&snapshot);
        let geometry = editor.minimap_viewport_geometry(bounds);
        let (paint, request) = build_minimap(
            editor,
            &path,
            &snapshot,
            false,
            bounds,
            geometry,
            2.0,
            crate::theme::current_theme(),
        );
        assert!(paint.image.is_none());
        request.expect("opening minimap must schedule its first raster")
    });
    cx.update(|cx| super::minimap_raster::schedule_minimap_raster(editor.clone(), request, cx));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert!(
            editor.minimap.active_frame().is_some(),
            "first raster must publish without another scroll"
        );
        assert_eq!(editor.scroll_y, 748.0);
    });
}

#[test]
fn contiguous_block_rows_form_one_active_closed_container() {
    let segments = editor_block_segments([
        block_row(EditorBlockEdge::Open, 0.0),
        block_row(EditorBlockEdge::Body, 28.0),
        block_row(EditorBlockEdge::Close, 56.0),
    ]);

    assert_eq!(segments.len(), 1);
    assert!(segments[0].open);
    assert!(segments[0].close);
    assert!(segments[0].active);
    assert_eq!(segments[0].top, 0.0);
    assert_eq!(segments[0].bottom, 84.0);
}

#[test]
fn every_decorated_block_adds_visual_text_inset() {
    let decoration = |kind| EditorBlockDecoration {
        kind,
        edge: EditorBlockEdge::Body,
        body_line: Some(1),
    };
    let source = decoration(EditorBlockKind::Source);
    let markdown = decoration(EditorBlockKind::MarkdownFence);
    let quote = decoration(EditorBlockKind::Quote);

    assert_eq!(editor_block_text_inset(Some(&source)), 42.0);
    assert_eq!(editor_block_text_inset(Some(&markdown)), 42.0);
    assert_eq!(editor_block_text_inset(Some(&quote)), 16.0);
    assert_eq!(editor_block_text_inset(None), 0.0);
}

#[test]
fn bottom_stays_pinned_while_measured_document_height_converges() {
    assert!(scroll_is_at_end(800.0, 200.0, 1_000.0));
    assert_eq!(stabilized_scroll_y(true, 800.0, 200.0, 1_120.0), 920.0);
    assert_eq!(stabilized_scroll_y(false, 420.0, 200.0, 1_120.0), 420.0);
    assert_eq!(stabilized_scroll_y(false, 980.0, 200.0, 1_120.0), 920.0);
}

#[test]
fn large_fold_animations_shape_only_a_bounded_sample_of_changed_lines() {
    let mut display_map = EditorLayoutMap::default();
    display_map.configure(1_000, 700.0);

    let lines = animated_paint_lines(&display_map, 0..1_000, std::slice::from_ref(&(1..999)));

    assert!(lines.len() <= MAX_ANIMATED_PAINT_LINES as usize + 2);
    assert_eq!(lines.first(), Some(&0));
    assert_eq!(lines.last(), Some(&999));
    assert!(lines.contains(&1));
    assert!(lines.contains(&998));
}

#[gpui::test]
fn swatch_quads_follow_the_literal_glyph_box(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let source = "palette: #ff0000 done\nbody\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(
            Path::new("swatch.org").to_path_buf(),
            source.as_bytes().to_vec(),
        )
        .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(gpui::size(px(900.0), px(400.0)));
    cx.run_until_parked();
    let theme = crate::theme::current_theme();

    let span = |swatch: Option<u32>, bytes: std::ops::Range<usize>| {
        crate::editor::syntax::EditorSemanticSpan {
            bytes,
            color: None,
            weight: crate::editor::syntax::EditorSemanticWeight::Normal,
            italic: false,
            underline: false,
            strikethrough: false,
            pill: false,
            swatch,
            link: None,
        }
    };

    // A span without a swatch must not paint anything.
    cx.read(|cx| {
        let editor = editor.read(cx);
        let row = editor
            .hit_rows
            .iter()
            .find(|row| row.line.0 == 0)
            .expect("the palette row is rendered");
        let mut quads = Vec::new();
        push_swatch_quads(
            &mut quads,
            row,
            std::slice::from_ref(&span(None, 0..1)),
            px(800.0),
            theme,
        );
        assert!(quads.is_empty());
    });

    cx.read(|cx| {
        let editor = editor.read(cx);
        let row = editor
            .hit_rows
            .iter()
            .find(|row| row.line.0 == 0)
            .expect("the palette row is rendered");
        let text = row.layout.text.clone();
        let start = text.find("#ff0000").expect("hex literal");
        let end = start + "#ff0000".len();
        let mut quads = Vec::new();
        push_swatch_quads(
            &mut quads,
            row,
            std::slice::from_ref(&span(Some(0xff0000ff), start..end)),
            px(800.0),
            theme,
        );
        assert_eq!(quads.len(), 1);
        let quad = &quads[0];
        let start_x = row.position_for_display_index(start).unwrap().x;
        let end_x = row.position_for_display_index(end).unwrap().x;
        // The shared pill geometry pads the glyph box on both sides.
        let pad = px(crate::editor::highlight::RangeHighlight::PAD_X);
        let inset = px(crate::editor::highlight::RangeHighlight::INSET_Y);
        assert_eq!(quad.bounds.left(), row.text_origin_x + start_x - pad);
        assert_eq!(quad.bounds.right(), row.text_origin_x + end_x + pad);
        assert_eq!(quad.bounds.top(), row.origin_y + inset);
        assert_eq!(quad.bounds.bottom(), row.origin_y + row.line_height - inset);
        assert_eq!(
            quad.corner_radii.top_left,
            px(crate::editor::highlight::RangeHighlight::RADIUS)
        );
        assert_eq!(
            quad.corner_radii.bottom_right,
            px(crate::editor::highlight::RangeHighlight::RADIUS)
        );
    });
}

/// A window on a document whose only standalone image has `before` filler lines
/// above it and `after` below, plus the temporary directory to clean up.
fn image_drag_window<'a>(
    cx: &'a mut gpui::TestAppContext,
    name: &str,
    before: usize,
    after: usize,
) -> (
    gpui::Entity<SemanticEditor>,
    &'a mut gpui::VisualTestContext,
    std::path::PathBuf,
) {
    cx.update(crate::editor::init);
    let dir = std::env::temp_dir().join(format!(
        "org-studio-image-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after the epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("fixture directory");
    std::fs::write(
        dir.join("image.svg"),
        br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 200"><rect width="400" height="200"/></svg>"#,
    )
    .expect("fixture image");
    let mut source = "filler\n".repeat(before);
    source.push_str("[[file:image.svg]]\n");
    source.push_str(&"filler\n".repeat(after));
    let session = cx.new(|_| {
        DocumentSession::from_utf8(dir.join(format!("{name}.org")), source.into_bytes())
            .expect("fixture document")
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.simulate_resize(gpui::size(px(900.0), px(400.0)));
    view.run_until_parked();
    (editor, view, dir)
}

/// Geometry of the fixture's image row in the current frame.
struct ImageSample {
    line: u64,
    /// Row top within the viewport.
    screen_y: f32,
    /// Measured row height.
    height: f32,
    total: f32,
    at_end: bool,
}

fn image_sample(
    editor: &gpui::Entity<SemanticEditor>,
    view: &mut gpui::VisualTestContext,
) -> ImageSample {
    editor.update(view, |editor, cx| {
        let snapshot = editor.snapshot(cx);
        let line = (0..snapshot.len_lines())
            .find(|line| {
                snapshot
                    .line_content_range(crate::document::LineIndex(*line))
                    .is_ok_and(|range| {
                        crate::org_syntax::standalone_image_path(&snapshot.copy_range(range))
                            .is_some()
                    })
            })
            .expect("the fixture has one standalone image line");
        ImageSample {
            line,
            screen_y: editor.display_map.line_start_y(line) - editor.scroll_y,
            height: editor.display_map.line_height_px(line),
            total: editor.display_map.total_height(),
            at_end: editor.scroll_at_end,
        }
    })
}

/// Asserts the document text contains `needle`, or lacks it when `None`.
fn assert_document_text(
    editor: &gpui::Entity<SemanticEditor>,
    view: &mut gpui::VisualTestContext,
    needle: Option<&str>,
) {
    editor.update(view, |editor, cx| {
        let snapshot = editor.snapshot(cx);
        let text = snapshot.copy_range(crate::document::ByteRange::new(0, snapshot.len_bytes()));
        match needle {
            Some(needle) => assert!(text.contains(needle), "expected {needle:?} in:\n{text}"),
            None => assert!(
                !text.contains("#+ATTR_ORG:"),
                "expected no attribute in:\n{text}"
            ),
        }
    });
}

/// Centre of the published image grip; the image must already be loaded.
fn grip_center(
    editor: &gpui::Entity<SemanticEditor>,
    view: &mut gpui::VisualTestContext,
) -> gpui::Point<gpui::Pixels> {
    editor
        .update(view, |editor, _| {
            editor.inline_image_handles.first().copied()
        })
        .expect("the grip is published once the image is loaded")
        .bounds
        .center()
}

fn press_grip(
    editor: &gpui::Entity<SemanticEditor>,
    view: &mut gpui::VisualTestContext,
    at: gpui::Point<gpui::Pixels>,
) {
    editor.update(view, |editor, cx| {
        assert!(editor.image_resize_down(
            &gpui::MouseDownEvent {
                button: gpui::MouseButton::Left,
                position: at,
                modifiers: gpui::Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            },
            cx,
        ));
    });
}

fn drag_grip_to(
    editor: &gpui::Entity<SemanticEditor>,
    view: &mut gpui::VisualTestContext,
    at: gpui::Point<gpui::Pixels>,
) {
    editor.update(view, |editor, cx| {
        assert!(editor.image_resize_move(at, cx));
    });
}

fn release_grip(editor: &gpui::Entity<SemanticEditor>, view: &mut gpui::VisualTestContext) {
    editor.update(view, |editor, cx| editor.image_resize_up(cx));
}

#[gpui::test]
fn dragging_an_inline_image_grip_writes_a_width_attribute(cx: &mut gpui::TestAppContext) {
    let (editor, view, dir) = image_drag_window(cx, "drag", 0, 1);
    let start = grip_center(&editor, view);
    let end = start + gpui::point(px(40.0), px(0.0));
    // Dragged through the real event path, so the element's input routing is
    // covered too.
    view.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
    // Every intermediate step must stay *measured* at its exact height.
    // Invalidating the row instead drops it to an estimated height, so the
    // document height oscillates between estimated and measured on each pointer
    // move and the viewport thrashes.
    let mut previous = 0.0;
    for step in [10.0f32, 20.0, 40.0] {
        view.simulate_mouse_move(
            start + gpui::point(px(step), px(0.0)),
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        let expected = 6.0 + (400.0 + step) / 2.0 + 6.0;
        let sample = image_sample(&editor, view);
        assert_eq!(
            sample.height, expected,
            "drag step +{step}px must stay measured at {expected}px, not an estimate"
        );
        assert!(
            sample.height >= previous,
            "dragged row height must grow monotonically, got {} after {previous}",
            sample.height
        );
        previous = sample.height;
    }
    view.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::default());
    view.run_until_parked();
    assert_document_text(&editor, view, Some("#+ATTR_ORG: :width 440"));

    // The commit is one ordinary edit, so a single undo restores auto size.
    view.simulate_keystrokes("cmd-z");
    view.run_until_parked();
    assert_document_text(&editor, view, None);
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

/// Writing a new `#+ATTR_ORG:` line above the image inserts a text line. The
/// image must keep its screen position through that commit: otherwise it jumps
/// down one row at the exact moment the drag ends. The document event that
/// repairs the rows is delivered after the edit, so the state sampled here is
/// the one the very next frame paints.
#[gpui::test]
fn committing_a_new_width_attribute_keeps_the_image_in_place(cx: &mut gpui::TestAppContext) {
    let (editor, view, dir) = image_drag_window(cx, "pin", 30, 30);

    editor.update(view, |editor, cx| {
        editor.scroll_y = editor.display_map.line_start_y(30) - 100.0;
        cx.notify();
    });
    view.run_until_parked();
    let before = image_sample(&editor, view);

    let start = grip_center(&editor, view);
    let end = start + gpui::point(px(40.0), px(0.0));
    press_grip(&editor, view, start);
    drag_grip_to(&editor, view, end);
    release_grip(&editor, view);

    let after = image_sample(&editor, view);
    assert_eq!(
        after.line,
        before.line + 1,
        "the attribute line is inserted above the image"
    );
    assert!(
        (after.screen_y - before.screen_y).abs() < 0.01,
        "no frame may paint the page shifted: {} -> {}",
        before.screen_y,
        after.screen_y
    );
    assert_eq!(
        after.height, 232.0,
        "the row stays measured at the dragged height"
    );
    // The repaired height must sit on the image's own line: the line splice runs
    // after the edit, so a repair in pre-splice numbering would spill the image's
    // height onto the following line and displace every row below it.
    editor.update(view, |editor, _| {
        assert_eq!(
            editor.display_map.line_height_px(after.line + 1),
            editor.display_map.base_line_height(),
            "the image height must not spill onto the following line"
        );
    });

    view.run_until_parked();
    let settled = image_sample(&editor, view);
    assert!(
        (settled.screen_y - before.screen_y).abs() < 0.01,
        "the image must stay put after the frame too: {} -> {}",
        before.screen_y,
        settled.screen_y
    );
    assert_eq!(settled.height, 232.0);
    assert_document_text(&editor, view, Some("#+ATTR_ORG: :width 440"));
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

/// With the viewport pinned at the document end, the commit must not let the end
/// pin jump: the document event that follows the edit used to invalidate the
/// repaired image row, collapsing the total height and pulling the scroll to a
/// stale end position for one frame.
#[gpui::test]
fn committing_at_the_document_end_keeps_the_image_in_place(cx: &mut gpui::TestAppContext) {
    let (editor, view, dir) = image_drag_window(cx, "end", 30, 4);

    // Pin the viewport to the end the way a scroll to the bottom does.
    editor.update(view, |editor, cx| {
        let viewport_height = editor.viewport.map_or(0.0, |b| f32::from(b.size.height));
        editor.scroll_y = (editor.display_map.total_height() - viewport_height).max(0.0);
        editor.scroll_at_end = true;
        cx.notify();
    });
    view.run_until_parked();
    assert!(
        image_sample(&editor, view).at_end,
        "the fixture must start pinned at the document end"
    );

    let start = grip_center(&editor, view);
    let end = start + gpui::point(px(40.0), px(0.0));
    press_grip(&editor, view, start);
    drag_grip_to(&editor, view, end);
    // Let a frame run with the drag applied, so the end pin has caught up with the
    // dragged row height. The reference is then a settled state and the assertions
    // below are about the commit alone.
    view.run_until_parked();
    let before = image_sample(&editor, view);

    release_grip(&editor, view);
    let after = image_sample(&editor, view);
    assert!(
        after.total >= before.total,
        "the document must not lose height on commit: {} -> {}",
        before.total,
        after.total
    );

    for step in ["immediate", "frame1", "frame2"] {
        let sample = image_sample(&editor, view);
        assert!(
            (sample.screen_y - before.screen_y).abs() < 0.01,
            "{step}: the image must keep its screen position: {} -> {}",
            before.screen_y,
            sample.screen_y
        );
        assert!(
            sample.total >= before.total,
            "{step}: the document must not lose height: {} -> {}",
            before.total,
            sample.total
        );
        assert!(
            sample.at_end,
            "{step}: the viewport must stay pinned at the end"
        );
        view.run_until_parked();
    }
    assert_document_text(&editor, view, Some("#+ATTR_ORG: :width"));
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

/// The minimap's prepared layout must use the same image sizing rule as the
/// editor's own frame, so an authored `#+ATTR_ORG: :width` cannot make the two
/// disagree about how tall an image row is.
#[gpui::test]
fn prepared_minimap_layout_honours_an_authored_image_width(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let dir = std::env::temp_dir().join(format!(
        "org-studio-image-prepped-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after the epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("fixture directory");
    std::fs::write(
        dir.join("image.svg"),
        br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 200"><rect width="400" height="200"/></svg>"#,
    )
    .expect("fixture image");
    let path = dir.join("sized.org");
    let source = "#+ATTR_ORG: :width 300\n[[file:image.svg]]\nfiller\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(path.clone(), source.as_bytes().to_vec()).expect("fixture")
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.simulate_resize(gpui::size(px(900.0), px(400.0)));
    view.run_until_parked();

    editor.update(view, |editor, cx| {
        let snapshot = editor.snapshot(cx);
        let preparation = prepare_minimap_layout_request(
            editor,
            &path,
            &snapshot,
            editor.display_map.wrap_width(),
            gpui::font(".SystemUIFont"),
            px(15.),
            crate::theme::current_theme(),
            cx.text_system().clone(),
        )
        .expect("layout request");
        schedule_minimap_layout_preparation(cx.entity(), preparation, cx);
    });
    view.run_until_parked();

    editor.update(view, |editor, _| {
        let prepared = editor
            .minimap
            .prepared_layout()
            .expect("the background layout is published");
        // A 400x200 image at an authored 300px width is 150px tall, plus the row
        // padding: the auto fit would have produced 212px instead.
        let height = prepared.line_height_px(1);
        assert!(
            (height - 162.0).abs() < 0.5,
            "the prepared layout must honour :width, got {height}px"
        );
    });
    std::fs::remove_dir_all(&dir).expect("cleanup");
}
