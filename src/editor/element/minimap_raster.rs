//! Minimap raster scheduling: tile requests, media dimension commits and the
//! text-row recipe handed to the rasterizer.

use std::sync::Arc;

use gpui::App;

use crate::document::{ByteRange, LineIndex, TextSnapshot};
use crate::editor::{
    SemanticEditor, layout_map::EditorLayoutMap,
    minimap_media::image_path as editor_minimap_image_path, syntax,
};

use super::blocks::editor_block_accent;
use super::minimap::MinimapMediaCandidate;
use super::scroll::{scroll_is_at_end, stabilized_scroll_y};
use super::{INLINE_IMAGE_MAX_WIDTH, INLINE_IMAGE_VERTICAL_PADDING};

pub(super) struct MinimapRasterRequest {
    pub(super) key: crate::editor::minimap::RasterKey,
    pub(super) layout: Arc<EditorLayoutMap>,
    pub(super) path: std::path::PathBuf,
    pub(super) snapshot: crate::document::DocumentSnapshot,
    pub(super) source_lines: Vec<u64>,
    pub(super) raster_lines: Vec<Option<crate::editor::minimap::RasterSourceRow>>,
    pub(super) media_candidates: Vec<MinimapMediaCandidate>,
    pub(super) syntax_service: Arc<syntax::EditorSyntaxService>,
    pub(super) theme: crate::theme::Theme,
    pub(super) content_top: f32,
    pub(super) viewport_generation: u64,
    pub(super) scale_factor: f32,
    pub(super) density: crate::minimap::Density,
    pub(super) line_height: f32,
    pub(super) visible_rows: usize,
    pub(super) repeated_rows: usize,
    pub(super) telemetry: Arc<crate::editor::minimap::EditorMinimapTelemetry>,
    pub(super) epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub(super) expected_epoch: u64,
}

pub(super) fn apply_editor_minimap_media_dimensions(
    editor: &mut SemanticEditor,
    media: &[crate::editor::minimap::RasterMedia],
) -> bool {
    let discovered = media
        .iter()
        .filter_map(|source| {
            source
                .dimensions
                .map(|dimensions| (source.line, source.line_start, dimensions))
        })
        .collect::<Vec<_>>();
    let changed_dimensions = {
        let mut known = editor.inline_image_line_dimensions.borrow_mut();
        discovered
            .into_iter()
            .filter(|(line, line_start, (width, height))| {
                if known.get(line) == Some(&(*line_start, *width, *height)) {
                    false
                } else {
                    known.insert(*line, (*line_start, *width, *height));
                    true
                }
            })
            .collect::<Vec<_>>()
    };
    if changed_dimensions.is_empty() {
        return false;
    }

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
    let wrap_width = editor.display_map.wrap_width().min(INLINE_IMAGE_MAX_WIDTH);
    let mut layout_changed = false;
    for (line, _, (width, height)) in changed_dimensions {
        let (_, fitted_height) = crate::preview::fitted_image_size(width, height, wrap_width);
        layout_changed |= editor.display_map.update_line_layout(
            line,
            1,
            fitted_height,
            INLINE_IMAGE_VERTICAL_PADDING,
            INLINE_IMAGE_VERTICAL_PADDING,
        );
    }
    if !layout_changed {
        return false;
    }

    let anchored = editor.animated_line_start_y(anchor_line)
        + anchor_fraction * editor.animated_line_height_px(anchor_line);
    editor.scroll_y = stabilized_scroll_y(
        was_at_end,
        anchored,
        viewport_height,
        editor.animated_document_height(),
    );
    editor.scroll_at_end = was_at_end;
    true
}

