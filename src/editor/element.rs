use std::time::Duration;
#[cfg(feature = "benchmarks")]
use std::time::Instant;
use std::{ops::Range, sync::Arc};

use gpui::{
    App, BorderStyle, Bounds, ContentMask, Corners, CursorStyle, Edges, Element, ElementId,
    ElementInputHandler, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, LayoutId, PaintQuad,
    Pixels, RenderImage, ShapedLine, Style, TextAlign, TextRun, Window, WrappedLine, fill, outline,
    point, px, quad, relative, rgb, rgba, size,
};

use crate::{
    document::{ByteOffset, ByteRange, LineIndex, TextSnapshot},
    theme::current_theme,
};

#[cfg(feature = "benchmarks")]
use super::FrameBenchmarkAction;
use super::{
    HitRow, SemanticEditor, ShapeKey, TableVisualFragment, TableVisualLayout,
    highlight::RangeHighlight,
    layout_map::EditorLayoutMap,
    minimap_media::{
        MinimapImagePaint, geometry as editor_minimap_media_geometry,
        image_path as editor_minimap_image_path,
    },
    syntax,
};

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
// Statistics-cookie progress bar. Height and gap are em-derived so the bar scales with
// the content font, and the gap clears the brackets, which descend below the baseline.
const COOKIE_BAR_HEIGHT_EM: f32 = 0.15;
const COOKIE_BAR_MIN_HEIGHT: f32 = 2.0;
const COOKIE_BAR_MAX_HEIGHT: f32 = 4.0;
const COOKIE_BAR_GAP_EM: f32 = 0.22;
// Stand-in when the text system cannot report the bracket ink boxes.
const COOKIE_BAR_SIDE_INSET_EM: f32 = 0.12;
const COOKIE_BAR_TRACK_ALPHA: u32 = 0x59;
const COOKIE_BAR_TRACK_ALPHA_DARK: u32 = 0x66;
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
    tag_pills: Vec<PaintQuad>,
    swatches: Vec<PaintQuad>,
    hover_quads: Vec<PaintQuad>,
    link_hits: Vec<super::LinkHit>,
    source_run_buttons: Vec<SourceRunButtonPaint>,
    source_copy_buttons: Vec<super::source_copy::CopyButtonPaint>,
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
    media: Vec<MinimapImagePaint>,
    overlays: Vec<PaintQuad>,
}

#[derive(Clone, Copy)]
struct MinimapMediaCandidate {
    line: u64,
    line_start: u64,
    row_offset_units: f32,
    row_height_units: f32,
}

struct MinimapRasterRequest {
    key: super::minimap::RasterKey,
    layout: Arc<EditorLayoutMap>,
    path: std::path::PathBuf,
    snapshot: crate::document::DocumentSnapshot,
    source_lines: Vec<u64>,
    raster_lines: Vec<Option<super::minimap::RasterSourceRow>>,
    media_candidates: Vec<MinimapMediaCandidate>,
    syntax_service: Arc<syntax::EditorSyntaxService>,
    theme: crate::theme::Theme,
    content_top: f32,
    viewport_generation: u64,
    scale_factor: f32,
    density: crate::minimap::Density,
    line_height: f32,
    visible_rows: usize,
    repeated_rows: usize,
    telemetry: Arc<super::minimap::EditorMinimapTelemetry>,
    epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    expected_epoch: u64,
}

struct MinimapLayoutPreparationRequest {
    key: super::minimap::PreparedLayoutKey,
    epoch: u64,
    cancellation_epoch: Arc<std::sync::atomic::AtomicU64>,
    layout: EditorLayoutMap,
    snapshot: crate::document::DocumentSnapshot,
    syntax_service: Arc<syntax::EditorSyntaxService>,
    text_system: Arc<gpui::TextSystem>,
    font: gpui::Font,
    font_size: Pixels,
    theme: crate::theme::Theme,
    fold_markers: Arc<std::collections::HashSet<u64>>,
}

