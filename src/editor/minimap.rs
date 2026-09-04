#[cfg(feature = "benchmarks")]
use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use gpui::{Bounds, Pixels, RenderImage};
use image::{Frame, RgbaImage};
use smallvec::SmallVec;

use crate::document::{ByteRange, Revision, TextSnapshot};

pub(super) const DEFAULT_WIDTH: f32 = 96.0;
pub(super) const MIN_WIDTH: f32 = 56.0;
pub(super) const MAX_WIDTH: f32 = 220.0;
pub(super) const RESIZE_HANDLE: f32 = 6.0;
const RASTER_PREFETCH_ROWS: usize = 64;
const MAX_RASTER_ROWS: usize = 700;
pub(super) const MAX_RICH_SPANS_PER_ROW: usize = 64;
pub(super) const MAX_RICH_SPANS_PER_REQUEST: usize = 4_096;
static RICH_SPAN_DEGRADED_ROWS: AtomicU64 = AtomicU64::new(0);
static NEXT_HOST_ID: AtomicU64 = AtomicU64::new(1);
static PERF_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

fn perf_enabled() -> bool {
    *PERF_ENABLED.get_or_init(|| std::env::var_os("ORG_STUDIO_EDITOR_MINIMAP_PERF").is_some())
}

pub(super) type ViewportGeometry = crate::minimap::ProjectionViewport;

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
        buffer.set_size(
            Some(width as f32 - 6.0 * scale_factor),
            Some(physical_line_height),
        );
        let base_attrs =
            attrs
                .clone()
                .weight(cosmic_weight(source.weight))
                .style(if source.italic {
                    cosmic_text::Style::Italic
                } else {
                    cosmic_text::Style::Normal
                });
        if source.spans.is_empty() {
            buffer.set_text(&source.text, &base_attrs, Shaping::Advanced, None);
        } else {
            let rich_text = rich_text_segments(source, &base_attrs);
            buffer.set_rich_text(rich_text, &base_attrs, Shaping::Advanced, None);
        }
        buffer.shape_until_scroll(font_system, false);
        let base = Color::rgb(
            (source.color >> 16) as u8,
            (source.color >> 8) as u8,
            source.color as u8,
        );
        let origin_x = (source.indent * scale_factor).round() as i32;
        buffer.draw(font_system, swash_cache, base, |x, y, w, h, color| {
            crate::minimap::paint_text_pixels(
                &mut pixels,
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
    let image = RgbaImage::from_raw(width, height, pixels).expect("valid minimap image dimensions");
    Some(RasterizedRows {
        image: Arc::new(RenderImage::new(SmallVec::from_elem(Frame::new(image), 1))),
        rasterizer_lock_wait,
    })
}

fn cosmic_weight(weight: TextWeight) -> cosmic_text::Weight {
    match weight {
        TextWeight::Normal => cosmic_text::Weight::NORMAL,
        TextWeight::Semibold => cosmic_text::Weight::SEMIBOLD,
        TextWeight::Bold => cosmic_text::Weight::BOLD,
    }
}

fn rich_text_segments<'a>(
    row: &'a TextRow,
    base: &cosmic_text::Attrs<'static>,
) -> Vec<(&'a str, cosmic_text::Attrs<'static>)> {
    use cosmic_text::{Color, Style, UnderlineStyle};

    let mut boundaries = vec![0, row.text.len()];
    for span in &row.spans {
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
            for span in row
                .spans
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
            Some((&row.text[range], attrs))
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
    let requested = (visible + RASTER_PREFETCH_ROWS * 2).min(MAX_RASTER_ROWS);
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
    image_paint_frames: AtomicU64,
    #[cfg(feature = "benchmarks")]
    visible_image_paint_frames: AtomicU64,
    #[cfg(feature = "benchmarks")]
    jobs: std::sync::Mutex<VecDeque<MinimapJobSample>>,
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
            image_paint_frames: AtomicU64::new(0),
            #[cfg(feature = "benchmarks")]
            visible_image_paint_frames: AtomicU64::new(0),
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
            "org_editor_minimap_summary host_id={} jobs={} rows={} visible_rows={} prefetch_rows={} repeated_rows={} degraded_rows={} prepare_p50_ms={:.3} prepare_p95_ms={:.3} rasterizer_lock_wait_p50_ms={:.3} rasterizer_lock_wait_p95_ms={:.3} raster_p50_ms={:.3} raster_p95_ms={:.3} cache_hits={} cache_misses={} stale_cancels={} publishes={} image_paint_frames={} visible_image_paint_frames={}",
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
    }
}

pub(super) struct EditorMinimapHost {
    pub(super) visible: bool,
    pub(super) width: f32,
    pub(super) bounds: Option<Bounds<Pixels>>,
    pub(super) drag: Option<crate::minimap::DragSession>,
    pub(super) resizing: Option<(f32, f32)>,
    pub(super) generation: u64,
    pub(super) revision: Option<Revision>,
    pub(super) search_marks: Arc<[ByteRange]>,
    pub(super) raster: std::sync::Mutex<Option<CachedRaster>>,
    pub(super) raster_build: std::sync::Mutex<Option<RasterKey>>,
    pub(super) raster_epoch: Arc<AtomicU64>,
    pub(super) scale_factor: f32,
    viewport_generation: u64,
    viewport_anchor: std::sync::Mutex<Option<ViewportAnchor>>,
    pub(super) telemetry: Arc<EditorMinimapTelemetry>,
}

impl Default for EditorMinimapHost {
    fn default() -> Self {
        Self {
            visible: true,
            width: DEFAULT_WIDTH,
            bounds: None,
            drag: None,
            resizing: None,
            generation: 0,
            revision: None,
            search_marks: Arc::from([]),
            raster: std::sync::Mutex::new(None),
            raster_build: std::sync::Mutex::new(None),
            raster_epoch: Arc::new(AtomicU64::new(0)),
            scale_factor: 1.0,
            viewport_generation: 0,
            viewport_anchor: std::sync::Mutex::new(None),
            telemetry: Arc::new(EditorMinimapTelemetry::new()),
        }
    }
}

impl EditorMinimapHost {
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
        let can_reuse = anchor.is_some_and(|anchor| {
            anchor.viewport_generation == self.viewport_generation
                && (anchor.track_height - track_height).abs() < 0.5
                && anchor.density == density
        });
        let preferred_content_top = if can_reuse {
            let previous = (*anchor).expect("reuse requires an anchor");
            viewport = crate::minimap::stabilize_projection_camera_with_line_height(
                viewport,
                total_units,
                visible_top,
                visible_bottom,
                density,
                self.line_height(density),
                previous.content_top,
            );
            previous.content_top
        } else {
            viewport.content_top
        };
        *anchor = Some(ViewportAnchor {
            viewport_generation: self.viewport_generation,
            track_height,
            density,
            content_top: preferred_content_top,
        });
        viewport
    }

    pub(super) fn note_viewport_changed(&mut self) {
        self.viewport_generation = self.viewport_generation.wrapping_add(1);
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
        if in_flight.is_some() {
            false
        } else {
            *in_flight = Some(key);
            true
        }
    }

    pub(super) fn invalidate_raster(&mut self) {
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

    fn semantic_span(byte: usize) -> super::super::syntax::EditorSemanticSpan {
        super::super::syntax::EditorSemanticSpan {
            bytes: byte..byte + 1,
            color: Some(super::super::syntax::EditorColorToken::Keyword),
            weight: super::super::syntax::EditorSemanticWeight::Bold,
            italic: false,
            underline: false,
            strikethrough: false,
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
    fn raster_builds_are_serialized_instead_of_replaced_during_scroll() {
        let host = EditorMinimapHost::default();
        let first = RasterKey {
            generation: 1,
            first_unit: 0,
            rows: 200,
            width: 96,
            scale_x100: 200,
            density: crate::minimap::Density::Compact,
        };
        let next = RasterKey {
            first_unit: 64,
            ..first
        };
        assert!(host.reserve_raster(first));
        assert!(!host.reserve_raster(next));
        assert_eq!(
            *host
                .raster_build
                .lock()
                .expect("editor minimap raster build poisoned"),
            Some(first)
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
        assert!(first.0 as f32 + first.1 as f32 > 100.0);
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
        };
        let base = cosmic_text::Attrs::new().family(cosmic_text::Family::Monospace);
        let segments = rich_text_segments(&row, &base);

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
        }));
        host.invalidate_raster();
        assert_ne!(host.raster_epoch.load(Ordering::Acquire), epoch);
        assert_eq!(host.telemetry.stale_cancels.load(Ordering::Relaxed), 1);
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
        };
        let image = Arc::new(RenderImage::new(SmallVec::from_elem(
            Frame::new(RgbaImage::new(1, 1)),
            1,
        )));
        *host.raster.lock().expect("editor minimap raster poisoned") = Some(CachedRaster {
            key,
            image,
            media: Arc::from([]),
            content_top: 0.0,
            viewport_generation: 0,
            line_height: raster_line_height(crate::minimap::Density::Compact, 1.0),
        });

        host.invalidate_raster();

        assert!(
            host.raster
                .lock()
                .expect("editor minimap raster poisoned")
                .is_some()
        );
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
