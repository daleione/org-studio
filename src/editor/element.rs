#[cfg(feature = "benchmarks")]
use std::time::Instant;
use std::{ops::Range, sync::Arc};

use gpui::{
    App, BorderStyle, Bounds, ContentMask, Corners, Element, ElementId, ElementInputHandler,
    GlobalElementId, LayoutId, PaintQuad, Pixels, RenderImage, ShapedLine, Style, TextAlign,
    TextRun, Window, WrappedLine, fill, outline, point, px, relative, rgba, size,
};

use crate::{
    document::{ByteRange, LineIndex, TextSnapshot},
    theme::current_theme,
};

#[cfg(feature = "benchmarks")]
use super::FrameBenchmarkAction;
use super::{HitRow, SemanticEditor, ShapeKey, layout_map::EditorLayoutMap, syntax};

const GUTTER_PADDING: f32 = 16.0;
const MAX_ANIMATED_PAINT_LINES: u64 = 192;

pub struct EditorElement {
    editor: gpui::Entity<SemanticEditor>,
}

impl EditorElement {
    pub fn new(editor: gpui::Entity<SemanticEditor>) -> Self {
        Self { editor }
    }
}

impl gpui::IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct PrepaintState {
    #[cfg(feature = "benchmarks")]
    started_at: Instant,
    rows: Vec<PaintRow>,
    selection: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
    gutter: PaintQuad,
    minimap: MinimapPaint,
}

struct MinimapPaint {
    bounds: Bounds<Pixels>,
    background: Option<PaintQuad>,
    image: Option<(Arc<RenderImage>, Bounds<Pixels>)>,
    overlays: Vec<PaintQuad>,
}

struct MinimapRasterRequest {
    key: super::minimap::RasterKey,
    rows: Vec<super::minimap::TextRow>,
    content_top: f32,
    viewport_generation: u64,
    scale_factor: f32,
    density: crate::minimap::Density,
    line_height: f32,
    epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    expected_epoch: u64,
}

struct PaintRow {
    hit: HitRow,
    gutter_layout: ShapedLine,
    shape_key: ShapeKey,
    visual_rows: usize,
    metrics: syntax::BlockMetrics,
    animated_height: f32,
    background: Option<PaintQuad>,
    animation_clip_y: Option<(Pixels, Pixels)>,
}

fn animated_paint_lines(
    display_map: &EditorLayoutMap,
    visible_lines: Range<u64>,
    fold_ranges: &[Range<u64>],
) -> Vec<u64> {
    if visible_lines.is_empty() {
        return Vec::new();
    }
    if fold_ranges.is_empty() {
        return visible_source_lines(display_map, visible_lines);
    }

    let mut unchanged = Vec::new();
    let mut animated_segments = Vec::new();
    let mut cursor = visible_lines.start;
    for range in fold_ranges {
        let start = range.start.max(visible_lines.start).min(visible_lines.end);
        let end = range.end.max(visible_lines.start).min(visible_lines.end);
        if cursor < start {
            unchanged.extend(visible_source_lines(display_map, cursor..start));
        }
        if start < end {
            let first = display_map.visible_ordinal_for_line(start);
            let last = display_map.visible_ordinal_for_line(end);
            if first < last {
                animated_segments.push((first, last));
            }
        }
        cursor = cursor.max(end);
    }
    if cursor < visible_lines.end {
        unchanged.extend(visible_source_lines(display_map, cursor..visible_lines.end));
    }

    let animated_count = animated_segments
        .iter()
        .map(|(start, end)| end - start)
        .sum::<u64>();
    if animated_count <= MAX_ANIMATED_PAINT_LINES {
        for (start, end) in animated_segments {
            unchanged.extend(visible_source_ordinals(display_map, start..end));
        }
    } else {
        for sample in 0..MAX_ANIMATED_PAINT_LINES {
            let rank = sample * (animated_count - 1) / (MAX_ANIMATED_PAINT_LINES - 1);
            let mut remaining = rank;
            for &(start, end) in &animated_segments {
                let count = end - start;
                if remaining < count {
                    if let Some(line) =
                        display_map.source_line_for_visible_ordinal(start + remaining)
                    {
                        unchanged.push(line);
                    }
                    break;
                }
                remaining -= count;
            }
        }
    }
    unchanged.sort_unstable();
    unchanged.dedup();
    unchanged
}

