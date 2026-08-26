use std::sync::OnceLock;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::{Hash, Hasher},
    ops::Range,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[cfg(test)]
use gpui::FontFallbacks;
use gpui::{
    BorderStyle, Bounds, Corners, CursorStyle, DispatchPhase, FontWeight, ListOffset, ListState,
    MouseButton, MouseMoveEvent, MouseUpEvent, RenderImage, SharedString, TextRun, canvas, div,
    fill, font, outline, point, prelude::*, px, rgba,
};
use image::{Frame, RgbaImage};
use smallvec::SmallVec;
#[cfg(test)]
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    document::SharedTextSnapshot,
    org_syntax::{
        BlockArena, BlockKind,
        inline::{InlineKind, InlineSpan},
    },
    theme::current_theme,
};

use super::code_highlight_style;
use super::{
    CodeHighlightSpan, DocumentFormat, PreviewDocument, PreviewRow, highlight_code,
    markdown::{MarkdownBlock, MarkdownKind},
    parse_document_inline,
    table::TableRowStyle,
};

const MINIMAP_FONT_PX: f32 = 2.0;
const MINIMAP_LINE_HEIGHT_PX: f32 = 2.6;
const MIN_THUMB_PX: f32 = 24.0;
const MINIMAP_EDGE_PADDING_PX: f32 = 4.0;
const SCROLL_WHEEL_LINE_PX: f32 = 20.0;
const MINIMAP_MANUAL_MIN_PX: f32 = 48.0;
pub(super) const MINIMAP_MANUAL_MAX_PX: f32 = 480.0;
const MINIMAP_COMPACT_WINDOW_MAX_FRACTION: f32 = 0.20;
const MINIMAP_LARGE_WINDOW_MAX_FRACTION: f32 = 0.30;
const MINIMAP_LARGE_WINDOW_TRANSITION_END_PX: f32 = 1920.0;
const MINIMAP_AUTO_COMPACT_MAX_PX: f32 = 96.0;
const MINIMAP_AUTO_GROW_START_PX: f32 = 1280.0;
const MINIMAP_AUTO_GROW_PER_PX: f32 = 0.11;
const MINIMAP_AUTO_MAX_PX: f32 = 220.0;
const MINIMAP_RESIZE_HANDLE_PX: f32 = 6.0;
// Leave the overwhelming majority of a 120Hz frame (8.333ms) to GPUI layout,
// paint and presentation. Projection refinement is cooperative and can take as
// many frames as necessary because an estimated projection is available first.
const MINIMAP_INDEX_FRAME_BUDGET: Duration = Duration::from_micros(350);
#[cfg(test)]
const PREVIEW_BASE_ROW_PX: f32 = 24.0;
const RASTER_TILE_ROWS: usize = 128;
const RASTER_TILE_CACHE_CAPACITY: usize = 6;
const PROJECTION_CHUNK_ROWS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MinimapDensity {
    Compact,
    Comfortable,
    Large,
    ExtraLarge,
    Maximum,
}

impl MinimapDensity {
    fn for_width(width: f32) -> Self {
        if width < 128.0 {
            Self::Compact
        } else if width < 184.0 {
            Self::Comfortable
        } else if width < 280.0 {
            Self::Large
        } else if width < 400.0 {
            Self::ExtraLarge
        } else {
            Self::Maximum
        }
    }

    fn font_px(self) -> f32 {
        match self {
            Self::Compact => MINIMAP_FONT_PX,
            Self::Comfortable => 2.4,
            Self::Large => 2.8,
            Self::ExtraLarge => 3.2,
            Self::Maximum => 3.6,
        }
    }

    fn line_height(self) -> f32 {
        match self {
            Self::Compact => MINIMAP_LINE_HEIGHT_PX,
            Self::Comfortable => 3.2,
            Self::Large => 3.8,
            Self::ExtraLarge => 4.4,
            Self::Maximum => 5.0,
        }
    }

    fn edge_padding(self) -> f32 {
        match self {
            Self::Compact => MINIMAP_EDGE_PADDING_PX,
            Self::Comfortable => 5.0,
            Self::Large => 6.0,
            Self::ExtraLarge => 7.0,
            Self::Maximum => 8.0,
        }
    }
}

fn minimap_trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("ORG_STUDIO_MINIMAP_TRACE").is_some())
}

pub(super) fn minimap_perf_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("ORG_STUDIO_MINIMAP_PERF").is_some() || minimap_trace_enabled()
    })
}

pub(super) fn automatic_width_for_viewport(viewport_width: f32) -> f32 {
    let compact = (viewport_width * 0.15).clamp(24.0, MINIMAP_AUTO_COMPACT_MAX_PX);
    if viewport_width <= MINIMAP_AUTO_GROW_START_PX {
        compact
    } else {
        (MINIMAP_AUTO_COMPACT_MAX_PX
            + (viewport_width - MINIMAP_AUTO_GROW_START_PX) * MINIMAP_AUTO_GROW_PER_PX)
            .min(MINIMAP_AUTO_MAX_PX)
    }
}

pub(super) fn width_for_viewport(viewport_width: f32, preferred: Option<u16>) -> f32 {
    preferred.map_or_else(
        || automatic_width_for_viewport(viewport_width),
        |width| manual_width_for_viewport(viewport_width, f32::from(width)),
    )
}

fn manual_window_max(viewport_width: f32) -> f32 {
    let large_window_progress = ((viewport_width - MINIMAP_AUTO_GROW_START_PX)
        / (MINIMAP_LARGE_WINDOW_TRANSITION_END_PX - MINIMAP_AUTO_GROW_START_PX))
        .clamp(0.0, 1.0);
    let fraction = MINIMAP_COMPACT_WINDOW_MAX_FRACTION
        + (MINIMAP_LARGE_WINDOW_MAX_FRACTION - MINIMAP_COMPACT_WINDOW_MAX_FRACTION)
            * large_window_progress;
    (viewport_width * fraction).clamp(24.0, MINIMAP_MANUAL_MAX_PX)
}

fn manual_width_for_viewport(viewport_width: f32, desired: f32) -> f32 {
    let window_max = manual_window_max(viewport_width);
    desired.clamp(MINIMAP_MANUAL_MIN_PX.min(window_max), window_max)
}

