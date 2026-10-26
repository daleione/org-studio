//! Font measurements shared by every row of one source table.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use super::{cell_projection, project_runs, slice_text_runs, syntax};
use crate::editor::org_commands::TableColumns;
use crate::{
    document::{
        DocumentFormat, DocumentSnapshot, LineIndex, Revision, RevisionDelta, TextSnapshot, table,
    },
    org_semantic::display::OrgDisplayOptions,
};
use gpui::{Font, Pixels, TextRun};

#[derive(Default)]
pub(in crate::editor) struct TableGeometryCache {
    key: Option<(Revision, DocumentFormat)>,
    columns: BTreeMap<u64, Arc<TableColumns>>,
    metrics: HashMap<(u64, DocumentFormat), Arc<TableColumnMetrics>>,
}

impl TableGeometryCache {
    pub(in crate::editor) fn invalidate_metrics(&mut self) {
        self.metrics.clear();
    }

    pub(in crate::editor) fn columns(
        &mut self,
        snapshot: &DocumentSnapshot,
        line: LineIndex,
        format: DocumentFormat,
    ) -> Option<Arc<TableColumns>> {
        let key = Some((snapshot.revision(), format));
        if self.key != key {
            self.key = key;
            self.columns.clear();
            self.metrics.clear();
        }
        let offset = snapshot.line_content_range(line).ok()?.start.0;
        if let Some((_, columns)) = self.columns.range(..=offset).next_back()
            && offset <= columns.source_range.end.0
        {
            return Some(columns.clone());
        }
        let start = crate::editor::org_commands::table_start(snapshot, line.0, format)?;
        let columns = Arc::new(crate::editor::org_commands::table_columns(
            snapshot, start, format,
        )?);
        self.columns
            .insert(columns.source_range.start.0, columns.clone());
        Some(columns)
    }

    pub(in crate::editor) fn measure(
        &mut self,
        snapshot: &DocumentSnapshot,
        columns: &TableColumns,
        font: Font,
        font_size: Pixels,
        text_system: &gpui::WindowTextSystem,
        display_options: OrgDisplayOptions,
    ) -> Arc<TableColumnMetrics> {
        let entry = self
            .metrics
            .entry((columns.source_range.start.0, columns.format));
        if let std::collections::hash_map::Entry::Occupied(entry) = &entry {
            let metrics = entry.get();
            if metrics.font == font
                && metrics.font_size == font_size
                && metrics.display_options == display_options
            {
                return metrics.clone();
            }
        }
        let metrics = Arc::new(TableColumnMetrics::measure(
            snapshot,
            columns,
            font,
            font_size,
            text_system,
            display_options,
        ));
        entry.insert_entry(metrics.clone());
        metrics
    }

    pub(in crate::editor) fn affected_by(&self, delta: &RevisionDelta) -> bool {
        self.columns.values().any(|columns| {
            // Several edits can arrive before the next frame refreshes geometry.
            self.key
                .is_none_or(|(revision, _)| revision != delta.before)
                || delta.edits.iter().any(|edit| {
                    edit.old.start <= columns.source_range.end
                        && edit.old.end >= columns.source_range.start
                })
        })
    }
}

/// Font-dependent geometry, shared by every row of a table.
pub(in crate::editor) struct TableColumnMetrics {
    pub(in crate::editor) display_options: OrgDisplayOptions,
    pub(in crate::editor) font: Font,
    pub(in crate::editor) font_size: Pixels,
    pub(in crate::editor) space_width: Pixels,
    pub(in crate::editor) pipe_width: Pixels,
    pub(in crate::editor) natural_widths: Vec<Pixels>,
    pub(super) raw_widths: Vec<Pixels>,
}

