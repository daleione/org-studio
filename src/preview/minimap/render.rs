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

use gpui::{
    BorderStyle, Bounds, Corners, CursorStyle, DispatchPhase, ListOffset, ListState, MouseButton,
    MouseMoveEvent, MouseUpEvent, canvas, div, fill, outline, point, prelude::*, px, rgba,
};

use crate::theme::current_theme;

use super::projection::MinimapRefinement;
use super::{
    MINIMAP_RESIZE_HANDLE_PX, MinimapDensity, MinimapDragSession, MinimapInteractionAnchor,
    MinimapLineIndex, MinimapProjectionReadiness, MinimapResizeSession, MinimapState,
    MinimapWidthChange, PreviewDisplayMap, PreviewLineKind, RASTER_TILE_ROWS, RasterRow,
    RasterTileKey, RasterTilePaint, RasterTileRequest, SCROLL_WHEEL_LINE_PX,
    current_resize_session, display_window_range, folded_signature, minimap_anchor_for_thumb_top,
    minimap_click_target_for_viewport, minimap_drag_target, minimap_perf_enabled,
    minimap_thumb_for_drag, minimap_trace_enabled, minimap_viewport_for_list_with_anchor,
    rasterize_tile, scroll_list_to_ratio, scroll_ratio_after_wheel, source_target_for_list_offset,
    take_resize_session, thumb_alphas, tile_key, width_from_resize_drag,
};