fn width_from_resize_drag(
    viewport_width: f32,
    session: MinimapResizeSession,
    pointer_x: f32,
) -> f32 {
    manual_width_for_viewport(
        viewport_width,
        session.start_width + session.start_pointer_x - pointer_x,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreviewLineKind {
    Blank,
    Text,
    Heading(u8),
    List,
    Quote,
    Code,
    Table,
    Image,
    Rule,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RowLayout {
    pub(super) font_size: f32,
    pub(super) line_height: f32,
    pub(super) min_height: f32,
    pub(super) padding_left: f32,
    pub(super) padding_right: f32,
    pub(super) padding_top: f32,
    pub(super) padding_bottom: f32,
    pub(super) margin_top: f32,
    pub(super) margin_bottom: f32,
    pub(super) fixed_height: Option<f32>,
}

impl RowLayout {
    const fn text(font_size: f32, line_height: f32) -> Self {
        Self {
            font_size,
            line_height,
            min_height: 24.0,
            padding_left: 0.0,
            padding_right: 0.0,
            padding_top: 0.0,
            padding_bottom: 0.0,
            margin_top: 0.0,
            margin_bottom: 0.0,
            fixed_height: None,
        }
    }

    const fn blank() -> Self {
        Self {
            fixed_height: Some(24.0),
            ..Self::text(14.0, 24.0)
        }
    }

    const fn image() -> Self {
        Self {
            padding_top: 8.0,
            padding_bottom: 8.0,
            ..Self::text(14.0, 24.0)
        }
    }

    const fn rule() -> Self {
        Self {
            margin_top: 20.0,
            margin_bottom: 20.0,
            fixed_height: Some(1.0),
            ..Self::text(14.0, 24.0)
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct DisplayRuns {
    pub(super) text: SharedString,
    pub(super) inline_spans: Arc<[InlineSpan]>,
    pub(super) code_spans: Arc<[CodeHighlightSpan]>,
}

pub(super) struct PreviewDisplayMap {
    text: SharedTextSnapshot,
    format: DocumentFormat,
    blocks: Arc<BlockArena>,
    markdown_blocks: Arc<Vec<MarkdownBlock>>,
    rows: Arc<Vec<PreviewRow>>,
    tables: Arc<HashMap<u32, TableRowStyle>>,
    image_sizes: Arc<HashMap<u32, (u32, u32)>>,
    display_runs: Mutex<DisplayRunCache>,
    display_lines: Mutex<DisplayLineCache>,
    raster_tiles: Mutex<RasterTileCache>,
    minimap_line_index: Mutex<Option<CachedMinimapLineIndex>>,
    minimap_line_index_build: Mutex<Option<MinimapLineIndexBuilder>>,
    minimap_drag: Arc<Mutex<Option<MinimapDragSession>>>,
    minimap_resize_drag: Arc<Mutex<Option<MinimapResizeSession>>>,
    minimap_interaction_anchor: Arc<Mutex<Option<MinimapInteractionAnchor>>>,
    initial_visible_batch_ready: AtomicBool,
    perf: MinimapPerfState,
}

struct MinimapPerfState {
    first_tile_completed: AtomicBool,
    first_pixels_painted: AtomicBool,
}

impl MinimapPerfState {
    fn new() -> Self {
        Self {
            first_tile_completed: AtomicBool::new(false),
            first_pixels_painted: AtomicBool::new(false),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MinimapDragSession {
    /// Distance from the top of the thumb to the pointer at mouse-down.
    /// Keeping this fixed gives the minimap standard scrollbar grab semantics.
    grab_offset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MinimapResizeSession {
    start_pointer_x: f32,
    start_width: f32,
}

fn current_resize_session(
    state: &Mutex<Option<MinimapResizeSession>>,
) -> Option<MinimapResizeSession> {
    *state.lock().expect("minimap resize state poisoned")
}

fn take_resize_session(
    state: &Mutex<Option<MinimapResizeSession>>,
) -> Option<MinimapResizeSession> {
    state.lock().expect("minimap resize state poisoned").take()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum MinimapWidthChange {
    Preview(f32),
    Commit(f32),
    Reset,
}

#[derive(Clone)]
struct MinimapLineIndex {
    width: u16,
    rows_signature: u64,
    density: MinimapDensity,
    projection: Arc<ProjectionSnapshot>,
    total: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ProjectionMeasure {
    display_lines: u32,
    pixels: f32,
    exact: bool,
}

impl ProjectionMeasure {
    fn new(display_lines: usize, pixels: f32, exact: bool) -> Self {
        Self {
            display_lines: display_lines.max(1).min(u32::MAX as usize) as u32,
            pixels: pixels.max(0.0),
            exact,
        }
    }
}

#[derive(Clone)]
struct ProjectionChunk {
    measures: Arc<[ProjectionMeasure]>,
    display_lines: usize,
    pixels: f32,
    exact_rows: usize,
}

impl ProjectionChunk {
    fn new(measures: Vec<ProjectionMeasure>) -> Self {
        let display_lines = measures
            .iter()
            .map(|measure| measure.display_lines as usize)
            .sum();
        let pixels = measures.iter().map(|measure| measure.pixels).sum();
        let exact_rows = measures.iter().filter(|measure| measure.exact).count();
        Self {
            measures: measures.into(),
            display_lines,
            pixels,
            exact_rows,
        }
    }
}

#[derive(Clone)]
struct ProjectionSnapshot {
    chunks: Arc<[Arc<ProjectionChunk>]>,
    display_prefix: Arc<[usize]>,
    pixel_prefix: Arc<[f32]>,
    rows: usize,
    exact_rows: usize,
}

impl ProjectionSnapshot {
    fn new(measures: Vec<ProjectionMeasure>) -> Self {
        let chunks = measures
            .chunks(PROJECTION_CHUNK_ROWS)
            .map(|chunk| Arc::new(ProjectionChunk::new(chunk.to_vec())))
            .collect::<Vec<_>>();
        Self::from_chunks(chunks)
    }

    fn from_chunks(chunks: Vec<Arc<ProjectionChunk>>) -> Self {
        let mut display_prefix = Vec::with_capacity(chunks.len() + 1);
        let mut pixel_prefix = Vec::with_capacity(chunks.len() + 1);
        display_prefix.push(0usize);
        pixel_prefix.push(0.0f32);
        let mut rows = 0usize;
        let mut exact_rows = 0usize;
        for chunk in &chunks {
            display_prefix.push(
                display_prefix
                    .last()
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(chunk.display_lines),
            );
            pixel_prefix.push(pixel_prefix.last().copied().unwrap_or(0.0) + chunk.pixels);
            rows += chunk.measures.len();
            exact_rows += chunk.exact_rows;
        }
        Self {
            chunks: chunks.into(),
            display_prefix: display_prefix.into(),
            pixel_prefix: pixel_prefix.into(),
            rows,
            exact_rows,
        }
    }

    fn replacing(&self, updates: &[(usize, ProjectionMeasure)]) -> Self {
        if updates.is_empty() {
            return self.clone();
        }
        let mut chunks = self.chunks.to_vec();
        let mut cursor = 0usize;
        while cursor < updates.len() {
            let chunk_index = updates[cursor].0 / PROJECTION_CHUNK_ROWS;
            if chunk_index >= chunks.len() {
                break;
            }
            let mut measures = chunks[chunk_index].measures.to_vec();
            while cursor < updates.len() && updates[cursor].0 / PROJECTION_CHUNK_ROWS == chunk_index
            {
                let local = updates[cursor].0 % PROJECTION_CHUNK_ROWS;
                if local < measures.len() {
                    measures[local] = updates[cursor].1;
                }
                cursor += 1;
            }
            chunks[chunk_index] = Arc::new(ProjectionChunk::new(measures));
        }
        Self::from_chunks(chunks)
    }

    fn total_display_lines(&self) -> usize {
        self.display_prefix.last().copied().unwrap_or(0)
    }

    fn total_pixels(&self) -> f32 {
        self.pixel_prefix.last().copied().unwrap_or(0.0)
    }

    fn estimated_heap_bytes(&self) -> usize {
        self.rows * std::mem::size_of::<ProjectionMeasure>()
            + self.chunks.len() * std::mem::size_of::<Arc<ProjectionChunk>>()
            + self.display_prefix.len() * std::mem::size_of::<usize>()
            + self.pixel_prefix.len() * std::mem::size_of::<f32>()
    }

    fn prefix_for_row(&self, row: usize) -> (usize, f32) {
        if self.rows == 0 {
            return (0, 0.0);
        }
        let row = row.min(self.rows);
        let chunk_index = (row / PROJECTION_CHUNK_ROWS).min(self.chunks.len());
        let mut display = self.display_prefix[chunk_index];
        let mut pixels = self.pixel_prefix[chunk_index];
        if chunk_index < self.chunks.len() {
            let local_end = row % PROJECTION_CHUNK_ROWS;
            for measure in self.chunks[chunk_index].measures.iter().take(local_end) {
                display = display.saturating_add(measure.display_lines as usize);
                pixels += measure.pixels;
            }
        }
        (display, pixels)
    }

    fn measure(&self, row: usize) -> ProjectionMeasure {
        let chunk = row / PROJECTION_CHUNK_ROWS;
        let local = row % PROJECTION_CHUNK_ROWS;
        self.chunks[chunk].measures[local]
    }

    fn locate_display(&self, display_line: usize) -> (usize, usize) {
        if self.rows == 0 {
            return (0, 0);
        }
        let chunk_index = self
            .display_prefix
            .partition_point(|&start| start <= display_line)
            .saturating_sub(1)
            .min(self.chunks.len() - 1);
        let mut remaining = display_line.saturating_sub(self.display_prefix[chunk_index]);
        for (local, measure) in self.chunks[chunk_index].measures.iter().enumerate() {
            let count = measure.display_lines as usize;
            if remaining < count {
                return (chunk_index * PROJECTION_CHUNK_ROWS + local, remaining);
            }
            remaining = remaining.saturating_sub(count);
        }
        (
            self.rows - 1,
            self.measure(self.rows - 1).display_lines as usize,
        )
    }

    fn locate_pixel(&self, pixel: f32) -> (usize, f32) {
        if self.rows == 0 {
            return (0, 0.0);
        }
        let pixel = pixel.clamp(0.0, self.total_pixels());
        let chunk_index = self
            .pixel_prefix
            .partition_point(|&start| start <= pixel)
            .saturating_sub(1)
            .min(self.chunks.len() - 1);
        let mut remaining = (pixel - self.pixel_prefix[chunk_index]).max(0.0);
        for (local, measure) in self.chunks[chunk_index].measures.iter().enumerate() {
            if remaining < measure.pixels || measure.pixels <= f32::EPSILON {
                return (chunk_index * PROJECTION_CHUNK_ROWS + local, remaining);
            }
            remaining -= measure.pixels;
        }
        let last = self.rows - 1;
        (last, self.measure(last).pixels)
    }
}

#[derive(Clone)]
struct CachedMinimapLineIndex {
    presentation_rows: Arc<Vec<usize>>,
    index: MinimapLineIndex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MinimapLineIndexKey {
    presentation_rows: usize,
    width: u16,
    density: MinimapDensity,
}

impl MinimapLineIndexKey {
    fn new(
        presentation_rows: &Arc<Vec<usize>>,
        available_width: f32,
        density: MinimapDensity,
    ) -> Self {
        Self {
            presentation_rows: Arc::as_ptr(presentation_rows) as usize,
            width: available_width.round().clamp(1.0, u16::MAX as f32) as u16,
            density,
        }
    }
}

struct MinimapLineIndexBuilder {
    key: MinimapLineIndexKey,
    presentation_rows: Arc<Vec<usize>>,
    rows_signature: u64,
    sequential_cursor: usize,
    priority_range: Range<usize>,
    priority_cursor: usize,
    exact_bits: Vec<u64>,
    exact_rows: usize,
    started_at: Instant,
    projection: Arc<ProjectionSnapshot>,
    pending_updates: Vec<(usize, ProjectionMeasure)>,
    slices: usize,
    work: Duration,
    max_slice: Duration,
}

impl MinimapLineIndexBuilder {
    fn new(
        model: &PreviewDisplayMap,
        key: MinimapLineIndexKey,
        presentation_rows: Arc<Vec<usize>>,
        available_width: f32,
        density: MinimapDensity,
    ) -> MinimapLineIndexBuilder {
        let started_at = Instant::now();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        presentation_rows.hash(&mut hasher);
        let rows_signature = hasher.finish();
        let estimated_index = model.estimated_minimap_line_index(
            &presentation_rows,
            key.width,
            rows_signature,
            available_width,
            density,
        );
        Self {
            key,
            projection: estimated_index.projection,
            presentation_rows,
            rows_signature,
            sequential_cursor: 0,
            priority_range: 0..0,
            priority_cursor: 0,
            exact_bits: vec![0; key.presentation_rows.div_ceil(64)],
            exact_rows: 0,
            started_at,
            pending_updates: Vec::with_capacity(16),
            slices: 0,
            work: Duration::ZERO,
            max_slice: Duration::ZERO,
        }
    }

    fn record_slice(&mut self, elapsed: Duration) {
        self.slices += 1;
        self.work += elapsed;
        self.max_slice = self.max_slice.max(elapsed);
    }

    fn publish_pending(&mut self) {
        if self.pending_updates.is_empty() {
            return;
        }
        self.projection = Arc::new(self.projection.replacing(&self.pending_updates));
        self.pending_updates.clear();
    }

    fn index(&self, density: MinimapDensity) -> MinimapLineIndex {
        MinimapLineIndex {
            width: self.key.width,
            rows_signature: self.rows_signature,
            density,
            total: self.projection.total_display_lines(),
            projection: self.projection.clone(),
        }
    }

    fn is_exact(&self, row: usize) -> bool {
        self.exact_bits
            .get(row / 64)
            .is_some_and(|bits| bits & (1u64 << (row % 64)) != 0)
    }

    fn mark_exact(&mut self, row: usize) {
        let bit = 1u64 << (row % 64);
        let word = &mut self.exact_bits[row / 64];
        if *word & bit == 0 {
            *word |= bit;
            self.exact_rows += 1;
        }
    }

    fn prioritize(&mut self, center: usize) {
        if self.presentation_rows.is_empty() {
            return;
        }
        let center = center.min(self.presentation_rows.len() - 1);
        if self.priority_range.contains(&center) {
            return;
        }
        let start = center.saturating_sub(RASTER_TILE_ROWS);
        let end = (center + RASTER_TILE_ROWS * 2).min(self.presentation_rows.len());
        self.priority_range = start..end;
        self.priority_cursor = start;
    }

    fn next_candidate(&mut self) -> Option<usize> {
        while self.priority_cursor < self.priority_range.end {
            let row = self.priority_cursor;
            self.priority_cursor += 1;
            if !self.is_exact(row) {
                return Some(row);
            }
        }
        while self.sequential_cursor < self.presentation_rows.len() {
            let row = self.sequential_cursor;
            self.sequential_cursor += 1;
            if !self.is_exact(row) {
                return Some(row);
            }
        }
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MinimapProjectionReadiness {
    Estimated,
    PartiallyExact,
    Exact,
}

struct MinimapLineIndexProgress {
    index: MinimapLineIndex,
    readiness: MinimapProjectionReadiness,
    exact_rows: usize,
}

impl MinimapLineIndex {
    fn locate(&self, display_line: usize) -> (usize, usize) {
        self.projection.locate_display(display_line)
    }

    fn pixel_for_list_offset(&self, offset: ListOffset) -> f32 {
        if self.projection.rows == 0 {
            return 0.0;
        }
        let row = offset.item_ix.min(self.projection.rows - 1);
        let (_, pixel_start) = self.projection.prefix_for_row(row);
        let row_height = self.projection.measure(row).pixels;
        (pixel_start + f32::from(offset.offset_in_item).clamp(0.0, row_height))
            .clamp(0.0, self.projection.total_pixels())
    }

    fn list_offset_for_display_position(&self, position: f32) -> ListOffset {
        if self.projection.rows == 0 {
            return ListOffset::default();
        }
        let position = position.clamp(0.0, self.total as f32);
        let (row, _) = self.locate(position.floor() as usize);
        let (display_start, _) = self.projection.prefix_for_row(row);
        let measure = self.projection.measure(row);
        let display_count = measure.display_lines.max(1) as f32;
        let pixel_height = measure.pixels;
        ListOffset {
            item_ix: row,
            offset_in_item: px(
                ((position - display_start as f32) / display_count).clamp(0.0, 1.0) * pixel_height,
            ),
        }
    }

    fn display_position_for_pixel(&self, pixel: f32) -> f32 {
        if self.projection.rows == 0 {
            return 0.0;
        }
        let document_pixels = self.projection.total_pixels();
        let pixel = pixel.clamp(0.0, document_pixels);
        let (row, pixel_in_row) = self.projection.locate_pixel(pixel);
        let (display_start, _) = self.projection.prefix_for_row(row);
        let measure = self.projection.measure(row);
        display_start as f32
            + (pixel_in_row / measure.pixels.max(1.0)).clamp(0.0, 1.0)
                * measure.display_lines.max(1) as f32
    }

    fn list_offset_for_pixel(&self, pixel: f32) -> ListOffset {
        if self.projection.rows == 0 {
            return ListOffset::default();
        }
        let (row, pixel_in_row) = self
            .projection
            .locate_pixel(pixel.clamp(0.0, self.projection.total_pixels()));
        ListOffset {
            item_ix: row,
            offset_in_item: px(pixel_in_row),
        }
    }

    fn document_pixels(&self) -> f32 {
        self.projection.total_pixels()
    }
}

impl PreviewDisplayMap {
    pub(super) fn cancel_minimap_interaction(&self) -> bool {
        let was_dragging = self
            .minimap_drag
            .lock()
            .expect("minimap drag state poisoned")
            .take()
            .is_some();
        let was_resizing = self
            .minimap_resize_drag
            .lock()
            .expect("minimap resize state poisoned")
            .take()
            .is_some();
        self.minimap_interaction_anchor
            .lock()
            .expect("minimap anchor poisoned")
            .take();
        was_dragging || was_resizing
    }
}

#[derive(Clone, Debug, PartialEq)]
struct DisplayLines {
    ranges: Arc<[Range<usize>]>,
    parent_height: f32,
}

struct DisplayLineCache {
    entries: HashMap<(usize, u16), DisplayLines>,
    order: VecDeque<(usize, u16)>,
}

impl DisplayLineCache {
    const CAPACITY: usize = 4096;

    fn insert(&mut self, key: (usize, u16), lines: DisplayLines) {
        if self.entries.contains_key(&key) {
            return;
        }
        while self.entries.len() >= Self::CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.order.push_back(key);
        self.entries.insert(key, lines);
    }
}

struct DisplayRunCache {
    entries: HashMap<usize, DisplayRuns>,
    order: VecDeque<usize>,
}

impl DisplayRunCache {
    const CAPACITY: usize = 4096;

    fn insert(&mut self, index: usize, runs: DisplayRuns) {
        if self.entries.contains_key(&index) {
            return;
        }
        while self.entries.len() >= Self::CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.order.push_back(index);
        self.entries.insert(index, runs);
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct RasterTileKey {
    first_row: usize,
    row_signature: u64,
    width: u16,
    theme_signature: u64,
    folded_signature: u64,
    wrap_signature: u64,
    scale_factor_x100: u16,
    density: MinimapDensity,
}

struct RasterTileCache {
    entries: HashMap<RasterTileKey, Arc<RenderImage>>,
    order: VecDeque<RasterTileKey>,
    in_flight: HashSet<RasterTileKey>,
}

impl RasterTileCache {
    const CAPACITY: usize = RASTER_TILE_CACHE_CAPACITY;

    fn image_or_fallback(&self, key: RasterTileKey) -> (Option<Arc<RenderImage>>, bool) {
        if let Some(image) = self.entries.get(&key).cloned() {
            return (Some(image), false);
        }
        let fallback = self.order.iter().rev().find_map(|candidate| {
            (candidate.first_row == key.first_row
                && candidate.row_signature == key.row_signature
                && candidate.theme_signature == key.theme_signature
                && candidate.folded_signature == key.folded_signature
                && candidate.scale_factor_x100 == key.scale_factor_x100)
                .then(|| self.entries.get(candidate).cloned())
                .flatten()
        });
        (fallback, true)
    }

    fn reserve(&mut self, keys: &[RasterTileKey]) -> bool {
        if keys.is_empty() || !self.in_flight.is_empty() {
            return false;
        }
        for &key in keys {
            if !self.entries.contains_key(&key) {
                self.in_flight.insert(key);
            }
        }
        !self.in_flight.is_empty()
    }

    fn insert_batch(
        &mut self,
        tiles: Vec<(RasterTileKey, Arc<RenderImage>)>,
        visible_keys: &[RasterTileKey],
    ) {
        let new_entries = tiles
            .iter()
            .filter(|(key, _)| !self.entries.contains_key(key))
            .count();
        // A complete visible frame is the atomic cache unit. Very tall windows may need more
        // than the normal six retained tiles, so allow this batch itself to define the temporary
        // capacity while evicting older, non-visible entries before publication.
        let target_capacity = Self::CAPACITY.max(visible_keys.len());
        while self.entries.len() + new_entries > target_capacity {
            let Some(position) = self
                .order
                .iter()
                .position(|candidate| !visible_keys.contains(candidate))
            else {
                break;
            };
            let oldest = self
                .order
                .remove(position)
                .expect("cache position is valid");
            self.entries.remove(&oldest);
        }
        for (key, image) in tiles {
            self.in_flight.remove(&key);
            if self.entries.contains_key(&key) {
                continue;
            }
            self.order.push_back(key);
            self.entries.insert(key, image);
        }
    }
}

#[derive(Clone)]
struct RasterTilePaint {
    image: Arc<RenderImage>,
    y: f32,
    width: f32,
    height: f32,
}

struct RasterizedTile {
    image: Arc<RenderImage>,
    line_count: usize,
    total: Duration,
    text_system_wait: Duration,
    cold_text_system: bool,
}

struct RasterTileRequest {
    key: RasterTileKey,
    rows: Vec<RasterRow>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct MinimapHitRow {
    top: f32,
    bottom: f32,
    presentation_index: usize,
    offset_in_item: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct MinimapViewport {
    content_top: f32,
    thumb: ThumbGeometry,
    scroll_ratio: f32,
    interaction_height: f32,
}

#[derive(Clone, Copy, Debug)]
struct MinimapInteractionAnchor {
    width: u16,
    rows_signature: u64,
    interaction_height: f32,
    scroll_ratio: f32,
    content_top: f32,
    thumb_top: f32,
}

impl MinimapInteractionAnchor {
    fn matches(self, index: &MinimapLineIndex, viewport: MinimapViewport) -> bool {
        self.width == index.width
            && self.rows_signature == index.rows_signature
            && (self.interaction_height - viewport.interaction_height).abs() < 0.5
            && (self.scroll_ratio - viewport.scroll_ratio).abs() < 0.001
    }
}

fn minimap_projection_height(
    total_lines: usize,
    track_height: f32,
    density: MinimapDensity,
) -> f32 {
    (total_lines as f32 * density.line_height() + density.edge_padding() * 2.0)
        .min(track_height)
        .max(0.0)
}

fn minimap_visible_display_range(
    index: &MinimapLineIndex,
    scroll_pixels: f32,
    viewport_pixels: f32,
) -> (f32, f32) {
    let document_pixels = index.document_pixels();
    let top = index.display_position_for_pixel(scroll_pixels);
    let bottom = index
        .display_position_for_pixel((scroll_pixels + viewport_pixels).clamp(0.0, document_pixels));
    (top, bottom.max(top))
}

fn minimap_thumb_height_for_scroll(
    index: &MinimapLineIndex,
    scroll_pixels: f32,
    viewport_pixels: f32,
    interaction_height: f32,
) -> f32 {
    let document_pixels = index.document_pixels();
    if document_pixels <= viewport_pixels || document_pixels <= f32::EPSILON {
        return interaction_height;
    }
    let (top, bottom) = minimap_visible_display_range(index, scroll_pixels, viewport_pixels);
    ((bottom - top) * index.density.line_height())
        .max(MIN_THUMB_PX)
        .min(interaction_height)
}

#[cfg(test)]
fn minimap_viewport(
    metrics: ScrollMetrics,
    total_lines: usize,
    track_height: f32,
) -> MinimapViewport {
    if total_lines == 0 || track_height <= 0.0 {
        return MinimapViewport::default();
    }
    let total = total_lines as f32;
    let visible_editor_lines = (metrics.viewport / PREVIEW_BASE_ROW_PX).max(1.0).min(total);
    let visible_minimap_lines = (track_height / MINIMAP_LINE_HEIGHT_PX).max(1.0);
    let progress = if metrics.max_offset > 0.0 {
        (metrics.offset / metrics.max_offset).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let editor_scroll_top = progress * (total - visible_editor_lines).max(0.0);
    let content_top = progress * (total - visible_minimap_lines).max(0.0);
    let height = (visible_editor_lines * MINIMAP_LINE_HEIGHT_PX)
        .max(MIN_THUMB_PX)
        .min(track_height);
    let top = ((editor_scroll_top - content_top) * MINIMAP_LINE_HEIGHT_PX)
        .clamp(0.0, (track_height - height).max(0.0));
    MinimapViewport {
        content_top,
        thumb: ThumbGeometry { top, height },
        scroll_ratio: progress,
        interaction_height: track_height,
    }
}

fn minimap_viewport_for_list(
    index: &MinimapLineIndex,
    list_state: &ListState,
    track_height: f32,
) -> MinimapViewport {
    if index.total == 0 || track_height <= 0.0 {
        return MinimapViewport::default();
    }
    let interaction_height = minimap_projection_height(index.total, track_height, index.density);
    let viewport_pixels = f32::from(list_state.viewport_bounds().size.height).max(0.0);
    let total = index.total as f32;
    let document_pixels = index.document_pixels();
    let max_scroll_pixels = (document_pixels - viewport_pixels).max(0.0);
    let scroll_pixels = index
        .pixel_for_list_offset(list_state.logical_scroll_top())
        .clamp(0.0, max_scroll_pixels);
    let progress = if max_scroll_pixels > 0.0 {
        scroll_pixels / max_scroll_pixels
    } else {
        0.0
    };
    let line_height = index.density.line_height();
    let edge_padding = index.density.edge_padding();
    let visible_minimap_lines = ((interaction_height - edge_padding * 2.0) / line_height).max(1.0);
    let max_content_top = (total - visible_minimap_lines).max(0.0);
    let (editor_top, editor_bottom) =
        minimap_visible_display_range(index, scroll_pixels, viewport_pixels);
    let minimum_content_top = (editor_bottom - visible_minimap_lines)
        .max(0.0)
        .min(max_content_top);
    let maximum_content_top = editor_top.max(minimum_content_top).min(max_content_top);
    let content_top = if max_content_top > 0.0 {
        (progress * max_content_top).clamp(minimum_content_top, maximum_content_top)
    } else {
        0.0
    };
    let height =
        minimap_thumb_height_for_scroll(index, scroll_pixels, viewport_pixels, interaction_height);
    let raw_top = edge_padding + (editor_top - content_top).max(0.0) * line_height;
    let top = if progress <= f32::EPSILON {
        0.0
    } else if progress >= 1.0 - f32::EPSILON {
        (interaction_height - height).max(0.0)
    } else {
        raw_top.clamp(0.0, (interaction_height - height).max(0.0))
    };
    MinimapViewport {
        content_top,
        thumb: ThumbGeometry { top, height },
        scroll_ratio: progress,
        interaction_height,
    }
}

fn minimap_viewport_for_list_with_anchor(
    index: &MinimapLineIndex,
    list_state: &ListState,
    track_height: f32,
    anchor: Option<MinimapInteractionAnchor>,
) -> MinimapViewport {
    let mut viewport = minimap_viewport_for_list(index, list_state, track_height);
    if let Some(anchor) = anchor.filter(|anchor| anchor.matches(index, viewport)) {
        let visible_minimap_lines = ((viewport.interaction_height
            - index.density.edge_padding() * 2.0)
            / index.density.line_height())
        .max(1.0);
        viewport.content_top = anchor
            .content_top
            .clamp(0.0, (index.total as f32 - visible_minimap_lines).max(0.0));
        viewport.thumb.top = anchor.thumb_top.clamp(
            0.0,
            (viewport.interaction_height - viewport.thumb.height).max(0.0),
        );
    }
    viewport
}

#[derive(Clone, Copy, Debug)]
struct MinimapClickTarget {
    offset: ListOffset,
    ratio: f32,
    clicked_display: f32,
    thumb_top: f32,
    thumb_height: f32,
}

fn minimap_click_target_for_viewport(
    index: &MinimapLineIndex,
    list_state: &ListState,
    viewport: MinimapViewport,
    local_y: f32,
) -> MinimapClickTarget {
    let clicked_display = (viewport.content_top
        + ((local_y - index.density.edge_padding()) / index.density.line_height()).max(0.0))
    .clamp(0.0, index.total as f32);
    let clicked_offset = index.list_offset_for_display_position(clicked_display);
    let viewport_pixels = f32::from(list_state.viewport_bounds().size.height).max(0.0);
    let document_pixels = index.document_pixels();
    let max_scroll_pixels = (document_pixels - viewport_pixels).max(0.0);
    let target_pixels = (index.pixel_for_list_offset(clicked_offset) - viewport_pixels * 0.5)
        .clamp(0.0, max_scroll_pixels);
    let ratio = if max_scroll_pixels <= f32::EPSILON {
        0.0
    } else {
        target_pixels / max_scroll_pixels
    };
    let target_thumb_height = minimap_thumb_height_for_scroll(
        index,
        target_pixels,
        viewport_pixels,
        viewport.interaction_height,
    );
    MinimapClickTarget {
        offset: index.list_offset_for_pixel(target_pixels),
        ratio,
        clicked_display,
        thumb_top: (local_y - target_thumb_height * 0.5).clamp(
            0.0,
            (viewport.interaction_height - target_thumb_height).max(0.0),
        ),
        thumb_height: target_thumb_height,
    }
}

fn minimap_drag_target(
    local_y: f32,
    grab_offset: f32,
    thumb_height: f32,
    track_height: f32,
) -> (f32, f32) {
    let travel = (track_height - thumb_height).max(0.0);
    if travel <= f32::EPSILON {
        (0.0, 0.0)
    } else {
        let thumb_top = (local_y - grab_offset).clamp(0.0, travel);
        (thumb_top / travel, thumb_top)
    }
}

fn scroll_ratio_after_wheel(
    index: &MinimapLineIndex,
    list_state: &ListState,
    wheel_delta_y: f32,
) -> f32 {
    let viewport_pixels = f32::from(list_state.viewport_bounds().size.height).max(0.0);
    let document_pixels = index.document_pixels();
    let max_scroll_pixels = (document_pixels - viewport_pixels).max(0.0);
    if max_scroll_pixels <= f32::EPSILON {
        return 0.0;
    }
    let current = index.pixel_for_list_offset(list_state.logical_scroll_top());
    // GPUI wheel deltas describe content movement; document scroll is the inverse.
    (current - wheel_delta_y).clamp(0.0, max_scroll_pixels) / max_scroll_pixels
}

fn scroll_list_to_ratio(index: &MinimapLineIndex, list_state: &ListState, ratio: f32) {
    let ratio = ratio.clamp(0.0, 1.0);
    if ratio >= 1.0 - f32::EPSILON && list_state.item_count() > 0 {
        list_state.scroll_to(ListOffset {
            item_ix: list_state.item_count() - 1,
            offset_in_item: px(0.0),
        });
    } else {
        let viewport_pixels = f32::from(list_state.viewport_bounds().size.height).max(0.0);
        let document_pixels = index.document_pixels();
        let max_scroll_pixels = (document_pixels - viewport_pixels).max(0.0);
        list_state.scroll_to(index.list_offset_for_pixel(ratio * max_scroll_pixels));
    }
}

#[derive(Clone)]
struct RasterRow {
    document_index: usize,
    lines: DisplayLines,
}

impl RasterRow {
    fn line_count(&self) -> usize {
        self.lines.ranges.len().max(1)
    }
}

fn theme_signature() -> u64 {
    let theme = current_theme();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    theme.background_alt.hash(&mut hasher);
    theme.foreground.hash(&mut hasher);
    theme.code_background.hash(&mut hasher);
    theme.heading.hash(&mut hasher);
    hasher.finish()
}

fn tile_key(
    rows: &[usize],
    first_row: usize,
    width: usize,
    folded_signature: u64,
    wrap_signature: u64,
    scale_factor: f32,
    density: MinimapDensity,
) -> RasterTileKey {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rows.hash(&mut hasher);
    RasterTileKey {
        first_row,
        row_signature: hasher.finish(),
        width: width.min(u16::MAX as usize) as u16,
        theme_signature: theme_signature(),
        folded_signature,
        wrap_signature,
        scale_factor_x100: (scale_factor * 100.0).round().clamp(1.0, u16::MAX as f32) as u16,
        density,
    }
}

fn folded_signature(model: &PreviewDisplayMap, rows: &[usize], folded: &HashSet<u32>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for &row in rows {
        let block_id = model.rows[row].block_id;
        if folded.contains(&block_id) {
            block_id.hash(&mut hasher);
        }
    }
    hasher.finish()
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DisplayWindow {
    rows: Range<usize>,
    skip_display_lines: usize,
}

fn display_window_range(
    total_rows: usize,
    anchor_row: usize,
    anchor_inner_line: usize,
    display_capacity: usize,
    anchor_display_offset: usize,
    mut line_count: impl FnMut(usize) -> usize,
) -> DisplayWindow {
    if total_rows == 0 || display_capacity == 0 {
        return DisplayWindow {
            rows: 0..0,
            skip_display_lines: 0,
        };
    }
    let anchor = anchor_row.min(total_rows - 1);
    let mut first = anchor;
    let mut lines_before_anchor = 0usize;
    let wanted_before_row = anchor_display_offset.saturating_sub(anchor_inner_line);
    while first > 0 && lines_before_anchor < wanted_before_row {
        first -= 1;
        lines_before_anchor += line_count(first).max(1);
    }
    let skip_display_lines = lines_before_anchor.saturating_sub(wanted_before_row)
        + anchor_inner_line.saturating_sub(anchor_display_offset);
    let mut end = first;
    let mut lines = 0usize;
    while end < total_rows && (lines < display_capacity + skip_display_lines || end <= anchor) {
        lines += line_count(end).max(1);
        end += 1;
    }
    DisplayWindow {
        rows: first..end,
        skip_display_lines,
    }
}

fn table_column_edges(style: &TableRowStyle, width: usize) -> SmallVec<[usize; 8]> {
    let columns = style.column_widths();
    let total = columns.iter().copied().sum::<usize>().max(1);
    let drawable = width.saturating_sub(8);
    let mut consumed = 0usize;
    columns
        .iter()
        .take(columns.len().saturating_sub(1))
        .map(|column| {
            consumed += column;
            4 + drawable * consumed / total
        })
        .collect()
}

fn cosmic_color(rgb: u32) -> cosmic_text::Color {
    cosmic_text::Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

fn syntax_color(kind: super::CodeHighlightKind) -> u32 {
    let theme = current_theme();
    match kind {
        super::CodeHighlightKind::Attribute => theme.attribute,
        super::CodeHighlightKind::Boolean | super::CodeHighlightKind::Constant => theme.constant,
        super::CodeHighlightKind::Comment => theme.comment,
        super::CodeHighlightKind::Function => theme.function,
        super::CodeHighlightKind::Keyword => theme.keyword,
        super::CodeHighlightKind::Number => theme.number,
        super::CodeHighlightKind::Operator | super::CodeHighlightKind::Punctuation => {
            theme.operator
        }
        super::CodeHighlightKind::Property | super::CodeHighlightKind::Variable => theme.variable,
        super::CodeHighlightKind::String => theme.string,
        super::CodeHighlightKind::Type => theme.type_name,
    }
}

fn cosmic_runs(
    kind: PreviewLineKind,
    line: &DisplayRuns,
) -> Vec<(&str, cosmic_text::Attrs<'static>)> {
    let mut boundaries =
        Vec::with_capacity((line.inline_spans.len() + line.code_spans.len()) * 2 + 2);
    boundaries.extend([0, line.text.len()]);
    for span in line.inline_spans.iter() {
        boundaries.extend([
            span.range.start.min(line.text.len()),
            span.range.end.min(line.text.len()),
        ]);
    }
    for span in line.code_spans.iter() {
        boundaries.extend([
            span.start.min(line.text.len()),
            span.end.min(line.text.len()),
        ]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
        .windows(2)
        .filter_map(|range| {
            let (start, end) = (range[0], range[1]);
            if start >= end
                || !line.text.is_char_boundary(start)
                || !line.text.is_char_boundary(end)
            {
                return None;
            }
            let mut color = kind_color(kind);
            let mut weight = cosmic_text::Weight::BLACK;
            let mut style = cosmic_text::Style::Normal;
            for inline in line
                .inline_spans
                .iter()
                .filter(|span| span.range.start < end && span.range.end > start)
                .map(|span| span.kind)
            {
                match inline {
                    InlineKind::Bold => weight = cosmic_text::Weight::BLACK,
                    InlineKind::Italic => style = cosmic_text::Style::Italic,
                    InlineKind::Code | InlineKind::Verbatim => color = current_theme().inline_code,
                    InlineKind::Link | InlineKind::FootnoteReference | InlineKind::Underline => {
                        color = current_theme().link
                    }
                    InlineKind::Timestamp => color = current_theme().date,
                    InlineKind::Target | InlineKind::RadioTarget => {
                        color = current_theme().attribute
                    }
                    InlineKind::Entity | InlineKind::Latex => color = current_theme().constant,
                    InlineKind::Strike => {}
                }
            }
            if let Some(span) = line
                .code_spans
                .iter()
                .find(|span| span.start < end && span.end > start)
            {
                color = syntax_color(span.kind);
                if matches!(span.kind, super::CodeHighlightKind::Comment) {
                    style = cosmic_text::Style::Italic;
                }
            }
            Some((
                &line.text[start..end],
                cosmic_text::Attrs::new()
                    .family(cosmic_text::Family::Name("Menlo"))
                    .weight(weight)
                    .style(style)
                    .color(cosmic_color(color)),
            ))
        })
        .collect()
}

fn fill_bgra(
    pixels: &mut [u8],
    image_width: usize,
    image_height: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    color: u32,
) {
    let end_y = (y + height).min(image_height);
    let end_x = (x + width).min(image_width);
    let pixel = [color as u8, (color >> 8) as u8, (color >> 16) as u8, 0xff];
    for row in y.min(image_height)..end_y {
        for column in x.min(image_width)..end_x {
            let offset = (row * image_width + column) * 4;
            pixels[offset..offset + 4].copy_from_slice(&pixel);
        }
    }
}

fn minimap_font_system() -> cosmic_text::FontSystem {
    cosmic_text::FontSystem::new()
}

type MinimapTextRasterizer = Mutex<(cosmic_text::FontSystem, cosmic_text::SwashCache)>;
static TEXT_RASTERIZER: OnceLock<MinimapTextRasterizer> = OnceLock::new();
static TEXT_RASTERIZER_PREWARMED: OnceLock<()> = OnceLock::new();

pub(super) fn prewarm_text_rasterizer() {
    use cosmic_text::{Attrs, Buffer, Color, Family, Metrics, Shaping, Wrap};

    let started = Instant::now();
    let rasterizer = TEXT_RASTERIZER
        .get_or_init(|| Mutex::new((minimap_font_system(), cosmic_text::SwashCache::new())));
    let mut did_work = false;
    TEXT_RASTERIZER_PREWARMED.get_or_init(|| {
        did_work = true;
        // FontSystem construction only scans the database. Shape and raster a bounded corpus as
        // well so the first real tile does not pay lazy fallback/font-face/Swash initialization.
        // This runs in the existing background prewarm task and never changes rendered content.
        let mut rasterizer = rasterizer.lock().expect("minimap rasterizer poisoned");
        let (font_system, swash_cache) = &mut *rasterizer;
        let attrs = Attrs::new()
            .family(Family::Name("Menlo"))
            .weight(cosmic_text::Weight::BLACK);
        let mut buffer = Buffer::new(font_system, Metrics::new(3.6, 5.0));
        buffer.set_size(Some(480.0), Some(20.0));
        buffer.set_wrap(Wrap::None);
        buffer.set_text(
            "Org Markdown 中文预览 AaZz 0123456789 +-*/_`#[](){} :=> ",
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
    });
    if minimap_perf_enabled() && did_work {
        eprintln!(
            "org_studio_minimap_text_prewarm elapsed_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0,
        );
    }
}

pub(super) fn prewarm_document_text(model: Arc<PreviewDisplayMap>) {
    use cosmic_text::{Attrs, Buffer, Color, Family, Metrics, Shaping, Wrap};

    const PREWARM_BUDGET: Duration = Duration::from_millis(350);
    let started = Instant::now();
    prewarm_text_rasterizer();
    if started.elapsed() >= PREWARM_BUDGET {
        return;
    }
    let rasterizer = TEXT_RASTERIZER
        .get()
        .expect("minimap rasterizer must exist after prewarm");
    let mut rasterizer = rasterizer.lock().expect("minimap rasterizer poisoned");
    let (font_system, swash_cache) = &mut *rasterizer;
    let density = MinimapDensity::Compact;
    let mut buffer = Buffer::new(
        font_system,
        Metrics::new(density.font_px(), density.line_height()),
    );
    buffer.set_size(
        Some(MINIMAP_AUTO_COMPACT_MAX_PX - 6.0),
        Some(density.line_height()),
    );
    buffer.set_wrap(Wrap::None);
    let attrs = Attrs::new()
        .family(Family::Name("Menlo"))
        .weight(cosmic_text::Weight::BLACK);

    // A 720px initial window exposes at most 277 compact display lines. Include bounded
    // overdraw and wrapped-row slack while keeping this independent of total document size.
    for row in 0..model.rows.len().min(384) {
        if started.elapsed() >= PREWARM_BUDGET {
            break;
        }
        let kind = model.row_kind(model.rows[row].block_id);
        let display = model.runs(row);
        buffer.set_rich_text(cosmic_runs(kind, &display), &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);
        buffer.draw(
            font_system,
            swash_cache,
            Color::rgb(0, 0, 0),
            |_, _, _, _, _| {},
        );
    }
}

fn rasterize_tile(
    model: &PreviewDisplayMap,
    presentation_rows: &[RasterRow],
    width: usize,
    folded: &HashSet<u32>,
    scale_factor: f32,
    density: MinimapDensity,
) -> RasterizedTile {
    use cosmic_text::{Attrs, Buffer, Color, Family, Metrics, Shaping, Wrap};

    let started = Instant::now();
    let line_count = presentation_rows
        .iter()
        .map(RasterRow::line_count)
        .sum::<usize>();
    let scale_factor = scale_factor.max(1.0);
    let physical_line_height = density.line_height() * scale_factor;
    let height = (line_count as f32 * physical_line_height).ceil().max(1.0) as u32;
    let logical_width = width.max(1);
    let width = (logical_width as f32 * scale_factor).ceil() as u32;
    let mut pixels = vec![0_u8; width as usize * height as usize * 4];
    let text_system_started = Instant::now();
    let cold_text_system = TEXT_RASTERIZER.get().is_none();
    let rasterizer = TEXT_RASTERIZER
        .get_or_init(|| Mutex::new((minimap_font_system(), cosmic_text::SwashCache::new())));
    let mut rasterizer = rasterizer.lock().expect("minimap rasterizer poisoned");
    let text_system_wait = text_system_started.elapsed();
    let (font_system, swash_cache) = &mut *rasterizer;
    let mut buffer = Buffer::new(
        font_system,
        Metrics::new(density.font_px() * scale_factor, physical_line_height),
    );
    buffer.set_size(
        Some(width as f32 - 6.0 * scale_factor),
        Some(physical_line_height),
    );
    buffer.set_wrap(Wrap::None);

    let mut line_slot = 0usize;
    for raster_row in presentation_rows {
        let document_index = raster_row.document_index;
        let kind = model.row_kind(model.rows[document_index].block_id);
        let layout = model.layout(document_index);
        let block_id = model.rows[document_index].block_id;
        let mut display_runs = minimap_runs(model.runs(document_index));
        if folded.contains(&block_id) && matches!(kind, PreviewLineKind::Heading(_)) {
            display_runs.text = format!("{} …", display_runs.text).into();
        }
        let color = kind_color(kind);
        let row_y = (line_slot as f32 * physical_line_height).round() as usize;
        let row_visual_height =
            (raster_row.line_count() as f32 * physical_line_height).ceil() as usize;
        let theme = current_theme();
        match kind {
            PreviewLineKind::Code => fill_bgra(
                &mut pixels,
                width as usize,
                height as usize,
                2,
                row_y,
                width.saturating_sub(4) as usize,
                row_visual_height,
                theme.code_background,
            ),
            PreviewLineKind::Table => fill_bgra(
                &mut pixels,
                width as usize,
                height as usize,
                2,
                row_y,
                width.saturating_sub(4) as usize,
                row_visual_height,
                theme.background_alt,
            ),
            PreviewLineKind::Quote => fill_bgra(
                &mut pixels,
                width as usize,
                height as usize,
                2,
                row_y,
                1,
                row_visual_height,
                theme.heading[1],
            ),
            PreviewLineKind::Rule => fill_bgra(
                &mut pixels,
                width as usize,
                height as usize,
                4,
                row_y + 1,
                width.saturating_sub(8) as usize,
                1,
                theme.border,
            ),
            PreviewLineKind::Image => fill_bgra(
                &mut pixels,
                width as usize,
                height as usize,
                5,
                row_y,
                model.minimap_image_width(document_index, width as usize),
                (density.line_height() * scale_factor).ceil() as usize,
                theme.attribute,
            ),
            PreviewLineKind::Heading(_) => fill_bgra(
                &mut pixels,
                width as usize,
                height as usize,
                3,
                row_y + 1,
                1,
                1,
                color,
            ),
            _ => {}
        }
        if kind == PreviewLineKind::Table
            && let Some(style) = model.table_layout(document_index)
        {
            let table_color = if style.is_separator() {
                theme.border
            } else {
                theme.foreground_dim
            };
            for x in table_column_edges(style, width as usize) {
                fill_bgra(
                    &mut pixels,
                    width as usize,
                    height as usize,
                    x,
                    row_y,
                    1,
                    row_visual_height,
                    table_color,
                );
            }
        }
        let attrs = Attrs::new()
            .family(Family::Name("Menlo"))
            .weight(cosmic_text::Weight::BLACK);
        let semantic_indent = match kind {
            PreviewLineKind::Heading(_) | PreviewLineKind::List => 6,
            PreviewLineKind::Quote => 7,
            _ => 4,
        };
        let indent = ((semantic_indent as f32
            + (layout.padding_left * density.font_px() / 14.0).round())
            * scale_factor)
            .round() as i32;
        let base = Color::rgb((color >> 16) as u8, (color >> 8) as u8, color as u8);
        for range in raster_row.lines.ranges.iter().cloned() {
            let segment = slice_display_runs(&display_runs, range);
            buffer.set_rich_text(cosmic_runs(kind, &segment), &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(font_system, false);
            let segment_y = (line_slot as f32 * physical_line_height).round() as i32;
            buffer.draw(font_system, swash_cache, base, |x, y, w, h, color| {
                let x = x + indent;
                let y = y + segment_y;
                for py in 0..h as i32 {
                    let target_y = y + py;
                    if target_y < 0 || target_y >= height as i32 {
                        continue;
                    }
                    for px_offset in 0..w as i32 {
                        let target_x = x + px_offset;
                        if target_x < 0 || target_x >= width as i32 {
                            continue;
                        }
                        let offset = (target_y as usize * width as usize + target_x as usize) * 4;
                        pixels[offset] = color.b();
                        pixels[offset + 1] = color.g();
                        pixels[offset + 2] = color.r();
                        pixels[offset + 3] = color.a();
                    }
                }
            });
            line_slot += 1;
        }
    }
    let buffer = RgbaImage::from_raw(width, height, pixels).expect("valid minimap tile dimensions");
    RasterizedTile {
        image: Arc::new(RenderImage::new(SmallVec::from_elem(Frame::new(buffer), 1))),
        line_count,
        total: started.elapsed(),
        text_system_wait,
        cold_text_system,
    }
}

// Historical CoreText implementation retained temporarily for comparison.
// The unified production path above uses cosmic-text on every platform.
#[cfg(any())]
fn rasterize_tile(
    model: &PreviewDisplayMap,
    presentation_rows: &[RasterRow],
    width: usize,
    folded: &HashSet<u32>,
) -> Arc<RenderImage> {
    use core_foundation::{
        attributed_string::CFMutableAttributedString,
        base::{CFRange, TCFType},
        string::CFString,
    };
    use core_graphics::{
        base::kCGImageAlphaPremultipliedLast,
        color::CGColor,
        color_space::CGColorSpace,
        context::CGContext,
        geometry::{CGPoint, CGRect, CGSize},
    };
    use core_text::{
        font,
        line::CTLine,
        string_attributes::{kCTFontAttributeName, kCTForegroundColorAttributeName},
    };

    static CORE_TEXT_RASTER_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    let _raster_guard = CORE_TEXT_RASTER_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("CoreText minimap raster lock poisoned");

    fn color(rgb: u32) -> CGColor {
        CGColor::rgb(
            ((rgb >> 16) & 0xff) as f64 / 255.0,
            ((rgb >> 8) & 0xff) as f64 / 255.0,
            (rgb & 0xff) as f64 / 255.0,
            1.0,
        )
    }
    fn utf16_range(text: &str, start: usize, end: usize) -> CFRange {
        let start = start.min(text.len());
        let end = end.min(text.len()).max(start);
        let utf16_start = text[..start].encode_utf16().count();
        let utf16_len = text[start..end].encode_utf16().count();
        CFRange::init(utf16_start as isize, utf16_len as isize)
    }

    let line_count = presentation_rows
        .iter()
        .map(RasterRow::line_count)
        .sum::<usize>();
    let height = (line_count as f32 * MINIMAP_LINE_HEIGHT_PX).ceil().max(1.0) as usize;
    let width = width.max(1);
    let mut pixels = vec![0_u8; width * height * 4];
    let context = CGContext::create_bitmap_context(
        Some(pixels.as_mut_ptr().cast()),
        width,
        height,
        8,
        width * 4,
        &CGColorSpace::create_device_rgb(),
        kCGImageAlphaPremultipliedLast,
    );
    context.set_should_antialias(true);
    // At minimap sizes Regular loses most stems to grayscale coverage. Zed's
    // minimap uses a black/heavy face; Menlo-Bold is the stable macOS face.
    let regular = font::new_from_name("Menlo-Bold", MINIMAP_FONT_PX as f64)
        .or_else(|_| font::new_from_name("Menlo", MINIMAP_FONT_PX as f64))
        .expect("Menlo must be available on macOS");
    let bold = font::new_from_name("Menlo-Bold", MINIMAP_FONT_PX as f64)
        .unwrap_or_else(|_| regular.clone());
    let italic = font::new_from_name("Menlo-BoldItalic", MINIMAP_FONT_PX as f64)
        .or_else(|_| font::new_from_name("Menlo-Italic", MINIMAP_FONT_PX as f64))
        .unwrap_or_else(|_| regular.clone());

    let mut line_slot = 0usize;
    for raster_row in presentation_rows {
        let document_index = raster_row.document_index;
        let kind = model.row_kind(model.rows[document_index].block_id);
        let layout = model.layout(document_index);
        let block_id = model.rows[document_index].block_id;
        let mut full_runs = minimap_runs(model.runs(document_index));
        if folded.contains(&block_id) && matches!(kind, PreviewLineKind::Heading(_)) {
            full_runs.text = format!("{} …", full_runs.text).into();
        }
        let row_y = line_slot as f64 * MINIMAP_LINE_HEIGHT_PX as f64;
        let row_visual_height = raster_row.line_count() as f64 * MINIMAP_LINE_HEIGHT_PX as f64;
        let bottom_y = height as f64 - row_y - row_visual_height;
        let fill = |rgb: u32, x: f64, y: f64, rect_width: f64, rect_height: f64| {
            context.set_fill_color(&color(rgb));
            context.fill_rect(CGRect::new(
                &CGPoint::new(x, y),
                &CGSize::new(rect_width.max(0.0), rect_height.max(0.0)),
            ));
        };
        match kind {
            PreviewLineKind::Code => fill(
                current_theme().code_background,
                2.0,
                bottom_y,
                width.saturating_sub(4) as f64,
                row_visual_height,
            ),
            PreviewLineKind::Table => fill(
                current_theme().background_alt,
                2.0,
                bottom_y,
                width.saturating_sub(4) as f64,
                row_visual_height,
            ),
            PreviewLineKind::Quote => fill(
                current_theme().heading[1],
                2.0,
                bottom_y,
                1.0,
                row_visual_height,
            ),
            PreviewLineKind::Rule => fill(
                current_theme().border,
                4.0,
                bottom_y + 1.0,
                width.saturating_sub(8) as f64,
                1.0,
            ),
            PreviewLineKind::Image => fill(
                current_theme().attribute,
                5.0,
                bottom_y,
                model.minimap_image_width(document_index, width) as f64,
                MINIMAP_LINE_HEIGHT_PX as f64,
            ),
            PreviewLineKind::Heading(_) => fill(kind_color(kind), 3.0, bottom_y + 1.0, 1.0, 1.0),
            _ => {}
        }
        if kind == PreviewLineKind::Table
            && let Some(style) = model.table_layout(document_index)
        {
            let table_color = if style.is_separator() {
                current_theme().border
            } else {
                current_theme().foreground_dim
            };
            for x in table_column_edges(style, width) {
                fill(table_color, x as f64, bottom_y, 1.0, row_visual_height);
            }
        }
        if full_runs.text.is_empty() {
            line_slot += raster_row.line_count();
            continue;
        }
        for display_range in raster_row.lines.ranges.iter().cloned() {
            let runs = slice_display_runs(&full_runs, display_range);
            let segment_y = line_slot as f64 * MINIMAP_LINE_HEIGHT_PX as f64;
            let mut attributed = CFMutableAttributedString::new();
            attributed.replace_str(&CFString::new(&runs.text), CFRange::init(0, 0));
            let full_range = CFRange::init(0, attributed.char_len());
            let scaled_font_size = (layout.font_size * MINIMAP_FONT_PX / 14.0).max(1.5) as f64;
            let row_regular = regular.clone_with_font_size(scaled_font_size);
            let row_bold = bold.clone_with_font_size(scaled_font_size);
            let row_italic = italic.clone_with_font_size(scaled_font_size);
            unsafe {
                attributed.set_attribute(full_range, kCTFontAttributeName, &row_regular);
                attributed.set_attribute(
                    full_range,
                    kCTForegroundColorAttributeName,
                    &color(kind_color(kind)),
                );
                for span in runs.inline_spans.iter() {
                    let range = utf16_range(&runs.text, span.range.start, span.range.end);
                    match span.kind {
                        InlineKind::Bold => {
                            attributed.set_attribute(range, kCTFontAttributeName, &row_bold)
                        }
                        InlineKind::Italic => {
                            attributed.set_attribute(range, kCTFontAttributeName, &row_italic)
                        }
                        InlineKind::Code | InlineKind::Verbatim => attributed.set_attribute(
                            range,
                            kCTForegroundColorAttributeName,
                            &color(current_theme().inline_code),
                        ),
                        InlineKind::Link
                        | InlineKind::FootnoteReference
                        | InlineKind::Underline => attributed.set_attribute(
                            range,
                            kCTForegroundColorAttributeName,
                            &color(current_theme().link),
                        ),
                        InlineKind::Timestamp => attributed.set_attribute(
                            range,
                            kCTForegroundColorAttributeName,
                            &color(current_theme().date),
                        ),
                        InlineKind::Target | InlineKind::RadioTarget => attributed.set_attribute(
                            range,
                            kCTForegroundColorAttributeName,
                            &color(current_theme().attribute),
                        ),
                        InlineKind::Entity | InlineKind::Latex => attributed.set_attribute(
                            range,
                            kCTForegroundColorAttributeName,
                            &color(current_theme().constant),
                        ),
                        InlineKind::Strike => {}
                    }
                }
                for span in runs.code_spans.iter() {
                    let range = utf16_range(&runs.text, span.start, span.end);
                    attributed.set_attribute(
                        range,
                        kCTForegroundColorAttributeName,
                        &color(syntax_color(span.kind)),
                    );
                    if matches!(span.kind, super::CodeHighlightKind::Comment) {
                        attributed.set_attribute(range, kCTFontAttributeName, &row_italic);
                    }
                }
            }
            let line = CTLine::new_with_attributed_string(attributed.as_concrete_TypeRef());
            let semantic_indent = match kind {
                PreviewLineKind::Heading(_) | PreviewLineKind::List => 6.0,
                PreviewLineKind::Quote => 7.0,
                _ => 4.0,
            };
            let indent =
                semantic_indent + layout.padding_left as f64 * MINIMAP_FONT_PX as f64 / 14.0;
            context.set_text_position(indent, height as f64 - segment_y - MINIMAP_FONT_PX as f64);
            line.draw(&context);
            line_slot += 1;
        }
    }
    // CoreGraphics writes RGBA; GPUI RenderImage consumes BGRA.
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buffer = RgbaImage::from_raw(width as u32, height as u32, pixels)
        .expect("valid minimap tile dimensions");
    Arc::new(RenderImage::new(SmallVec::from_elem(Frame::new(buffer), 1)))
}

impl PreviewDisplayMap {
    pub(super) fn runs(&self, row: usize) -> DisplayRuns {
        if let Some(runs) = self
            .display_runs
            .lock()
            .expect("display-run cache poisoned")
            .entries
            .get(&row)
            .cloned()
        {
            return runs;
        }
        let runs = materialize_runs(self, row);
        self.display_runs
            .lock()
            .expect("display-run cache poisoned")
            .insert(row, runs.clone());
        runs
    }

    pub(super) fn is_heading(&self, row: usize) -> bool {
        self.rows
            .get(row)
            .is_some_and(|line| matches!(self.row_kind(line.block_id), PreviewLineKind::Heading(_)))
    }

    fn display_lines(
        &self,
        row: usize,
        available_width: f32,
        text_system: &gpui::WindowTextSystem,
    ) -> DisplayLines {
        let width_key = available_width.round().clamp(1.0, u16::MAX as f32) as u16;
        let key = (row, width_key);
        if let Some(lines) = self
            .display_lines
            .lock()
            .expect("display-line cache poisoned")
            .entries
            .get(&key)
            .cloned()
        {
            return lines;
        }

        let display = self.runs(row);
        let kind = self.row_kind(self.rows[row].block_id);
        let layout = self.layout(row);
        let mut base_font = font("Menlo");
        base_font.weight = match kind {
            PreviewLineKind::Heading(1 | 2) => FontWeight::SEMIBOLD,
            PreviewLineKind::Heading(_) => FontWeight::MEDIUM,
            _ => FontWeight::NORMAL,
        };
        let text_runs = minimap_text_runs(kind, &display, base_font);
        let marker_width = if matches!(kind, PreviewLineKind::Heading(_)) {
            20.0
        } else {
            0.0
        };
        let wrap_width =
            (available_width - layout.padding_left - layout.padding_right - marker_width).max(1.0);
        let mut starts = vec![0];
        if !display.text.is_empty()
            && let Ok(shaped) = text_system.shape_text(
                display.text.clone(),
                px(layout.font_size),
                &text_runs,
                Some(px(wrap_width)),
                None,
            )
            && let Some(line) = shaped.first()
        {
            starts.extend(line.wrap_boundaries().iter().filter_map(|boundary| {
                line.runs()
                    .get(boundary.run_ix)
                    .and_then(|run| run.glyphs.get(boundary.glyph_ix))
                    .map(|glyph| glyph.index)
            }));
        }
        starts.push(display.text.len());
        starts.sort_unstable();
        starts.dedup();
        let mut ranges = starts
            .windows(2)
            .filter_map(|pair| (pair[0] < pair[1]).then_some(pair[0]..pair[1]))
            .collect::<Vec<_>>();
        if ranges.is_empty() {
            ranges.push(0..display.text.len());
        }
        let parent_height = self
            .image_size(row, available_width)
            .map(|(_, height)| height + layout.padding_top + layout.padding_bottom)
            .or(layout.fixed_height)
            .unwrap_or_else(|| {
                (ranges.len().max(1) as f32 * layout.line_height
                    + layout.padding_top
                    + layout.padding_bottom)
                    .max(layout.min_height)
            })
            + layout.margin_top
            + layout.margin_bottom;
        let lines = DisplayLines {
            ranges: ranges.into(),
            parent_height,
        };
        self.display_lines
            .lock()
            .expect("display-line cache poisoned")
            .insert(key, lines.clone());
        lines
    }

    fn estimated_minimap_line_index(
        &self,
        presentation_rows: &[usize],
        width: u16,
        rows_signature: u64,
        available_width: f32,
        density: MinimapDensity,
    ) -> MinimapLineIndex {
        let mut measures = Vec::with_capacity(presentation_rows.len());

        for &row in presentation_rows {
            let layout = self.layout(row);
            let kind = self.row_kind(self.rows[row].block_id);
            let marker_width = if matches!(kind, PreviewLineKind::Heading(_)) {
                20.0
            } else {
                0.0
            };
            let wrap_width =
                (available_width - layout.padding_left - layout.padding_right - marker_width)
                    .max(1.0);
            let source_bytes = self.rows[row]
                .content
                .end
                .0
                .saturating_sub(self.rows[row].content.start.0)
                as f32;
            // This estimate intentionally uses metadata only: no text copy, inline
            // parsing, shaping or font lock. The coefficient is a conservative
            // average across ASCII and UTF-8 CJK source. Exact wrapping replaces it.
            let estimated_text_width = source_bytes * layout.font_size * 0.5;
            let line_count = if layout.fixed_height.is_some()
                || self.image_sizes.contains_key(&self.rows[row].block_id)
            {
                1
            } else {
                (estimated_text_width / wrap_width).ceil().max(1.0) as usize
            };
            let parent_height = self
                .image_size(row, available_width)
                .map(|(_, height)| height + layout.padding_top + layout.padding_bottom)
                .or(layout.fixed_height)
                .unwrap_or_else(|| {
                    (line_count as f32 * layout.line_height
                        + layout.padding_top
                        + layout.padding_bottom)
                        .max(layout.min_height)
                })
                + layout.margin_top
                + layout.margin_bottom;
            measures.push(ProjectionMeasure::new(line_count, parent_height, false));
        }

        let projection = Arc::new(ProjectionSnapshot::new(measures));
        MinimapLineIndex {
            width,
            rows_signature,
            density,
            total: projection.total_display_lines(),
            projection,
        }
    }

    fn advance_minimap_line_index(
        &self,
        presentation_rows: &Arc<Vec<usize>>,
        available_width: f32,
        density: MinimapDensity,
        priority_row: usize,
        allow_refinement: bool,
        text_system: &gpui::WindowTextSystem,
    ) -> MinimapLineIndexProgress {
        let key = MinimapLineIndexKey::new(presentation_rows, available_width, density);
        let cached = self
            .minimap_line_index
            .lock()
            .expect("minimap line index poisoned")
            .as_ref()
            .filter(|cached| {
                Arc::ptr_eq(&cached.presentation_rows, presentation_rows)
                    && cached.index.width == key.width
                    && cached.index.density == density
            })
            .map(|cached| cached.index.clone());
        if let Some(index) = cached {
            return MinimapLineIndexProgress {
                exact_rows: presentation_rows.len(),
                index,
                readiness: MinimapProjectionReadiness::Exact,
            };
        }
        let mut build = self
            .minimap_line_index_build
            .lock()
            .expect("minimap line-index builder poisoned");
        let created = build.as_ref().is_none_or(|builder| builder.key != key);
        if created {
            *build = Some(MinimapLineIndexBuilder::new(
                self,
                key,
                presentation_rows.clone(),
                available_width,
                density,
            ));
        }
        if created {
            let builder = build.as_mut().expect("line-index builder initialized");
            if minimap_perf_enabled() {
                eprintln!(
                    "org_studio_minimap_ready readiness=estimated rows={} exact_rows=0 width={} elapsed_ms={:.3} projection_bytes={}",
                    builder.presentation_rows.len(),
                    key.width,
                    builder.started_at.elapsed().as_secs_f64() * 1000.0,
                    builder.projection.estimated_heap_bytes(),
                );
            }
            return MinimapLineIndexProgress {
                index: builder.index(density),
                readiness: MinimapProjectionReadiness::Estimated,
                exact_rows: 0,
            };
        }
        if !allow_refinement {
            let builder = build.as_ref().expect("line-index builder initialized");
            return MinimapLineIndexProgress {
                index: builder.index(density),
                readiness: if builder.exact_rows == 0 {
                    MinimapProjectionReadiness::Estimated
                } else {
                    MinimapProjectionReadiness::PartiallyExact
                },
                exact_rows: builder.exact_rows,
            };
        }
        let started = Instant::now();
        let builder = build.as_mut().expect("line-index builder initialized");
        builder.prioritize(priority_row);
        let mut processed = 0usize;
        while builder.exact_rows < builder.presentation_rows.len() {
            let Some(projection_row) = builder.next_candidate() else {
                break;
            };
            let row = builder.presentation_rows[projection_row];
            let lines = self.display_lines(row, available_width, text_system);
            let count = lines.ranges.len().max(1);
            builder.pending_updates.push((
                projection_row,
                ProjectionMeasure::new(count, lines.parent_height, true),
            ));
            builder.mark_exact(projection_row);
            processed += 1;
            if processed.is_multiple_of(4) && started.elapsed() >= MINIMAP_INDEX_FRAME_BUDGET {
                let elapsed = started.elapsed();
                builder.record_slice(elapsed);
                builder
                    .pending_updates
                    .sort_unstable_by_key(|update| update.0);
                let batch_rows = builder.pending_updates.len();
                let publish_started = Instant::now();
                builder.publish_pending();
                let publish_elapsed = publish_started.elapsed();
                if minimap_trace_enabled() || builder.projection.exact_rows == batch_rows {
                    eprintln!(
                        "org_studio_minimap_projection_publish readiness=partially_exact exact_rows={} rows={} batch_rows={} commit_ms={:.3}",
                        builder.projection.exact_rows,
                        builder.presentation_rows.len(),
                        batch_rows,
                        publish_elapsed.as_secs_f64() * 1000.0,
                    );
                }
                if minimap_trace_enabled() {
                    eprintln!(
                        "org_studio_minimap_index_slice rows_done={} rows_total={} slice_ms={:.3}",
                        builder.exact_rows,
                        builder.presentation_rows.len(),
                        elapsed.as_secs_f64() * 1000.0,
                    );
                }
                return MinimapLineIndexProgress {
                    index: builder.index(density),
                    readiness: MinimapProjectionReadiness::PartiallyExact,
                    exact_rows: builder.exact_rows,
                };
            }
        }
        builder.record_slice(started.elapsed());
        let builder = build.as_mut().expect("completed line-index builder");
        builder
            .pending_updates
            .sort_unstable_by_key(|update| update.0);
        builder.publish_pending();
        let builder = build.take().expect("completed line-index builder");
        if minimap_perf_enabled() {
            eprintln!(
                "org_studio_minimap_index_ready rows={} width={} density={:?} elapsed_ms={:.3} work_ms={:.3} max_slice_ms={:.3} slices={}",
                builder.presentation_rows.len(),
                key.width,
                density,
                builder.started_at.elapsed().as_secs_f64() * 1000.0,
                builder.work.as_secs_f64() * 1000.0,
                builder.max_slice.as_secs_f64() * 1000.0,
                builder.slices,
            );
        }
        let index = builder.index(density);
        *self
            .minimap_line_index
            .lock()
            .expect("minimap line index poisoned") = Some(CachedMinimapLineIndex {
            presentation_rows: builder.presentation_rows,
            index: index.clone(),
        });
        MinimapLineIndexProgress {
            exact_rows: presentation_rows.len(),
            index,
            readiness: MinimapProjectionReadiness::Exact,
        }
    }

    pub(super) fn is_table(&self, row: usize) -> bool {
        self.rows
            .get(row)
            .is_some_and(|line| self.row_kind(line.block_id) == PreviewLineKind::Table)
    }

    pub(super) fn layout(&self, row: usize) -> RowLayout {
        let row = self.rows[row];
        match self.format {
            DocumentFormat::Org => match &self.blocks.nodes()[row.block_id as usize].kind {
                BlockKind::Heading { level } => RowLayout::text(
                    match level {
                        1 => 22.0,
                        2 => 18.0,
                        3 => 15.0,
                        _ => 14.0,
                    },
                    24.0,
                ),
                BlockKind::BlankLine => RowLayout::blank(),
                BlockKind::Paragraph => RowLayout::text(14.0, 22.0),
                BlockKind::ListItem => RowLayout {
                    padding_left: 4.0,
                    ..RowLayout::text(14.0, 22.0)
                },
                BlockKind::Image { .. } => RowLayout::image(),
                BlockKind::Planning | BlockKind::FixedWidth | BlockKind::FootnoteDefinition => {
                    RowLayout::text(13.0, 22.0)
                }
                BlockKind::SourceBlock { .. } => RowLayout {
                    padding_left: 16.0,
                    padding_right: 16.0,
                    padding_top: 2.0,
                    padding_bottom: 2.0,
                    ..RowLayout::text(13.0, 19.0)
                },
                BlockKind::ExampleBlock | BlockKind::Raw | BlockKind::ExportBlock { .. } => {
                    RowLayout {
                        padding_left: 16.0,
                        padding_right: 16.0,
                        padding_top: 2.0,
                        padding_bottom: 2.0,
                        ..RowLayout::text(13.0, 21.0)
                    }
                }
                BlockKind::QuoteBlock => RowLayout {
                    padding_left: 16.0,
                    padding_right: 8.0,
                    padding_top: 8.0,
                    padding_bottom: 8.0,
                    ..RowLayout::text(16.0, 25.0)
                },
                BlockKind::VerseBlock | BlockKind::CenterBlock => RowLayout::text(13.0, 22.0),
                BlockKind::SpecialBlock { .. } => RowLayout {
                    padding_left: 16.0,
                    padding_right: 16.0,
                    padding_top: 2.0,
                    padding_bottom: 2.0,
                    ..RowLayout::text(13.0, 19.0)
                },
                BlockKind::Drawer { .. } => RowLayout {
                    padding_left: 12.0,
                    padding_right: 12.0,
                    padding_top: 2.0,
                    padding_bottom: 2.0,
                    ..RowLayout::text(12.0, 19.0)
                },
                BlockKind::Keyword => RowLayout {
                    padding_top: 3.0,
                    padding_bottom: 3.0,
                    ..RowLayout::text(12.0, 18.0)
                },
                BlockKind::HorizontalRule => RowLayout::rule(),
                BlockKind::Comment | BlockKind::CommentBlock => RowLayout {
                    fixed_height: Some(0.0),
                    ..RowLayout::text(14.0, 24.0)
                },
                BlockKind::TableRow => RowLayout::text(13.0, 24.0),
            },
            DocumentFormat::Markdown => match &self.markdown_blocks[row.block_id as usize].kind {
                MarkdownKind::Heading { level } => RowLayout::text(
                    match level {
                        1 => 22.0,
                        2 => 18.0,
                        3 => 15.0,
                        _ => 14.0,
                    },
                    24.0,
                ),
                MarkdownKind::Blank => RowLayout::blank(),
                MarkdownKind::Paragraph => RowLayout::text(14.0, 22.0),
                MarkdownKind::ListItem => RowLayout {
                    padding_left: 4.0,
                    ..RowLayout::text(14.0, 22.0)
                },
                MarkdownKind::Quote => RowLayout {
                    padding_left: 16.0,
                    padding_right: 8.0,
                    padding_top: 4.0,
                    padding_bottom: 4.0,
                    ..RowLayout::text(15.0, 23.0)
                },
                MarkdownKind::Code { .. } => RowLayout {
                    padding_left: 16.0,
                    padding_right: 16.0,
                    padding_top: 2.0,
                    padding_bottom: 2.0,
                    ..RowLayout::text(13.0, 19.0)
                },
                MarkdownKind::TableRow => RowLayout::text(13.0, 24.0),
                MarkdownKind::HorizontalRule => RowLayout::rule(),
                MarkdownKind::Image { .. } => RowLayout::image(),
            },
        }
    }

    pub(super) fn table_layout(&self, row: usize) -> Option<&TableRowStyle> {
        self.tables.get(&self.rows[row].block_id)
    }

    pub(super) fn image_size(&self, row: usize, available_width: f32) -> Option<(f32, f32)> {
        self.image_sizes
            .get(&self.rows[row].block_id)
            .map(|&(width, height)| super::fitted_image_size(width, height, available_width))
    }

    fn minimap_image_width(&self, row: usize, minimap_width: usize) -> usize {
        self.image_size(row, minimap_width.saturating_sub(10) as f32)
            .map(|(width, _)| width.round() as usize)
            .unwrap_or_else(|| (minimap_width as f32 * 0.65) as usize)
            .min(minimap_width.saturating_sub(10))
    }

    fn row_kind(&self, block_id: u32) -> PreviewLineKind {
        row_kind(self.format, &self.blocks, &self.markdown_blocks, block_id)
    }

    fn code_language(&self, block_id: u32) -> Option<&str> {
        code_language(self.format, &self.blocks, &self.markdown_blocks, block_id)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct ThumbGeometry {
    pub top: f32,
    pub height: f32,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct MinimapLayout {
    first_line: usize,
    thumb: ThumbGeometry,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ScrollMetrics {
    offset: f32,
    max_offset: f32,
    viewport: f32,
}

pub(super) fn build_display_map(document: &PreviewDocument) -> PreviewDisplayMap {
    PreviewDisplayMap {
        text: document.text.clone(),
        format: document.format,
        blocks: document.blocks.clone(),
        markdown_blocks: document.markdown_blocks.clone(),
        rows: document.rows.clone(),
        tables: document.tables.clone(),
        image_sizes: document.image_sizes.clone(),
        display_runs: Mutex::new(DisplayRunCache {
            entries: HashMap::with_capacity(DisplayRunCache::CAPACITY),
            order: VecDeque::with_capacity(DisplayRunCache::CAPACITY),
        }),
        display_lines: Mutex::new(DisplayLineCache {
            entries: HashMap::with_capacity(DisplayLineCache::CAPACITY),
            order: VecDeque::with_capacity(DisplayLineCache::CAPACITY),
        }),
        raster_tiles: Mutex::new(RasterTileCache {
            entries: HashMap::with_capacity(RasterTileCache::CAPACITY),
            order: VecDeque::with_capacity(RasterTileCache::CAPACITY),
            in_flight: HashSet::with_capacity(RasterTileCache::CAPACITY),
        }),
        minimap_line_index: Mutex::new(None),
        minimap_line_index_build: Mutex::new(None),
        minimap_drag: Arc::new(Mutex::new(None)),
        minimap_resize_drag: Arc::new(Mutex::new(None)),
        minimap_interaction_anchor: Arc::new(Mutex::new(None)),
        initial_visible_batch_ready: AtomicBool::new(false),
        perf: MinimapPerfState::new(),
    }
}

impl PreviewDisplayMap {
    pub(super) fn initial_minimap_batch_ready(&self) -> bool {
        self.initial_visible_batch_ready.load(Ordering::Acquire)
    }
}

fn materialize_runs(model: &PreviewDisplayMap, row: usize) -> DisplayRuns {
    let line = &model.rows[row];
    let kind = model.row_kind(line.block_id);
    let source = model.text.copy_range(line.content);
    let source = source.trim_end_matches(['\r', '\n']);
    let parse_inline = matches!(
        kind,
        PreviewLineKind::Text
            | PreviewLineKind::Heading(_)
            | PreviewLineKind::List
            | PreviewLineKind::Quote
    );
    let (text, inline_spans): (SharedString, Arc<[InlineSpan]>) = if parse_inline {
        let parsed = parse_document_inline(model.format, source);
        (parsed.text.into(), parsed.spans.into())
    } else {
        (source.to_owned().into(), Arc::from([]))
    };
    let code_spans = model
        .code_language(line.block_id)
        .and_then(|language| highlight_code(language, &text).ok())
        .unwrap_or_default()
        .into();
    DisplayRuns {
        text,
        inline_spans,
        code_spans,
    }
}

#[cfg(test)]
fn truncate_for_minimap(text: &str) -> String {
    const MAX_MINIMAP_COLUMNS: usize = 1024;
    text.graphemes(true).take(MAX_MINIMAP_COLUMNS).collect()
}

#[cfg(test)]
fn minimap_font() -> gpui::Font {
    let mut minimap_font = font("Menlo");
    // Match Zed's 2px BLACK rendering and make CJK/emoji fallback deterministic.
    minimap_font.weight = FontWeight::BLACK;
    minimap_font.fallbacks = Some(FontFallbacks::from_fonts(vec![
        "PingFang SC".to_owned(),
        "Apple Color Emoji".to_owned(),
    ]));
    minimap_font
}

pub(super) fn render(
    model: Arc<PreviewDisplayMap>,
    presentation_rows: Arc<Vec<usize>>,
    folded: Arc<HashSet<u32>>,
    list_state: ListState,
    editor_width: f32,
    minimap_width: f32,
    thumb_visibility: crate::settings::MinimapThumbVisibility,
    generation: u64,
    presentation_revision: u64,
    opened_at: Instant,
    _on_seek: impl Fn(f32, bool, &mut gpui::Window, &mut gpui::App) + 'static,
    on_width_change: impl Fn(MinimapWidthChange, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let theme = current_theme();
    let density = MinimapDensity::for_width(minimap_width);
    let minimap_line_height = density.line_height();
    let minimap_edge_padding = density.edge_padding();
    let drag_session = model.minimap_drag.clone();
    let resize_session = model.minimap_resize_drag.clone();
    let on_width_change = Arc::new(on_width_change);
    let interaction_anchor = model.minimap_interaction_anchor.clone();
    let paint_list = list_state.clone();
    let shape_list = list_state.clone();
    let shape_model = model.clone();
    let paint_model = model.clone();
    let shape_rows = presentation_rows.clone();
    let shape_folded = folded;
    let active_line_index = Arc::new(Mutex::new(None::<MinimapLineIndex>));
    let shape_line_index = active_line_index.clone();
    let paint_line_index = active_line_index.clone();
    let shape_anchor = interaction_anchor.clone();
    let paint_anchor = interaction_anchor.clone();
    let track_bounds = Arc::new(Mutex::new(Bounds::default()));
    let shape_bounds = track_bounds.clone();
    let paint_drag = drag_session.clone();
    let event_drag = drag_session.clone();
    let event_resize = resize_session.clone();
    let event_width_change = on_width_change.clone();
    let event_anchor = interaction_anchor.clone();
    let event_line_index = active_line_index.clone();
    let event_list = list_state.clone();
    let hovered = Arc::new(AtomicBool::new(false));
    let paint_hovered = hovered.clone();
    let content = canvas(
        move |bounds, window, cx| {
            profiling::scope!("Minimap::shape_visible");
            *shape_bounds.lock().expect("minimap bounds poisoned") = bounds;
            let capacity = (f32::from(bounds.size.height) / minimap_line_height)
                .floor()
                .max(1.0) as usize;
            let width = f32::from(bounds.size.width).ceil().max(1.0) as usize;
            let scale_factor = window.scale_factor().max(1.0);
            let parent_width = (editor_width - 110.0 - minimap_width).max(120.0);
            let priority_row = shape_list
                .logical_scroll_top()
                .item_ix
                .min(shape_rows.len().saturating_sub(1));
            let interaction_active = shape_model
                .minimap_drag
                .lock()
                .expect("minimap drag state poisoned")
                .is_some()
                || shape_model
                    .minimap_resize_drag
                    .lock()
                    .expect("minimap resize state poisoned")
                    .is_some();
            let projection = {
                profiling::scope!("Minimap::line_index");
                shape_model.advance_minimap_line_index(
                    &shape_rows,
                    parent_width,
                    density,
                    priority_row,
                    !interaction_active,
                    window.text_system(),
                )
            };
            if projection.readiness != MinimapProjectionReadiness::Exact && !interaction_active {
                // Drive cooperative exact refinement at display cadence. The
                // estimated projection remains paintable throughout the process.
                window.request_animation_frame();
                if minimap_trace_enabled() {
                    eprintln!(
                        "org_studio_minimap_projection_progress readiness=estimated exact_rows={} rows_total={}",
                        projection.exact_rows,
                        shape_rows.len(),
                    );
                }
            }
            let line_index = projection.index;
            *shape_line_index.lock().expect("active line index poisoned") =
                Some(line_index.clone());
            let viewport = minimap_viewport_for_list_with_anchor(
                &line_index,
                &shape_list,
                f32::from(bounds.size.height),
                *shape_anchor.lock().expect("minimap anchor poisoned"),
            );
            let content_line = viewport.content_top.floor() as usize;
            let content_fraction = viewport.content_top.fract();
            let (anchor_row, anchor_inner_line) = line_index.locate(content_line);
            let visible_range = display_window_range(
                shape_rows.len(),
                anchor_row,
                anchor_inner_line,
                capacity,
                0,
                |index| {
                    shape_model
                        .display_lines(shape_rows[index], parent_width, window.text_system())
                        .ranges
                        .len()
                },
            );
            let first_line = visible_range.rows.start;
            let last_line = visible_range.rows.end;
            let first_tile = first_line / RASTER_TILE_ROWS * RASTER_TILE_ROWS;
            let mut tiles: SmallVec<[RasterTilePaint; 6]> = SmallVec::new();
            let mut raster_requests: SmallVec<[RasterTileRequest; 6]> = SmallVec::new();
            let mut visible_keys: SmallVec<[RasterTileKey; 6]> = SmallVec::new();
            let mut tile_y = minimap_edge_padding - content_fraction * minimap_line_height;
            for tile_start in (first_tile..last_line).step_by(RASTER_TILE_ROWS) {
                let tile_end = (tile_start + RASTER_TILE_ROWS).min(shape_rows.len());
                let tile_rows = &shape_rows[tile_start..tile_end];
                let mut wrap_hasher = std::collections::hash_map::DefaultHasher::new();
                let raster_rows = tile_rows
                    .iter()
                    .map(|&row| {
                        let mut lines =
                            shape_model.display_lines(row, parent_width, window.text_system());
                        let block_id = shape_model.rows[row].block_id;
                        if shape_folded.contains(&block_id)
                            && matches!(shape_model.row_kind(block_id), PreviewLineKind::Heading(_))
                        {
                            let mut ranges = lines.ranges.to_vec();
                            if let Some(last) = ranges.last_mut() {
                                last.end += " …".len();
                            }
                            lines.ranges = ranges.into();
                        }
                        RasterRow {
                            document_index: row,
                            lines,
                        }
                    })
                    .collect::<Vec<_>>();
                for row in &raster_rows {
                    let lines = &row.lines;
                    lines.parent_height.to_bits().hash(&mut wrap_hasher);
                    for range in lines.ranges.iter() {
                        range.start.hash(&mut wrap_hasher);
                        range.end.hash(&mut wrap_hasher);
                    }
                }
                let tile_line_count = raster_rows.iter().map(RasterRow::line_count).sum::<usize>();
                if tile_start == first_tile {
                    let hidden_lines = raster_rows
                        .iter()
                        .take(first_line.saturating_sub(tile_start))
                        .map(RasterRow::line_count)
                        .sum::<usize>();
                    tile_y -= (hidden_lines + visible_range.skip_display_lines) as f32
                        * minimap_line_height;
                }
                let tile_height = (tile_line_count as f32 * minimap_line_height)
                    .ceil()
                    .max(1.0);
                let key = tile_key(
                    tile_rows,
                    tile_start,
                    width,
                    folded_signature(&shape_model, tile_rows, &shape_folded),
                    wrap_hasher.finish(),
                    scale_factor,
                    density,
                );
                visible_keys.push(key);
                let (image, is_missing) = shape_model
                    .raster_tiles
                    .lock()
                    .expect("minimap raster tile cache poisoned")
                    .image_or_fallback(key);
                if let Some(image) = image {
                    tiles.push(RasterTilePaint {
                        image,
                        y: tile_y,
                        width: width as f32,
                        height: tile_height,
                    });
                }
                if is_missing {
                    raster_requests.push(RasterTileRequest {
                        key,
                        rows: raster_rows,
                    });
                }
                tile_y += tile_height;
            }
            let request_count = raster_requests.len();
            let request_keys = raster_requests
                .iter()
                .map(|request| request.key)
                .collect::<SmallVec<[_; 6]>>();
            let should_spawn = shape_model
                .raster_tiles
                .lock()
                .expect("minimap raster tile cache poisoned")
                .reserve(&request_keys);
            if should_spawn {
                let tile_model = shape_model.clone();
                let tile_folded = shape_folded.clone();
                let atomic_batch = request_count > 1;
                let background = cx.background_executor().spawn(async move {
                    let mut completed = Vec::with_capacity(raster_requests.len());
                    for request in raster_requests {
                        let rasterized = rasterize_tile(
                            &tile_model,
                            &request.rows,
                            width,
                            &tile_folded,
                            scale_factor,
                            density,
                        );
                        if minimap_perf_enabled() {
                            let first = !tile_model
                                .perf
                                .first_tile_completed
                                .swap(true, Ordering::AcqRel);
                            eprintln!(
                                "org_studio_minimap_tile_ready generation={} revision={} tile_start={} rows={} lines={} width={} total_ms={:.3} text_system_wait_ms={:.3} cold_text_system={} first={} since_open_ms={:.3}",
                                generation,
                                presentation_revision,
                                request.key.first_row,
                                request.rows.len(),
                                rasterized.line_count,
                                width,
                                rasterized.total.as_secs_f64() * 1000.0,
                                rasterized.text_system_wait.as_secs_f64() * 1000.0,
                                rasterized.cold_text_system,
                                first,
                                opened_at.elapsed().as_secs_f64() * 1000.0,
                            );
                        }
                        completed.push((request.key, rasterized.image));
                    }
                    let completed_count = completed.len();
                    tile_model
                        .raster_tiles
                        .lock()
                        .expect("minimap raster tile cache poisoned")
                        .insert_batch(completed, &visible_keys);
                    // Publish a complete visible minimap frame as one unit. The preview keeps its
                    // loading cover in place until this release store, so users never see the
                    // document appear first and the minimap fill one or two frames later.
                    tile_model
                        .initial_visible_batch_ready
                        .store(true, Ordering::Release);
                    if minimap_perf_enabled() && atomic_batch {
                        eprintln!(
                            "org_studio_minimap_tile_batch_ready tiles={} since_open_ms={:.3}",
                            completed_count,
                            opened_at.elapsed().as_secs_f64() * 1000.0,
                        );
                    }
                });
                cx.spawn(async move |cx| {
                    background.await;
                    cx.refresh();
                })
                .detach();
            }
            tiles
        },
        move |bounds, tiles: SmallVec<[RasterTilePaint; 6]>, window, cx| {
            profiling::scope!("Minimap::paint");
            if !tiles.is_empty()
                && !paint_model
                    .perf
                    .first_pixels_painted
                    .swap(true, Ordering::AcqRel)
            {
                if minimap_perf_enabled() {
                    eprintln!(
                        "org_studio_minimap_first_pixels generation={} revision={} tiles={} since_open_ms={:.3}",
                        generation,
                        presentation_revision,
                        tiles.len(),
                        opened_at.elapsed().as_secs_f64() * 1000.0,
                    );
                    eprintln!(
                        "org_preview_coherent_first_frame generation={} revision={} since_open_ms={:.3}",
                        generation,
                        presentation_revision,
                        opened_at.elapsed().as_secs_f64() * 1000.0,
                    );
                }
                if std::env::var_os("ORG_STUDIO_EXIT_AFTER_MINIMAP_FRAME").is_some() {
                    cx.quit();
                }
            }
            for tile in tiles {
                let image_bounds = Bounds::new(
                    point(bounds.origin.x, bounds.origin.y + px(tile.y)),
                    gpui::size(px(tile.width), px(tile.height)),
                );
                let _ = window.paint_image(
                    image_bounds,
                    image_bounds,
                    Corners::default(),
                    tile.image,
                    0,
                    false,
                );
            }
            let track_height = f32::from(bounds.size.height);
            let thumb = paint_line_index
                .lock()
                .expect("active line index poisoned")
                .as_ref()
                .map(|index| {
                    minimap_viewport_for_list_with_anchor(
                        index,
                        &paint_list,
                        track_height,
                        *paint_anchor.lock().expect("minimap anchor poisoned"),
                    )
                    .thumb
                })
                .unwrap_or_default();
            let thumb_bounds = Bounds::new(
                point(bounds.origin.x, bounds.origin.y + px(thumb.top)),
                gpui::size(bounds.size.width, px(thumb.height)),
            );
            let active = paint_drag
                .lock()
                .expect("minimap drag state poisoned")
                .is_some();
            let hovered = paint_hovered.load(Ordering::Relaxed);
            let (fill_alpha, border_alpha) = thumb_alphas(
                active,
                hovered,
                thumb_visibility == crate::settings::MinimapThumbVisibility::Hover,
            );
            window.paint_quad(fill(
                thumb_bounds,
                rgba((theme.foreground << 8) | fill_alpha),
            ));
            window.paint_quad(outline(
                thumb_bounds,
                rgba((theme.foreground << 8) | border_alpha),
                BorderStyle::default(),
            ));

            // Register drag listeners on the window, not the minimap hitbox. GPUI keeps
            // delivering native drag events to the originating window after the pointer
            // leaves this element, so horizontal escape does not cancel the gesture.
            let move_drag = event_drag.clone();
            let move_resize = event_resize.clone();
            let move_width_change = event_width_change.clone();
            let move_anchor = event_anchor.clone();
            let move_index = event_line_index.clone();
            let move_list = event_list.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _cx| {
                if phase != DispatchPhase::Bubble || !event.dragging() {
                    return;
                }
                if let Some(session) = current_resize_session(&move_resize) {
                    let width = width_from_resize_drag(
                        editor_width,
                        session,
                        f32::from(event.position.x),
                    );
                    move_width_change(MinimapWidthChange::Preview(width), window, _cx);
                    window.set_window_cursor_style(CursorStyle::ResizeLeftRight);
                    return;
                }
                let session = *move_drag.lock().expect("minimap drag state poisoned");
                let Some(session) = session else {
                    return;
                };
                let index = move_index
                    .lock()
                    .expect("active line index poisoned")
                    .clone();
                let Some(index) = index else {
                    return;
                };
                let local_y = f32::from(event.position.y - bounds.origin.y);
                let current_anchor = *move_anchor.lock().expect("minimap anchor poisoned");
                let viewport = minimap_viewport_for_list_with_anchor(
                    &index,
                    &move_list,
                    f32::from(bounds.size.height),
                    current_anchor,
                );
                let raw_thumb_top = local_y - session.grab_offset;
                let (target, desired_thumb_top) = minimap_drag_target(
                    local_y,
                    session.grab_offset,
                    viewport.thumb.height,
                    viewport.interaction_height,
                );
                if minimap_trace_enabled() {
                    eprintln!(
                        "minimap drag y={local_y:.2} grab={:.2} raw_top={raw_thumb_top:.2} current_ratio={:.5} target_ratio={target:.5}",
                        session.grab_offset,
                        viewport.scroll_ratio,
                    );
                }
                scroll_list_to_ratio(&index, &move_list, target);
                if current_anchor.is_some_and(|anchor| anchor.matches(&index, viewport)) {
                    let travel = (viewport.interaction_height - viewport.thumb.height).max(0.0);
                    let visible_minimap_lines = ((viewport.interaction_height
                        - index.density.edge_padding() * 2.0)
                        / index.density.line_height())
                        .max(1.0);
                    let max_content_top =
                        (index.total as f32 - visible_minimap_lines).max(0.0);
                    let content_top = if raw_thumb_top < 0.0 || raw_thumb_top > travel {
                        target * max_content_top
                    } else {
                        viewport.content_top
                    };
                    *move_anchor.lock().expect("minimap anchor poisoned") =
                        Some(MinimapInteractionAnchor {
                            width: index.width,
                            rows_signature: index.rows_signature,
                            interaction_height: viewport.interaction_height,
                            scroll_ratio: target,
                            content_top,
                            thumb_top: desired_thumb_top,
                        });
                }
                window.refresh();
            });

            let up_drag = event_drag.clone();
            let up_resize = event_resize.clone();
            let up_width_change = event_width_change.clone();
            let up_list = event_list.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                    return;
                }
                // `take_resize_session` drops the mutex guard before invoking the
                // callback. Commit clears all interaction state and therefore locks
                // this same mutex again.
                if let Some(session) = take_resize_session(&up_resize) {
                    let width = width_from_resize_drag(
                        editor_width,
                        session,
                        f32::from(event.position.x),
                    );
                    up_width_change(MinimapWidthChange::Commit(width), window, cx);
                    window.refresh();
                    return;
                }
                if up_drag
                    .lock()
                    .expect("minimap drag state poisoned")
                    .take()
                    .is_some()
                {
                    up_list.scrollbar_drag_ended();
                    window.refresh();
                }
            });
        },
    )
    .size_full();

    let down_bounds = track_bounds;
    let down_line_index = active_line_index.clone();
    let down_drag = drag_session.clone();
    let down_anchor = interaction_anchor.clone();
    let scroll_anchor = interaction_anchor.clone();
    let scroll_drag = drag_session;
    let scroll_line_index = active_line_index;
    let scroll_list = list_state.clone();
    let down_list = list_state.clone();
    let handle_resize = resize_session;
    let handle_width_change = on_width_change;
    let hover_state = hovered;
    div()
        .id("document-minimap")
        .h_full()
        .w(px(minimap_width))
        .flex_none()
        .relative()
        .overflow_hidden()
        .border_l_1()
        .border_color(gpui::rgb(theme.border))
        .bg(gpui::rgb(theme.background_alt))
        .cursor_pointer()
        .on_hover(move |is_hovered, window, _| {
            if hover_state.swap(*is_hovered, Ordering::Relaxed) != *is_hovered {
                window.refresh();
            }
        })
        .on_mouse_down(MouseButton::Left, move |event, window, _cx| {
            let bounds = *down_bounds.lock().expect("minimap bounds poisoned");
            let local_y = f32::from(event.position.y - bounds.origin.y);
            let track_height = f32::from(bounds.size.height);
            let index = down_line_index
                .lock()
                .expect("active line index poisoned")
                .clone();
            let Some(index) = index else {
                return;
            };
            let viewport = minimap_viewport_for_list_with_anchor(
                &index,
                &down_list,
                track_height,
                *down_anchor.lock().expect("minimap anchor poisoned"),
            );
            let thumb = viewport.thumb;
            let clicked_thumb = local_y >= thumb.top && local_y <= thumb.top + thumb.height;
            let grab_offset;
            if minimap_trace_enabled() {
                eprintln!(
                    "minimap down y={local_y:.2} thumb_top={:.2} thumb_h={:.2} extent={:.2} inside={clicked_thumb} content_top={:.2}",
                    thumb.top,
                    thumb.height,
                    viewport.interaction_height,
                    viewport.content_top,
                );
            }
            if !clicked_thumb {
                let target =
                    minimap_click_target_for_viewport(&index, &down_list, viewport, local_y);
                *down_anchor.lock().expect("minimap anchor poisoned") =
                    Some(MinimapInteractionAnchor {
                        width: index.width,
                        rows_signature: index.rows_signature,
                        interaction_height: viewport.interaction_height,
                        scroll_ratio: target.ratio,
                        content_top: viewport.content_top,
                        thumb_top: target.thumb_top,
                    });
                if minimap_trace_enabled() {
                    eprintln!(
                        "minimap click display={:.3} target_ratio={:.5} anchor_content={:.3} anchor_thumb={:.2}",
                        target.clicked_display,
                        target.ratio,
                        viewport.content_top,
                        target.thumb_top,
                    );
                }
                if target.ratio >= 1.0 - f32::EPSILON && down_list.item_count() > 0 {
                    down_list.scroll_to(ListOffset {
                        item_ix: down_list.item_count() - 1,
                        offset_in_item: px(0.0),
                    });
                } else {
                    down_list.scroll_to(target.offset);
                }
                grab_offset = (local_y - target.thumb_top).clamp(0.0, target.thumb_height);
                window.refresh();
            } else {
                grab_offset = (local_y - thumb.top).clamp(0.0, thumb.height);
                *down_anchor.lock().expect("minimap anchor poisoned") =
                    Some(MinimapInteractionAnchor {
                        width: index.width,
                        rows_signature: index.rows_signature,
                        interaction_height: viewport.interaction_height,
                        scroll_ratio: viewport.scroll_ratio,
                        content_top: viewport.content_top,
                        thumb_top: viewport.thumb.top,
                    });
            }
            down_list.scrollbar_drag_started();
            *down_drag.lock().expect("minimap drag state poisoned") =
                Some(MinimapDragSession { grab_offset });
        })
        .on_scroll_wheel(move |event, window, cx| {
            let delta_y = f32::from(event.delta.pixel_delta(px(SCROLL_WHEEL_LINE_PX)).y);
            if delta_y.abs() <= f32::EPSILON {
                return;
            }
            cx.stop_propagation();
            if scroll_drag
                .lock()
                .expect("minimap drag state poisoned")
                .is_some()
            {
                return;
            }
            let index = scroll_line_index
                .lock()
                .expect("active line index poisoned")
                .clone();
            let Some(index) = index else {
                return;
            };
            scroll_anchor
                .lock()
                .expect("minimap anchor poisoned")
                .take();
            let target = scroll_ratio_after_wheel(&index, &scroll_list, delta_y);
            scroll_list_to_ratio(&index, &scroll_list, target);
            window.refresh();
        })
        .child(content)
        .child(
            div()
                .id("document-minimap-resize-handle")
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(MINIMAP_RESIZE_HANDLE_PX))
                .cursor(CursorStyle::ResizeLeftRight)
                .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    cx.stop_propagation();
                    if event.click_count >= 2 {
                        handle_resize
                            .lock()
                            .expect("minimap resize state poisoned")
                            .take();
                        handle_width_change(MinimapWidthChange::Reset, window, cx);
                    } else {
                        *handle_resize
                            .lock()
                            .expect("minimap resize state poisoned") =
                            Some(MinimapResizeSession {
                                start_pointer_x: f32::from(event.position.x),
                                start_width: minimap_width,
                            });
                        handle_width_change(
                            MinimapWidthChange::Preview(minimap_width),
                            window,
                            cx,
                        );
                    }
                    window.set_window_cursor_style(CursorStyle::ResizeLeftRight);
                    window.refresh();
                }),
        )
}

#[cfg(test)]
fn hit_test_minimap_row(rows: &[MinimapHitRow], y: f32) -> Option<MinimapHitRow> {
    rows.iter()
        .find(|row| y >= row.top && y < row.bottom)
        .copied()
}

fn thumb_alphas(active: bool, hovered: bool, hover_only: bool) -> (u32, u32) {
    if active {
        (0x48, 0xe8)
    } else if hovered {
        (0x34, 0xd0)
    } else if hover_only {
        (0, 0)
    } else {
        (0x24, 0xb0)
    }
}

#[cfg(test)]
fn local_y_ratio(pointer_y: gpui::Pixels, bounds: Bounds<gpui::Pixels>) -> f32 {
    let height = f32::from(bounds.size.height);
    if height <= 0.0 {
        0.0
    } else {
        (f32::from(pointer_y - bounds.origin.y) / height).clamp(0.0, 1.0)
    }
}

pub(super) fn seek_to_ratio(list_state: &ListState, ratio: f32, center: bool) {
    let metrics = scroll_metrics(list_state);
    let mut offset = ratio.clamp(0.0, 1.0) * metrics.max_offset;
    if center {
        offset = (offset - metrics.viewport * 0.5).clamp(0.0, metrics.max_offset);
    }
    // Use the list's scrollbar protocol. It maps the exact measured/estimated
    // SumTree height to a ListOffset and honors the height frozen at drag start.
    list_state.set_offset_from_scrollbar(point(px(0.0), px(-offset)));
}

#[cfg(test)]
fn thumb_geometry(list_state: &ListState, total_lines: usize, track_height: f32) -> ThumbGeometry {
    let metrics = scroll_metrics(list_state);
    thumb_geometry_for_document(metrics, total_lines, track_height)
}

#[cfg(test)]
fn minimap_layout_from_metrics(
    total_lines: usize,
    minimap_capacity: usize,
    track_height: f32,
    metrics: ScrollMetrics,
) -> MinimapLayout {
    if total_lines == 0 || track_height <= 0.0 {
        return MinimapLayout::default();
    }
    let progress = if metrics.max_offset > 0.0 {
        (metrics.offset / metrics.max_offset).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let first_line =
        (progress * total_lines.saturating_sub(minimap_capacity) as f32).round() as usize;
    MinimapLayout {
        first_line,
        thumb: thumb_geometry_for_document(metrics, total_lines, track_height),
    }
}

fn scroll_metrics(list_state: &ListState) -> ScrollMetrics {
    ScrollMetrics {
        offset: -f32::from(list_state.scroll_px_offset_for_scrollbar().y),
        max_offset: f32::from(list_state.max_offset_for_scrollbar().y).max(0.0),
        viewport: f32::from(list_state.viewport_bounds().size.height).max(0.0),
    }
}

#[cfg(test)]
fn thumb_geometry_for_document(
    metrics: ScrollMetrics,
    total_lines: usize,
    track_height: f32,
) -> ThumbGeometry {
    minimap_viewport(metrics, total_lines, track_height).thumb
}

#[cfg(test)]
fn minimap_layout_from_range(
    total_lines: usize,
    visible_start: usize,
    visible_end: usize,
    minimap_capacity: usize,
    track_height: f32,
) -> MinimapLayout {
    if total_lines == 0 || track_height <= 0.0 {
        return MinimapLayout::default();
    }
    let start = visible_start.min(total_lines - 1);
    let end = visible_end.max(start + 1).min(total_lines);
    let visible_lines = end - start;
    let non_visible_lines = total_lines.saturating_sub(visible_lines);
    let scroll_progress = if non_visible_lines > 0 {
        (start as f32 / non_visible_lines as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let first_line =
        (scroll_progress * total_lines.saturating_sub(minimap_capacity) as f32).round() as usize;
    let raw_top =
        MINIMAP_EDGE_PADDING_PX + start.saturating_sub(first_line) as f32 * MINIMAP_LINE_HEIGHT_PX;
    let raw_height = visible_lines as f32 * MINIMAP_LINE_HEIGHT_PX;
    let height = raw_height.max(MIN_THUMB_PX).min(track_height);
    let top = raw_top.min((track_height - height).max(0.0));
    MinimapLayout {
        first_line,
        thumb: ThumbGeometry { top, height },
    }
}

#[cfg(test)]
fn thumb_geometry_from_metrics(
    scroll: f32,
    max_offset: f32,
    viewport: f32,
    track_height: f32,
) -> ThumbGeometry {
    let track_height = track_height.max(0.0);
    let max_offset = max_offset.max(0.0);
    let viewport = viewport.max(0.0);
    let content = max_offset + viewport;
    if content <= 0.0 || track_height <= 0.0 {
        return ThumbGeometry::default();
    }
    let scroll = scroll.clamp(0.0, max_offset);
    let raw_height = track_height * viewport / content;
    let height = raw_height.max(MIN_THUMB_PX).min(track_height);
    let travel = (track_height - height).max(0.0);
    let top = if max_offset > 0.0 {
        travel * scroll / max_offset
    } else {
        0.0
    };
    ThumbGeometry { top, height }
}

fn row_kind(
    format: DocumentFormat,
    blocks: &BlockArena,
    markdown_blocks: &[MarkdownBlock],
    block_id: u32,
) -> PreviewLineKind {
    match format {
        DocumentFormat::Org => match &blocks.nodes()[block_id as usize].kind {
            BlockKind::Heading { level } => PreviewLineKind::Heading((*level).min(4) as u8),
            BlockKind::ListItem => PreviewLineKind::List,
            BlockKind::QuoteBlock => PreviewLineKind::Quote,
            BlockKind::SourceBlock { .. } | BlockKind::ExampleBlock => PreviewLineKind::Code,
            BlockKind::TableRow => PreviewLineKind::Table,
            BlockKind::Image { .. } => PreviewLineKind::Image,
            BlockKind::HorizontalRule => PreviewLineKind::Rule,
            BlockKind::BlankLine => PreviewLineKind::Blank,
            _ => PreviewLineKind::Text,
        },
        DocumentFormat::Markdown => match &markdown_blocks[block_id as usize].kind {
            MarkdownKind::Blank => PreviewLineKind::Blank,
            MarkdownKind::Heading { level } => PreviewLineKind::Heading((*level).min(4) as u8),
            MarkdownKind::ListItem => PreviewLineKind::List,
            MarkdownKind::Quote => PreviewLineKind::Quote,
            MarkdownKind::Code { .. } => PreviewLineKind::Code,
            MarkdownKind::TableRow => PreviewLineKind::Table,
            MarkdownKind::Image { .. } => PreviewLineKind::Image,
            MarkdownKind::HorizontalRule => PreviewLineKind::Rule,
            _ => PreviewLineKind::Text,
        },
    }
}

fn code_language<'a>(
    format: DocumentFormat,
    blocks: &'a BlockArena,
    markdown_blocks: &'a [MarkdownBlock],
    block_id: u32,
) -> Option<&'a str> {
    match format {
        DocumentFormat::Org => {
            let block = &blocks.nodes()[block_id as usize];
            let BlockKind::SourceBlock { language } = &block.kind else {
                return None;
            };
            language.as_deref()
        }
        DocumentFormat::Markdown => {
            let block = &markdown_blocks[block_id as usize];
            let MarkdownKind::Code { language, .. } = &block.kind else {
                return None;
            };
            language.as_deref()
        }
    }
}

fn kind_color(kind: PreviewLineKind) -> u32 {
    let theme = current_theme();
    match kind {
        PreviewLineKind::Heading(level) => theme.heading[level.saturating_sub(1).min(3) as usize],
        PreviewLineKind::Code => theme.code_boundary,
        PreviewLineKind::Table => theme.meta,
        PreviewLineKind::Quote => theme.quote,
        PreviewLineKind::List => theme.foreground,
        PreviewLineKind::Image => theme.attribute,
        PreviewLineKind::Rule => theme.border,
        PreviewLineKind::Text | PreviewLineKind::Blank => theme.foreground,
    }
}

fn minimap_runs(runs: DisplayRuns) -> DisplayRuns {
    runs
}

fn slice_display_runs(runs: &DisplayRuns, range: Range<usize>) -> DisplayRuns {
    let start = range.start.min(runs.text.len());
    let end = range.end.min(runs.text.len()).max(start);
    DisplayRuns {
        text: runs.text[start..end].to_owned().into(),
        inline_spans: runs
            .inline_spans
            .iter()
            .filter_map(|span| {
                (span.range.start < end && span.range.end > start).then(|| {
                    let mut span = span.clone();
                    span.range =
                        span.range.start.max(start) - start..span.range.end.min(end) - start;
                    span
                })
            })
            .collect::<Vec<_>>()
            .into(),
        code_spans: runs
            .code_spans
            .iter()
            .filter_map(|span| {
                (span.start < end && span.end > start).then_some(CodeHighlightSpan {
                    start: span.start.max(start) - start,
                    end: span.end.min(end) - start,
                    kind: span.kind,
                })
            })
            .collect::<Vec<_>>()
            .into(),
    }
}

fn minimap_text_runs(
    kind: PreviewLineKind,
    line: &DisplayRuns,
    base_font: gpui::Font,
) -> Vec<TextRun> {
    let mut boundaries =
        Vec::with_capacity((line.inline_spans.len() + line.code_spans.len()) * 2 + 2);
    boundaries.push(0);
    boundaries.push(line.text.len());
    for span in line.inline_spans.iter() {
        boundaries.push(span.range.start.min(line.text.len()));
        boundaries.push(span.range.end.min(line.text.len()));
    }
    for span in line.code_spans.iter() {
        boundaries.push(span.start.min(line.text.len()));
        boundaries.push(span.end.min(line.text.len()));
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    boundaries
        .windows(2)
        .filter_map(|range| {
            let start = range[0];
            let end = range[1];
            (start < end).then(|| {
                let kinds = line
                    .inline_spans
                    .iter()
                    .filter(|span| span.range.start < end && span.range.end > start)
                    .map(|span| span.kind);
                let mut run_font = base_font.clone();
                let mut color: gpui::Hsla = gpui::rgb(kind_color(kind)).into();
                let mut strikethrough = None;
                let mut underline = None;
                for kind in kinds {
                    match kind {
                        InlineKind::Bold => run_font.weight = FontWeight::BOLD,
                        InlineKind::Italic => run_font.style = gpui::FontStyle::Italic,
                        InlineKind::Strike => strikethrough = Some(Default::default()),
                        InlineKind::Code | InlineKind::Verbatim => {
                            color = gpui::rgb(current_theme().code_foreground).into()
                        }
                        InlineKind::Link | InlineKind::FootnoteReference => {
                            color = gpui::rgb(current_theme().link).into()
                        }
                        InlineKind::Timestamp => color = gpui::rgb(current_theme().date).into(),
                        InlineKind::Target | InlineKind::RadioTarget => {
                            color = gpui::rgb(current_theme().attribute).into()
                        }
                        InlineKind::Entity | InlineKind::Latex => {
                            color = gpui::rgb(current_theme().constant).into()
                        }
                        InlineKind::Underline => underline = Some(Default::default()),
                    }
                }
                if let Some(span) = line
                    .code_spans
                    .iter()
                    .find(|span| span.start < end && span.end > start)
                {
                    let style = code_highlight_style(span.kind);
                    if let Some(syntax_color) = style.color {
                        color = syntax_color;
                    }
                    if let Some(font_style) = style.font_style {
                        run_font.style = font_style;
                    }
                }
                TextRun {
                    len: end - start,
                    font: run_font,
                    color,
                    background_color: None,
                    underline,
                    strikethrough,
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_line_index(
        width: u16,
        rows_signature: u64,
        density: MinimapDensity,
        display_prefix: &[usize],
        pixel_prefix: &[f32],
    ) -> MinimapLineIndex {
        assert_eq!(display_prefix.len(), pixel_prefix.len());
        let measures = display_prefix
            .windows(2)
            .zip(pixel_prefix.windows(2))
            .map(|(display, pixels)| {
                ProjectionMeasure::new(display[1] - display[0], pixels[1] - pixels[0], true)
            })
            .collect();
        let projection = Arc::new(ProjectionSnapshot::new(measures));
        MinimapLineIndex {
            width,
            rows_signature,
            density,
            total: projection.total_display_lines(),
            projection,
        }
    }

    #[test]
    fn minimap_width_matches_render_constraints() {
        assert_eq!(width_for_viewport(100.0, None), 24.0);
        assert!((width_for_viewport(400.0, None) - 60.0).abs() < 0.001);
        assert!((width_for_viewport(2_000.0, None) - 175.2).abs() < 0.001);
        assert_eq!(width_for_viewport(4_000.0, None), 220.0);
        assert_eq!(width_for_viewport(800.0, Some(220)), 160.0);
        assert_eq!(width_for_viewport(2_000.0, Some(220)), 220.0);
        assert_eq!(manual_width_for_viewport(800.0, 300.0), 160.0);
        assert_eq!(manual_width_for_viewport(1_200.0, 480.0), 240.0);
        assert!((manual_width_for_viewport(1_500.0, 480.0) - 351.5625).abs() < 0.001);
        assert_eq!(manual_width_for_viewport(2_000.0, 600.0), 480.0);
        assert_eq!(manual_width_for_viewport(2_000.0, 20.0), 48.0);
        let resize = MinimapResizeSession {
            start_pointer_x: 1_800.0,
            start_width: 160.0,
        };
        assert_eq!(width_from_resize_drag(2_000.0, resize, 1_760.0), 200.0);
        assert_eq!(width_from_resize_drag(2_000.0, resize, 1_900.0), 60.0);
        assert_eq!(width_from_resize_drag(800.0, resize, 1_000.0), 160.0);
        assert_eq!(MinimapDensity::for_width(96.0), MinimapDensity::Compact);
        assert_eq!(
            MinimapDensity::for_width(160.0),
            MinimapDensity::Comfortable
        );
        assert_eq!(MinimapDensity::for_width(220.0), MinimapDensity::Large);
        assert_eq!(MinimapDensity::for_width(320.0), MinimapDensity::ExtraLarge);
        assert_eq!(MinimapDensity::for_width(440.0), MinimapDensity::Maximum);
        assert_eq!(MinimapDensity::Compact.font_px(), 2.0);
        assert_eq!(MinimapDensity::Comfortable.font_px(), 2.4);
        assert_eq!(MinimapDensity::Large.font_px(), 2.8);
        assert_eq!(MinimapDensity::ExtraLarge.font_px(), 3.2);
        assert_eq!(MinimapDensity::Maximum.font_px(), 3.6);
    }

    #[test]
    fn taking_resize_session_releases_lock_before_commit_callback() {
        let state = Mutex::new(Some(MinimapResizeSession {
            start_pointer_x: 100.0,
            start_width: 160.0,
        }));
        let session = take_resize_session(&state).expect("active resize session");
        assert_eq!(session.start_width, 160.0);
        let mut guard = state
            .try_lock()
            .expect("commit callback must be able to re-enter interaction cleanup");
        *guard = None;
    }

    #[test]
    fn projection_snapshot_publishes_exact_chunks_without_rebuilding_unchanged_leaves() {
        let estimates = (0..600)
            .map(|_| ProjectionMeasure::new(1, 24.0, false))
            .collect();
        let initial = ProjectionSnapshot::new(estimates);
        assert_eq!(initial.chunks.len(), 3);
        assert_eq!(initial.exact_rows, 0);
        assert_eq!(initial.total_display_lines(), 600);

        let first_chunk = initial.chunks[0].clone();
        let last_chunk = initial.chunks[2].clone();
        let updated = initial.replacing(&[
            (255, ProjectionMeasure::new(3, 72.0, true)),
            (256, ProjectionMeasure::new(2, 48.0, true)),
        ]);

        assert!(!Arc::ptr_eq(&first_chunk, &updated.chunks[0]));
        assert!(!Arc::ptr_eq(&initial.chunks[1], &updated.chunks[1]));
        assert!(Arc::ptr_eq(&last_chunk, &updated.chunks[2]));
        assert_eq!(updated.exact_rows, 2);
        assert_eq!(updated.total_display_lines(), 603);
        assert_eq!(updated.total_pixels(), 600.0 * 24.0 + 72.0);
        assert_eq!(updated.locate_display(255), (255, 0));
        assert_eq!(updated.locate_display(257), (255, 2));
        assert_eq!(updated.locate_display(258), (256, 0));
        assert_eq!(updated.prefix_for_row(257), (260, 6240.0));
    }

    #[test]
    fn projection_snapshot_prefix_and_reverse_lookup_match_a_naive_model() {
        let measures = (0..10_000)
            .map(|row| ProjectionMeasure::new(row % 5 + 1, (row % 7 + 1) as f32 * 3.0, false))
            .collect::<Vec<_>>();
        let projection = ProjectionSnapshot::new(measures.clone());
        let mut display = 0usize;
        let mut pixels = 0.0f32;
        for (row, measure) in measures.iter().enumerate() {
            assert_eq!(projection.prefix_for_row(row), (display, pixels));
            assert_eq!(projection.locate_display(display), (row, 0));
            assert_eq!(projection.locate_pixel(pixels), (row, 0.0));
            display += measure.display_lines as usize;
            pixels += measure.pixels;
        }
        assert_eq!(projection.prefix_for_row(measures.len()), (display, pixels));
        assert_eq!(projection.total_display_lines(), display);
        assert_eq!(projection.total_pixels(), pixels);

        let fifty_mib_fixture_rows = ProjectionSnapshot::new(
            (0..341_392)
                .map(|_| ProjectionMeasure::new(1, 24.0, false))
                .collect(),
        );
        assert!(fifty_mib_fixture_rows.estimated_heap_bytes() <= 8 * 1024 * 1024);
    }

    #[test]
    fn projection_scheduler_prioritizes_the_current_view_before_sequential_work() {
        let row_count = 2_000;
        let projection = Arc::new(ProjectionSnapshot::new(
            (0..row_count)
                .map(|_| ProjectionMeasure::new(1, 24.0, false))
                .collect(),
        ));
        let mut builder = MinimapLineIndexBuilder {
            key: MinimapLineIndexKey {
                presentation_rows: 1,
                width: 800,
                density: MinimapDensity::Compact,
            },
            presentation_rows: Arc::new((0..row_count).collect()),
            rows_signature: 1,
            sequential_cursor: 0,
            priority_range: 0..0,
            priority_cursor: 0,
            exact_bits: vec![0; row_count.div_ceil(64)],
            exact_rows: 0,
            started_at: Instant::now(),
            projection,
            pending_updates: Vec::new(),
            slices: 0,
            work: Duration::ZERO,
            max_slice: Duration::ZERO,
        };

        builder.prioritize(1_000);
        assert_eq!(builder.next_candidate(), Some(872));
        builder.mark_exact(872);
        assert_eq!(builder.next_candidate(), Some(873));

        builder.prioritize(1_600);
        assert_eq!(builder.next_candidate(), Some(1_472));
        for row in 1_472..1_856 {
            builder.mark_exact(row);
        }
        builder.priority_cursor = builder.priority_range.end;
        assert_eq!(builder.next_candidate(), Some(0));
    }

    #[test]
    fn larger_density_expands_the_projection_and_visible_thumb_span() {
        let mut index = test_line_index(100, 1, MinimapDensity::Compact, &[0, 100], &[0.0, 100.0]);
        assert_eq!(
            minimap_projection_height(100, 1_000.0, index.density),
            268.0
        );
        assert_eq!(
            minimap_thumb_height_for_scroll(&index, 0.0, 50.0, 1_000.0),
            130.0
        );
        index.density = MinimapDensity::Large;
        assert_eq!(
            minimap_projection_height(100, 1_000.0, index.density),
            392.0
        );
        assert_eq!(
            minimap_thumb_height_for_scroll(&index, 0.0, 50.0, 1_000.0),
            190.0
        );
    }

    #[test]
    fn display_line_index_locates_wrapped_rows_without_changing_units() {
        let index = test_line_index(
            800,
            1,
            MinimapDensity::Compact,
            &[0, 1, 4, 6],
            &[0.0, 24.0, 96.0, 144.0],
        );
        assert_eq!(index.locate(0), (0, 0));
        assert_eq!(index.locate(1), (1, 0));
        assert_eq!(index.locate(3), (1, 2));
        assert_eq!(index.locate(4), (2, 0));
        assert_eq!(index.locate(5), (2, 1));
        assert_eq!(
            index.pixel_for_list_offset(index.list_offset_for_pixel(0.0)),
            0.0
        );
        assert_eq!(
            index.pixel_for_list_offset(index.list_offset_for_pixel(72.0)),
            72.0
        );
        assert_eq!(
            index.pixel_for_list_offset(index.list_offset_for_pixel(144.0)),
            144.0
        );
    }

    #[test]
    fn minimap_content_hit_test_uses_rendered_row_heights() {
        let rows = [
            MinimapHitRow {
                top: 4.0,
                bottom: 6.6,
                presentation_index: 17,
                offset_in_item: 0.0,
            },
            MinimapHitRow {
                top: 6.6,
                bottom: 9.2,
                presentation_index: 18,
                offset_in_item: 0.0,
            },
            MinimapHitRow {
                top: 9.2,
                bottom: 11.8,
                presentation_index: 18,
                offset_in_item: 24.0,
            },
        ];

        let hit = hit_test_minimap_row(&rows, 4.0).expect("first row");
        assert_eq!(hit.presentation_index, 17);
        assert_eq!(hit.offset_in_item, 0.0);
        let wrapped_hit = hit_test_minimap_row(&rows, 10.0).expect("wrapped line");
        assert_eq!(wrapped_hit.presentation_index, 18);
        assert_eq!(wrapped_hit.offset_in_item, 24.0);
        assert_eq!(hit_test_minimap_row(&rows, 3.99), None);
        assert_eq!(hit_test_minimap_row(&rows, 11.8), None);
    }

    #[test]
    fn display_window_expands_wraps_and_bottom_aligns() {
        let counts = [1, 3, 1, 2, 1];
        assert_eq!(
            display_window_range(counts.len(), 0, 0, 4, 0, |i| counts[i]),
            DisplayWindow {
                rows: 0..2,
                skip_display_lines: 0,
            }
        );
        assert_eq!(
            display_window_range(counts.len(), 4, 0, 4, 3, |i| counts[i]),
            DisplayWindow {
                rows: 2..5,
                skip_display_lines: 0,
            }
        );
        assert_eq!(
            display_window_range(counts.len(), 2, 0, 3, 1, |i| counts[i]),
            DisplayWindow {
                rows: 1..4,
                skip_display_lines: 2,
            }
        );
        assert_eq!(
            display_window_range(counts.len(), 1, 2, 3, 1, |i| counts[i]),
            DisplayWindow {
                rows: 1..3,
                skip_display_lines: 1,
            }
        );
        for (anchor, inner, offset) in [(0, 0, 0), (2, 0, 1), (1, 2, 1), (4, 0, 3)] {
            let window =
                display_window_range(counts.len(), anchor, inner, 4, offset, |i| counts[i]);
            let anchor_local_line = (counts[window.rows.start..anchor].iter().sum::<usize>()
                + inner)
                .saturating_sub(window.skip_display_lines);
            assert_eq!(anchor_local_line, offset);
        }
    }

    #[test]
    fn display_run_slices_rebase_inline_and_syntax_ranges() {
        let runs = DisplayRuns {
            text: "0123456789".into(),
            inline_spans: Arc::from([InlineSpan {
                source: 2..8,
                range: 2..8,
                kind: InlineKind::Bold,
            }]),
            code_spans: Arc::from([CodeHighlightSpan {
                start: 4,
                end: 9,
                kind: super::super::CodeHighlightKind::String,
            }]),
        };
        let sliced = slice_display_runs(&runs, 5..9);
        assert_eq!(sliced.text.as_ref(), "5678");
        assert_eq!(sliced.inline_spans[0].range, 0..3);
        assert_eq!(
            (sliced.code_spans[0].start, sliced.code_spans[0].end),
            (0, 4)
        );
    }

    fn empty_runs(text: &'static str) -> DisplayRuns {
        DisplayRuns {
            text: text.into(),
            inline_spans: Arc::from([]),
            code_spans: Arc::from([]),
        }
    }

    #[test]
    fn display_run_cache_is_strictly_bounded() {
        let mut cache = DisplayRunCache {
            entries: HashMap::with_capacity(DisplayRunCache::CAPACITY),
            order: VecDeque::with_capacity(DisplayRunCache::CAPACITY),
        };
        for row in 0..DisplayRunCache::CAPACITY + 17 {
            cache.insert(row, empty_runs("row"));
        }
        assert_eq!(cache.entries.len(), DisplayRunCache::CAPACITY);
        assert_eq!(cache.order.len(), DisplayRunCache::CAPACITY);
        assert!(!cache.entries.contains_key(&0));
        assert!(
            cache
                .entries
                .contains_key(&(DisplayRunCache::CAPACITY + 16))
        );
    }

    #[test]
    fn raster_tile_key_rejects_fold_and_resize_reuse() {
        let density = MinimapDensity::Compact;
        let original = tile_key(&[0, 1, 2, 3], 0, 96, 0, 10, 2.0, density);
        let folded = tile_key(&[0, 3], 0, 96, 1, 10, 2.0, density);
        let resized = tile_key(&[0, 1, 2, 3], 0, 72, 0, 11, 2.0, density);
        let rewrapped = tile_key(&[0, 1, 2, 3], 0, 96, 0, 12, 2.0, density);
        let next_tile = tile_key(&[128, 129], 128, 96, 0, 10, 2.0, density);
        let other_scale = tile_key(&[0, 1, 2, 3], 0, 96, 0, 10, 1.0, density);
        let comfortable = tile_key(
            &[0, 1, 2, 3],
            0,
            96,
            0,
            10,
            2.0,
            MinimapDensity::Comfortable,
        );
        assert_ne!(original, folded);
        assert_ne!(original, resized);
        assert_ne!(original, rewrapped);
        assert_ne!(original, next_tile);
        assert_ne!(original, other_scale);
        assert_ne!(original, comfortable);
    }

    #[test]
    fn raster_tile_cache_and_in_flight_sets_are_bounded_by_design() {
        assert_eq!(RASTER_TILE_ROWS, 128);
        assert_eq!(RasterTileCache::CAPACITY, 6);
        let tile_bytes = 96.0 * (RASTER_TILE_ROWS as f32 * MINIMAP_LINE_HEIGHT_PX).ceil() * 4.0;
        assert!(tile_bytes * (RasterTileCache::CAPACITY as f32) < 1_000_000.0);
        let retina_maximum_bytes = MINIMAP_MANUAL_MAX_PX
            * (RASTER_TILE_ROWS as f32 * MinimapDensity::Maximum.line_height()).ceil()
            * 4.0
            * 4.0
            * RasterTileCache::CAPACITY as f32;
        assert!(retina_maximum_bytes < 30_000_000.0);

        let first = tile_key(&[0], 0, 96, 0, 1, 2.0, MinimapDensity::Compact);
        let second = tile_key(&[128], 128, 96, 0, 1, 2.0, MinimapDensity::Compact);
        let mut cache = RasterTileCache {
            entries: HashMap::new(),
            order: VecDeque::new(),
            in_flight: HashSet::new(),
        };
        assert!(cache.image_or_fallback(first).1);
        assert!(cache.image_or_fallback(second).1);
        assert!(cache.reserve(&[first, second]));
        assert_eq!(cache.in_flight.len(), 2);
        assert!(!cache.reserve(&[second]));
    }

    #[test]
    fn complete_visible_batch_is_published_without_internal_eviction() {
        let density = MinimapDensity::Compact;
        let keys = (0..RasterTileCache::CAPACITY + 1)
            .map(|index| {
                let row = index * RASTER_TILE_ROWS;
                tile_key(&[row], row, 96, 0, 1, 2.0, density)
            })
            .collect::<Vec<_>>();
        let mut cache = RasterTileCache {
            entries: HashMap::new(),
            order: VecDeque::new(),
            in_flight: HashSet::new(),
        };
        assert!(cache.reserve(&keys));
        let completed = keys
            .iter()
            .copied()
            .map(|key| {
                let pixels = RgbaImage::new(1, 1);
                let image = Arc::new(RenderImage::new(SmallVec::from_elem(Frame::new(pixels), 1)));
                (key, image)
            })
            .collect();
        cache.insert_batch(completed, &keys);

        assert_eq!(cache.entries.len(), keys.len());
        assert!(cache.in_flight.is_empty());
        assert!(keys.iter().all(|key| cache.entries.contains_key(key)));
    }

    #[gpui::test]
    fn real_list_state_uses_exact_variable_row_heights_and_resize(cx: &mut gpui::TestAppContext) {
        use gpui::{AppContext, Context, IntoElement, Render, Styled, Window, list, size};

        let cx = cx.add_empty_window();
        let heights = Arc::new([24.0_f32, 240.0, 48.0, 300.0, 24.0]);
        let state = ListState::new(heights.len(), gpui::ListAlignment::Top, px(0.0)).measure_all();

        struct VariableRows {
            state: ListState,
            heights: Arc<[f32; 5]>,
        }
        impl Render for VariableRows {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let heights = self.heights.clone();
                list(self.state.clone(), move |index, _, _| {
                    div().h(px(heights[index])).w_full().into_any()
                })
                .size_full()
            }
        }

        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(100.0), px(100.0)),
            |_, cx| {
                cx.new(|_| VariableRows {
                    state: state.clone(),
                    heights: heights.clone(),
                })
                .into_any_element()
            },
        );

        let index = test_line_index(
            100,
            1,
            MinimapDensity::Compact,
            &[0, 10, 120, 140, 280, 300],
            &[0.0, 24.0, 264.0, 312.0, 612.0, 636.0],
        );
        state.scroll_to(index.list_offset_for_pixel(200.0));
        let wheel_down = scroll_ratio_after_wheel(&index, &state, -40.0);
        let wheel_up = scroll_ratio_after_wheel(&index, &state, 40.0);
        assert!((wheel_down - 240.0 / 536.0).abs() < 0.001);
        assert!((wheel_up - 160.0 / 536.0).abs() < 0.001);
        state.scroll_to(index.list_offset_for_pixel(0.0));
        assert_eq!(scroll_ratio_after_wheel(&index, &state, 200.0), 0.0);
        state.scroll_to(index.list_offset_for_pixel(536.0));
        assert_eq!(scroll_ratio_after_wheel(&index, &state, -200.0), 1.0);

        scroll_list_to_ratio(&index, &state, 1.0);
        let bottom = minimap_viewport_for_list(&index, &state, 500.0);
        assert_eq!(bottom.scroll_ratio, 1.0);
        assert!((bottom.thumb.top + bottom.thumb.height - 500.0).abs() < 0.001);

        state.scroll_to(ListOffset::default());
        let before_content_click = minimap_viewport_for_list(&index, &state, 500.0);
        let content_target =
            minimap_click_target_for_viewport(&index, &state, before_content_click, 250.0);
        let clicked_offset = index.list_offset_for_display_position(content_target.clicked_display);
        let clicked_pixel = index.pixel_for_list_offset(clicked_offset);
        state.scroll_to(content_target.offset);
        let anchor = MinimapInteractionAnchor {
            width: index.width,
            rows_signature: index.rows_signature,
            interaction_height: before_content_click.interaction_height,
            scroll_ratio: content_target.ratio,
            content_top: before_content_click.content_top,
            thumb_top: content_target.thumb_top,
        };
        let after_content_click =
            minimap_viewport_for_list_with_anchor(&index, &state, 500.0, Some(anchor));
        assert_eq!(
            after_content_click.content_top, before_content_click.content_top,
            "the content under a small-window click must not be replaced"
        );
        assert!(
            (after_content_click.thumb.top + after_content_click.thumb.height * 0.5 - 250.0).abs()
                < 0.001
        );
        assert!(
            (index.pixel_for_list_offset(content_target.offset) + 50.0 - clicked_pixel).abs()
                < 0.001,
            "the clicked minimap content must land at the left viewport center"
        );

        let short_projection = test_line_index(
            100,
            2,
            MinimapDensity::Compact,
            &[0, 1, 3, 4, 8, 9],
            &[0.0, 24.0, 264.0, 312.0, 612.0, 636.0],
        );
        scroll_list_to_ratio(&short_projection, &state, 1.0);
        let short_bottom = minimap_viewport_for_list(&short_projection, &state, 500.0);
        let projected_height = 9.0 * MINIMAP_LINE_HEIGHT_PX + MINIMAP_EDGE_PADDING_PX * 2.0;
        assert_eq!(short_bottom.scroll_ratio, 1.0);
        assert!(
            (short_bottom.thumb.top + short_bottom.thumb.height - projected_height).abs() < 0.001,
            "a fullscreen thumb must stop at the document projection, not in track whitespace"
        );

        let max = f32::from(state.max_offset_for_scrollbar().y);
        assert_eq!(max, 536.0);
        seek_to_ratio(&state, 1.0, false);
        assert_eq!(-f32::from(state.scroll_px_offset_for_scrollbar().y), max);
        let compact = thumb_geometry(&state, heights.len(), 500.0);

        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(100.0), px(200.0)),
            |_, cx| {
                cx.new(|_| VariableRows {
                    state: state.clone(),
                    heights: heights.clone(),
                })
                .into_any_element()
            },
        );
        let tall = thumb_geometry(&state, heights.len(), 500.0);
        assert_eq!(tall.height, compact.height);
        assert_eq!(tall.height, MIN_THUMB_PX);

        // Model folding a heading subtree by replacing three presentation
        // rows (including the 300px image-like row) with one placeholder.
        state.splice(1..4, 1);
        state.clone().measure_all();
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(100.0), px(100.0)),
            |_, cx| {
                cx.new(|_| VariableRows {
                    state: state.clone(),
                    heights: heights.clone(),
                })
                .into_any_element()
            },
        );
        assert_eq!(state.item_count(), 3);
        seek_to_ratio(&state, 1.0, false);
        let folded_max = f32::from(state.max_offset_for_scrollbar().y);
        assert_eq!(
            -f32::from(state.scroll_px_offset_for_scrollbar().y),
            folded_max
        );
    }

    #[test]
    fn sampled_text_is_bounded_and_uses_zed_scale() {
        assert_eq!(MINIMAP_FONT_PX, 2.0);
    }

    #[test]
    fn pixel_scroll_metrics_drive_content_and_thumb_together() {
        let metrics = ScrollMetrics {
            offset: 4_500.0,
            max_offset: 9_000.0,
            viewport: 1_000.0,
        };
        let layout = minimap_layout_from_metrics(1_000, 200, 500.0, metrics);
        assert_eq!(layout.first_line, 400);
        assert!((layout.thumb.height - 108.333_336).abs() < 0.001);
        assert!((layout.thumb.top - 195.833_33).abs() < 0.001);
    }

    #[test]
    fn thumb_height_does_not_change_when_virtual_height_converges() {
        // 400 rows deliberately stays below the old 4096-row branch. The
        // viewport extent must not depend on ListState's progressively refined
        // max offset for ordinary documents either.
        let before_measurement = thumb_geometry_for_document(
            ScrollMetrics {
                offset: 1_000.0,
                max_offset: 4_000.0,
                viewport: 1_000.0,
            },
            400,
            500.0,
        );
        let after_measurement = thumb_geometry_for_document(
            ScrollMetrics {
                offset: 2_000.0,
                max_offset: 8_000.0,
                viewport: 1_000.0,
            },
            400,
            500.0,
        );
        assert_eq!(before_measurement, after_measurement);
    }

    #[test]
    fn fullscreen_uses_one_coordinate_space_without_a_blank_prefix() {
        let before_list_layout = minimap_viewport(
            ScrollMetrics {
                offset: 4_000.0,
                max_offset: 8_000.0,
                viewport: 800.0,
            },
            400,
            1_400.0,
        );
        let after_list_layout = minimap_viewport(
            ScrollMetrics {
                offset: 3_800.0,
                max_offset: 7_600.0,
                viewport: 1_200.0,
            },
            400,
            1_400.0,
        );

        // The whole 400-line projection fits in the 538-line minimap. Resize
        // cannot scroll its content into blank space, before or after ListState
        // commits the new viewport measurement.
        assert_eq!(before_list_layout.content_top, 0.0);
        assert_eq!(after_list_layout.content_top, 0.0);
        assert!(before_list_layout.thumb.top >= 0.0);
        assert!(after_list_layout.thumb.top >= 0.0);
    }

    #[test]
    fn upward_scroll_keeps_content_window_inside_document() {
        let viewport = 800.0;
        let track = 520.0;
        let total = 1_000;
        let mut previous_top = f32::MAX;
        for step in (0..=20).rev() {
            let progress = step as f32 / 20.0;
            let layout = minimap_viewport(
                ScrollMetrics {
                    offset: progress * 9_000.0,
                    max_offset: 9_000.0,
                    viewport,
                },
                total,
                track,
            );
            let capacity = track / MINIMAP_LINE_HEIGHT_PX;
            assert!(layout.content_top >= 0.0);
            assert!(layout.content_top <= total as f32 - capacity + f32::EPSILON);
            assert!(layout.content_top <= previous_top);
            assert!(layout.thumb.top >= 0.0);
            assert!(layout.thumb.top + layout.thumb.height <= track + f32::EPSILON);
            previous_top = layout.content_top;
        }
        assert_eq!(previous_top, 0.0);
    }

    #[test]
    fn bottom_alignment_uses_total_display_lines_not_source_rows() {
        // 400 source rows expand to 900 wrapped display lines. At the bottom,
        // the minimap window must end at display line 900 exactly.
        let track = 520.0;
        let capacity = track / MINIMAP_LINE_HEIGHT_PX;
        let layout = minimap_viewport(
            ScrollMetrics {
                offset: 9_000.0,
                max_offset: 9_000.0,
                viewport: 800.0,
            },
            900,
            track,
        );
        assert!((layout.content_top + capacity - 900.0).abs() < 0.001);
        assert!((layout.thumb.top + layout.thumb.height - track).abs() < 0.001);
    }

    #[test]
    fn thumb_visual_state_has_clear_hover_and_active_steps() {
        let idle = thumb_alphas(false, false, false);
        let hovered = thumb_alphas(false, true, false);
        let active = thumb_alphas(true, true, false);
        assert!(idle.0 < hovered.0 && hovered.0 < active.0);
        assert!(idle.1 < hovered.1 && hovered.1 < active.1);
        assert_eq!(thumb_alphas(false, false, true), (0, 0));
    }

    #[test]
    fn thumb_mapping_reaches_both_ends_for_variable_height_content() {
        let start = thumb_geometry_from_metrics(0.0, 9000.0, 1000.0, 500.0);
        let middle = thumb_geometry_from_metrics(4500.0, 9000.0, 1000.0, 500.0);
        let end = thumb_geometry_from_metrics(9000.0, 9000.0, 1000.0, 500.0);
        assert_eq!(
            start,
            ThumbGeometry {
                top: 0.0,
                height: 50.0
            }
        );
        assert_eq!(
            middle,
            ThumbGeometry {
                top: 225.0,
                height: 50.0
            }
        );
        assert_eq!(
            end,
            ThumbGeometry {
                top: 450.0,
                height: 50.0
            }
        );
    }

    #[test]
    fn minimum_thumb_height_keeps_the_end_reachable() {
        let end = thumb_geometry_from_metrics(99_900.0, 99_900.0, 100.0, 500.0);
        assert_eq!(end.height, MIN_THUMB_PX);
        assert_eq!(end.top + end.height, 500.0);
    }

    #[test]
    fn zed_layout_uses_one_coordinate_for_content_and_thumb() {
        let top = minimap_layout_from_range(1000, 0, 100, 200, 520.0);
        let middle = minimap_layout_from_range(1000, 450, 550, 200, 520.0);
        let end = minimap_layout_from_range(1000, 900, 1000, 200, 520.0);

        assert_eq!(top.first_line, 0);
        assert_eq!(
            top.thumb,
            ThumbGeometry {
                top: 4.0,
                height: 260.0
            }
        );
        assert_eq!(middle.first_line, 400);
        assert_eq!(
            middle.thumb,
            ThumbGeometry {
                top: 134.0,
                height: 260.0
            }
        );
        assert_eq!(end.first_line, 800);
        assert_eq!(end.thumb.top + end.thumb.height, 520.0);
    }

    #[test]
    fn short_document_does_not_fake_scroll_the_minimap_content() {
        let layout = minimap_layout_from_range(100, 70, 100, 200, 520.0);
        assert_eq!(layout.first_line, 0);
        assert_eq!(
            layout.thumb,
            ThumbGeometry {
                top: 186.0,
                height: 78.0
            }
        );
    }

    #[test]
    fn empty_single_line_and_tiny_track_degrade_safely() {
        assert_eq!(
            minimap_layout_from_range(0, 0, 0, 1, 2.0),
            MinimapLayout::default()
        );
        let single = minimap_layout_from_range(1, 0, 1, 1, 2.0);
        assert_eq!(single.first_line, 0);
        assert_eq!(
            single.thumb,
            ThumbGeometry {
                top: 0.0,
                height: 2.0
            }
        );
    }

    #[test]
    fn pointer_ratio_is_relative_to_minimap_not_window() {
        let bounds = Bounds::new(point(px(1500.0), px(80.0)), gpui::size(px(92.0), px(600.0)));
        assert_eq!(local_y_ratio(px(80.0), bounds), 0.0);
        assert_eq!(local_y_ratio(px(380.0), bounds), 0.5);
        assert_eq!(local_y_ratio(px(680.0), bounds), 1.0);
        assert_eq!(local_y_ratio(px(40.0), bounds), 0.0);
        assert_eq!(local_y_ratio(px(900.0), bounds), 1.0);
    }

    #[test]
    fn dragging_is_the_exact_inverse_of_thumb_travel() {
        assert_eq!(minimap_drag_target(360.0, 40.0, 80.0, 720.0), (0.5, 320.0));
        assert_eq!(minimap_drag_target(-100.0, 40.0, 80.0, 720.0), (0.0, 0.0));
        assert_eq!(minimap_drag_target(900.0, 40.0, 80.0, 720.0), (1.0, 640.0));
    }

    #[test]
    fn absolute_drag_keeps_the_grab_point_and_clamps_outside_the_track() {
        let thumb_height = 80.0;
        let track_height = 320.0;
        let grab_offset = 20.0;
        assert_eq!(
            minimap_drag_target(140.0, grab_offset, thumb_height, track_height),
            (0.5, 120.0)
        );
        assert_eq!(
            minimap_drag_target(-40.0, grab_offset, thumb_height, track_height),
            (0.0, 0.0)
        );
        assert_eq!(
            minimap_drag_target(400.0, grab_offset, thumb_height, track_height),
            (1.0, 240.0)
        );
    }

    #[gpui::test]
    fn window_drag_listener_survives_leaving_the_minimap_hitbox(cx: &mut gpui::TestAppContext) {
        use gpui::{AppContext, Context, Modifiers, Render, Window, size};

        let cx = cx.add_empty_window();
        let active = Arc::new(AtomicBool::new(false));
        let move_positions = Arc::new(Mutex::new(Vec::<f32>::new()));

        struct DragHarness {
            active: Arc<AtomicBool>,
            move_positions: Arc<Mutex<Vec<f32>>>,
        }

        impl Render for DragHarness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let down_active = self.active.clone();
                let event_active = self.active.clone();
                let event_positions = self.move_positions.clone();
                div()
                    .size_full()
                    .on_mouse_down(MouseButton::Left, move |_, _, _| {
                        down_active.store(true, Ordering::Release);
                    })
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |_, _, window, _| {
                                let move_active = event_active.clone();
                                let move_positions = event_positions.clone();
                                window.on_mouse_event(
                                    move |event: &MouseMoveEvent, phase, _, _| {
                                        if phase == DispatchPhase::Bubble
                                            && event.dragging()
                                            && move_active.load(Ordering::Acquire)
                                        {
                                            move_positions
                                                .lock()
                                                .expect("drag test positions poisoned")
                                                .push(f32::from(event.position.x));
                                        }
                                    },
                                );
                            },
                        )
                        .size_full(),
                    )
            }
        }

        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(100.0), px(100.0)),
            |_, cx| {
                cx.new(|_| DragHarness {
                    active: active.clone(),
                    move_positions: move_positions.clone(),
                })
                .into_any_element()
            },
        );
        cx.simulate_mouse_down(
            point(px(50.0), px(50.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(px(180.0), px(140.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        assert_eq!(
            move_positions
                .lock()
                .expect("drag test positions poisoned")
                .as_slice(),
            [180.0]
        );
    }

    #[test]
    fn thumb_height_uses_the_exact_visible_display_span() {
        let index = test_line_index(
            100,
            1,
            MinimapDensity::Compact,
            &[0, 100, 101, 201],
            &[0.0, 100.0, 400.0, 500.0],
        );

        let dense_text = minimap_thumb_height_for_scroll(&index, 0.0, 100.0, 500.0);
        let tall_block = minimap_thumb_height_for_scroll(&index, 150.0, 100.0, 500.0);
        let final_text = minimap_thumb_height_for_scroll(&index, 400.0, 100.0, 500.0);

        assert_eq!(dense_text, 260.0);
        assert_eq!(tall_block, MIN_THUMB_PX);
        assert_eq!(final_text, 260.0);
        let (top, bottom) = minimap_visible_display_range(&index, 0.0, 100.0);
        assert_eq!((top, bottom), (0.0, 100.0));
    }

    #[test]
    fn inline_display_runs_cover_the_same_rendered_text() {
        let parsed = parse_document_inline(
            DocumentFormat::Markdown,
            "plain **bold** *italic* and [link](target)",
        );
        let line = DisplayRuns {
            text: parsed.text.into(),
            inline_spans: parsed.spans.into(),
            code_spans: Arc::from([]),
        };
        let mut base_font = font("Menlo");
        base_font.weight = FontWeight::BLACK;
        let runs = minimap_text_runs(PreviewLineKind::Text, &line, base_font);
        assert_eq!(
            runs.iter().map(|run| run.len).sum::<usize>(),
            line.text.len()
        );
        assert!(runs.len() > 1);
        assert!(
            runs.iter()
                .any(|run| run.font.style == gpui::FontStyle::Italic)
        );
    }

    #[test]
    fn minimap_line_limit_preserves_utf8_boundaries() {
        let source = "中".repeat(1100);
        let truncated = truncate_for_minimap(&source);
        assert_eq!(truncated.graphemes(true).count(), 1024);
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn minimap_font_has_black_weight_and_cjk_fallback() {
        let font = minimap_font();
        assert_eq!(font.weight, FontWeight::BLACK);
        let fallbacks = font.fallbacks.expect("fallbacks");
        assert!(
            fallbacks
                .fallback_list()
                .iter()
                .any(|name| name == "PingFang SC")
        );
        assert!(
            fallbacks
                .fallback_list()
                .iter()
                .any(|name| name == "Apple Color Emoji")
        );
    }

    #[test]
    fn viewport_border_exceeds_three_to_one_contrast() {
        let theme = current_theme();
        let border = composite_rgb(theme.foreground, theme.background_alt, 0xb0 as f32 / 255.0);
        assert!(contrast_ratio(border, theme.background_alt) >= 3.0);
    }

    #[test]
    fn truncation_preserves_combining_clusters_and_rtl_text() {
        let combining = format!("{}tail", "e\u{301}".repeat(1024));
        let truncated = truncate_for_minimap(&combining);
        assert!(truncated.ends_with('\u{301}'));
        assert_eq!(truncated.graphemes(true).count(), 1024);

        let rtl = "مرحبا بالعالم";
        assert_eq!(truncate_for_minimap(rtl), rtl);
    }
}

#[cfg(test)]
fn composite_rgb(foreground: u32, background: u32, alpha: f32) -> u32 {
    let channel = |shift: u32| {
        let foreground = ((foreground >> shift) & 0xffu32) as f32;
        let background = ((background >> shift) & 0xffu32) as f32;
        (foreground * alpha + background * (1.0 - alpha)).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

#[cfg(test)]
fn contrast_ratio(first: u32, second: u32) -> f32 {
    let luminance = |color: u32| {
        let linear = |value: u32| {
            let value = value as f32 / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear((color >> 16) & 0xff)
            + 0.7152 * linear((color >> 8) & 0xff)
            + 0.0722 * linear(color & 0xff)
    };
    let first = luminance(first);
    let second = luminance(second);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}