fn visible_source_lines(display_map: &EditorLayoutMap, lines: Range<u64>) -> Vec<u64> {
    let start = display_map.visible_ordinal_for_line(lines.start);
    let end = display_map.visible_ordinal_for_line(lines.end);
    visible_source_ordinals(display_map, start..end)
}

fn visible_source_ordinals(display_map: &EditorLayoutMap, ordinals: Range<u64>) -> Vec<u64> {
    ordinals
        .filter_map(|ordinal| display_map.source_line_for_visible_ordinal(ordinal))
        .collect()
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        #[cfg(feature = "benchmarks")]
        let started_at = Instant::now();
        let snapshot = self.editor.read(cx).snapshot(cx);
        self.editor.update(cx, |editor, _| {
            editor.minimap.update_snapshot(&snapshot);
            editor.minimap.scale_factor = window.scale_factor().max(1.0);
        });
        let digits = snapshot.len_lines().max(1).ilog10() + 1;
        let gutter_width = digits as f32 * 9.0 + GUTTER_PADDING * 2.0;
        let minimap_width = if self.editor.read(cx).minimap.visible {
            self.editor.read(cx).minimap.width
        } else {
            0.0
        };
        let wrap_width = (f32::from(bounds.size.width) - gutter_width - minimap_width).max(1.0);
        self.editor.update(cx, |editor, _| {
            let viewport_height = f32::from(bounds.size.height);
            let was_at_end = scroll_is_at_end(
                editor.scroll_y,
                viewport_height,
                editor.animated_document_height(),
            );
            let anchor_line = editor.animated_line_at_y(editor.scroll_y);
            let anchor_start = editor.animated_line_start_y(anchor_line);
            let anchor_height = editor.animated_line_height_px(anchor_line).max(1.0);
            let anchor_fraction =
                ((editor.scroll_y - anchor_start) / anchor_height).clamp(0.0, 1.0);
            let layout_reconfigured = editor
                .display_map
                .configure(snapshot.len_lines(), wrap_width);
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
        });
        let editor = self.editor.read(cx);
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
        let style_snapshot = syntax::SparseEditorStyleSnapshot::for_lines(
            editor.session.read(cx).path(),
            &snapshot,
            &paint_lines,
            &editor.syntax_cache,
        );
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
        let mut rows = Vec::with_capacity(paint_lines.len());
        let mut selection_quads = Vec::new();
        let mut caret = None;
        let mut next_y = first_line_y;
        let minimap_bounds = Bounds::new(
            point(bounds.right() - px(minimap_width), bounds.top()),
            size(px(minimap_width), bounds.size.height),
        );

        for line_number in paint_lines {
            if editor.display_map.is_hidden(line_number) {
                continue;
            }
            let line = LineIndex(line_number);
            let Ok(full_range) = snapshot.line_range(line) else {
                continue;
            };
            let anchor = snapshot
                .line_index_at(selection.head())
                .ok()
                .filter(|selection_line| *selection_line == line)
                .map(|_| selection.head());
            let Some(source_line) = editor.display_map.source_line(&snapshot, line, anchor) else {
                continue;
            };
            let source_content_range = source_line.source_range;
            let content_range = source_line.visible_range;
            let display = source_line.display;
            let folded = editor.fold_markers.contains(&line_number);
            let line_animation_scale =
                if fold_ranges.iter().any(|range| range.contains(&line_number)) {
                    fold_scale
                } else {
                    1.0
                };
            let line_style = style_snapshot
                .line(line_number)
                .expect("visible style snapshot covers every visible source line");
            let metrics = line_style.metrics;
            let display_text = folded_display_text(display.text.clone(), folded);
            let text: gpui::SharedString = display_text.into();
            let base_run = TextRun {
                len: text.len(),
                font: style.font(),
                color: gpui::rgb(theme.foreground).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let marked_display = local_marked(marked, content_range, &display);
            let runs = syntax::runs(
                editor.session.read(cx).path(),
                &text,
                base_run,
                line_style,
                marked_display.clone(),
                theme,
            );
            let effective_wrap_width = editor.display_map.soft_wrap().then_some(px(wrap_width));
            let shape_key = shape_key(
                &text,
                px(f32::from(font_size) * metrics.font_scale),
                marked_display,
                effective_wrap_width,
                line_style.id.cache_key(),
                line_style.code_language.clone(),
            );
            let layout = editor
                .shape_cache
                .get(&shape_key)
                .cloned()
                .unwrap_or_else(|| {
                    window
                        .text_system()
                        .shape_text(
                            text,
                            px(f32::from(font_size) * metrics.font_scale),
                            &runs,
                            effective_wrap_width,
                            None,
                        )
                        .ok()
                        .and_then(|lines| lines.into_iter().next())
                        .map(Arc::new)
                        .unwrap_or_else(|| Arc::new(WrappedLine::default()))
                });
            let visual_rows = layout.wrap_boundaries().len() + 1;
            let number: gpui::SharedString = (line_number + 1).to_string().into();
            let gutter_run = TextRun {
                len: number.len(),
                font: style.font(),
                color: gpui::rgb(theme.foreground_dim).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let gutter_layout =
                window
                    .text_system()
                    .shape_line(number, font_size, &[gutter_run], None);
            let total_height =
                metrics.before + visual_rows as f32 * metrics.line_height + metrics.after;
            // A fold transition may paint a sparse sample, so those rows need absolute animated
            // coordinates. Normal editing paints contiguous visible rows and deliberately uses
            // the heights shaped in this frame; after a window-width change that avoids showing
            // one stale-baseline frame followed by a visible correction.
            let block_top_y = if fold_animation_active {
                editor.animated_line_start_y(line_number)
            } else {
                next_y
            };
            let block_top = bounds.top() + px(block_top_y - scroll_y);
            let origin_y = block_top + px(metrics.before);
            let animated_height = total_height * line_animation_scale;
            let animation_clip_y = (line_animation_scale < 0.999)
                .then_some((block_top, block_top + px(animated_height)));
            let hit = HitRow {
                range: source_line.visible_range,
                line,
                origin_y,
                visible_top: block_top,
                visible_bottom: block_top + px(animated_height),
                text_origin_x,
                line_height: px(metrics.line_height),
                display,
                layout,
            };

            let selected = selection.range();
            let selected_start = selected.start.0.max(full_range.start.0);
            let selected_end = selected.end.0.min(full_range.end.0);
            if selected_start < selected_end && line_animation_scale >= 0.999 {
                let source_local_start = selected_start
                    .saturating_sub(content_range.start.0)
                    .min(hit.layout.len() as u64) as usize;
                let source_local_end = selected_end
                    .saturating_sub(content_range.start.0)
                    .min(hit.layout.len() as u64) as usize;
                let local_start = hit.display.source_to_display(source_local_start);
                let local_end = hit.display.source_to_display(source_local_end);
                push_selection_quads(
                    &mut selection_quads,
                    &hit,
                    local_start,
                    local_end,
                    selected_end > source_content_range.end.0,
                    px(wrap_width),
                );
            }

            if anchor.is_some()
                && selection.is_empty()
                && selection.head() >= content_range.start
                && selection.head() <= content_range.end
                && line_animation_scale >= 0.999
            {
                let source_local = selection
                    .head()
                    .0
                    .saturating_sub(content_range.start.0)
                    .min(hit.layout.len() as u64) as usize;
                let local = hit.display.source_to_display(source_local);
                let position = hit
                    .layout
                    .position_for_index(local, px(metrics.line_height))
                    .unwrap_or_default();
                caret = Some(fill(
                    Bounds::new(
                        point(text_origin_x + position.x, origin_y + position.y + px(2.0)),
                        size(px(1.5), px((metrics.line_height - 4.0).max(1.0))),
                    ),
                    gpui::rgb(theme.foreground),
                ));
            }
            rows.push(PaintRow {
                hit,
                gutter_layout,
                shape_key,
                visual_rows,
                metrics,
                animated_height,
                background: editor_row_background(
                    line_style.id,
                    anchor.is_some(),
                    Bounds::new(
                        point(text_origin_x, block_top),
                        size(px(wrap_width), px(animated_height)),
                    ),
                    theme,
                ),
                animation_clip_y,
            });
            if !fold_animation_active {
                next_y += total_height;
            }
        }

        let (_, fallback_top, fallback_bottom) =
            editor.minimap_source_viewport(f32::from(bounds.size.height));
        let visible_top =
            visible_position_in_paint_rows(editor, &rows, bounds.top()).unwrap_or(fallback_top);
        let visible_bottom = visible_position_in_paint_rows(editor, &rows, bounds.bottom())
            .unwrap_or(fallback_bottom)
            .clamp(visible_top, editor.display_map.visible_line_count() as f32);
        let minimap_geometry = editor.minimap_viewport_geometry_for_source_range(
            minimap_bounds,
            visible_top,
            visible_bottom,
        );
        let (minimap, minimap_raster_request) = build_minimap(
            editor,
            &snapshot,
            minimap_bounds,
            minimap_geometry,
            window.scale_factor(),
            theme,
        );
        if editor.fold_animation.is_none()
            && let Some(request) = minimap_raster_request
        {
            schedule_minimap_raster(self.editor.clone(), request, cx);
        }
        PrepaintState {
            #[cfg(feature = "benchmarks")]
            started_at,
            rows,
            selection: selection_quads,
            caret,
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

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        let hits = state
            .rows
            .iter()
            .map(|row| row.hit.clone())
            .collect::<Arc<[HitRow]>>();
        let shaped = state
            .rows
            .iter()
            .map(|row| (row.shape_key.clone(), row.hit.layout.clone()))
            .collect::<Vec<_>>();
        let gutter = state.gutter.clone();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(gutter);
            if let Some(background) = &state.minimap.background {
                window.paint_quad(background.clone());
            }
            if let Some((image, image_bounds)) = &state.minimap.image {
                let _ = window.paint_image(
                    *image_bounds,
                    *image_bounds,
                    Corners::default(),
                    image.clone(),
                    0,
                    false,
                );
            }
            for quad in &state.minimap.overlays {
                window.paint_quad(quad.clone());
            }
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
                    for background in state.rows.iter().filter_map(|row| row.background.clone()) {
                        window.paint_quad(background);
                    }
                    for selection in state.selection.drain(..) {
                        window.paint_quad(selection);
                    }
                    for row in &state.rows {
                        let mut paint = |window: &mut Window| {
                            let _ = row.hit.layout.paint(
                                point(row.hit.text_origin_x, row.hit.origin_y),
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
                                    paint,
                                );
                            }
                        } else {
                            paint(window);
                        }
                    }
                    if focus_handle.is_focused(window)
                        && let Some(caret) = state.caret.take()
                    {
                        window.paint_quad(caret);
                    }
                },
            );
        });
        #[cfg(feature = "benchmarks")]
        let elapsed = state.started_at.elapsed();
        let measured_rows = state
            .rows
            .iter()
            .map(|row| (row.hit.line.0, row.visual_rows, row.metrics))
            .collect::<Vec<_>>();
        let _benchmark = self.editor.update(cx, |editor, cx| {
            let viewport_changed = editor.viewport != Some(bounds);
            editor.viewport = Some(bounds);
            editor.minimap.bounds = editor.minimap.visible.then_some(Bounds::new(
                point(bounds.right() - px(editor.minimap.width), bounds.top()),
                size(px(editor.minimap.width), bounds.size.height),
            ));
            editor.hit_rows = hits;
            if editor.display_map.soft_wrap() {
                editor.scroll_x = 0.0;
            } else {
                editor.scroll_x = editor.scroll_x.min(editor.max_horizontal_scroll());
            }
            if editor.shape_cache.len() + shaped.len() > 512 {
                editor.shape_cache.clear();
            }
            editor.shape_cache.extend(shaped);
            let viewport_height = f32::from(bounds.size.height);
            let was_at_end = scroll_is_at_end(
                editor.scroll_y,
                viewport_height,
                editor.animated_document_height(),
            );
            let anchor_line = editor.animated_line_at_y(editor.scroll_y);
            let anchor_start = editor.animated_line_start_y(anchor_line);
            let anchor_fraction = ((editor.scroll_y - anchor_start)
                / editor.animated_line_height_px(anchor_line).max(1.0))
            .clamp(0.0, 1.0);
            let layout_changed =
                measured_rows
                    .into_iter()
                    .fold(false, |changed, (line, rows, metrics)| {
                        editor.display_map.update_line_layout(
                            line,
                            rows,
                            metrics.line_height,
                            metrics.before,
                            metrics.after,
                        ) || changed
                    });
            let anchored = if layout_changed {
                editor.animated_line_start_y(anchor_line)
                    + anchor_fraction * editor.animated_line_height_px(anchor_line)
            } else {
                editor.scroll_y
            };
            let settled_scroll_y = stabilized_scroll_y(
                was_at_end,
                anchored,
                viewport_height,
                editor.animated_document_height(),
            );
            let scroll_settled = (settled_scroll_y - editor.scroll_y).abs() > 0.5;
            editor.scroll_y = settled_scroll_y;
            let snapshot = editor.snapshot(cx);
            let pending_reveal = editor.pending_reveal_caret;
            if pending_reveal {
                let caret_line = snapshot.line_index_at(editor.selection.head()).ok();
                let has_exact_row = caret_line.is_some_and(|caret_line| {
                    editor.hit_rows.iter().any(|row| {
                        row.line == caret_line
                            && editor.selection.head() >= row.range.start
                            && editor.selection.head() <= row.range.end
                    })
                });
                editor.pending_reveal_caret = !has_exact_row;
                editor.reveal_caret(&snapshot);
            }
            let final_anchor_line = editor.animated_line_at_y(editor.scroll_y);
            editor.layout_anchor = snapshot
                .line_content_range(LineIndex(final_anchor_line))
                .ok()
                .map(|range| {
                    crate::document::RevisionRange::new(
                        snapshot.revision(),
                        ByteRange::new(range.start.0, range.start.0),
                    )
                });
            if viewport_changed || layout_changed || scroll_settled || pending_reveal {
                cx.notify();
            }
            #[cfg(feature = "benchmarks")]
            return Some(editor.record_frame_benchmark(elapsed, cx));
            #[cfg(not(feature = "benchmarks"))]
            None::<()>
        });
        #[cfg(feature = "benchmarks")]
        match _benchmark.expect("benchmark builds always return a frame action") {
            FrameBenchmarkAction::Inactive => {}
            FrameBenchmarkAction::Continue => window.request_animation_frame(),
            FrameBenchmarkAction::Complete => {
                if std::env::var_os("ORG_STUDIO_EXIT_AFTER_EDITOR_BENCH").is_some() {
                    cx.quit();
                }
            }
        }
    }
}

