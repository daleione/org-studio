use super::*;

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
                        plot: None,
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
    cx.update(|cx| {
        super::super::minimap_raster::schedule_minimap_raster(editor.clone(), request, cx)
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert!(
            editor.minimap.active_frame().is_some(),
            "first raster must publish without another scroll"
        );
        assert_eq!(editor.scroll_y, 748.0);
    });
}

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

#[gpui::test]
fn fold_animation_keeps_plots_outside_its_region_visible(cx: &mut gpui::TestAppContext) {
    use crate::document::{ByteOffset, Selection};

    cx.update(crate::editor::init);
    let source = concat!(
        "#+PLOT: title:\"First\" ind:1\n| x | y |\n|---+---|\n| a | 2 |\n\n",
        "* Details\n#+begin_src rust\nfn main() {}\n#+end_src\n\n",
        "#+PLOT: title:\"Second\" ind:1\n| x | y |\n|---+---|\n| b | 3 |\n",
    );
    let session = cx.new(|_| {
        DocumentSession::from_utf8("fold-plots.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.simulate_resize(gpui::size(px(1000.0), px(1400.0)));
    view.run_until_parked();
    view.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    view.run_until_parked();
    let (first_line, first_plot, first_bounds, first_image) = view.read(|cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.plots.lines.len(), 2);
        let (&line, plot) = editor
            .plots
            .lines
            .iter()
            .min_by_key(|(line, _)| *line)
            .unwrap();
        (
            line,
            plot.entry.id,
            editor
                .inline_image_handles
                .iter()
                .find(|handle| handle.plot == Some(plot.entry.id))
                .unwrap()
                .bounds,
            plot.state.as_ref().unwrap().rendered().unwrap().clone(),
        )
    });
    view.update(|window, cx| {
        let focus_handle = editor.read(cx).focus_handle.clone();
        window.focus(&focus_handle, cx);
    });
    for (boundary, expected_plots) in [("#+begin_src", 2), ("* Details", 1)] {
        for collapsing in [true, false] {
            editor.update(view, |editor, cx| {
                editor.set_selection(
                    Selection::caret(ByteOffset(source.find(boundary).unwrap() as u64)),
                    cx,
                );
            });
            view.simulate_keystrokes("tab");
            for progress in [0.0, 0.5, 1.0] {
                view.update(|window, cx| {
                    editor.update(cx, |editor, cx| {
                        let animation = editor.fold_animation.as_mut().unwrap();
                        assert_eq!(animation.collapsing, collapsing);
                        animation.progress = progress;
                        cx.notify();
                    });
                    window.refresh();
                    window.draw(cx).clear(cx);
                    let editor = editor.read(cx);
                    assert_eq!(
                        editor
                            .inline_image_handles
                            .iter()
                            .filter(|handle| { handle.axis == crate::plot::PlotResizeAxis::Both })
                            .count(),
                        expected_plots,
                        "{boundary}, {collapsing}, {progress}"
                    );
                    let first = editor
                        .inline_image_handles
                        .iter()
                        .find(|handle| handle.plot == Some(first_plot))
                        .unwrap();
                    assert_eq!(first.bounds, first_bounds);
                    let image = editor.plots.lines[&first_line]
                        .state
                        .as_ref()
                        .unwrap()
                        .rendered()
                        .unwrap();
                    assert!(std::sync::Arc::ptr_eq(image, &first_image));
                });
            }
            editor.update(view, |editor, cx| {
                editor.finish_fold_animation();
                cx.notify();
            });
            view.run_until_parked();
        }
    }
}

#[gpui::test]
fn a_linked_chart_keeps_its_image_when_new_numbers_finish_rendering(cx: &mut gpui::TestAppContext) {
    use crate::document::{ByteOffset, EditOrigin, Selection};

    let dir = tempfile::tempdir().unwrap();
    let source = "#+PLOT: ind:1 with:histograms file:\"chart.svg\"\n| category | revenue |\n|----------+---------|\n| service  |     510 |\n| product  |     650 |\n\n#+ATTR_ORG: :width 320\n[[file:chart.svg]]\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(dir.path().join("plot.org"), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.simulate_resize(gpui::size(px(1100.0), px(900.0)));
    view.run_until_parked();
    view.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    view.run_until_parked();
    let height = editor.read_with(view, |editor, _| {
        let row = editor.hit_rows.iter().find(|row| row.line.0 == 7).unwrap();
        assert!(row.inline_image_preview);
        row.line_height
    });
    let updates = std::rc::Rc::new(std::cell::Cell::new(0));
    let _observer = view.update(|window, cx| {
        let handle = window.window_handle();
        let service = crate::plot::PlotService::shared(cx);
        let editor = editor.clone();
        let updates = updates.clone();
        cx.observe(&service, move |_, cx| {
            cx.update_window(handle, |_, window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
                let editor = editor.read(cx);
                let row = editor.hit_rows.iter().find(|row| row.line.0 == 7).unwrap();
                assert!(
                    row.inline_image_preview,
                    "a finished chart must be immediately paintable"
                );
                assert!((row.line_height - height).abs() < px(0.01));
                updates.set(updates.get() + 1);
            })
            .unwrap();
        })
    });
    for number in ["520", "530", "510"] {
        editor.update(view, |editor, cx| {
            let start = ByteOffset(source.find("510").unwrap() as u64);
            editor.set_selection(Selection::new(start, ByteOffset(start.0 + 3)), cx);
            editor.replace_selection(number, EditOrigin::Typing, cx);
        });
        view.run_until_parked();
        view.executor()
            .advance_clock(std::time::Duration::from_millis(81));
        view.run_until_parked();
        editor.read_with(view, |editor, _| {
            let row = editor.hit_rows.iter().find(|row| row.line.0 == 7).unwrap();
            assert!(row.inline_image_preview);
            assert!((row.line_height - height).abs() < px(0.01));
        });
    }
    assert!(updates.get() >= 2, "exercise uncached chart completions");
}

