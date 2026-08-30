use std::sync::Arc;
#[cfg(feature = "benchmarks")]
use std::time::Instant;

use gpui::{
    App, Bounds, ContentMask, Element, ElementId, ElementInputHandler, GlobalElementId, LayoutId,
    PaintQuad, Pixels, ShapedLine, Style, TextAlign, TextRun, Window, WrappedLine, fill, point, px,
    relative, rgba, size,
};

use crate::{
    document::{ByteRange, LineIndex, TextSnapshot},
    theme::current_theme,
};

#[cfg(feature = "benchmarks")]
use super::FrameBenchmarkAction;
use super::{HitRow, LINE_HEIGHT, ShapeKey, SourceEditor, syntax};

const GUTTER_PADDING: f32 = 16.0;

pub struct EditorElement {
    editor: gpui::Entity<SourceEditor>,
}

impl EditorElement {
    pub fn new(editor: gpui::Entity<SourceEditor>) -> Self {
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
}

struct PaintRow {
    hit: HitRow,
    gutter_layout: ShapedLine,
    shape_key: ShapeKey,
    visual_rows: usize,
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
        let digits = snapshot.len_lines().max(1).ilog10() + 1;
        let gutter_width = digits as f32 * 9.0 + GUTTER_PADDING * 2.0;
        let wrap_width = (f32::from(bounds.size.width) - gutter_width).max(1.0);
        self.editor.update(cx, |editor, _| {
            editor
                .display_map
                .configure(snapshot.len_lines(), wrap_width);
        });
        let editor = self.editor.read(cx);
        let selection = editor.selection;
        let marked = editor.marked.as_ref().map(|range| range.bytes);
        let scroll_y = editor.scroll_y;
        let visible_lines = editor.display_map.visible_line_range(
            &snapshot,
            scroll_y,
            f32::from(bounds.size.height),
        );
        let first_line = visible_lines.start;
        let first_visual_row = editor.display_map.line_start_visual_row(first_line);
        let line_offset = scroll_y - first_visual_row as f32 * LINE_HEIGHT;
        let last_line = visible_lines.end;
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
        let mut rows = Vec::with_capacity((last_line - first_line) as usize);
        let mut selection_quads = Vec::new();
        let mut caret = None;
        let mut next_visual_row = first_visual_row;

        for line_number in first_line..last_line {
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
            let text: gpui::SharedString = display.text.clone().into();
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
                marked_display.clone(),
                theme,
            );
            let effective_wrap_width = editor.display_map.soft_wrap().then_some(px(wrap_width));
            let shape_key = shape_key(
                &text,
                font_size,
                marked_display,
                effective_wrap_width,
                syntax::cache_key(editor.session.read(cx).path()),
            );
            let layout = editor
                .shape_cache
                .get(&shape_key)
                .cloned()
                .unwrap_or_else(|| {
                    window
                        .text_system()
                        .shape_text(text, font_size, &runs, effective_wrap_width, None)
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
            let origin_y = bounds.top()
                + px((next_visual_row - first_visual_row) as f32 * LINE_HEIGHT - line_offset);
            let hit = HitRow {
                range: source_line.visible_range,
                line,
                origin_y,
                text_origin_x,
                display,
                layout,
            };

            let selected = selection.range();
            let selected_start = selected.start.0.max(full_range.start.0);
            let selected_end = selected.end.0.min(full_range.end.0);
            if selected_start < selected_end {
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

            if selection.is_empty()
                && selection.head() >= content_range.start
                && selection.head() <= content_range.end
            {
                let source_local = selection
                    .head()
                    .0
                    .saturating_sub(content_range.start.0)
                    .min(hit.layout.len() as u64) as usize;
                let local = hit.display.source_to_display(source_local);
                let position = hit
                    .layout
                    .position_for_index(local, px(LINE_HEIGHT))
                    .unwrap_or_default();
                caret = Some(fill(
                    Bounds::new(
                        point(text_origin_x + position.x, origin_y + position.y + px(2.0)),
                        size(px(1.5), px(LINE_HEIGHT - 4.0)),
                    ),
                    gpui::rgb(theme.foreground),
                ));
            }
            rows.push(PaintRow {
                hit,
                gutter_layout,
                shape_key,
                visual_rows,
            });
            next_visual_row = next_visual_row.saturating_add(visual_rows as u64);
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
            let text_left = state.gutter.bounds.right() + px(1.0);
            for row in &state.rows {
                let number_x = text_left - px(GUTTER_PADDING) - row.gutter_layout.width();
                let _ = row.gutter_layout.paint(
                    point(number_x, row.hit.origin_y),
                    px(LINE_HEIGHT),
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            let text_bounds =
                Bounds::from_corners(point(text_left, bounds.top()), bounds.bottom_right());
            window.with_content_mask(
                Some(ContentMask {
                    bounds: text_bounds,
                }),
                |window| {
                    for selection in state.selection.drain(..) {
                        window.paint_quad(selection);
                    }
                    for row in &state.rows {
                        let _ = row.hit.layout.paint(
                            point(row.hit.text_origin_x, row.hit.origin_y),
                            px(LINE_HEIGHT),
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
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
            .map(|row| (row.hit.line.0, row.visual_rows))
            .collect::<Vec<_>>();
        let _benchmark = self.editor.update(cx, |editor, cx| {
            let viewport_changed = editor.viewport != Some(bounds);
            editor.viewport = Some(bounds);
            editor.hit_rows = hits;
            if editor.shape_cache.len() + shaped.len() > 512 {
                editor.shape_cache.clear();
            }
            editor.shape_cache.extend(shaped);
            let layout_changed = measured_rows
                .into_iter()
                .fold(false, |changed, (line, rows)| {
                    editor.display_map.update_line_rows(line, rows) || changed
                });
            if viewport_changed || layout_changed {
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

fn shape_key(
    text: &gpui::SharedString,
    font_size: Pixels,
    marked: Option<std::ops::Range<usize>>,
    wrap_width: Option<Pixels>,
    syntax_key: u8,
) -> ShapeKey {
    ShapeKey {
        text: text.clone(),
        font_size_bits: f32::from(font_size).to_bits(),
        wrap_width_bits: wrap_width.map_or(0, |width| f32::from(width).to_bits()),
        syntax_key,
        marked: marked.map(|range| (range.start, range.end)),
    }
}

fn local_marked(
    marked: Option<ByteRange>,
    line: ByteRange,
    display: &super::display_map::DisplayLineText,
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

fn push_selection_quads(
    quads: &mut Vec<PaintQuad>,
    hit: &HitRow,
    start: usize,
    end: usize,
    include_newline: bool,
    wrap_width: Pixels,
) {
    let line_height = px(LINE_HEIGHT);
    let start_position = hit
        .layout
        .position_for_index(start, line_height)
        .unwrap_or_default();
    let end_position = hit
        .layout
        .position_for_index(end, line_height)
        .unwrap_or(start_position);
    let first_row = (f32::from(start_position.y) / LINE_HEIGHT).round() as usize;
    let last_row = (f32::from(end_position.y) / LINE_HEIGHT).round() as usize;
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
                        hit.origin_y + px(row as f32 * LINE_HEIGHT),
                    ),
                    point(
                        hit.text_origin_x + right,
                        hit.origin_y + px((row + 1) as f32 * LINE_HEIGHT),
                    ),
                ),
                rgba(0x3a81c34a),
            ));
        }
    }
}
