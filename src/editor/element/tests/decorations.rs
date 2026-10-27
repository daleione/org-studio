use super::super::state::PaintedCaret;
use super::*;
use crate::document::{ByteOffset, Selection};
use gpui::{Entity, TestAppContext, TextRun, VisualTestContext, point, size};

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
fn cookie_progress_bar_stays_under_statistics_after_link_projection(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let source = "#+STARTUP: hidestars\n*** [[https://example.org/status][Status]] [2/5]\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("progress.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session.clone(), cx));
    view.simulate_resize(gpui::size(px(900.), px(400.)));
    view.run_until_parked();
    view.update(|window, cx| {
        let bounds = editor.read(cx).viewport.unwrap();
        let frame = super::super::prepaint::build_frame(&editor, bounds, window, cx);
        let paint = frame.rows.iter().find(|row| row.hit.line.0 == 1).unwrap();
        let row = &paint.hit;
        assert_eq!(row.layout.text.as_ref(), "*** Status [2/5]");
        assert_eq!(paint.preview_runs[0].len, 2);
        assert_eq!(paint.preview_runs[0].color, gpui::transparent_black());
        assert_eq!(frame.hover_quads.len(), 2);
        let track = &frame.hover_quads[0].bounds;
        let fill = &frame.hover_quads[1].bounds;
        assert!((fill.size.width / track.size.width - 0.4).abs() < 0.01);
        let cookie = row.layout.text.find("[2/5]").unwrap();
        let left = row.position_for_display_index(cookie).unwrap();
        let right = row.position_for_display_index(cookie + 5).unwrap();
        assert!(track.left() >= row.text_origin_x + left.x);
        assert!(track.right() <= row.text_origin_x + right.x);
        let snapshot = session.read(cx).snapshot();
        assert_eq!(
            snapshot.copy_range(crate::document::ByteRange::new(0, snapshot.len_bytes())),
            source
        );
    });
}

fn caret_editor<'a>(
    cx: &'a mut TestAppContext,
    source: &str,
) -> (Entity<SemanticEditor>, &'a mut VisualTestContext) {
    cx.update(crate::editor::init);
    let session = cx.new(|_| {
        DocumentSession::from_utf8("caret.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.simulate_resize(size(px(1100.), px(700.)));
    view.run_until_parked();
    (editor, view)
}

fn prepared_frame(
    editor: &Entity<SemanticEditor>,
    view: &mut VisualTestContext,
) -> super::super::PrepaintState {
    view.update(|window, cx| {
        let bounds = editor.read(cx).viewport.unwrap();
        super::super::prepaint::build_frame(editor, bounds, window, cx)
    })
}

#[gpui::test]
fn block_caret_covers_one_shaped_grapheme_and_preserves_its_baseline(cx: &mut TestAppContext) {
    let source = "Maple 界 e\u{301} 👩‍🔬";
    let (editor, view) = caret_editor(cx, source);
    for grapheme in ["M", "界", "e\u{301}", "👩‍🔬"] {
        let index = source.find(grapheme).unwrap();
        editor.update(view, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(index as u64)), cx);
        });
        view.run_until_parked();
        let frame = prepared_frame(&editor, view);
        let caret = frame.caret.unwrap();
        {
            let row = &frame.rows[0].hit;
            let layout = &row.layout.unwrapped_layout;
            let start = layout.x_for_index(index);
            let end = layout.x_for_index(index + grapheme.len());
            assert_eq!(
                caret.bounds().size,
                size(end - start, row.line_height - px(2.))
            );
            assert_eq!(
                caret.bounds().origin,
                point(row.text_origin_x + start, row.origin_y + px(1.))
            );
            assert_eq!(caret.quad.corner_radii, gpui::Corners::all(px(1.)));
            let text = caret.text.unwrap();
            assert_eq!(text.line.text.as_ref(), source);
            assert_eq!(text.origin, point(row.text_origin_x, row.origin_y));
            assert_eq!(text.line.ascent, layout.ascent);
            assert_eq!(text.line.descent, layout.descent);
            assert!(std::sync::Arc::ptr_eq(&text.line, layout));
        }
    }
    view.update(|window, cx| {
        let bounds = editor.read(cx).viewport.unwrap();
        let frame = super::super::prepaint::build_frame(&editor, bounds, window, cx);
        let row = &frame.rows[0];
        for theme in [
            &crate::theme::ORG_STUDIO_LIGHT,
            &crate::theme::ORG_STUDIO_DARK,
        ] {
            let caret = PaintedCaret::character(
                &row.hit,
                0,
                &row.preview_runs,
                &crate::typography::editor_font(),
                theme,
                window,
            )
            .unwrap();
            assert_eq!(
                caret.quad.background,
                gpui::rgb(theme.caret_background).into()
            );
            assert!(std::sync::Arc::ptr_eq(
                &caret.text.unwrap().line,
                &row.hit.layout.unwrapped_layout
            ));
        }
    });
}

