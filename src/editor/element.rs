use std::sync::Arc;
#[cfg(feature = "benchmarks")]
use std::time::Instant;

use gpui::{
    App, Bounds, ContentMask, Element, ElementId, ElementInputHandler, GlobalElementId, LayoutId,
    PaintQuad, Pixels, ShapedLine, Style, TextAlign, TextRun, UnderlineStyle, Window, fill, point,
    px, relative, rgba, size,
};

use crate::{
    document::{ByteRange, LineIndex, TextSnapshot},
    theme::current_theme,
};

#[cfg(feature = "benchmarks")]
use super::FrameBenchmarkAction;
use super::{HitRow, LINE_HEIGHT, ShapeKey, SourceEditor};

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
        let editor = self.editor.read(cx);
        let snapshot = editor.snapshot(cx);
        let selection = editor.selection;
        let marked = editor.marked.as_ref().map(|range| range.bytes);
        let scroll_y = editor.scroll_y;
        let visible_lines = editor.display_map.visible_line_range(
            &snapshot,
            scroll_y,
            f32::from(bounds.size.height),
        );
        let first_line = visible_lines.start;
        let line_offset = scroll_y - first_line as f32 * LINE_HEIGHT;
        let last_line = visible_lines.end;
        let digits = snapshot.len_lines().max(1).ilog10() + 1;
        let gutter_width = digits as f32 * 9.0 + GUTTER_PADDING * 2.0;
        let text_origin_x = bounds.left() + px(gutter_width);
        let theme = current_theme();
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let mut rows = Vec::with_capacity((last_line - first_line) as usize);
        let mut selection_quads = Vec::new();
        let mut caret = None;

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
            let text: gpui::SharedString = source_line.text.into();
            let base_run = TextRun {
                len: text.len(),
                font: style.font(),
                color: gpui::rgb(theme.foreground).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let runs = marked_runs(base_run, marked, content_range, text.len());
            let shape_key = shape_key(&text, font_size, marked, content_range);
            let layout = editor
                .shape_cache
                .get(&shape_key)
                .cloned()
                .unwrap_or_else(|| {
                    window
                        .text_system()
                        .shape_line(text, font_size, &runs, None)
                });
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
            let origin_y =
                bounds.top() + px((line_number - first_line) as f32 * LINE_HEIGHT - line_offset);
            let hit = HitRow {
                range: source_line.visible_range,
                origin_y,
                text_origin_x,
                layout,
            };

            let selected = selection.range();
            let selected_start = selected.start.0.max(full_range.start.0);
            let selected_end = selected.end.0.min(full_range.end.0);
            if selected_start < selected_end {
                let local_start = selected_start
                    .saturating_sub(content_range.start.0)
                    .min(hit.layout.len() as u64) as usize;
                let local_end = selected_end
                    .saturating_sub(content_range.start.0)
                    .min(hit.layout.len() as u64) as usize;
                let end_x = if selected_end > source_content_range.end.0 {
                    hit.layout.x_for_index(local_end) + px(8.0)
                } else {
                    hit.layout.x_for_index(local_end)
                };
                selection_quads.push(fill(
                    Bounds::from_corners(
                        point(
                            text_origin_x + hit.layout.x_for_index(local_start),
                            origin_y,
                        ),
                        point(text_origin_x + end_x, origin_y + px(LINE_HEIGHT)),
                    ),
                    rgba(0x3a81c34a),
                ));
            }

            if selection.is_empty()
                && selection.head() >= content_range.start
                && selection.head() <= content_range.end
            {
                let local = selection
                    .head()
                    .0
                    .saturating_sub(content_range.start.0)
                    .min(hit.layout.len() as u64) as usize;
                caret = Some(fill(
                    Bounds::new(
                        point(
                            text_origin_x + hit.layout.x_for_index(local),
                            origin_y + px(2.0),
                        ),
                        size(px(1.5), px(LINE_HEIGHT - 4.0)),
                    ),
                    gpui::rgb(theme.foreground),
                ));
            }
            rows.push(PaintRow {
                hit,
                gutter_layout,
                shape_key,
            });
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
            for selection in state.selection.drain(..) {
                window.paint_quad(selection);
            }
            for row in &state.rows {
                let number_x =
                    row.hit.text_origin_x - px(GUTTER_PADDING) - row.gutter_layout.width();
                let _ = row.gutter_layout.paint(
                    point(number_x, row.hit.origin_y),
                    px(LINE_HEIGHT),
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
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
        });
        #[cfg(feature = "benchmarks")]
        let elapsed = state.started_at.elapsed();
        let benchmark = self.editor.update(cx, |editor, cx| {
            let viewport_changed = editor.viewport != Some(bounds);
            editor.viewport = Some(bounds);
            editor.hit_rows = hits;
            if editor.shape_cache.len() + shaped.len() > 512 {
                editor.shape_cache.clear();
            }
            editor.shape_cache.extend(shaped);
            if viewport_changed {
                cx.notify();
            }
            #[cfg(feature = "benchmarks")]
            return editor.record_frame_benchmark(elapsed, cx);
        });
        #[cfg(not(feature = "benchmarks"))]
        let _ = benchmark;
        #[cfg(feature = "benchmarks")]
        match benchmark {
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
    marked: Option<ByteRange>,
    line: ByteRange,
) -> ShapeKey {
    let marked = marked.and_then(|marked| {
        let start = marked.start.0.max(line.start.0);
        let end = marked.end.0.min(line.end.0);
        (start < end).then_some((
            (start - line.start.0) as usize,
            (end - line.start.0) as usize,
        ))
    });
    ShapeKey {
        text: text.clone(),
        font_size_bits: f32::from(font_size).to_bits(),
        marked,
    }
}

fn marked_runs(
    base: TextRun,
    marked: Option<ByteRange>,
    line: ByteRange,
    visible_len: usize,
) -> Vec<TextRun> {
    let Some(marked) = marked else {
        return vec![base];
    };
    let start = marked.start.0.max(line.start.0);
    let end = marked.end.0.min(line.end.0);
    if start >= end {
        return vec![base];
    }
    let local_start = (start - line.start.0).min(visible_len as u64) as usize;
    let local_end = (end - line.start.0).min(visible_len as u64) as usize;
    [
        TextRun {
            len: local_start,
            ..base.clone()
        },
        TextRun {
            len: local_end.saturating_sub(local_start),
            underline: Some(UnderlineStyle {
                color: Some(base.color),
                thickness: px(1.0),
                wavy: false,
            }),
            ..base.clone()
        },
        TextRun {
            len: visible_len.saturating_sub(local_end),
            ..base
        },
    ]
    .into_iter()
    .filter(|run| run.len > 0)
    .collect()
}