fn visible_position_in_paint_rows(
    editor: &SemanticEditor,
    rows: &[PaintRow],
    y: Pixels,
) -> Option<f32> {
    let last = rows.last()?;
    for row in rows {
        let block_top = row.hit.origin_y - px(row.metrics.before);
        let block_height = row.animated_height;
        let block_bottom = block_top + px(block_height);
        if y <= block_bottom || row.hit.line == last.hit.line {
            let fraction = (f32::from(y - block_top) / block_height.max(1.0)).clamp(0.0, 1.0);
            let source_y = editor.animated_line_start_y(row.hit.line.0)
                + fraction * editor.animated_line_height_px(row.hit.line.0);
            return Some(editor.animated_visible_position_at_y(source_y));
        }
    }
    None
}

fn scroll_is_at_end(scroll_y: f32, viewport_height: f32, document_height: f32) -> bool {
    scroll_y + viewport_height + 0.5 >= document_height
}

fn stabilized_scroll_y(
    was_at_end: bool,
    anchored: f32,
    viewport_height: f32,
    document_height: f32,
) -> f32 {
    let max_scroll = (document_height - viewport_height).max(0.0);
    if was_at_end {
        max_scroll
    } else {
        anchored.clamp(0.0, max_scroll)
    }
}

fn shape_key(
    text: &gpui::SharedString,
    font_size: Pixels,
    marked: Option<std::ops::Range<usize>>,
    wrap_width: Option<Pixels>,
    syntax_key: u8,
    code_language: Option<Arc<str>>,
) -> ShapeKey {
    ShapeKey {
        text: text.clone(),
        font_size_bits: f32::from(font_size).to_bits(),
        wrap_width_bits: wrap_width.map_or(0, |width| f32::from(width).to_bits()),
        syntax_key,
        code_language,
        marked: marked.map(|range| (range.start, range.end)),
    }
}