#[gpui::test]
fn block_caret_has_space_width_at_line_end_and_on_empty_lines(cx: &mut TestAppContext) {
    let source = "Cedar\n\nBirch";
    let (editor, view) = caret_editor(cx, source);
    for index in [5, 6, source.len()] {
        editor.update(view, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(index as u64)), cx);
        });
        view.run_until_parked();
        let frame = prepared_frame(&editor, view);
        let caret = frame.caret.unwrap();
        view.update(|window, _| {
            let row = &frame
                .rows
                .iter()
                .find(|row| row.hit.range.end.0 == index as u64)
                .unwrap()
                .hit;
            let space = window.text_system().shape_line(
                " ".into(),
                row.layout.font_size(),
                &[TextRun {
                    len: 1,
                    font: crate::typography::editor_font(),
                    ..Default::default()
                }],
                None,
            );
            assert_eq!(
                caret.bounds().size,
                size(space.width(), row.line_height - px(2.))
            );
            assert_eq!(
                caret.bounds().left(),
                row.text_origin_x + row.layout.width()
            );
            assert!(caret.text.is_none());
        });
    }
    editor.update(view, |editor, cx| {
        editor.set_selection(Selection::new(ByteOffset(0), ByteOffset(3)), cx);
    });
    view.run_until_parked();
    assert!(editor.read_with(view, |editor, _| editor.painted_caret_bounds().is_none()));
}

#[gpui::test]
fn block_caret_tracks_characters_on_both_sides_of_a_soft_wrap(cx: &mut TestAppContext) {
    let source = "Amber cedar maple birch ".repeat(12);
    let (editor, view) = caret_editor(cx, &source);
    view.simulate_resize(size(px(420.), px(700.)));
    view.run_until_parked();
    let boundary = editor.read_with(view, |editor, _| {
        let row = &editor.hit_rows[0];
        let boundary = row.layout.wrap_boundaries()[0];
        row.layout.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index
    });
    for index in [boundary - 1, boundary] {
        editor.update(view, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(index as u64)), cx);
        });
        view.run_until_parked();
        editor.read_with(view, |editor, _| {
            let row = &editor.hit_rows[0];
            let caret = editor.painted_caret_bounds().unwrap();
            let layout = &row.layout.unwrapped_layout;
            assert_eq!(
                caret.origin,
                point(row.text_origin_x, row.origin_y + px(1.))
                    + row.position_for_display_index(index).unwrap()
            );
            assert_eq!(
                caret.size.width,
                layout.x_for_index(index + 1) - layout.x_for_index(index)
            );
            if index == boundary {
                assert_eq!(
                    caret.origin,
                    point(row.text_origin_x, row.origin_y + row.line_height + px(1.))
                );
            } else {
                assert_eq!(caret.top(), row.origin_y + px(1.));
            }
        });
    }
}

#[gpui::test]
fn range_highlight_ending_at_a_soft_wrap_stays_on_the_previous_row(cx: &mut TestAppContext) {
    let (editor, view) = caret_editor(cx, &"Amber cedar maple birch ".repeat(12));
    view.simulate_resize(size(px(420.), px(700.)));
    view.run_until_parked();
    editor.read_with(view, |editor, _| {
        let row = &editor.hit_rows[0];
        let boundary = row.layout.wrap_boundaries()[0];
        let end = row.layout.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index;
        let end_position = row.layout.position_for_index(end, row.line_height).unwrap();
        let mut quads = Vec::new();
        crate::editor::highlight::RangeHighlight::rounded(gpui::rgb(0x6688aa).into()).paint(
            &mut quads,
            row,
            0..end,
            px(editor.display_map.wrap_width()),
        );
        assert_eq!(
            quads.len(),
            1,
            "a range ending at a wrap must not paint the next row"
        );
        assert_eq!(
            quads[0].bounds.right(),
            row.text_origin_x + end_position.x + px(3.)
        );
    });
}

