//! Minimap text styles, visibility and glyph rasterization.

use crate::document::DocumentFormat;
use gpui::{FontWeight, RenderImage};
use image::{Frame, RgbaImage};
use smallvec::SmallVec;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthStr;

pub(in crate::editor) const MAX_RICH_SPANS_PER_ROW: usize = 64;
pub(in crate::editor) const MAX_RICH_SPANS_PER_REQUEST: usize = 4_096;
static RICH_SPAN_DEGRADED_ROWS: AtomicU64 = AtomicU64::new(0);

pub(in crate::editor) struct TextRow {
    pub(in crate::editor) reveal_ranges: Vec<std::ops::Range<usize>>,
    pub(in crate::editor) display_options: crate::org_semantic::display::OrgDisplayOptions,
    pub(in crate::editor) text: String,
    pub(in crate::editor) font_role: crate::typography::FontRole,
    pub(in crate::editor) color: u32,
    pub(in crate::editor) weight: FontWeight,
    pub(in crate::editor) italic: bool,
    pub(in crate::editor) spans: Vec<TextSpan>,
    pub(in crate::editor) indent: f32,
    pub(in crate::editor) block_background: Option<u32>,
    pub(in crate::editor) block_accent: Option<u32>,
    pub(in crate::editor) block_edge: Option<crate::editor::syntax::EditorBlockEdge>,
    pub(in crate::editor) table: Option<TableRowGeometry>,
}

#[derive(Clone, Debug)]
pub(in crate::editor) struct TableRowGeometry {
    cell_widths: Arc<[usize]>,
    pub(in crate::editor) format: DocumentFormat,
}

impl TableRowGeometry {
    pub(in crate::editor) fn from_source(text: &str, format: DocumentFormat) -> Option<Self> {
        let parsed = crate::document::table::parse_line(text, format);
        (crate::document::table::delimiter_offsets(text, format).len() >= 2).then(|| Self {
            cell_widths: parsed
                .cells
                .iter()
                .map(|cell| UnicodeWidthStr::width(&text[cell.raw_range.clone()]))
                .collect(),
            format,
        })
    }
}

pub(in crate::editor) struct RasterizedRows {
    pub(in crate::editor) image: Arc<RenderImage>,
    pub(in crate::editor) rasterizer_lock_wait: Duration,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::editor) struct TextSpan {
    pub(in crate::editor) bytes: std::ops::Range<usize>,
    pub(in crate::editor) color: Option<cosmic_text::Color>,
    pub(in crate::editor) weight: Option<FontWeight>,
    pub(in crate::editor) italic: bool,
    pub(in crate::editor) underline: bool,
    pub(in crate::editor) strikethrough: bool,
}

#[derive(Debug)]
pub(in crate::editor) struct RichSpanBudget {
    remaining: usize,
    degraded_rows: u64,
}

impl Default for RichSpanBudget {
    fn default() -> Self {
        Self {
            remaining: MAX_RICH_SPANS_PER_REQUEST,
            degraded_rows: 0,
        }
    }
}

impl RichSpanBudget {
    pub(in crate::editor) fn adapt(
        &mut self,
        text: &str,
        mut spans: Vec<crate::editor::syntax::EditorSemanticSpan>,
        theme: &crate::theme::Theme,
    ) -> Vec<TextSpan> {
        // Swatches are editor-only geometry with no minimap representation, so
        // they must not consume the rich-span budget.
        spans.retain(|span| span.swatch.is_none());
        let valid_span = |span: &crate::editor::syntax::EditorSemanticSpan| {
            span.bytes.start < span.bytes.end
                && span.bytes.end <= text.len()
                && text.is_char_boundary(span.bytes.start)
                && text.is_char_boundary(span.bytes.end)
        };
        if !spans.iter().all(valid_span)
            || spans.len() > MAX_RICH_SPANS_PER_ROW
            || spans.len() > self.remaining
        {
            self.degraded_rows += 1;
            RICH_SPAN_DEGRADED_ROWS.fetch_add(1, Ordering::Relaxed);
            // Visibility is part of the content, so style degradation must retain it.
            spans.retain(|span| {
                span.color == Some(crate::editor::syntax::EditorColorToken::Hidden)
                    && valid_span(span)
            });
        }
        self.remaining = self.remaining.saturating_sub(spans.len());
        spans
            .into_iter()
            .map(|span| TextSpan {
                bytes: span.bytes,
                color: span.color.map(|token| {
                    if token == crate::editor::syntax::EditorColorToken::Hidden {
                        cosmic_text::Color::rgba(0, 0, 0, 0)
                    } else {
                        let color = token.resolve(theme);
                        cosmic_text::Color::rgb(
                            (color >> 16) as u8,
                            (color >> 8) as u8,
                            color as u8,
                        )
                    }
                }),
                weight: span.weight,
                italic: span.italic,
                underline: span.underline,
                strikethrough: span.strikethrough,
            })
            .collect()
    }

