use super::*;
use crate::document::{DocumentSession, EditOrigin, Selection};
use gpui::AppContext;

#[gpui::test]
fn vertical_navigation_crosses_a_memory_plot(cx: &mut gpui::TestAppContext) {
    cx.update(super::super::init);
    let source = "#+PLOT: ind:1\n| month | orders |\n|---+---|\n| July | 120 |\n| August | 180 |\n\nAfter the chart\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("navigation.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(gpui::size(gpui::px(1100.0), gpui::px(900.0)));
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    editor.update(cx, |editor, _| {
        editor
            .plots
            .sources
            .values()
            .next()
            .unwrap()
            .display
            .lock()
            .unwrap()
            .requested_width = Some(800.0);
    });
    let table_end = ByteOffset(source.find("\n\n").unwrap() as u64);
    let next_line = ByteOffset(table_end.0 + 1);
    for height in [900.0, 180.0] {
        cx.simulate_resize(gpui::size(gpui::px(1100.0), gpui::px(height)));
        editor.update(cx, |editor, cx| {
            editor.set_selection(Selection::caret(table_end), cx);
        });
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| {
            assert!(editor.display_map.trailing_height(4) > editor.base_line_height());
        });
        cx.simulate_keystrokes("down");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.selection().head()),
            next_line,
            "Down must cross the chart at viewport height {height}"
        );
        cx.simulate_keystrokes("up");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.selection().head()),
            table_end,
            "Up must return to the table at viewport height {height}"
        );
        cx.simulate_keystrokes("shift-down");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.selection()),
            Selection::new(table_end, next_line)
        );
        cx.simulate_keystrokes("shift-up");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.selection()),
            Selection::caret(table_end)
        );
    }
}

#[gpui::test]
fn minimap_loads_plot_images_outside_the_body_and_releases_its_requests(
    cx: &mut gpui::TestAppContext,
) {
    let chart = |title: &str| {
        format!("#+PLOT: title:\"{title}\" ind:1\n| x | y |\n|---+---|\n| a | 2 |\n\n")
    };
    let source = format!(
        "{}{}{}{}{}",
        chart("body"),
        "filler\n".repeat(50),
        chart("minimap"),
        "filler\n".repeat(1500),
        chart("distant")
    );
    let session =
        cx.new(|_| DocumentSession::from_utf8("media.org".into(), source.into_bytes()).unwrap());
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(gpui::size(gpui::px(1000.0), gpui::px(500.0)));
    editor.update(cx, |editor, cx| editor.set_minimap(true, Some(96), cx));
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.plots.sources.len(), 3);
        let entry = editor
            .plots
            .sources
            .values()
            .find(|entry| {
                entry
                    .source
                    .options
                    .iter()
                    .any(|option| option.contains("minimap"))
            })
            .unwrap();
        assert!(
            !editor
                .plots
                .lines
                .values()
                .any(|plot| plot.entry.id == entry.id)
        );
        let demands = editor.plots.minimap_demands.borrow();
        assert!(
            demands[&entry.id]
                .state()
                .as_ref()
                .and_then(crate::plot::PlotRenderState::rendered)
                .is_some()
        );
        assert!(
            demands.len() < editor.plots.sources.len(),
            "only request plots in the minimap's bounded range"
        );
    });
    editor.update(cx, |editor, cx| editor.set_minimap(false, Some(96), cx));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert!(editor.plots.minimap_demands.borrow().is_empty())
    });
}

