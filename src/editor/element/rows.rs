//! Decides which source lines this frame paints, and shapes each of them into
//! a paint row plus its background quads, link hits and caret.
use std::collections::HashMap;
use std::ops::Range;

use gpui::TextStyle;

use crate::document::{DocumentFormat, DocumentSnapshot, Selection};
use crate::editor::layout_map::EditorLayoutMap;
use crate::editor::table_layout::TableVisualLayout;
use crate::theme::Theme;

use crate::editor::inline_image::INLINE_IMAGE_VERTICAL_PADDING;

use super::state::InlineImage;
use super::*;

pub(super) const MAX_ANIMATED_PAINT_LINES: u64 = 192;
pub(super) fn animated_paint_lines(
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

pub(super) fn visible_source_lines(display_map: &EditorLayoutMap, lines: Range<u64>) -> Vec<u64> {
    let start = display_map.visible_ordinal_for_line(lines.start);
    let end = display_map.visible_ordinal_for_line(lines.end);
    visible_source_ordinals(display_map, start..end)
}

pub(super) fn visible_source_ordinals(
    display_map: &EditorLayoutMap,
    ordinals: Range<u64>,
) -> Vec<u64> {
    ordinals
        .filter_map(|ordinal| display_map.source_line_for_visible_ordinal(ordinal))
        .collect()
}

pub(super) struct RowShaping<'a> {
    pub(super) editor: &'a SemanticEditor,
    pub(super) snapshot: &'a DocumentSnapshot,
    pub(super) window: &'a mut Window,
    pub(super) cx: &'a App,
    pub(super) theme: &'a Theme,
    pub(super) style: &'a TextStyle,
    pub(super) font_size: Pixels,
    pub(super) generated: bool,
    pub(super) styles_pending: bool,
    pub(super) style_snapshot: &'a syntax::SparseEditorStyleSnapshot,
    pub(super) document_format: DocumentFormat,
    pub(super) editor_focused: bool,
    pub(super) selection: Selection,
    pub(super) marked: Option<ByteRange>,
    pub(super) scroll_y: f32,
    pub(super) fold_animation_active: bool,
    pub(super) fold_ranges: Arc<[Range<u64>]>,
    pub(super) fold_scale: f32,
    pub(super) text_origin_x: Pixels,
    pub(super) wrap_width: f32,
    pub(super) inline_images: &'a HashMap<u64, InlineImage>,
    pub(super) bounds: Bounds<Pixels>,
    pub(super) next_y: f32,
    pub(super) rows: Vec<PaintRow>,
    pub(super) tag_pill_quads: Vec<PaintQuad>,
    pub(super) swatch_quads: Vec<PaintQuad>,
    pub(super) hover_quads: Vec<PaintQuad>,
    pub(super) selection_quads: Vec<PaintQuad>,
    pub(super) link_hits: Vec<crate::editor::LinkHit>,
    pub(super) caret: Option<PaintQuad>,
    pub(super) image_resize_handles: Vec<InlineImageResizeHandlePaint>,
}

