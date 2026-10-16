use super::*;
use gpui::{GlyphId, WrapBoundary, canvas, div, point, prelude::*};

fn geometry(
    frame: &PreviewFrame,
) -> Vec<(Pixels, Vec<WrapBoundary>, Vec<(GlyphId, Point<Pixels>)>)> {
    frame
        .commands
        .iter()
        .filter_map(|command| match &command.paint {
            Paint::Wrapped { layout, line, .. } => {
                assert!(Arc::ptr_eq(&**line.borrow(), layout));
                Some((
                    layout.font_size(),
                    layout.wrap_boundaries.to_vec(),
                    layout
                        .runs()
                        .iter()
                        .flat_map(|run| run.glyphs.iter().map(|glyph| (glyph.id, glyph.position)))
                        .collect(),
                ))
            }
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn recorded_rounded_boxes_match_the_live_style_painter(cx: &mut gpui::TestAppContext) {
    let cx = cx.add_empty_window();
    let slot = FrameSlot::default();
    let publish = slot.clone();
    cx.draw(
        point(px(0.), px(0.)),
        size(px(80.), px(28.)),
        move |_, _| {
            DocumentStage::new(
                record_box(
                    div()
                        .size_full()
                        .rounded_full()
                        .bg(rgb(0xff3300))
                        .border_1()
                        .border_color(rgb(0x112233)),
                ),
                publish.clone(),
                true,
            )
            .into_any_element()
        },
    );
    let frame = slot.latest().unwrap();
    let (live, scale) = cx.update(|window, _| (window.painted_quads(), window.scale_factor()));
    let recorded = frame
        .commands
        .iter()
        .filter_map(|command| {
            if let Paint::Quad(quad) = &command.paint {
                Some(quad)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(recorded.len(), 2);
    for quad in recorded {
        let native = live
            .iter()
            .find(|native| {
                native.background == quad.background && native.border_color == quad.border_color
            })
            .expect("matching live fill or border");
        assert_eq!(native.corner_radii, quad.corner_radii.scale(scale));
        assert_eq!(native.border_widths, quad.border_widths.scale(scale));
    }
}

#[gpui::test]
fn drawing_at_multiple_sizes_preserves_original_glyphs_and_wraps(cx: &mut gpui::TestAppContext) {
    let cx = cx.add_empty_window();
    let slot = FrameSlot::default();
    let publish = slot.clone();
    cx.draw(
        point(px(0.), px(0.)),
        size(px(130.), px(300.)),
        move |_, _| {
            DocumentStage::new(
                div()
                    .size_full()
                    .text_size(px(18.))
                    .line_height(px(28.))
                    .child(RecordedText::new("中文 mixed words wrap without reflow")),
                publish.clone(),
                true,
            )
            .into_any_element()
        },
    );
    let frame = slot.latest().unwrap();
    let before = geometry(&frame);
    assert!(!before.is_empty());
    assert!(before.iter().any(|(_, wraps, _)| !wraps.is_empty()));
    let preview = frame.clone();
    cx.draw(
        point(px(0.), px(0.)),
        size(px(600.), px(400.)),
        move |_, _| {
            let frame = preview.clone();
            canvas(
                |_, _, _| (),
                move |_, _, window, cx| {
                    let mut allocations = None;
                    for scale in [0.2, 0.65, 1., 0.35, 0.35] {
                        let bounds = Bounds::new(
                            point(px(200.), px(20.)),
                            size(
                                frame.bounds.size.width * scale,
                                frame.bounds.size.height * scale,
                            ),
                        );
                        frame.paint(bounds, window, cx);
                        let current = frame
                            .commands
                            .iter()
                            .filter_map(|command| {
                                let Paint::Wrapped { layout, scaled, .. } = &command.paint else {
                                    return None;
                                };
                                let cache = scaled.borrow();
                                let (_, cached) = cache.as_ref()?;
                                if scale != 1. {
                                    assert_eq!(cached.font_size(), layout.font_size() * scale);
                                    assert_eq!(cached.wrap_boundaries, layout.wrap_boundaries);
                                }
                                Some((
                                    Arc::as_ptr(cached) as usize,
                                    cached
                                        .runs()
                                        .iter()
                                        .map(|run| run.glyphs.as_ptr() as usize)
                                        .collect::<Vec<_>>(),
                                ))
                            })
                            .collect::<Vec<_>>();
                        if let Some(previous) = &allocations {
                            assert_eq!(
                                &current, previous,
                                "animation must reuse its glyph buffers"
                            );
                        }
                        allocations = Some(current);
                    }
                },
            )
            .size_full()
            .into_any_element()
        },
    );
    assert_eq!(geometry(&frame), before);
}

#[gpui::test]
fn a_loading_document_cannot_reuse_another_documents_frame(cx: &mut gpui::TestAppContext) {
    let cx = cx.add_empty_window();
    let slot = FrameSlot::default();
    for ready in [true, false, true] {
        let publish = slot.clone();
        cx.draw(
            point(px(0.), px(0.)),
            size(px(200.), px(100.)),
            move |_, _| {
                DocumentStage::new(
                    div().size_full().child(RecordedText::new(if ready {
                        "Real document"
                    } else {
                        "Preparing Reading…"
                    })),
                    publish.clone(),
                    true,
                )
                .with_ready(ready)
                .into_any_element()
            },
        );
        assert_eq!(slot.latest().is_some(), ready);
        assert!(!recording());
    }
}

#[gpui::test]
fn later_live_frames_do_not_mutate_the_frozen_document(cx: &mut gpui::TestAppContext) {
    let cx = cx.add_empty_window();
    let slot = FrameSlot::default();
    for (text, enabled) in [
        ("Original text", true),
        ("Hidden live update", false),
        ("Updated text", true),
    ] {
        let publish = slot.clone();
        cx.draw(
            point(px(0.), px(0.)),
            size(px(200.), px(100.)),
            move |_, _| {
                DocumentStage::new(
                    div().size_full().child(RecordedText::new(text)),
                    publish.clone(),
                    enabled,
                )
                .into_any_element()
            },
        );
        let frame = slot.latest().unwrap();
        let texts = frame
            .commands
            .iter()
            .filter_map(|c| match &c.paint {
                Paint::Wrapped { line, .. } => Some(line.borrow().text.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if enabled {
            assert!(texts.iter().any(|s| s == text));
        } else {
            assert!(texts.iter().any(|s| s == "Original text"));
        }
        assert!(!recording());
    }
}