#[gpui::test]
fn editing_around_plots_keeps_images_and_geometry_stable(cx: &mut gpui::TestAppContext) {
    fn images(
        editor: &SemanticEditor,
    ) -> HashMap<PlotId, std::sync::Arc<crate::plot::PlotRendered>> {
        editor
            .plots
            .lines
            .values()
            .map(|plot| {
                let image = plot
                    .state
                    .as_ref()
                    .and_then(crate::plot::PlotRenderState::rendered)
                    .expect("an existing plot must keep its image while source updates");
                (plot.entry.id, image.clone())
            })
            .collect()
    }
    let source = "#+PLOT: title:\"first\" ind:1\n| x | y |\n|---+---|\n| a | 2 |\n\n#+PLOT: title:\"middle\" ind:1\n| x | y |\n|---+---|\n| b | 3 |\n\n#+PLOT: title:\"last\" ind:1 file:\"chart.svg\"\n| x | y |\n|---+---|\n| c | 4 |\n\n#+ATTR_ORG: :width 320\n[[file:chart.svg]]\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("plots.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(gpui::size(gpui::px(1200.0), gpui::px(1400.0)));
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    let (before, changed) = editor.read_with(cx, |editor, _| {
        let images = images(editor);
        assert_eq!(images.len(), 3);
        let changed = editor
            .plots
            .lines
            .values()
            .find(|plot| {
                plot.entry
                    .source
                    .options
                    .iter()
                    .any(|option| option.contains("middle"))
            })
            .unwrap()
            .entry
            .id;
        (images, changed)
    });
    let boundaries = editor.read_with(cx, |editor, _| {
        editor
            .plots
            .lines
            .iter()
            .filter(|(_, plot)| plot.entry.id != changed)
            .map(|(&line, plot)| {
                let end = plot
                    .entry
                    .source
                    .image_link
                    .map_or(plot.entry.source.table.end, |link| link.end);
                (plot.entry.id, line, end)
            })
            .collect::<Vec<_>>()
    });
    for (id, line, end) in boundaries {
        let (height, top) = editor.read_with(cx, |editor, _| {
            (
                editor.display_map.line_height_px(line),
                editor.display_map.line_start_y(line),
            )
        });
        for insert in [true, false] {
            editor.update(cx, |editor, cx| {
                editor.set_selection(
                    Selection::new(end, ByteOffset(end.0 + u64::from(!insert))),
                    cx,
                );
                editor.replace_selection(if insert { "\n" } else { "" }, EditOrigin::Typing, cx);
            });
            for indexed in [false, true] {
                if indexed {
                    cx.executor()
                        .advance_clock(std::time::Duration::from_millis(81));
                }
                cx.run_until_parked();
                editor.read_with(cx, |editor, _| {
                    let plot = editor
                        .plots
                        .lines
                        .get(&line)
                        .expect("image anchor survives");
                    assert_eq!(plot.entry.id, id, "blank lines must not move the image");
                    assert!((editor.display_map.line_height_px(line) - height).abs() < 0.01);
                    assert!((editor.display_map.line_start_y(line) - top).abs() < 0.01);
                    let during = images(editor);
                    assert_eq!(during.len(), before.len());
                    for (id, image) in during {
                        assert!(std::sync::Arc::ptr_eq(&image, &before[&id]));
                    }
                });
            }
        }
    }
    editor.update(cx, |editor, cx| {
        let start = source.find("| b | 3").unwrap() as u64 + 6;
        editor.set_selection(Selection::new(ByteOffset(start), ByteOffset(start + 1)), cx);
        editor.replace_selection("300", EditOrigin::Typing, cx);
    });
    cx.update(|_, cx| {
        let view = editor.read(cx);
        let snapshot = view.snapshot(cx);
        prepare(
            &editor,
            &snapshot,
            view.display_map.wrap_width(),
            view.viewport.unwrap(),
            cx,
        );
        let view = editor.read(cx);
        assert!(!view.plots.host.as_ref().unwrap().is_current());
        let during = images(view);
        assert_eq!(during.len(), 3);
        for (id, image) in during {
            assert!(std::sync::Arc::ptr_eq(&image, &before[&id]));
        }
        let linked = view
            .plots
            .lines
            .values()
            .find_map(|plot| plot.entry.source.image_link)
            .unwrap();
        assert_eq!(snapshot.copy_range(linked).trim(), "[[file:chart.svg]]");
    });
    let _observer = cx.update(|_, cx| {
        let before = before.clone();
        cx.observe(&editor, move |editor, cx| {
            let images = images(editor.read(cx));
            assert_eq!(images.len(), 3);
            for (id, image) in images {
                if id != changed {
                    assert!(std::sync::Arc::ptr_eq(&image, &before[&id]));
                }
            }
        })
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        let after = images(editor);
        assert_eq!(after.len(), 3);
        assert!(!std::sync::Arc::ptr_eq(&after[&changed], &before[&changed]));
    });
}

