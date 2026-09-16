#[cfg(feature = "benchmarks")]
use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

#[cfg(feature = "benchmarks")]
use gpui::px;
use gpui::{Bounds, Font, Pixels, RenderImage};
use image::{Frame, RgbaImage};
use smallvec::SmallVec;
use unicode_width::UnicodeWidthStr;

use crate::document::{ByteRange, DocumentFormat, Revision, TextSnapshot};

pub(super) const DEFAULT_WIDTH: f32 = 96.0;
pub(super) const MIN_WIDTH: f32 = 56.0;
pub(super) const MAX_WIDTH: f32 = 220.0;
pub(super) const RESIZE_HANDLE: f32 = 6.0;
const RASTER_PREFETCH_ROWS: usize = 64;
// Row heights and soft-wrap boundaries are discovered while the Editor paints. Publishing every
// discovery immediately makes the minimap pixels underneath a moving viewport appear to redraw.
// Keep live Editor geometry authoritative, but coalesce those measurement-only raster updates
// until input and measurement have both been quiet for a few display frames.
const LAYOUT_RASTER_SETTLE: Duration = Duration::from_millis(50);
// Keep one complete scheduling quantum beyond the point where the next raster key is selected.
// Without this extra lookahead, the previous raster ends exactly at the visible track boundary
// when the key advances, so even a one-frame async delay exposes an empty strip at the bottom.
const RASTER_LOOKAHEAD_ROWS: usize = RASTER_PREFETCH_ROWS * 2;
const MAX_RASTER_ROWS: usize = 700;
pub(super) const MAX_RICH_SPANS_PER_ROW: usize = 64;
pub(super) const MAX_RICH_SPANS_PER_REQUEST: usize = 4_096;
static RICH_SPAN_DEGRADED_ROWS: AtomicU64 = AtomicU64::new(0);
static NEXT_HOST_ID: AtomicU64 = AtomicU64::new(1);
static PERF_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
#[cfg(feature = "benchmarks")]
static TRACE_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

fn perf_enabled() -> bool {
    *PERF_ENABLED.get_or_init(|| std::env::var_os("ORG_STUDIO_EDITOR_MINIMAP_PERF").is_some())
}