pub(super) fn schedule_minimap_raster(
    editor: gpui::Entity<SemanticEditor>,
    request: MinimapRasterRequest,
    cx: &mut App,
) {
    let key = request.key;
    let layout = request.layout.clone();
    let content_top = request.content_top;
    let viewport_generation = request.viewport_generation;
    let line_height = request.line_height;
    let publish_telemetry = request.telemetry.clone();
    let background = cx.background_executor().spawn(async move {
        let host_id = request.telemetry.host_id();
        let prepare_started = std::time::Instant::now();
        let _prepare = tracing::info_span!("editor_minimap_prepare", host_id).entered();
        if request.epoch.load(std::sync::atomic::Ordering::Acquire) != request.expected_epoch {
            return None;
        }
        let mut semantics = syntax::SparseEditorStyleSnapshot::query_lines(
            &request.path,
            &request.snapshot,
            &request.source_lines,
            &request.syntax_service,
        );
        if semantics.pending {
            if !semantics.start_builder {
                return None;
            }
            request
                .syntax_service
                .build_focused(&request.path, &request.snapshot);
            semantics = syntax::SparseEditorStyleSnapshot::query_lines(
                &request.path,
                &request.snapshot,
                &request.source_lines,
                &request.syntax_service,
            );
            if semantics.pending {
                return None;
            }
        }
        debug_assert_eq!(semantics.snapshot.revision, request.snapshot.revision());
        let mut rich_span_budget = crate::editor::minimap::RichSpanBudget::default();
        let mut media = Vec::new();
        let mut image_lines = std::collections::HashSet::new();
        for candidate in &request.media_candidates {
            let range = request
                .snapshot
                .line_content_range(LineIndex(candidate.line))
                .ok();
            let text = range
                .map(|range| bounded_minimap_text(&request.snapshot, range, 1_024))
                .unwrap_or_default();
            if let Some(path) = editor_minimap_image_path(&request.path, &text) {
                image_lines.insert(candidate.line);
                let dimensions = crate::preview::image_dimensions(&path).ok();
                media.push(crate::editor::minimap::RasterMedia {
                    line: candidate.line,
                    line_start: candidate.line_start,
                    row_offset_units: candidate.row_offset_units,
                    row_height_units: candidate.row_height_units,
                    path,
                    dimensions,
                });
            }
        }
        let document_format = crate::document::DocumentFormat::from_path(&request.path);
        let rows = request
            .raster_lines
            .iter()
            .map(|source| {
                let Some(source) = *source else {
                    return minimap_text_row(
                        String::new(),
                        None,
                        Vec::new(),
                        &mut rich_span_budget,
                        &request.theme,
                    );
                };
                let line = source.line;
                let complete_source_line = source.text_range == Some((0, None));
                let mut text = request
                    .snapshot
                    .line_content_range(LineIndex(line))
                    .ok()
                    .map(|range| bounded_minimap_text(&request.snapshot, range, 1_024))
                    .unwrap_or_default();
                if image_lines.contains(&line) {
                    text.clear();
                } else if let Some((start, end)) = source.text_range {
                    let start = start as usize;
                    let end = end.map_or(text.len(), |end| end as usize).min(text.len());
                    if start < end && text.is_char_boundary(start) && text.is_char_boundary(end) {
                        text = text[start..end].to_owned();
                    } else {
                        text.clear();
                    }
                } else {
                    text.clear();
                }
                let line_style = semantics.snapshot.line(line);
                let spans = line_style
                    .map(|style| syntax::semantic_spans(&request.path, &text, style))
                    .unwrap_or_default();
                let mut row = minimap_text_row(
                    text,
                    line_style,
                    spans,
                    &mut rich_span_budget,
                    &request.theme,
                );
                row.table = if complete_source_line {
                    line_style
                        .filter(|style| style.id == syntax::EditorStyleId::Table)
                        .and_then(|_| {
                            crate::editor::minimap::TableRowGeometry::from_source(
                                &row.text,
                                document_format,
                            )
                        })
                } else {
                    None
                };
                row
            })
            .collect::<Vec<_>>();
        if rich_span_budget.degraded_rows() > 0 {
            tracing::info!(
                minimap_rich_span_degraded_rows = rich_span_budget.degraded_rows(),
                minimap_rich_span_degraded_rows_total =
                    crate::editor::minimap::rich_span_degraded_rows(),
                "editor minimap rich spans degraded to base style"
            );
        }
        let degraded_rows = rich_span_budget.degraded_rows();
        let prepare_elapsed = prepare_started.elapsed();
        drop(_prepare);
        let raster_started = std::time::Instant::now();
        let _raster = tracing::info_span!("editor_minimap_raster", host_id).entered();
        let rasterized = crate::editor::minimap::rasterize_text_rows(
            &rows,
            usize::from(key.width),
            request.scale_factor,
            request.density,
            request.line_height,
            &request.epoch,
            request.expected_epoch,
        );
        let raster_elapsed = raster_started.elapsed();
        if let Some(rasterized) = &rasterized {
            request.telemetry.report_job(
                rows.len(),
                request.visible_rows,
                request.repeated_rows,
                degraded_rows,
                prepare_elapsed,
                raster_elapsed,
                rasterized.rasterizer_lock_wait,
            );
        }
        rasterized.map(|rasterized| (rasterized.image, Arc::<[_]>::from(media)))
    });
    cx.spawn(async move |cx| {
        let raster = background.await;
        editor.update(cx, |editor, cx| {
            let host_id = publish_telemetry.host_id();
            let _publish = tracing::info_span!("editor_minimap_publish", host_id).entered();
            let mut in_flight = editor
                .minimap
                .raster_build
                .lock()
                .expect("editor minimap raster build poisoned");
            if *in_flight != Some(key) {
                return;
            }
            let Some((image, media)) = raster else {
                *in_flight = None;
                return;
            };
            *in_flight = None;
            drop(in_flight);

            // A layout request may have superseded this raster after it started.
            if editor.minimap.active_frame().is_some()
                && editor.minimap.layout_preparation_pending()
            {
                cx.notify();
                return;
            }

            // The minimap scans a bounded window ahead of the Editor viewport.
            // Resolve image dimensions there and commit the corresponding Editor
            // row height before publishing the raster. The next raster is then
            // built against final visual units, so reaching the image cannot make
            // the whole minimap shift when the inline image is loaded later.
            if apply_editor_minimap_media_dimensions(editor, &media) {
                editor.minimap.note_layout_changed();
                cx.notify();
                return;
            }
            if editor.minimap.active_frame().is_some()
                && !editor.minimap.background_camera_movable()
                && editor
                    .minimap
                    .is_layout_refinement_generation(key.generation)
            {
                // A measurement-only candidate can contain better soft-wrap geometry, but
                // replacing the active frame while its background crop is pinned to an edge
                // makes the whole background visibly jump. Keep it deferred until that crop is
                // moving and can hand its source anchor to the coherent candidate.
                cx.notify();
                return;
            }
            editor
                .minimap
                .publish_frame(crate::editor::minimap::PreparedEditorMinimapFrame::new(
                    layout,
                    crate::editor::minimap::CachedRaster {
                        key,
                        image,
                        media,
                        content_top,
                        viewport_generation,
                        line_height,
                    },
                ));
            publish_telemetry.note_publish();
            cx.notify();
        });
    })
    .detach();
}

