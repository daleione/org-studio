#[cfg(test)]
use gpui::{Bounds, point};
use gpui::{ListOffset, ListState, px};

use super::{MIN_THUMB_PX, MinimapDensity, MinimapDragSession, MinimapLineIndex};
#[cfg(test)]
use super::{MINIMAP_EDGE_PADDING_PX, MINIMAP_LINE_HEIGHT_PX, PREVIEW_BASE_ROW_PX};
use crate::preview::{
    coordinates::{Bias, SourcePoint},
    projection::{PreviewProjectionSnapshot, VisualRowId},
};

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg(test)]
pub(in crate::preview) struct MinimapHitRow {
    pub(in crate::preview) top: f32,
    pub(in crate::preview) bottom: f32,
    pub(in crate::preview) presentation_index: usize,
    pub(in crate::preview) offset_in_item: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::preview) struct MinimapViewport {
    pub(in crate::preview) content_top: f32,
    pub(in crate::preview) thumb: ThumbGeometry,
    pub(in crate::preview) scroll_ratio: f32,
    pub(in crate::preview) interaction_height: f32,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::preview) struct MinimapInteractionAnchor {
    pub(in crate::preview) layout: crate::preview::layout::LayoutKey,
    pub(in crate::preview) width: u16,
    pub(in crate::preview) rows_signature: u64,
    pub(in crate::preview) interaction_height: f32,
    pub(in crate::preview) content_top: f32,
}

impl MinimapInteractionAnchor {
    pub(in crate::preview) fn matches(
        self,
        index: &MinimapLineIndex,
        interaction_height: f32,
    ) -> bool {
        self.layout == index.layout
            && self.width == index.width
            && self.rows_signature == index.rows_signature
            && (self.interaction_height - interaction_height).abs() < 0.5
    }
}

pub(in crate::preview) fn minimap_projection_height(
    total_lines: usize,
    track_height: f32,
    density: MinimapDensity,
) -> f32 {
    (total_lines as f32 * density.line_height() + density.edge_padding() * 2.0)
        .min(track_height)
        .max(0.0)
}

pub(in crate::preview) fn minimap_visible_display_range(
    index: &MinimapLineIndex,
    scroll_pixels: f32,
    viewport_pixels: f32,
) -> (f32, f32) {
    let document_pixels = index.document_pixels();
    let top = index.display_position_for_pixel(scroll_pixels);
    let bottom = index
        .display_position_for_pixel((scroll_pixels + viewport_pixels).clamp(0.0, document_pixels));
    (top, bottom.max(top))
}

pub(in crate::preview) fn minimap_thumb_height_for_scroll(
    index: &MinimapLineIndex,
    scroll_pixels: f32,
    viewport_pixels: f32,
    interaction_height: f32,
) -> f32 {
    let document_pixels = index.document_pixels();
    if document_pixels <= viewport_pixels || document_pixels <= f32::EPSILON {
        return interaction_height;
    }
    let (top, bottom) = minimap_visible_display_range(index, scroll_pixels, viewport_pixels);
    ((bottom - top) * index.density.line_height())
        .max(MIN_THUMB_PX)
        .min(interaction_height)
}

