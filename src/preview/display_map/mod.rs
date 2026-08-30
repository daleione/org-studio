use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
    sync::{Arc, Mutex},
};

#[cfg(test)]
use gpui::FontFallbacks;
use gpui::{FontWeight, SharedString, TextRun, font, px};

use crate::{
    document::SharedTextSnapshot,
    org_syntax::{
        BlockArena, BlockKind,
        inline::{InlineKind, InlineSpan},
    },
    preview::{
        CodeHighlightSpan, DocumentFormat, PreviewRow, PreviewSnapshot, code_highlight_style,
        highlight_code,
        markdown::{MarkdownBlock, MarkdownKind},
        parse_document_inline,
        projection::{PreviewProjectionSnapshot, VisualRowId, VisualRowKind},
        table::TableRowProjection,
    },
    theme::current_theme,
};
#[cfg(test)]
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PreviewLineKind {
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
pub(in crate::preview) struct RowLayout {
    pub(in crate::preview) font_size: f32,
    pub(in crate::preview) line_height: f32,
    pub(in crate::preview) min_height: f32,
    pub(in crate::preview) padding_left: f32,
    pub(in crate::preview) padding_right: f32,
    pub(in crate::preview) padding_top: f32,
    pub(in crate::preview) padding_bottom: f32,
    pub(in crate::preview) margin_top: f32,
    pub(in crate::preview) margin_bottom: f32,
    pub(in crate::preview) fixed_height: Option<f32>,
}

impl RowLayout {
    pub(super) const fn text(font_size: f32, line_height: f32) -> Self {
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

    pub(super) const fn blank() -> Self {
        Self {
            fixed_height: Some(24.0),
            ..Self::text(14.0, 24.0)
        }
    }

    pub(super) const fn image() -> Self {
        Self {
            padding_top: 8.0,
            padding_bottom: 8.0,
            ..Self::text(14.0, 24.0)
        }
    }

    pub(super) const fn rule() -> Self {
        Self {
            margin_top: 20.0,
            margin_bottom: 20.0,
            fixed_height: Some(1.0),
            ..Self::text(14.0, 24.0)
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::preview) struct DisplayRuns {
    pub(in crate::preview) text: SharedString,
    pub(in crate::preview) inline_spans: Arc<[InlineSpan]>,
    pub(in crate::preview) code_spans: Arc<[CodeHighlightSpan]>,
}

pub(in crate::preview) struct PreviewDisplayMap {
    pub(super) text: SharedTextSnapshot,
    pub(super) format: DocumentFormat,
    pub(super) projection: Arc<PreviewProjectionSnapshot>,
    pub(super) display_runs: Mutex<DisplayRunCache>,
    pub(super) display_lines: Mutex<DisplayLineCache>,
}

impl PreviewDisplayMap {
    pub(super) fn source_row(&self, row: usize) -> PreviewRow {
        self.projection
            .source_row(row)
            .expect("preview row range maps to the current revision")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct DisplayLines {
    pub(super) ranges: Arc<[Range<usize>]>,
    pub(super) parent_height: f32,
}

pub(super) struct DisplayLineCache {
    entries: HashMap<((VisualRowId, u64), u16), DisplayLines>,
    order: VecDeque<((VisualRowId, u64), u16)>,
}

impl DisplayLineCache {
    const CAPACITY: usize = 4096;

    pub(super) fn insert(&mut self, key: ((VisualRowId, u64), u16), lines: DisplayLines) {
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

pub(super) struct DisplayRunCache {
    pub(super) entries: HashMap<(VisualRowId, u64), DisplayRuns>,
    pub(super) order: VecDeque<(VisualRowId, u64)>,
}

impl DisplayRunCache {
    pub(super) const CAPACITY: usize = 4096;

    pub(super) fn insert(&mut self, identity: (VisualRowId, u64), runs: DisplayRuns) {
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
    pub(in crate::preview) fn runs(&self, row: usize) -> DisplayRuns {
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

    pub(in crate::preview) fn is_heading(&self, row: usize) -> bool {
        self.projection
            .rows
            .get(row)
            .is_some_and(|_| matches!(self.row_kind(row), PreviewLineKind::Heading(_)))
    }

    pub(super) fn display_lines(
        &self,
        row: usize,
        available_width: f32,
        text_system: &gpui::WindowTextSystem,
    ) -> DisplayLines {
        let width_key = available_width.round().clamp(1.0, u16::MAX as f32) as u16;
        let visual_row = self
            .projection
            .rows
            .get(row)
            .expect("preview row index is in bounds");
        let key = ((visual_row.id, visual_row.semantic_revision), width_key);
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
        let layout = self.layout(row);
        if kind == PreviewLineKind::Table {
            // A table row is laid out from its parsed cells and shared column geometry, not by
            // wrapping the source `| ... |` text. Keeping it to one display line matches the
            // preview row and prevents the minimap from inventing extra rows from markup width.
            let lines = DisplayLines {
                ranges: std::iter::once(0..display.text.len())
                    .collect::<Vec<_>>()
                    .into(),
                parent_height: layout.min_height.max(layout.line_height),
            };
            self.display_lines
                .lock()
                .expect("display-line cache poisoned")
                .insert(key, lines.clone());
            return lines;
        }
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

    pub(in crate::preview) fn is_table(&self, row: usize) -> bool {
        self.projection
            .rows
            .get(row)
            .is_some_and(|_| self.row_kind(row) == PreviewLineKind::Table)
    }

    pub(in crate::preview) fn layout(&self, row: usize) -> RowLayout {
        self.projection
            .rows
            .get(row)
            .expect("preview row index is in bounds")
            .layout
    }

    pub(super) fn source_row_layout(
        format: DocumentFormat,
        blocks: &BlockArena,
        markdown_blocks: &[MarkdownBlock],
        block_id: u32,
    ) -> RowLayout {
        match format {
            DocumentFormat::Org => match &blocks.nodes()[block_id as usize].kind {
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
            DocumentFormat::Markdown => match &markdown_blocks[block_id as usize].kind {
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

    pub(in crate::preview) fn table_projection(&self, row: usize) -> Option<&TableRowProjection> {
        match &self.projection.rows.get(row)?.kind {
            VisualRowKind::Table(table) => Some(table),
            _ => None,
        }
    }

    pub(in crate::preview) fn image_size(
        &self,
        row: usize,
        available_width: f32,
    ) -> Option<(f32, f32)> {
        match self.projection.rows.get(row)?.kind {
            VisualRowKind::Image { width, height } => Some((width, height)),
            _ => None,
        }
        .map(|(width, height)| crate::preview::fitted_image_size(width, height, available_width))
    }

    pub(super) fn row_kind(&self, row: usize) -> PreviewLineKind {
        match &self
            .projection
            .rows
            .get(row)
            .expect("preview row index is in bounds")
            .kind
        {
            VisualRowKind::Text => PreviewLineKind::Text,
            VisualRowKind::Heading(level) => PreviewLineKind::Heading(*level),
            VisualRowKind::List => PreviewLineKind::List,
            VisualRowKind::Quote => PreviewLineKind::Quote,
            VisualRowKind::Code => PreviewLineKind::Code,
            VisualRowKind::Blank | VisualRowKind::Hidden => PreviewLineKind::Blank,
            VisualRowKind::Table(_) => PreviewLineKind::Table,
            VisualRowKind::Image { .. } => PreviewLineKind::Image,
            VisualRowKind::Rule => PreviewLineKind::Rule,
        }
    }
}

pub(in crate::preview) fn build_display_map(document: &PreviewSnapshot) -> PreviewDisplayMap {
    PreviewDisplayMap {
        text: document.text.clone(),
        format: document.format,
        projection: document.projection.clone(),
        display_runs: Mutex::new(DisplayRunCache {
            entries: HashMap::with_capacity(DisplayRunCache::CAPACITY),
            order: VecDeque::with_capacity(DisplayRunCache::CAPACITY),
        }),
        display_lines: Mutex::new(DisplayLineCache {
            entries: HashMap::with_capacity(DisplayLineCache::CAPACITY),
            order: VecDeque::with_capacity(DisplayLineCache::CAPACITY),
        }),
    }
}

pub(super) fn materialize_runs(model: &PreviewDisplayMap, row: usize) -> DisplayRuns {
    let line = model.source_row(row);
    let kind = model.row_kind(row);
    let source = model.text.copy_range(line.content.range);
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
        code_spans,
    }
}

#[cfg(test)]
pub(super) fn truncate_for_minimap(text: &str) -> String {
    const MAX_MINIMAP_COLUMNS: usize = 1024;
    text.graphemes(true).take(MAX_MINIMAP_COLUMNS).collect()
}

#[cfg(test)]
pub(super) fn minimap_font() -> gpui::Font {
    let mut minimap_font = font("Menlo");
    // Match Zed's 2px BLACK rendering and make CJK/emoji fallback deterministic.
    minimap_font.weight = FontWeight::BLACK;
    minimap_font.fallbacks = Some(FontFallbacks::from_fonts(vec![
        "PingFang SC".to_owned(),
        "Apple Color Emoji".to_owned(),
    ]));
    minimap_font
}

pub(super) fn kind_color(kind: PreviewLineKind) -> u32 {
    let theme = current_theme();
    match kind {
        PreviewLineKind::Heading(level) => theme.heading[level.saturating_sub(1).min(3) as usize],
        PreviewLineKind::Code => theme.code_boundary,
        PreviewLineKind::Table => theme.foreground,
        PreviewLineKind::Quote => theme.quote,
        PreviewLineKind::List => theme.foreground,
        PreviewLineKind::Image => theme.attribute,
        PreviewLineKind::Rule => theme.border,
        PreviewLineKind::Text | PreviewLineKind::Blank => theme.foreground,
    }
}

pub(super) fn minimap_runs(runs: DisplayRuns) -> DisplayRuns {
    runs
}

pub(super) fn slice_display_runs(runs: &DisplayRuns, range: Range<usize>) -> DisplayRuns {
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

pub(super) fn minimap_text_runs(
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