#[gpui::test]
fn background_layout_preserves_a_linked_plots_in_progress_resize(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("chart.svg"),
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="360pt" height="187.5pt" viewBox="0 0 360 187.5"><rect width="360" height="187.5"/></svg>"#,
    )
    .unwrap();
    let source = format!(
        "{}#+PLOT: ind:1 with:histograms file:\"chart.svg\"\n| category | revenue |\n|---+---|\n| a | 420 |\n| b | 650 |\n\n#+ATTR_ORG: :width 866.2164\n[[file:chart.svg]]\n",
        "filler\n".repeat(30)
    );
    let session = cx.new(|_| {
        DocumentSession::from_utf8(dir.path().join("plot.org"), source.into_bytes()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    view.simulate_resize(gpui::size(px(1200.0), px(600.0)));
    editor.update(view, |editor, cx| editor.set_minimap(true, Some(80), cx));
    view.run_until_parked();
    view.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    view.run_until_parked();
    editor.update(view, |editor, cx| {
        editor.scroll_y = (editor.display_map.total_height() - 600.0).max(0.0);
        editor.scroll_at_end = true;
        cx.notify();
    });
    view.run_until_parked();
    view.simulate_mouse_move(
        gpui::point(px(100.0), px(100.0)),
        None,
        gpui::Modifiers::default(),
    );
    let start = grip_center(&editor, view);
    press_grip(&editor, view, start);
    for delta in [-10.0f32, -30.0, -60.0, -20.0] {
        // Start a measurement of the authored width, then drag before it publishes.
        editor.update(view, |editor, cx| {
            editor.minimap.cancel_layout_preparation();
            let path = editor.session.read(cx).syntax_path().to_path_buf();
            let preparation = prepare_minimap_layout_request(
                editor,
                &path,
                &editor.snapshot(cx),
                editor.display_map.wrap_width(),
                crate::typography::editor_font(),
                px(editor.font_size_px()),
                cx.text_system().clone(),
            )
            .unwrap();
            schedule_minimap_layout_preparation(cx.entity(), preparation, cx);
        });
        drag_grip_to(&editor, view, start + gpui::point(px(delta), px(0.0)));
        let before = image_sample(&editor, view);
        let samples = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let _observation = view.update(|_, cx| {
            let samples = samples.clone();
            let line = before.line;
            cx.observe(&editor, move |editor, cx| {
                let editor = editor.read(cx);
                samples
                    .borrow_mut()
                    .push((editor.display_map.line_height_px(line), editor.scroll_y));
            })
        });
        for frame in 0..3 {
            view.simulate_mouse_move(
                start + gpui::point(px(delta), px(0.0)),
                Some(gpui::MouseButton::Left),
                gpui::Modifiers::default(),
            );
            view.run_until_parked();
            let after = image_sample(&editor, view);
            assert!(
                (after.height - before.height).abs() < 0.01,
                "drag {delta}, frame {frame}: row height {} -> {} (screen {} -> {}, total {} -> {})",
                before.height,
                after.height,
                before.screen_y,
                after.screen_y,
                before.total,
                after.total,
            );
        }
        assert!(
            !samples.borrow().is_empty()
                && samples
                    .borrow()
                    .iter()
                    .all(|(height, _)| (height - before.height).abs() < 0.01),
            "drag {delta}, expected height {}, samples {:?}",
            before.height,
            samples.borrow(),
        );
    }
    release_grip(&editor, view);
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
