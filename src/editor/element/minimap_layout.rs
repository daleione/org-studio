//! Background minimap layout preparation: request construction and the
//! off-thread pass that shapes the full document for the minimap.

use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Pixels, TextRun, point, px};

use crate::document::{LineIndex, TextSnapshot};
use crate::editor::{SemanticEditor, layout_map::EditorLayoutMap, syntax};

use super::blocks::editor_block_text_inset;
use super::rows::visible_source_lines;
use super::scroll::{scroll_is_at_end, stabilized_scroll_y};
use super::text::{folded_display_text, table_visual_layout};
use super::{
    BLOCK_RIGHT_INSET, BLOCK_TEXT_RIGHT_PADDING, INLINE_IMAGE_MAX_WIDTH,
    INLINE_IMAGE_VERTICAL_PADDING,
};

pub(super) struct MinimapLayoutPreparationRequest {
    pub(super) key: crate::editor::minimap::PreparedLayoutKey,
    pub(super) epoch: u64,
    pub(super) cancellation_epoch: Arc<std::sync::atomic::AtomicU64>,
    pub(super) layout: EditorLayoutMap,
    pub(super) snapshot: crate::document::DocumentSnapshot,
    pub(super) syntax_service: Arc<syntax::EditorSyntaxService>,
    pub(super) text_system: Arc<gpui::TextSystem>,
    pub(super) font: gpui::Font,
    pub(super) font_size: Pixels,
    pub(super) theme: crate::theme::Theme,
    pub(super) fold_markers: Arc<std::collections::HashSet<u64>>,
}

pub(super) fn prepare_minimap_layout_request(
    editor: &SemanticEditor,
    path: &std::path::Path,
    snapshot: &crate::document::DocumentSnapshot,
    wrap_width: f32,
    font: gpui::Font,
    font_size: Pixels,
    theme: &crate::theme::Theme,
    text_system: Arc<gpui::TextSystem>,
) -> Option<MinimapLayoutPreparationRequest> {
    if !editor.minimap.visible && !editor.layout_reflow_pending || editor.fold_animation.is_some() {
        return None;
    }
    let mut inline_image_overrides = editor
        .inline_image_preview_overrides
        .iter()
        .map(|(&line_start, &enabled)| (line_start, enabled))
        .collect::<Vec<_>>();
    inline_image_overrides.sort_unstable_by_key(|entry| entry.0);
    let key = crate::editor::minimap::PreparedLayoutKey {
        revision: snapshot.revision(),
        path: path.to_path_buf(),
        line_count: snapshot.len_lines(),
        wrap_width_bits: wrap_width.to_bits(),
        base_line_height_bits: editor.display_map.base_line_height().to_bits(),
        soft_wrap: editor.display_map.soft_wrap(),
        font: font.clone(),
        font_size_bits: f32::from(font_size).to_bits(),
        content_scale_bits: editor.content_font_size().scale().to_bits(),
        fold_revision: editor.fold_animation_revision,
        inline_images: editor.inline_image_previews,
        inline_image_overrides: inline_image_overrides.into(),
        inline_image_resource_generation: editor.inline_image_cache.borrow().resource_generation,
    };
    let epoch = editor.minimap.reserve_layout_preparation(key.clone())?;
    let mut layout = editor.display_map.clone();
    layout.configure(snapshot.len_lines(), wrap_width);
    Some(MinimapLayoutPreparationRequest {
        key,
        epoch,
        cancellation_epoch: editor.minimap.layout_preparation_epoch(),
        layout,
        snapshot: snapshot.clone(),
        syntax_service: editor.syntax_service.clone(),
        text_system,
        font,
        font_size,
        theme: *theme,
        fold_markers: editor.fold_markers.clone(),
    })
}

