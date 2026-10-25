use super::*;

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
        assert!(!Arc::ptr_eq(&rows[0].layout, &rows[1].layout));
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