struct PaintRow {
    hit: HitRow,
    gutter_layout: ShapedLine,
    source_line_number_layout: Option<ShapedLine>,
    shape_key: ShapeKey,
    visual_rows: usize,
    metrics: syntax::BlockMetrics,
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

fn slice_text_runs(runs: &[TextRun], range: Range<usize>) -> Vec<TextRun> {
    let mut sliced = Vec::new();
    let mut offset = 0usize;
    for run in runs {
        let run_range = offset..offset + run.len;
        let start = run_range.start.max(range.start);
        let end = run_range.end.min(range.end);
        if start < end {
            let mut run = run.clone();
            run.len = end - start;
            sliced.push(run);
        }
        offset = run_range.end;
        if offset >= range.end {
            break;
        }
    }
    sliced
}

fn table_visual_layout(
    text: &gpui::SharedString,
    runs: &[TextRun],
    font_size: Pixels,
    columns: &[usize],
    format: crate::document::DocumentFormat,
    text_system: &gpui::WindowTextSystem,
) -> Option<Arc<TableVisualLayout>> {
    let delimiters = crate::document::table::delimiter_offsets(text, format)
        .into_iter()
        .map(|start| start..start + 1)
        .collect::<Vec<_>>();
    if delimiters.len() < 2 || columns.is_empty() {
        return None;
    }

    let mut base = runs.first()?.clone();
    base.len = 1;
    let space: gpui::SharedString = " ".into();
    let space_advance = text_system
        .shape_line(space, font_size, std::slice::from_ref(&base), None)
        .width();
    let mut fragments = Vec::with_capacity(delimiters.len() * 2 + 1);
    let shape_fragment = |range: Range<usize>, x: Pixels| -> Option<TableVisualFragment> {
        if range.is_empty() {
            return None;
        }
        let fragment_text: gpui::SharedString = text[range.clone()].to_owned().into();
        let fragment_runs = slice_text_runs(runs, range.clone());
        (!fragment_runs.is_empty()).then(|| TableVisualFragment {
            display_range: range,
            x,
            layout: Arc::new(text_system.shape_line(
                fragment_text,
                font_size,
                &fragment_runs,
                None,
            )),
        })
    };

    let first = delimiters.first()?.clone();
    let indent = shape_fragment(0..first.start, Pixels::ZERO);
    let mut delimiter_x = indent
        .as_ref()
        .map_or(Pixels::ZERO, |fragment| fragment.layout.width());
    if let Some(indent) = indent {
        fragments.push(indent);
    }

    for (column, delimiter_range) in delimiters.iter().enumerate() {
        let delimiter = shape_fragment(delimiter_range.clone(), delimiter_x)?;
        let delimiter_width = delimiter.layout.width();
        fragments.push(delimiter);

        let segment_end = delimiters
            .get(column + 1)
            .map_or(text.len(), |next| next.start);
        if let Some(segment) = shape_fragment(
            delimiter_range.end..segment_end,
            delimiter_x + delimiter_width,
        ) {
            fragments.push(segment);
        }
        if delimiters.get(column + 1).is_some() {
            let logical_width = columns.get(column).copied().unwrap_or(1).saturating_add(2) as f32;
            delimiter_x += delimiter_width + space_advance * logical_width;
        }
    }

    let width = fragments.iter().fold(Pixels::ZERO, |width, fragment| {
        width.max(fragment.x + fragment.layout.width())
    });
    Some(Arc::new(TableVisualLayout {
        fragments: fragments.into(),
        width,
        len: text.len(),
    }))
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
            if editor.minimap.settle_layout_raster_invalidation() {
                window.request_animation_frame();
            }
        });
        let digits = snapshot.len_lines().max(1).ilog10() + 1;
        let editor_font_size = self.editor.read(cx).font_size_px();
        let generated = self.editor.read(cx).generated_highlights.is_some();
        let gutter_width = digits as f32 * editor_font_size * 0.6 + GUTTER_PADDING * 2.0;
        let (minimap_full_width, minimap_visible, minimap_reveal) = {
            let editor = self.editor.read(cx);
            (
                editor.minimap.width,
                editor.minimap.visible,
                editor.minimap.reveal,
            )
        };
        let (minimap_layout_width, minimap_visual_width) = crate::motion::sliding_panel_widths(
            minimap_full_width,
            minimap_visible,
            minimap_reveal,
        );
        let target_wrap_width =
            (f32::from(bounds.size.width) - gutter_width - minimap_layout_width).max(1.0);
        self.editor.update(cx, |editor, _| {
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
            let editor = self.editor.read(cx);
            let viewport_height = f32::from(bounds.size.height);
            // A width change invalidates all measured wrap heights. At the document
            // end, clearing them would paint one sparse/estimated frame before the
            // complete target-width layout is ready. Keep the current layout
            // authoritative until that replacement can happen atomically.
            editor.layout_reflow_pending
                && editor.display_map.wrap_width().to_bits() != target_wrap_width.to_bits()
                && (editor.scroll_at_end
                    || scroll_is_at_end(
                        editor.scroll_y,
                        viewport_height,
                        editor.animated_document_height(),
                    ))
        };
        let wrap_width = if defer_minimap_reflow {
            self.editor.read(cx).display_map.wrap_width()
        } else {
            target_wrap_width
        };
        self.editor.update(cx, |editor, _| {
            let viewport_height = f32::from(bounds.size.height);
            let was_at_end = editor.scroll_at_end
                || scroll_is_at_end(
                    editor.scroll_y,
                    viewport_height,
                    editor.animated_document_height(),
                );
            // Toggling the minimap changes the wrapping width. At the document end,
            // clearing all measured rows immediately makes the bottom camera use a
            // baseline-height estimate until the complete minimap layout arrives.
            // Keep the old, coherent layout for that short preparation window and
            // publish the new-width layout atomically below.
            let anchor_line = editor.animated_line_at_y(editor.scroll_y);
            let anchor_start = editor.animated_line_start_y(anchor_line);
            let anchor_height = editor.animated_line_height_px(anchor_line).max(1.0);
            let anchor_fraction =
                ((editor.scroll_y - anchor_start) / anchor_height).clamp(0.0, 1.0);
            let layout_reconfigured = !defer_minimap_reflow
                && editor
                    .display_map
                    .configure(snapshot.len_lines(), target_wrap_width);
            if layout_reconfigured {
                editor.layout_reflow_pending = false;
            }
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
                        target_wrap_width.min(INLINE_IMAGE_MAX_WIDTH),
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
            editor.scroll_at_end = was_at_end;
        });
        let inline_image_candidates = {
            let editor = self.editor.read(cx);
            let document_path = editor.session.read(cx).syntax_path().to_path_buf();
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
                let loaded = window.use_asset::<super::image_loader::EditorImageLoader>(&path, cx);
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
            theme,
            cx.text_system().clone(),
        );
        let mut rows = Vec::with_capacity(paint_lines.len());
        let mut selection_quads = Vec::new();
        let mut tag_pill_quads = Vec::new();
        let mut swatch_quads = Vec::new();
        let mut hover_quads = Vec::new();
        let mut link_hits = Vec::new();
        let mut caret = None;
        let mut next_y = first_line_y;
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
            let fallback_style;
            let line_style = if generated {
                fallback_style = syntax::EditorLineStyle::pending_fallback(source_content_range);
                &fallback_style
            } else if let Some(style) = style_snapshot.line(line_number) {
                style
            } else {
                debug_assert!(styles_pending);
                fallback_style = syntax::EditorLineStyle::pending_fallback(source_content_range);
                &fallback_style
            };
            let table_columns = (line_style.id == syntax::EditorStyleId::Table)
                .then(|| editor.aligned_table_column_widths(&snapshot, line, document_format))
                .flatten();
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
            let mut semantic_row = Vec::new();
            let runs = if let Some(highlights) = editor.generated_highlights.as_ref() {
                highlights.get(line_number as usize).map_or_else(
                    || vec![base_run.clone()],
                    |highlights| super::read_only::highlighted_runs(base_run.clone(), highlights),
                )
            } else {
                semantic_row = syntax::semantic_spans(
                    editor.session.read(cx).syntax_path(),
                    &text,
                    line_style,
                );
                let mut runs = syntax::runs_from_spans(
                    &semantic_row,
                    base_run,
                    line_style,
                    marked_display.clone(),
                    theme,
                );
                syntax::apply_swatch_text(&mut runs, &semantic_row, theme);
                runs
            };
            let shaped_font_size = px(f32::from(font_size) * metrics.font_scale);
            let table_layout = table_columns
                .as_ref()
                .and_then(|columns| {
                    table_visual_layout(
                        &text,
                        &runs,
                        shaped_font_size,
                        columns,
                        document_format,
                        window.text_system(),
                    )
                })
                .filter(|layout| {
                    !editor.display_map.soft_wrap() || f32::from(layout.width) <= row_wrap_width
                });
            let effective_wrap_width = (table_layout.is_none() && editor.display_map.soft_wrap())
                .then_some(px(row_wrap_width));
            let fence_backticks = markdown_fence_backticks(&text, line_style.block.as_ref());
            let mut shape_key = shape_key(
                &text,
                shaped_font_size,
                marked_display,
                effective_wrap_width,
                if fence_backticks.is_some() {
                    28 // Markdown fence glyph placement has its own cached layout.
                } else {
                    line_style.id.cache_key()
                },
                line_style.code_language.clone(),
            );
            // Equal text can carry different faces on different agenda dates.
            shape_key.generated_line = generated.then_some(line_number);
            let layout = editor
                .shape_cache
                .get(&shape_key)
                .cloned()
                .unwrap_or_else(|| {
                    window
                        .text_system()
                        .shape_text(text, shaped_font_size, &runs, effective_wrap_width, None)
                        .ok()
                        .and_then(|lines| lines.into_iter().next())
                        .map(|mut line| {
                            if let Some(range) = fence_backticks {
                                lower_fence_backticks(&mut line, range, shaped_font_size * 0.30);
                            }
                            Arc::new(line)
                        })
                        .unwrap_or_else(|| Arc::new(WrappedLine::default()))
                });
            let visual_rows = if inline_image_source.is_some() {
                1
            } else {
                layout.wrap_boundaries().len() + 1
            };
            let number: gpui::SharedString = (line_number + 1).to_string().into();
            let active_gutter = anchor.is_some() && editor_focused;
            let mut gutter_font = style.font();
            if active_gutter {
                gutter_font.weight = FontWeight::SEMIBOLD;
            }
            let gutter_run = TextRun {
                len: number.len(),
                font: gutter_font,
                color: gpui::rgb(if active_gutter {
                    theme.link
                } else {
                    theme.foreground_dim
                })
                .into(),
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
                table_layout,
                inline_image_preview: inline_image.is_some(),
            };

            if !semantic_row.is_empty() && inline_image.is_none() && line_animation_scale >= 0.999 {
                push_tag_pill_quads(
                    &mut tag_pill_quads,
                    &hit,
                    &semantic_row,
                    px(row_wrap_width),
                    theme,
                    editor.inline_tag_highlight(),
                );
            }

            // Document-authored hex colors ride in the background layer, below
            // hover, search and selection, so those highlights stay visible.
            if inline_image.is_none() && line_animation_scale >= 0.999 {
                push_swatch_quads(
                    &mut swatch_quads,
                    &hit,
                    &semantic_row,
                    px(row_wrap_width),
                    theme,
                );
            }

            // Heading statistics cookie: progress rides on a thin bar under the digits.
            if matches!(line_style.id, syntax::EditorStyleId::Heading(_))
                && !folded
                && inline_image.is_none()
                && line_animation_scale >= 0.999
                && let Some((range, ratio)) =
                    crate::org_syntax::cookie::trailing_progress(&hit.layout.text)
            {
                push_cookie_progress_quads(&mut hover_quads, &hit, range, ratio, theme, window);
            }

            for span in &semantic_row {
                if let Some(meta) = &span.link {
                    link_hits.push(super::LinkHit {
                        line,
                        display_range: span.bytes.clone(),
                        meta: meta.clone(),
                    });
                }
            }
            if let Some(hovered) = editor
                .hovered_link
                .as_ref()
                .filter(|(hover_line, _)| *hover_line == line)
                .and_then(|(_, range)| {
                    editor
                        .link_hits
                        .iter()
                        .find(|hit| hit.line == line && hit.display_range == *range)
                })
                && inline_image.is_none()
                && line_animation_scale >= 0.999
            {
                push_link_hover_quad(&mut hover_quads, &hit, hovered, px(row_wrap_width), theme);
            }

            let first = editor
                .search_ranges
                .partition_point(|r| r.end <= full_range.start);
            for range in editor.search_ranges[first..]
                .iter()
                .take_while(|r| r.start < full_range.end)
            {
                let start = range
                    .start
                    .0
                    .max(content_range.start.0)
                    .saturating_sub(content_range.start.0)
                    .min(content_range.len()) as usize;
                let end = range
                    .end
                    .0
                    .min(content_range.end.0)
                    .saturating_sub(content_range.start.0)
                    .min(content_range.len()) as usize;
                if start < end && inline_image.is_none() {
                    push_search_quads(
                        &mut selection_quads,
                        &hit,
                        hit.display.source_to_display(start),
                        hit.display.source_to_display(end),
                        px(row_wrap_width),
                        Some(*range) == editor.search_current,
                    );
                }
            }
            if let Some(range) = editor.timestamp_highlight() {
                let start = range
                    .start
                    .0
                    .max(content_range.start.0)
                    .saturating_sub(content_range.start.0)
                    .min(content_range.len()) as usize;
                let end = range
                    .end
                    .0
                    .min(content_range.end.0)
                    .saturating_sub(content_range.start.0)
                    .min(content_range.len()) as usize;
                if start < end && inline_image.is_none() {
                    RangeHighlight::rounded(rgba((theme.date << 8) | 0x22).into()).paint(
                        &mut hover_quads,
                        &hit,
                        hit.display.source_to_display(start)..hit.display.source_to_display(end),
                        px(row_wrap_width),
                    );
                }
            }
            if let Some(range) = editor.todo_highlight() {
                let start = range
                    .start
                    .0
                    .max(content_range.start.0)
                    .saturating_sub(content_range.start.0)
                    .min(content_range.len()) as usize;
                let end = range
                    .end
                    .0
                    .min(content_range.end.0)
                    .saturating_sub(content_range.start.0)
                    .min(content_range.len()) as usize;
                if start < end && inline_image.is_none() {
                    RangeHighlight::rounded(rgba((theme.todo << 8) | 0x22).into()).paint(
                        &mut hover_quads,
                        &hit,
                        hit.display.source_to_display(start)..hit.display.source_to_display(end),
                        px(row_wrap_width),
                    );
                }
            }
            if let Some(range) = editor.inline_background_highlight() {
                let start = range
                    .start
                    .0
                    .max(content_range.start.0)
                    .saturating_sub(content_range.start.0)
                    .min(content_range.len()) as usize;
                let end = range
                    .end
                    .0
                    .min(content_range.end.0)
                    .saturating_sub(content_range.start.0)
                    .min(content_range.len()) as usize;
                if start < end && inline_image.is_none() {
                    RangeHighlight::rounded(rgba((theme.link << 8) | 0x22).into()).paint(
                        &mut hover_quads,
                        &hit,
                        hit.display.source_to_display(start)..hit.display.source_to_display(end),
                        px(row_wrap_width),
                    );
                }
            }
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
                caret = Some(
                    fill(
                        Bounds::new(
                            point(caret_x, image.bounds.top() + px(1.0)),
                            size(px(4.0), (image.bounds.size.height - px(2.0)).max(px(1.0))),
                        ),
                        gpui::rgb(theme.foreground),
                    )
                    .corner_radii(px(0.8)),
                );
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
                let position = hit.position_for_display_index(local).unwrap_or_default();
                caret = Some(
                    fill(
                        Bounds::new(
                            point(
                                row_text_origin_x + position.x,
                                origin_y + position.y + px(2.0),
                            ),
                            size(px(4.0), px((metrics.line_height - 4.0).max(1.0))),
                        ),
                        gpui::rgb(theme.foreground),
                    )
                    .corner_radii(px(0.8)),
                );
            }
            rows.push(PaintRow {
                hit,
                gutter_layout,
                source_line_number_layout,
                shape_key,
                visual_rows,
                metrics,
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
            schedule_syntax_builder(self.editor.clone(), service, path, syntax_snapshot, cx);
        }
        if schedule_minimap && let Some(request) = minimap_raster_request {
            schedule_minimap_raster(self.editor.clone(), request, cx);
        }
        if schedule_minimap && let Some(request) = minimap_layout_preparation {
            schedule_minimap_layout_preparation(self.editor.clone(), request, cx);
        }
        let viewport_text_left = bounds.left() + px(gutter_width);
        let viewport_text_right = bounds.right() - px(minimap_layout_width);
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
                    + row.hit.visual_width()
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
                    Some(super::SourceRunPhase::Failure) => ("!", theme.error),
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
        let source_copy_buttons = rows
            .iter()
            .filter(|row| {
                row.block.as_ref().is_some_and(|block| {
                    matches!(
                        block.kind,
                        syntax::EditorBlockKind::Source | syntax::EditorBlockKind::MarkdownFence
                    ) && block.edge == syntax::EditorBlockEdge::Open
                })
            })
            .filter_map(|row| {
                let block_right = block_content_right - px(BLOCK_RIGHT_INSET);
                let right = block_right.min(viewport_text_right) - px(10.);
                let left = right - px(26.);
                if left < viewport_text_left || row.hit.origin_y < bounds.top() {
                    return None;
                }
                // Use the painted block boundary, including its top inset and
                // rounded corner, rather than the editor's outer text rectangle.
                let probe = point(right, row.hit.visible_top + px(BLOCK_VERTICAL_INSET + 1.));
                let block = block_backgrounds
                    .iter()
                    .find(|quad| quad.bounds.contains(&probe))?
                    .bounds;
                let height = px(26.).min(block.size.height - px(10.));
                if height < px(14.) {
                    return None;
                }
                let top = (row.hit.origin_y + (row.hit.line_height - height) / 2.)
                    .max(block.top() + px(5.));
                let button = Bounds::new(point(left, top), size(px(26.), height));
                if button.bottom() > bounds.bottom() || button.bottom() > block.bottom() - px(5.) {
                    return None;
                }
                Some(super::source_copy::CopyButtonPaint::new(
                    button,
                    snapshot.revision(),
                    row.hit.range.start,
                    copy_feedback == Some((snapshot.revision(), row.hit.range.start)),
                    window,
                ))
            })
            .collect();
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
        let caret_visible = focus_handle.is_focused(window) && state.caret.is_some();
        let caret_opacity = self
            .editor
            .update(cx, |editor, cx| editor.caret_opacity(caret_visible, cx));
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
                #[cfg(feature = "benchmarks")]
                self.editor
                    .read(cx)
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
                    window.use_asset::<super::image_loader::EditorImageLoader>(&source, cx)
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
                    for pill in state.tag_pills.drain(..) {
                        window.paint_quad(pill);
                    }
                    for swatch in state.swatches.drain(..) {
                        window.paint_quad(swatch);
                    }
                    for pill in state.hover_quads.drain(..) {
                        window.paint_quad(pill);
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
                            } else if let Some(table) = &row.hit.table_layout {
                                for fragment in table.fragments.iter() {
                                    let _ = fragment.layout.paint(
                                        point(row.hit.text_origin_x + fragment.x, row.hit.origin_y),
                                        row.hit.line_height,
                                        TextAlign::Left,
                                        None,
                                        window,
                                        cx,
                                    );
                                }
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
                    for button in &state.source_copy_buttons {
                        button.paint(window, cx);
                    }
                    if focus_handle.is_focused(window)
                        && let Some(mut caret) = state.caret.take()
                    {
                        caret.background = caret.background.opacity(caret_opacity);
                        window.paint_quad(caret);
                    }
                },
            );
        });

        if self.editor.read(cx).inline_highlight().is_some() {
            window.set_window_cursor_style(CursorStyle::PointingHand);
        }
        #[cfg(feature = "benchmarks")]
        let elapsed = state.started_at.elapsed();
        let measured_rows = state
            .rows
            .iter()
            .map(|row| {
                let wrap_starts = (1..row.visual_rows)
                    .map(|visual_row| {
                        let display = row
                            .hit
                            .layout
                            .index_for_position(
                                point(px(0.0), px(visual_row as f32 * row.metrics.line_height)),
                                px(row.metrics.line_height),
                            )
                            .unwrap_or_else(|index| index);
                        row.hit.display.display_to_source(display)
                    })
                    .collect::<Vec<_>>();
                (row.hit.line.0, row.visual_rows, row.metrics, wrap_starts)
            })
            .collect::<Vec<_>>();
        let _benchmark = self.editor.update(cx, |editor, cx| {
            let viewport_changed = editor.viewport != Some(bounds);
            editor.viewport = Some(bounds);
            editor.minimap.bounds = (editor.minimap.visible && editor.minimap.reveal >= 1.0)
                .then_some(Bounds::new(
                    point(bounds.right() - px(editor.minimap.width), bounds.top()),
                    size(px(editor.minimap.width), bounds.size.height),
                ));
            editor.hit_rows = hits;
            editor.link_hits = Arc::from(std::mem::take(&mut state.link_hits));
            editor.source_run_buttons = source_run_button_hits;
            editor.source_copy.buttons = state
                .source_copy_buttons
                .iter()
                .map(|button| button.hit)
                .collect();
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
            let layout_changed = measured_rows.into_iter().fold(
                false,
                |changed, (line, rows, metrics, wrap_starts)| {
                    let height_changed = editor.display_map.update_line_layout(
                        line,
                        rows,
                        metrics.line_height,
                        metrics.before,
                        metrics.after,
                    );
                    let wraps_changed = editor
                        .display_map
                        .update_line_wrap_starts(line, &wrap_starts);
                    height_changed || wraps_changed || changed
                },
            );
            if layout_changed {
                editor.minimap.note_layout_changed();
            }
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
            editor.scroll_at_end = was_at_end;
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
            let search_revealed = editor.reveal_pending_search(&snapshot);
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
            if viewport_changed
                || layout_changed
                || scroll_settled
                || pending_reveal
                || search_revealed
            {
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
}

fn scroll_is_at_end(scroll_y: f32, viewport_height: f32, document_height: f32) -> bool {
    scroll_y + viewport_height + 0.5 >= document_height
}

fn minimap_geometry_for_layout(
    editor: &SemanticEditor,
    layout: &EditorLayoutMap,
    viewport_height: f32,
    density: crate::minimap::Density,
    line_height: f32,
) -> super::minimap::ViewportGeometry {
    let viewport = super::minimap::source_viewport_for_layout(
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
fn minimap_camera_source(layout: &EditorLayoutMap, content_top: f32) -> f64 {
    let y = content_top.max(0.0) * layout.base_line_height().max(1.0);
    let line = layout.line_at_y(y);
    let fraction =
        ((y - layout.line_start_y(line)) / layout.line_height_px(line).max(1.0)).clamp(0.0, 1.0);
    line as f64 + f64::from(fraction)
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

// Keep source bytes and all horizontal/caret geometry intact. Only the glyphs
// of a Markdown backtick boundary receive an optical vertical adjustment.
fn markdown_fence_backticks(
    text: &str,
    block: Option<&syntax::EditorBlockDecoration>,
) -> Option<Range<usize>> {
    let block = block?;
    if block.kind != syntax::EditorBlockKind::MarkdownFence
        || block.edge == syntax::EditorBlockEdge::Body
    {
        return None;
    }
    let trimmed = text.trim_start();
    let start = text.len() - trimmed.len();
    let count = trimmed.bytes().take_while(|byte| *byte == b'`').count();
    (count >= 3).then_some(start..start + count)
}

fn lower_fence_backticks(line: &mut WrappedLine, range: Range<usize>, offset: Pixels) {
    let source = &line.unwrapped_layout;
    let mut runs = source.runs.clone();
    for glyph in runs.iter_mut().flat_map(|run| &mut run.glyphs) {
        if range.contains(&glyph.index) {
            glyph.position.y += offset;
        }
    }
    let layout = gpui::WrappedLineLayout {
        unwrapped_layout: Arc::new(gpui::LineLayout {
            font_size: source.font_size,
            width: source.width,
            ascent: source.ascent,
            descent: source.descent,
            runs,
            len: source.len,
        }),
        wrap_boundaries: line.wrap_boundaries.clone(),
        wrap_width: line.wrap_width,
    };
    *std::ops::DerefMut::deref_mut(line) = Arc::new(layout);
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
        generated_line: None,
        text: text.clone(),
        font_size_bits: f32::from(font_size).to_bits(),
        wrap_width_bits: wrap_width.map_or(0, |width| f32::from(width).to_bits()),
        syntax_key,
        code_language,
        marked: marked.map(|range| (range.start, range.end)),
        theme_generation: crate::theme::theme_generation(),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_minimap(
    editor: &SemanticEditor,
    path: &std::path::Path,
    snapshot: &crate::document::DocumentSnapshot,
    semantics_pending: bool,
    bounds: Bounds<Pixels>,
    geometry: super::minimap::ViewportGeometry,
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
    let (first_unit, row_count) = super::minimap::raster_window(
        request_geometry.content_top,
        request_geometry.interaction_height,
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
            super::minimap::fill_visual_rows(
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
        let placement_content_top = super::minimap::raster_placement_content_top(
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

fn schedule_syntax_builder(
    editor: gpui::Entity<SemanticEditor>,
    service: Arc<syntax::EditorSyntaxService>,
    path: std::path::PathBuf,
    snapshot: crate::document::DocumentSnapshot,
    cx: &mut App,
) {
    let host_id = editor.read(cx).minimap.telemetry.host_id();
    let background = cx.background_executor().spawn(async move {
        let _build = tracing::info_span!("editor_semantic_checkpoint_build", host_id).entered();
        service.build_focused(&path, &snapshot);
    });
    cx.spawn(async move |cx| {
        background.await;
        editor.update(cx, |_, cx| cx.notify());
    })
    .detach();
}

fn prepare_minimap_layout_request(
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
    let key = super::minimap::PreparedLayoutKey {
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

fn schedule_minimap_layout_preparation(
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
                            super::org_commands::table_start(
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
                                    super::org_commands::aligned_table_column_widths(
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

fn apply_editor_minimap_media_dimensions(
    editor: &mut SemanticEditor,
    media: &[super::minimap::RasterMedia],
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

fn schedule_minimap_raster(
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
        let mut rich_span_budget = super::minimap::RichSpanBudget::default();
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
                media.push(super::minimap::RasterMedia {
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
                            super::minimap::TableRowGeometry::from_source(
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
                minimap_rich_span_degraded_rows_total = super::minimap::rich_span_degraded_rows(),
                "editor minimap rich spans degraded to base style"
            );
        }
        let degraded_rows = rich_span_budget.degraded_rows();
        let prepare_elapsed = prepare_started.elapsed();
        drop(_prepare);
        let raster_started = std::time::Instant::now();
        let _raster = tracing::info_span!("editor_minimap_raster", host_id).entered();
        let rasterized = super::minimap::rasterize_text_rows(
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
                .publish_frame(super::minimap::PreparedEditorMinimapFrame::new(
                    layout,
                    super::minimap::CachedRaster {
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

fn minimap_text_color(kind: syntax::EditorStyleId, theme: &crate::theme::Theme) -> u32 {
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

fn minimap_text_row(
    text: String,
    line_style: Option<&syntax::EditorLineStyle>,
    spans: Vec<syntax::EditorSemanticSpan>,
    rich_span_budget: &mut super::minimap::RichSpanBudget,
    theme: &crate::theme::Theme,
) -> super::minimap::TextRow {
    let block = line_style.and_then(|style| style.block.as_ref());
    let rich_spans = rich_span_budget.adapt(&text, spans, theme);
    super::minimap::TextRow {
        color: line_style.map_or(theme.foreground, |style| {
            minimap_text_color(style.id, theme)
        }),
        weight: if matches!(
            line_style.map(|style| style.id),
            Some(syntax::EditorStyleId::Heading(_))
        ) {
            super::minimap::TextWeight::Bold
        } else {
            super::minimap::TextWeight::Semibold
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
    let start_position = hit.position_for_display_index(start).unwrap_or_default();
    let end_position = hit
        .position_for_display_index(end)
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
                rgba((crate::theme::current_theme().accent << 8) | 0x4a),
            ));
        }
    }
}

/// Paints the opaque swatch behind every hex literal in the row. Quads stay
/// in the background layer, so hover, search and selection highlights still
/// paint on top. Fill and glyph color are per-literal, while radius and padding
/// come from [`RangeHighlight`].
fn push_swatch_quads(
    quads: &mut Vec<PaintQuad>,
    hit: &HitRow,
    spans: &[syntax::EditorSemanticSpan],
    wrap_width: Pixels,
    theme: &crate::theme::Theme,
) {
    for span in spans {
        let Some(literal) = span.swatch else {
            continue;
        };
        if span.bytes.start >= span.bytes.end {
            continue;
        }
        let fill = rgba(crate::org_syntax::color::swatch_fill(
            literal,
            theme.background,
        ));
        RangeHighlight::rounded(fill.into()).paint(quads, hit, span.bytes.clone(), wrap_width);
    }
}

/// Paints one rounded pill behind each Org tag span, keeping the `:` source
/// separators outside the fill so every byte of the raw heading stays visible.
fn push_tag_pill_quads(
    quads: &mut Vec<PaintQuad>,
    hit: &HitRow,
    spans: &[syntax::EditorSemanticSpan],
    wrap_width: Pixels,
    theme: &crate::theme::Theme,
    active: Option<ByteRange>,
) {
    for span in spans.iter().filter(|span| span.pill) {
        let start = span.bytes.start.saturating_add(1);
        let end = span.bytes.end.saturating_sub(1);
        if start >= end {
            continue;
        }
        let source_start = hit.range.start.0 + hit.display.display_to_source(start) as u64;
        let source_end = hit.range.start.0 + hit.display.display_to_source(end) as u64;
        let hovered =
            active.is_some_and(|range| range.start.0 < source_end && range.end.0 > source_start);
        // Change the existing pill's fill, never paint a second rounded rectangle.
        let fill = rgba((theme.attribute << 8) | if hovered { 0x2a } else { 0x16 });
        RangeHighlight::rounded(fill.into()).paint(quads, hit, start..end, wrap_width);
    }
}

/// Draws the statistics-cookie progress bar: a track under the digits plus the
/// meta-colored fill for `ratio`.
fn push_cookie_progress_quads(
    quads: &mut Vec<PaintQuad>,
    hit: &HitRow,
    range: Range<usize>,
    ratio: f32,
    theme: &crate::theme::Theme,
    window: &gpui::Window,
) {
    let Some(start) = hit.position_for_display_index(range.start) else {
        return;
    };
    let Some(end) = hit.position_for_display_index(range.end) else {
        return;
    };
    // A wrapped cookie would need its fill sliced per visual row; skip that rare case.
    if (f32::from(start.y) - f32::from(end.y)).abs() > f32::EPSILON {
        return;
    }
    let (left_inset, right_inset) = cookie_bar_insets(hit, &range, end.x - start.x, window);
    let left = hit.text_origin_x + start.x + left_inset;
    let right = hit.text_origin_x + end.x - right_inset;
    if right - left <= px(1.0) {
        return;
    }
    let row_top = hit.origin_y + start.y;
    let line_height = hit.line_height;
    let ascent = hit.layout.ascent();
    let descent = hit.layout.descent();
    let font_size = hit.layout.font_size();
    let (top, bottom) = cookie_bar_bounds(row_top, line_height, ascent, descent, font_size);
    let height = bottom - top;
    let radius = height / 2.0;
    let corners = Corners {
        top_left: radius,
        top_right: radius,
        bottom_right: radius,
        bottom_left: radius,
    };
    let track_alpha = if is_dark_theme(theme) {
        COOKIE_BAR_TRACK_ALPHA_DARK
    } else {
        COOKIE_BAR_TRACK_ALPHA
    };
    quads.push(quad(
        Bounds::from_corners(point(left, top), point(right, bottom)),
        corners,
        rgba((theme.meta << 8) | track_alpha),
        Edges::default(),
        rgba(0),
        BorderStyle::default(),
    ));
    let ratio = ratio.clamp(0.0, 1.0);
    let filled = left + px(f32::from(right - left) * ratio);
    if filled > left {
        quads.push(quad(
            Bounds::from_corners(point(left, top), point(filled, bottom)),
            corners,
            rgb(theme.meta),
            Edges::default(),
            rgba(0),
            BorderStyle::default(),
        ));
    }
}

/// Pulls the bar's ends from the bracket cells to the bracket strokes. The ink boxes
/// come from the shaped run's own font; the em inset stands in when they are missing.
fn cookie_bar_insets(
    hit: &HitRow,
    range: &Range<usize>,
    token_width: Pixels,
    window: &gpui::Window,
) -> (Pixels, Pixels) {
    let font_size = hit.layout.font_size();
    // Never eat more than a third of the token per side.
    let max_inset = token_width * 0.35;
    let fallback = (font_size * COOKIE_BAR_SIDE_INSET_EM).min(max_inset);
    let ink = hit
        .layout
        .runs()
        .iter()
        .find(|run| run.glyphs.iter().any(|glyph| glyph.index == range.start))
        .and_then(|run| {
            let text_system = window.text_system();
            let open = text_system
                .typographic_bounds(run.font_id, font_size, '[')
                .ok()?;
            let close = text_system
                .typographic_bounds(run.font_id, font_size, ']')
                .ok()?;
            let advance = text_system.advance(run.font_id, font_size, ']').ok()?.width;
            Some((open.origin.x, advance - close.right()))
        });
    let (left, right) = ink.unwrap_or((fallback, fallback));
    // A font reporting nothing useful must not collapse or invert the bar.
    (
        left.clamp(px(0.0), max_inset),
        right.clamp(px(0.0), max_inset),
    )
}

/// Vertical placement: a gap below the digit baseline, bounded by the row's leading.
fn cookie_bar_bounds(
    row_top: Pixels,
    line_height: Pixels,
    ascent: Pixels,
    descent: Pixels,
    font_size: Pixels,
) -> (Pixels, Pixels) {
    let line_height = line_height.max(px(1.0));
    let leading = ((line_height - ascent - descent) / 2.0).max(px(0.0));
    let height = (font_size * COOKIE_BAR_HEIGHT_EM)
        .clamp(px(COOKIE_BAR_MIN_HEIGHT), px(COOKIE_BAR_MAX_HEIGHT))
        .min(line_height);
    let baseline = row_top + leading + ascent;
    // The leading below the glyph box belongs to this row, so the bar may use it.
    let max_bottom = row_top + line_height + leading;
    let top = (baseline + font_size * COOKIE_BAR_GAP_EM)
        .min(max_bottom - height)
        .max(row_top);
    (top, top + height)
}

/// Whether the editor background is dark enough to need the stronger track alpha.
fn is_dark_theme(theme: &crate::theme::Theme) -> bool {
    let background = theme.editor_background;
    let luminance = 0.2126 * f32::from((background >> 16) as u8)
        + 0.7152 * f32::from((background >> 8) as u8)
        + 0.0722 * f32::from(background as u8);
    luminance < 128.0
}

/// Highlights the link currently under the pointer.
fn push_link_hover_quad(
    quads: &mut Vec<PaintQuad>,
    hit: &HitRow,
    link: &super::LinkHit,
    wrap_width: Pixels,
    theme: &crate::theme::Theme,
) {
    RangeHighlight::rounded(rgba((syntax::link_accent(&link.meta.kind, theme) << 8) | 0x26).into())
        .paint(quads, hit, link.display_range.clone(), wrap_width);
}

fn push_search_quads(
    quads: &mut Vec<PaintQuad>,
    hit: &HitRow,
    start: usize,
    end: usize,
    wrap_width: Pixels,
    current: bool,
) {
    let line_height = hit.line_height;
    let start_position = hit.position_for_display_index(start).unwrap_or_default();
    let end_position = hit
        .position_for_display_index(end)
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
        let right = if row == last_row {
            end_position.x
        } else {
            wrap_width
        };

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
                rgba(if current {
                    crate::theme::current_theme().search_current
                } else {
                    crate::theme::current_theme().search_match
                }),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        COOKIE_BAR_GAP_EM, COOKIE_BAR_MAX_HEIGHT, EditorBlockPaintRow, MAX_ANIMATED_PAINT_LINES,
        animated_paint_lines, apply_editor_minimap_media_dimensions, cookie_bar_bounds,
        cookie_bar_insets, editor_block_horizontal_bounds, editor_block_segments,
        editor_block_text_inset, folded_display_text, minimap_text_row, scroll_is_at_end,
        stabilized_scroll_y,
    };
    use crate::document::{DocumentSession, DocumentSnapshot, TextSnapshot};
    use crate::editor::{
        SemanticEditor,
        layout_map::EditorLayoutMap,
        minimap::RasterMedia,
        syntax::{
            EditorBlockDecoration, EditorBlockEdge, EditorBlockKind, EditorStyleId,
            EditorSyntaxService, SparseEditorStyleSnapshot, semantic_spans,
        },
    };
    use gpui::{AppContext, px};
    use std::path::Path;

    #[gpui::test]
    fn tab_alignment_holds_minimap_pixels_until_complete_layout(cx: &mut gpui::TestAppContext) {
        use crate::document::ByteOffset;
        use crate::editor::org_commands::{EditorCommandContext, TableNavigation};
        use std::sync::Arc;
        cx.update(crate::editor::init);
        for (path, table) in [
            ("align.org", "| a|bbb|\n|---+---|\n|长字段| x|\n"),
            ("align.md", "| a|bbb|\n|---|---|\n|长字段| x|\n"),
        ] {
            let path = std::path::PathBuf::from(path);
            let source = format!("{table}{}", "body text\n".repeat(200));
            let session =
                cx.new(|_| DocumentSession::from_utf8(path.clone(), source.into_bytes()).unwrap());
            let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
            view.simulate_resize(gpui::size(px(800.), px(500.)));
            view.run_until_parked();
            editor.update(view, |editor, cx| {
                let previous = editor
                    .minimap
                    .active_frame()
                    .expect("initial minimap is painted");
                assert!(editor.minimap.active_frame_has_complete_layout());
                let bounds = editor.minimap.bounds.unwrap();
                // Retain a stale bootstrap anchor while the complete frame is scrolled.
                let bootstrap = editor.minimap_viewport_geometry(bounds);
                let (total, top, bottom) =
                    editor.minimap_source_viewport(f32::from(bounds.size.height));
                editor.scroll_y = 1_200.0;
                editor.minimap.note_viewport_scrolled(1_200.0);
                editor.minimap.stabilize_viewport(
                    bootstrap,
                    total,
                    top,
                    bottom,
                    f32::from(bounds.size.height),
                    crate::minimap::Density::for_width(f32::from(bounds.size.width)),
                );
                let original_geometry = editor.minimap_viewport_geometry(bounds);
                let snapshot = editor.snapshot(cx);
                let context = EditorCommandContext::at(&path, &snapshot, ByteOffset(2)).unwrap();
                assert!(editor.align_table_from_context(
                    &snapshot,
                    &context,
                    TableNavigation::NextCell,
                    cx
                ));
                let snapshot = editor.snapshot(cx);
                assert_eq!(snapshot.revision().0, 1);
                editor.minimap.update_snapshot(&snapshot);
                // Reserve, but deliberately hold back the complete-layout job. This
                // exercises the intermediate frame regardless of executor timing.
                let preparation = super::prepare_minimap_layout_request(
                    editor,
                    &path,
                    &snapshot,
                    editor.display_map.wrap_width(),
                    gpui::font(".SystemUIFont"),
                    px(15.),
                    crate::theme::current_theme(),
                    cx.text_system().clone(),
                )
                .unwrap();
                let geometry = editor.minimap_viewport_geometry(bounds);
                assert!(
                    (geometry.content_top - original_geometry.content_top).abs() < 0.001,
                    "pending layout switched the active image to a stale bootstrap camera: {} -> {}",
                    original_geometry.content_top,
                    geometry.content_top,
                );
                let (paint, request) = super::build_minimap(
                    editor,
                    &path,
                    &snapshot,
                    false,
                    bounds,
                    geometry,
                    1.,
                    crate::theme::current_theme(),
                );
                assert!(Arc::ptr_eq(&paint.image.unwrap().0, &previous.raster.image));
                assert!(
                    request.is_none(),
                    "Tab must not rasterize new table text using an unfinished layout"
                );
                // Once the first Tab has aligned the table, the next Tab only
                // navigates: neither the document nor raster generation changes.
                let generation = editor.minimap.generation;
                let context =
                    EditorCommandContext::at(&path, &snapshot, editor.selection().head()).unwrap();
                assert!(editor.align_table_from_context(
                    &snapshot,
                    &context,
                    TableNavigation::NextCell,
                    cx
                ));
                assert_eq!(editor.snapshot(cx).revision(), snapshot.revision());
                assert_eq!(editor.minimap.generation, generation);
                let complete = Arc::new(editor.display_map.clone());
                assert!(editor.minimap.publish_prepared_layout(
                    &preparation.key,
                    preparation.epoch,
                    complete.clone()
                ));
                editor.minimap.invalidate_raster();
                let (_, request) = super::build_minimap(
                    editor,
                    &path,
                    &snapshot,
                    false,
                    bounds,
                    geometry,
                    1.,
                    crate::theme::current_theme(),
                );
                assert!(Arc::ptr_eq(
                    &request
                        .expect("minimap refresh resumes with complete geometry")
                        .layout,
                    &complete
                ));
            });
        }
    }

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
    fn minimap_rows_adapt_the_canonical_editor_semantics() {
        let source = "plain\n* heading\n- list\n| table |\n> quote\n:KEY: value\n#+title: title\n# comment\n#+begin_src rust\nlet x = 1;\n#+end_src\n";
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let lines = (0..11).collect::<Vec<_>>();
        let styles = SparseEditorStyleSnapshot::for_lines(
            Path::new("contract.org"),
            &snapshot,
            &lines,
            &EditorSyntaxService::default(),
        );
        let theme = crate::theme::current_theme();
        let expected = [
            EditorStyleId::Plain,
            EditorStyleId::Heading(1),
            EditorStyleId::List,
            EditorStyleId::Table,
            EditorStyleId::Quote,
            EditorStyleId::Property,
            EditorStyleId::Meta,
            EditorStyleId::Comment,
            EditorStyleId::CodeBoundary,
            EditorStyleId::Code,
            EditorStyleId::CodeBoundary,
        ];
        let mut rich_span_budget = super::super::minimap::RichSpanBudget::default();

        for (line, expected_style) in expected.into_iter().enumerate() {
            let style = styles.line(line as u64).expect("canonical line semantics");
            assert_eq!(style.id, expected_style);
            let text = snapshot.copy_range(style.source_range);
            let spans = semantic_spans(Path::new("contract.org"), &text, style);
            let row = minimap_text_row(text, Some(style), spans, &mut rich_span_budget, theme);
            assert_eq!(row.color, super::minimap_text_color(style.id, theme));
            assert_eq!(row.block_edge, style.block.as_ref().map(|block| block.edge));
            if line == 9 {
                assert!(
                    row.spans
                        .iter()
                        .any(|span| span.color == Some(theme.keyword))
                );
            }
        }

        for line in 8..=10 {
            let style = styles.line(line).unwrap();
            let row = minimap_text_row(
                String::new(),
                Some(style),
                Vec::new(),
                &mut rich_span_budget,
                theme,
            );
            assert!(row.block_background.is_some());
            assert!(row.block_accent.is_some());
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
    fn cookie_bar_hangs_below_the_baseline_inside_its_row() {
        // Level-two heading at the default content size: a 30px row of 20.1px type.
        let line_height = 30.0_f32;
        let font_size = 20.1_f32;
        let ascent = 18.65_f32;
        let descent = 4.74_f32;
        let leading = (line_height - ascent - descent) / 2.0;
        let baseline = leading + ascent;
        let (top, bottom) = cookie_bar_bounds(
            px(0.0),
            px(line_height),
            px(ascent),
            px(descent),
            px(font_size),
        );
        assert!(
            (f32::from(top) - (baseline + font_size * COOKIE_BAR_GAP_EM)).abs() < 0.01,
            "the bar hangs a gap below the digits: {top:?}"
        );
        // Clear of the brackets, whose glyphs descend below the baseline.
        assert!(f32::from(top) > baseline + 3.0);
        assert!(f32::from(bottom) <= line_height + leading + 0.001);
        assert!(f32::from(bottom - top) > 2.0);

        // Tight rows clamp the bar rather than let it reach the next row.
        for line_height in [10.0_f32, 12.0, 15.6, 18.0, 24.0, 40.0] {
            let leading = ((line_height - ascent - descent) / 2.0).max(0.0);
            let (top, bottom) = cookie_bar_bounds(
                px(40.0),
                px(line_height),
                px(ascent),
                px(descent),
                px(font_size),
            );
            assert!(f32::from(top) >= 40.0, "line_height {line_height}: {top:?}");
            assert!(
                f32::from(bottom) <= 40.0 + line_height + leading + 0.001,
                "line_height {line_height}: {bottom:?}"
            );
            let height = f32::from(bottom - top);
            assert!(height > 0.0 && height <= COOKIE_BAR_MAX_HEIGHT + 0.001);
        }
    }

    #[gpui::test]
    fn cookie_bar_clears_the_real_heading_metrics(cx: &mut gpui::TestAppContext) {
        cx.update(crate::editor::init);
        let source = "** DONE Phase 3：颜色收敛与深色验收 [4/7]\nbody\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                Path::new("cookie.org").to_path_buf(),
                source.as_bytes().to_vec(),
            )
            .unwrap()
        });
        let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        cx.simulate_resize(gpui::size(px(900.0), px(400.0)));
        cx.run_until_parked();
        let (line_height, font_size, ascent, descent) = cx.read(|cx| {
            let editor = editor.read(cx);
            let row = editor
                .hit_rows
                .iter()
                .find(|row| row.line.0 == 0)
                .expect("the heading row is rendered");
            (
                row.line_height,
                row.layout.font_size(),
                row.layout.ascent(),
                row.layout.descent(),
            )
        });
        let (top, bottom) = cookie_bar_bounds(px(0.0), line_height, ascent, descent, font_size);
        let line_height = f32::from(line_height);
        let ascent = f32::from(ascent);
        let descent = f32::from(descent);
        let leading = ((line_height - ascent - descent) / 2.0).max(0.0);
        let baseline = leading + ascent;
        // The real font must leave room for the gap under the digits.
        assert!(
            f32::from(top) > baseline + 3.0,
            "bar top {top:?} against baseline {baseline} (line height {line_height})"
        );
        assert!(f32::from(bottom) <= line_height + leading + 0.001);
        assert!(f32::from(bottom - top) >= 2.0);

        // Insets come from the real font's ink boxes and must never invert the bar.
        let (left_inset, right_inset, token_width) = cx.update(|window, cx| {
            let editor = editor.read(cx);
            let row = editor
                .hit_rows
                .iter()
                .find(|row| row.line.0 == 0)
                .expect("the heading row is rendered");
            let range = crate::org_syntax::cookie::trailing_progress(&row.layout.text)
                .expect("trailing cookie")
                .0;
            let start = row
                .position_for_display_index(range.start)
                .expect("cookie start");
            let end = row
                .position_for_display_index(range.end)
                .expect("cookie end");
            let width = end.x - start.x;
            let (left, right) = cookie_bar_insets(row, &range, width, window);
            (left, right, width)
        });
        assert!(f32::from(left_inset) > 0.0, "left inset {left_inset:?}");
        assert!(f32::from(right_inset) > 0.0, "right inset {right_inset:?}");
        assert!(left_inset + right_inset < token_width * 0.7);
    }

    #[gpui::test]
    fn caret_blink_resets_on_movement_and_stops_when_hidden(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(Path::new("caret.md").to_path_buf(), b"hello".to_vec())
                .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session, cx));
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.caret_opacity(true, cx), 1.0);
            assert!(editor.caret_blink_task.is_some());
            editor.caret_blink.as_mut().unwrap().2 =
                std::time::Instant::now() - std::time::Duration::from_millis(800);
            assert_eq!(editor.caret_opacity(true, cx), 0.0);
            editor.selection = super::super::Selection::caret(crate::document::ByteOffset(1));
            assert_eq!(editor.caret_opacity(true, cx), 1.0);
            editor.caret_opacity(false, cx);
            assert!(editor.caret_blink.is_none());
            assert!(editor.caret_blink_task.is_none());
        });
    }

    #[gpui::test]
    fn minimap_prefetch_commits_image_height_before_the_editor_reaches_it(
        cx: &mut gpui::TestAppContext,
    ) {
        let source = "line\n".repeat(100);
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("prefetched-image.org"),
                source.into_bytes(),
            )
            .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session, cx));

        editor.update(cx, |editor, _| {
            editor.display_map.configure(100, 800.0);
            let initial_height = editor.display_map.total_height();
            let media = [RasterMedia {
                line: 50,
                line_start: 250,
                row_offset_units: 50.0,
                row_height_units: 1.0,
                path: std::path::PathBuf::from("image.svg"),
                dimensions: Some((100, 400)),
            }];

            assert!(apply_editor_minimap_media_dimensions(editor, &media));
            let resolved_height = editor.display_map.total_height();
            assert!(resolved_height > initial_height + 300.0);
            assert_eq!(
                editor.inline_image_line_dimensions.borrow().get(&50),
                Some(&(250, 100, 400))
            );
            assert!(!apply_editor_minimap_media_dimensions(editor, &media));
            assert_eq!(editor.display_map.total_height(), resolved_height);
        });
    }

    #[test]
    fn block_chrome_scrolls_horizontally_with_its_source_text() {
        let (left, right) = editor_block_horizontal_bounds(px(100.0), px(700.0), 180.0);
        assert_eq!(left, px(-80.0));
        assert_eq!(right, px(520.0));
    }

    #[gpui::test]
    fn minimap_opened_after_scroll_keeps_visible_svg_dimensions_stable(
        cx: &mut gpui::TestAppContext,
    ) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview-basics.org");
        let source = std::fs::read(&path).unwrap();
        let session = cx.new(|_| DocumentSession::from_utf8(path.clone(), source).unwrap());
        let editor = cx.new(|cx| SemanticEditor::new(session.clone(), cx));
        let media = cx.update(|cx| {
            let snapshot = session.read(cx).snapshot();
            (0..snapshot.len_lines())
                .filter_map(|line| {
                    let range = snapshot
                        .line_content_range(crate::document::LineIndex(line))
                        .ok()?;
                    let text = snapshot.copy_range(range);
                    let target = crate::org_syntax::standalone_image_path(&text)?;
                    let path = crate::preview::resolve_image_path(&path, target);
                    let image = crate::editor::image_loader::decode_svg(
                        &std::fs::read(&path).unwrap(),
                        &cx.svg_renderer(),
                    )
                    .unwrap();
                    let dimensions = crate::preview::image_dimensions(&path).unwrap();
                    Some((
                        RasterMedia {
                            line,
                            line_start: range.start.0,
                            row_offset_units: 0.0,
                            row_height_units: 1.0,
                            path,
                            dimensions: Some(dimensions),
                        },
                        image,
                    ))
                })
                .collect::<Vec<_>>()
        });
        assert!(
            media.len() >= 2,
            "fixture must cover multiple SVGs so competing dimension writers are exercised"
        );
        editor.update(cx, |editor, cx| {
            editor.set_minimap(false, Some(144), cx);
            editor
                .display_map
                .configure(session.read(cx).snapshot().len_lines(), 1606.0);
            editor.viewport = Some(gpui::Bounds::new(
                gpui::point(px(0.0), px(0.0)),
                gpui::size(px(1800.0), px(1028.0)),
            ));
            editor.scroll_y = 748.0;
            editor.set_minimap(true, Some(144), cx);
            // Each frame measures visible body images, then the background minimap
            // publishes its discovered dimensions. These writers must converge without
            // another scroll moving the SVGs out of the body viewport.
            for _ in 0..3 {
                for (source, image) in &media {
                    let (_, (width, height)) =
                        editor.accept_inline_image_render(&source.path, image.clone());
                    editor
                        .inline_image_line_dimensions
                        .borrow_mut()
                        .insert(source.line, (source.line_start, width, height));
                    let (_, height) = crate::preview::fitted_image_size(width, height, 640.0);
                    editor
                        .display_map
                        .update_line_layout(source.line, 1, height, 6.0, 6.0);
                    assert!(
                        !apply_editor_minimap_media_dimensions(
                            editor,
                            std::slice::from_ref(source)
                        ),
                        "visible SVG must not repeatedly reject minimap publication: {:?}",
                        source.path,
                    );
                }
                assert_eq!(editor.scroll_y, 748.0);
            }
        });
        let request = editor.update(cx, |editor, cx| {
            let bounds = gpui::Bounds::new(
                gpui::point(px(1656.0), px(0.0)),
                gpui::size(px(144.0), px(1028.0)),
            );
            let snapshot = session.read(cx).snapshot();
            editor.minimap.update_snapshot(&snapshot);
            let geometry = editor.minimap_viewport_geometry(bounds);
            let (paint, request) = super::build_minimap(
                editor,
                &path,
                &snapshot,
                false,
                bounds,
                geometry,
                2.0,
                crate::theme::current_theme(),
            );
            assert!(paint.image.is_none());
            request.expect("opening minimap must schedule its first raster")
        });
        cx.update(|cx| super::schedule_minimap_raster(editor.clone(), request, cx));
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| {
            assert!(
                editor.minimap.active_frame().is_some(),
                "first raster must publish without another scroll"
            );
            assert_eq!(editor.scroll_y, 748.0);
        });
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

    #[gpui::test]
    fn swatch_quads_follow_the_literal_glyph_box(cx: &mut gpui::TestAppContext) {
        cx.update(crate::editor::init);
        let source = "palette: #ff0000 done\nbody\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                Path::new("swatch.org").to_path_buf(),
                source.as_bytes().to_vec(),
            )
            .unwrap()
        });
        let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        cx.simulate_resize(gpui::size(px(900.0), px(400.0)));
        cx.run_until_parked();
        let theme = crate::theme::current_theme();

        let span = |swatch: Option<u32>, bytes: std::ops::Range<usize>| {
            crate::editor::syntax::EditorSemanticSpan {
                bytes,
                color: None,
                weight: crate::editor::syntax::EditorSemanticWeight::Normal,
                italic: false,
                underline: false,
                strikethrough: false,
                pill: false,
                swatch,
                link: None,
            }
        };

        // A span without a swatch must not paint anything.
        cx.read(|cx| {
            let editor = editor.read(cx);
            let row = editor
                .hit_rows
                .iter()
                .find(|row| row.line.0 == 0)
                .expect("the palette row is rendered");
            let mut quads = Vec::new();
            super::push_swatch_quads(
                &mut quads,
                row,
                std::slice::from_ref(&span(None, 0..1)),
                px(800.0),
                theme,
            );
            assert!(quads.is_empty());
        });

        cx.read(|cx| {
            let editor = editor.read(cx);
            let row = editor
                .hit_rows
                .iter()
                .find(|row| row.line.0 == 0)
                .expect("the palette row is rendered");
            let text = row.layout.text.clone();
            let start = text.find("#ff0000").expect("hex literal");
            let end = start + "#ff0000".len();
            let mut quads = Vec::new();
            super::push_swatch_quads(
                &mut quads,
                row,
                std::slice::from_ref(&span(Some(0xff0000ff), start..end)),
                px(800.0),
                theme,
            );
            assert_eq!(quads.len(), 1);
            let quad = &quads[0];
            let start_x = row.position_for_display_index(start).unwrap().x;
            let end_x = row.position_for_display_index(end).unwrap().x;
            // The shared pill geometry pads the glyph box on both sides.
            let pad = px(super::RangeHighlight::PAD_X);
            let inset = px(super::RangeHighlight::INSET_Y);
            assert_eq!(quad.bounds.left(), row.text_origin_x + start_x - pad);
            assert_eq!(quad.bounds.right(), row.text_origin_x + end_x + pad);
            assert_eq!(quad.bounds.top(), row.origin_y + inset);
            assert_eq!(quad.bounds.bottom(), row.origin_y + row.line_height - inset);
            assert_eq!(
                quad.corner_radii.top_left,
                px(super::RangeHighlight::RADIUS)
            );
            assert_eq!(
                quad.corner_radii.bottom_right,
                px(super::RangeHighlight::RADIUS)
            );
        });
    }
}

#[cfg(test)]
mod resize_pin_tests {
    use super::SemanticEditor;
    use crate::document::DocumentSession;
    use gpui::{AppContext, px};

    /// Reproduction for the "drag to the very bottom, then enlarge the window"
    /// jitter: a minimap drag jumps to the end without measuring the middle of
    /// the document, so those rows still carry one-line height estimates.
    /// Enlarging the window pulls rows into the viewport and the total height
    /// wobbles between estimate and measurement. The pinned-at-end scroll must
    /// stay glued to the document bottom instead of oscillating between the
    /// pin path and the anchor path.
    #[gpui::test]
    fn enlarging_while_pinned_at_bottom_stays_pinned(cx: &mut gpui::TestAppContext) {
        cx.update(crate::editor::init);
        let line = "lorem ipsum dolor sit amet consectetur adipiscing elit ".repeat(12);
        let source = format!("{line}\n").repeat(600);
        let session =
            cx.new(|_| DocumentSession::from_utf8("pin.md".into(), source.into_bytes()).unwrap());
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));

        view.simulate_resize(gpui::size(px(520.), px(600.)));
        view.run_until_parked();

        // Jump to the very bottom the way a minimap thumb drag does: the middle
        // of the document is never measured.
        editor.update(view, |editor, cx| {
            let bounds = editor.minimap.bounds.expect("minimap painted");
            let bottom = bounds.bottom() - px(1.0);
            editor.seek_from_minimap(bottom, cx);
        });
        view.run_until_parked();
        let stable_height = editor.update(view, |editor, _| editor.animated_document_height());

        // Enlarge the window continuously the way a live window drag delivers
        // resizes: many size steps without letting the layout settle in
        // between, then sample the scroll every frame.
        let mut samples = Vec::new();
        for step in 0..40 {
            let width = 520.0 + step as f32 * 6.0;
            view.simulate_resize(gpui::size(px(width), px(600.)));
            editor.update(view, |editor, _| {
                let viewport_height = f32::from(editor.viewport.unwrap().size.height);
                let max_scroll = (editor.animated_document_height() - viewport_height).max(0.0);
                samples.push((
                    editor.scroll_y,
                    max_scroll,
                    editor.animated_document_height(),
                    editor.layout_reflow_pending,
                ));
            });
        }
        view.run_until_parked();
        editor.update(view, |editor, _| {
            let viewport_height = f32::from(editor.viewport.unwrap().size.height);
            let max_scroll = (editor.animated_document_height() - viewport_height).max(0.0);
            samples.push((
                editor.scroll_y,
                max_scroll,
                editor.animated_document_height(),
                editor.layout_reflow_pending,
            ));
        });
        for (index, (scroll_y, max_scroll, document_height, reflow_pending)) in
            samples.iter().enumerate()
        {
            assert!(
                *scroll_y <= max_scroll + 1.0,
                "frame {index}: scroll left the pinned end (scroll {scroll_y:.1} > max {max_scroll:.1}); samples: {samples:?}",
            );
            if *reflow_pending {
                assert!(
                    (*document_height - stable_height).abs() <= 0.5,
                    "frame {index}: pending reflow exposed an estimated layout height ({document_height:.1} != stable {stable_height:.1}); samples: {samples:?}",
                );
            }
        }
    }
}
