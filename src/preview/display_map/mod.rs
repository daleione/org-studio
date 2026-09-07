use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
    sync::{Arc, Mutex},
};

use gpui::FontFallbacks;
use gpui::{FontWeight, SharedString, TextRun, font, px};

use crate::{
    document::SharedTextSnapshot,
    org_semantic::OrgFileConfig,
    org_syntax::inline::{InlineKind, InlineSpan},
    preview::{
        CodeHighlightSpan, DocumentFormat, PreviewRow, PreviewSnapshot, code_highlight_style,
        highlight_code,
        org_line::{parse_heading_with_config, parse_list_item},
        parse_document_inline,
        projection::{ReadingCodeRow, ReadingProjection, VisualRowId, VisualRowKind},
        style::PreviewStyle,
        table::{ReadingTableLayout, TableRowProjection},
    },
};
#[cfg(test)]
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreviewLineKind {
    Blank,
    Text,
    Heading(u8),
    List,
    Caption,
    Quote,
    Code,
    Table,
    Image,
    Rule,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RowLayout {
    pub(crate) font_size: f32,
    pub(crate) line_height: f32,
    pub(crate) min_height: f32,
    pub(crate) padding_left: f32,
    pub(crate) padding_right: f32,
    pub(crate) padding_top: f32,
    pub(crate) padding_bottom: f32,
    pub(crate) margin_top: f32,
    pub(crate) margin_bottom: f32,
    pub(crate) fixed_height: Option<f32>,
}

impl RowLayout {
    pub(crate) const fn text(font_size: f32, line_height: f32) -> Self {
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

    pub(crate) fn scaled(self, scale: f32) -> Self {
        Self {
            font_size: self.font_size * scale,
            line_height: self.line_height * scale,
            min_height: self.min_height * scale,
            padding_left: self.padding_left * scale,
            padding_right: self.padding_right * scale,
            padding_top: self.padding_top * scale,
            padding_bottom: self.padding_bottom * scale,
            margin_top: self.margin_top * scale,
            margin_bottom: self.margin_bottom * scale,
            fixed_height: self.fixed_height.map(|height| height * scale),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DisplayRuns {
    pub(crate) text: SharedString,
    pub(crate) inline_spans: Arc<[InlineSpan]>,
    pub(crate) links: Arc<[InlineLink]>,
    pub(crate) code_spans: Arc<[CodeHighlightSpan]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InlineLink {
    pub(crate) range: Range<usize>,
    pub(crate) destination: Arc<str>,
}

pub(crate) struct PreviewDisplayMap {
    pub(crate) text: SharedTextSnapshot,
    pub(crate) format: DocumentFormat,
    pub(crate) projection: Arc<ReadingProjection>,
    pub(crate) org_config: Option<Arc<OrgFileConfig>>,
    pub(crate) display_runs: Mutex<DisplayRunCache>,
    pub(crate) display_lines: Mutex<DisplayLineCache>,
    table_layouts: Mutex<TableLayoutCache>,
}

impl PreviewDisplayMap {
    pub(crate) fn source_row(&self, row: usize) -> PreviewRow {
        self.projection
            .source_row(row)
            .expect("preview row range maps to the current revision")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DisplayLines {
    pub(crate) ranges: Arc<[Range<usize>]>,
    pub(crate) parent_height: f32,
    pub(crate) table: Option<ResolvedTableDisplay>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedTableDisplay {
    pub(crate) layout: Arc<ReadingTableLayout>,
    pub(crate) wrapped_cells: Arc<[Vec<String>]>,
}

pub(crate) struct DisplayLineCache {
    entries: HashMap<((VisualRowId, u64), u16, u16, u64), DisplayLines>,
    order: VecDeque<((VisualRowId, u64), u16, u16, u64)>,
}

struct TableLayoutCache {
    entries: HashMap<(crate::org_syntax::BlockId, u16, u16, u64), Arc<ReadingTableLayout>>,
    order: VecDeque<(crate::org_syntax::BlockId, u16, u16, u64)>,
}

impl TableLayoutCache {
    const CAPACITY: usize = 256;

    fn insert(
        &mut self,
        key: (crate::org_syntax::BlockId, u16, u16, u64),
        layout: Arc<ReadingTableLayout>,
    ) {
        if self.entries.contains_key(&key) {
            return;
        }
        while self.entries.len() >= Self::CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.order.push_back(key);
        self.entries.insert(key, layout);
    }
}

impl DisplayLineCache {
    const CAPACITY: usize = 4096;

    pub(crate) fn insert(&mut self, key: ((VisualRowId, u64), u16, u16, u64), lines: DisplayLines) {
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

pub(crate) struct DisplayRunCache {
    pub(crate) entries: HashMap<(VisualRowId, u64), DisplayRuns>,
    pub(crate) order: VecDeque<(VisualRowId, u64)>,
}

impl DisplayRunCache {
    pub(crate) const CAPACITY: usize = 4096;

    pub(crate) fn insert(&mut self, identity: (VisualRowId, u64), runs: DisplayRuns) {
        if self.entries.contains_key(&identity) {
            return;
        }
        while self.entries.len() >= Self::CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.order.push_back(identity);
        self.entries.insert(identity, runs);
    }
}

impl PreviewDisplayMap {
    pub(crate) fn reading_table_layout(
        &self,
        row: usize,
        available_width: f32,
        zoom: f32,
        style: PreviewStyle,
    ) -> Option<Arc<ReadingTableLayout>> {
        let projection = self.table_projection(row)?;
        // Use the same quantized identity as DisplayLineCache. Tiny sub-pixel width changes must
        // not make row measurement and the rendered table select different layout generations.
        let width_key = available_width.round().clamp(1.0, u16::MAX as f32) as u16;
        let zoom_key = (zoom * 1_000.0).round().clamp(1.0, u16::MAX as f32) as u16;
        let key = (
            projection.group_id(),
            width_key,
            zoom_key,
            style.layout_key(),
        );
        if let Some(layout) = self
            .table_layouts
            .lock()
            .expect("table layout cache poisoned")
            .entries
            .get(&key)
            .cloned()
        {
            return Some(layout);
        }
        // Resolve from the canonical values represented by the key. Otherwise whichever pane
        // first inserts a sub-pixel width would silently define the geometry for the other pane.
        let layout = projection.resolved_reading_layout(
            f32::from(width_key),
            f32::from(zoom_key) / 1_000.0,
            style,
        );
        self.table_layouts
            .lock()
            .expect("table layout cache poisoned")
            .insert(key, layout.clone());
        Some(layout)
    }

    pub(crate) fn runs(&self, row: usize) -> DisplayRuns {
        let visual_row = self
            .projection
            .rows
            .get(row)
            .expect("preview row index is in bounds");
        let identity = (visual_row.id, visual_row.semantic_revision);
        if let Some(runs) = self
            .display_runs
            .lock()
            .expect("display-run cache poisoned")
            .entries
            .get(&identity)
            .cloned()
        {
            return runs;
        }
        let runs = materialize_runs(self, row);
        self.display_runs
            .lock()
            .expect("display-run cache poisoned")
            .insert(identity, runs.clone());
        runs
    }

    pub(crate) fn is_heading(&self, row: usize) -> bool {
        self.projection
            .rows
            .get(row)
            .is_some_and(|_| matches!(self.row_kind(row), PreviewLineKind::Heading(_)))
    }

    pub(crate) fn display_lines(
        &self,
        row: usize,
        available_width: f32,
        zoom: f32,
        style: PreviewStyle,
        text_system: &gpui::WindowTextSystem,
    ) -> DisplayLines {
        let width_key = available_width.round().clamp(1.0, u16::MAX as f32) as u16;
        let zoom_key = (zoom * 1_000.0).round().clamp(1.0, u16::MAX as f32) as u16;
        let visual_row = self
            .projection
            .rows
            .get(row)
            .expect("preview row index is in bounds");
        let key = (
            (visual_row.id, visual_row.semantic_revision),
            width_key,
            zoom_key,
            style.layout_key(),
        );
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
        let kind = self.row_kind(row);
        let layout = self.layout(row, style).scaled(zoom);
        if kind == PreviewLineKind::Table {
            let table = self.table_projection(row);
            let resolved_table = table.map(|table| {
                let table_layout = self
                    .reading_table_layout(row, available_width, zoom, style)
                    .expect("table row has a resolved layout");
                let wrapped_cells = if table.is_separator() {
                    Vec::new()
                } else {
                    table.shaped_display_cells(
                        &display.text,
                        &table_layout,
                        zoom,
                        style,
                        text_system,
                    )
                };
                ResolvedTableDisplay {
                    layout: table_layout,
                    wrapped_cells: wrapped_cells.into(),
                }
            });
            let line_count = resolved_table
                .as_ref()
                .map(|table| {
                    table
                        .wrapped_cells
                        .iter()
                        .map(|lines| lines.len().max(1))
                        .max()
                        .unwrap_or(1)
                })
                .unwrap_or(1);
            let lines = DisplayLines {
                ranges: std::iter::once(0..display.text.len())
                    .collect::<Vec<_>>()
                    .into(),
                parent_height: if table.is_some_and(TableRowProjection::is_separator) {
                    2.0 + layout.margin_top + layout.margin_bottom
                } else {
                    (line_count as f32 * layout.line_height
                        + style.spacing.table_cell_y * 2.0 * zoom)
                        .max(layout.min_height)
                        + layout.margin_top
                        + layout.margin_bottom
                },
                table: resolved_table,
            };
            self.display_lines
                .lock()
                .expect("display-line cache poisoned")
                .insert(key, lines.clone());
            return lines;
        }
        let mut base_font = preview_minimap_font(style, kind);
        base_font.weight = match kind {
            PreviewLineKind::Heading(1 | 2) => FontWeight::SEMIBOLD,
            PreviewLineKind::Heading(_) => FontWeight::MEDIUM,
            _ => FontWeight::NORMAL,
        };
        let text_runs = minimap_text_runs(kind, &display, base_font, style);
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
            .map(|(_, height)| {
                height
                    + self.media_layout_extra_height(row)
                    + layout.padding_top
                    + layout.padding_bottom
            })
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
            table: None,
        };
        self.display_lines
            .lock()
            .expect("display-line cache poisoned")
            .insert(key, lines.clone());
        lines
    }

    pub(crate) fn layout(&self, row: usize, style: PreviewStyle) -> RowLayout {
        let visual_row = self
            .projection
            .rows
            .get(row)
            .expect("preview row index is in bounds");
        let mut layout = style.row_layout(visual_row.style_kind);
        if row == 0 {
            layout.margin_top += style.spacing.content_padding_top;
        }
        if row + 1 == self.projection.rows.len() {
            layout.margin_bottom += style.spacing.content_padding_bottom;
        }
        layout
    }

    pub(crate) fn estimated_measure(
        &self,
        row: usize,
        available_width: f32,
        zoom: f32,
        style: PreviewStyle,
    ) -> crate::preview::layout::ResolvedRow {
        let layout = self.layout(row, style).scaled(zoom);
        let source_row = self.source_row(row);
        let kind = self.row_kind(row);
        let marker_width = if matches!(kind, PreviewLineKind::Heading(_)) {
            20.0
        } else {
            0.0
        };
        let wrap_width =
            (available_width - layout.padding_left - layout.padding_right - marker_width).max(1.0);
        let source_bytes = source_row.content.range.len() as f32;
        let estimated_text_width = source_bytes * layout.font_size * 0.5;
        let line_count = if kind == PreviewLineKind::Table {
            let display = self.runs(row);
            self.table_projection(row)
                .map(|table| {
                    table.estimated_line_count(&display.text, available_width, zoom, style)
                })
                .unwrap_or(1)
        } else if layout.fixed_height.is_some() || self.image_size(row, available_width).is_some() {
            1
        } else {
            (estimated_text_width / wrap_width).ceil().max(1.0) as usize
        };
        let parent_height = self
            .image_size(row, available_width)
            .map(|(_, height)| {
                height
                    + self.media_layout_extra_height(row)
                    + layout.padding_top
                    + layout.padding_bottom
            })
            .or_else(|| {
                (kind == PreviewLineKind::Table
                    && self
                        .table_projection(row)
                        .is_some_and(TableRowProjection::is_separator))
                .then_some(2.0)
            })
            .or(layout.fixed_height)
            .unwrap_or_else(|| {
                (line_count as f32 * layout.line_height
                    + layout.padding_top
                    + layout.padding_bottom)
                    .max(layout.min_height)
            })
            + layout.margin_top
            + layout.margin_bottom;
        crate::preview::layout::ResolvedRow::new(line_count, parent_height, false)
    }

    pub(crate) fn with_presentation_tail_padding(
        &self,
        row: usize,
        presentation_index: usize,
        presentation_len: usize,
        zoom: f32,
        style: PreviewStyle,
        measure: crate::preview::layout::ResolvedRow,
    ) -> crate::preview::layout::ResolvedRow {
        if presentation_index + 1 != presentation_len || row + 1 == self.projection.rows.len() {
            return measure;
        }
        crate::preview::layout::ResolvedRow::new(
            measure.display_lines as usize,
            measure.pixels + style.spacing.content_padding_bottom * zoom,
            measure.exact,
        )
    }

    pub(crate) fn without_presentation_tail_padding(
        &self,
        row: usize,
        presentation_index: usize,
        presentation_len: usize,
        zoom: f32,
        style: PreviewStyle,
        measure: crate::preview::layout::ResolvedRow,
    ) -> crate::preview::layout::ResolvedRow {
        if presentation_index + 1 != presentation_len || row + 1 == self.projection.rows.len() {
            return measure;
        }
        crate::preview::layout::ResolvedRow::new(
            measure.display_lines as usize,
            (measure.pixels - style.spacing.content_padding_bottom * zoom).max(0.0),
            measure.exact,
        )
    }

    pub(crate) fn table_projection(&self, row: usize) -> Option<&TableRowProjection> {
        match &self.projection.rows.get(row)?.kind {
            VisualRowKind::Table(table) => Some(table),
            _ => None,
        }
    }

    pub(crate) fn image_size(&self, row: usize, available_width: f32) -> Option<(f32, f32)> {
        match &self.projection.rows.get(row)?.kind {
            VisualRowKind::Image { dimensions } => dimensions.map(|(width, height)| {
                crate::preview::fitted_image_size(width, height, available_width)
            }),
            VisualRowKind::Diagram(diagram) => diagram.dimensions().map(|(width, height)| {
                let scale = (available_width.min(960.0) / width)
                    .min(480.0 / height)
                    .min(1.0);
                (width * scale, height * scale)
            }),
            _ => None,
        }
    }

    fn media_layout_extra_height(&self, row: usize) -> f32 {
        match &self.projection.rows.get(row).map(|row| &row.kind) {
            Some(VisualRowKind::Diagram(diagram)) => diagram.layout_extra_height(),
            _ => 0.0,
        }
    }

    pub(crate) fn row_kind(&self, row: usize) -> PreviewLineKind {
        match &self
            .projection
            .rows
            .get(row)
            .expect("preview row index is in bounds")
            .kind
        {
            VisualRowKind::Text => PreviewLineKind::Text,
            VisualRowKind::Heading(level) => PreviewLineKind::Heading(*level),
            VisualRowKind::List(_) => PreviewLineKind::List,
            VisualRowKind::Caption => PreviewLineKind::Caption,
            VisualRowKind::Quote => PreviewLineKind::Quote,
            VisualRowKind::Code(_) => PreviewLineKind::Code,
            VisualRowKind::Blank | VisualRowKind::Hidden => PreviewLineKind::Blank,
            VisualRowKind::Table(_) => PreviewLineKind::Table,
            VisualRowKind::Image { .. } | VisualRowKind::Diagram(_) => PreviewLineKind::Image,
            VisualRowKind::Rule => PreviewLineKind::Rule,
        }
    }
}

pub(crate) fn build_display_map(document: &PreviewSnapshot) -> PreviewDisplayMap {
    PreviewDisplayMap {
        text: document.text.clone(),
        format: document.format,
        projection: document.projection.clone(),
        org_config: document
            .semantic
            .as_ref()
            .map(|semantic| semantic.config.clone()),
        display_runs: Mutex::new(DisplayRunCache {
            entries: HashMap::with_capacity(DisplayRunCache::CAPACITY),
            order: VecDeque::with_capacity(DisplayRunCache::CAPACITY),
        }),
        display_lines: Mutex::new(DisplayLineCache {
            entries: HashMap::with_capacity(DisplayLineCache::CAPACITY),
            order: VecDeque::with_capacity(DisplayLineCache::CAPACITY),
        }),
        table_layouts: Mutex::new(TableLayoutCache {
            entries: HashMap::with_capacity(TableLayoutCache::CAPACITY),
            order: VecDeque::with_capacity(TableLayoutCache::CAPACITY),
        }),
    }
}

pub(crate) fn build_display_map_reusing(
    document: &PreviewSnapshot,
    previous: &PreviewDisplayMap,
) -> PreviewDisplayMap {
    let old_runs = previous
        .display_runs
        .lock()
        .expect("display-run cache poisoned");
    // Row identities include their semantic revision and are never recycled. Keeping this bounded
    // cache is therefore safe: stale entries cannot match new rows and disappear naturally via
    // the existing LRU capacity, without scanning every row in a large document after each edit.
    let run_entries = old_runs.entries.clone();
    let run_order = old_runs.order.clone();
    drop(old_runs);

    let old_lines = previous
        .display_lines
        .lock()
        .expect("display-line cache poisoned");
    let line_entries = old_lines.entries.clone();
    let line_order = old_lines.order.clone();

    PreviewDisplayMap {
        text: document.text.clone(),
        format: document.format,
        projection: document.projection.clone(),
        org_config: document
            .semantic
            .as_ref()
            .map(|semantic| semantic.config.clone()),
        display_runs: Mutex::new(DisplayRunCache {
            entries: run_entries,
            order: run_order,
        }),
        display_lines: Mutex::new(DisplayLineCache {
            entries: line_entries,
            order: line_order,
        }),
        // Layout identities are presentation-local and cheap to rebuild. Do not carry viewport
        // state across document snapshots even when semantic display runs can be reused safely.
        table_layouts: Mutex::new(TableLayoutCache {
            entries: HashMap::with_capacity(TableLayoutCache::CAPACITY),
            order: VecDeque::with_capacity(TableLayoutCache::CAPACITY),
        }),
    }
}

pub(crate) fn materialize_runs(model: &PreviewDisplayMap, row: usize) -> DisplayRuns {
    let visual = model
        .projection
        .rows
        .get(row)
        .expect("reading row index is in bounds");
    let line = model.source_row(row);
    let kind = model.row_kind(row);
    let source = model.text.copy_range(line.content.range);
    let source = source.trim_end_matches(['\r', '\n']);
    let source = match (&model.format, &visual.kind, &kind) {
        (DocumentFormat::Org, _, PreviewLineKind::Heading(_)) => {
            parse_heading_with_config(source, model.org_config.as_deref()).title
        }
        (_, _, PreviewLineKind::List) => {
            let parts = parse_list_item(source);
            parts.term.map_or(parts.body.clone(), |term| {
                format!("{term} — {}", parts.body)
            })
        }
        (DocumentFormat::Org, _, PreviewLineKind::Caption) => source
            .split_once(':')
            .map_or(source, |(_, caption)| caption)
            .trim_start()
            .to_owned(),
        (_, VisualRowKind::Code(ReadingCodeRow::Start), _) => {
            visual.code_language.as_deref().unwrap_or("code").to_owned()
        }
        (_, VisualRowKind::Code(ReadingCodeRow::End), _) => String::new(),
        (_, VisualRowKind::Diagram(_), _) => String::new(),
        (_, VisualRowKind::Hidden, _) => String::new(),
        _ => source.to_owned(),
    };
    let parse_inline = matches!(
        kind,
        PreviewLineKind::Text
            | PreviewLineKind::Heading(_)
            | PreviewLineKind::List
            | PreviewLineKind::Caption
            | PreviewLineKind::Quote
    );
    let (text, inline_spans, links): (SharedString, Arc<[InlineSpan]>, Arc<[InlineLink]>) =
        if parse_inline {
            let parsed = parse_document_inline(model.format, &source);
            let links = parsed
                .spans
                .iter()
                .filter(|span| span.kind == InlineKind::Link)
                .filter_map(|span| {
                    source
                        .get(span.source.clone())
                        .and_then(|raw| link_destination(model.format, raw))
                        .map(|destination| InlineLink {
                            range: span.range.clone(),
                            destination: destination.into(),
                        })
                })
                .collect::<Vec<_>>()
                .into();
            (parsed.text.into(), parsed.spans.into(), links)
        } else {
            (source.into(), Arc::from([]), Arc::from([]))
        };
    let code_spans = model
        .projection
        .rows
        .get(row)
        .and_then(|row| row.code_language.as_deref())
        .and_then(|language| highlight_code(language, &text).ok())
        .unwrap_or_default()
        .into();
    DisplayRuns {
        text,
        inline_spans,
        links,
        code_spans,
    }
}

fn link_destination(format: DocumentFormat, raw: &str) -> Option<&str> {
    match format {
        DocumentFormat::Org => {
            let inside = raw.strip_prefix("[[")?.strip_suffix("]]")?;
            Some(inside.split_once("][").map_or(inside, |(target, _)| target))
        }
        DocumentFormat::Markdown => {
            if let Some(destination) = raw.strip_prefix('<').and_then(|raw| raw.strip_suffix('>')) {
                return (!destination.is_empty()).then_some(destination);
            }
            let (_, destination) = raw.rsplit_once("](")?;
            destination.strip_suffix(')')
        }
    }
    .filter(|destination| !destination.is_empty())
}

#[cfg(test)]
pub(crate) fn truncate_for_minimap(text: &str) -> String {
    const MAX_MINIMAP_COLUMNS: usize = 1024;
    text.graphemes(true).take(MAX_MINIMAP_COLUMNS).collect()
}

pub(crate) fn preview_minimap_font(style: PreviewStyle, kind: PreviewLineKind) -> gpui::Font {
    let (family, fallbacks) = if kind == PreviewLineKind::Code {
        (
            style.typography.code_family,
            style.typography.code_fallbacks,
        )
    } else {
        (
            style.typography.body_family,
            style.typography.body_fallbacks,
        )
    };
    let mut minimap_font = font(family);
    minimap_font.weight = FontWeight::BLACK;
    minimap_font.fallbacks = Some(FontFallbacks::from_fonts(
        fallbacks
            .iter()
            .map(|family| (*family).to_owned())
            .collect(),
    ));
    minimap_font
}

#[cfg(test)]
pub(crate) fn minimap_font() -> gpui::Font {
    preview_minimap_font(
        *crate::preview::preview_style(crate::preview::PreviewStyleId::Base),
        PreviewLineKind::Text,
    )
}

pub(crate) fn kind_color(kind: PreviewLineKind, style: PreviewStyle) -> u32 {
    let palette = style.palette;
    match kind {
        PreviewLineKind::Heading(level) => palette.heading[level.saturating_sub(1).min(3) as usize],
        PreviewLineKind::Code => palette.code_boundary,
        PreviewLineKind::Table => palette.foreground,
        PreviewLineKind::Quote => palette.quote,
        PreviewLineKind::List => palette.foreground,
        PreviewLineKind::Caption => palette.meta,
        PreviewLineKind::Image => palette.attribute,
        PreviewLineKind::Rule => palette.border,
        PreviewLineKind::Text | PreviewLineKind::Blank => palette.foreground,
    }
}

pub(crate) fn minimap_runs(runs: DisplayRuns) -> DisplayRuns {
    runs
}

pub(crate) fn slice_display_runs(runs: &DisplayRuns, range: Range<usize>) -> DisplayRuns {
    let start = range.start.min(runs.text.len());
    let end = range.end.min(runs.text.len()).max(start);
    DisplayRuns {
        text: runs.text[start..end].to_owned().into(),
        inline_spans: runs
            .inline_spans
            .iter()
            .filter(|span| span.range.start < end && span.range.end > start)
            .map(|span| {
                let mut span = span.clone();
                span.range = span.range.start.max(start) - start..span.range.end.min(end) - start;
                span
            })
            .collect::<Vec<_>>()
            .into(),
        links: runs
            .links
            .iter()
            .filter(|link| link.range.start < end && link.range.end > start)
            .map(|link| InlineLink {
                range: link.range.start.max(start) - start..link.range.end.min(end) - start,
                destination: link.destination.clone(),
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

pub(crate) fn minimap_text_runs(
    kind: PreviewLineKind,
    line: &DisplayRuns,
    base_font: gpui::Font,
    preview_style: PreviewStyle,
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
                let palette = preview_style.palette;
                let mut color: gpui::Hsla = gpui::rgb(kind_color(kind, preview_style)).into();
                let mut strikethrough = None;
                let mut underline = None;
                for kind in kinds {
                    match kind {
                        InlineKind::Bold => run_font.weight = FontWeight::BOLD,
                        InlineKind::Italic => run_font.style = gpui::FontStyle::Italic,
                        InlineKind::Strike => strikethrough = Some(Default::default()),
                        InlineKind::Code | InlineKind::Verbatim => {
                            color = gpui::rgb(palette.code_foreground).into()
                        }
                        InlineKind::Link | InlineKind::FootnoteReference => {
                            color = gpui::rgb(palette.link).into()
                        }
                        InlineKind::Timestamp => color = gpui::rgb(palette.date).into(),
                        InlineKind::Target | InlineKind::RadioTarget => {
                            color = gpui::rgb(palette.attribute).into()
                        }
                        InlineKind::Entity | InlineKind::Latex => {
                            color = gpui::rgb(palette.constant).into()
                        }
                        InlineKind::Underline => underline = Some(Default::default()),
                    }
                }
                if let Some(span) = line
                    .code_spans
                    .iter()
                    .find(|span| span.start < end && span.end > start)
                {
                    let highlight = code_highlight_style(span.kind, preview_style);
                    if let Some(syntax_color) = highlight.color {
                        color = syntax_color;
                    }
                    if let Some(font_style) = highlight.font_style {
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
mod link_tests {
    use super::*;

    #[test]
    fn extracts_org_markdown_and_autolink_destinations() {
        assert_eq!(
            link_destination(DocumentFormat::Org, "[[file:notes.org][Notes]]"),
            Some("file:notes.org")
        );
        assert_eq!(
            link_destination(DocumentFormat::Markdown, "[Notes](notes.md#part)"),
            Some("notes.md#part")
        );
        assert_eq!(
            link_destination(DocumentFormat::Markdown, "<https://example.com>"),
            Some("https://example.com")
        );
    }
}
