use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::{Hash, Hasher},
    ops::Range,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use gpui::RenderImage;
use image::{Frame, RgbaImage};
use smallvec::SmallVec;
use unicode_width::UnicodeWidthStr;

use crate::{
    org_syntax::inline::InlineKind,
    preview::{
        table::Alignment,
        visual_recipe::{
            PrimitiveWidth, VisualContent as RowContent, VisualPrimitive as RowPrimitive,
            resolve_visual_row,
        },
    },
    theme::current_theme,
};

use super::{
    DisplayLines, DisplayRuns, MINIMAP_AUTO_COMPACT_MAX_PX, MinimapDensity, PreviewDisplayMap,
    PreviewLineKind, RASTER_TILE_CACHE_CAPACITY, kind_color, minimap_perf_enabled, minimap_runs,
    slice_display_runs,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::preview) struct RasterTileKey {
    pub(in crate::preview) first_row: usize,
    pub(in crate::preview) row_signature: u64,
    pub(in crate::preview) width: u16,
    pub(in crate::preview) theme_signature: u64,
    pub(in crate::preview) folded_signature: u64,
    pub(in crate::preview) wrap_signature: u64,
    pub(in crate::preview) scale_factor_x100: u16,
    pub(in crate::preview) density: MinimapDensity,
}

pub(in crate::preview) struct RasterTileCache {
    pub(in crate::preview) entries: HashMap<RasterTileKey, Arc<RenderImage>>,
    pub(in crate::preview) order: VecDeque<RasterTileKey>,
    pub(in crate::preview) in_flight: HashSet<RasterTileKey>,
}

impl RasterTileCache {
    pub(in crate::preview) const CAPACITY: usize = RASTER_TILE_CACHE_CAPACITY;