#[cfg(test)]
pub(in crate::preview) fn minimap_viewport(
    metrics: ScrollMetrics,
    total_lines: usize,
    track_height: f32,
) -> MinimapViewport {
    if total_lines == 0 || track_height <= 0.0 {
        return MinimapViewport::default();
    }
    let total = total_lines as f32;
    let visible_editor_lines = (metrics.viewport / PREVIEW_BASE_ROW_PX).max(1.0).min(total);
    let visible_minimap_lines = (track_height / MINIMAP_LINE_HEIGHT_PX).max(1.0);
    let progress = if metrics.max_offset > 0.0 {
        (metrics.offset / metrics.max_offset).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let editor_scroll_top = progress * (total - visible_editor_lines).max(0.0);
    let content_top = progress * (total - visible_minimap_lines).max(0.0);
    let height = (visible_editor_lines * MINIMAP_LINE_HEIGHT_PX)
        .max(MIN_THUMB_PX)
        .min(track_height);
    let top = ((editor_scroll_top - content_top) * MINIMAP_LINE_HEIGHT_PX)
        .clamp(0.0, (track_height - height).max(0.0));
    MinimapViewport {
        content_top,
        thumb: ThumbGeometry { top, height },
        scroll_ratio: progress,
        interaction_height: track_height,
    }
}

pub(in crate::preview) fn minimap_viewport_for_list(
    index: &MinimapLineIndex,
    list_state: &ListState,
    track_height: f32,
) -> MinimapViewport {
    if index.total == 0 || track_height <= 0.0 {
        return MinimapViewport::default();
    }
    let interaction_height = minimap_projection_height(index.total, track_height, index.density);
    let viewport_pixels = f32::from(list_state.viewport_bounds().size.height).max(0.0);
    let total = index.total as f32;
    let document_pixels = index.document_pixels();
    let max_scroll_pixels = (document_pixels - viewport_pixels).max(0.0);
    let scroll_pixels = index
        .pixel_for_list_offset(list_state.logical_scroll_top())
        .clamp(0.0, max_scroll_pixels);
    let progress = if max_scroll_pixels > 0.0 {
        scroll_pixels / max_scroll_pixels
    } else {
        0.0
    };
    let (editor_top, editor_bottom) =
        minimap_visible_display_range(index, scroll_pixels, viewport_pixels);
    let shared = crate::minimap::projection_viewport(
        total,
        editor_top,
        editor_bottom,
        progress,
        track_height,
        index.density,
    );
    debug_assert!((shared.interaction_height - interaction_height).abs() < 0.01);
    MinimapViewport {
        content_top: shared.content_top,
        thumb: ThumbGeometry {
            top: shared.thumb_top,
            height: shared.thumb_height,
        },
        scroll_ratio: shared.scroll_ratio,
        interaction_height: shared.interaction_height,
    }
}

pub(in crate::preview) fn minimap_viewport_for_list_with_anchor(
    index: &MinimapLineIndex,
    list_state: &ListState,
    track_height: f32,
    anchor: Option<MinimapInteractionAnchor>,
) -> MinimapViewport {
    let mut viewport = minimap_viewport_for_list(index, list_state, track_height);
    if let Some(anchor) = anchor.filter(|anchor| anchor.matches(index, viewport.interaction_height))
    {
        let viewport_pixels = f32::from(list_state.viewport_bounds().size.height).max(0.0);
        let document_pixels = index.document_pixels();
        let max_scroll_pixels = (document_pixels - viewport_pixels).max(0.0);
        let scroll_pixels = index
            .pixel_for_list_offset(list_state.logical_scroll_top())
            .clamp(0.0, max_scroll_pixels);
        let (editor_top, editor_bottom) =
            minimap_visible_display_range(index, scroll_pixels, viewport_pixels);
        let shared = crate::minimap::stabilize_projection_camera(
            crate::minimap::ProjectionViewport {
                content_top: viewport.content_top,
                thumb_top: viewport.thumb.top,
                thumb_height: viewport.thumb.height,
                interaction_height: viewport.interaction_height,
                scroll_ratio: viewport.scroll_ratio,
            },
            index.total as f32,
            editor_top,
            editor_bottom,
            index.density,
            anchor.content_top,
        );
        viewport.content_top = shared.content_top;
        viewport.thumb.top = shared.thumb_top;
    }
    viewport
}

pub(in crate::preview) fn minimap_anchor_for_thumb_top(
    index: &MinimapLineIndex,
    list_state: &ListState,
    track_height: f32,
    thumb_top: f32,
) -> MinimapInteractionAnchor {
    // Wrapped rows do not have a uniform preview-pixel/display-line ratio. Choose the
    // minimap camera that makes the settled, projection-derived thumb meet the dragged
    // position, so dropping the transient drag overlay cannot make it jump.
    let viewport = minimap_viewport_for_list(index, list_state, track_height);
    let visible_minimap_lines = ((viewport.interaction_height
        - index.density.edge_padding() * 2.0)
        / index.density.line_height())
    .max(1.0);
    let max_content_top = (index.total as f32 - visible_minimap_lines).max(0.0);
    let viewport_pixels = f32::from(list_state.viewport_bounds().size.height).max(0.0);
    let document_pixels = index.document_pixels();
    let max_scroll_pixels = (document_pixels - viewport_pixels).max(0.0);
    let scroll_pixels = index
        .pixel_for_list_offset(list_state.logical_scroll_top())
        .clamp(0.0, max_scroll_pixels);
    let (editor_top, editor_bottom) =
        minimap_visible_display_range(index, scroll_pixels, viewport_pixels);
    let minimum_content_top = (editor_bottom - visible_minimap_lines)
        .max(0.0)
        .min(max_content_top);
    let maximum_content_top = editor_top.max(minimum_content_top).min(max_content_top);
    let desired_content_top = editor_top - (thumb_top / index.density.line_height()).max(0.0);

    MinimapInteractionAnchor {
        layout: index.layout,
        width: index.width,
        rows_signature: index.rows_signature,
        interaction_height: viewport.interaction_height,
        content_top: desired_content_top.clamp(minimum_content_top, maximum_content_top),
    }
}

pub(in crate::preview) fn minimap_thumb_for_drag(
    viewport: MinimapViewport,
    session: Option<MinimapDragSession>,
) -> ThumbGeometry {
    let mut thumb = viewport.thumb;
    if let Some(session) = session {
        // During a drag the pointer is the visual source of truth. Deriving `top` again
        // from scroll progress would lag or stick whenever wrapped-row heights vary.
        thumb.top = session
            .current_thumb_top
            .clamp(0.0, (viewport.interaction_height - thumb.height).max(0.0));
    }
    thumb
}

#[derive(Clone, Copy, Debug)]
pub(in crate::preview) struct MinimapClickTarget {
    pub(in crate::preview) offset: ListOffset,
    pub(in crate::preview) ratio: f32,
    pub(in crate::preview) clicked_display: f32,
    pub(in crate::preview) thumb_top: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::preview) struct MinimapSourceTarget {
    pub(in crate::preview) point: SourcePoint,
    pub(in crate::preview) row: VisualRowId,
    pub(in crate::preview) bias: Bias,
}

pub(in crate::preview) fn source_target_for_list_offset(
    projection: &PreviewProjectionSnapshot,
    presentation_rows: &[usize],
    offset: ListOffset,
    bias: Bias,
) -> Option<MinimapSourceTarget> {
    let document_row = *presentation_rows.get(offset.item_ix)?;
    let visual = projection.rows.get(document_row)?;
    let source = projection.source_row(document_row)?.content.range;
    let source_point = SourcePoint {
        revision: projection.revision,
        offset: source.start,
    };
    let visual_point = projection.source_to_visual(source_point, bias).ok()?;
    let point = projection.visual_to_source(visual_point).ok()?;
    Some(MinimapSourceTarget {
        point,
        row: visual.id,
        bias,
    })
}

pub(in crate::preview) fn minimap_click_target_for_viewport(
    index: &MinimapLineIndex,
    list_state: &ListState,
    viewport: MinimapViewport,
    local_y: f32,
) -> MinimapClickTarget {
    let clicked_display = (viewport.content_top
        + ((local_y - index.density.edge_padding()) / index.density.line_height()).max(0.0))
    .clamp(0.0, index.total as f32);
    let clicked_offset = index.list_offset_for_display_position(clicked_display);
    let viewport_pixels = f32::from(list_state.viewport_bounds().size.height).max(0.0);
    let document_pixels = index.document_pixels();
    let max_scroll_pixels = (document_pixels - viewport_pixels).max(0.0);
    let target_pixels = (index.pixel_for_list_offset(clicked_offset) - viewport_pixels * 0.5)
        .clamp(0.0, max_scroll_pixels);
    let ratio = if max_scroll_pixels <= f32::EPSILON {
        0.0
    } else {
        target_pixels / max_scroll_pixels
    };
    let target_thumb_height = minimap_thumb_height_for_scroll(
        index,
        target_pixels,
        viewport_pixels,
        viewport.interaction_height,
    );
    MinimapClickTarget {
        offset: index.list_offset_for_pixel(target_pixels),
        ratio,
        clicked_display,
        thumb_top: (local_y - target_thumb_height * 0.5).clamp(
            0.0,
            (viewport.interaction_height - target_thumb_height).max(0.0),
        ),
    }
}

pub(in crate::preview) fn minimap_drag_target(
    local_y: f32,
    session: MinimapDragSession,
    thumb_height: f32,
    track_height: f32,
) -> (f32, f32) {
    crate::minimap::drag_target(local_y, session, thumb_height, track_height)
}

pub(in crate::preview) fn scroll_ratio_after_wheel(
    index: &MinimapLineIndex,
    list_state: &ListState,
    wheel_delta_y: f32,
) -> f32 {
    let viewport_pixels = f32::from(list_state.viewport_bounds().size.height).max(0.0);
    let document_pixels = index.document_pixels();
    let max_scroll_pixels = (document_pixels - viewport_pixels).max(0.0);
    if max_scroll_pixels <= f32::EPSILON {
        return 0.0;
    }
    let current = index.pixel_for_list_offset(list_state.logical_scroll_top());
    // GPUI wheel deltas describe content movement; document scroll is the inverse.
    (current - wheel_delta_y).clamp(0.0, max_scroll_pixels) / max_scroll_pixels
}

pub(in crate::preview) fn scroll_list_to_ratio(
    index: &MinimapLineIndex,
    list_state: &ListState,
    ratio: f32,
) {
    let ratio = ratio.clamp(0.0, 1.0);
    if ratio >= 1.0 - f32::EPSILON && list_state.item_count() > 0 {
        list_state.scroll_to(ListOffset {
            item_ix: list_state.item_count() - 1,
            offset_in_item: px(0.0),
        });
    } else {
        let viewport_pixels = f32::from(list_state.viewport_bounds().size.height).max(0.0);
        let document_pixels = index.document_pixels();
        let max_scroll_pixels = (document_pixels - viewport_pixels).max(0.0);
        list_state.scroll_to(index.list_offset_for_pixel(ratio * max_scroll_pixels));
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::preview) struct ThumbGeometry {
    pub top: f32,
    pub height: f32,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::preview) struct MinimapLayout {
    pub(in crate::preview) first_line: usize,
    pub(in crate::preview) thumb: ThumbGeometry,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::preview) struct ScrollMetrics {
    pub(in crate::preview) offset: f32,
    pub(in crate::preview) max_offset: f32,
    pub(in crate::preview) viewport: f32,
}

#[cfg(test)]
pub(in crate::preview) fn hit_test_minimap_row(
    rows: &[MinimapHitRow],
    y: f32,
) -> Option<MinimapHitRow> {
    rows.iter()
        .find(|row| y >= row.top && y < row.bottom)
        .copied()
}

pub(in crate::preview) use crate::minimap::thumb_alphas;

#[cfg(test)]
pub(in crate::preview) fn local_y_ratio(
    pointer_y: gpui::Pixels,
    bounds: Bounds<gpui::Pixels>,
) -> f32 {
    let height = f32::from(bounds.size.height);
    if height <= 0.0 {
        0.0
    } else {
        (f32::from(pointer_y - bounds.origin.y) / height).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
pub(in crate::preview) fn seek_to_ratio(list_state: &ListState, ratio: f32, center: bool) {
    let metrics = scroll_metrics(list_state);
    let mut offset = ratio.clamp(0.0, 1.0) * metrics.max_offset;
    if center {
        offset = (offset - metrics.viewport * 0.5).clamp(0.0, metrics.max_offset);
    }
    // Use the list's scrollbar protocol. It maps the exact measured/estimated
    // SumTree height to a ListOffset and honors the height frozen at drag start.
    list_state.set_offset_from_scrollbar(point(px(0.0), px(-offset)));
}

#[cfg(test)]
pub(in crate::preview) fn thumb_geometry(
    list_state: &ListState,
    total_lines: usize,
    track_height: f32,
) -> ThumbGeometry {
    let metrics = scroll_metrics(list_state);
    thumb_geometry_for_document(metrics, total_lines, track_height)
}

#[cfg(test)]
pub(in crate::preview) fn minimap_layout_from_metrics(
    total_lines: usize,
    minimap_capacity: usize,
    track_height: f32,
    metrics: ScrollMetrics,
) -> MinimapLayout {
    if total_lines == 0 || track_height <= 0.0 {
        return MinimapLayout::default();
    }
    let progress = if metrics.max_offset > 0.0 {
        (metrics.offset / metrics.max_offset).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let first_line =
        (progress * total_lines.saturating_sub(minimap_capacity) as f32).round() as usize;
    MinimapLayout {
        first_line,
        thumb: thumb_geometry_for_document(metrics, total_lines, track_height),
    }
}

#[cfg(test)]
pub(in crate::preview) fn scroll_metrics(list_state: &ListState) -> ScrollMetrics {
    ScrollMetrics {
        offset: -f32::from(list_state.scroll_px_offset_for_scrollbar().y),
        max_offset: f32::from(list_state.max_offset_for_scrollbar().y).max(0.0),
        viewport: f32::from(list_state.viewport_bounds().size.height).max(0.0),
    }
}

#[cfg(test)]
pub(in crate::preview) fn thumb_geometry_for_document(
    metrics: ScrollMetrics,
    total_lines: usize,
    track_height: f32,
) -> ThumbGeometry {
    minimap_viewport(metrics, total_lines, track_height).thumb
}

#[cfg(test)]
pub(in crate::preview) fn minimap_layout_from_range(
    total_lines: usize,
    visible_start: usize,
    visible_end: usize,
    minimap_capacity: usize,
    track_height: f32,
) -> MinimapLayout {
    if total_lines == 0 || track_height <= 0.0 {
        return MinimapLayout::default();
    }
    let start = visible_start.min(total_lines - 1);
    let end = visible_end.max(start + 1).min(total_lines);
    let visible_lines = end - start;
    let non_visible_lines = total_lines.saturating_sub(visible_lines);
    let scroll_progress = if non_visible_lines > 0 {
        (start as f32 / non_visible_lines as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let first_line =
        (scroll_progress * total_lines.saturating_sub(minimap_capacity) as f32).round() as usize;
    let raw_top =
        MINIMAP_EDGE_PADDING_PX + start.saturating_sub(first_line) as f32 * MINIMAP_LINE_HEIGHT_PX;
    let raw_height = visible_lines as f32 * MINIMAP_LINE_HEIGHT_PX;
    let height = raw_height.max(MIN_THUMB_PX).min(track_height);
    let top = raw_top.min((track_height - height).max(0.0));
    MinimapLayout {
        first_line,
        thumb: ThumbGeometry { top, height },
    }
}

#[cfg(test)]
pub(in crate::preview) fn thumb_geometry_from_metrics(
    scroll: f32,
    max_offset: f32,
    viewport: f32,
    track_height: f32,
) -> ThumbGeometry {
    let track_height = track_height.max(0.0);
    let max_offset = max_offset.max(0.0);
    let viewport = viewport.max(0.0);
    let content = max_offset + viewport;
    if content <= 0.0 || track_height <= 0.0 {
        return ThumbGeometry::default();
    }
    let scroll = scroll.clamp(0.0, max_offset);
    let raw_height = track_height * viewport / content;
    let height = raw_height.max(MIN_THUMB_PX).min(track_height);
    let travel = (track_height - height).max(0.0);
    let top = if max_offset > 0.0 {
        travel * scroll / max_offset
    } else {
        0.0
    };
    ThumbGeometry { top, height }
}
