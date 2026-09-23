//! Element prepaint: shapes the visible rows, resolves inline images and
//! block chrome, and publishes render requests to the minimap and syntax
//! services.
use crate::document::DocumentSnapshot;

use super::blocks::prepare_block_chrome;
use super::buttons::{prepare_copy_buttons, prepare_run_buttons};
use super::inline_images::resolve_inline_images;
use super::rows::RowShaping;
use super::*;
use crate::editor::inline_image::{
    INLINE_IMAGE_VERTICAL_PADDING, image_sizing, resolved_image_size,
};

pub(super) fn build_frame(
    host: &gpui::Entity<SemanticEditor>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> PrepaintState {
    #[cfg(feature = "benchmarks")]
    let started_at = Instant::now();
    let snapshot = host.read(cx).snapshot(cx);
    host.update(cx, |editor, _| {
        editor.minimap.update_snapshot(&snapshot);
        editor.minimap.scale_factor = window.scale_factor().max(1.0);
        if editor.minimap.settle_layout_raster_invalidation() {
            window.request_animation_frame();
        }
    });
    let plan = plan_frame(host, &snapshot, bounds, cx);
    let FramePlan {
        gutter_width,
        minimap_full_width,
        minimap_layout_width,
        minimap_visual_width,
        target_wrap_width,
        defer_minimap_reflow,
    } = plan;
    let wrap_width = if defer_minimap_reflow {
        host.read(cx).display_map.wrap_width()
    } else {
        target_wrap_width
    };
    reconfigure_layout(
        host,
        &snapshot,
        target_wrap_width,
        defer_minimap_reflow,
        f32::from(bounds.size.height),
        cx,
    );
    let generated = host.read(cx).generated_highlights.is_some();
    let inline_images = resolve_inline_images(host, &snapshot, bounds, wrap_width, window, cx);
    let editor = host.read(cx);
    let editor_focused = editor.focus_handle.is_focused(window);
    let selection = editor.selection;
    let marked = editor.marked.as_ref().map(|range| range.bytes);
    let scroll_y = editor.scroll_y;
    let visible_lines =
        editor.animated_visible_line_range(&snapshot, scroll_y, f32::from(bounds.size.height));
    let first_line_y = editor.animated_line_start_y(visible_lines.start);
    let fold_animation_active = editor.fold_animation.is_some();
    let (fold_ranges, fold_scale) = editor
        .fold_animation
        .as_ref()
        .map(|animation| (animation.changed_ranges.clone(), animation.scale()))
        .unwrap_or_else(|| (Arc::from([]), 1.0));
    let paint_lines = animated_paint_lines(&editor.display_map, visible_lines, &fold_ranges);
    let editor_path = editor.session.read(cx).syntax_path().to_path_buf();
    let document_format = crate::document::DocumentFormat::from_path(&editor_path);
    let style_query = syntax::SparseEditorStyleSnapshot::query_lines(
        &editor_path,
        &snapshot,
        &paint_lines,
        &editor.syntax_service,
    );
    let syntax_build_request = style_query.start_builder.then(|| {
        (
            editor.syntax_service.clone(),
            editor_path.clone(),
            snapshot.clone(),
        )
    });
    let styles_pending = style_query.pending;
    let style_snapshot = style_query.snapshot;
    debug_assert_eq!(style_snapshot.revision, snapshot.revision());
    let text_origin_x = bounds.left()
        + px(gutter_width
            - if editor.display_map.soft_wrap() {
                0.0
            } else {
                editor.scroll_x
            });
    let theme = current_theme();
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let minimap_layout_preparation = prepare_minimap_layout_request(
        editor,
        &editor_path,
        &snapshot,
        target_wrap_width,
        style.font(),
        font_size,
        cx.text_system().clone(),
    );
    let minimap_bounds = Bounds::new(
        point(bounds.right() - px(minimap_visual_width), bounds.top()),
        size(
            px(if minimap_visual_width > 0.0 {
                minimap_full_width
            } else {
                0.0
            }),
            bounds.size.height,
        ),
    );

    let (
        rows,
        tag_pill_quads,
        swatch_quads,
        hover_quads,
        selection_quads,
        link_hits,
        caret,
        image_resize_handles,
    ) = {
        let mut shaping = RowShaping {
            editor,
            snapshot: &snapshot,
            window,
            cx,
            theme,
            style: &style,
            font_size,
            generated,
            styles_pending,
            style_snapshot: &style_snapshot,
            document_format,
            editor_focused,
            selection,
            marked,
            scroll_y,
            fold_animation_active,
            fold_ranges,
            fold_scale,
            text_origin_x,
            wrap_width,
            inline_images: &inline_images,
            bounds,
            next_y: first_line_y,
            rows: Vec::with_capacity(paint_lines.len()),
            tag_pill_quads: Vec::new(),
            swatch_quads: Vec::new(),
            hover_quads: Vec::new(),
            selection_quads: Vec::new(),
            link_hits: Vec::new(),
            caret: None,
            image_resize_handles: Vec::new(),
        };
        for line_number in paint_lines {
            shaping.shape_row(line_number);
        }
        (
            shaping.rows,
            shaping.tag_pill_quads,
            shaping.swatch_quads,
            shaping.hover_quads,
            shaping.selection_quads,
            shaping.link_hits,
            shaping.caret,
            shaping.image_resize_handles,
        )
    };

    let (_, visible_top, visible_bottom) =
        editor.minimap_source_viewport(f32::from(bounds.size.height));
    let minimap_geometry = editor.minimap_viewport_geometry_for_source_range(
        minimap_bounds,
        visible_top,
        visible_bottom,
    );
    let (minimap, minimap_raster_request) = build_minimap(
        editor,
        editor.session.read(cx).syntax_path(),
        &snapshot,
        styles_pending,
        minimap_bounds,
        minimap_geometry,
        window.scale_factor(),
        theme,
    );
    let schedule_minimap = editor.fold_animation.is_none();
    let source_run_feedback = editor.source_run_feedback;
    let copy_feedback = editor.source_copy.feedback;
    let horizontal_scroll = editor.scroll_x;
    if let Some((service, path, syntax_snapshot)) = syntax_build_request {
        schedule_syntax_builder(host.clone(), service, path, syntax_snapshot, cx);
    }
    if schedule_minimap && let Some(request) = minimap_raster_request {
        schedule_minimap_raster(host.clone(), request, cx);
    }
    if schedule_minimap && let Some(request) = minimap_layout_preparation {
        schedule_minimap_layout_preparation(host.clone(), request, cx);
    }
    let chrome = prepare_block_chrome(
        &rows,
        bounds,
        gutter_width,
        minimap_layout_width,
        horizontal_scroll,
        theme,
    );
    let source_run_buttons = prepare_run_buttons(
        &rows,
        &chrome,
        source_run_feedback,
        theme,
        &style,
        font_size,
        window,
    );
    let source_copy_buttons = prepare_copy_buttons(
        &rows,
        &chrome,
        bounds,
        snapshot.revision(),
        copy_feedback,
        window,
    );
    let block_backgrounds = chrome.backgrounds;
    let block_left = chrome.left;
    PrepaintState {
        #[cfg(feature = "benchmarks")]
        started_at,
        rows,
        block_backgrounds,
        tag_pills: tag_pill_quads,
        swatches: swatch_quads,
        hover_quads,
        link_hits,
        source_run_buttons,
        source_copy_buttons,
        selection: selection_quads,
        caret,
        // Drag grips for the frame's inline images, consumed by paint and by
        // the editor's resize hit-testing.
        image_resize_handles,
        content_left: block_left,
        gutter: fill(
            Bounds::new(
                bounds.origin,
                size(px(gutter_width - 1.0), bounds.size.height),
            ),
            gpui::rgb(theme.background_alt),
        ),
        minimap,
    }
}

/// Frame-level geometry decided before any row is shaped.
struct FramePlan {
    gutter_width: f32,
    minimap_full_width: f32,
    minimap_layout_width: f32,
    minimap_visual_width: f32,
    target_wrap_width: f32,
    defer_minimap_reflow: bool,
}

/// Measures the gutter and wrap target, and flags a deferred reflow when the
/// document is pinned at its end while the width changes.
fn plan_frame(
    host: &gpui::Entity<SemanticEditor>,
    snapshot: &DocumentSnapshot,
    bounds: Bounds<Pixels>,
    cx: &mut App,
) -> FramePlan {
    let digits = snapshot.len_lines().max(1).ilog10() + 1;
    let editor_font_size = host.read(cx).font_size_px();
    let gutter_width = digits as f32 * editor_font_size * 0.6 + GUTTER_PADDING * 2.0;
    let (minimap_full_width, minimap_visible, minimap_reveal) = {
        let editor = host.read(cx);
        (
            editor.minimap.width,
            editor.minimap.visible,
            editor.minimap.reveal,
        )
    };
    let (minimap_layout_width, minimap_visual_width) =
        crate::motion::sliding_panel_widths(minimap_full_width, minimap_visible, minimap_reveal);
    let target_wrap_width =
        (f32::from(bounds.size.width) - gutter_width - minimap_layout_width).max(1.0);
    host.update(cx, |editor, _| {
        let viewport_height = f32::from(bounds.size.height);
        let at_end = editor.scroll_at_end
            || scroll_is_at_end(
                editor.scroll_y,
                viewport_height,
                editor.animated_document_height(),
            );
        let width_changed =
            editor.display_map.wrap_width().to_bits() != target_wrap_width.to_bits();
        if at_end && width_changed && !editor.layout_reflow_pending {
            editor.layout_reflow_pending = true;
            editor.minimap.cancel_layout_preparation();
        }
    });
    let defer_minimap_reflow = {
        let editor = host.read(cx);
        let viewport_height = f32::from(bounds.size.height);
        // At the document end, even a temporary height estimate can move the
        // bottom camera. Keep the current layout authoritative until the
        // complete target-width layout can replace it atomically.
        editor.layout_reflow_pending
            && editor.display_map.wrap_width().to_bits() != target_wrap_width.to_bits()
            && (editor.scroll_at_end
                || scroll_is_at_end(
                    editor.scroll_y,
                    viewport_height,
                    editor.animated_document_height(),
                ))
    };
    FramePlan {
        gutter_width,
        minimap_full_width,
        minimap_layout_width,
        minimap_visual_width,
        target_wrap_width,
        defer_minimap_reflow,
    }
}

/// Publishes the new wrap width to the layout map, keeping the anchored scroll
/// position stable across the reflow.
fn reconfigure_layout(
    host: &gpui::Entity<SemanticEditor>,
    snapshot: &DocumentSnapshot,
    target_wrap_width: f32,
    defer_minimap_reflow: bool,
    viewport_height: f32,
    cx: &mut App,
) {
    host.update(cx, |editor, _| {
        let was_at_end = editor.scroll_at_end
            || scroll_is_at_end(
                editor.scroll_y,
                viewport_height,
                editor.animated_document_height(),
            );
        // Toggling the minimap changes the wrapping width. Keep the old,
        // coherent layout at the document end until the complete replacement
        // arrives, then publish the new width atomically below.
        let anchor_line = editor.animated_line_at_y(editor.scroll_y);
        let anchor_start = editor.animated_line_start_y(anchor_line);
        let anchor_height = editor.animated_line_height_px(anchor_line).max(1.0);
        let anchor_fraction = ((editor.scroll_y - anchor_start) / anchor_height).clamp(0.0, 1.0);
        let layout_reconfigured = !defer_minimap_reflow
            && editor
                .display_map
                .configure_for_resize(snapshot.len_lines(), target_wrap_width);
        if layout_reconfigured {
            editor.layout_reflow_pending = false;
        }
        if layout_reconfigured {
            let inline_image_lines = editor
                .inline_image_line_dimensions
                .borrow()
                .iter()
                .map(|(&line, metrics)| (line, metrics.clone()))
                .collect::<Vec<_>>();
            let sizing = image_sizing(target_wrap_width, viewport_height, editor.font_size_px());
            for (line, metrics) in inline_image_lines {
                if !editor.previews_inline_image_at(ByteOffset(metrics.line_start)) {
                    continue;
                }
                let (_, height) = resolved_image_size(editor, &metrics, &sizing);
                editor.display_map.update_line_layout(
                    line,
                    1,
                    height,
                    INLINE_IMAGE_VERTICAL_PADDING,
                    INLINE_IMAGE_VERTICAL_PADDING,
                );
            }
        }
        let anchored = if layout_reconfigured {
            editor.minimap.invalidate_raster();
            editor.animated_line_start_y(anchor_line)
                + anchor_fraction * editor.animated_line_height_px(anchor_line)
        } else {
            editor.scroll_y
        };
        editor.scroll_y = stabilized_scroll_y(
            was_at_end,
            anchored,
            viewport_height,
            editor.animated_document_height(),
        );
        editor.scroll_at_end = was_at_end;
    });
}

pub(super) fn schedule_syntax_builder(
    host: gpui::Entity<SemanticEditor>,
    service: Arc<syntax::EditorSyntaxService>,
    path: std::path::PathBuf,
    snapshot: crate::document::DocumentSnapshot,
    cx: &mut App,
) {
    let host_id = host.read(cx).minimap.telemetry.host_id();
    let background = cx.background_executor().spawn(async move {
        let _build = tracing::info_span!("editor_semantic_checkpoint_build", host_id).entered();
        service.build_focused(&path, &snapshot);
    });
    cx.spawn(async move |cx| {
        background.await;
        host.update(cx, |_, cx| cx.notify());
    })
    .detach();
}