    pub(in crate::editor) fn degraded_rows(&self) -> u64 {
        self.degraded_rows
    }
}

pub(in crate::editor) fn rich_span_degraded_rows() -> u64 {
    RICH_SPAN_DEGRADED_ROWS.load(Ordering::Relaxed)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::editor) fn rasterize_text_rows(
    rows: &[TextRow],
    logical_width: usize,
    scale_factor: f32,
    density: crate::minimap::Density,
    line_height: f32,
    epoch: &AtomicU64,
    expected_epoch: u64,
) -> Option<RasterizedRows> {
    use cosmic_text::{Attrs, Buffer, Color, Metrics, Shaping, Wrap};

    let scale_factor = scale_factor.max(1.0);
    let physical_line_height = line_height * scale_factor;
    let logical_width = logical_width.max(1);
    let width = (logical_width as f32 * scale_factor).ceil().max(1.0) as u32;
    let height = (rows.len().max(1) as f32 * physical_line_height)
        .ceil()
        .max(1.0) as u32;
    let mut pixels = vec![0_u8; width as usize * height as usize * 4];
    let lock_started = Instant::now();
    let lock_wait_span = tracing::info_span!("minimap_rasterizer_lock_wait").entered();
    let mut rasterizer = crate::minimap::text_rasterizer()
        .lock()
        .expect("minimap rasterizer poisoned");
    let rasterizer_lock_wait = lock_started.elapsed();
    drop(lock_wait_span);
    if epoch.load(Ordering::Acquire) != expected_epoch {
        return None;
    }
    let (font_system, swash_cache) = &mut *rasterizer;
    let attrs = Attrs::new().family(crate::typography::cosmic_family(
        &crate::typography::fonts().editor_family,
    ));
    let table_attrs = Attrs::new().family(crate::typography::cosmic_family(
        &crate::typography::fonts().table_family,
    ));
    let mut buffer = Buffer::new(
        font_system,
        Metrics::new(density.font_px() * scale_factor, physical_line_height),
    );
    buffer.set_wrap(Wrap::None);
    let space_advance = if rows.iter().any(|row| row.table.is_some()) {
        buffer.set_text(" ", &table_attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);
        buffer
            .layout_runs()
            .next()
            .map_or(density.font_px() * scale_factor * 0.6, |run| run.line_w)
            .max(1.0)
    } else {
        0.0
    };
    for (row, source) in rows.iter().enumerate() {
        if row.is_multiple_of(32) && epoch.load(Ordering::Relaxed) != expected_epoch {
            return None;
        }
        let origin_y = (row as f32 * physical_line_height).round() as i32;
        if let Some(background) = source.block_background {
            crate::minimap::paint_text_pixels(
                &mut pixels,
                width as usize,
                height as usize,
                1,
                origin_y,
                width.saturating_sub(2),
                physical_line_height.ceil() as u32,
                Color::rgba(
                    (background >> 16) as u8,
                    (background >> 8) as u8,
                    background as u8,
                    0x24,
                ),
            );
        }
        if let Some(accent) = source.block_accent {
            let accent = Color::rgba(
                (accent >> 16) as u8,
                (accent >> 8) as u8,
                accent as u8,
                0xb8,
            );
            crate::minimap::paint_text_pixels(
                &mut pixels,
                width as usize,
                height as usize,
                1,
                origin_y,
                (1.0 * scale_factor).ceil() as u32,
                physical_line_height.ceil() as u32,
                accent,
            );
            if matches!(
                source.block_edge,
                Some(
                    crate::editor::syntax::EditorBlockEdge::Open
                        | crate::editor::syntax::EditorBlockEdge::Close
                )
            ) {
                crate::minimap::paint_text_pixels(
                    &mut pixels,
                    width as usize,
                    height as usize,
                    1,
                    if source.block_edge == Some(crate::editor::syntax::EditorBlockEdge::Close) {
                        origin_y + physical_line_height.ceil() as i32 - 1
                    } else {
                        origin_y
                    },
                    width.saturating_sub(2),
                    1,
                    accent,
                );
            }
        }
        let attrs = if matches!(source.font_role, crate::typography::FontRole::Table) {
            &table_attrs
        } else {
            &attrs
        };
        let base_attrs =
            attrs
                .clone()
                .weight(cosmic_weight(source.weight))
                .style(if source.italic {
                    cosmic_text::Style::Italic
                } else {
                    cosmic_text::Style::Normal
                });
        let base = Color::rgb(
            (source.color >> 16) as u8,
            (source.color >> 8) as u8,
            source.color as u8,
        );
        let origin_x = (source.indent * scale_factor).round() as i32;
        if let Some(table) = &source.table {
            for (range, x_units) in table_fragments(&source.text, table) {
                let mut spans = slice_text_spans(&source.spans, &range);
                let reveal = source
                    .reveal_ranges
                    .iter()
                    .filter_map(|reveal| {
                        let start = reveal.start.max(range.start);
                        let end = reveal.end.min(range.end);
                        (start < end).then(|| start - range.start..end - range.start)
                    })
                    .collect::<Vec<_>>();
                let text = &source.text[range];
                let entities = crate::editor::table_layout::cell_projection(
                    text,
                    table.format,
                    None,
                    source.display_options,
                    &reveal,
                );
                if let Some(entities) = &entities {
                    for span in &mut spans {
                        span.bytes = entities.source_to_display(span.bytes.start)
                            ..entities.source_to_display(span.bytes.end);
                    }
                    spans.retain(|span| !span.bytes.is_empty());
                }
                let text = entities
                    .as_ref()
                    .map_or(text, |entities| entities.text.as_str());
                draw_minimap_text(
                    &mut buffer,
                    font_system,
                    swash_cache,
                    &mut pixels,
                    width,
                    height,
                    physical_line_height,
                    text,
                    &spans,
                    &base_attrs,
                    source.font_role.cjk_family(),
                    base,
                    origin_x + (x_units as f32 * space_advance).round() as i32,
                    origin_y,
                );
            }
        } else {
            draw_minimap_text(
                &mut buffer,
                font_system,
                swash_cache,
                &mut pixels,
                width,
                height,
                physical_line_height,
                &source.text,
                &source.spans,
                &base_attrs,
                source.font_role.cjk_family(),
                base,
                origin_x,
                origin_y,
            );
        }
    }
    let image = RgbaImage::from_raw(width, height, pixels).expect("valid minimap image dimensions");
    Some(RasterizedRows {
        image: Arc::new(RenderImage::new(SmallVec::from_elem(Frame::new(image), 1))),
        rasterizer_lock_wait,
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_minimap_text(
    buffer: &mut cosmic_text::Buffer,
    font_system: &mut cosmic_text::FontSystem,
    swash_cache: &mut cosmic_text::SwashCache,
    pixels: &mut [u8],
    width: u32,
    height: u32,
    line_height: f32,
    text: &str,
    spans: &[TextSpan],
    attrs: &cosmic_text::Attrs<'static>,
    chinese_family: Option<&'static str>,
    color: cosmic_text::Color,
    origin_x: i32,
    origin_y: i32,
) {
    use cosmic_text::Shaping;

    buffer.set_size(Some(width as f32), Some(line_height));
    if spans.is_empty() && chinese_family.is_none() {
        buffer.set_text(text, attrs, Shaping::Advanced, None);
    } else {
        buffer.set_rich_text(
            rich_text_segments(text, spans, attrs, chinese_family),
            attrs,
            Shaping::Advanced,
            None,
        );
    }
    buffer.shape_until_scroll(font_system, false);
    buffer.draw(font_system, swash_cache, color, |x, y, w, h, color| {
        crate::minimap::paint_text_pixels(
            pixels,
            width as usize,
            height as usize,
            x + origin_x,
            y + origin_y,
            w,
            h,
            color,
        );
    });
}

fn table_fragments(text: &str, table: &TableRowGeometry) -> Vec<(std::ops::Range<usize>, usize)> {
    let delimiters = crate::document::table::delimiter_offsets(text, table.format);
    let Some(&first) = delimiters.first() else {
        return vec![(0..text.len(), 0)];
    };
    let mut fragments = Vec::with_capacity(delimiters.len() + usize::from(first > 0));
    if first > 0 {
        fragments.push((0..first, 0));
    }
    let mut delimiter_x = UnicodeWidthStr::width(&text[..first]);
    for (column, &start) in delimiters.iter().enumerate() {
        let end = delimiters.get(column + 1).copied().unwrap_or(text.len());
        fragments.push((start..end, delimiter_x));
        if column + 1 < delimiters.len() {
            delimiter_x += table.cell_widths.get(column).copied().unwrap_or(0) + 1;
        }
    }
    fragments
}

fn slice_text_spans(spans: &[TextSpan], range: &std::ops::Range<usize>) -> Vec<TextSpan> {
    spans
        .iter()
        .filter_map(|span| {
            let start = span.bytes.start.max(range.start);
            let end = span.bytes.end.min(range.end);
            (start < end).then(|| TextSpan {
                bytes: start - range.start..end - range.start,
                color: span.color,
                weight: span.weight,
                italic: span.italic,
                underline: span.underline,
                strikethrough: span.strikethrough,
            })
        })
        .collect()
}

fn cosmic_weight(weight: FontWeight) -> cosmic_text::Weight {
    cosmic_text::Weight(weight.0 as u16)
}

fn rich_text_segments<'a>(
    text: &'a str,
    spans: &[TextSpan],
    base: &cosmic_text::Attrs<'static>,
    chinese_family: Option<&'static str>,
) -> Vec<(&'a str, cosmic_text::Attrs<'static>)> {
    use cosmic_text::{Style, UnderlineStyle};

    let chinese = if chinese_family.is_some() {
        crate::typography::cjk_ranges(text)
    } else {
        Vec::new()
    };
    let mut boundaries = vec![0, text.len()];
    for span in spans {
        boundaries.extend([span.bytes.start, span.bytes.end]);
    }
    for range in &chinese {
        boundaries.extend([range.start, range.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
        .windows(2)
        .filter_map(|boundary| {
            let range = boundary[0]..boundary[1];
            if range.is_empty() {
                return None;
            }
            let mut attrs = base.clone();
            if let Some(family) = chinese_family
                && chinese.iter().any(|r| r.contains(&range.start))
            {
                attrs = attrs.family(crate::typography::cosmic_family(family));
            }
            for span in spans
                .iter()
                .filter(|span| span.bytes.start <= range.start && range.start < span.bytes.end)
            {
                if let Some(color) = span.color {
                    attrs = attrs.color(color);
                }
                if let Some(weight) = span.weight {
                    attrs = attrs.weight(cosmic_weight(weight));
                }
                if span.italic {
                    attrs = attrs.style(Style::Italic);
                }
                if span.underline {
                    attrs = attrs.underline(UnderlineStyle::Single);
                }
                if span.strikethrough {
                    attrs = attrs.strikethrough();
                }
            }
            Some((&text[range], attrs))
        })
        .collect()
}

#[cfg(test)]
mod tests;
