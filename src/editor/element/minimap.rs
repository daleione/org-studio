//! Editor-side minimap frame builder: source geometry projection and the
//! `MemapPaint` frame (background, raster image, media, overlays) handed to
//! the paint pass.

use std::sync::Arc;

use gpui::{
    App, BorderStyle, Bounds, Corners, PaintQuad, Pixels, RenderImage, Window, fill, outline,
    point, px, rgba, size,
};

use crate::document::{LineIndex, TextSnapshot};
use crate::editor::{
    SemanticEditor, layout_map::EditorLayoutMap, minimap_media::MinimapImagePaint,
};

use crate::editor::minimap_media::geometry as editor_minimap_media_geometry;

use super::PrepaintState;
use super::minimap_raster::MinimapRasterRequest;
use super::rows::visible_source_lines;

pub(super) struct MinimapPaint {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) background: Option<PaintQuad>,
    pub(super) image: Option<(Arc<RenderImage>, Bounds<Pixels>)>,
    pub(super) media: Vec<MinimapImagePaint>,
    pub(super) overlays: Vec<PaintQuad>,
}

#[derive(Clone, Copy)]
pub(super) struct MinimapMediaCandidate {
    pub(super) line: u64,
    pub(super) line_start: u64,
    pub(super) row_offset_units: f32,
    pub(super) row_height_units: f32,
}

pub(super) fn minimap_geometry_for_layout(
    editor: &SemanticEditor,
    layout: &EditorLayoutMap,
    viewport_height: f32,
    density: crate::minimap::Density,
    line_height: f32,
) -> crate::editor::minimap::ViewportGeometry {
    let viewport = crate::editor::minimap::source_viewport_for_layout(
        layout,
        &editor.display_map,
        editor.scroll_y,
        editor.animated_document_height(),
        viewport_height,
    );
    crate::minimap::projection_viewport_with_line_height(
        viewport.total_units,
        viewport.visible_top,
        viewport.visible_bottom,
        viewport.scroll_ratio,
        viewport_height,
        density,
        line_height,
    )
}