    pub(in crate::preview) fn image_or_fallback(
        &self,
        key: RasterTileKey,
    ) -> (Option<Arc<RenderImage>>, bool) {
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

    pub(in crate::preview) fn reserve(&mut self, keys: &[RasterTileKey]) -> bool {
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

    pub(in crate::preview) fn insert_batch(
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
pub(in crate::preview) struct RasterTilePaint {
    pub(in crate::preview) image: Arc<RenderImage>,
    pub(in crate::preview) y: f32,
    pub(in crate::preview) width: f32,
    pub(in crate::preview) height: f32,
}

pub(in crate::preview) struct RasterizedTile {
    pub(in crate::preview) image: Arc<RenderImage>,
    pub(in crate::preview) line_count: usize,
    pub(in crate::preview) total: Duration,
    pub(in crate::preview) text_system_wait: Duration,
    pub(in crate::preview) cold_text_system: bool,
}

pub(in crate::preview) struct RasterTileRequest {
    pub(in crate::preview) key: RasterTileKey,
    pub(in crate::preview) rows: Vec<RasterRow>,
}

#[derive(Clone)]
pub(in crate::preview) struct RasterRow {
    pub(in crate::preview) document_index: usize,
    pub(in crate::preview) lines: DisplayLines,
}

impl RasterRow {
    pub(in crate::preview) fn line_count(&self) -> usize {
        self.lines.ranges.len().max(1)
    }
}

pub(in crate::preview) fn theme_signature() -> u64 {
    let theme = current_theme();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    theme.background_alt.hash(&mut hasher);
    theme.foreground.hash(&mut hasher);
    theme.code_background.hash(&mut hasher);
    theme.heading.hash(&mut hasher);
    hasher.finish()
}

pub(in crate::preview) fn tile_key(
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

pub(in crate::preview) fn folded_signature(
    model: &PreviewDisplayMap,
    rows: &[usize],
    folded: &HashSet<u32>,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for &row in rows {
        let block_id = model.source_row(row).block_id;
        if folded.contains(&block_id) {
            block_id.hash(&mut hasher);
        }
    }
    hasher.finish()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::preview) struct DisplayWindow {
    pub(in crate::preview) rows: Range<usize>,
    pub(in crate::preview) skip_display_lines: usize,
}

pub(in crate::preview) fn display_window_range(
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

pub(in crate::preview) fn cosmic_color(rgb: u32) -> cosmic_text::Color {
    cosmic_text::Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub(in crate::preview) fn syntax_color(kind: crate::preview::CodeHighlightKind) -> u32 {
    let theme = current_theme();
    match kind {
        crate::preview::CodeHighlightKind::Attribute => theme.attribute,
        crate::preview::CodeHighlightKind::Boolean
        | crate::preview::CodeHighlightKind::Constant => theme.constant,
        crate::preview::CodeHighlightKind::Comment => theme.comment,
        crate::preview::CodeHighlightKind::Function => theme.function,
        crate::preview::CodeHighlightKind::Keyword => theme.keyword,
        crate::preview::CodeHighlightKind::Number => theme.number,
        crate::preview::CodeHighlightKind::Operator
        | crate::preview::CodeHighlightKind::Punctuation => theme.operator,
        crate::preview::CodeHighlightKind::Property
        | crate::preview::CodeHighlightKind::Variable => theme.variable,
        crate::preview::CodeHighlightKind::String => theme.string,
        crate::preview::CodeHighlightKind::Type => theme.type_name,
    }
}

pub(in crate::preview) fn cosmic_runs(
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
                if matches!(span.kind, crate::preview::CodeHighlightKind::Comment) {
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

#[allow(clippy::too_many_arguments)]
pub(in crate::preview) fn fill_bgra(
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

#[allow(clippy::too_many_arguments)]
fn paint_text_pixels(
    pixels: &mut [u8],
    image_width: usize,
    image_height: usize,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    color: cosmic_text::Color,
) {
    for py in 0..height as i32 {
        let target_y = y + py;
        if target_y < 0 || target_y >= image_height as i32 {
            continue;
        }
        for px_offset in 0..width as i32 {
            let target_x = x + px_offset;
            if target_x < 0 || target_x >= image_width as i32 {
                continue;
            }
            let offset = (target_y as usize * image_width + target_x as usize) * 4;
            pixels[offset] = color.b();
            pixels[offset + 1] = color.g();
            pixels[offset + 2] = color.r();
            pixels[offset + 3] = color.a();
        }
    }
}

pub(in crate::preview) fn minimap_font_system() -> cosmic_text::FontSystem {
    cosmic_text::FontSystem::new()
}

pub(in crate::preview) type MinimapTextRasterizer =
    Mutex<(cosmic_text::FontSystem, cosmic_text::SwashCache)>;
pub(in crate::preview) static TEXT_RASTERIZER: OnceLock<MinimapTextRasterizer> = OnceLock::new();
pub(in crate::preview) static TEXT_RASTERIZER_PREWARMED: OnceLock<()> = OnceLock::new();

pub(in crate::preview) fn prewarm_text_rasterizer() {
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

pub(in crate::preview) fn prewarm_document_text(model: Arc<PreviewDisplayMap>) {
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
    for row in 0..model.projection.rows.len().min(384) {
        if started.elapsed() >= PREWARM_BUDGET {
            break;
        }
        let kind = model.row_kind(row);
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

pub(in crate::preview) fn rasterize_tile(
    model: &PreviewDisplayMap,
    presentation_rows: &[RasterRow],
    width: usize,
    parent_width: f32,
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
        let scene = resolve_visual_row(
            model,
            document_index,
            logical_width as f32,
            density.font_px(),
        );
        let kind = scene.kind;
        let block_id = model.source_row(document_index).block_id;
        let mut display_runs = minimap_runs(model.runs(document_index));
        if folded.contains(&block_id) && scene.folded_ellipsis {
            display_runs.text = format!("{} …", display_runs.text).into();
        }
        let color = scene.color;
        let row_y = (line_slot as f32 * physical_line_height).round() as usize;
        let row_visual_height =
            (raster_row.line_count() as f32 * physical_line_height).ceil() as usize;
        for primitive in &scene.primitives {
            let (x, y, primitive_width, primitive_height, token) = match *primitive {
                RowPrimitive::Rect { x, width, color } => {
                    (x, row_y, width, row_visual_height, color)
                }
                RowPrimitive::VerticalLine { x, color } => (
                    x,
                    row_y,
                    PrimitiveWidth::Fixed(1.0),
                    row_visual_height,
                    color,
                ),
                RowPrimitive::HorizontalLine { x, width, color } => (x, row_y + 1, width, 1, color),
            };
            fill_bgra(
                &mut pixels,
                width as usize,
                height as usize,
                (x * scale_factor).round() as usize,
                y,
                (primitive_width.resolve(logical_width as f32, x) * scale_factor)
                    .ceil()
                    .max(1.0) as usize,
                primitive_height,
                token.resolve(),
            );
        }
        let attrs = Attrs::new()
            .family(Family::Name("Menlo"))
            .weight(cosmic_text::Weight::BLACK);
        let base = Color::rgb((color >> 16) as u8, (color >> 8) as u8, color as u8);
        if let RowContent::Table(projection) = &scene.content {
            let theme = current_theme();
            let table = crate::preview::table::project_table(
                projection.table(),
                parent_width,
                logical_width as f32,
            );
            let columns = &table.columns;
            let table_color = if projection.is_separator() {
                theme.border
            } else {
                theme.foreground_dim
            };
            for separator_x in &table.separators {
                fill_bgra(
                    &mut pixels,
                    width as usize,
                    height as usize,
                    (*separator_x * scale_factor).round() as usize,
                    row_y,
                    1,
                    row_visual_height,
                    table_color,
                );
            }
            if projection.is_separator() {
                let end_x = table
                    .separators
                    .last()
                    .copied()
                    .unwrap_or(4.0)
                    .min(logical_width as f32 - 4.0);
                fill_bgra(
                    &mut pixels,
                    width as usize,
                    height as usize,
                    (4.0 * scale_factor).round() as usize,
                    row_y + row_visual_height / 2,
                    ((end_x - 4.0) * scale_factor).ceil().max(1.0) as usize,
                    1,
                    table_color,
                );
            } else {
                for (index, column) in columns.iter().enumerate() {
                    let Some(cell) = projection.cells().get(index) else {
                        continue;
                    };
                    let cell_text = cell.text(&display_runs.text);
                    let cell_width =
                        ((column.content_end_x - column.content_start_x) * scale_factor).max(1.0);
                    buffer.set_size(Some(cell_width), Some(physical_line_height));
                    buffer.set_text(cell_text, &attrs, Shaping::Advanced, None);
                    buffer.shape_until_scroll(font_system, false);
                    let estimated_text_width = UnicodeWidthStr::width(cell_text) as f32
                        * density.font_px()
                        * 0.62
                        * scale_factor;
                    let free = (cell_width - estimated_text_width).max(0.0);
                    let alignment = projection
                        .columns()
                        .get(index)
                        .map(|column| column.alignment())
                        .unwrap_or_default();
                    let align_offset = match alignment {
                        Alignment::Center => free * 0.5,
                        Alignment::Right => free,
                        Alignment::Left if cell.align_right() => free,
                        Alignment::Left => 0.0,
                    };
                    let origin_x =
                        (column.content_start_x * scale_factor + align_offset).round() as i32;
                    let segment_y = (line_slot as f32 * physical_line_height).round() as i32;
                    buffer.draw(font_system, swash_cache, base, |x, y, w, h, color| {
                        let x = x + origin_x;
                        let y = y + segment_y;
                        paint_text_pixels(
                            &mut pixels,
                            width as usize,
                            height as usize,
                            x,
                            y,
                            w,
                            h,
                            color,
                        );
                    });
                }
            }
            line_slot += 1;
            continue;
        }
        buffer.set_size(
            Some(width as f32 - 6.0 * scale_factor),
            Some(physical_line_height),
        );
        if matches!(scene.content, RowContent::None) {
            line_slot += raster_row.line_count();
            continue;
        }
        let indent = (scene.indent * scale_factor).round() as i32;
        for range in raster_row.lines.ranges.iter().cloned() {
            let segment = slice_display_runs(&display_runs, range);
            buffer.set_rich_text(cosmic_runs(kind, &segment), &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(font_system, false);
            let segment_y = (line_slot as f32 * physical_line_height).round() as i32;
            buffer.draw(font_system, swash_cache, base, |x, y, w, h, color| {
                let x = x + indent;
                let y = y + segment_y;
                paint_text_pixels(
                    &mut pixels,
                    width as usize,
                    height as usize,
                    x,
                    y,
                    w,
                    h,
                    color,
                );
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