#[gpui::test]
fn tab_at_the_end_of_a_plotted_table_preserves_its_image(cx: &mut gpui::TestAppContext) {
    cx.update(super::super::init);
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        "{}#+PLOT: ind:1 with:histograms file:\"unwritten.svg\"\n| 类别 | 收入 |\n|------+------|\n| 服务 |  420 |\n| 产品 |  650 |\n| 订阅 |  510 |\n\n#+ATTR_ORG: :width 320\n[[file:unwritten.svg]]\n{}",
        "intro\n".repeat(12),
        "after\n".repeat(30),
    );
    let session = cx.new(|_| {
        DocumentSession::from_utf8(
            dir.path().join("table-chart.org"),
            source.as_bytes().to_vec(),
        )
        .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
    cx.simulate_resize(gpui::size(gpui::px(1100.0), gpui::px(900.0)));
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    editor.update(cx, |editor, cx| {
        editor.set_selection(
            Selection::caret(ByteOffset(source.find("510").unwrap() as u64 + 3)),
            cx,
        );
    });
    cx.run_until_parked();
    let geometry = |editor: &SemanticEditor, cx: &gpui::App| {
        let snapshot = editor.snapshot(cx);
        let plot = editor.plots.lines.values().next().expect("bound chart");
        assert!(
            plot.state
                .as_ref()
                .and_then(crate::plot::PlotRenderState::rendered)
                .is_some()
        );
        let line = snapshot
            .line_index_at(plot.entry.source.image_link.unwrap().start)
            .unwrap()
            .0;
        (
            line,
            editor.display_map.line_height_px(line),
            editor.display_map.total_height(),
            editor.scroll_y,
        )
    };
    let before = editor.read_with(cx, geometry);
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    let during = editor.read_with(cx, geometry);
    assert_eq!(during.0, before.0 + 1);
    assert!(
        (during.1 - before.1).abs() < 0.01,
        "image row collapsed during Tab: {before:?} -> {during:?}"
    );
    assert!(
        (during.3 - before.3).abs() < 0.01,
        "Tab moved the viewport: {before:?} -> {during:?}"
    );
    editor.read_with(cx, |editor, _| {
        assert!((during.2 - before.2 - editor.base_line_height()).abs() < 0.01);
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    let after = editor.read_with(cx, geometry);
    assert!((after.1 - before.1).abs() < 0.01);
    assert!((after.3 - before.3).abs() < 0.01);
}

#[gpui::test]
fn memory_plot_resize_is_transient_and_survives_source_edits(cx: &mut gpui::TestAppContext) {
    let source = "#+PLOT: ind:1 with:linespoints\n| x | y |\n|---+---|\n| 1 | 2 |\n| 2 | 3 |\n\nFollowing text.\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8(
            std::path::PathBuf::from("plot.org"),
            source.as_bytes().to_vec(),
        )
        .unwrap()
    });
    let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session.clone(), cx));
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    cx.read(|cx| {
        let editor = editor.read(cx);
        let snapshot = session.read(cx).snapshot();
        assert_eq!(snapshot.revision(), crate::document::Revision::INITIAL);
        assert_eq!(
            snapshot.copy_range(crate::document::ByteRange::new(0, snapshot.len_bytes())),
            source
        );
        assert_eq!(editor.plots.lines.len(), 1);
        assert!(editor.display_map.trailing_height(4) > 200.0);
        assert_eq!(editor.display_map.line_count(), snapshot.len_lines());
    });
    let (id, requested) = editor.update(cx, |editor, cx| {
        let plot = editor.plots.lines.values().next().unwrap().clone();
        let handle = *editor
            .inline_image_handles
            .iter()
            .find(|handle| handle.plot == Some(plot.entry.id))
            .unwrap();
        let position = handle.bounds.center();
        assert!(editor.image_resize_down(
            &gpui::MouseDownEvent {
                position,
                button: gpui::MouseButton::Left,
                click_count: 1,
                modifiers: Default::default(),
                first_mouse: false,
            },
            cx
        ));
        assert!(
            editor.image_resize_move(position + gpui::point(gpui::px(80.0), gpui::px(50.0)), cx)
        );
        {
            let display = plot.entry.display.lock().unwrap();
            assert!(display.resizing);
            assert_eq!(display.requested_height, Some(handle.height + 50.0));
        }
        let sizing = super::super::inline_image::image_sizing(
            editor.display_map.wrap_width(),
            900.0,
            editor.font_size_px(),
        );
        let demand = plot.demand.as_ref().unwrap().clone();
        let before = demand.state().and_then(|state| state.rendered().cloned());
        let during = editor.plots.host.as_mut().unwrap().present(
            plot.entry.clone(),
            sizing,
            plot.style.clone(),
            true,
            cx,
        );
        let during = during.state.and_then(|state| state.rendered().cloned());
        assert!(
            before
                .as_ref()
                .zip(during.as_ref())
                .is_some_and(|(before, during)| Arc::ptr_eq(before, during)),
            "dragging retains the existing rendered image"
        );
        editor.image_resize_up(cx);
        assert!(!plot.entry.display.lock().unwrap().resizing);
        assert_eq!(
            editor.snapshot(cx).revision(),
            crate::document::Revision::INITIAL
        );
        (
            plot.entry.id,
            plot.entry.display.lock().unwrap().requested_width,
        )
    });
    cx.run_until_parked();
    editor.update(cx, |editor, cx| {
        let handle = *editor
            .inline_image_handles
            .iter()
            .find(|handle| {
                handle.plot == Some(id) && handle.axis == crate::plot::PlotResizeAxis::Height
            })
            .unwrap();
        let position = handle.bounds.center();
        let document = editor.plots.host.as_ref().unwrap().document.clone();
        let before = document.read(cx).display_revision;
        let previous_height = document.read(cx).entries[0]
            .display
            .lock()
            .unwrap()
            .requested_height;
        assert!(editor.image_resize_down(
            &gpui::MouseDownEvent {
                position,
                button: gpui::MouseButton::Left,
                click_count: 1,
                modifiers: Default::default(),
                first_mouse: false,
            },
            cx
        ));
        editor.image_resize_move(position + gpui::point(gpui::px(40.0), gpui::px(40.0)), cx);
        assert_eq!(
            document.read(cx).entries[0]
                .display
                .lock()
                .unwrap()
                .requested_width,
            requested,
            "dragging the bottom edge only changes height"
        );
        assert!(editor.cancel_inline_image_resize(cx));
        let document = document.read(cx);
        assert_eq!(
            document.entries[0].display.lock().unwrap().requested_height,
            previous_height
        );
        assert_eq!(
            document.entries[0].display.lock().unwrap().requested_width,
            requested
        );
        assert!(
            document.display_revision > before,
            "cancel must invalidate every host's image geometry"
        );
    });
    editor.update(cx, |editor, cx| {
        editor.set_selection(Selection::caret(ByteOffset(0)), cx);
        editor.replace_selection("intro\n", EditOrigin::Typing, cx);
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    cx.read(|cx| {
        let editor = editor.read(cx);
        let plot = editor
            .plots
            .lines
            .values()
            .next()
            .expect("plot survives insertion above");
        assert_eq!(plot.entry.id, id);
        assert_eq!(
            plot.entry.display.lock().unwrap().requested_width,
            requested
        );
        assert_eq!(plot.entry.source.declaration.start, ByteOffset(6));
        assert!(editor.display_map.trailing_height(5) > 200.0);
        assert_eq!(editor.display_map.trailing_height(4), 0.0);
    });
    editor.update(cx, |editor, cx| {
        let value = source.find("| 1 | 2").unwrap() as u64 + 6 + 6;
        editor.set_selection(Selection::new(ByteOffset(value), ByteOffset(value + 1)), cx);
        editor.replace_selection("20", EditOrigin::Typing, cx);
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    cx.read(|cx| {
        let plot = editor.read(cx).plots.lines.values().next().unwrap();
        assert_eq!(plot.entry.id, id);
        assert_eq!(
            plot.entry.display.lock().unwrap().requested_width,
            requested
        );
        assert!(
            !editor
                .read(cx)
                .snapshot(cx)
                .copy_range(crate::document::ByteRange::new(
                    0,
                    editor.read(cx).snapshot(cx).len_bytes()
                ))
                .contains("ATTR_ORG")
        );
    });
    let previous = editor.update(cx, |editor, cx| {
        let plot = editor.plots.lines.values().next().unwrap().clone();
        let range = crate::document::ByteRange {
            start: plot.entry.source.declaration.start,
            end: plot.entry.source.table.end,
        };
        let replacement = editor
            .snapshot(cx)
            .copy_range(range)
            .replace("| 1 | 20 |", "| 1 | 40 |\n| 3 | 50 |");
        editor.set_selection(Selection::new(range.start, range.end), cx);
        editor.replace_selection(&replacement, EditOrigin::Paste, cx);
        plot
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        let plot = editor.plots.lines.values().next().unwrap();
        assert_eq!(
            plot.entry.id, id,
            "replacing the result keeps the resized chart"
        );
        assert_eq!((plot.width, plot.height), (previous.width, previous.height));
        let rendered = plot
            .state
            .as_ref()
            .and_then(crate::plot::PlotRenderState::rendered)
            .unwrap();
        let previous = previous
            .state
            .as_ref()
            .and_then(crate::plot::PlotRenderState::rendered)
            .unwrap();
        assert_eq!(rendered.artifact.key.size, previous.artifact.key.size);
        assert_ne!(rendered.artifact.key.content, previous.artifact.key.content);
    });
    let (document, entry, settled_size, dragged_revision) = editor.update(cx, |editor, cx| {
        let plot = editor.plots.lines.values().next().unwrap().clone();
        let settled_size = {
            let display = plot.entry.display.lock().unwrap();
            (display.requested_width, display.requested_height)
        };
        let handle = *editor
            .inline_image_handles
            .iter()
            .find(|handle| handle.plot == Some(id))
            .unwrap();
        let position = handle.bounds.center();
        editor.image_resize_down(
            &gpui::MouseDownEvent {
                position,
                button: gpui::MouseButton::Left,
                click_count: 1,
                modifiers: Default::default(),
                first_mouse: false,
            },
            cx,
        );
        editor.image_resize_move(position + gpui::point(gpui::px(40.0), gpui::px(40.0)), cx);
        let revision = plot.document.read(cx).display_revision;
        (plot.document, plot.entry, settled_size, revision)
    });
    cx.update(|window, cx| {
        window.replace_root(cx, |_, _| gpui::Empty);
        window.draw(cx).clear(cx);
    });
    cx.cx.update(|_| drop(editor));
    cx.run_until_parked();
    cx.read(|cx| {
        let display = entry.display.lock().unwrap();
        assert_eq!(
            (display.requested_width, display.requested_height),
            settled_size
        );
        assert!(!display.resizing);
        assert!(
            document.read(cx).display_revision > dragged_revision,
            "closing the editor must restore the shared chart size"
        );
    });
}