pub(super) fn bounded_minimap_text(
    snapshot: &crate::document::DocumentSnapshot,
    range: ByteRange,
    byte_limit: u64,
) -> String {
    let mut end = (range.start.0 + byte_limit).min(range.end.0);
    while end > range.start.0 && !snapshot.is_char_boundary(crate::document::ByteOffset(end)) {
        end -= 1;
    }
    snapshot.copy_range(ByteRange::new(range.start.0, end))
}

pub(super) fn minimap_text_color(kind: syntax::EditorStyleId, theme: &crate::theme::Theme) -> u32 {
    match kind {
        syntax::EditorStyleId::Heading(level) => {
            theme.heading[(level.saturating_sub(1) as usize).min(3)]
        }
        syntax::EditorStyleId::CodeBoundary | syntax::EditorStyleId::Meta => theme.meta,
        syntax::EditorStyleId::Code => theme.code_foreground,
        syntax::EditorStyleId::Quote => theme.quote,
        syntax::EditorStyleId::Property => theme.attribute,
        syntax::EditorStyleId::Comment => theme.comment,
        syntax::EditorStyleId::Table => theme.link,
        syntax::EditorStyleId::List | syntax::EditorStyleId::Plain => theme.foreground,
    }
}

pub(super) fn minimap_text_row(
    text: String,
    line_style: Option<&syntax::EditorLineStyle>,
    spans: Vec<syntax::EditorSemanticSpan>,
    rich_span_budget: &mut crate::editor::minimap::RichSpanBudget,
    theme: &crate::theme::Theme,
) -> crate::editor::minimap::TextRow {
    let block = line_style.and_then(|style| style.block.as_ref());
    let rich_spans = rich_span_budget.adapt(&text, spans, theme);
    crate::editor::minimap::TextRow {
        color: line_style.map_or(theme.foreground, |style| {
            minimap_text_color(style.id, theme)
        }),
        weight: if matches!(
            line_style.map(|style| style.id),
            Some(syntax::EditorStyleId::Heading(_))
        ) {
            crate::editor::minimap::TextWeight::Bold
        } else {
            crate::editor::minimap::TextWeight::Semibold
        },
        italic: matches!(
            line_style.map(|style| style.id),
            Some(syntax::EditorStyleId::Comment)
        ),
        spans: rich_spans,
        text,
        indent: if block.is_some() { 5.0 } else { 3.0 },
        block_background: block.map(|_| theme.code_background),
        block_accent: block.map(|block| editor_block_accent(&block.kind, theme)),
        block_edge: block.map(|block| block.edge),
        table: None,
    }
}