impl RowShaping<'_> {
    /// Shapes one visible source line into a paint row plus its decorations.
    #[allow(clippy::too_many_lines)]
    pub(super) fn shape_row(&mut self, line_number: u64) {
        if self.editor.display_map.is_hidden(line_number) {
            return;
        }
        let line = LineIndex(line_number);
        let Ok(full_range) = self.snapshot.line_range(line) else {
            return;
        };
        let anchor = self
            .snapshot
            .line_index_at(self.selection.head())
            .ok()
            .filter(|selection_line| *selection_line == line)
            .map(|_| self.selection.head());
        let Some(source_line) = self
            .editor
            .display_map
            .source_line(self.snapshot, line, anchor)
        else {
            return;
        };
        let source_content_range = source_line.source_range;
        let content_range = source_line.visible_range;
        let display = source_line.display;
        let folded = self.editor.fold_markers.contains(&line_number);
        let line_animation_scale = if self
            .fold_ranges
            .iter()
            .any(|range| range.contains(&line_number))
        {
            self.fold_scale
        } else {
            1.0
        };
        let fallback_style;
        let line_style = if self.generated {
            fallback_style = syntax::EditorLineStyle::pending_fallback(source_content_range);
            &fallback_style
        } else if let Some(line_style) = self.style_snapshot.line(line_number) {
            line_style
        } else {
            debug_assert!(self.styles_pending);
            fallback_style = syntax::EditorLineStyle::pending_fallback(source_content_range);
            &fallback_style
        };
        let table_columns = if line_style.id == syntax::EditorStyleId::Table {
            self.editor
                .table_columns(self.snapshot, line, self.document_format)
        } else {
            None
        };
        let text_inset = editor_block_text_inset(line_style.block.as_ref());
        let row_wrap_width = if text_inset > 0.0 {
            (self.wrap_width - text_inset - BLOCK_RIGHT_INSET - BLOCK_TEXT_RIGHT_PADDING).max(1.0)
        } else {
            self.wrap_width
        };
        let effective_wrap_width = self
            .editor
            .display_map
            .soft_wrap()
            .then_some(px(row_wrap_width));
        let row_text_origin_x = self.text_origin_x + px(text_inset);
        let mut metrics = line_style
            .metrics
            .scaled(self.editor.content_font_size().scale());
        let display_text = folded_display_text(display.text.clone(), folded);
        let inline_image_source = (!folded)
            .then(|| self.inline_images.get(&line_number).cloned())
            .flatten();
        if let Some(image) = &inline_image_source {
            metrics.before = INLINE_IMAGE_VERTICAL_PADDING;
            metrics.line_height = image.height;
            metrics.after = INLINE_IMAGE_VERTICAL_PADDING;
        }
        let text: gpui::SharedString = display_text.into();
        let base_run = TextRun {
            len: text.len(),
            font: self.style.font(),
            color: gpui::rgb(self.theme.foreground).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let marked_display = local_marked(self.marked, content_range, &display);
        let mut semantic_row = Vec::new();
        let mut active_inline_code = None;
        let runs = if let Some(highlights) = self.editor.generated_highlights.as_ref() {
            highlights.get(line_number as usize).map_or_else(
                || vec![base_run.clone()],
                |highlights| {
                    crate::editor::read_only::highlighted_runs(base_run.clone(), highlights)
                },
            )
        } else {
            semantic_row = syntax::semantic_spans(
                self.editor.session.read(self.cx).syntax_path(),
                &text,
                line_style,
            );
            let caret = anchor
                .filter(|offset| {
                    self.editor_focused
                        && *offset >= content_range.start
                        && *offset <= content_range.end
                })
                .map(|offset| {
                    display.source_to_display((offset.0 - content_range.start.0) as usize)
                });
            active_inline_code = syntax::emphasize_inline_delimiters(&mut semantic_row, caret);
            let mut runs = syntax::runs_from_spans(
                &semantic_row,
                base_run,
                line_style,
                marked_display.clone(),
                self.theme,
            );
            syntax::apply_swatch_text(&mut runs, &semantic_row, self.theme);
            runs
        };
        let shaped_font_size = px(f32::from(self.font_size) * metrics.font_scale);
        let table_layout = table_columns.as_ref().and_then(|columns| {
            TableVisualLayout::shape(
                &text,
                &runs,
                shaped_font_size,
                columns,
                self.document_format,
                effective_wrap_width,
                self.window.text_system(),
            )
        });
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
        shape_key.generated_line = self.generated.then_some(line_number);
        shape_key.active_inline_code = active_inline_code.map(|range| (range.start, range.end));
        let layout = self
            .editor
            .shape_cache
            .get(&shape_key)
            .cloned()
            .unwrap_or_else(|| {
                self.window
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
        } else if let Some(table) = &table_layout {
            table.visual_rows
        } else {
            layout.wrap_boundaries().len() + 1
        };
        let number: gpui::SharedString = (line_number + 1).to_string().into();
        let active_gutter = anchor.is_some() && self.editor_focused;
        let mut gutter_font = self.style.font();
        if active_gutter {
            gutter_font.weight = FontWeight::SEMIBOLD;
        }
        let gutter_run = TextRun {
            len: number.len(),
            font: gutter_font,
            color: gpui::rgb(if active_gutter {
                self.theme.link
            } else {
                self.theme.foreground_dim
            })
            .into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let gutter_layout =
            self.window
                .text_system()
                .shape_line(number, self.font_size, &[gutter_run], None);
        let source_line_number_layout = line_style
            .block
            .as_ref()
            .filter(|block| is_source_block_kind(&block.kind))
            .and_then(|block| {
                block
                    .body_line
                    .map(|body_line| (body_line, editor_block_accent(&block.kind, self.theme)))
            })
            .map(|(body_line, accent)| {
                let number: gpui::SharedString = body_line.to_string().into();
                let run = TextRun {
                    len: number.len(),
                    font: self.style.font(),
                    color: rgba((accent << 8) | 0xd0).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                self.window.text_system().shape_line(
                    number,
                    px(f32::from(self.font_size) * SOURCE_LINE_NUMBER_FONT_SCALE),
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
        let block_top_y = if self.fold_animation_active {
            self.editor.animated_line_start_y(line_number)
        } else {
            self.next_y
        };
        let block_top = self.bounds.top() + px(block_top_y - self.scroll_y);
        let origin_y = block_top + px(metrics.before);
        let animated_height = total_height * line_animation_scale;
        let animation_clip_y =
            (line_animation_scale < 0.999).then_some((block_top, block_top + px(animated_height)));
        let inline_image = inline_image_source.as_ref().map(|image| {
            let image_bounds = Bounds::new(
                point(row_text_origin_x, origin_y),
                size(px(image.width), px(image.height)),
            );
            InlineImagePaint {
                image: image.image.clone(),
                bounds: image_bounds,
            }
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

        // The grip sits inside the image's bottom-right corner; a fold
        // transition skips it because the row is only partially painted.
        if let (Some(preview), Some(source)) = (inline_image.as_ref(), inline_image_source.as_ref())
            && line_animation_scale >= 0.999
        {
            let grip = Bounds::new(
                point(
                    preview.bounds.right() - px(INLINE_IMAGE_RESIZE_HANDLE_SIZE),
                    preview.bounds.bottom() - px(INLINE_IMAGE_RESIZE_HANDLE_SIZE),
                ),
                size(
                    px(INLINE_IMAGE_RESIZE_HANDLE_SIZE),
                    px(INLINE_IMAGE_RESIZE_HANDLE_SIZE),
                ),
            );
            let interaction_bounds = Bounds::new(
                point(
                    grip.left() - px(INLINE_IMAGE_RESIZE_HANDLE_SLOP),
                    grip.top() - px(INLINE_IMAGE_RESIZE_HANDLE_SLOP),
                ),
                size(
                    px(INLINE_IMAGE_RESIZE_HANDLE_SIZE + INLINE_IMAGE_RESIZE_HANDLE_SLOP * 2.0),
                    px(INLINE_IMAGE_RESIZE_HANDLE_SIZE + INLINE_IMAGE_RESIZE_HANDLE_SLOP * 2.0),
                ),
            );
            self.image_resize_handles
                .push(InlineImageResizeHandlePaint {
                    bounds: grip,
                    interaction_bounds,
                    hitbox: self
                        .window
                        .insert_hitbox(interaction_bounds, HitboxBehavior::Normal),
                    image_bounds: preview.bounds,
                    image_hitbox: self
                        .window
                        .insert_hitbox(preview.bounds, HitboxBehavior::Normal),
                    line: line_number,
                    line_start: source.line_start,
                });
        }

        if !semantic_row.is_empty() && inline_image.is_none() && line_animation_scale >= 0.999 {
            push_tag_pill_quads(
                &mut self.tag_pill_quads,
                &hit,
                &semantic_row,
                px(row_wrap_width),
                self.theme,
                self.editor.inline_tag_highlight(),
            );
        }

        // Document-authored hex colors ride in the background layer, below
        // hover, search and selection, so those highlights stay visible.
        if inline_image.is_none() && line_animation_scale >= 0.999 {
            push_swatch_quads(
                &mut self.swatch_quads,
                &hit,
                &semantic_row,
                px(row_wrap_width),
                self.theme,
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
            push_cookie_progress_quads(
                &mut self.hover_quads,
                &hit,
                range,
                ratio,
                self.theme,
                self.window,
            );
        }

        for span in &semantic_row {
            if let Some(meta) = &span.link {
                self.link_hits.push(crate::editor::LinkHit {
                    line,
                    display_range: span.bytes.clone(),
                    meta: meta.clone(),
                });
            }
        }
        if let Some(hovered) = self
            .editor
            .hovered_link
            .as_ref()
            .filter(|(hover_line, _)| *hover_line == line)
            .and_then(|(_, range)| {
                self.editor
                    .link_hits
                    .iter()
                    .find(|hit| hit.line == line && hit.display_range == *range)
            })
            && inline_image.is_none()
            && line_animation_scale >= 0.999
        {
            push_link_hover_quad(
                &mut self.hover_quads,
                &hit,
                hovered,
                px(row_wrap_width),
                self.theme,
            );
        }

        let first = self
            .editor
            .search_ranges
            .partition_point(|r| r.end <= full_range.start);
        for range in self.editor.search_ranges[first..]
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
                    &mut self.selection_quads,
                    &hit,
                    hit.display.source_to_display(start),
                    hit.display.source_to_display(end),
                    px(row_wrap_width),
                    Some(*range) == self.editor.search_current,
                );
            }
        }
        if let Some(range) = self.editor.timestamp_highlight() {
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
                RangeHighlight::rounded(rgba((self.theme.date << 8) | 0x22).into()).paint(
                    &mut self.hover_quads,
                    &hit,
                    hit.display.source_to_display(start)..hit.display.source_to_display(end),
                    px(row_wrap_width),
                );
            }
        }
        if let Some(range) = self.editor.todo_highlight() {
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
                RangeHighlight::rounded(rgba((self.theme.todo << 8) | 0x22).into()).paint(
                    &mut self.hover_quads,
                    &hit,
                    hit.display.source_to_display(start)..hit.display.source_to_display(end),
                    px(row_wrap_width),
                );
            }
        }
        if let Some(range) = self.editor.inline_background_highlight() {
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
                RangeHighlight::rounded(rgba((self.theme.link << 8) | 0x22).into()).paint(
                    &mut self.hover_quads,
                    &hit,
                    hit.display.source_to_display(start)..hit.display.source_to_display(end),
                    px(row_wrap_width),
                );
            }
        }
        let selected = self.selection.range();
        let selected_start = selected.start.0.max(full_range.start.0);
        let selected_end = selected.end.0.min(full_range.end.0);
        if inline_image.is_none() && selected_start < selected_end && line_animation_scale >= 0.999
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
                &mut self.selection_quads,
                &hit,
                local_start,
                local_end,
                selected_end > source_content_range.end.0,
                px(row_wrap_width),
            );
        }

        if let Some(image) = inline_image.as_ref()
            && anchor.is_some()
            && self.selection.is_empty()
            && self.selection.head() >= content_range.start
            && self.selection.head() <= content_range.end
            && line_animation_scale >= 0.999
        {
            let caret_x = if self.selection.head() <= content_range.start {
                image.bounds.left() - px(2.0)
            } else {
                image.bounds.right() + px(1.0)
            };
            self.caret = Some(
                fill(
                    Bounds::new(
                        point(caret_x, image.bounds.top() + px(1.0)),
                        size(px(4.0), (image.bounds.size.height - px(2.0)).max(px(1.0))),
                    ),
                    gpui::rgb(self.theme.foreground),
                )
                .corner_radii(px(0.8)),
            );
        } else if inline_image.is_none()
            && anchor.is_some()
            && self.selection.is_empty()
            && self.selection.head() >= content_range.start
            && self.selection.head() <= content_range.end
            && line_animation_scale >= 0.999
        {
            let source_local = self
                .selection
                .head()
                .0
                .saturating_sub(content_range.start.0)
                .min(hit.layout.len() as u64) as usize;
            let local = hit.display.source_to_display(source_local);
            let position = hit.position_for_display_index(local).unwrap_or_default();
            self.caret = Some(
                fill(
                    Bounds::new(
                        point(
                            row_text_origin_x + position.x,
                            origin_y + position.y + px(2.0),
                        ),
                        size(px(4.0), px((metrics.line_height - 4.0).max(1.0))),
                    ),
                    gpui::rgb(self.theme.foreground),
                )
                .corner_radii(px(0.8)),
            );
        }
        self.rows.push(PaintRow {
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
                self.theme,
            ),
            animation_clip_y,
            inline_image,
        });
        if !self.fold_animation_active {
            self.next_y += total_height;
        }
    }
}
