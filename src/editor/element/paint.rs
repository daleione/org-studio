//! Element paint: gutter, text layer and the prepared minimap frame, painted
//! inside the element's content mask.
//!
//! The inline-image resize grip is an SVG icon rendered once per process.
use std::sync::{Arc, OnceLock};

use gpui::RenderImage;

use super::minimap::paint_minimap_layer;
use super::quads::table_hover_quads;
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
    let editor_focused = focus_handle.is_focused(window);
    let has_selection = !host.read(cx).selection().is_empty();
    let image_resize = host.read(cx).image_resize;
    let caret_visible = editor_focused && state.caret.is_some();
    let caret_opacity = host.update(cx, |editor, cx| editor.caret_opacity(caret_visible, cx));
    let table_hover = host.read(cx).hovered_table_cell();
    let table_source = table_hover.map(|_| {
        let editor = host.read(cx);
        (
            editor.snapshot(cx),
            crate::document::DocumentFormat::from_path(editor.session.read(cx).syntax_path()),
        )
    });
    window.handle_input(
        &focus_handle,
        ElementInputHandler::new(bounds, host.clone()),
        cx,
    );
    let gutter = state.gutter.clone();
    let gutter_bounds = gutter.bounds;
    let theme = current_theme();
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        crate::document_preview::quad(window, gutter);
        paint_minimap_layer(host, state, window, cx);
        for plot in &mut state.plots {
            plot.paint(window, cx);
        }
        let text_left = state.gutter.bounds.right() + px(1.0);
        for (index, row) in state.rows.iter().enumerate() {
            let number_x = text_left - px(GUTTER_PADDING) - row.gutter_layout.width();
            let active_gutter = row.selected || (row.active && editor_focused && !has_selection);
            let mut paint = |window: &mut Window| {
                if row.selected {
                    let band = Bounds::from_corners(
                        point(gutter_bounds.left(), row.hit.visible_top),
                        point(gutter_bounds.right(), row.hit.visible_bottom),
                    );
                    crate::document_preview::quad(
                        window,
                        fill(band, rgba((theme.accent << 8) | 0x1c)),
                    );
                    crate::document_preview::quad(
                        window,
                        fill(
                            Bounds::new(
                                point(band.right() - px(2.), band.top()),
                                size(px(2.), band.size.height),
                            ),
                            gpui::rgb(theme.accent),
                        ),
                    );
                }
                let _ = crate::document_preview::paint_shaped(
                    &row.gutter_layout,
                    point(number_x, row.hit.origin_y),
                    row.hit.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                if row.visual_rows > 1 {
                    paint_continuation_markers(
                        window,
                        row,
                        number_x,
                        text_left - px(GUTTER_PADDING / 2.0),
                        active_gutter,
                    );
                }
                if row.inline_image.is_some() && row.hit.line_height > px(88.0) {
                    paint_tall_image_marker(
                        window,
                        row,
                        number_x,
                        index.checked_sub(1).and_then(|index| state.rows.get(index)),
                        state.rows.get(index + 1),
                        active_gutter,
                    );
                }
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
        window.with_content_mask(
            Some(ContentMask {
                bounds: gutter_bounds,
            }),
            |window| {
                for button in state
                    .source_run_buttons
                    .iter()
                    .filter(|button| button.hit.visible && button.gutter)
                {
                    paint_run_button(button, window, cx);
                }
            },
        );
        let text_right = if state.minimap.background.is_none() {
            bounds.right()
        } else {
            state.minimap.bounds.left()
        };
        let text_bounds = Bounds::from_corners(
            point(text_left, bounds.top()),
            point(text_right, bounds.bottom()),
        );
        host.update(cx, |editor, _| {
            editor.painted_caret =
                state
                    .caret
                    .clone()
                    .filter(|_| editor_focused)
                    .and_then(|mut caret| {
                        caret.bounds = caret.bounds.intersect(&text_bounds);
                        (caret.bounds.size.width > px(0.) && caret.bounds.size.height > px(0.))
                            .then_some(caret)
                    });
        });
        window.with_content_mask(
            Some(ContentMask {
                bounds: text_bounds,
            }),
            |window| {
                for background in state.rows.iter().filter_map(|row| row.background.clone()) {
                    crate::document_preview::quad(window, background);
                }
                for background in &state.block_backgrounds {
                    crate::document_preview::quad(window, background.clone());
                }
                if let (Some((line, column, header)), Some((snapshot, format))) =
                    (table_hover, table_source.as_ref())
                {
                    for quad in table_hover_quads(
                        state.rows.iter().map(|row| &row.hit),
                        snapshot,
                        *format,
                        line,
                        column,
                        header,
                        crate::theme::current_theme().accent,
                    ) {
                        crate::document_preview::quad(window, quad);
                    }
                }
                for pill in state.tag_pills.drain(..) {
                    crate::document_preview::quad(window, pill);
                }
                for swatch in state.swatches.drain(..) {
                    crate::document_preview::quad(window, swatch);
                }
                for pill in state.hover_quads.drain(..) {
                    crate::document_preview::quad(window, pill);
                }
                for selection in state.selection.drain(..) {
                    crate::document_preview::quad(window, selection);
                }
                for button in state
                    .source_run_buttons
                    .iter()
                    .filter(|button| button.hit.visible && !button.gutter)
                {
                    paint_run_button(button, window, cx);
                }
                for row in &state.rows {
                    let run_button_visible = state
                        .source_run_buttons
                        .iter()
                        .any(|button| button.line == row.hit.line && button.hit.visible);
                    if let Some(number_layout) = &row.source_line_number_layout
                        && !run_button_visible
                    {
                        let number_x = state.content_left
                            + px(row.block.as_ref().map_or(BLOCK_LEFT_INSET, |block| {
                                editor_block_left_inset(&block.kind)
                            }) + SOURCE_GUTTER_INSET)
                            + px(
                                ((SOURCE_GUTTER_WIDTH - f32::from(number_layout.width())) / 2.0)
                                    .max(0.0),
                            );
                        let mut paint_number = |window: &mut Window| {
                            let _ = crate::document_preview::paint_shaped(
                                number_layout,
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
                            let selected = row.active
                                || row.selected
                                || image_resize.is_some_and(|resize| resize.line == row.hit.line.0)
                                || host.read(cx).plots.lines.get(&row.hit.line.0).is_some_and(
                                    |plot| plot.entry.display.lock().unwrap().selected,
                                );
                            let frame = crate::components::image_frame::ImageFrame::new(
                                theme.background,
                                selected.then_some(theme.accent),
                            );
                            let _ =
                                frame.paint_image(preview.bounds, preview.image.clone(), window);
                        } else if let Some(table) = &row.hit.table_layout {
                            for fragment in table.fragments.iter() {
                                let repeats = if fragment.delimiter {
                                    table.visual_rows
                                } else {
                                    1
                                };
                                for visual_row in 0..repeats {
                                    let _ = crate::document_preview::paint_wrapped(
                                        &fragment.layout,
                                        &fragment.preview_runs,
                                        point(
                                            row.hit.text_origin_x + fragment.x,
                                            row.hit.origin_y + row.hit.line_height * visual_row,
                                        ),
                                        row.hit.line_height,
                                        TextAlign::Left,
                                        None,
                                        window,
                                        cx,
                                    );
                                }
                            }
                        } else {
                            let _ = crate::document_preview::paint_wrapped(
                                &row.hit.layout,
                                &row.preview_runs,
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
                for error in &mut state.plot_output_errors {
                    error.paint(window, cx);
                }
                let grip_icon = resize_grip_icon(cx);
                for handle in &state.image_resize_handles {
                    for (axis, _, hitbox) in &handle.edges {
                        if hitbox.is_hovered(window) {
                            window.set_cursor_style(axis.cursor(), hitbox);
                        }
                    }
                    let dragging = host.read(cx).image_resize.is_some_and(|session| {
                        session.plot == handle.plot
                            && (handle.plot.is_some() || session.line_start == handle.line_start)
                    });
                    if handle.hitbox.is_hovered(window) || dragging {
                        let cursor = host
                            .read(cx)
                            .image_resize
                            .filter(|session| session.plot == handle.plot)
                            .map_or(INLINE_IMAGE_RESIZE_CURSOR, |session| session.axis.cursor());
                        window.set_cursor_style(cursor, &handle.hitbox);
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
                        let icon_bounds = handle.bounds.inset(px(1.0));
                        let _ = crate::document_preview::image(
                            window,
                            handle.image_bounds.inset(px(1.0)),
                            icon_bounds,
                            Corners {
                                bottom_right: px(crate::components::image_frame::CONTENT_RADIUS),
                                ..Corners::default()
                            },
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
                    crate::document_preview::quad(window, caret);
                }
            },
        );
    });

    SemanticEditor::capture_table_resize(host, window, cx);
    let editor = host.read(cx);
    if editor.table_resize_is_hovered() {
        window.set_window_cursor_style(CursorStyle::ResizeLeftRight);
    } else if editor.inline_highlight().is_some() || editor.hovered_link.is_some() {
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

fn paint_run_button(button: &SourceRunButtonPaint, window: &mut Window, cx: &mut App) {
    let hovered = button.hitbox.is_hovered(window);
    if hovered {
        window.set_cursor_style(CursorStyle::PointingHand, &button.hitbox);
        let radius = px(5.0);
        crate::document_preview::quad(
            window,
            quad(
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
            ),
        );
    }
    let icon_x = button.bounds.left()
        + px(
            ((f32::from(button.bounds.size.width) - f32::from(button.icon.width())) / 2.0).max(0.0),
        );
    let _ = crate::document_preview::paint_shaped(
        &button.icon,
        point(icon_x, button.bounds.top()),
        button.bounds.size.height,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

/// Connect all wrapped rows to the center of their source line number.
fn paint_continuation_markers(
    window: &mut Window,
    row: &PaintRow,
    number_x: Pixels,
    tip_x: Pixels,
    active: bool,
) {
    let line_height = row.hit.line_height;
    let origin_y = row.hit.origin_y;
    let height = (f32::from(line_height) * 0.64).clamp(8.0, 15.0);
    let stroke = (height * 0.11).clamp(1.2, 1.8) + if active { 0.2 } else { 0.0 };
    let stem_x = number_x + row.gutter_layout.width() / 2.0;
    let bend_offset = (line_height - px(height)) / 2.0 + px(height * 0.72);
    let last_bend_y = origin_y + line_height * (row.visual_rows - 1) + bend_offset;
    let head = px(height * 0.24);
    let mut arrow = gpui::PathBuilder::stroke(px(stroke));
    arrow.move_to(point(stem_x, origin_y + line_height - px(1.0)));
    arrow.line_to(point(stem_x, last_bend_y));
    for visual_row in 1..row.visual_rows {
        let bend_y = origin_y + line_height * visual_row + bend_offset;
        arrow.move_to(point(stem_x, bend_y));
        arrow.line_to(point(tip_x, bend_y));
        arrow.move_to(point(tip_x - head, bend_y - head));
        arrow.line_to(point(tip_x, bend_y));
        arrow.line_to(point(tip_x - head, bend_y + head));
    }
    if let Ok(path) = arrow.build() {
        let theme = current_theme();
        let (color, alpha) = if active {
            (theme.link, 0xcc)
        } else {
            (theme.line_number, 0x99)
        };
        crate::document_preview::path(window, path, rgba((color << 8) | alpha));
    }
}

/// Bracket a tall image row around its centered line number.
fn paint_tall_image_marker(
    window: &mut Window,
    row: &PaintRow,
    number_x: Pixels,
    previous: Option<&PaintRow>,
    next: Option<&PaintRow>,
    active: bool,
) {
    let (number_top, number_bottom) = row.gutter_number_span();
    let gap = px(3.0);
    let top = previous
        .filter(|previous| {
            previous.hit.line.0 + 1 == row.hit.line.0 && previous.inline_image.is_none()
        })
        .map_or(row.hit.visible_top, |previous| {
            previous.gutter_number_span().1 + gap
        });
    let bottom = next
        .filter(|next| next.hit.line.0 == row.hit.line.0 + 1 && next.inline_image.is_none())
        .map_or(row.hit.visible_bottom, |next| {
            next.gutter_number_span().0 - gap
        });
    let x = number_x + row.gutter_layout.width() / 2.0;
    let stroke = px(if active { 1.3 } else { 1.1 });
    let theme = current_theme();
    let (color, alpha) = if active {
        (theme.link, 0xcc)
    } else {
        (theme.line_number, 0x88)
    };
    let color = rgba((color << 8) | alpha);
    for (start, end) in [(top, number_top - gap), (number_bottom + gap, bottom)] {
        if end > start {
            crate::document_preview::quad(
                window,
                fill(
                    Bounds::new(point(x - stroke / 2.0, start), size(stroke, end - start)),
                    color,
                )
                .corner_radii(stroke / 2.0),
            );
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
