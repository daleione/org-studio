use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::{Hash, Hasher},
    ops::Range,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use gpui::RenderImage;
use image::{Frame, RgbaImage};
use smallvec::SmallVec;
use unicode_width::UnicodeWidthStr;

use crate::{
    org_syntax::inline::InlineKind,
    preview::{PreviewStyle, table::Alignment},
};

use super::{
    DisplayLines, DisplayRuns, MinimapDensity, PreviewDisplayMap, PreviewLineKind,
    RASTER_TILE_CACHE_CAPACITY, kind_color, minimap_perf_enabled, minimap_runs,
    scene::{
        PrimitiveWidth, VisualContent as RowContent, VisualPrimitive as RowPrimitive,
        resolve_visual_row,
    },
    slice_display_runs,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RasterTileKey {
    /// Tile position in the current presentation. Stable across a local fold even when the rows
    /// occupying this slot change.
    pub(crate) tile_start: usize,
    /// Stable visual identity of the first row, used for exact-content fallback across reflow.
    pub(crate) first_row_id: usize,
    pub(crate) row_signature: u64,
    pub(crate) width: u16,
    pub(crate) theme_signature: u64,
    pub(crate) folded_signature: u64,
    pub(crate) wrap_signature: u64,
    pub(crate) scale_factor_x100: u16,
    pub(crate) density: MinimapDensity,
}

pub(crate) struct RasterTileCache {
    pub(crate) entries: HashMap<RasterTileKey, Arc<RenderImage>>,
    pub(crate) order: VecDeque<RasterTileKey>,
    pub(crate) in_flight: HashSet<RasterTileKey>,
}

impl RasterTileCache {
    pub(crate) const CAPACITY: usize = RASTER_TILE_CACHE_CAPACITY;

    pub(crate) fn image_or_fallback(&self, key: RasterTileKey) -> (Option<Arc<RenderImage>>, bool) {
        if let Some(image) = self.entries.get(&key).cloned() {
            return (Some(image), false);
        }
        let fallback = self
            .order
            .iter()
            .rev()
            .find(|candidate| {
                candidate.first_row_id == key.first_row_id
                    && candidate.row_signature == key.row_signature
                    && candidate.theme_signature == key.theme_signature
                    && candidate.folded_signature == key.folded_signature
                    && candidate.scale_factor_x100 == key.scale_factor_x100
            })
            .or_else(|| {
                // Folding replaces a complete visible batch. Keep the image previously painted
                // in the same tile slot until that batch is ready, rather than exposing an empty
                // canvas between the two frames. Style changes retain a whole paint frame in
                // MinimapState because their row geometry may differ.
                self.order.iter().rev().find(|candidate| {
                    candidate.tile_start == key.tile_start
                        && candidate.width == key.width
                        && candidate.theme_signature == key.theme_signature
                        && candidate.scale_factor_x100 == key.scale_factor_x100
                        && candidate.density == key.density
                })
            })
            .and_then(|candidate| self.entries.get(candidate).cloned());
        (fallback, true)
    }

    pub(crate) fn reserve(&mut self, keys: &[RasterTileKey]) -> bool {
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

    pub(crate) fn insert_batch(
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
pub(crate) struct RasterTilePaint {
    pub(crate) image: Arc<RenderImage>,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

pub(crate) struct RasterizedTile {
    pub(crate) image: Arc<RenderImage>,
    pub(crate) line_count: usize,
    pub(crate) total: Duration,
    pub(crate) text_system_wait: Duration,
    pub(crate) cold_text_system: bool,
}

pub(crate) struct RasterTileRequest {
    pub(crate) key: RasterTileKey,
    pub(crate) rows: Vec<RasterRow>,
}

#[derive(Clone)]
pub(crate) struct RasterRow {
    pub(crate) document_index: usize,
    pub(crate) lines: DisplayLines,
}

impl RasterRow {
    pub(crate) fn line_count(&self) -> usize {
        self.lines.ranges.len().max(1)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn tile_key(
    rows: &[usize],
    tile_start: usize,
    width: usize,
    folded_signature: u64,
    wrap_signature: u64,
    scale_factor: f32,
    density: MinimapDensity,
    style: PreviewStyle,
) -> RasterTileKey {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rows.hash(&mut hasher);
    RasterTileKey {
        tile_start,
        first_row_id: rows.first().copied().unwrap_or(0),
        row_signature: hasher.finish(),
        width: width.min(u16::MAX as usize) as u16,
        theme_signature: style.paint_key(),
        folded_signature,
        wrap_signature,
        scale_factor_x100: (scale_factor * 100.0).round().clamp(1.0, u16::MAX as f32) as u16,
        density,
    }
}

pub(crate) fn folded_signature(
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
pub(crate) struct DisplayWindow {
    pub(crate) rows: Range<usize>,
    pub(crate) skip_display_lines: usize,
}

pub(crate) fn display_window_range(
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

pub(crate) fn cosmic_color(rgb: u32) -> cosmic_text::Color {
    cosmic_text::Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub(crate) fn syntax_color(kind: crate::preview::CodeHighlightKind, style: PreviewStyle) -> u32 {
    let palette = style.palette;
    match kind {
        crate::preview::CodeHighlightKind::Attribute => palette.attribute,
        crate::preview::CodeHighlightKind::Boolean
        | crate::preview::CodeHighlightKind::Constant => palette.constant,
        crate::preview::CodeHighlightKind::Comment => palette.comment,
        crate::preview::CodeHighlightKind::Function => palette.function,
        crate::preview::CodeHighlightKind::Keyword => palette.keyword,
        crate::preview::CodeHighlightKind::Number => palette.number,
        crate::preview::CodeHighlightKind::Operator
        | crate::preview::CodeHighlightKind::Punctuation => palette.operator,
        crate::preview::CodeHighlightKind::Property
        | crate::preview::CodeHighlightKind::Variable => palette.variable,
        crate::preview::CodeHighlightKind::String => palette.string,
        crate::preview::CodeHighlightKind::Type => palette.type_name,
    }
}

pub(crate) fn cosmic_runs(
    kind: PreviewLineKind,
    line: &DisplayRuns,
    preview_style: PreviewStyle,
) -> Vec<(&str, cosmic_text::Attrs<'static>)> {
    let family = if kind == PreviewLineKind::Code {
        preview_style.typography.code_family
    } else {
        preview_style.typography.body_family
    };
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
            let palette = preview_style.palette;
            let mut color = kind_color(kind, preview_style);
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
                    InlineKind::Code | InlineKind::Verbatim => color = palette.inline_code,
                    InlineKind::Link | InlineKind::FootnoteReference | InlineKind::Underline => {
                        color = palette.link
                    }
                    InlineKind::Timestamp => color = palette.date,
                    InlineKind::Target | InlineKind::RadioTarget => color = palette.attribute,
                    InlineKind::Entity | InlineKind::Latex => color = palette.constant,
                    InlineKind::Strike => {}
                }
            }
            if let Some(span) = line
                .code_spans
                .iter()
                .find(|span| span.start < end && span.end > start)
            {
                color = syntax_color(span.kind, preview_style);
                if matches!(span.kind, crate::preview::CodeHighlightKind::Comment) {
                    style = cosmic_text::Style::Italic;
                }
            }
            Some((
                &line.text[start..end],
                cosmic_text::Attrs::new()
                    .family(cosmic_text::Family::Name(family))
                    .weight(weight)
                    .style(style)
                    .color(cosmic_color(color)),
            ))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn fill_bgra(
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
pub(crate) static TEXT_RASTERIZER_PREWARMED: OnceLock<()> = OnceLock::new();

pub(crate) fn prewarm_text_rasterizer() {
    use cosmic_text::{Attrs, Buffer, Color, Family, Metrics, Shaping, Wrap};

    let started = Instant::now();
    let rasterizer = crate::minimap::text_rasterizer();
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
            "Org Markdown CJK \u{4e2d}\u{6587} AaZz 0123456789 +-*/_`#[](){} :=> ",
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn rasterize_tile(
    model: &PreviewDisplayMap,
    presentation_rows: &[RasterRow],
    width: usize,
    parent_width: f32,
    folded: &HashSet<u32>,
    scale_factor: f32,
    density: MinimapDensity,
    style: PreviewStyle,
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
    let cold_text_system = !crate::minimap::text_rasterizer_initialized();
    let rasterizer = crate::minimap::text_rasterizer();
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
            style,
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
                token.resolve(style),
            );
        }
        let attrs = Attrs::new()
            .family(Family::Name(if kind == PreviewLineKind::Code {
                style.typography.code_family
            } else {
                style.typography.body_family
            }))
            .weight(cosmic_text::Weight::BLACK);
        let base = Color::rgb((color >> 16) as u8, (color >> 8) as u8, color as u8);
        if let RowContent::Table(projection) = &scene.content {
            let table = crate::preview::table::project_table(
                projection.table(),
                parent_width,
                logical_width as f32,
            );
            let columns = &table.columns;
            let table_color = if projection.is_separator() {
                style.palette.border
            } else {
                style.palette.foreground_dim
            };
            if style.variants.table == crate::preview::style::TableVariant::Grid {
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
                        crate::minimap::paint_text_pixels(
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
            buffer.set_rich_text(
                cosmic_runs(kind, &segment, style),
                &attrs,
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(font_system, false);
            let segment_y = (line_slot as f32 * physical_line_height).round() as i32;
            buffer.draw(font_system, swash_cache, base, |x, y, w, h, color| {
                let x = x + indent;
                let y = y + segment_y;
                crate::minimap::paint_text_pixels(
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