#[allow(clippy::too_many_arguments)]
pub fn render(
    model: Arc<PreviewDisplayMap>,
    state: Arc<MinimapState>,
    presentation_rows: Arc<Vec<usize>>,
    folded: Arc<HashSet<u32>>,
    list_state: ListState,
    editor_width: f32,
    minimap_width: f32,
    thumb_visibility: crate::settings::MinimapThumbVisibility,
    generation: u64,
    presentation_revision: u64,
    opened_at: Instant,
    on_seek: impl Fn(
        super::viewport::MinimapSourceTarget,
        ListOffset,
        &mut gpui::Window,
        &mut gpui::App,
    ) + 'static,
    on_width_change: impl Fn(MinimapWidthChange, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let theme = current_theme();
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
            let capacity = (f32::from(bounds.size.height) / minimap_line_height)
                .floor()
                .max(1.0) as usize;
            let width = f32::from(bounds.size.width).ceil().max(1.0) as usize;
            let scale_factor = window.scale_factor().max(1.0);
            let parent_width = (editor_width - 110.0 - minimap_width).max(120.0);
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
                    density,
                    MinimapRefinement {
                        priority_row,
                        allow: !interaction_active,
                        fold_revision: presentation_revision,
                    },
                    window.text_system(),
                )
            };
            if projection.readiness != MinimapProjectionReadiness::Exact && !interaction_active {
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
            let content_line = viewport.content_top.floor() as usize;
            let content_fraction = viewport.content_top.fract();
            let (anchor_row, anchor_inner_line) = line_index.locate(content_line);
            let visible_range = display_window_range(
                shape_rows.len(),
                anchor_row,
                anchor_inner_line,
                capacity,
                0,
                |index| {
                    shape_model
                        .display_lines(shape_rows[index], parent_width, window.text_system())
                        .ranges
                        .len()
                },
            );
            let first_line = visible_range.rows.start;
            let last_line = visible_range.rows.end;
            let first_tile = first_line / RASTER_TILE_ROWS * RASTER_TILE_ROWS;
            let mut tiles: SmallVec<[RasterTilePaint; 6]> = SmallVec::new();
            let mut raster_requests: SmallVec<[RasterTileRequest; 6]> = SmallVec::new();
            let mut visible_keys: SmallVec<[RasterTileKey; 6]> = SmallVec::new();
            let mut tile_y = minimap_edge_padding - content_fraction * minimap_line_height;
            for tile_start in (first_tile..last_line).step_by(RASTER_TILE_ROWS) {
                let tile_end = (tile_start + RASTER_TILE_ROWS).min(shape_rows.len());
                let tile_rows = &shape_rows[tile_start..tile_end];
                let mut wrap_hasher = std::collections::hash_map::DefaultHasher::new();
                shape_model.projection.revision.hash(&mut wrap_hasher);
                parent_width.to_bits().hash(&mut wrap_hasher);
                let raster_rows = tile_rows
                    .iter()
                    .map(|&row| {
                        let mut lines =
                            shape_model.display_lines(row, parent_width, window.text_system());
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
                        RasterRow {
                            document_index: row,
                            lines,
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
                    for range in lines.ranges.iter() {
                        range.start.hash(&mut wrap_hasher);
                        range.end.hash(&mut wrap_hasher);
                    }
                }
                let tile_line_count = raster_rows.iter().map(RasterRow::line_count).sum::<usize>();
                if tile_start == first_tile {
                    let hidden_lines = raster_rows
                        .iter()
                        .take(first_line.saturating_sub(tile_start))
                        .map(RasterRow::line_count)
                        .sum::<usize>();
                    tile_y -= (hidden_lines + visible_range.skip_display_lines) as f32
                        * minimap_line_height;
                }
                let tile_height = (tile_line_count as f32 * minimap_line_height)
                    .ceil()
                    .max(1.0);
                let key = tile_key(
                    &tile_identity_rows,
                    tile_start,
                    width,
                    folded_signature(&shape_model, tile_rows, &shape_folded),
                    wrap_hasher.finish(),
                    scale_factor,
                    density,
                );
                visible_keys.push(key);
                let (image, is_missing) = shape_state
                    .raster_tiles
                    .lock()
                    .expect("minimap raster tile cache poisoned")
                    .image_or_fallback(key);
                if let Some(image) = image {
                    tiles.push(RasterTilePaint {
                        image,
                        y: tile_y,
                        width: width as f32,
                        height: tile_height,
                    });
                }
                if is_missing {
                    raster_requests.push(RasterTileRequest {
                        key,
                        rows: raster_rows,
                    });
                }
                tile_y += tile_height;
            }
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
            if should_spawn {
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
                            parent_width,
                            &tile_folded,
                            scale_factor,
                            density,
                        );
                        if minimap_perf_enabled() {
                            let first = !tile_state
                                .perf
                                .first_tile_completed
                                .swap(true, Ordering::AcqRel);
                            eprintln!(
                                "org_studio_minimap_tile_ready generation={} revision={} tile_start={} rows={} lines={} width={} total_ms={:.3} text_system_wait_ms={:.3} cold_text_system={} first={} since_open_ms={:.3}",
                                generation,
                                presentation_revision,
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
                    let completed_count = completed.len();
                    tile_state
                        .raster_tiles
                        .lock()
                        .expect("minimap raster tile cache poisoned")
                        .insert_batch(completed, &visible_keys);
                    // Publish a complete visible minimap frame as one unit. The preview keeps its
                    // loading cover in place until this release store, so users never see the
                    // document appear first and the minimap fill one or two frames later.
                    tile_state
                        .initial_visible_batch_ready
                        .store(true, Ordering::Release);
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
            tiles
        },
        move |bounds, tiles: SmallVec<[RasterTilePaint; 6]>, window, cx| {
            profiling::scope!("Minimap::paint");
            if !tiles.is_empty()
                && !paint_state
                    .perf
                    .first_pixels_painted
                    .swap(true, Ordering::AcqRel)
            {
                if minimap_perf_enabled() {
                    eprintln!(
                        "org_studio_minimap_first_pixels generation={} revision={} tiles={} since_open_ms={:.3}",
                        generation,
                        presentation_revision,
                        tiles.len(),
                        opened_at.elapsed().as_secs_f64() * 1000.0,
                    );
                    eprintln!(
                        "org_preview_coherent_first_frame generation={} revision={} since_open_ms={:.3}",
                        generation,
                        presentation_revision,
                        opened_at.elapsed().as_secs_f64() * 1000.0,
                    );
                }
                if std::env::var_os("ORG_STUDIO_EXIT_AFTER_MINIMAP_FRAME").is_some() {
                    cx.quit();
                }
            }
            for tile in tiles {
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
            let thumb_bounds = Bounds::new(
                point(bounds.origin.x, bounds.origin.y + px(thumb.top)),
                gpui::size(bounds.size.width, px(thumb.height)),
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
                rgba((theme.foreground << 8) | fill_alpha),
            ));
            window.paint_quad(outline(
                thumb_bounds,
                rgba((theme.foreground << 8) | border_alpha),
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
        .border_color(gpui::rgb(theme.border))
        .bg(gpui::rgb(theme.background_alt))
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
                *down_anchor.lock().expect("minimap anchor poisoned") =
                    Some(MinimapInteractionAnchor {
                        layout: index.layout,
                        width: index.width,
                        rows_signature: index.rows_signature,
                        interaction_height: viewport.interaction_height,
                        content_top: viewport.content_top,
                    });
                if minimap_trace_enabled() {
                    eprintln!(
                        "minimap click display={:.3} target_ratio={:.5} anchor_content={:.3} anchor_thumb={:.2}",
                        target.clicked_display,
                        target.ratio,
                        viewport.content_top,
                        target.thumb_top,
                    );
                }
                if target.ratio >= 1.0 - f32::EPSILON && down_list.item_count() > 0 {
                    down_list.scroll_to(ListOffset {
                        item_ix: down_list.item_count() - 1,
                        offset_in_item: px(0.0),
                    });
                } else {
                    down_list.scroll_to(target.offset);
                }
                drag_start = (target.thumb_top, target.ratio);
                window.refresh();
            } else {
                drag_start = (thumb.top, viewport.scroll_ratio);
                *down_anchor.lock().expect("minimap anchor poisoned") =
                    Some(MinimapInteractionAnchor {
                        layout: index.layout,
                        width: index.width,
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