impl TableColumnMetrics {
    pub(in crate::editor) fn measure(
        snapshot: &DocumentSnapshot,
        columns: &TableColumns,
        font: Font,
        font_size: Pixels,
        text_system: &gpui::WindowTextSystem,
        display_options: OrgDisplayOptions,
    ) -> Self {
        let measure = |text: &str| {
            text_system
                .shape_line(
                    text.to_owned().into(),
                    font_size,
                    &[TextRun {
                        len: text.len(),
                        font: font.clone(),
                        ..Default::default()
                    }],
                    None,
                )
                .width()
        };
        let space_width = measure(" ");
        let pipe_width = measure("|");
        let mut natural_widths = columns
            .minimum_widths
            .iter()
            .map(|width| space_width * *width)
            .collect::<Vec<_>>();
        let mut raw_widths = vec![Pixels::ZERO; natural_widths.len()];
        let first = snapshot
            .line_index_at(columns.source_range.start)
            .expect("table starts on a line");
        let last = snapshot
            .line_index_at(columns.source_range.end)
            .expect("table ends on a line");
        let path = std::path::Path::new(match columns.format {
            DocumentFormat::Org => "table.org",
            DocumentFormat::Markdown => "table.md",
        });
        for line in first.0..=last.0 {
            let range = snapshot
                .line_content_range(LineIndex(line))
                .expect("table line exists");
            let text = snapshot.copy_range(range).replace('\t', "    ");
            let row = table::parse_line(&text, columns.format);
            if row.separator {
                for (width, cell) in raw_widths.iter_mut().zip(row.cells) {
                    *width = (*width).max(measure(&text[cell.raw_range]));
                }
                continue;
            }
            let mut style = syntax::EditorLineStyle::pending_fallback(range);
            style.id = syntax::EditorStyleId::Table;
            let runs = syntax::runs_from_spans(
                &syntax::semantic_spans(path, &text, &style),
                TextRun {
                    len: text.len(),
                    font: font.clone(),
                    ..Default::default()
                },
                &style,
                None,
                crate::theme::current_theme(),
            );
            for (column, ((width, raw_width), cell)) in natural_widths
                .iter_mut()
                .zip(&mut raw_widths)
                .zip(row.cells)
                .enumerate()
            {
                let whitespace = measure(&text[cell.raw_range.start..cell.text_range.start])
                    + measure(&text[cell.text_range.end..cell.raw_range.end]);
                let content = &text[cell.text_range.clone()];
                let source_padding =
                    if columns.format == DocumentFormat::Org && columns.source_widths.is_some() {
                        columns.widths[column]
                            .saturating_sub(table::text_width(content, columns.format))
                    } else {
                        0
                    };
                let runs = slice_text_runs(&runs, cell.text_range);
                let projection =
                    cell_projection(content, columns.format, None, display_options, &[]);
                let content = projection
                    .as_ref()
                    .map_or(content, |projection| projection.text.as_str());
                let runs = match &projection {
                    Some(projection) => project_runs(&runs, projection),
                    None => runs,
                };
                let measured = text_system
                    .shape_line(content.to_owned().into(), font_size, &runs, None)
                    .width();
                *width = (*width).max(measured + space_width * source_padding);
                *raw_width = (*raw_width).max(measured + whitespace);
            }
        }
        for width in &mut natural_widths {
            *width += space_width * 2.;
        }
        Self {
            display_options,
            font,
            font_size,
            space_width,
            pipe_width,
            natural_widths,
            raw_widths,
        }
    }

    /// Source padding beyond the longest cell remains real extra space. Narrow
    /// columns scale below the measured content width, keeping the mapping invertible.
    pub(in crate::editor) fn column_width(
        &self,
        column: usize,
        requested: usize,
        natural: usize,
    ) -> Pixels {
        let content = self.natural_widths[column] - self.space_width * 2.;
        let width = if requested < natural {
            content * (requested as f32 / natural as f32)
        } else {
            content + self.space_width * (requested - natural)
        };
        width + self.space_width * 2.
    }

    pub(in crate::editor) fn source_width(
        &self,
        column: usize,
        width: Pixels,
        natural: usize,
    ) -> usize {
        let content = (width - self.space_width * 2.).max(Pixels::ZERO);
        let natural_content = self.natural_widths[column] - self.space_width * 2.;
        let width = if content < natural_content {
            content / natural_content * natural as f32
        } else {
            natural as f32 + (content - natural_content) / self.space_width
        };
        width.round() as usize
    }
}