pub(crate) fn prewarm_text_rasterizer() {
    use cosmic_text::{Attrs, Buffer, Color, Family, Metrics, Shaping, Style, Weight, Wrap};
    static PREWARMED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    PREWARMED.get_or_init(|| {
        let started = Instant::now();
        let mut rasterizer = crate::minimap::text_rasterizer()
            .lock()
            .expect("minimap rasterizer poisoned");
        let (font_system, swash_cache) = &mut *rasterizer;
        let mut buffer = Buffer::new(font_system, Metrics::new(6.0, 8.0));
        buffer.set_size(Some(1600.0), Some(8.0));
        buffer.set_wrap(Wrap::None);
        // Preload the Editor font family and all rendered weight/style combinations.
        for (weight, style) in [
            (Weight::NORMAL, Style::Normal),
            (Weight::BOLD, Style::Normal),
            (Weight::SEMIBOLD, Style::Normal),
            (Weight::NORMAL, Style::Italic),
            (Weight::BOLD, Style::Italic),
            (Weight::SEMIBOLD, Style::Italic),
        ] {
            let attrs = Attrs::new()
                .family(Family::Name(super::EDITOR_FONT_FAMILY))
                .weight(weight)
                .style(style);
            buffer.set_text(
                "Org Markdown 中文 α 😀 e\u{301} AaZz 0123456789 +-*/_`#[](){} :=> ",
                &attrs,
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(font_system, false);
            buffer.draw(
                font_system,
                swash_cache,
                Color::rgb(0, 0, 0),
                |_, _, _, _, _| {},
            );
        }
        if perf_enabled() {
            eprintln!(
                "org_editor_minimap_text_prewarm elapsed_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    });
}

#[cfg(feature = "benchmarks")]
fn trace_enabled() -> bool {
    *TRACE_ENABLED.get_or_init(|| std::env::var_os("ORG_STUDIO_EDITOR_MINIMAP_TRACE").is_some())
}

pub(super) type ViewportGeometry = crate::minimap::ProjectionViewport;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SourceViewport {
    pub(super) total_units: f32,
    pub(super) visible_top: f32,
    pub(super) visible_bottom: f32,
    pub(super) scroll_ratio: f32,
}

/// Projects the live Editor scroll onto one immutable minimap layout snapshot.
///
/// Editor layout measurements may continue changing while a prepared frame is visible. Mapping
/// through the live scroll ratio keeps the prepared frame internally stable without introducing
/// a second scroll camera, and pins both document endpoints exactly.
pub(super) fn source_viewport_for_layout(
    layout: &super::layout_map::EditorLayoutMap,
    live_layout: &super::layout_map::EditorLayoutMap,
    live_scroll_y: f32,
    live_document_height: f32,
    viewport_height: f32,
) -> SourceViewport {
    let base_line_height = layout.base_line_height().max(1.0);
    let minimap_document_height = layout.total_height();
    let total_units = minimap_document_height / base_line_height;
    if total_units <= 0.0 {
        return SourceViewport {
            total_units: 0.0,
            visible_top: 0.0,
            visible_bottom: 0.0,
            scroll_ratio: 0.0,
        };
    }

    let live_max_scroll = (live_document_height - viewport_height).max(0.0);
    let live_scroll_y = live_scroll_y.clamp(0.0, live_max_scroll);
    let live_at_start = live_scroll_y <= 0.5;
    let live_at_end = live_scroll_y + viewport_height + 0.5 >= live_document_height;
    let minimap_max_scroll = (minimap_document_height - viewport_height).max(0.0);
    let minimap_scroll_y = if live_at_start || live_max_scroll <= 0.0 {
        0.0
    } else if live_at_end {
        minimap_max_scroll
    } else {
        // The Editor preserves a source-line anchor when newly measured wraps change live
        // document height. Preserve that same anchor in the prepared minimap frame. A ratio of
        // live document totals can move backwards when the denominator grows mid-scroll.
        let line = live_layout.line_at_y(live_scroll_y);
        let live_line_start = live_layout.line_start_y(line);
        let line_fraction = ((live_scroll_y - live_line_start)
            / live_layout.line_height_px(line).max(1.0))
        .clamp(0.0, 1.0);
        (layout.line_start_y(line) + line_fraction * layout.line_height_px(line))
            .clamp(0.0, minimap_max_scroll)
    };
    let scroll_ratio = if minimap_max_scroll > 0.0 {
        (minimap_scroll_y / minimap_max_scroll).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let visible_top = minimap_scroll_y / base_line_height;
    let visible_bottom = if live_at_end {
        total_units
    } else {
        ((minimap_scroll_y + viewport_height).min(minimap_document_height) / base_line_height)
            .clamp(visible_top, total_units)
    };
    SourceViewport {
        total_units,
        visible_top,
        visible_bottom,
        scroll_ratio,
    }
}

#[cfg(test)]
pub(super) fn viewport_geometry(
    total_units: f32,
    scroll_top: f32,
    viewport_units: f32,
    track_height: f32,
    density: crate::minimap::Density,
) -> ViewportGeometry {
    viewport_geometry_for_range(
        total_units,
        scroll_top,
        (scroll_top + viewport_units).min(total_units),
        track_height,
        density,
    )
}

#[cfg(test)]
pub(super) fn viewport_geometry_for_range(
    total_units: f32,
    visible_top: f32,
    visible_bottom: f32,
    track_height: f32,
    density: crate::minimap::Density,
) -> ViewportGeometry {
    let line_height = raster_line_height(density, 1.0);
    let viewport_units = (visible_bottom - visible_top).max(0.0);
    let max_scroll = (total_units - viewport_units).max(0.0);
    let scroll_top = visible_top.clamp(0.0, max_scroll);
    let scroll_ratio = if max_scroll > 0.0 {
        scroll_top / max_scroll
    } else {
        0.0
    };
    crate::minimap::projection_viewport_with_line_height(
        total_units,
        scroll_top,
        visible_bottom.clamp(scroll_top, total_units),
        scroll_ratio,
        track_height,
        density,
        line_height,
    )
}

pub(super) fn raster_line_height(density: crate::minimap::Density, scale_factor: f32) -> f32 {
    let scale_factor = scale_factor.max(1.0);
    (density.line_height() * scale_factor).round().max(1.0) / scale_factor
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RasterSourceRow {
    pub(super) line: u64,
    pub(super) text_range: Option<(u32, Option<u32>)>,
}

pub(super) fn fill_visual_rows(
    raster_lines: &mut [Option<RasterSourceRow>],
    line: u64,
    row_offset_units: f32,
    row_height_units: f32,
    wrap_starts: &[u32],
) {
    if row_height_units <= 0.0 || raster_lines.is_empty() {
        return;
    }
    let source_text_row = row_offset_units.floor() as isize;
    let first = source_text_row;
    let end = (row_offset_units + row_height_units).ceil() as isize;
    let first = first.clamp(0, raster_lines.len() as isize) as usize;
    let end = end.clamp(first as isize, raster_lines.len() as isize) as usize;
    // Soft wrap, block spacing and inline media can make one source line occupy several visual
    // rows. Populate its complete span so asynchronous raster publication cannot expose holes.
    for (row, slot) in raster_lines[first..end].iter_mut().enumerate() {
        let raster_row = first + row;
        let visual_row = raster_row as isize - source_text_row;
        let text_range = usize::try_from(visual_row).ok().and_then(|visual_row| {
            if visual_row > wrap_starts.len() {
                return None;
            }
            let start = visual_row
                .checked_sub(1)
                .map_or(0, |previous| wrap_starts[previous]);
            Some((start, wrap_starts.get(visual_row).copied()))
        });
        *slot = Some(RasterSourceRow {
            line,
            // Keep the complete occupied span for backgrounds and media. Text rows use the
            // Editor's measured wrap boundaries; block spacing beyond them stays text-free.
            text_range,
        });
    }
}

pub(super) struct TextRow {
    pub(super) text: String,
    pub(super) color: u32,
    pub(super) weight: TextWeight,
    pub(super) italic: bool,
    pub(super) spans: Vec<TextSpan>,
    pub(super) indent: f32,
    pub(super) block_background: Option<u32>,
    pub(super) block_accent: Option<u32>,
    pub(super) block_edge: Option<super::syntax::EditorBlockEdge>,
    pub(super) table: Option<TableRowGeometry>,
}

#[derive(Clone, Debug)]
pub(super) struct TableRowGeometry {
    cell_widths: Arc<[usize]>,
    pub(super) format: DocumentFormat,
}

impl TableRowGeometry {
    pub(super) fn from_source(text: &str, format: DocumentFormat) -> Option<Self> {
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

pub(super) struct RasterizedRows {
    pub(super) image: Arc<RenderImage>,
    pub(super) rasterizer_lock_wait: Duration,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum TextWeight {
    #[default]
    Normal,
    Semibold,
    Bold,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TextSpan {
    pub(super) bytes: std::ops::Range<usize>,
    pub(super) color: Option<u32>,
    pub(super) weight: TextWeight,
    pub(super) italic: bool,
    pub(super) underline: bool,
    pub(super) strikethrough: bool,
}

#[derive(Debug)]
pub(super) struct RichSpanBudget {
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
    pub(super) fn adapt(
        &mut self,
        text: &str,
        spans: Vec<super::syntax::EditorSemanticSpan>,
        theme: &crate::theme::Theme,
    ) -> Vec<TextSpan> {
        let valid = spans.iter().all(|span| {
            span.bytes.start < span.bytes.end
                && span.bytes.end <= text.len()
                && text.is_char_boundary(span.bytes.start)
                && text.is_char_boundary(span.bytes.end)
        });
        if !valid || spans.len() > MAX_RICH_SPANS_PER_ROW || spans.len() > self.remaining {
            self.degraded_rows += 1;
            RICH_SPAN_DEGRADED_ROWS.fetch_add(1, Ordering::Relaxed);
            return Vec::new();
        }
        self.remaining -= spans.len();
        spans
            .into_iter()
            .map(|span| TextSpan {
                bytes: span.bytes,
                color: span.color.map(|token| token.resolve(theme)),
                weight: match span.weight {
                    super::syntax::EditorSemanticWeight::Normal => TextWeight::Normal,
                    super::syntax::EditorSemanticWeight::Semibold => TextWeight::Semibold,
                    super::syntax::EditorSemanticWeight::Bold => TextWeight::Bold,
                },
                italic: span.italic,
                underline: span.underline,
                strikethrough: span.strikethrough,
            })
            .collect()
    }

    pub(super) fn degraded_rows(&self) -> u64 {
        self.degraded_rows
    }
}

pub(super) fn rich_span_degraded_rows() -> u64 {
    RICH_SPAN_DEGRADED_ROWS.load(Ordering::Relaxed)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn rasterize_text_rows(
    rows: &[TextRow],
    logical_width: usize,
    scale_factor: f32,
    density: crate::minimap::Density,
    line_height: f32,
    epoch: &AtomicU64,
    expected_epoch: u64,
) -> Option<RasterizedRows> {
    use cosmic_text::{Attrs, Buffer, Color, Family, Metrics, Shaping, Wrap};

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
    let attrs = Attrs::new().family(Family::Name(super::EDITOR_FONT_FAMILY));
    let mut buffer = Buffer::new(
        font_system,
        Metrics::new(density.font_px() * scale_factor, physical_line_height),
    );
    buffer.set_wrap(Wrap::None);
    buffer.set_text(" ", &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    let space_advance = buffer
        .layout_runs()
        .next()
        .map_or(density.font_px() * scale_factor * 0.6, |run| run.line_w)
        .max(1.0);
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
                Some(super::syntax::EditorBlockEdge::Open | super::syntax::EditorBlockEdge::Close)
            ) {
                crate::minimap::paint_text_pixels(
                    &mut pixels,
                    width as usize,
                    height as usize,
                    1,
                    if source.block_edge == Some(super::syntax::EditorBlockEdge::Close) {
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
                let spans = slice_text_spans(&source.spans, &range);
                draw_minimap_text(
                    &mut buffer,
                    font_system,
                    swash_cache,
                    &mut pixels,
                    width,
                    height,
                    physical_line_height,
                    &source.text[range],
                    &spans,
                    &base_attrs,
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
    color: cosmic_text::Color,
    origin_x: i32,
    origin_y: i32,
) {
    use cosmic_text::Shaping;

    buffer.set_size(Some(width as f32), Some(line_height));
    if spans.is_empty() {
        buffer.set_text(text, attrs, Shaping::Advanced, None);
    } else {
        buffer.set_rich_text(
            rich_text_segments(text, spans, attrs),
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

fn cosmic_weight(weight: TextWeight) -> cosmic_text::Weight {
    match weight {
        TextWeight::Normal => cosmic_text::Weight::NORMAL,
        TextWeight::Semibold => cosmic_text::Weight::SEMIBOLD,
        TextWeight::Bold => cosmic_text::Weight::BOLD,
    }
}

fn rich_text_segments<'a>(
    text: &'a str,
    spans: &[TextSpan],
    base: &cosmic_text::Attrs<'static>,
) -> Vec<(&'a str, cosmic_text::Attrs<'static>)> {
    use cosmic_text::{Color, Style, UnderlineStyle};

    let mut boundaries = vec![0, text.len()];
    for span in spans {
        boundaries.extend([span.bytes.start, span.bytes.end]);
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
            for span in spans
                .iter()
                .filter(|span| span.bytes.start <= range.start && range.start < span.bytes.end)
            {
                if let Some(color) = span.color {
                    attrs = attrs.color(Color::rgb(
                        (color >> 16) as u8,
                        (color >> 8) as u8,
                        color as u8,
                    ));
                }
                if span.weight != TextWeight::Normal {
                    attrs = attrs.weight(cosmic_weight(span.weight));
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

pub(super) fn raster_window(
    content_top: f32,
    interaction_height: f32,
    total_units: f32,
    density: crate::minimap::Density,
    line_height: f32,
) -> (u64, usize) {
    let first_visible = content_top.floor().max(0.0) as u64;
    let first = first_visible.saturating_sub(RASTER_PREFETCH_ROWS as u64)
        / RASTER_PREFETCH_ROWS as u64
        * RASTER_PREFETCH_ROWS as u64;
    let visible = ((interaction_height - density.edge_padding() * 2.0) / line_height.max(1.0))
        .ceil()
        .clamp(1.0, (MAX_RASTER_ROWS - RASTER_PREFETCH_ROWS * 2) as f32) as usize
        + 1;
    let requested = (visible + RASTER_PREFETCH_ROWS + RASTER_LOOKAHEAD_ROWS).min(MAX_RASTER_ROWS);
    let available = (total_units.ceil().max(1.0) as u64)
        .saturating_sub(first)
        .min(usize::MAX as u64) as usize;
    (first, requested.min(available))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RasterKey {
    pub(super) generation: u64,
    pub(super) first_unit: u64,
    pub(super) rows: u16,
    pub(super) width: u16,
    pub(super) scale_x100: u16,
    pub(super) density: crate::minimap::Density,
    /// Minimap rasters bake palette colors into the image; the theme
    /// generation keeps them from outliving a theme switch.
    pub(super) theme_generation: u64,
}

#[derive(Clone)]
pub(super) struct CachedRaster {
    pub(super) key: RasterKey,
    pub(super) image: Arc<RenderImage>,
    pub(super) media: Arc<[RasterMedia]>,
    pub(super) content_top: f32,
    pub(super) viewport_generation: u64,
    pub(super) line_height: f32,
}

#[derive(Clone)]
pub(super) struct PreparedEditorMinimapFrame {
    pub(super) geometry_generation: EditorVisualGeometryGeneration,
    pub(super) layout: Arc<super::layout_map::EditorLayoutMap>,
    pub(super) raster: CachedRaster,
    /// A complete frame stays complete while a newer revision's layout is building.
    uses_complete_layout: bool,
}

/// Identity of every input that can affect Editor visual-row geometry.
///
/// Raster generations also change when a visible row is measured. Keeping that transient state
/// out of this key lets one complete background layout remain authoritative while the live Editor
/// catches up with the same measurements during painting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PreparedLayoutKey {
    pub(super) revision: Revision,
    pub(super) path: std::path::PathBuf,
    pub(super) line_count: u64,
    pub(super) wrap_width_bits: u32,
    pub(super) base_line_height_bits: u32,
    pub(super) soft_wrap: bool,
    pub(super) font: Font,
    pub(super) font_size_bits: u32,
    pub(super) content_scale_bits: u32,
    pub(super) fold_revision: u64,
    pub(super) inline_images: bool,
    pub(super) inline_image_overrides: Arc<[(u64, bool)]>,
    pub(super) inline_image_resource_generation: u64,
}

#[derive(Clone)]
struct PreparedLayout {
    key: PreparedLayoutKey,
    layout: Arc<super::layout_map::EditorLayoutMap>,
}

#[derive(Default)]
struct LayoutPreparationState {
    desired: Option<PreparedLayoutKey>,
    in_flight: Option<PreparedLayoutKey>,
    ready: Option<PreparedLayout>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct EditorVisualGeometryGeneration {
    raster_generation: u64,
    layout_identity: usize,
}

impl PreparedEditorMinimapFrame {
    pub(super) fn new(
        layout: Arc<super::layout_map::EditorLayoutMap>,
        raster: CachedRaster,
    ) -> Self {
        Self {
            geometry_generation: EditorVisualGeometryGeneration {
                raster_generation: raster.key.generation,
                layout_identity: Arc::as_ptr(&layout) as usize,
            },
            layout,
            raster,
            uses_complete_layout: false,
        }
    }

    pub(super) fn is_coherent(&self) -> bool {
        self.geometry_generation
            == (EditorVisualGeometryGeneration {
                raster_generation: self.raster.key.generation,
                layout_identity: Arc::as_ptr(&self.layout) as usize,
            })
    }

    #[cfg(feature = "benchmarks")]
    pub(super) fn geometry_identity(&self) -> u64 {
        self.geometry_generation.layout_identity as u64
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RasterMedia {
    pub(super) line: u64,
    pub(super) line_start: u64,
    pub(super) row_offset_units: f32,
    pub(super) row_height_units: f32,
    pub(super) path: std::path::PathBuf,
    pub(super) dimensions: Option<(u32, u32)>,
}

pub(super) fn raster_placement_content_top(
    cached: &CachedRaster,
    viewport_generation: u64,
    current_content_top: f32,
) -> f32 {
    if cached.viewport_generation == viewport_generation {
        cached.content_top
    } else {
        current_content_top
    }
}

#[derive(Clone, Copy, Debug)]
struct ViewportAnchor {
    viewport_generation: u64,
    track_height: f32,
    density: crate::minimap::Density,
    content_top: f32,
    raw_content_top: f32,
    velocity: f32,
    painted_content_top: f32,
    max_content_top: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct ScrollCamera {
    layout_identity: usize,
    content_y: f32,
    pending_scroll_delta: f32,
    velocity_scale: f32,
    direction: i8,
    initialized: bool,
    reset: bool,
}

pub(super) struct EditorMinimapTelemetry {
    host_id: u64,
    pending_since: std::sync::Mutex<Option<Instant>>,
    last_request: std::sync::Mutex<Option<(Revision, Arc<[u64]>)>>,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    stale_cancels: AtomicU64,
    publishes: AtomicU64,
    #[cfg(feature = "benchmarks")]
    camera_trace: std::sync::Mutex<CameraTraceState>,
    #[cfg(feature = "benchmarks")]
    camera_reverse_frames: AtomicU64,
    #[cfg(feature = "benchmarks")]
    camera_stationary_shift_frames: AtomicU64,
    #[cfg(feature = "benchmarks")]
    camera_stall_frames: AtomicU64,
    #[cfg(feature = "benchmarks")]
    camera_velocity_jank_frames: AtomicU64,
    #[cfg(feature = "benchmarks")]
    image_paint_frames: AtomicU64,
    #[cfg(feature = "benchmarks")]
    visible_image_paint_frames: AtomicU64,
    #[cfg(feature = "benchmarks")]
    complete_image_paint_frames: AtomicU64,
    #[cfg(feature = "benchmarks")]
    jobs: std::sync::Mutex<VecDeque<MinimapJobSample>>,
}

#[cfg(feature = "benchmarks")]
#[derive(Clone, Copy)]
struct CameraTracePoint {
    at: Instant,
    editor_scroll_y: f32,
    content_top: f32,
    source_anchor: f64,
    thumb_top: f32,
    geometry_generation: u64,
}

#[cfg(feature = "benchmarks")]
#[derive(Clone, Copy)]
struct CameraTraceSample {
    dt_ms: f32,
    scroll_delta: f32,
    content_delta: f32,
    source_delta: f64,
    thumb_delta: f32,
    raster_first_unit: Option<u64>,
    raster_viewport_generation: Option<u64>,
    viewport_generation: u64,
    geometry_generation: u64,
    publishes: u64,
}

#[cfg(feature = "benchmarks")]
#[derive(Default)]
struct CameraTraceState {
    previous: Option<CameraTracePoint>,
    previous_velocity: Option<(u64, i8, f32)>,
    samples: Vec<CameraTraceSample>,
    moving: bool,
    burst: u64,
}

#[cfg(feature = "benchmarks")]
#[derive(Clone, Copy)]
struct MinimapJobSample {
    rows: usize,
    visible_rows: usize,
    repeated_rows: usize,
    degraded_rows: u64,
    prepare: Duration,
    raster: Duration,
    rasterizer_lock_wait: Duration,
}

impl EditorMinimapTelemetry {
    fn new() -> Self {
        Self {
            host_id: NEXT_HOST_ID.fetch_add(1, Ordering::Relaxed),
            pending_since: std::sync::Mutex::new(None),
            last_request: std::sync::Mutex::new(None),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            stale_cancels: AtomicU64::new(0),
            publishes: AtomicU64::new(0),
            #[cfg(feature = "benchmarks")]
            camera_trace: std::sync::Mutex::new(CameraTraceState::default()),
            #[cfg(feature = "benchmarks")]
            camera_reverse_frames: AtomicU64::new(0),
            #[cfg(feature = "benchmarks")]
            camera_stationary_shift_frames: AtomicU64::new(0),
            #[cfg(feature = "benchmarks")]
            camera_stall_frames: AtomicU64::new(0),
            #[cfg(feature = "benchmarks")]
            camera_velocity_jank_frames: AtomicU64::new(0),
            #[cfg(feature = "benchmarks")]
            image_paint_frames: AtomicU64::new(0),
            #[cfg(feature = "benchmarks")]
            visible_image_paint_frames: AtomicU64::new(0),
            #[cfg(feature = "benchmarks")]
            complete_image_paint_frames: AtomicU64::new(0),
            #[cfg(feature = "benchmarks")]
            jobs: std::sync::Mutex::new(VecDeque::with_capacity(2_048)),
        }
    }

    pub(super) fn host_id(&self) -> u64 {
        self.host_id
    }

    fn note_cache_lookup(&self, hit: bool) {
        if hit {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.cache_misses.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn note_semantics_pending(&self, pending: bool) {
        let mut since = self
            .pending_since
            .lock()
            .expect("editor minimap pending telemetry poisoned");
        match (pending, since.as_ref()) {
            (true, None) => {
                *since = Some(Instant::now());
                if perf_enabled() {
                    eprintln!(
                        "org_editor_minimap_pending host_id={} state=start",
                        self.host_id
                    );
                }
            }
            (false, Some(started)) => {
                let age = started.elapsed();
                *since = None;
                if perf_enabled() {
                    eprintln!(
                        "org_editor_minimap_pending host_id={} state=ready age_ms={:.3}",
                        self.host_id,
                        age.as_secs_f64() * 1_000.0,
                    );
                }
            }
            _ => {}
        }
    }

    pub(super) fn note_request(&self, revision: Revision, lines: &[u64]) -> usize {
        let mut previous = self
            .last_request
            .lock()
            .expect("editor minimap request telemetry poisoned");
        let repeated = previous
            .as_ref()
            .filter(|(previous_revision, _)| *previous_revision == revision)
            .map_or(0, |(_, previous_lines)| {
                lines
                    .iter()
                    .filter(|line| previous_lines.binary_search(line).is_ok())
                    .count()
            });
        *previous = Some((revision, Arc::from(lines)));
        repeated
    }

    fn note_cancel(&self) {
        self.stale_cancels.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn note_publish(&self) -> u64 {
        self.publishes.fetch_add(1, Ordering::Relaxed) + 1
    }

    #[cfg(feature = "benchmarks")]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn note_camera(
        &self,
        editor_scroll_y: f32,
        content_top: f32,
        source_anchor: f64,
        thumb_top: f32,
        at_endpoint: bool,
        background_camera_movable: bool,
        viewport_generation: u64,
        geometry_generation: u64,
        raster_first_unit: Option<u64>,
        raster_viewport_generation: Option<u64>,
    ) {
        if !perf_enabled() {
            return;
        }
        let now = Instant::now();
        let mut trace = self
            .camera_trace
            .lock()
            .expect("editor minimap camera telemetry poisoned");
        if let Some(previous) = trace.previous {
            let scroll_delta = editor_scroll_y - previous.editor_scroll_y;
            let content_delta = content_top - previous.content_top;
            let source_delta = source_anchor - previous.source_anchor;
            let same_geometry = previous.geometry_generation == geometry_generation;
            let moving = scroll_delta.abs() > 0.05;
            if (scroll_delta > 0.5 && source_delta < -0.001)
                || (scroll_delta < -0.5 && source_delta > 0.001)
            {
                self.camera_reverse_frames.fetch_add(1, Ordering::Relaxed);
                eprintln!(
                    "org_editor_minimap_camera_reverse host_id={} scroll_y={:.3} scroll_delta={:.3} content_delta={:.5} source_delta={:.5} geometry_generation={} previous_geometry_generation={} publishes={}",
                    self.host_id,
                    editor_scroll_y,
                    scroll_delta,
                    content_delta,
                    source_delta,
                    geometry_generation,
                    previous.geometry_generation,
                    self.publishes.load(Ordering::Relaxed),
                );
            }
            if scroll_delta.abs() <= 0.05 && source_delta.abs() > 0.001 {
                self.camera_stationary_shift_frames
                    .fetch_add(1, Ordering::Relaxed);
                eprintln!(
                    "org_editor_minimap_stationary_shift host_id={} scroll_y={:.3} content_delta={:.5} source_delta={:.5} thumb_delta={:.5} viewport_generation={} geometry_generation={} publishes={}",
                    self.host_id,
                    editor_scroll_y,
                    content_delta,
                    source_delta,
                    thumb_top - previous.thumb_top,
                    viewport_generation,
                    geometry_generation,
                    self.publishes.load(Ordering::Relaxed),
                );
            }
            if moving && !at_endpoint && background_camera_movable && same_geometry {
                let direction = if scroll_delta > 0.0 { 1 } else { -1 };
                let velocity = content_delta / scroll_delta;
                if content_delta * scroll_delta <= 0.0 || velocity.abs() < 0.002 {
                    self.camera_stall_frames.fetch_add(1, Ordering::Relaxed);
                    eprintln!(
                        "org_editor_minimap_camera_stall host_id={} scroll_y={:.3} scroll_delta={:.3} content_delta={:.5} velocity={:.6} geometry_generation={} publishes={}",
                        self.host_id,
                        editor_scroll_y,
                        scroll_delta,
                        content_delta,
                        velocity,
                        geometry_generation,
                        self.publishes.load(Ordering::Relaxed),
                    );
                }
                if let Some((previous_generation, previous_direction, previous_velocity)) =
                    trace.previous_velocity
                    && previous_generation == geometry_generation
                    && previous_direction == direction
                {
                    let denominator = previous_velocity.abs().max(velocity.abs()).max(0.001);
                    if (velocity - previous_velocity).abs() / denominator > 0.20 {
                        self.camera_velocity_jank_frames
                            .fetch_add(1, Ordering::Relaxed);
                        eprintln!(
                            "org_editor_minimap_camera_velocity_jank host_id={} scroll_y={:.3} scroll_delta={:.3} velocity={:.6} previous_velocity={:.6} geometry_generation={} publishes={}",
                            self.host_id,
                            editor_scroll_y,
                            scroll_delta,
                            velocity,
                            previous_velocity,
                            geometry_generation,
                            self.publishes.load(Ordering::Relaxed),
                        );
                    }
                }
                trace.previous_velocity = Some((geometry_generation, direction, velocity));
            } else {
                trace.previous_velocity = None;
            }
            if trace_enabled() {
                if moving {
                    trace.moving = true;
                    if trace.samples.len() < 4_096 {
                        trace.samples.push(CameraTraceSample {
                            dt_ms: now.duration_since(previous.at).as_secs_f32() * 1_000.0,
                            scroll_delta,
                            content_delta,
                            source_delta,
                            thumb_delta: thumb_top - previous.thumb_top,
                            raster_first_unit,
                            raster_viewport_generation,
                            viewport_generation,
                            geometry_generation,
                            publishes: self.publishes.load(Ordering::Relaxed),
                        });
                    }
                } else if trace.moving {
                    trace.burst = trace.burst.wrapping_add(1);
                    let burst = trace.burst;
                    eprintln!(
                        "org_editor_minimap_camera_trace_begin host_id={} burst={} samples={}",
                        self.host_id,
                        burst,
                        trace.samples.len()
                    );
                    for (index, sample) in trace.samples.drain(..).enumerate() {
                        eprintln!(
                            "org_editor_minimap_camera_trace host_id={} burst={} index={} dt_ms={:.3} scroll_delta={:.3} content_delta={:.5} source_delta={:.5} thumb_delta={:.5} raster_first_unit={} raster_viewport_generation={} viewport_generation={} geometry_generation={} publishes={}",
                            self.host_id,
                            burst,
                            index,
                            sample.dt_ms,
                            sample.scroll_delta,
                            sample.content_delta,
                            sample.source_delta,
                            sample.thumb_delta,
                            sample
                                .raster_first_unit
                                .map_or_else(|| "none".to_owned(), |value| value.to_string()),
                            sample
                                .raster_viewport_generation
                                .map_or_else(|| "none".to_owned(), |value| value.to_string()),
                            sample.viewport_generation,
                            sample.geometry_generation,
                            sample.publishes,
                        );
                    }
                    eprintln!(
                        "org_editor_minimap_camera_trace_end host_id={} burst={}",
                        self.host_id, burst
                    );
                    trace.moving = false;
                }
            }
        }
        trace.previous = Some(CameraTracePoint {
            at: now,
            editor_scroll_y,
            content_top,
            source_anchor,
            thumb_top,
            geometry_generation,
        });
    }

    #[cfg(feature = "benchmarks")]
    pub(super) fn note_image_paint(
        &self,
        image_bounds: Bounds<Pixels>,
        minimap_bounds: Bounds<Pixels>,
    ) {
        if !perf_enabled() {
            return;
        }
        self.image_paint_frames.fetch_add(1, Ordering::Relaxed);
        let visible = image_bounds.intersect(&minimap_bounds);
        if f32::from(visible.size.width) <= 0.0 || f32::from(visible.size.height) <= 0.0 {
            return;
        }
        let density = crate::minimap::Density::for_width(f32::from(minimap_bounds.size.width));
        let padding = px(density.edge_padding());
        if image_bounds.top() <= minimap_bounds.top() + padding + px(0.5)
            && image_bounds.bottom() >= minimap_bounds.bottom() - padding - px(0.5)
        {
            self.complete_image_paint_frames
                .fetch_add(1, Ordering::Relaxed);
        }
        if self
            .visible_image_paint_frames
            .fetch_add(1, Ordering::Relaxed)
            == 0
        {
            eprintln!(
                "org_editor_minimap_first_visible_image host_id={} image_bounds={image_bounds:?} minimap_bounds={minimap_bounds:?}",
                self.host_id,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn report_job(
        &self,
        rows: usize,
        visible_rows: usize,
        repeated_rows: usize,
        degraded_rows: u64,
        prepare: Duration,
        raster: Duration,
        rasterizer_lock_wait: Duration,
    ) {
        #[cfg(not(feature = "benchmarks"))]
        let _ = (
            rows,
            visible_rows,
            repeated_rows,
            degraded_rows,
            prepare,
            raster,
            rasterizer_lock_wait,
        );
        #[cfg(feature = "benchmarks")]
        {
            if !perf_enabled() {
                return;
            }
            let mut jobs = self
                .jobs
                .lock()
                .expect("editor minimap job telemetry poisoned");
            if jobs.len() == 2_048 {
                jobs.pop_front();
            }
            jobs.push_back(MinimapJobSample {
                rows,
                visible_rows,
                repeated_rows,
                degraded_rows,
                prepare,
                raster,
                rasterizer_lock_wait,
            });
        }
    }

    #[cfg(feature = "benchmarks")]
    pub(super) fn report_summary(&self) {
        if !perf_enabled() {
            return;
        }
        let jobs = self
            .jobs
            .lock()
            .expect("editor minimap job telemetry poisoned");
        let percentile_ms = |value: fn(&MinimapJobSample) -> Duration, percentile: f64| {
            if jobs.is_empty() {
                return 0.0;
            }
            let mut values = jobs.iter().map(value).collect::<Vec<_>>();
            values.sort_unstable();
            let index = ((values.len() - 1) as f64 * percentile).ceil() as usize;
            values[index].as_secs_f64() * 1_000.0
        };
        let rows = jobs.iter().map(|job| job.rows).sum::<usize>();
        let visible_rows = jobs.iter().map(|job| job.visible_rows).sum::<usize>();
        let repeated_rows = jobs.iter().map(|job| job.repeated_rows).sum::<usize>();
        let degraded_rows = jobs.iter().map(|job| job.degraded_rows).sum::<u64>();
        eprintln!(
            "org_editor_minimap_summary host_id={} jobs={} rows={} visible_rows={} prefetch_rows={} repeated_rows={} degraded_rows={} prepare_p50_ms={:.3} prepare_p95_ms={:.3} rasterizer_lock_wait_p50_ms={:.3} rasterizer_lock_wait_p95_ms={:.3} raster_p50_ms={:.3} raster_p95_ms={:.3} cache_hits={} cache_misses={} stale_cancels={} publishes={} image_paint_frames={} visible_image_paint_frames={} complete_image_paint_frames={} camera_reverse_frames={} camera_stationary_shift_frames={} camera_stall_frames={} camera_velocity_jank_frames={}",
            self.host_id,
            jobs.len(),
            rows,
            visible_rows,
            rows.saturating_sub(visible_rows),
            repeated_rows,
            degraded_rows,
            percentile_ms(|job| job.prepare, 0.50),
            percentile_ms(|job| job.prepare, 0.95),
            percentile_ms(|job| job.rasterizer_lock_wait, 0.50),
            percentile_ms(|job| job.rasterizer_lock_wait, 0.95),
            percentile_ms(|job| job.raster, 0.50),
            percentile_ms(|job| job.raster, 0.95),
            self.cache_hits.load(Ordering::Relaxed),
            self.cache_misses.load(Ordering::Relaxed),
            self.stale_cancels.load(Ordering::Relaxed),
            self.publishes.load(Ordering::Relaxed),
            self.image_paint_frames.load(Ordering::Relaxed),
            self.visible_image_paint_frames.load(Ordering::Relaxed),
            self.complete_image_paint_frames.load(Ordering::Relaxed),
            self.camera_reverse_frames.load(Ordering::Relaxed),
            self.camera_stationary_shift_frames.load(Ordering::Relaxed),
            self.camera_stall_frames.load(Ordering::Relaxed),
            self.camera_velocity_jank_frames.load(Ordering::Relaxed),
        );
    }

    #[cfg(feature = "benchmarks")]
    pub(super) fn reset_samples(&self) {
        self.jobs
            .lock()
            .expect("editor minimap job telemetry poisoned")
            .clear();
        self.last_request
            .lock()
            .expect("editor minimap request telemetry poisoned")
            .take();
        self.cache_hits.store(0, Ordering::Relaxed);
        self.cache_misses.store(0, Ordering::Relaxed);
        self.stale_cancels.store(0, Ordering::Relaxed);
        self.publishes.store(0, Ordering::Relaxed);
        self.image_paint_frames.store(0, Ordering::Relaxed);
        self.visible_image_paint_frames.store(0, Ordering::Relaxed);
        self.complete_image_paint_frames.store(0, Ordering::Relaxed);
        *self
            .camera_trace
            .lock()
            .expect("editor minimap camera telemetry poisoned") = CameraTraceState::default();
        self.camera_reverse_frames.store(0, Ordering::Relaxed);
        self.camera_stationary_shift_frames
            .store(0, Ordering::Relaxed);
        self.camera_stall_frames.store(0, Ordering::Relaxed);
        self.camera_velocity_jank_frames.store(0, Ordering::Relaxed);
    }
}

pub(super) struct EditorMinimapHost {
    pub(super) visible: bool,
    pub(super) reveal: f32,
    pub(super) width: f32,
    pub(super) bounds: Option<Bounds<Pixels>>,
    pub(super) drag: Option<crate::minimap::DragSession>,
    pub(super) resizing: Option<(f32, f32)>,
    pub(super) generation: u64,
    pub(super) revision: Option<Revision>,
    pub(super) search_marks: Arc<[ByteRange]>,
    pub(super) prepared_frame: std::sync::Mutex<Option<PreparedEditorMinimapFrame>>,
    pub(super) raster_build: std::sync::Mutex<Option<RasterKey>>,
    pub(super) raster_epoch: Arc<AtomicU64>,
    pub(super) scale_factor: f32,
    viewport_generation: u64,
    pending_layout_raster_invalidation: bool,
    layout_refinement_generation: Option<u64>,
    last_layout_raster_activity: Instant,
    viewport_anchor: std::sync::Mutex<Option<ViewportAnchor>>,
    scroll_camera: std::sync::Mutex<ScrollCamera>,
    layout_preparation: std::sync::Mutex<LayoutPreparationState>,
    layout_preparation_epoch: Arc<AtomicU64>,
    pub(super) telemetry: Arc<EditorMinimapTelemetry>,
}

impl Default for EditorMinimapHost {
    fn default() -> Self {
        Self {
            visible: true,
            reveal: 1.0,
            width: DEFAULT_WIDTH,
            bounds: None,
            drag: None,
            resizing: None,
            generation: 0,
            revision: None,
            search_marks: Arc::from([]),
            prepared_frame: std::sync::Mutex::new(None),
            raster_build: std::sync::Mutex::new(None),
            raster_epoch: Arc::new(AtomicU64::new(0)),
            scale_factor: 1.0,
            viewport_generation: 0,
            pending_layout_raster_invalidation: false,
            layout_refinement_generation: None,
            last_layout_raster_activity: Instant::now(),
            viewport_anchor: std::sync::Mutex::new(None),
            scroll_camera: std::sync::Mutex::new(ScrollCamera::default()),
            layout_preparation: std::sync::Mutex::new(LayoutPreparationState::default()),
            layout_preparation_epoch: Arc::new(AtomicU64::new(0)),
            telemetry: Arc::new(EditorMinimapTelemetry::new()),
        }
    }
}

impl EditorMinimapHost {
    /// Selects the current geometry configuration and reserves at most one build for it.
    /// Returns the cancellation epoch captured by the new background job.
    pub(super) fn reserve_layout_preparation(&self, key: PreparedLayoutKey) -> Option<u64> {
        let mut state = self
            .layout_preparation
            .lock()
            .expect("editor minimap layout preparation poisoned");
        if state.desired.as_ref() != Some(&key) {
            state.desired = Some(key.clone());
            state.in_flight = None;
            state.ready = None;
            self.layout_preparation_epoch.fetch_add(1, Ordering::AcqRel);
        }
        if state.ready.as_ref().is_some_and(|ready| ready.key == key)
            || state.in_flight.as_ref() == Some(&key)
        {
            return None;
        }
        state.in_flight = Some(key);
        Some(self.layout_preparation_epoch.load(Ordering::Acquire))
    }

    pub(super) fn layout_preparation_epoch(&self) -> Arc<AtomicU64> {
        self.layout_preparation_epoch.clone()
    }

    pub(super) fn cancel_layout_preparation(&self) {
        let mut state = self
            .layout_preparation
            .lock()
            .expect("editor minimap layout preparation poisoned");
        state.desired = None;
        state.in_flight = None;
        state.ready = None;
        self.layout_preparation_epoch.fetch_add(1, Ordering::AcqRel);
    }

    /// Publishes a complete layout only if none of its geometry inputs changed while it built.
    pub(super) fn publish_prepared_layout(
        &self,
        key: &PreparedLayoutKey,
        epoch: u64,
        layout: Arc<super::layout_map::EditorLayoutMap>,
    ) -> bool {
        if self.layout_preparation_epoch.load(Ordering::Acquire) != epoch {
            return false;
        }
        let mut state = self
            .layout_preparation
            .lock()
            .expect("editor minimap layout preparation poisoned");
        if state.desired.as_ref() != Some(key) || state.in_flight.as_ref() != Some(key) {
            return false;
        }
        state.in_flight = None;
        state.ready = Some(PreparedLayout {
            key: key.clone(),
            layout,
        });
        true
    }

    pub(super) fn abandon_layout_preparation(&self, key: &PreparedLayoutKey, epoch: u64) -> bool {
        if self.layout_preparation_epoch.load(Ordering::Acquire) != epoch {
            return false;
        }
        let mut state = self
            .layout_preparation
            .lock()
            .expect("editor minimap layout preparation poisoned");
        if state.desired.as_ref() != Some(key) || state.in_flight.as_ref() != Some(key) {
            return false;
        }
        state.in_flight = None;
        true
    }

    /// A new document or geometry configuration needs a complete layout before
    /// it can replace an already displayed frame. Keep this true across retries.
    pub(super) fn layout_preparation_pending(&self) -> bool {
        let state = self
            .layout_preparation
            .lock()
            .expect("editor minimap layout preparation poisoned");
        state.desired.as_ref().is_some_and(|desired| {
            state
                .ready
                .as_ref()
                .is_none_or(|ready| &ready.key != desired)
        })
    }

    pub(super) fn prepared_layout(&self) -> Option<Arc<super::layout_map::EditorLayoutMap>> {
        let state = self
            .layout_preparation
            .lock()
            .expect("editor minimap layout preparation poisoned");
        let desired = state.desired.as_ref()?;
        state
            .ready
            .as_ref()
            .filter(|ready| &ready.key == desired)
            .map(|ready| ready.layout.clone())
    }

    pub(super) fn active_frame_has_complete_layout(&self) -> bool {
        self.active_frame()
            .is_some_and(|active| active.uses_complete_layout)
    }

    pub(super) fn active_frame(&self) -> Option<PreparedEditorMinimapFrame> {
        self.prepared_frame
            .lock()
            .expect("editor minimap prepared frame poisoned")
            .clone()
    }

    pub(super) fn active_layout(&self) -> Option<Arc<super::layout_map::EditorLayoutMap>> {
        self.active_frame().map(|frame| frame.layout)
    }

    pub(super) fn publish_frame(&self, mut frame: PreparedEditorMinimapFrame) {
        debug_assert!(frame.is_coherent());
        // Record provenance at publication instead of comparing with the latest
        // prepared layout on every paint. Reserving its successor clears `ready`,
        // but must not send this frame back through the stale bootstrap camera.
        frame.uses_complete_layout |= self
            .prepared_layout()
            .is_some_and(|prepared| Arc::ptr_eq(&frame.layout, &prepared));
        let mut active = self
            .prepared_frame
            .lock()
            .expect("editor minimap prepared frame poisoned");
        if let Some(previous) = active.as_ref()
            && !Arc::ptr_eq(&previous.layout, &frame.layout)
        {
            let mut camera = self
                .scroll_camera
                .lock()
                .expect("editor minimap scroll camera poisoned");
            if camera.initialized && !camera.reset {
                let line = previous.layout.line_at_y(camera.content_y);
                let previous_line_start = previous.layout.line_start_y(line);
                let line_fraction = ((camera.content_y - previous_line_start)
                    / previous.layout.line_height_px(line).max(1.0))
                .clamp(0.0, 1.0);
                camera.content_y = frame.layout.line_start_y(line)
                    + line_fraction * frame.layout.line_height_px(line);
                camera.layout_identity = Arc::as_ptr(&frame.layout) as usize;
            }
            let mut viewport_anchor = self
                .viewport_anchor
                .lock()
                .expect("editor minimap viewport anchor poisoned");
            if let Some(anchor) = viewport_anchor.as_mut() {
                let previous_y =
                    anchor.painted_content_top * previous.layout.base_line_height().max(1.0);
                let line = previous.layout.line_at_y(previous_y);
                let previous_line_start = previous.layout.line_start_y(line);
                let line_fraction = ((previous_y - previous_line_start)
                    / previous.layout.line_height_px(line).max(1.0))
                .clamp(0.0, 1.0);
                let next_y = frame.layout.line_start_y(line)
                    + line_fraction * frame.layout.line_height_px(line);
                anchor.content_top = next_y / frame.layout.base_line_height().max(1.0);
                anchor.raw_content_top = f32::NAN;
                anchor.velocity = 0.0;
                anchor.viewport_generation = self.viewport_generation;
            }
        }
        *active = Some(frame);
        drop(active);
    }
    pub(super) fn note_semantics_pending(&self, pending: bool) {
        self.telemetry.note_semantics_pending(pending);
    }

    pub(super) fn note_cache_lookup(&self, hit: bool) {
        self.telemetry.note_cache_lookup(hit);
    }

    pub(super) fn stabilize_viewport(
        &self,
        mut viewport: ViewportGeometry,
        total_units: f32,
        visible_top: f32,
        visible_bottom: f32,
        track_height: f32,
        density: crate::minimap::Density,
    ) -> ViewportGeometry {
        let mut anchor = self
            .viewport_anchor
            .lock()
            .expect("editor minimap viewport anchor poisoned");
        let compatible = anchor.is_some_and(|anchor| {
            (anchor.track_height - track_height).abs() < 0.5 && anchor.density == density
        });
        let raw_content_top = viewport.content_top;
        let visible_minimap_units = ((viewport.interaction_height - density.edge_padding() * 2.0)
            / self.line_height(density).max(1.0))
        .max(1.0);
        let max_content_top = (total_units - visible_minimap_units).max(0.0);
        let mut velocity = 0.0;
        let preferred_content_top;
        if compatible {
            let previous = (*anchor).expect("compatible anchor must exist");
            let preferred = if previous.viewport_generation == self.viewport_generation {
                previous.content_top
            } else if previous.raw_content_top.is_finite() {
                let raw_delta = raw_content_top - previous.raw_content_top;
                let same_direction = previous.velocity * raw_delta > 0.0;
                velocity = if same_direction && previous.velocity.abs() > 0.01 {
                    let first = previous.velocity * 0.90;
                    let second = previous.velocity * 1.10;
                    raw_delta.clamp(first.min(second), first.max(second))
                } else {
                    raw_delta
                };
                previous.content_top + velocity
            } else {
                previous.content_top
            };
            viewport = crate::minimap::stabilize_projection_camera_with_line_height(
                viewport,
                total_units,
                visible_top,
                visible_bottom,
                density,
                self.line_height(density),
                preferred,
            );
            preferred_content_top = preferred;
        } else {
            viewport.content_top = raw_content_top;
            preferred_content_top = raw_content_top;
        }
        *anchor = Some(ViewportAnchor {
            viewport_generation: self.viewport_generation,
            track_height,
            density,
            content_top: preferred_content_top,
            raw_content_top,
            velocity,
            painted_content_top: viewport.content_top,
            max_content_top,
        });
        viewport
    }

    pub(super) fn background_camera_movable(&self) -> bool {
        self.viewport_anchor
            .lock()
            .expect("editor minimap viewport anchor poisoned")
            .is_some_and(|anchor| {
                let margin = 2.0;
                anchor.painted_content_top > margin
                    && anchor.painted_content_top < anchor.max_content_top - margin
            })
    }

    pub(super) fn note_viewport_changed(&mut self) {
        self.viewport_generation = self.viewport_generation.wrapping_add(1);
        *self
            .viewport_anchor
            .lock()
            .expect("editor minimap viewport anchor poisoned") = None;
        let mut camera = self
            .scroll_camera
            .lock()
            .expect("editor minimap scroll camera poisoned");
        camera.pending_scroll_delta = 0.0;
        camera.reset = true;
        if self.pending_layout_raster_invalidation {
            self.last_layout_raster_activity = Instant::now();
        }
    }

    pub(super) fn note_viewport_scrolled(&mut self, delta_y: f32) {
        self.viewport_generation = self.viewport_generation.wrapping_add(1);
        let mut camera = self
            .scroll_camera
            .lock()
            .expect("editor minimap scroll camera poisoned");
        camera.pending_scroll_delta += delta_y;
        if self.pending_layout_raster_invalidation {
            self.last_layout_raster_activity = Instant::now();
        }
    }

    pub(super) fn project_scroll_camera(
        &self,
        live: &super::layout_map::EditorLayoutMap,
        prepared: &PreparedEditorMinimapFrame,
        live_scroll_y: f32,
        live_max_scroll: f32,
        prepared_max_scroll: f32,
    ) -> f32 {
        let at_start = live_scroll_y <= 0.5;
        let at_end = live_scroll_y + 0.5 >= live_max_scroll;
        let layout_identity = Arc::as_ptr(&prepared.layout) as usize;
        let mut camera = self
            .scroll_camera
            .lock()
            .expect("editor minimap scroll camera poisoned");
        if at_start {
            camera.content_y = 0.0;
            camera.pending_scroll_delta = 0.0;
            camera.velocity_scale = 0.0;
            camera.direction = 0;
            camera.layout_identity = layout_identity;
            camera.initialized = true;
            camera.reset = false;
            return 0.0;
        }
        if at_end {
            camera.content_y = prepared_max_scroll;
            camera.pending_scroll_delta = 0.0;
            camera.velocity_scale = 0.0;
            camera.direction = 0;
            camera.layout_identity = layout_identity;
            camera.initialized = true;
            camera.reset = false;
            return prepared_max_scroll;
        }
        if !camera.initialized || camera.reset || camera.layout_identity != layout_identity {
            let line = live.line_at_y(live_scroll_y);
            let live_line_start = live.line_start_y(line);
            let line_fraction = ((live_scroll_y - live_line_start)
                / live.line_height_px(line).max(1.0))
            .clamp(0.0, 1.0);
            camera.content_y = (prepared.layout.line_start_y(line)
                + line_fraction * prepared.layout.line_height_px(line))
            .clamp(0.0, prepared_max_scroll);
            camera.layout_identity = layout_identity;
            camera.initialized = true;
            camera.reset = false;
            camera.pending_scroll_delta = 0.0;
            camera.velocity_scale = 0.0;
            camera.direction = 0;
            return camera.content_y;
        }

        let delta = std::mem::take(&mut camera.pending_scroll_delta);
        let desired_scale = if delta > 0.0 {
            let live_before = (live_scroll_y - delta).clamp(0.0, live_max_scroll);
            let live_remaining = (live_max_scroll - live_before).max(delta);
            let prepared_remaining = (prepared_max_scroll - camera.content_y).max(0.0);
            Some((1, prepared_remaining / live_remaining))
        } else if delta < 0.0 {
            let live_before = (live_scroll_y - delta).clamp(0.0, live_max_scroll);
            let live_distance = live_before.max(-delta);
            Some((-1, camera.content_y.max(0.0) / live_distance))
        } else {
            None
        };
        if let Some((direction, desired_scale)) = desired_scale {
            let scale = if camera.direction == direction && camera.velocity_scale > 0.0 {
                desired_scale.clamp(camera.velocity_scale * 0.90, camera.velocity_scale * 1.10)
            } else {
                desired_scale
            };
            camera.direction = direction;
            camera.velocity_scale = scale;
            camera.content_y += delta * scale;
        }
        camera.content_y = camera.content_y.clamp(0.0, prepared_max_scroll);
        camera.content_y
    }

    pub(super) fn note_layout_changed(&mut self) {
        self.pending_layout_raster_invalidation = true;
        self.last_layout_raster_activity = Instant::now();
    }

    /// Returns whether another frame is needed before the pending raster update can settle.
    pub(super) fn settle_layout_raster_invalidation(&mut self) -> bool {
        if !self.pending_layout_raster_invalidation {
            return false;
        }
        if self.last_layout_raster_activity.elapsed() < LAYOUT_RASTER_SETTLE {
            return true;
        }
        self.pending_layout_raster_invalidation = false;
        // A complete prepared layout already contains the rows the live Editor just measured.
        // Rebuilding from the live sparse map would replace exact minimap geometry with a partial
        // snapshot and reintroduce the refresh-under-the-thumb regression.
        if self.prepared_layout().is_some() {
            return false;
        }
        self.invalidate_raster();
        self.layout_refinement_generation = Some(self.generation);
        false
    }

    pub(super) fn is_layout_refinement_generation(&self, generation: u64) -> bool {
        self.layout_refinement_generation == Some(generation)
    }

    pub(super) fn viewport_generation(&self) -> u64 {
        self.viewport_generation
    }

    pub(super) fn line_height(&self, density: crate::minimap::Density) -> f32 {
        raster_line_height(density, self.scale_factor)
    }

    pub(super) fn reserve_raster(&self, key: RasterKey) -> bool {
        let mut in_flight = self
            .raster_build
            .lock()
            .expect("editor minimap raster build poisoned");
        match *in_flight {
            Some(current) if current == key => false,
            Some(_) => {
                // A fast scroll can leave a bounded raster request far behind the current
                // camera. Make the newest window win immediately; the epoch check aborts the
                // stale job before or during rasterization, and its completion cannot clear the
                // replacement key.
                self.raster_epoch.fetch_add(1, Ordering::AcqRel);
                self.telemetry.note_cancel();
                *in_flight = Some(key);
                true
            }
            None => {
                *in_flight = Some(key);
                true
            }
        }
    }

    pub(super) fn invalidate_raster(&mut self) {
        self.pending_layout_raster_invalidation = false;
        self.layout_refinement_generation = None;
        self.generation = self.generation.wrapping_add(1);
        self.raster_epoch.fetch_add(1, Ordering::AcqRel);
        let cancelled = self
            .raster_build
            .lock()
            .expect("editor minimap raster build poisoned")
            .take()
            .is_some();
        if cancelled {
            self.telemetry.note_cancel();
        }
    }

    pub(super) fn update_snapshot(&mut self, snapshot: &crate::document::DocumentSnapshot) {
        if self.revision != Some(snapshot.revision()) {
            self.invalidate_raster();
            self.revision = Some(snapshot.revision());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_delimiter_columns(text: &str, format: DocumentFormat) -> Vec<usize> {
        let geometry = TableRowGeometry::from_source(text, format).unwrap();
        table_fragments(text, &geometry)
            .into_iter()
            .filter_map(|(range, column)| {
                matches!(text.as_bytes().get(range.start), Some(b'|' | b'+')).then_some(column)
            })
            .collect()
    }

    #[test]
    fn table_fragments_align_mixed_width_rows_in_logical_columns() {
        assert_eq!(
            table_delimiter_columns("| English | 中文    |", DocumentFormat::Org),
            vec![0, 10, 20]
        );
        assert_eq!(
            table_delimiter_columns("| 中文    | English |", DocumentFormat::Org),
            vec![0, 10, 20]
        );
        assert_eq!(
            table_delimiter_columns("|a| bbbb |", DocumentFormat::Org),
            vec![0, 2, 9],
            "an unaligned source row must keep its original logical delimiter positions"
        );
    }

    fn prepared_layout_key(revision: u64, wrap_width: f32) -> PreparedLayoutKey {
        PreparedLayoutKey {
            revision: Revision(revision),
            path: std::path::PathBuf::from("fixture.org"),
            line_count: 100,
            wrap_width_bits: wrap_width.to_bits(),
            base_line_height_bits: 24.0_f32.to_bits(),
            soft_wrap: true,
            font: Font::default(),
            font_size_bits: 15.0_f32.to_bits(),
            content_scale_bits: 1.0_f32.to_bits(),
            fold_revision: 0,
            inline_images: true,
            inline_image_overrides: Arc::from([]),
            inline_image_resource_generation: 0,
        }
    }

    #[test]
    fn prepared_layout_build_is_deduplicated_and_published_for_the_exact_key() {
        let host = EditorMinimapHost::default();
        let key = prepared_layout_key(1, 400.0);
        let epoch = host
            .reserve_layout_preparation(key.clone())
            .expect("first request should reserve a build");
        assert!(host.reserve_layout_preparation(key.clone()).is_none());

        let mut layout = super::super::layout_map::EditorLayoutMap::default();
        layout.configure(100, 400.0);
        let layout = Arc::new(layout);
        assert!(host.publish_prepared_layout(&key, epoch, layout.clone()));
        assert!(Arc::ptr_eq(
            &host
                .prepared_layout()
                .expect("prepared layout should be ready"),
            &layout
        ));
        assert!(host.reserve_layout_preparation(key).is_none());
    }

    #[test]
    fn changed_geometry_key_cancels_stale_background_publication() {
        let host = EditorMinimapHost::default();
        let old = prepared_layout_key(1, 400.0);
        let old_epoch = host
            .reserve_layout_preparation(old.clone())
            .expect("old build should reserve");
        let new = prepared_layout_key(1, 500.0);
        let new_epoch = host
            .reserve_layout_preparation(new.clone())
            .expect("new geometry should replace the build");
        assert_ne!(old_epoch, new_epoch);

        let layout = Arc::new(super::super::layout_map::EditorLayoutMap::default());
        assert!(!host.publish_prepared_layout(&old, old_epoch, layout));
        assert!(host.prepared_layout().is_none());
    }

    #[test]
    fn complete_layout_ignores_later_sparse_visible_measurements() {
        let mut host = EditorMinimapHost::default();
        let key = prepared_layout_key(1, 400.0);
        let epoch = host
            .reserve_layout_preparation(key.clone())
            .expect("build should reserve");
        let layout = Arc::new(super::super::layout_map::EditorLayoutMap::default());
        assert!(host.publish_prepared_layout(&key, epoch, layout));
        let generation = host.generation;
        let raster_epoch = host.raster_epoch.load(Ordering::Acquire);

        host.note_layout_changed();
        host.last_layout_raster_activity = Instant::now() - LAYOUT_RASTER_SETTLE;
        assert!(!host.settle_layout_raster_invalidation());
        assert_eq!(host.generation, generation);
        assert_eq!(host.raster_epoch.load(Ordering::Acquire), raster_epoch);
        assert!(!host.pending_layout_raster_invalidation);
    }

    #[test]
    fn wrapped_rows_fill_text_segments_and_background_spacing_without_repetition() {
        let mut raster_lines = vec![None; 8];
        fill_visual_rows(&mut raster_lines, 12, 1.25, 3.5, &[5, 9]);
        fill_visual_rows(&mut raster_lines, 13, 4.75, 1.0, &[]);

        assert_eq!(
            raster_lines,
            vec![
                None,
                Some(RasterSourceRow {
                    line: 12,
                    text_range: Some((0, Some(5))),
                }),
                Some(RasterSourceRow {
                    line: 12,
                    text_range: Some((5, Some(9))),
                }),
                Some(RasterSourceRow {
                    line: 12,
                    text_range: Some((9, None)),
                }),
                Some(RasterSourceRow {
                    line: 13,
                    text_range: Some((0, None)),
                }),
                Some(RasterSourceRow {
                    line: 13,
                    text_range: None,
                }),
                None,
                None
            ]
        );
    }

    fn semantic_span(byte: usize) -> super::super::syntax::EditorSemanticSpan {
        super::super::syntax::EditorSemanticSpan {
            bytes: byte..byte + 1,
            color: Some(super::super::syntax::EditorColorToken::Keyword),
            weight: super::super::syntax::EditorSemanticWeight::Bold,
            italic: false,
            underline: false,
            strikethrough: false,
            pill: false,
            link: None,
        }
    }

    #[test]
    fn hosts_keep_independent_generation_and_interaction_state() {
        let snapshot =
            crate::document::DocumentSnapshot::from_utf8(b"* one\nbody\n".to_vec()).unwrap();
        let mut left = EditorMinimapHost::default();
        let mut right = EditorMinimapHost::default();
        left.update_snapshot(&snapshot);
        right.update_snapshot(&snapshot);
        left.drag = Some(crate::minimap::DragSession {
            start_pointer_y: 4.0,
            start_thumb_top: 0.0,
            start_ratio: 0.0,
            current_thumb_top: 0.0,
        });
        left.width = 140.0;
        assert_eq!(left.generation, 1);
        assert_eq!(right.generation, 1);
        assert_eq!(right.drag, None);
        assert_eq!(right.width, DEFAULT_WIDTH);
        assert_ne!(left.telemetry.host_id(), right.telemetry.host_id());
    }

    #[test]
    fn pending_age_is_owned_and_resolved_by_each_host() {
        let host = EditorMinimapHost::default();
        host.note_semantics_pending(true);
        assert!(host.telemetry.pending_since.lock().unwrap().is_some());
        host.note_semantics_pending(true);
        assert!(host.telemetry.pending_since.lock().unwrap().is_some());
        host.note_semantics_pending(false);
        assert!(host.telemetry.pending_since.lock().unwrap().is_none());
    }

    #[test]
    fn repeated_raster_rows_are_counted_only_within_the_same_revision() {
        let telemetry = EditorMinimapTelemetry::new();
        assert_eq!(telemetry.note_request(Revision(1), &[10, 11, 12]), 0);
        assert_eq!(telemetry.note_request(Revision(1), &[12, 13, 14]), 1);
        assert_eq!(telemetry.note_request(Revision(2), &[12, 13, 14]), 0);
    }

    #[test]
    fn newer_raster_window_supersedes_an_in_flight_scroll_request() {
        let host = EditorMinimapHost::default();
        let first = RasterKey {
            generation: 1,
            first_unit: 0,
            rows: 200,
            width: 96,
            scale_x100: 200,
            density: crate::minimap::Density::Compact,
            theme_generation: 0,
        };
        let next = RasterKey {
            first_unit: 64,
            ..first
        };
        assert!(host.reserve_raster(first));
        let first_epoch = host.raster_epoch.load(Ordering::Acquire);
        assert!(!host.reserve_raster(first));
        assert_eq!(host.raster_epoch.load(Ordering::Acquire), first_epoch);
        assert!(host.reserve_raster(next));
        assert_ne!(host.raster_epoch.load(Ordering::Acquire), first_epoch);
        assert_eq!(host.telemetry.stale_cancels.load(Ordering::Relaxed), 1);
        assert_eq!(
            *host
                .raster_build
                .lock()
                .expect("editor minimap raster build poisoned"),
            Some(next)
        );
    }

    #[test]
    fn raster_window_is_stable_within_a_prefetched_scroll_chunk() {
        let density = crate::minimap::Density::Compact;
        let line_height = raster_line_height(density, 2.0);
        let first = raster_window(70.0, 300.0, 1_000.0, density, line_height);
        let nearby = raster_window(100.0, 300.0, 1_000.0, density, line_height);
        let next = raster_window(129.0, 300.0, 1_000.0, density, line_height);
        assert_eq!(first, nearby);
        assert_eq!(first.0, 0);
        assert_eq!(next.0, 64);
        let visible_rows = ((300.0 - density.edge_padding() * 2.0) / line_height).ceil() + 1.0;
        let old_end = first.0 as f32 + first.1 as f32;
        let next_visible_end = 129.0 + visible_rows;
        assert!(old_end - next_visible_end >= (RASTER_PREFETCH_ROWS - 1) as f32);
    }

    #[test]
    fn raster_window_does_not_include_rows_after_document_end() {
        let density = crate::minimap::Density::Compact;
        let line_height = raster_line_height(density, 2.0);
        let (first, rows) = raster_window(900.0, 300.0, 1_000.0, density, line_height);
        assert_eq!(first + rows as u64, 1_000);
    }

    #[test]
    fn editor_raster_rows_align_to_physical_pixels() {
        for density in [
            crate::minimap::Density::Compact,
            crate::minimap::Density::Comfortable,
            crate::minimap::Density::Large,
            crate::minimap::Density::ExtraLarge,
            crate::minimap::Density::Maximum,
        ] {
            for scale_factor in [1.0, 1.5, 2.0, 3.0] {
                let physical = raster_line_height(density, scale_factor) * scale_factor;
                assert!((physical - physical.round()).abs() < f32::EPSILON);
            }
        }
    }

    #[test]
    fn rich_span_budget_degrades_the_whole_row_after_the_row_limit() {
        let text = "a".repeat(MAX_RICH_SPANS_PER_ROW + 1);
        let spans = (0..text.len()).map(semantic_span).collect();
        let before = rich_span_degraded_rows();
        let adapted = RichSpanBudget::default().adapt(&text, spans, crate::theme::current_theme());

        assert!(adapted.is_empty());
        assert!(rich_span_degraded_rows() > before);
    }

    #[test]
    fn rich_span_budget_degrades_rows_after_the_request_limit() {
        let mut budget = RichSpanBudget::default();
        let theme = crate::theme::current_theme();
        for _ in 0..MAX_RICH_SPANS_PER_REQUEST {
            assert_eq!(budget.adapt("a", vec![semantic_span(0)], theme).len(), 1);
        }
        assert!(budget.adapt("a", vec![semantic_span(0)], theme).is_empty());
    }

    #[test]
    fn rich_text_segments_preserve_color_emphasis_and_utf8_boundaries() {
        let row = TextRow {
            text: "甲😀b".to_owned(),
            color: 0,
            weight: TextWeight::Normal,
            italic: false,
            spans: vec![TextSpan {
                bytes: 3..7,
                color: Some(0x12_34_56),
                weight: TextWeight::Bold,
                italic: true,
                underline: true,
                strikethrough: true,
            }],
            indent: 0.0,
            block_background: None,
            block_accent: None,
            block_edge: None,
            table: None,
        };
        let base = cosmic_text::Attrs::new().family(cosmic_text::Family::Monospace);
        let segments = rich_text_segments(&row.text, &row.spans, &base);

        assert_eq!(
            segments.iter().map(|(text, _)| *text).collect::<String>(),
            row.text
        );
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[1].0, "😀");
        assert_eq!(
            segments[1].1.color_opt,
            Some(cosmic_text::Color::rgb(0x12, 0x34, 0x56))
        );
        assert_eq!(segments[1].1.weight, cosmic_text::Weight::BOLD);
        assert_eq!(segments[1].1.style, cosmic_text::Style::Italic);
        assert!(segments[1].1.text_decoration.strikethrough);
        assert_eq!(
            segments[1].1.text_decoration.underline,
            cosmic_text::UnderlineStyle::Single
        );
    }

    #[test]
    fn invalidation_cancels_the_previous_raster_epoch() {
        let mut host = EditorMinimapHost::default();
        let epoch = host.raster_epoch.load(Ordering::Acquire);
        assert!(host.reserve_raster(RasterKey {
            generation: 0,
            first_unit: 0,
            rows: 1,
            width: 96,
            scale_x100: 100,
            density: crate::minimap::Density::Compact,
            theme_generation: 0,
        }));
        host.invalidate_raster();
        assert_ne!(host.raster_epoch.load(Ordering::Acquire), epoch);
        assert_eq!(host.telemetry.stale_cancels.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn measured_layout_updates_are_published_once_after_scroll_settles() {
        let mut host = EditorMinimapHost::default();
        let generation = host.generation;
        let epoch = host.raster_epoch.load(Ordering::Acquire);

        host.note_layout_changed();
        assert!(host.settle_layout_raster_invalidation());
        assert_eq!(host.generation, generation);
        assert_eq!(host.raster_epoch.load(Ordering::Acquire), epoch);

        host.note_viewport_changed();
        host.last_layout_raster_activity = Instant::now() - LAYOUT_RASTER_SETTLE;
        assert!(!host.settle_layout_raster_invalidation());
        assert_eq!(host.generation, generation.wrapping_add(1));
        assert_ne!(host.raster_epoch.load(Ordering::Acquire), epoch);
        assert!(!host.pending_layout_raster_invalidation);

        assert!(!host.settle_layout_raster_invalidation());
        assert_eq!(host.generation, generation.wrapping_add(1));
    }

    #[test]
    fn explicit_invalidation_supersedes_a_pending_layout_update() {
        let mut host = EditorMinimapHost::default();
        host.note_layout_changed();

        host.invalidate_raster();

        assert!(!host.pending_layout_raster_invalidation);
        assert!(!host.settle_layout_raster_invalidation());
        assert_eq!(host.generation, 1);
    }

    #[test]
    fn invalidation_keeps_previous_pixels_until_replacement_is_ready() {
        let mut host = EditorMinimapHost::default();
        let key = RasterKey {
            generation: 0,
            first_unit: 0,
            rows: 1,
            width: 1,
            scale_x100: 100,
            density: crate::minimap::Density::Compact,
            theme_generation: 0,
        };
        let image = Arc::new(RenderImage::new(SmallVec::from_elem(
            Frame::new(RgbaImage::new(1, 1)),
            1,
        )));
        let layout = Arc::new(super::super::layout_map::EditorLayoutMap::default());
        host.publish_frame(PreparedEditorMinimapFrame::new(
            layout,
            CachedRaster {
                key,
                image,
                media: Arc::from([]),
                content_top: 0.0,
                viewport_generation: 0,
                line_height: raster_line_height(crate::minimap::Density::Compact, 1.0),
            },
        ));

        host.invalidate_raster();

        assert!(
            host.prepared_frame
                .lock()
                .expect("editor minimap prepared frame poisoned")
                .is_some()
        );
    }

    #[test]
    fn prepared_frame_owns_the_layout_used_by_its_raster() {
        let mut layout = super::super::layout_map::EditorLayoutMap::default();
        layout.configure(100, 400.0);
        let layout = Arc::new(layout);
        let key = RasterKey {
            generation: 7,
            first_unit: 32,
            rows: 1,
            width: 1,
            scale_x100: 100,
            density: crate::minimap::Density::Compact,
            theme_generation: 0,
        };
        let image = Arc::new(RenderImage::new(SmallVec::from_elem(
            Frame::new(RgbaImage::new(1, 1)),
            1,
        )));
        let frame = PreparedEditorMinimapFrame::new(
            layout.clone(),
            CachedRaster {
                key,
                image,
                media: Arc::from([]),
                content_top: 32.0,
                viewport_generation: 3,
                line_height: 1.0,
            },
        );

        assert!(frame.is_coherent());
        assert_eq!(frame.geometry_generation.raster_generation, 7);
        assert_eq!(
            frame.geometry_generation.layout_identity,
            Arc::as_ptr(&layout) as usize
        );
    }

    #[test]
    fn publishing_refined_geometry_preserves_the_active_source_anchor() {
        fn raster(generation: u64) -> CachedRaster {
            CachedRaster {
                key: RasterKey {
                    generation,
                    first_unit: 0,
                    rows: 1,
                    width: 1,
                    scale_x100: 100,
                    density: crate::minimap::Density::Compact,
                    theme_generation: 0,
                },
                image: Arc::new(RenderImage::new(SmallVec::from_elem(
                    Frame::new(RgbaImage::new(1, 1)),
                    1,
                ))),
                media: Arc::from([]),
                content_top: 0.0,
                viewport_generation: 0,
                line_height: 1.0,
            }
        }

        let mut original = super::super::layout_map::EditorLayoutMap::default();
        original.configure(1_000, 400.0);
        let original = Arc::new(original);
        let mut refined = original.as_ref().clone();
        refined.update_line_layout(100, 12, 24.0, 0.0, 0.0);
        let refined = Arc::new(refined);
        let host = EditorMinimapHost::default();
        host.publish_frame(PreparedEditorMinimapFrame::new(original.clone(), raster(1)));
        {
            let mut camera = host
                .scroll_camera
                .lock()
                .expect("editor minimap scroll camera poisoned");
            camera.content_y = original.line_start_y(500) + original.line_height_px(500) * 0.25;
            camera.layout_identity = Arc::as_ptr(&original) as usize;
            camera.initialized = true;
        }

        host.publish_frame(PreparedEditorMinimapFrame::new(refined.clone(), raster(2)));

        let camera = host
            .scroll_camera
            .lock()
            .expect("editor minimap scroll camera poisoned");
        let expected = refined.line_start_y(500) + refined.line_height_px(500) * 0.25;
        assert!((camera.content_y - expected).abs() < 0.001);
        assert_eq!(camera.layout_identity, Arc::as_ptr(&refined) as usize);
    }

    #[test]
    fn prepared_layout_projection_pins_both_live_scroll_endpoints() {
        let mut layout = super::super::layout_map::EditorLayoutMap::default();
        layout.configure(1_000, 400.0);
        layout.update_line_layout(900, 8, 24.0, 0.0, 0.0);
        let viewport_height = 240.0;
        let live_document_height = 40_000.0;

        let top = source_viewport_for_layout(
            &layout,
            &layout,
            0.0,
            live_document_height,
            viewport_height,
        );
        let bottom = source_viewport_for_layout(
            &layout,
            &layout,
            live_document_height - viewport_height,
            live_document_height,
            viewport_height,
        );

        assert_eq!(top.visible_top, 0.0);
        assert_eq!(top.scroll_ratio, 0.0);
        assert_eq!(bottom.visible_bottom, bottom.total_units);
        assert_eq!(bottom.scroll_ratio, 1.0);
    }

    #[test]
    fn prepared_layout_projection_does_not_reverse_when_live_height_grows() {
        let mut prepared = super::super::layout_map::EditorLayoutMap::default();
        prepared.configure(1_000, 400.0);
        let mut live = prepared.clone();
        let anchor_line = 500;
        let anchor_fraction = 0.25;
        let viewport_height = 240.0;
        let before_scroll =
            live.line_start_y(anchor_line) + anchor_fraction * live.line_height_px(anchor_line);
        let before = source_viewport_for_layout(
            &prepared,
            &live,
            before_scroll,
            live.total_height(),
            viewport_height,
        );

        live.update_line_layout(100, 12, 24.0, 0.0, 0.0);
        live.update_line_layout(300, 8, 24.0, 0.0, 0.0);
        let after_scroll =
            live.line_start_y(anchor_line) + anchor_fraction * live.line_height_px(anchor_line);
        let after = source_viewport_for_layout(
            &prepared,
            &live,
            after_scroll,
            live.total_height(),
            viewport_height,
        );

        assert!((after.visible_top - before.visible_top).abs() < 0.001);
        assert!((after.scroll_ratio - before.scroll_ratio).abs() < 0.001);
    }

    #[test]
    fn retained_raster_stays_on_its_camera_until_the_editor_actually_scrolls() {
        let key = RasterKey {
            generation: 0,
            first_unit: 64,
            rows: 1,
            width: 1,
            scale_x100: 100,
            density: crate::minimap::Density::Compact,
            theme_generation: 0,
        };
        let image = Arc::new(RenderImage::new(SmallVec::from_elem(
            Frame::new(RgbaImage::new(1, 1)),
            1,
        )));
        let cached = CachedRaster {
            key,
            image,
            media: Arc::from([]),
            content_top: 100.0,
            viewport_generation: 7,
            line_height: raster_line_height(crate::minimap::Density::Compact, 2.0),
        };

        assert_eq!(raster_placement_content_top(&cached, 7, 101.0), 100.0);
        assert_eq!(raster_placement_content_top(&cached, 8, 101.0), 101.0);
    }

    #[test]
    fn editor_viewport_geometry_pins_the_thumb_to_both_ends() {
        let density = crate::minimap::Density::Compact;
        let start = viewport_geometry(1_000.0, 0.0, 100.0, 300.0, density);
        let end = viewport_geometry(1_000.0, 900.0, 100.0, 300.0, density);
        assert_eq!(start.thumb_top, 0.0);
        assert!((end.thumb_top - (end.interaction_height - end.thumb_height)).abs() < 0.001);
    }

    #[test]
    fn document_growth_keeps_the_viewport_top_stable_without_editor_scroll() {
        let host = EditorMinimapHost::default();
        let density = crate::minimap::Density::Compact;
        let before = host.stabilize_viewport(
            viewport_geometry(1_000.0, 400.0, 30.0, 300.0, density),
            1_000.0,
            400.0,
            430.0,
            300.0,
            density,
        );
        let after = host.stabilize_viewport(
            viewport_geometry(1_001.0, 400.0, 30.0, 300.0, density),
            1_001.0,
            400.0,
            430.0,
            300.0,
            density,
        );
        assert_eq!(after.content_top, before.content_top);
        assert_eq!(after.thumb_top, before.thumb_top);
    }

    #[test]
    fn document_growth_does_not_jump_when_the_viewport_stops_being_at_the_end() {
        let host = EditorMinimapHost::default();
        let density = crate::minimap::Density::Compact;
        let before = host.stabilize_viewport(
            viewport_geometry_for_range(749.0, 699.0, 749.0, 700.0, density),
            749.0,
            699.0,
            749.0,
            700.0,
            density,
        );
        let after = host.stabilize_viewport(
            viewport_geometry_for_range(754.0, 700.0, 750.0, 700.0, density),
            754.0,
            700.0,
            750.0,
            700.0,
            density,
        );
        assert!((after.content_top - before.content_top - 1.0).abs() < 0.001);
        assert_eq!(after.thumb_top, before.thumb_top);
        assert_eq!(after.thumb_height, before.thumb_height);
    }

    #[test]
    fn actual_editor_scroll_releases_the_stable_camera() {
        let mut host = EditorMinimapHost::default();
        let density = crate::minimap::Density::Compact;
        let before = host.stabilize_viewport(
            viewport_geometry_for_range(1_000.0, 400.0, 430.0, 300.0, density),
            1_000.0,
            400.0,
            430.0,
            300.0,
            density,
        );
        let without_scroll = host.stabilize_viewport(
            viewport_geometry_for_range(1_000.0, 410.0, 440.0, 300.0, density),
            1_000.0,
            410.0,
            440.0,
            300.0,
            density,
        );
        assert_eq!(without_scroll.content_top, before.content_top);

        host.note_viewport_changed();
        let projected = viewport_geometry_for_range(1_000.0, 600.0, 630.0, 300.0, density);
        let after_scroll =
            host.stabilize_viewport(projected, 1_000.0, 600.0, 630.0, 300.0, density);
        assert_eq!(after_scroll.content_top, projected.content_top);
        assert_ne!(after_scroll.content_top, without_scroll.content_top);
    }

    #[test]
    fn reversible_edits_do_not_ratchet_the_camera() {
        let host = EditorMinimapHost::default();
        let density = crate::minimap::Density::Large;
        let original = host.stabilize_viewport(
            viewport_geometry_for_range(745.0, 721.0, 742.0, 691.0, density),
            745.0,
            721.0,
            742.0,
            691.0,
            density,
        );
        for total in 746..=749 {
            let bottom = 742.0 + (total - 745) as f32 * 0.5;
            host.stabilize_viewport(
                viewport_geometry_for_range(total as f32, 721.0, bottom, 691.0, density),
                total as f32,
                721.0,
                bottom,
                691.0,
                density,
            );
        }
        for total in (745..=748).rev() {
            let bottom = 742.0 + (total - 745) as f32 * 0.5;
            host.stabilize_viewport(
                viewport_geometry_for_range(total as f32, 721.0, bottom, 691.0, density),
                total as f32,
                721.0,
                bottom,
                691.0,
                density,
            );
        }
        let restored = host.stabilize_viewport(
            viewport_geometry_for_range(745.0, 721.0, 742.0, 691.0, density),
            745.0,
            721.0,
            742.0,
            691.0,
            density,
        );
        assert_eq!(restored.content_top, original.content_top);
        assert_eq!(restored.thumb_top, original.thumb_top);
        assert_eq!(restored.thumb_height, original.thumb_height);
    }
}
