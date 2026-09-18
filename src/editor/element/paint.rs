//! Element paint: gutter, text layer and the prepared minimap frame, painted
//! inside the element's content mask.
//!
//! The inline-image resize grip is an SVG icon rendered once per process.
use std::sync::{Arc, OnceLock};

use gpui::RenderImage;

use super::minimap::paint_minimap_layer;
use super::scroll::publish_frame;
use super::*;

pub(super) fn paint_frame(
    host: &gpui::Entity<SemanticEditor>,
    bounds: Bounds<Pixels>,
    state: &mut PrepaintState,
    window: &mut Window,
    cx: &mut App,
) {
    let focus_handle = host.read(cx).focus_handle.clone();
    let caret_visible = focus_handle.is_focused(window) && state.caret.is_some();
    let caret_opacity = host.update(cx, |editor, cx| editor.caret_opacity(caret_visible, cx));
    window.handle_input(
        &focus_handle,
        ElementInputHandler::new(bounds, host.clone()),
        cx,
    );
    let _hits = state
        .rows
        .iter()
        .map(|row| row.hit.clone())
        .collect::<Arc<[HitRow]>>();
    let _source_run_button_hits = state
        .source_run_buttons
        .iter()
        .map(|button| crate::editor::SourceRunButtonHit {
            bounds: button.interaction_bounds,
            source_offset: button.source_offset,
        })
        .collect::<Arc<[_]>>();
    let _shaped = state
        .rows
        .iter()
        .map(|row| (row.shape_key.clone(), row.hit.layout.clone()))
        .collect::<Vec<_>>();
    let gutter = state.gutter.clone();
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        window.paint_quad(gutter);
        paint_minimap_layer(host, state, window, cx);
        let text_left = state.gutter.bounds.right() + px(1.0);
        for row in &state.rows {
            let number_x = text_left - px(GUTTER_PADDING) - row.gutter_layout.width();
            let mut paint = |window: &mut Window| {
                let _ = row.gutter_layout.paint(
                    point(number_x, row.hit.origin_y),
                    row.hit.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            };
            if let Some((top, bottom)) = row.animation_clip_y {
                if bottom > top {
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: Bounds::from_corners(
                                point(bounds.left(), top),
                                point(text_left, bottom),
                            ),
                        }),
                        paint,
                    );
                }
            } else {
                paint(window);
            }
        }
        let text_right = if state.minimap.background.is_none() {
            bounds.right()
        } else {
            state.minimap.bounds.left()
        };
        let text_bounds = Bounds::from_corners(
            point(text_left, bounds.top()),
            point(text_right, bounds.bottom()),
        );
        window.with_content_mask(
            Some(ContentMask {
                bounds: text_bounds,
            }),
            |window| {
                for background in &state.block_backgrounds {
                    window.paint_quad(background.clone());
                }
                for background in state.rows.iter().filter_map(|row| row.background.clone()) {
                    window.paint_quad(background);
                }
                for pill in state.tag_pills.drain(..) {
                    window.paint_quad(pill);
                }
                for swatch in state.swatches.drain(..) {
                    window.paint_quad(swatch);
                }
                for pill in state.hover_quads.drain(..) {
                    window.paint_quad(pill);
                }
                for selection in state.selection.drain(..) {
                    window.paint_quad(selection);
                }
                for button in &state.source_run_buttons {
                    let hovered = button.hitbox.is_hovered(window);
                    if hovered {
                        window.set_cursor_style(CursorStyle::PointingHand, &button.hitbox);
                        let radius = px(5.0);
                        window.paint_quad(quad(
                            button.bounds,
                            Corners {
                                top_left: radius,
                                top_right: radius,
                                bottom_right: radius,
                                bottom_left: radius,
                            },
                            rgba((button.accent << 8) | 0x1f),
                            Edges::default(),
                            rgba(0),
                            BorderStyle::default(),
                        ));
                    }
                    let icon_x = button.bounds.left()
                        + px(
                            ((SOURCE_RUN_BUTTON_SIZE - f32::from(button.icon.width())) / 2.0)
                                .max(0.0),
                        );
                    let _ = button.icon.paint(
                        point(icon_x, button.bounds.top()),
                        button.bounds.size.height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
                for row in &state.rows {
                    if let Some(number_layout) = &row.source_line_number_layout {
                        let number_x = state.content_left
                            + px(BLOCK_LEFT_INSET + SOURCE_GUTTER_INSET)
                            + px(
                                ((SOURCE_GUTTER_WIDTH - f32::from(number_layout.width())) / 2.0)
                                    .max(0.0),
                            );
                        let mut paint_number = |window: &mut Window| {
                            let _ = number_layout.paint(
                                point(number_x, row.hit.origin_y),
                                row.hit.line_height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        };
                        if let Some((top, bottom)) = row.animation_clip_y {
                            if bottom > top {
                                window.with_content_mask(
                                    Some(ContentMask {
                                        bounds: Bounds::from_corners(
                                            point(text_bounds.left(), top),
                                            point(text_bounds.right(), bottom),
                                        ),
                                    }),
                                    paint_number,
                                );
                            }
                        } else {
                            paint_number(window);
                        }
                    }
                    let mut paint = |window: &mut Window| {
                        if let Some(preview) = &row.inline_image {
                            let radius = px(4.0);
                            let _ = window.paint_image(
                                preview.bounds,
                                preview.bounds,
                                Corners {
                                    top_left: radius,
                                    top_right: radius,
                                    bottom_right: radius,
                                    bottom_left: radius,
                                },
                                preview.image.clone(),
                                0,
                                false,
                            );
                        } else if let Some(table) = &row.hit.table_layout {
                            for fragment in table.fragments.iter() {
                                let _ = fragment.layout.paint(
                                    point(row.hit.text_origin_x + fragment.x, row.hit.origin_y),
                                    row.hit.line_height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        } else {
                            let _ = row.hit.layout.paint(
                                point(row.hit.text_origin_x, row.hit.origin_y),
                                row.hit.line_height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                    };
                    if let Some((top, bottom)) = row.animation_clip_y {
                        if bottom > top {
                            window.with_content_mask(
                                Some(ContentMask {
                                    bounds: Bounds::from_corners(
                                        point(text_bounds.left(), top),
                                        point(text_bounds.right(), bottom),
                                    ),
                                }),
                                paint,
                            );
                        }
                    } else {
                        paint(window);
                    }
                }
                let grip_icon = resize_grip_icon(cx);
                for handle in &state.image_resize_handles {
                    let dragging = host
                        .read(cx)
                        .inline_image_resize_width(handle.line_start)
                        .is_some();
                    if handle.hitbox.is_hovered(window) || dragging {
                        window.set_cursor_style(INLINE_IMAGE_RESIZE_CURSOR, &handle.hitbox);
                    }
                    // The icon only appears while the pointer is over the image (or
                    // while dragging it).
                    if !(dragging
                        || handle.image_hitbox.is_hovered(window)
                        || handle.hitbox.is_hovered(window))
                    {
                        continue;
                    }
                    if let Some(icon) = grip_icon.as_ref() {
                        let _ = window.paint_image(
                            handle.bounds,
                            handle.bounds,
                            Corners::default(),
                            icon.clone(),
                            0,
                            false,
                        );
                    }
                }
                for button in &state.source_copy_buttons {
                    button.paint(window, cx);
                }
                if focus_handle.is_focused(window)
                    && let Some(mut caret) = state.caret.take()
                {
                    caret.background = caret.background.opacity(caret_opacity);
                    window.paint_quad(caret);
                }
            },
        );
    });

    if host.read(cx).inline_highlight().is_some() {
        window.set_window_cursor_style(CursorStyle::PointingHand);
    }
    let _benchmark = publish_frame(host, bounds, state, cx);
    #[cfg(feature = "benchmarks")]
    match _benchmark.expect("benchmark builds always return a frame action") {
        FrameBenchmarkAction::Inactive => {}
        FrameBenchmarkAction::Continue => {
            let executor = cx.background_executor().clone();
            window
                .spawn(cx, async move |cx| {
                    executor.timer(Duration::from_millis(1)).await;
                    let _ = cx.update(|window, _| {
                        window.activate_window();
                    });
                    cx.refresh();
                })
                .detach();
        }
        FrameBenchmarkAction::Complete => {
            if std::env::var_os("ORG_STUDIO_EXIT_AFTER_EDITOR_BENCH").is_some() {
                cx.quit();
            }
        }
    }
}

/// Rounded resize grip drawn at the bottom-right corner of a resizable image.
const RESIZE_GRIP_SVG: &[u8] = include_bytes!("../../../assets/editor/resize.svg");

/// The grip icon, rasterized from the bundled SVG once per process.
fn resize_grip_icon(cx: &mut App) -> Option<Arc<RenderImage>> {
    static ICON: OnceLock<Option<Arc<RenderImage>>> = OnceLock::new();
    ICON.get_or_init(|| {
        crate::editor::image_loader::decode_svg(RESIZE_GRIP_SVG, &cx.svg_renderer())
            .ok()
            .map(|loaded| loaded.image)
    })
    .clone()
}