fn build_minimap(
    editor: &SemanticEditor,
    snapshot: &crate::document::DocumentSnapshot,
    bounds: Bounds<Pixels>,
    geometry: super::minimap::ViewportGeometry,
    scale_factor: f32,
    theme: &crate::theme::Theme,
) -> (MinimapPaint, Option<MinimapRasterRequest>) {
    if !editor.minimap.visible || f32::from(bounds.size.width) <= 0.0 {
        return (
            MinimapPaint {
                bounds,
                background: None,
                image: None,
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
    let total_units = editor.display_map.visible_line_count().max(1) as f32;
    let (first_unit, row_count) = super::minimap::raster_window(
        geometry.content_top,
        geometry.interaction_height,
        total_units,
        density,
        line_height,
    );
    let key = super::minimap::RasterKey {
        generation: editor.minimap.generation,
        first_unit,
        rows: row_count.min(u16::MAX as usize) as u16,
        width: f32::from(bounds.size.width).ceil().min(u16::MAX as f32) as u16,
        scale_x100: (scale_factor.max(1.0) * 100.0).round().min(u16::MAX as f32) as u16,
        density,
    };
    let cached_raster = editor
        .minimap
        .raster
        .lock()
        .expect("editor minimap raster poisoned")
        .as_ref()
        .cloned();
    let cached_matches = cached_raster
        .as_ref()
        .is_some_and(|cached| cached.key == key);
    let mut raster_request = None;
    if !cached_matches && editor.minimap.reserve_raster(key) {
        let rows = (0..row_count)
            .map(|offset| {
                let line = editor
                    .display_map
                    .source_line_for_visible_ordinal(first_unit + offset as u64)
                    .unwrap_or_else(|| snapshot.len_lines().saturating_sub(1));
                let text = snapshot
                    .line_content_range(LineIndex(line))
                    .ok()
                    .map(|range| bounded_minimap_text(snapshot, range, 1_024))
                    .unwrap_or_default();
                let kind = super::minimap::classify_line(&text);
                super::minimap::TextRow {
                    color: minimap_text_color(kind, theme),
                    text,
                    indent: 3.0,
                }
            })
            .collect::<Vec<_>>();
        raster_request = Some(MinimapRasterRequest {
            key,
            rows,
            content_top: geometry.content_top,
            viewport_generation: editor.minimap.viewport_generation(),
            scale_factor,
            density,
            line_height,
            epoch: editor.minimap.raster_epoch.clone(),
            expected_epoch: editor
                .minimap
                .raster_epoch
                .load(std::sync::atomic::Ordering::Acquire),
        });
    }
    let image = cached_raster.map(|cached| {
        let same_camera = cached.viewport_generation == editor.minimap.viewport_generation();
        let placement_content_top = super::minimap::raster_placement_content_top(
            &cached,
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
        (
            cached.image,
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
    let thumb_bounds = Bounds::new(
        point(bounds.left(), bounds.top() + px(thumb_top)),
        size(bounds.size.width, px(geometry.thumb_height)),
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
            let line_unit = editor.display_map.line_start_y(line.0) / super::LINE_HEIGHT;
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
            overlays,
        },
        raster_request,
    )
}

fn schedule_minimap_raster(
    editor: gpui::Entity<SemanticEditor>,
    request: MinimapRasterRequest,
    cx: &mut App,
) {
    let key = request.key;
    let content_top = request.content_top;
    let viewport_generation = request.viewport_generation;
    let line_height = request.line_height;
    let background = cx.background_executor().spawn(async move {
        super::minimap::rasterize_text_rows(
            &request.rows,
            usize::from(key.width),
            request.scale_factor,
            request.density,
            request.line_height,
            &request.epoch,
            request.expected_epoch,
        )
    });
    cx.spawn(async move |cx| {
        let Some(image) = background.await else {
            return;
        };
        editor.update(cx, |editor, cx| {
            let mut in_flight = editor
                .minimap
                .raster_build
                .lock()
                .expect("editor minimap raster build poisoned");
            if *in_flight != Some(key) {
                return;
            }
            *editor
                .minimap
                .raster
                .lock()
                .expect("editor minimap raster poisoned") = Some(super::minimap::CachedRaster {
                key,
                image,
                content_top,
                viewport_generation,
                line_height,
            });
            *in_flight = None;
            cx.notify();
        });
    })
    .detach();
}

fn bounded_minimap_text(
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

fn minimap_text_color(kind: super::minimap::LineKind, theme: &crate::theme::Theme) -> u32 {
    match kind {
        super::minimap::LineKind::Heading(level) => {
            theme.heading[(level.saturating_sub(1) as usize).min(3)]
        }
        super::minimap::LineKind::Code => theme.code_boundary,
        super::minimap::LineKind::Quote => theme.quote,
        super::minimap::LineKind::Property => theme.meta,
        super::minimap::LineKind::Table | super::minimap::LineKind::Plain => theme.foreground,
    }
}

fn local_marked(
    marked: Option<ByteRange>,
    line: ByteRange,
    display: &super::layout_map::DisplayLineText,
) -> Option<std::ops::Range<usize>> {
    let marked = marked?;
    let start = marked.start.0.max(line.start.0);
    let end = marked.end.0.min(line.end.0);
    if start >= end {
        return None;
    }
    Some(
        display.source_to_display((start - line.start.0) as usize)
            ..display.source_to_display((end - line.start.0) as usize),
    )
}

fn folded_display_text(mut text: String, folded: bool) -> String {
    if folded {
        text.push_str("...");
    }
    text
}

fn editor_row_background(
    style: syntax::EditorStyleId,
    active: bool,
    bounds: Bounds<Pixels>,
    theme: &crate::theme::Theme,
) -> Option<PaintQuad> {
    let color = match style {
        syntax::EditorStyleId::CodeBoundary => theme.code_boundary_background,
        syntax::EditorStyleId::Code if active => theme.code_active_background,
        syntax::EditorStyleId::Code => theme.code_background,
        _ if active => theme.background_alt,
        _ => return None,
    };
    Some(fill(bounds, gpui::rgb(color)))
}

fn push_selection_quads(
    quads: &mut Vec<PaintQuad>,
    hit: &HitRow,
    start: usize,
    end: usize,
    include_newline: bool,
    wrap_width: Pixels,
) {
    let line_height = hit.line_height;
    let start_position = hit
        .layout
        .position_for_index(start, line_height)
        .unwrap_or_default();
    let end_position = hit
        .layout
        .position_for_index(end, line_height)
        .unwrap_or(start_position);
    let line_height_px = f32::from(line_height).max(1.0);
    let first_row = (f32::from(start_position.y) / line_height_px).round() as usize;
    let last_row = (f32::from(end_position.y) / line_height_px).round() as usize;
    for row in first_row..=last_row {
        let left = if row == first_row {
            start_position.x
        } else {
            Pixels::ZERO
        };
        let mut right = if row == last_row {
            end_position.x
        } else {
            wrap_width
        };
        if include_newline && row == last_row {
            right += px(8.0);
        }
        if right > left {
            quads.push(fill(
                Bounds::from_corners(
                    point(
                        hit.text_origin_x + left,
                        hit.origin_y + px(row as f32 * line_height_px),
                    ),
                    point(
                        hit.text_origin_x + right,
                        hit.origin_y + px((row + 1) as f32 * line_height_px),
                    ),
                ),
                rgba(0x3f78f24a),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_ANIMATED_PAINT_LINES, animated_paint_lines, folded_display_text, scroll_is_at_end,
        stabilized_scroll_y,
    };
    use crate::editor::layout_map::EditorLayoutMap;

    #[test]
    fn folded_heading_display_adds_an_ellipsis_without_changing_source_text() {
        assert_eq!(
            folded_display_text("* Heading".to_owned(), true),
            "* Heading..."
        );
        assert_eq!(
            folded_display_text("* Heading".to_owned(), false),
            "* Heading"
        );
    }

    #[test]
    fn bottom_stays_pinned_while_measured_document_height_converges() {
        assert!(scroll_is_at_end(800.0, 200.0, 1_000.0));
        assert_eq!(stabilized_scroll_y(true, 800.0, 200.0, 1_120.0), 920.0);
        assert_eq!(stabilized_scroll_y(false, 420.0, 200.0, 1_120.0), 420.0);
        assert_eq!(stabilized_scroll_y(false, 980.0, 200.0, 1_120.0), 920.0);
    }

    #[test]
    fn large_fold_animations_shape_only_a_bounded_sample_of_changed_lines() {
        let mut display_map = EditorLayoutMap::default();
        display_map.configure(1_000, 700.0);

        let lines = animated_paint_lines(&display_map, 0..1_000, std::slice::from_ref(&(1..999)));

        assert!(lines.len() <= MAX_ANIMATED_PAINT_LINES as usize + 2);
        assert_eq!(lines.first(), Some(&0));
        assert_eq!(lines.last(), Some(&999));
        assert!(lines.contains(&1));
        assert!(lines.contains(&998));
    }
}