#[cfg(feature = "benchmarks")]
pub(super) fn minimap_camera_source(layout: &EditorLayoutMap, content_top: f32) -> f64 {
    let y = content_top.max(0.0) * layout.base_line_height().max(1.0);
    let line = layout.line_at_y(y);
    let fraction =
        ((y - layout.line_start_y(line)) / layout.line_height_px(line).max(1.0)).clamp(0.0, 1.0);
    line as f64 + f64::from(fraction)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_minimap(
    editor: &SemanticEditor,
    path: &std::path::Path,
    snapshot: &crate::document::DocumentSnapshot,
    semantics_pending: bool,
    bounds: Bounds<Pixels>,
    geometry: crate::editor::minimap::ViewportGeometry,
    scale_factor: f32,
    theme: &crate::theme::Theme,
) -> (MinimapPaint, Option<MinimapRasterRequest>) {
    editor.minimap.note_semantics_pending(semantics_pending);
    if f32::from(bounds.size.width) <= 0.0 {
        return (
            MinimapPaint {
                bounds,
                background: None,
                image: None,
                media: Vec::new(),
                overlays: Vec::new(),
            },
            None,
        );
    }
    let background = fill(bounds, rgba((theme.background_alt << 8) | 0xff));
    let mut overlays = vec![fill(
        Bounds::new(bounds.origin, size(px(1.0), bounds.size.height)),
        gpui::rgb(theme.border),
    )];
    let density = crate::minimap::Density::for_width(f32::from(bounds.size.width));
    let line_height = editor.minimap.line_height(density);
    let active_frame = editor.minimap.active_frame();
    let minimap_layout = active_frame
        .as_ref()
        .map_or(&editor.display_map, |frame| frame.layout.as_ref());
    let cached_raster = active_frame.as_ref().map(|frame| frame.raster.clone());
    let request_layout = editor.minimap.prepared_layout().unwrap_or_else(|| {
        active_frame
            .as_ref()
            .filter(|frame| frame.raster.key.generation == editor.minimap.generation)
            .map_or_else(
                || Arc::new(editor.display_map.clone()),
                |frame| frame.layout.clone(),
            )
    });
    let request_geometry = if std::ptr::eq(request_layout.as_ref(), minimap_layout) {
        geometry
    } else {
        minimap_geometry_for_layout(
            editor,
            request_layout.as_ref(),
            f32::from(bounds.size.height),
            density,
            line_height,
        )
    };
    let editor_line_height = request_layout.base_line_height().max(1.0);
    let total_units = (request_layout.total_height() / editor_line_height).max(1.0);
    let (first_unit, row_count) = crate::editor::minimap::raster_window(
        request_geometry.content_top,
        request_geometry.interaction_height,
        total_units,
        density,
        line_height,
    );
    let key = crate::editor::minimap::RasterKey {
        generation: editor.minimap.generation,
        first_unit,
        rows: row_count.min(u16::MAX as usize) as u16,
        width: f32::from(bounds.size.width).ceil().min(u16::MAX as f32) as u16,
        scale_x100: (scale_factor.max(1.0) * 100.0).round().min(u16::MAX as f32) as u16,
        density,
        theme_generation: crate::theme::theme_generation(),
    };
    #[cfg(feature = "benchmarks")]
    {
        let active_total_units =
            minimap_layout.total_height() / minimap_layout.base_line_height().max(1.0);
        let visible_minimap_units = ((geometry.interaction_height - density.edge_padding() * 2.0)
            / line_height.max(1.0))
        .max(1.0);
        let max_content_top = (active_total_units - visible_minimap_units).max(0.0);
        let camera_margin = 2.0;
        editor.minimap.telemetry.note_camera(
            editor.scroll_y,
            geometry.content_top,
            minimap_camera_source(minimap_layout, geometry.content_top),
            geometry.thumb_top,
            editor.scroll_y <= 0.5
                || editor.scroll_y + f32::from(bounds.size.height) + 0.5
                    >= editor.animated_document_height(),
            geometry.content_top > camera_margin
                && geometry.content_top < max_content_top - camera_margin,
            editor.minimap.viewport_generation(),
            active_frame
                .as_ref()
                .map_or(editor.minimap.generation, |frame| frame.geometry_identity()),
            cached_raster.as_ref().map(|cached| cached.key.first_unit),
            cached_raster
                .as_ref()
                .map(|cached| cached.viewport_generation),
        );
    }
    let cached_matches = cached_raster
        .as_ref()
        .is_some_and(|cached| cached.key == key);
    editor.minimap.note_cache_lookup(cached_matches);
    let defer_layout_refinement = active_frame.is_some()
        && !editor.minimap.background_camera_movable()
        && editor
            .minimap
            .is_layout_refinement_generation(key.generation);
    // Editing a table can change its byte ranges and wrap boundaries together.
    // Do not briefly replace the old image with new text shaped against the live,
    // partially measured layout while complete geometry is still being prepared.
    // With no previous image, retain the existing fast first-frame path.
    let awaiting_layout = active_frame.is_some() && editor.minimap.layout_preparation_pending();
    let mut raster_request = None;
    if !semantics_pending
        && !awaiting_layout
        && !cached_matches
        && !defer_layout_refinement
        && editor.minimap.reserve_raster(key)
    {
        let first_y = first_unit as f32 * editor_line_height;
        let end_y = (first_unit as f32 + row_count as f32) * editor_line_height;
        let first_line = request_layout.line_at_y(first_y);
        let last_line = request_layout
            .line_at_y(end_y.min(request_layout.total_height()))
            .saturating_add(1)
            .min(snapshot.len_lines());
        let source_lines = visible_source_lines(request_layout.as_ref(), first_line..last_line);
        let mut raster_lines = vec![None; row_count];
        let mut media_candidates = Vec::with_capacity(source_lines.len());
        for &line in &source_lines {
            let start_units = request_layout.line_start_y(line) / editor_line_height;
            let height_units = request_layout.line_height_px(line) / editor_line_height;
            let row_offset_units = start_units - first_unit as f32;
            crate::editor::minimap::fill_visual_rows(
                &mut raster_lines,
                line,
                row_offset_units,
                height_units,
                request_layout.line_wrap_starts(line),
            );
            if let Ok(range) = snapshot.line_content_range(LineIndex(line))
                && editor.previews_inline_image_at(range.start)
            {
                media_candidates.push(MinimapMediaCandidate {
                    line,
                    line_start: range.start.0,
                    row_offset_units,
                    row_height_units: height_units,
                });
            }
        }
        let visible_rows = (((request_geometry.interaction_height - density.edge_padding() * 2.0)
            / line_height.max(1.0))
        .ceil() as usize
            + 1)
        .min(raster_lines.len());
        let repeated_rows = editor
            .minimap
            .telemetry
            .note_request(snapshot.revision(), &source_lines);
        raster_request = Some(MinimapRasterRequest {
            key,
            layout: request_layout,
            path: path.to_path_buf(),
            snapshot: snapshot.clone(),
            source_lines,
            raster_lines,
            media_candidates,
            syntax_service: editor.syntax_service.clone(),
            theme: *theme,
            content_top: request_geometry.content_top,
            viewport_generation: editor.minimap.viewport_generation(),
            scale_factor,
            density,
            line_height,
            visible_rows,
            repeated_rows,
            telemetry: editor.minimap.telemetry.clone(),
            epoch: editor.minimap.raster_epoch.clone(),
            expected_epoch: editor
                .minimap
                .raster_epoch
                .load(std::sync::atomic::Ordering::Acquire),
        });
    }
    let mut media = Vec::new();
    let image = cached_raster.as_ref().map(|cached| {
        let same_camera = cached.viewport_generation == editor.minimap.viewport_generation();
        let placement_content_top = crate::editor::minimap::raster_placement_content_top(
            cached,
            editor.minimap.viewport_generation(),
            geometry.content_top,
        );
        let placement_line_height = if same_camera {
            cached.line_height
        } else {
            line_height
        };
        let image_y = density.edge_padding()
            + (cached.key.first_unit as f32 - placement_content_top) * placement_line_height;
        let image_height = usize::from(cached.key.rows) as f32 * cached.line_height;
        for source in cached.media.iter() {
            let row_y = density.edge_padding()
                + (cached.key.first_unit as f32 + source.row_offset_units - placement_content_top)
                    * placement_line_height;
            let row_height = source.row_height_units * placement_line_height;
            // Keep bounded raster-window media in the paint frame even while it
            // is just outside the visible track. `use_asset` can then start the
            // load during prefetch instead of popping the image in after it has
            // entered the viewport.
            media.push(MinimapImagePaint {
                path: source.path.clone(),
                row_y,
                row_height,
            });
        }
        (
            cached.image.clone(),
            Bounds::new(
                point(bounds.left(), bounds.top() + px(image_y)),
                size(bounds.size.width, px(image_height)),
            ),
        )
    });
    let thumb_top = editor
        .minimap
        .drag
        .map_or(geometry.thumb_top, |drag| drag.current_thumb_top)
        .clamp(
            0.0,
            (geometry.interaction_height - geometry.thumb_height).max(0.0),
        );
    let (visual_thumb_top, visual_thumb_height) =
        crate::minimap::visual_thumb_geometry(thumb_top, geometry.thumb_height, density);
    let thumb_bounds = Bounds::new(
        point(bounds.left(), bounds.top() + px(visual_thumb_top)),
        size(bounds.size.width, px(visual_thumb_height)),
    );
    let (fill_alpha, border_alpha) =
        crate::minimap::thumb_alphas(editor.minimap.drag.is_some(), false, false);
    overlays.push(fill(
        thumb_bounds,
        rgba((theme.foreground << 8) | fill_alpha),
    ));
    overlays.push(outline(
        thumb_bounds,
        rgba((theme.foreground << 8) | border_alpha),
        BorderStyle::default(),
    ));
    for mark in editor.minimap.search_marks.iter().take(2_000) {
        if let Ok(line) = snapshot.line_index_at(mark.start) {
            let line_unit = minimap_layout.line_start_y(line.0) / editor_line_height;
            let y = density.edge_padding() + (line_unit - geometry.content_top) * line_height;
            if y < 0.0 || y > geometry.interaction_height {
                continue;
            }
            overlays.push(fill(
                Bounds::new(
                    point(bounds.right() - px(5.0), bounds.top() + px(y)),
                    size(px(4.0), px(2.0)),
                ),
                rgba((theme.link << 8) | 0xee),
            ));
        }
    }
    (
        MinimapPaint {
            bounds,
            background: Some(background),
            image,
            media,
            overlays,
        },
        raster_request,
    )
}

/// Paints the prepared minimap frame: background, raster image, media previews
/// and overlays.
///
/// `host` is only read on benchmark builds, where it feeds image telemetry.
#[cfg_attr(not(feature = "benchmarks"), allow(unused_variables))]
pub(super) fn paint_minimap_layer(
    host: &gpui::Entity<SemanticEditor>,
    state: &mut PrepaintState,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(background) = &state.minimap.background {
        window.paint_quad(background.clone());
    }
    if let Some((image, image_bounds)) = &state.minimap.image {
        #[cfg(feature = "benchmarks")]
        host.read(cx)
            .minimap
            .telemetry
            .note_image_paint(*image_bounds, state.minimap.bounds);
        let _ = window.paint_image(
            *image_bounds,
            *image_bounds,
            Corners::default(),
            image.clone(),
            0,
            false,
        );
    }
    for media in &state.minimap.media {
        let source: Arc<std::path::Path> = media.path.clone().into();
        let Some(Ok(loaded)) =
            window.use_asset::<crate::editor::image_loader::EditorImageLoader>(&source, cx)
        else {
            continue;
        };
        let image = loaded.image;
        let Some((x, y, width, height)) = editor_minimap_media_geometry(
            loaded.dimensions.0 as f32,
            loaded.dimensions.1 as f32,
            f32::from(state.minimap.bounds.size.width),
            media.row_y,
            media.row_height,
        ) else {
            continue;
        };
        let image_bounds = Bounds::new(
            point(
                state.minimap.bounds.left() + px(x),
                state.minimap.bounds.top() + px(y),
            ),
            size(px(width), px(height)),
        );
        let visible_image_bounds = image_bounds.intersect(&state.minimap.bounds);
        if f32::from(visible_image_bounds.size.width) <= 0.0
            || f32::from(visible_image_bounds.size.height) <= 0.0
        {
            continue;
        }
        let _ = window.paint_image(
            image_bounds,
            image_bounds,
            Corners::default(),
            image,
            0,
            false,
        );
    }
    for quad in &state.minimap.overlays {
        window.paint_quad(quad.clone());
    }
}