#[gpui::test]
fn block_caret_uses_projected_table_cell_text(cx: &mut TestAppContext) {
    let source = "| Value | Note |\n|-------+------|\n| \\alpha{} | 桦e\u{301} |\n";
    let (editor, view) = caret_editor(cx, source);
    for (token, expected) in [("\\alpha{}", "α"), ("桦", "桦"), ("e\u{301}", "e\u{301}")] {
        let index = source.find(token).unwrap();
        editor.update(view, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(index as u64)), cx);
        });
        view.run_until_parked();
        let frame = prepared_frame(&editor, view);
        let caret = frame.caret.unwrap();
        {
            let row = &frame
                .rows
                .iter()
                .find(|row| row.hit.line.0 == 2)
                .unwrap()
                .hit;
            let table = row.table_layout.as_ref().unwrap();
            let fragment = table
                .fragments
                .iter()
                .find(|fragment| fragment.layout.text.contains(expected))
                .unwrap();
            let text = caret.text.as_ref().unwrap();
            assert_eq!(text.line.text, fragment.layout.text);
            assert!(std::sync::Arc::ptr_eq(
                &text.line,
                &fragment.layout.unwrapped_layout
            ));
            let local = fragment.layout.text.find(expected).unwrap();
            let layout = &fragment.layout.unwrapped_layout;
            let start = layout.x_for_index(local);
            assert!(
                (caret.bounds().left() - (row.text_origin_x + fragment.x + start)).abs() < px(0.01)
            );
            assert_eq!(
                caret.bounds().size.width,
                layout.x_for_index(local + expected.len()) - start
            );
        }
    }
}

#[gpui::test]
fn block_caret_stays_visible_without_idle_redraws(cx: &mut TestAppContext) {
    let (editor, view) = caret_editor(cx, "Cedar");
    editor.update(view, |editor, cx| editor.set_minimap(false, None, cx));
    view.run_until_parked();
    view.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    view.run_until_parked();

    let bounds = editor.read_with(view, |editor, _| editor.painted_caret_bounds().unwrap());
    let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
    let _observation = view.update(|_, cx| {
        let notifications = notifications.clone();
        cx.observe(&editor, move |_, _| {
            notifications.set(notifications.get() + 1);
        })
    });
    for _ in 0..4 {
        view.executor()
            .advance_clock(std::time::Duration::from_millis(600));
        view.run_until_parked();
        assert_eq!(
            editor.read_with(view, |editor, _| editor.painted_caret_bounds().unwrap()),
            bounds
        );
    }
    assert_eq!(
        notifications.get(),
        0,
        "idle caret must not schedule redraws"
    );
}

#[gpui::test]
fn compressed_table_rule_caret_keeps_the_original_glyph_origin(cx: &mut TestAppContext) {
    let source = "| Name | Notes |\n|------+-------|\n| Fir | Amber cedar maple birch oak willow chestnut elm |\n";
    let (editor, view) = caret_editor(cx, source);
    editor.update(view, |editor, cx| {
        assert!(editor.align_table_at_selection(cx))
    });
    view.simulate_resize(size(px(420.), px(700.)));
    view.run_until_parked();
    let index = editor.read_with(view, |editor, _| {
        let row = editor.hit_rows.iter().find(|row| row.line.0 == 1).unwrap();
        let rule = row
            .table_layout
            .as_ref()
            .unwrap()
            .fragments
            .iter()
            .find(|fragment| fragment.scaled_source)
            .unwrap();
        let local = rule.display_range.start + rule.display_range.len() / 2;
        ByteOffset(row.range.start.0 + row.display.display_to_source(local) as u64)
    });
    editor.update(view, |editor, cx| {
        editor.set_selection(Selection::caret(index), cx)
    });
    view.run_until_parked();
    let frame = prepared_frame(&editor, view);
    let caret = frame.caret.unwrap();
    {
        let row = &frame
            .rows
            .iter()
            .find(|row| row.hit.line.0 == 1)
            .unwrap()
            .hit;
        let local = row
            .display
            .source_to_display((index.0 - row.range.start.0) as usize);
        let table = row.table_layout.as_ref().unwrap();
        let rule = table
            .fragments
            .iter()
            .find(|fragment| fragment.display_range.contains(&local))
            .unwrap();
        let text = caret.text.as_ref().unwrap();
        assert_eq!(text.origin, point(row.text_origin_x + rule.x, row.origin_y));
        assert!(std::sync::Arc::ptr_eq(
            &text.line,
            &rule.layout.unwrapped_layout
        ));
        assert_eq!(
            caret.bounds().left(),
            row.text_origin_x + table.position_for_index(local, row.line_height).x
        );
    }
}

