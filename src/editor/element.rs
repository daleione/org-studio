#[cfg(feature = "benchmarks")]
use std::time::Instant;
use std::{ops::Range, sync::Arc};

use gpui::{
    App, BorderStyle, Bounds, ContentMask, Corners, CursorStyle, Edges, Element, ElementId,
    ElementInputHandler, GlobalElementId, Hitbox, HitboxBehavior, LayoutId, PaintQuad, Pixels,
    RenderImage, ShapedLine, Style, TextAlign, TextRun, Window, WrappedLine, fill, outline, point,
    px, quad, relative, rgba, size,
};

use crate::{
    document::{ByteOffset, ByteRange, LineIndex, TextSnapshot},
    theme::current_theme,
};

#[cfg(feature = "benchmarks")]
use super::FrameBenchmarkAction;
use super::{HitRow, SemanticEditor, ShapeKey, layout_map::EditorLayoutMap, syntax};

const GUTTER_PADDING: f32 = 16.0;
const MAX_ANIMATED_PAINT_LINES: u64 = 192;
const BLOCK_LEFT_INSET: f32 = 8.0;
const BLOCK_RIGHT_INSET: f32 = 16.0;
const BLOCK_TEXT_INSET: f32 = 16.0;
const BLOCK_TEXT_RIGHT_PADDING: f32 = 8.0;
const SOURCE_GUTTER_INSET: f32 = 1.0;
const SOURCE_GUTTER_WIDTH: f32 = 24.0;
const SOURCE_TEXT_INSET: f32 = 42.0;
const SOURCE_LINE_NUMBER_FONT_SCALE: f32 = 0.70;
const SOURCE_RUN_BUTTON_SIZE: f32 = 20.0;
const SOURCE_RUN_BUTTON_HIT_SLOP: f32 = 4.0;
const SOURCE_RUN_ICON_FONT_SCALE: f32 = 0.82;
const BLOCK_VERTICAL_INSET: f32 = 2.0;
const BLOCK_RADIUS: f32 = 7.0;
const INLINE_IMAGE_VERTICAL_PADDING: f32 = 6.0;
const INLINE_IMAGE_MAX_WIDTH: f32 = 640.0;

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
    block_backgrounds: Vec<PaintQuad>,
    source_run_buttons: Vec<SourceRunButtonPaint>,
    selection: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
    gutter: PaintQuad,
    content_left: Pixels,
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
    source_line_number_layout: Option<ShapedLine>,
    shape_key: ShapeKey,
    visual_rows: usize,
    metrics: syntax::BlockMetrics,
    animated_height: f32,
    block: Option<syntax::EditorBlockDecoration>,
    active: bool,
    folded: bool,
    background: Option<PaintQuad>,
    animation_clip_y: Option<(Pixels, Pixels)>,
    inline_image: Option<InlineImagePaint>,
}

struct InlineImagePaint {
    image: Arc<RenderImage>,
    bounds: Bounds<Pixels>,
}

