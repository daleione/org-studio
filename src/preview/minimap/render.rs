use smallvec::SmallVec;
use std::{
    collections::HashSet,
    hash::{Hash, Hasher},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use crate::{
    document::DocumentFormat,
    org_syntax::BlockKind,
    preview::{
        PreviewSnapshot, PreviewStyle,
        markdown::MarkdownKind,
        projection::{VisualRow, VisualRowKind},
    },
};
use gpui::{
    BorderStyle, Bounds, ContentMask, Corners, CursorStyle, DispatchPhase, ListOffset, ListState,
    MouseButton, MouseMoveEvent, MouseUpEvent, RenderImage, canvas, div, fill, outline, point,
    prelude::*, px, rgba,
};

use super::projection::MinimapRefinement;
use super::{
    MINIMAP_RESIZE_HANDLE_PX, MinimapDensity, MinimapDragSession, MinimapInteractionAnchor,
    MinimapLineIndex, MinimapProjectionReadiness, MinimapResizeSession, MinimapState,
    MinimapWidthChange, PreviewDisplayMap, PreviewLineKind, RASTER_TILE_ROWS, RasterRow,
    RasterTableGeometry, RasterTileKey, RasterTilePaint, RasterTileRequest, SCROLL_WHEEL_LINE_PX,
    current_resize_session, folded_signature, minimap_anchor_for_thumb_top,
    minimap_click_target_for_viewport, minimap_drag_target, minimap_perf_enabled,
    minimap_thumb_for_drag, minimap_trace_enabled, minimap_viewport_for_list_with_anchor,
    raster_tile_window, rasterize_tile, scroll_list_to_ratio, scroll_ratio_after_wheel,
    source_target_for_list_offset, take_resize_session, thumb_alphas, tile_key,
    width_from_resize_drag,
};

struct MinimapMediaPaint {
    image: Arc<RenderImage>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    row_y: f32,
    row_height: f32,
}

#[derive(Default)]
struct MinimapPaintFrame {
    tiles: SmallVec<[RasterTilePaint; 6]>,
    media: SmallVec<[MinimapMediaPaint; 4]>,
}

fn minimap_media_geometry(
    reading_width: f32,
    reading_height: f32,
    reading_line_height: f32,
    minimap_line_height: f32,
    minimap_width: f32,
    row_y: f32,
    row_height: f32,
) -> Option<(f32, f32, f32, f32)> {
    if !reading_width.is_finite()
        || !reading_height.is_finite()
        || reading_width <= 0.0
        || reading_height <= 0.0
        || reading_line_height <= 0.0
        || minimap_line_height <= 0.0
        || row_height <= 0.0
    {
        return None;
    }
    let inset = 5.0;
    let available_width = (minimap_width - inset * 2.0).max(1.0);
    // Use the same uniform scale that maps a Reading body line to a
    // minimap line. Filling the minimap width independently enlarges a
    // narrow image relative to the surrounding page.
    let scale = (minimap_line_height / reading_line_height)
        .min(available_width / reading_width)
        .min(row_height / reading_height)
        .min(1.0);
    let width = reading_width * scale;
    let height = reading_height * scale;
    Some((
        // Reading media starts at the content origin. Its minimap projection
        // follows the same left/top anchor instead of centering inside the row.
        inset, row_y, width, height,
    ))
}

fn minimap_media_image(
    document: &PreviewSnapshot,
    visual: &VisualRow,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
) -> Option<Arc<RenderImage>> {
    match &visual.kind {
        VisualRowKind::Image { .. } => {
            let source = match document.format {
                DocumentFormat::Org => {
                    match &document.blocks.nodes().get(visual.block_id as usize)?.kind {
                        BlockKind::Image { path } => path.as_ref(),
                        _ => return None,
                    }
                }
                DocumentFormat::Markdown => {
                    match &document.markdown_blocks.get(visual.block_id as usize)?.kind {
                        MarkdownKind::Image { path } => path.as_str(),
                        _ => return None,
                    }
                }
            };
            let path = crate::preview::resolve_image_path(&document.path, source);
            let resource: gpui::Resource = path.into();
            window
                .use_asset::<gpui::ImgResourceLoader>(&resource, cx)?
                .ok()
        }
        VisualRowKind::Diagram(crate::preview::diagram::DiagramProjection::Ready {
            image, ..
        }) => image.clone().use_render_image(window, cx),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    document: Arc<PreviewSnapshot>,
    model: Arc<PreviewDisplayMap>,
    state: Arc<MinimapState>,
    presentation_rows: Arc<Vec<usize>>,
    folded: Arc<HashSet<u32>>,
    list_state: ListState,
    editor_width: f32,
    minimap_width: f32,
    thumb_visibility: crate::settings::MinimapThumbVisibility,
    generation: u64,
    geometry_revision: u64,
    zoom: f32,
    style: PreviewStyle,
    allow_projection_refinement: bool,
    opened_at: Instant,
    on_seek: impl Fn(
        super::viewport::MinimapSourceTarget,
        ListOffset,
        &mut gpui::Window,
        &mut gpui::App,
    ) + 'static,
    on_width_change: impl Fn(MinimapWidthChange, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let palette = style.palette;
    let density = MinimapDensity::for_width(minimap_width);
    let minimap_line_height = density.line_height();
    let minimap_edge_padding = density.edge_padding();
    let drag_session = state.drag.clone();
    let resize_session = state.resize_drag.clone();
    let on_width_change = Arc::new(on_width_change);
    let on_seek = Arc::new(on_seek);
    let interaction_anchor = state.interaction_anchor.clone();
    let paint_list = list_state.clone();
    let shape_list = list_state.clone();
    let shape_model = model.clone();
    let shape_document = document;
    let shape_state = state.clone();
    let paint_state = state.clone();
    let shape_rows = presentation_rows.clone();
    let shape_folded = folded;
    let active_line_index = Arc::new(Mutex::new(None::<MinimapLineIndex>));
    let shape_line_index = active_line_index.clone();
    let paint_line_index = active_line_index.clone();
    let shape_anchor = interaction_anchor.clone();
    let paint_anchor = interaction_anchor.clone();
    let track_bounds = Arc::new(Mutex::new(Bounds::default()));
    let shape_bounds = track_bounds.clone();
    let paint_drag = drag_session.clone();
    let event_drag = drag_session.clone();
    let event_resize = resize_session.clone();
    let event_width_change = on_width_change.clone();
    let event_anchor = interaction_anchor.clone();
    let event_line_index = active_line_index.clone();
    let event_list = list_state.clone();
    let hovered = Arc::new(AtomicBool::new(false));
    let paint_hovered = hovered.clone();
    let content = canvas(
        move |bounds, window, cx| {
            profiling::scope!("Minimap::shape_visible");
            *shape_bounds.lock().expect("minimap bounds poisoned") = bounds;
            let visible_units = ((f32::from(bounds.size.height) - minimap_edge_padding * 2.0)
                / minimap_line_height)
                .max(1.0);
            let width = f32::from(bounds.size.width).ceil().max(1.0) as usize;
            let scale_factor = window.scale_factor().max(1.0);
            let parent_width =
                crate::preview::layout::reading_content_width(editor_width, minimap_width, style);
            let priority_row = shape_list
                .logical_scroll_top()
                .item_ix
                .min(shape_rows.len().saturating_sub(1));
            let interaction_active = shape_state
                .drag
                .lock()
                .expect("minimap drag state poisoned")
                .is_some()
                || shape_state
                    .resize_drag
                    .lock()
                    .expect("minimap resize state poisoned")
                    .is_some();
            let projection = {
                profiling::scope!("Minimap::line_index");
                shape_model.advance_minimap_line_index(
                    &shape_state,
                    &shape_rows,
                    parent_width,
                    minimap_width,
                    density,
                    MinimapRefinement {
                        priority_row,
                        allow: allow_projection_refinement && !interaction_active,
                        geometry_revision,
                    },
                    zoom,
                    style,
                    window.text_system(),
                )
            };
            let projection_readiness = projection.readiness;
            let projection_exact_rows = projection.exact_rows;
            if projection.readiness != MinimapProjectionReadiness::Exact
                && allow_projection_refinement
                && !interaction_active
            {
                // Drive cooperative exact refinement at display cadence. The
                // estimated projection remains paintable throughout the process.
                window.request_animation_frame();
                if minimap_trace_enabled() {
                    eprintln!(
                        "org_studio_minimap_projection_progress readiness=estimated exact_rows={} rows_total={}",
                        projection.exact_rows,
                        shape_rows.len(),
                    );
                }
            }
            let line_index = projection.index;
            *shape_line_index.lock().expect("active line index poisoned") =
                Some(line_index.clone());
            let viewport = minimap_viewport_for_list_with_anchor(
                &line_index,
                &shape_list,
                f32::from(bounds.size.height),
                *shape_anchor.lock().expect("minimap anchor poisoned"),
            );
            if minimap_trace_enabled()
                && projection_readiness == MinimapProjectionReadiness::Exact
            {
                let scroll = shape_list.logical_scroll_top();
                let viewport_pixels = f32::from(shape_list.viewport_bounds().size.height).max(0.0);
                let scroll_pixels = line_index.pixel_for_list_offset(scroll);
                let (visible_top, visible_bottom) = super::viewport::minimap_visible_display_range(
                    &line_index,
                    scroll_pixels,
                    viewport_pixels,
                );
                eprintln!(
                    "org_studio_minimap_viewport readiness={projection_readiness:?} exact_rows={projection_exact_rows} rows={} scroll_item={} scroll_inner={:.3} scroll_pixels={scroll_pixels:.3} viewport_pixels={viewport_pixels:.3} visible_top={visible_top:.3} visible_bottom={visible_bottom:.3} total_units={:.3} document_pixels={:.3} content_top={:.3} thumb_top={:.3} thumb_height={:.3}",
                    shape_rows.len(),
                    scroll.item_ix,
                    f32::from(scroll.offset_in_item),
                    line_index.total_units(),
                    line_index.document_pixels(),
                    viewport.content_top,
                    viewport.thumb.top,
                    viewport.thumb.height,
                );
            }
            let visible_range = if shape_rows.is_empty() {
                0..0
            } else {
                let start_pixel = line_index.pixel_for_display_position(viewport.content_top);
                let end_pixel = line_index
                    .pixel_for_display_position(viewport.content_top + visible_units)
                    .min(line_index.document_pixels());
                let first = line_index.projection.locate_pixel(start_pixel).0;
                let end = if end_pixel + f32::EPSILON >= line_index.document_pixels() {
                    shape_rows.len()
                } else {
                    line_index
                        .projection
                        .locate_pixel(end_pixel)
                        .0
                        .saturating_add(1)
                        .min(shape_rows.len())
                };
                first..end.max(first.saturating_add(1).min(shape_rows.len()))
            };
            let mut media: SmallVec<[MinimapMediaPaint; 4]> = SmallVec::new();
            let media_range = visible_range.start.saturating_sub(RASTER_TILE_ROWS)
                ..visible_range
                    .end
                    .saturating_add(RASTER_TILE_ROWS)
                    .min(shape_rows.len());
            for presentation_index in media_range {
                let document_index = shape_rows[presentation_index];
                let Some(visual) = shape_model.projection.rows.get(document_index) else {
                    continue;
                };
                if !matches!(
                    visual.kind,
                    VisualRowKind::Image { .. } | VisualRowKind::Diagram(_)
                ) {
                    continue;
                }
                let Some(image) =
                    minimap_media_image(&shape_document, visual, window, cx)
                else {
                    continue;
                };
                let Some((reading_width, reading_height)) =
                    shape_model.image_size(document_index, parent_width)
                else {
                    continue;
                };
                let (_, pixel_start) = line_index.projection.prefix_for_row(presentation_index);
                let row_pixels = line_index.projection.measure(presentation_index).pixels;
                let row_y = minimap_edge_padding
                    + (line_index.display_position_for_pixel(pixel_start) - viewport.content_top)
                        * minimap_line_height;
                let row_height = row_pixels / line_index.reading_line_height * minimap_line_height;
                let Some((x, y, width, height)) = minimap_media_geometry(
                    reading_width,
                    reading_height,
                    style.typography.body_line_height * zoom,
                    minimap_line_height,
                    width as f32,
                    row_y,
                    row_height,
                ) else {
                    continue;
                };
                // `minimap_media_image` touches the asset throughout the bounded
                // prefetch range. Only visible images become paint operations,
                // but upcoming media has already started loading.
                if y + height >= 0.0 && y <= f32::from(bounds.size.height) {
                    media.push(MinimapMediaPaint {
                        image,
                        x,
                        y,
                        width,
                        height,
                        row_y,
                        row_height,
                    });
                }
            }
            let first_line = visible_range.start;
            let last_line = visible_range.end;
            let mut tiles: SmallVec<[RasterTilePaint; 6]> = SmallVec::new();
            let mut visible_requests: SmallVec<[RasterTileRequest; 6]> = SmallVec::new();
            let mut prefetch_requests: SmallVec<[RasterTileRequest; 2]> = SmallVec::new();
            let mut visible_keys: SmallVec<[RasterTileKey; 6]> = SmallVec::new();
            let mut prefetch_hasher = std::collections::hash_map::DefaultHasher::new();
            generation.hash(&mut prefetch_hasher);
            geometry_revision.hash(&mut prefetch_hasher);
            shape_document.revision.hash(&mut prefetch_hasher);
            width.hash(&mut prefetch_hasher);
            density.hash(&mut prefetch_hasher);
            style.paint_key().hash(&mut prefetch_hasher);
            style.layout_key().hash(&mut prefetch_hasher);
            zoom.to_bits().hash(&mut prefetch_hasher);
            scale_factor.to_bits().hash(&mut prefetch_hasher);
            first_line
                .saturating_div(RASTER_TILE_ROWS)
                .hash(&mut prefetch_hasher);
            last_line
                .saturating_sub(1)
                .saturating_div(RASTER_TILE_ROWS)
                .hash(&mut prefetch_hasher);
            let prefetch_signature = prefetch_hasher.finish().max(1);
            let cache_busy = !shape_state
                .raster_tiles
                .lock()
                .expect("minimap raster tile cache poisoned")
                .in_flight
                .is_empty();
            let include_prefetch = projection_readiness == MinimapProjectionReadiness::Exact
                && !cache_busy
                && shape_state
                    .raster_prefetch_signature
                    .load(Ordering::Acquire)
                    != prefetch_signature;
            let tile_window = raster_tile_window(
                shape_rows.len(),
                first_line..last_line,
                usize::from(include_prefetch),
            );
            for tile_start in tile_window.step_by(RASTER_TILE_ROWS) {
                let tile_end = (tile_start + RASTER_TILE_ROWS).min(shape_rows.len());
                let tile_rows = &shape_rows[tile_start..tile_end];
                let (_, tile_pixel_start) = line_index.projection.prefix_for_row(tile_start);
                let (_, tile_pixel_end) = line_index.projection.prefix_for_row(tile_end);
                let is_visible_tile = tile_start < last_line && tile_end > first_line;
                let mut wrap_hasher = std::collections::hash_map::DefaultHasher::new();
                parent_width.to_bits().hash(&mut wrap_hasher);
                line_index.layout.hash(&mut wrap_hasher);
                line_index
                    .reading_line_height
                    .to_bits()
                    .hash(&mut wrap_hasher);
                let raster_rows = tile_rows
                    .iter()
                    .enumerate()
                    .map(|(offset, &row)| {
                        let mut lines =
                            shape_model.display_lines(
                                row,
                                parent_width,
                                zoom,
                                style,
                                window.text_system(),
                            );
                        let block_id = shape_model.source_row(row).block_id;
                        if shape_folded.contains(&block_id)
                            && matches!(shape_model.row_kind(row), PreviewLineKind::Heading(_))
                        {
                            let mut ranges = lines.ranges.to_vec();
                            if let Some(last) = ranges.last_mut() {
                                last.end += " …".len();
                            }
                            lines.ranges = ranges.into();
                        }
                        let presentation_index = tile_start + offset;
                        let (_, row_pixel_start) =
                            line_index.projection.prefix_for_row(presentation_index);
                        let measure = line_index.projection.measure(presentation_index);
                        let row_layout = shape_model.layout(row, style).scaled(zoom);
                        let is_table = shape_model.row_kind(row) == PreviewLineKind::Table;
                        let content_offset_pixels = row_layout.margin_top
                            + if is_table {
                                style.spacing.table_cell_y * zoom
                            } else {
                                row_layout.padding_top
                            };
                        let content_line_step_pixels = if is_table {
                            (style.typography.body_line_height - 3.0).max(18.0) * zoom
                        } else {
                            row_layout.line_height
                        };
                        let table = shape_model.table_projection(row).map(|projection| {
                            let display = shape_model.runs(row);
                            RasterTableGeometry {
                                projected: crate::preview::table::project_table(
                                    projection.table(),
                                    parent_width,
                                    zoom,
                                    width as f32,
                                    style,
                                ),
                                wrapped_cells: projection.shaped_display_cells(
                                    &display.text,
                                    parent_width,
                                    zoom,
                                    style,
                                    window.text_system(),
                                ),
                            }
                        });
                        RasterRow {
                            document_index: row,
                            display_line_count: lines.ranges.len().max(1),
                            lines,
                            offset_units: (row_pixel_start - tile_pixel_start)
                                / line_index.reading_line_height,
                            height_units: measure.pixels / line_index.reading_line_height,
                            content_offset_units: content_offset_pixels
                                / line_index.reading_line_height,
                            content_line_step_units: content_line_step_pixels
                                / line_index.reading_line_height,
                            table,
                        }
                    })
                    .collect::<Vec<_>>();
                let tile_identity_rows = tile_rows
                    .iter()
                    .map(|&row| {
                        let visual = shape_model
                            .projection
                            .rows
                            .get(row)
                            .expect("presentation row exists");
                        visual.id.0 as usize
                    })
                    .collect::<Vec<_>>();
                for row in &raster_rows {
                    let visual = shape_model
                        .projection
                        .rows
                        .get(row.document_index)
                        .expect("raster row exists");
                    visual.id.hash(&mut wrap_hasher);
                    visual.semantic_revision.hash(&mut wrap_hasher);
                    let lines = &row.lines;
                    lines.parent_height.to_bits().hash(&mut wrap_hasher);
                    row.offset_units.to_bits().hash(&mut wrap_hasher);
                    row.height_units.to_bits().hash(&mut wrap_hasher);
                    row.content_offset_units.to_bits().hash(&mut wrap_hasher);
                    row.content_line_step_units.to_bits().hash(&mut wrap_hasher);
                    if let Some(table) = &row.table {
                        for column in &table.projected.columns {
                            column.start_x.to_bits().hash(&mut wrap_hasher);
                            column.end_x.to_bits().hash(&mut wrap_hasher);
                            column.content_start_x.to_bits().hash(&mut wrap_hasher);
                            column.content_end_x.to_bits().hash(&mut wrap_hasher);
                        }
                        for cell in &table.wrapped_cells {
                            cell.hash(&mut wrap_hasher);
                        }
                    }
                    for range in lines.ranges.iter() {
                        range.start.hash(&mut wrap_hasher);
                        range.end.hash(&mut wrap_hasher);
                    }
                }
                let tile_y = minimap_edge_padding
                    + (line_index.display_position_for_pixel(tile_pixel_start)
                        - viewport.content_top)
                        * minimap_line_height;
                let tile_height = ((tile_pixel_end - tile_pixel_start)
                    / line_index.reading_line_height
                    * minimap_line_height)
                    .max(1.0);
                let key = tile_key(
                    &tile_identity_rows,
                    tile_start,
                    width,
                    folded_signature(&shape_model, tile_rows, &shape_folded),
                    wrap_hasher.finish(),
                    scale_factor,
                    density,
                    style,
                );
                if is_visible_tile {
                    visible_keys.push(key);
                }
                let (image, is_missing) = shape_state
                    .raster_tiles
                    .lock()
                    .expect("minimap raster tile cache poisoned")
                    .image_or_fallback(key);
                if is_visible_tile
                    && let Some(image) = image
                    && tile_y + tile_height >= 0.0
                    && tile_y <= f32::from(bounds.size.height)
                {
                    tiles.push(RasterTilePaint {
                        image,
                        y: tile_y,
                        width: width as f32,
                        height: tile_height,
                    });
                }
                if is_missing {
                    let request = RasterTileRequest {
                        key,
                        rows: raster_rows,
                    };
                    if is_visible_tile {
                        visible_requests.push(request);
                    } else {
                        prefetch_requests.push(request);
                    }
                }
            }
            // Never let speculative work delay a missing visible batch. Once every visible
            // tile is ready, prepare one adjacent tile on each side before it enters the
            // Minimap camera so scrolling cannot reveal a late raster replacement.
            let visible_request_count = visible_requests.len();
            let raster_requests = if visible_requests.is_empty() {
                visible_requests.extend(prefetch_requests);
                visible_requests
            } else {
                visible_requests
            };
            let request_count = raster_requests.len();
            let request_keys = raster_requests
                .iter()
                .map(|request| request.key)
                .collect::<SmallVec<[_; 6]>>();
            let should_spawn = shape_state
                .raster_tiles
                .lock()
                .expect("minimap raster tile cache poisoned")
                .reserve(&request_keys);
            if include_prefetch
                && visible_request_count == 0
                && (request_count == 0 || should_spawn)
            {
                shape_state
                    .raster_prefetch_signature
                    .store(prefetch_signature, Ordering::Release);
            }
            if should_spawn {
                let raster_epoch = shape_state.raster_epoch.load(Ordering::Acquire);
                let tile_model = shape_model.clone();
                let tile_state = shape_state.clone();
                let tile_folded = shape_folded.clone();
                let atomic_batch = request_count > 1;
                let background = cx.background_executor().spawn(async move {
                    let mut completed = Vec::with_capacity(raster_requests.len());
                    for request in raster_requests {
                        let rasterized = rasterize_tile(
                            &tile_model,
                            &request.rows,
                            width,
                            &tile_folded,
                            scale_factor,
                            density,
                            style,
                        );
                        if minimap_perf_enabled() {
                            let first = !tile_state
                                .perf
                                .first_tile_completed
                                .swap(true, Ordering::AcqRel);
                            eprintln!(
                                "org_studio_minimap_tile_ready generation={} revision={} tile_start={} rows={} lines={} width={} total_ms={:.3} text_system_wait_ms={:.3} cold_text_system={} first={} since_open_ms={:.3}",
                                generation,
                                geometry_revision,
                                request.key.tile_start,
                                request.rows.len(),
                                rasterized.line_count,
                                width,
                                rasterized.total.as_secs_f64() * 1000.0,
                                rasterized.text_system_wait.as_secs_f64() * 1000.0,
                                rasterized.cold_text_system,
                                first,
                                opened_at.elapsed().as_secs_f64() * 1000.0,
                            );
                        }
                        completed.push((request.key, rasterized.image));
                    }
                    if tile_state.raster_epoch.load(Ordering::Acquire) != raster_epoch {
                        return;
                    }
                    let completed_count = completed.len();
                    tile_state
                        .raster_tiles
                        .lock()
                        .expect("minimap raster tile cache poisoned")
                        .insert_batch(completed, &visible_keys);
                    // `insert_batch` publishes the complete visible replacement while the cache
                    // lock is held. Shape keeps painting the retained previous frame until this
                    // point, so no partial style frame is exposed.
                    if minimap_perf_enabled() && atomic_batch {
                        eprintln!(
                            "org_studio_minimap_tile_batch_ready tiles={} since_open_ms={:.3}",
                            completed_count,
                            opened_at.elapsed().as_secs_f64() * 1000.0,
                        );
                    }
                });
                cx.spawn(async move |cx| {
                    background.await;
                    cx.refresh();
                })
                .detach();
            }
            if visible_request_count == 0 && !tiles.is_empty() {
                *shape_state
                    .retained_style_frame
                    .lock()
                    .expect("retained minimap frame poisoned") = tiles.iter().cloned().collect();
                shape_state
                    .retain_style_frame
                    .store(false, Ordering::Release);
            } else if shape_state.retain_style_frame.load(Ordering::Acquire) {
                tiles = shape_state
                    .retained_style_frame
                    .lock()
                    .expect("retained minimap frame poisoned")
                    .iter()
                    .cloned()
                    .collect();
            }
            MinimapPaintFrame { tiles, media }
        },
        move |bounds, frame: MinimapPaintFrame, window, cx| {
            profiling::scope!("Minimap::paint");
            if !frame.tiles.is_empty()
                && !paint_state
                    .perf
                    .first_pixels_painted
                    .swap(true, Ordering::AcqRel)
            {
                if minimap_perf_enabled() {
                    eprintln!(
                        "org_studio_minimap_first_pixels generation={} revision={} tiles={} since_open_ms={:.3}",
                        generation,
                        geometry_revision,
                        frame.tiles.len(),
                        opened_at.elapsed().as_secs_f64() * 1000.0,
                    );
                    eprintln!(
                        "org_preview_coherent_first_frame generation={} revision={} since_open_ms={:.3}",
                        generation,
                        geometry_revision,
                        opened_at.elapsed().as_secs_f64() * 1000.0,
                    );
                }
                if std::env::var_os("ORG_STUDIO_EXIT_AFTER_MINIMAP_FRAME").is_some() {
                    cx.quit();
                }
            }
            for tile in frame.tiles {
                let image_bounds = Bounds::new(
                    point(bounds.origin.x, bounds.origin.y + px(tile.y)),
                    gpui::size(px(tile.width), px(tile.height)),
                );
                let _ = window.paint_image(
                    image_bounds,
                    image_bounds,
                    Corners::default(),
                    tile.image,
                    0,
                    false,
                );
            }
            for media in frame.media {
                let image_bounds = Bounds::new(
                    point(
                        bounds.origin.x + px(media.x),
                        bounds.origin.y + px(media.y),
                    ),
                    gpui::size(px(media.width), px(media.height)),
                );
                let row_bounds = Bounds::new(
                    point(bounds.origin.x, bounds.origin.y + px(media.row_y)),
                    gpui::size(bounds.size.width, px(media.row_height)),
                )
                .intersect(&bounds);
                // Keep rounding or a decoder-specific image size from painting
                // across the media row boundary and covering following text.
                window.with_content_mask(Some(ContentMask { bounds: row_bounds }), |window| {
                    let _ = window.paint_image(
                        image_bounds,
                        image_bounds,
                        Corners::default(),
                        media.image,
                        0,
                        false,
                    );
                });
            }
            let track_height = f32::from(bounds.size.height);
            let viewport = paint_line_index
                .lock()
                .expect("active line index poisoned")
                .as_ref()
                .map(|index| {
                    minimap_viewport_for_list_with_anchor(
                        index,
                        &paint_list,
                        track_height,
                        *paint_anchor.lock().expect("minimap anchor poisoned"),
                    )
                })
                .unwrap_or_default();
            let thumb = minimap_thumb_for_drag(
                viewport,
                *paint_drag.lock().expect("minimap drag state poisoned"),
            );
            let (visual_thumb_top, visual_thumb_height) = crate::minimap::visual_thumb_geometry(
                thumb.top,
                thumb.height,
                density,
            );
            let thumb_bounds = Bounds::new(
                point(bounds.origin.x, bounds.origin.y + px(visual_thumb_top)),
                gpui::size(bounds.size.width, px(visual_thumb_height)),
            );
            let active = paint_drag
                .lock()
                .expect("minimap drag state poisoned")
                .is_some();
            let hovered = paint_hovered.load(Ordering::Relaxed);
            let (fill_alpha, border_alpha) = thumb_alphas(
                active,
                hovered,
                thumb_visibility == crate::settings::MinimapThumbVisibility::Hover,
            );
            window.paint_quad(fill(
                thumb_bounds,
                rgba((palette.foreground << 8) | fill_alpha),
            ));
            window.paint_quad(outline(
                thumb_bounds,
                rgba((palette.foreground << 8) | border_alpha),
                BorderStyle::default(),
            ));

            // Register drag listeners on the window, not the minimap hitbox. GPUI keeps
            // delivering native drag events to the originating window after the pointer
            // leaves this element, so horizontal escape does not cancel the gesture.
            let move_drag = event_drag.clone();
            let move_resize = event_resize.clone();
            let move_width_change = event_width_change.clone();
            let move_anchor = event_anchor.clone();
            let move_index = event_line_index.clone();
            let move_list = event_list.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _cx| {
                if phase != DispatchPhase::Bubble || !event.dragging() {
                    return;
                }
                if let Some(session) = current_resize_session(&move_resize) {
                    let width = width_from_resize_drag(
                        editor_width,
                        session,
                        f32::from(event.position.x),
                    );
                    move_width_change(MinimapWidthChange::Preview(width), window, _cx);
                    window.set_window_cursor_style(CursorStyle::ResizeLeftRight);
                    return;
                }
                let session = *move_drag.lock().expect("minimap drag state poisoned");
                let Some(session) = session else {
                    return;
                };
                let index = move_index
                    .lock()
                    .expect("active line index poisoned")
                    .clone();
                let Some(index) = index else {
                    return;
                };
                let local_y = f32::from(event.position.y - bounds.origin.y);
                let current_anchor = *move_anchor.lock().expect("minimap anchor poisoned");
                let viewport = minimap_viewport_for_list_with_anchor(
                    &index,
                    &move_list,
                    f32::from(bounds.size.height),
                    current_anchor,
                );
                let (target, desired_thumb_top) = minimap_drag_target(
                    local_y,
                    session,
                    viewport.thumb.height,
                    viewport.interaction_height,
                );
                if minimap_trace_enabled() {
                    eprintln!(
                        "minimap drag y={local_y:.2} start_y={:.2} current_ratio={:.5} target_ratio={target:.5}",
                        session.start_pointer_y,
                        viewport.scroll_ratio,
                    );
                }
                scroll_list_to_ratio(&index, &move_list, target);
                let settled_viewport = minimap_viewport_for_list_with_anchor(
                    &index,
                    &move_list,
                    f32::from(bounds.size.height),
                    current_anchor,
                );
                let desired_thumb_top = desired_thumb_top.clamp(
                    0.0,
                    (settled_viewport.interaction_height - settled_viewport.thumb.height).max(0.0),
                );
                *move_anchor.lock().expect("minimap anchor poisoned") =
                    Some(minimap_anchor_for_thumb_top(
                        &index,
                        &move_list,
                        f32::from(bounds.size.height),
                        desired_thumb_top,
                    ));
                if let Some(active_session) = move_drag
                    .lock()
                    .expect("minimap drag state poisoned")
                    .as_mut()
                {
                    active_session.current_thumb_top = desired_thumb_top;
                }
                window.refresh();
            });

            let up_drag = event_drag.clone();
            let up_resize = event_resize.clone();
            let up_width_change = event_width_change.clone();
            let up_list = event_list.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                    return;
                }
                // `take_resize_session` drops the mutex guard before invoking the
                // callback. Commit clears all interaction state and therefore locks
                // this same mutex again.
                if let Some(session) = take_resize_session(&up_resize) {
                    let width = width_from_resize_drag(
                        editor_width,
                        session,
                        f32::from(event.position.x),
                    );
                    up_width_change(MinimapWidthChange::Commit(width), window, cx);
                    window.refresh();
                    return;
                }
                if up_drag
                    .lock()
                    .expect("minimap drag state poisoned")
                    .take()
                    .is_some()
                {
                    up_list.scrollbar_drag_ended();
                    window.refresh();
                }
            });
        },
    )
    .size_full();

    let down_bounds = track_bounds;
    let down_line_index = active_line_index.clone();
    let down_drag = drag_session.clone();
    let down_anchor = interaction_anchor.clone();
    let scroll_drag = drag_session;
    let scroll_line_index = active_line_index;
    let scroll_list = list_state.clone();
    let down_list = list_state.clone();
    let down_projection = model.projection.clone();
    let down_rows = presentation_rows.clone();
    let down_seek = on_seek;
    let handle_resize = resize_session;
    let handle_width_change = on_width_change;
    let hover_state = hovered;
    div()
        .id("document-minimap")
        .h_full()
        .w(px(minimap_width))
        .flex_none()
        .relative()
        .overflow_hidden()
        .border_l_1()
        .border_color(gpui::rgb(palette.border))
        .bg(gpui::rgb(palette.surface))
        .cursor_pointer()
        .on_hover(move |is_hovered, window, _| {
            if hover_state.swap(*is_hovered, Ordering::Relaxed) != *is_hovered {
                window.refresh();
            }
        })
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            let bounds = *down_bounds.lock().expect("minimap bounds poisoned");
            let local_y = f32::from(event.position.y - bounds.origin.y);
            let track_height = f32::from(bounds.size.height);
            let index = down_line_index
                .lock()
                .expect("active line index poisoned")
                .clone();
            let Some(index) = index else {
                return;
            };
            let viewport = minimap_viewport_for_list_with_anchor(
                &index,
                &down_list,
                track_height,
                *down_anchor.lock().expect("minimap anchor poisoned"),
            );
            let thumb = viewport.thumb;
            let clicked_thumb = local_y >= thumb.top && local_y <= thumb.top + thumb.height;
            let drag_start;
            if minimap_trace_enabled() {
                eprintln!(
                    "minimap down y={local_y:.2} thumb_top={:.2} thumb_h={:.2} extent={:.2} inside={clicked_thumb} content_top={:.2}",
                    thumb.top,
                    thumb.height,
                    viewport.interaction_height,
                    viewport.content_top,
                );
            }
            if !clicked_thumb {
                let target =
                    minimap_click_target_for_viewport(&index, &down_list, viewport, local_y);
                if let Some(source_target) = source_target_for_list_offset(
                    &down_projection,
                    &down_rows,
                    target.offset,
                    crate::preview::coordinates::Bias::for_boundary(false),
                ) {
                    down_seek(source_target, target.offset, window, cx);
                }
                if minimap_trace_enabled() {
                    eprintln!(
                        "minimap click display={:.3} target_ratio={:.5} anchor_content={:.3} anchor_thumb={:.2}",
                        target.clicked_display,
                        target.ratio,
                        viewport.content_top,
                        target.thumb_top,
                    );
                }
                scroll_list_to_ratio(&index, &down_list, target.ratio);
                *down_anchor.lock().expect("minimap anchor poisoned") =
                    Some(minimap_anchor_for_thumb_top(
                        &index,
                        &down_list,
                        track_height,
                        target.thumb_top,
                    ));
                drag_start = (target.thumb_top, target.ratio);
                window.refresh();
            } else {
                drag_start = (thumb.top, viewport.scroll_ratio);
                *down_anchor.lock().expect("minimap anchor poisoned") =
                    Some(MinimapInteractionAnchor {
                        layout: index.layout,
                        width: index.width,
                        minimap_width: index.minimap_width,
                        rows_signature: index.rows_signature,
                        interaction_height: viewport.interaction_height,
                        content_top: viewport.content_top,
                    });
            }
            down_list.scrollbar_drag_started();
            *down_drag.lock().expect("minimap drag state poisoned") = Some(MinimapDragSession {
                start_pointer_y: local_y,
                start_thumb_top: drag_start.0,
                start_ratio: drag_start.1,
                current_thumb_top: drag_start.0,
            });
        })
        .on_scroll_wheel(move |event, window, cx| {
            let delta_y = f32::from(event.delta.pixel_delta(px(SCROLL_WHEEL_LINE_PX)).y);
            if delta_y.abs() <= f32::EPSILON {
                return;
            }
            cx.stop_propagation();
            if scroll_drag
                .lock()
                .expect("minimap drag state poisoned")
                .is_some()
            {
                return;
            }
            let index = scroll_line_index
                .lock()
                .expect("active line index poisoned")
                .clone();
            let Some(index) = index else {
                return;
            };
            let target = scroll_ratio_after_wheel(&index, &scroll_list, delta_y);
            scroll_list_to_ratio(&index, &scroll_list, target);
            window.refresh();
        })
        .child(content)
        .child(
            div()
                .id("document-minimap-resize-handle")
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(MINIMAP_RESIZE_HANDLE_PX))
                .cursor(CursorStyle::ResizeLeftRight)
                .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    cx.stop_propagation();
                    if event.click_count >= 2 {
                        handle_resize
                            .lock()
                            .expect("minimap resize state poisoned")
                            .take();
                        handle_width_change(MinimapWidthChange::Reset, window, cx);
                    } else {
                        *handle_resize
                            .lock()
                            .expect("minimap resize state poisoned") =
                            Some(MinimapResizeSession {
                                start_pointer_x: f32::from(event.position.x),
                                start_width: minimap_width,
                            });
                        handle_width_change(
                            MinimapWidthChange::Preview(minimap_width),
                            window,
                            cx,
                        );
                    }
                    window.set_window_cursor_style(CursorStyle::ResizeLeftRight);
                    window.refresh();
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::minimap_media_geometry;

    #[test]
    fn minimap_media_geometry_preserves_aspect_ratio_and_row_bounds() {
        let (x, y, width, height) =
            minimap_media_geometry(800.0, 400.0, 27.52, 3.8, 110.0, 20.0, 80.0).unwrap();
        assert!((width / height - 2.0).abs() < 0.001);
        assert_eq!(x, 5.0);
        assert_eq!(y, 20.0);
        assert!(x + width <= 105.0);
        assert!(y + height <= 100.0);

        let (_, tall_y, tall_width, tall_height) =
            minimap_media_geometry(200.0, 800.0, 27.52, 3.8, 110.0, -10.0, 60.0).unwrap();
        assert!((tall_width / tall_height - 0.25).abs() < 0.001);
        assert_eq!(tall_y, -10.0);
        assert!(tall_y + tall_height <= 50.001);
    }
}