#[test]
fn clipping_a_caret_preserves_its_paint_geometry() {
    let caret = PaintedCaret::new(
        gpui::Bounds::new(point(px(10.), px(20.)), size(px(12.), px(24.))),
        &crate::theme::ORG_STUDIO_LIGHT,
        None,
    );
    let original = caret.bounds();
    let viewport = gpui::Bounds::new(point(px(15.), px(0.)), size(px(100.), px(100.)));
    let visible = caret.visible_bounds(viewport).unwrap();
    assert_eq!(visible.left(), viewport.left());
    assert_eq!(visible.right(), original.right());
    assert_eq!(caret.bounds(), original);
    assert!(
        caret
            .visible_bounds(gpui::Bounds::new(
                point(px(100.), px(0.)),
                size(px(100.), px(100.))
            ))
            .is_none()
    );
}

#[gpui::test]
fn table_range_ending_at_a_cell_wrap_uses_the_previous_rows_text_width(cx: &mut TestAppContext) {
    let source = "| Name | Notes |\n|------+-------|\n| Fir | Amber cedar maple birch oak willow chestnut elm |\n";
    let (editor, view) = caret_editor(cx, source);
    editor.update(view, |editor, cx| {
        assert!(editor.align_table_at_selection(cx))
    });
    view.simulate_resize(size(px(420.), px(700.)));
    view.run_until_parked();
    editor.read_with(view, |editor, _| {
        let row = editor.hit_rows.iter().find(|row| row.line.0 == 2).unwrap();
        let table = row.table_layout.as_ref().unwrap();
        let cell = table
            .fragments
            .iter()
            .find(|fragment| !fragment.layout.wrap_boundaries().is_empty())
            .unwrap();
        let boundary = cell.layout.wrap_boundaries()[0];
        let end = cell.layout.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index;
        let bounds = table.range_bounds(
            cell.content_range.start..cell.content_range.start + end,
            row.line_height,
        );
        assert_eq!(bounds.len(), 1);
        assert_eq!(bounds[0].top(), px(0.));
        assert_eq!(
            bounds[0].right(),
            cell.x + cell.layout.unwrapped_layout.x_for_index(end)
        );
    });
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
            weight: None,
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

#[gpui::test]
fn inline_delimiters_preserve_geometry_and_reuse_layouts_in_both_display_modes(
    cx: &mut gpui::TestAppContext,
) {
    use crate::command::OrgDisplayFeature;
    use crate::document::{ByteOffset, Selection};
    use std::sync::Arc;
    cx.update(crate::editor::init);
    let source = "Before =sample_value= after ".repeat(8);
    let session = cx.new(|_| {
        DocumentSession::from_utf8("notes.org".into(), source.clone().into_bytes()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.simulate_resize(gpui::size(px(390.), px(600.)));
    view.run_until_parked();
    let start = editor.read_with(view, |e, _| {
        gpui::point(e.hit_rows[0].text_origin_x, e.hit_rows[0].origin_y + px(8.))
    });
    view.simulate_click(start, gpui::Modifiers::default());
    for show_markers in [false, true] {
        if show_markers {
            editor.update(view, |editor, cx| {
                editor.toggle_org_display(OrgDisplayFeature::EmphasisMarkers, cx)
            });
        }
        let rows = [0, 9, 10, 0].map(|offset| {
            editor.update(view, |e, cx| {
                e.set_selection(Selection::caret(ByteOffset(offset)), cx)
            });
            view.run_until_parked();
            editor.read_with(view, |e, _| e.hit_rows[0].clone())
        });
        assert!(!rows[0].layout.wrap_boundaries().is_empty());
        // Entering an already hidden delimiter does not change painted text.
        // Visible delimiters change color, so only that mode needs a new layout.
        assert_eq!(Arc::ptr_eq(&rows[0].layout, &rows[1].layout), !show_markers);
        assert!(Arc::ptr_eq(&rows[1].layout, &rows[2].layout));
        assert!(Arc::ptr_eq(&rows[0].layout, &rows[3].layout));
        let expected = if show_markers {
            source.clone()
        } else {
            "Before sample_value after ".repeat(8)
        };
        for row in &rows {
            assert_eq!(row.layout.text.as_ref(), expected);
            for index in 0..=source.len() {
                assert_eq!(
                    row.position_for_display_index(row.display.source_to_display(index)),
                    rows[0].position_for_display_index(rows[0].display.source_to_display(index))
                );
            }
        }
    }
}