struct SourceRunButtonPaint {
    source_offset: ByteOffset,
    bounds: Bounds<Pixels>,
    interaction_bounds: Bounds<Pixels>,
    hitbox: Hitbox,
    accent: u32,
    icon: ShapedLine,
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
        let editor_font_size = self.editor.read(cx).font_size_px();
        let gutter_width = digits as f32 * editor_font_size * 0.6 + GUTTER_PADDING * 2.0;
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
            if layout_reconfigured {
                let inline_image_lines = editor
                    .inline_image_line_dimensions
                    .borrow()
                    .iter()
                    .map(|(&line, &(line_start, width, height))| (line, line_start, width, height))
                    .collect::<Vec<_>>();
                for (line, line_start, width, height) in inline_image_lines {
                    if !editor.previews_inline_image_at(ByteOffset(line_start)) {
                        continue;
                    }
                    let (_, height) = crate::preview::fitted_image_size(
                        width,
                        height,
                        wrap_width.min(INLINE_IMAGE_MAX_WIDTH),
                    );
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
        });
        let inline_image_candidates = {
            let editor = self.editor.read(cx);
            let document_path = editor.session.read(cx).path().to_path_buf();
            if crate::document::DocumentFormat::from_path(&document_path)
                == crate::document::DocumentFormat::Org
            {
                let visible_lines = editor.animated_visible_line_range(
                    &snapshot,
                    editor.scroll_y,
                    f32::from(bounds.size.height),
                );
                visible_lines
                    .filter(|line| !editor.display_map.is_hidden(*line))
                    .filter_map(|line| {
                        let range = snapshot.line_content_range(LineIndex(line)).ok()?;
                        if !editor.previews_inline_image_at(range.start) {
                            return None;
                        }
                        let text = snapshot.copy_range(range);
                        let target = crate::org_syntax::standalone_image_path(&text)?;
                        let path = crate::preview::resolve_image_path(&document_path, target);
                        let cached = editor.cached_inline_image_render(&path);
                        if let Some((_, dimensions)) = &cached {
                            editor
                                .inline_image_line_dimensions
                                .borrow_mut()
                                .insert(line, (range.start.0, dimensions.0, dimensions.1));
                        }
                        Some((line, range.start.0, path, cached))
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            }
        };
        let inline_images = inline_image_candidates
            .into_iter()
            .filter_map(|(line, line_start, path, cached)| {
                let previous_dimensions = cached.as_ref().map(|(_, dimensions)| *dimensions);
                let path: Arc<std::path::Path> = path.into();
                let resource: gpui::Resource = path.clone().into();
                let loaded = window.use_asset::<gpui::ImgResourceLoader>(&resource, cx);
                let (image, dimensions) = match loaded {
                    Some(Ok(image)) => self
                        .editor
                        .read(cx)
                        .accept_inline_image_render(path.as_ref(), image),
                    Some(Err(_)) => {
                        let changed = self.editor.read(cx).fail_inline_image_render(path.as_ref());
                        if changed {
                            self.editor.update(cx, |editor, cx| {
                                editor
                                    .inline_image_line_dimensions
                                    .borrow_mut()
                                    .remove(&line);
                                editor.display_map.invalidate_line_layout(line);
                                cx.notify();
                            });
                        }
                        return None;
                    }
                    None => cached?,
                };
                if previous_dimensions != Some(dimensions) {
                    self.editor.update(cx, |editor, cx| {
                        editor
                            .inline_image_line_dimensions
                            .borrow_mut()
                            .insert(line, (line_start, dimensions.0, dimensions.1));
                        editor.display_map.invalidate_line_layout(line);
                        cx.notify();
                    });
                } else {
                    self.editor
                        .read(cx)
                        .inline_image_line_dimensions
                        .borrow_mut()
                        .insert(line, (line_start, dimensions.0, dimensions.1));
                }
                let (width, height) = crate::preview::fitted_image_size(
                    dimensions.0,
                    dimensions.1,
                    wrap_width.min(INLINE_IMAGE_MAX_WIDTH),
                );
                Some((line, (image, width, height, path)))
            })
            .collect::<std::collections::HashMap<_, _>>();
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
            let text_inset = editor_block_text_inset(line_style.block.as_ref());
            let row_text_origin_x = text_origin_x + px(text_inset);
            let row_wrap_width = if text_inset > 0.0 {
                (wrap_width - text_inset - BLOCK_RIGHT_INSET - BLOCK_TEXT_RIGHT_PADDING).max(1.0)
            } else {
                wrap_width
            };
            let mut metrics = line_style
                .metrics
                .scaled(editor.content_font_size().scale());
            let display_text = folded_display_text(display.text.clone(), folded);
            let inline_image_source = (!folded)
                .then(|| inline_images.get(&line_number).cloned())
                .flatten();
            if let Some((_, _, height, _)) = &inline_image_source {
                metrics.before = INLINE_IMAGE_VERTICAL_PADDING;
                metrics.line_height = *height;
                metrics.after = INLINE_IMAGE_VERTICAL_PADDING;
            }
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
            let effective_wrap_width = editor.display_map.soft_wrap().then_some(px(row_wrap_width));
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
            let visual_rows = if inline_image_source.is_some() {
                1
            } else {
                layout.wrap_boundaries().len() + 1
            };
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
            let source_line_number_layout = line_style
                .block
                .as_ref()
                .filter(|block| is_source_block_kind(&block.kind))
                .and_then(|block| {
                    block
                        .body_line
                        .map(|body_line| (body_line, editor_block_accent(&block.kind, theme)))
                })
                .map(|(body_line, accent)| {
                    let number: gpui::SharedString = body_line.to_string().into();
                    let run = TextRun {
                        len: number.len(),
                        font: style.font(),
                        color: rgba((accent << 8) | 0xd0).into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    window.text_system().shape_line(
                        number,
                        px(f32::from(font_size) * SOURCE_LINE_NUMBER_FONT_SCALE),
                        &[run],
                        None,
                    )
                });
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
            let inline_image = inline_image_source.map(|(image, width, height, _)| {
                let bounds = Bounds::new(
                    point(row_text_origin_x, origin_y),
                    size(px(width), px(height)),
                );
                InlineImagePaint { image, bounds }
            });
            let hit = HitRow {
                range: source_line.visible_range,
                line,
                origin_y,
                visible_top: block_top,
                visible_bottom: block_top + px(animated_height),
                text_origin_x: row_text_origin_x,
                line_height: px(metrics.line_height),
                display,
                layout,
                inline_image_preview: inline_image.is_some(),
            };

            let selected = selection.range();
            let selected_start = selected.start.0.max(full_range.start.0);
            let selected_end = selected.end.0.min(full_range.end.0);
            if inline_image.is_none()
                && selected_start < selected_end
                && line_animation_scale >= 0.999
            {
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
                    px(row_wrap_width),
                );
            }

            if let Some(image) = inline_image.as_ref()
                && anchor.is_some()
                && selection.is_empty()
                && selection.head() >= content_range.start
                && selection.head() <= content_range.end
                && line_animation_scale >= 0.999
            {
                let caret_x = if selection.head() <= content_range.start {
                    image.bounds.left() - px(2.0)
                } else {
                    image.bounds.right() + px(1.0)
                };
                caret = Some(fill(
                    Bounds::new(
                        point(caret_x, image.bounds.top() + px(1.0)),
                        size(px(1.5), (image.bounds.size.height - px(2.0)).max(px(1.0))),
                    ),
                    gpui::rgb(theme.foreground),
                ));
            } else if inline_image.is_none()
                && anchor.is_some()
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
                        point(
                            row_text_origin_x + position.x,
                            origin_y + position.y + px(2.0),
                        ),
                        size(px(1.5), px((metrics.line_height - 4.0).max(1.0))),
                    ),
                    gpui::rgb(theme.foreground),
                ));
            }
            rows.push(PaintRow {
                hit,
                gutter_layout,
                source_line_number_layout,
                shape_key,
                visual_rows,
                metrics,
                animated_height,
                block: line_style.block.clone(),
                active: anchor.is_some(),
                folded,
                background: editor_row_background(
                    line_style.id,
                    anchor.is_some(),
                    Bounds::new(
                        point(row_text_origin_x, block_top),
                        size(px(row_wrap_width), px(animated_height)),
                    ),
                    theme,
                ),
                animation_clip_y,
                inline_image,
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
        let schedule_minimap = editor.fold_animation.is_none();
        let source_run_feedback = editor.source_run_feedback;
        let horizontal_scroll = editor.scroll_x;
        if schedule_minimap && let Some(request) = minimap_raster_request {
            schedule_minimap_raster(self.editor.clone(), request, cx);
        }
        let viewport_text_left = bounds.left() + px(gutter_width);
        let viewport_text_right = bounds.right() - px(minimap_width);
        let (block_left, block_minimum_right) = editor_block_horizontal_bounds(
            viewport_text_left,
            viewport_text_right,
            horizontal_scroll,
        );
        let block_content_right = rows
            .iter()
            .filter(|row| row.block.is_some())
            .map(|row| {
                row.hit.text_origin_x
                    + row.hit.layout.width()
                    + px(BLOCK_TEXT_RIGHT_PADDING + BLOCK_RIGHT_INSET)
            })
            .fold(block_minimum_right, |right, candidate| right.max(candidate));
        let mut block_backgrounds =
            editor_block_backgrounds(&rows, block_left, block_content_right, theme);
        block_backgrounds.extend(editor_source_gutter_backgrounds(
            &rows,
            block_left,
            block_content_right,
            theme,
        ));
        let source_run_buttons = rows
            .iter()
            .filter(|row| {
                row.block.as_ref().is_some_and(|block| {
                    block.kind == syntax::EditorBlockKind::Source
                        && block.edge == syntax::EditorBlockEdge::Open
                })
            })
            .filter_map(|row| {
                let button_left = block_left
                    + px(BLOCK_LEFT_INSET
                        + SOURCE_GUTTER_INSET
                        + (SOURCE_GUTTER_WIDTH - SOURCE_RUN_BUTTON_SIZE) / 2.0);
                let row_height = f32::from(row.hit.visible_bottom - row.hit.visible_top);
                let button_top = row.hit.visible_top
                    + px(((row_height - SOURCE_RUN_BUTTON_SIZE) / 2.0).max(0.0));
                let bounds = Bounds::new(
                    point(button_left, button_top),
                    size(px(SOURCE_RUN_BUTTON_SIZE), px(SOURCE_RUN_BUTTON_SIZE)),
                );
                let interaction_bounds = Bounds::new(
                    point(
                        button_left - px(SOURCE_RUN_BUTTON_HIT_SLOP),
                        button_top - px(SOURCE_RUN_BUTTON_HIT_SLOP),
                    ),
                    size(
                        px(SOURCE_RUN_BUTTON_SIZE + SOURCE_RUN_BUTTON_HIT_SLOP * 2.0),
                        px(SOURCE_RUN_BUTTON_SIZE + SOURCE_RUN_BUTTON_HIT_SLOP * 2.0),
                    ),
                );
                let interaction_left = interaction_bounds.left().max(viewport_text_left);
                let interaction_right = interaction_bounds.right().min(viewport_text_right);
                if interaction_right <= interaction_left {
                    return None;
                }
                let interaction_bounds = Bounds::from_corners(
                    point(interaction_left, interaction_bounds.top()),
                    point(interaction_right, interaction_bounds.bottom()),
                );
                let feedback = source_run_feedback
                    .filter(|feedback| feedback.source_offset == row.hit.range.start);
                let (icon_text, accent) = match feedback.map(|feedback| feedback.phase) {
                    Some(super::SourceRunPhase::Running) => {
                        window.request_animation_frame();
                        let frames = ["◐", "◓", "◑", "◒"];
                        let frame = feedback
                            .map(|feedback| {
                                (feedback.started_at.elapsed().as_millis() / 90) as usize
                                    % frames.len()
                            })
                            .unwrap_or(0);
                        (frames[frame], theme.meta)
                    }
                    Some(super::SourceRunPhase::Success) => ("✓", theme.heading[2]),
                    Some(super::SourceRunPhase::Failure) => ("!", 0xb23a63),
                    None => ("▶", theme.meta),
                };
                let icon: gpui::SharedString = icon_text.into();
                let run = TextRun {
                    len: icon.len(),
                    font: style.font(),
                    color: rgba((accent << 8) | 0xd0).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                Some(SourceRunButtonPaint {
                    source_offset: row.hit.range.start,
                    bounds,
                    interaction_bounds,
                    hitbox: window.insert_hitbox(interaction_bounds, HitboxBehavior::Normal),
                    accent,
                    icon: window.text_system().shape_line(
                        icon,
                        px(f32::from(font_size) * SOURCE_RUN_ICON_FONT_SCALE),
                        &[run],
                        None,
                    ),
                })
            })
            .collect();
        PrepaintState {
            #[cfg(feature = "benchmarks")]
            started_at,
            rows,
            block_backgrounds,
            source_run_buttons,
            selection: selection_quads,
            caret,
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
        let source_run_button_hits = state
            .source_run_buttons
            .iter()
            .map(|button| super::SourceRunButtonHit {
                bounds: button.interaction_bounds,
                source_offset: button.source_offset,
            })
            .collect::<Arc<[_]>>();
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
                    for background in &state.block_backgrounds {
                        window.paint_quad(background.clone());
                    }
                    for background in state.rows.iter().filter_map(|row| row.background.clone()) {
                        window.paint_quad(background);
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
                                + px(((SOURCE_GUTTER_WIDTH - f32::from(number_layout.width()))
                                    / 2.0)
                                    .max(0.0));
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
            editor.source_run_buttons = source_run_button_hits;
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
            let line_unit = editor.display_map.line_start_y(line.0)
                / editor.display_map.base_line_height().max(1.0);
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

#[derive(Clone, Debug, PartialEq)]
struct EditorBlockPaintRow {
    block: Option<syntax::EditorBlockDecoration>,
    top: f32,
    bottom: f32,
    active: bool,
    folded: bool,
}

fn editor_block_text_inset(block: Option<&syntax::EditorBlockDecoration>) -> f32 {
    block.map_or(0.0, |block| {
        if is_source_block_kind(&block.kind) {
            SOURCE_TEXT_INSET
        } else {
            BLOCK_TEXT_INSET
        }
    })
}

fn is_source_block_kind(kind: &syntax::EditorBlockKind) -> bool {
    matches!(
        kind,
        syntax::EditorBlockKind::Source | syntax::EditorBlockKind::MarkdownFence
    )
}

fn editor_block_accent(kind: &syntax::EditorBlockKind, theme: &crate::theme::Theme) -> u32 {
    match kind {
        syntax::EditorBlockKind::Source => theme.meta,
        syntax::EditorBlockKind::Example => theme.attribute,
        syntax::EditorBlockKind::Quote => theme.string,
        syntax::EditorBlockKind::Verse => theme.function,
        syntax::EditorBlockKind::Center => theme.heading[2],
        syntax::EditorBlockKind::Comment => theme.comment,
        syntax::EditorBlockKind::Export => theme.type_name,
        syntax::EditorBlockKind::Special(_) => theme.foreground_dim,
        syntax::EditorBlockKind::MarkdownFence => theme.meta,
    }
}

#[derive(Clone, Debug, PartialEq)]
struct EditorBlockPaintSegment {
    kind: syntax::EditorBlockKind,
    top: f32,
    bottom: f32,
    open: bool,
    close: bool,
    active: bool,
}

fn editor_block_segments(
    rows: impl IntoIterator<Item = EditorBlockPaintRow>,
) -> Vec<EditorBlockPaintSegment> {
    let mut segments = Vec::new();
    let mut current: Option<EditorBlockPaintSegment> = None;
    for row in rows {
        let Some(block) = row.block else {
            if let Some(segment) = current.take() {
                segments.push(segment);
            }
            continue;
        };
        let starts_new = current.as_ref().is_some_and(|segment| {
            block.edge == syntax::EditorBlockEdge::Open
                || block.kind != segment.kind
                || (row.top - segment.bottom).abs() > 0.75
        });
        if starts_new && let Some(segment) = current.take() {
            segments.push(segment);
        }
        let open = block.edge == syntax::EditorBlockEdge::Open;
        let close = block.edge == syntax::EditorBlockEdge::Close || row.folded;
        let segment = current.get_or_insert(EditorBlockPaintSegment {
            kind: block.kind.clone(),
            top: row.top,
            bottom: row.bottom,
            open,
            close,
            active: row.active,
        });
        segment.bottom = segment.bottom.max(row.bottom);
        segment.open |= open;
        segment.close |= close;
        segment.active |= row.active;
        if close && let Some(segment) = current.take() {
            segments.push(segment);
        }
    }
    if let Some(segment) = current {
        segments.push(segment);
    }
    segments
}

fn editor_block_horizontal_bounds(
    viewport_left: Pixels,
    viewport_right: Pixels,
    scroll_x: f32,
) -> (Pixels, Pixels) {
    let offset = px(scroll_x);
    (viewport_left - offset, viewport_right - offset)
}

fn editor_block_backgrounds(
    rows: &[PaintRow],
    text_left: Pixels,
    text_right: Pixels,
    theme: &crate::theme::Theme,
) -> Vec<PaintQuad> {
    let left = text_left + px(BLOCK_LEFT_INSET);
    let right = text_right - px(BLOCK_RIGHT_INSET);
    if right <= left {
        return Vec::new();
    }
    editor_block_segments(rows.iter().map(|row| EditorBlockPaintRow {
        block: row.block.clone(),
        top: f32::from(row.hit.visible_top),
        bottom: f32::from(row.hit.visible_bottom),
        active: row.active,
        folded: row.folded,
    }))
    .into_iter()
    .filter_map(|segment| {
        let top = segment.top
            + if segment.open {
                BLOCK_VERTICAL_INSET
            } else {
                0.0
            };
        let bottom = segment.bottom
            - if segment.close {
                BLOCK_VERTICAL_INSET
            } else {
                0.0
            };
        if bottom <= top {
            return None;
        }
        let accent = editor_block_accent(&segment.kind, theme);
        let radius = px(BLOCK_RADIUS);
        let zero = px(0.0);
        let border = px(1.0);
        let corners = Corners {
            top_left: if segment.open { radius } else { zero },
            top_right: if segment.open { radius } else { zero },
            bottom_right: if segment.close { radius } else { zero },
            bottom_left: if segment.close { radius } else { zero },
        };
        let border_widths = Edges {
            top: if segment.open { border } else { zero },
            right: border,
            bottom: if segment.close { border } else { zero },
            left: border,
        };
        let fill_alpha = if segment.active { 0x11 } else { 0x0c };
        let border_alpha = if segment.active { 0x57 } else { 0x2e };
        Some(quad(
            Bounds::from_corners(point(left, px(top)), point(right, px(bottom))),
            corners,
            rgba((accent << 8) | fill_alpha),
            border_widths,
            rgba((accent << 8) | border_alpha),
            BorderStyle::default(),
        ))
    })
    .collect()
}

fn editor_source_gutter_backgrounds(
    rows: &[PaintRow],
    text_left: Pixels,
    text_right: Pixels,
    theme: &crate::theme::Theme,
) -> Vec<PaintQuad> {
    let left = text_left + px(BLOCK_LEFT_INSET + SOURCE_GUTTER_INSET);
    let right = left + px(SOURCE_GUTTER_WIDTH);
    if right >= text_right - px(BLOCK_RIGHT_INSET) {
        return Vec::new();
    }
    editor_block_segments(rows.iter().map(|row| EditorBlockPaintRow {
        block: row.block.clone(),
        top: f32::from(row.hit.visible_top),
        bottom: f32::from(row.hit.visible_bottom),
        active: row.active,
        folded: row.folded,
    }))
    .into_iter()
    .filter(|segment| is_source_block_kind(&segment.kind))
    .filter_map(|segment| {
        let top = segment.top
            + if segment.open {
                BLOCK_VERTICAL_INSET + SOURCE_GUTTER_INSET
            } else {
                0.0
            };
        let bottom = segment.bottom
            - if segment.close {
                BLOCK_VERTICAL_INSET + SOURCE_GUTTER_INSET
            } else {
                0.0
            };
        if bottom <= top {
            return None;
        }
        let accent = editor_block_accent(&segment.kind, theme);
        let radius = px((BLOCK_RADIUS - SOURCE_GUTTER_INSET).max(0.0));
        let zero = px(0.0);
        let corners = Corners {
            top_left: if segment.open { radius } else { zero },
            top_right: zero,
            bottom_right: zero,
            bottom_left: if segment.close { radius } else { zero },
        };
        Some(quad(
            Bounds::from_corners(point(left, px(top)), point(right, px(bottom))),
            corners,
            rgba((accent << 8) | 0x06),
            Edges {
                top: zero,
                right: px(1.0),
                bottom: zero,
                left: zero,
            },
            rgba((accent << 8) | 0x24),
            BorderStyle::default(),
        ))
    })
    .collect()
}

fn editor_row_background(
    style: syntax::EditorStyleId,
    active: bool,
    bounds: Bounds<Pixels>,
    theme: &crate::theme::Theme,
) -> Option<PaintQuad> {
    let color = match style {
        syntax::EditorStyleId::CodeBoundary | syntax::EditorStyleId::Code => return None,
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
        EditorBlockPaintRow, MAX_ANIMATED_PAINT_LINES, animated_paint_lines,
        editor_block_horizontal_bounds, editor_block_segments, editor_block_text_inset,
        folded_display_text, scroll_is_at_end, stabilized_scroll_y,
    };
    use crate::editor::{
        layout_map::EditorLayoutMap,
        syntax::{EditorBlockDecoration, EditorBlockEdge, EditorBlockKind},
    };
    use gpui::px;

    fn block_row(edge: EditorBlockEdge, top: f32) -> EditorBlockPaintRow {
        EditorBlockPaintRow {
            block: Some(EditorBlockDecoration {
                kind: EditorBlockKind::Source,
                edge,
                body_line: (edge == EditorBlockEdge::Body).then_some(1),
            }),
            top,
            bottom: top + 28.0,
            active: edge == EditorBlockEdge::Body,
            folded: false,
        }
    }

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
    fn block_chrome_scrolls_horizontally_with_its_source_text() {
        let (left, right) = editor_block_horizontal_bounds(px(100.0), px(700.0), 180.0);
        assert_eq!(left, px(-80.0));
        assert_eq!(right, px(520.0));
    }

    #[test]
    fn contiguous_block_rows_form_one_active_closed_container() {
        let segments = editor_block_segments([
            block_row(EditorBlockEdge::Open, 0.0),
            block_row(EditorBlockEdge::Body, 28.0),
            block_row(EditorBlockEdge::Close, 56.0),
        ]);

        assert_eq!(segments.len(), 1);
        assert!(segments[0].open);
        assert!(segments[0].close);
        assert!(segments[0].active);
        assert_eq!(segments[0].top, 0.0);
        assert_eq!(segments[0].bottom, 84.0);
    }

    #[test]
    fn every_decorated_block_adds_visual_text_inset() {
        let decoration = |kind| EditorBlockDecoration {
            kind,
            edge: EditorBlockEdge::Body,
            body_line: Some(1),
        };
        let source = decoration(EditorBlockKind::Source);
        let markdown = decoration(EditorBlockKind::MarkdownFence);
        let quote = decoration(EditorBlockKind::Quote);

        assert_eq!(editor_block_text_inset(Some(&source)), 42.0);
        assert_eq!(editor_block_text_inset(Some(&markdown)), 42.0);
        assert_eq!(editor_block_text_inset(Some(&quote)), 16.0);
        assert_eq!(editor_block_text_inset(None), 0.0);
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