pub(super) fn schedule_minimap_layout_preparation(
    editor: gpui::Entity<SemanticEditor>,
    request: MinimapLayoutPreparationRequest,
    cx: &mut App,
) {
    const YIELD_LINE_INTERVAL: u64 = 128;

    let key = request.key.clone();
    let epoch = request.epoch;
    let cancellation_epoch = request.cancellation_epoch.clone();
    let scheduler = cx.background_executor().clone();
    let background = cx.background_executor().spawn(async move {
        let started_at = std::time::Instant::now();
        let text_system = gpui::WindowTextSystem::new(request.text_system.clone());
        let mut layout = request.layout;
        let line_count = request.snapshot.len_lines();
        if line_count > 0 {
            let last = [line_count - 1];
            let tail = syntax::SparseEditorStyleSnapshot::query_lines(
                &request.key.path,
                &request.snapshot,
                &last,
                &request.syntax_service,
            );
            if tail.pending {
                // A visible-line request may already own the builder token. It also observes the
                // furthest requested line, so completing it here is safe and guarantees this
                // supposedly complete geometry never falls back to plain-line metrics.
                request
                    .syntax_service
                    .build_focused(&request.key.path, &request.snapshot);
            }
        }

        let base_run_color = gpui::rgb(request.theme.foreground).into();
        let content_scale = f32::from_bits(request.key.content_scale_bits);
        let overrides = request
            .key
            .inline_image_overrides
            .iter()
            .copied()
            .collect::<std::collections::HashMap<_, _>>();
        let mut aligned_tables = std::collections::HashMap::<u64, Option<Arc<[usize]>>>::new();
        let document_format = crate::document::DocumentFormat::from_path(&request.key.path);

        for chunk_start in (0..line_count).step_by(YIELD_LINE_INTERVAL as usize) {
            if cancellation_epoch.load(std::sync::atomic::Ordering::Acquire) != epoch {
                return None;
            }
            let chunk_end = (chunk_start + YIELD_LINE_INTERVAL).min(line_count);
            let source_lines = visible_source_lines(&layout, chunk_start..chunk_end);
            let mut style_query = syntax::SparseEditorStyleSnapshot::query_lines(
                &request.key.path,
                &request.snapshot,
                &source_lines,
                &request.syntax_service,
            );
            if style_query.pending {
                request
                    .syntax_service
                    .build_focused(&request.key.path, &request.snapshot);
                style_query = syntax::SparseEditorStyleSnapshot::query_lines(
                    &request.key.path,
                    &request.snapshot,
                    &source_lines,
                    &request.syntax_service,
                );
                if style_query.pending {
                    return None;
                }
            }
            let styles = style_query.snapshot;

            for line_number in source_lines {
                let line = LineIndex(line_number);
                let Ok(source_range) = request.snapshot.line_content_range(line) else {
                    continue;
                };
                let Some(source_line) = layout.source_line(&request.snapshot, line, None) else {
                    continue;
                };
                let folded = request.fold_markers.contains(&line_number);
                let fallback_style;
                let line_style = if let Some(style) = styles.line(line_number) {
                    style
                } else {
                    fallback_style = syntax::EditorLineStyle::pending_fallback(source_range);
                    &fallback_style
                };
                let text_inset = editor_block_text_inset(line_style.block.as_ref());
                let row_wrap_width = if text_inset > 0.0 {
                    (layout.wrap_width()
                        - text_inset
                        - BLOCK_RIGHT_INSET
                        - BLOCK_TEXT_RIGHT_PADDING)
                        .max(1.0)
                } else {
                    layout.wrap_width()
                };
                let mut metrics = line_style.metrics.scaled(content_scale);
                let image_enabled = overrides
                    .get(&source_range.start.0)
                    .copied()
                    .unwrap_or(request.key.inline_images);
                let inline_image_height = (!folded && image_enabled)
                    .then(|| crate::org_syntax::standalone_image_path(&source_line.display.text))
                    .flatten()
                    .and_then(|target| {
                        let path = crate::preview::resolve_image_path(&request.key.path, target);
                        crate::preview::image_dimensions(&path).ok()
                    })
                    .map(|(width, height)| {
                        crate::preview::fitted_image_size(
                            width,
                            height,
                            layout.wrap_width().min(INLINE_IMAGE_MAX_WIDTH),
                        )
                        .1
                    });

                let (visual_rows, wrap_starts) = if let Some(height) = inline_image_height {
                    metrics.before = INLINE_IMAGE_VERTICAL_PADDING;
                    metrics.line_height = height;
                    metrics.after = INLINE_IMAGE_VERTICAL_PADDING;
                    (1, Vec::new())
                } else {
                    let display = source_line.display;
                    let text: gpui::SharedString =
                        folded_display_text(display.text.clone(), folded).into();
                    let runs = syntax::runs(
                        &request.key.path,
                        &text,
                        TextRun {
                            len: text.len(),
                            font: request.font.clone(),
                            color: base_run_color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        },
                        line_style,
                        None,
                        &request.theme,
                    );
                    let table_columns = (line_style.id == syntax::EditorStyleId::Table)
                        .then(|| {
                            crate::editor::org_commands::table_start(
                                &request.snapshot,
                                line_number,
                                document_format,
                            )
                        })
                        .flatten()
                        .and_then(|start| {
                            aligned_tables
                                .entry(start)
                                .or_insert_with(|| {
                                    crate::editor::org_commands::aligned_table_column_widths(
                                        &request.snapshot,
                                        start,
                                        document_format,
                                    )
                                    .map(Arc::<[usize]>::from)
                                })
                                .clone()
                        });
                    let shaped_font_size = px(f32::from(request.font_size) * metrics.font_scale);
                    let table_fits = table_columns
                        .as_ref()
                        .and_then(|columns| {
                            table_visual_layout(
                                &text,
                                &runs,
                                shaped_font_size,
                                columns,
                                document_format,
                                &text_system,
                            )
                        })
                        .is_some_and(|layout| f32::from(layout.width) <= row_wrap_width);
                    let effective_wrap_width =
                        (request.key.soft_wrap && !table_fits).then_some(px(row_wrap_width));
                    let wrapped = text_system
                        .shape_text(text, shaped_font_size, &runs, effective_wrap_width, None)
                        .ok()
                        .and_then(|lines| lines.into_iter().next())
                        .unwrap_or_default();
                    let visual_rows = wrapped.wrap_boundaries().len() + 1;
                    let wrap_starts = (1..visual_rows)
                        .map(|visual_row| {
                            let display_index = wrapped
                                .index_for_position(
                                    point(px(0.0), px(visual_row as f32 * metrics.line_height)),
                                    px(metrics.line_height),
                                )
                                .unwrap_or_else(|index| index);
                            display.display_to_source(display_index)
                        })
                        .collect::<Vec<_>>();
                    (visual_rows, wrap_starts)
                };
                layout.update_line_layout(
                    line_number,
                    visual_rows,
                    metrics.line_height,
                    metrics.before,
                    metrics.after,
                );
                layout.update_line_wrap_starts(line_number, &wrap_starts);
            }
            scheduler.timer(Duration::from_millis(0)).await;
        }
        Some((Arc::new(layout), started_at.elapsed()))
    });

    cx.spawn(async move |cx| {
        let prepared = background.await;
        editor.update(cx, |editor, cx| {
            let Some((layout, elapsed)) = prepared else {
                if editor.minimap.abandon_layout_preparation(&key, epoch) {
                    cx.notify();
                }
                return;
            };
            if editor
                .minimap
                .publish_prepared_layout(&key, epoch, layout.clone())
            {
                // Once complete geometry exists, make it authoritative for both the Editor and
                // its minimap. Keeping the live Editor sparse would require two scroll cameras
                // and gives the thumb/background a different follow policy from Reading.
                let viewport_height = editor
                    .viewport
                    .map_or(0.0, |viewport| f32::from(viewport.size.height));
                let was_at_end = editor.scroll_at_end
                    || scroll_is_at_end(
                        editor.scroll_y,
                        viewport_height,
                        editor.animated_document_height(),
                    );
                let anchor_line = editor.animated_line_at_y(editor.scroll_y);
                let anchor_start = editor.animated_line_start_y(anchor_line);
                let anchor_fraction = ((editor.scroll_y - anchor_start)
                    / editor.animated_line_height_px(anchor_line).max(1.0))
                .clamp(0.0, 1.0);
                editor.display_map = layout.as_ref().clone();
                editor.layout_reflow_pending = false;
                let anchored = editor.animated_line_start_y(anchor_line)
                    + anchor_fraction * editor.animated_line_height_px(anchor_line);
                editor.scroll_y = stabilized_scroll_y(
                    was_at_end,
                    anchored,
                    viewport_height,
                    editor.animated_document_height(),
                );
                editor.scroll_at_end = was_at_end;
                editor.hit_rows = Arc::from([]);
                if std::env::var_os("ORG_STUDIO_EDITOR_MINIMAP_PERF").is_some() {
                    eprintln!(
                        "org_editor_minimap_layout_ready host_id={} lines={} elapsed_ms={:.3}",
                        editor.minimap.telemetry.host_id(),
                        key.line_count,
                        elapsed.as_secs_f64() * 1_000.0,
                    );
                }
                editor.minimap.note_viewport_changed();
                editor.minimap.invalidate_raster();
                cx.notify();
            }
        });
    })
    .detach();
}
