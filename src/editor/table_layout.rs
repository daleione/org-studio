//! Source table shaping and geometry shared by the editor and minimap.
use std::{ops::Range, sync::Arc};

use gpui::{Bounds, Font, Pixels, Point, TextRun, WrappedLine, point, px};

use super::{org_commands::TableColumns, syntax};
use crate::document::{DocumentFormat, DocumentSnapshot, LineIndex, TextSnapshot, table};
use crate::org_syntax::inline::EntityText;

/// Font-dependent geometry, shared by every row of a table.
pub(super) struct TableColumnMetrics {
    pub(super) font: Font,
    pub(super) font_size: Pixels,
    pub(super) space_width: Pixels,
    pub(super) pipe_width: Pixels,
    pub(super) natural_widths: Vec<Pixels>,
    raw_widths: Vec<Pixels>,
}

impl TableColumnMetrics {
    pub(super) fn measure(
        snapshot: &DocumentSnapshot,
        columns: &TableColumns,
        font: Font,
        font_size: Pixels,
        text_system: &gpui::WindowTextSystem,
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
                let entities = cell_entities(content, columns.format, None);
                let content = entities
                    .as_ref()
                    .map_or(content, |entities| entities.text.as_str());
                let runs = match &entities {
                    Some(entities) => entity_runs(&runs, entities),
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
    pub(super) fn column_width(&self, column: usize, requested: usize, natural: usize) -> Pixels {
        let content = self.natural_widths[column] - self.space_width * 2.;
        let width = if requested < natural {
            content * (requested as f32 / natural as f32)
        } else {
            content + self.space_width * (requested - natural)
        };
        width + self.space_width * 2.
    }

    pub(super) fn source_width(&self, column: usize, width: Pixels, natural: usize) -> usize {
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

pub(super) struct TableVisualFragment {
    pub(super) preview_runs: Vec<TextRun>,
    pub(super) display_range: Range<usize>,
    pub(super) content_range: Range<usize>,
    pub(super) x: Pixels,
    pub(super) width: Pixels,
    pub(super) layout: WrappedLine,
    pub(super) delimiter: bool,
    // Separator glyphs are shortened to fit the column, while their source stays intact.
    pub(super) scaled_source: bool,
    entities: Option<EntityText>,
}

impl TableVisualFragment {
    fn position(&self, index: usize, line_height: Pixels) -> Point<Pixels> {
        if self.scaled_source {
            let fraction = index.saturating_sub(self.display_range.start) as f32
                / self.display_range.len().max(1) as f32;
            return point(self.x + self.width * fraction, Pixels::ZERO);
        }
        let local = index.clamp(self.content_range.start, self.content_range.end)
            - self.content_range.start;
        let local = self
            .entities
            .as_ref()
            .map_or(local, |entities| entities.source_to_display(local));
        if let Some(row) = self.layout.wrap_boundaries().iter().position(|boundary| {
            self.layout.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index == local
        }) {
            return point(self.x, line_height * (row + 1));
        }
        let position = self
            .layout
            .position_for_index(local, line_height)
            .unwrap_or_default();
        point(self.x + position.x, position.y)
    }

    fn closest_index(&self, position: Point<Pixels>, line_height: Pixels) -> usize {
        if self.scaled_source {
            let fraction =
                (f32::from(position.x - self.x) / f32::from(self.width).max(1.)).clamp(0., 1.);
            return self.display_range.start
                + (fraction * self.display_range.len() as f32).round() as usize;
        }
        let last_y = line_height * self.layout.wrap_boundaries().len();
        let local = self
            .layout
            .closest_index_for_position(
                point(position.x - self.x, position.y.clamp(Pixels::ZERO, last_y)),
                line_height,
            )
            .unwrap_or_else(|index| index);
        let local = self
            .entities
            .as_ref()
            .map_or(local, |entities| entities.display_to_source(local));
        self.content_range.start + local.min(self.content_range.len())
    }
}

pub(crate) struct TableVisualLayout {
    pub(super) fragments: Vec<TableVisualFragment>,
    pub(super) width: Pixels,
    len: usize,
    pub(super) visual_rows: usize,
    pub(super) column_widths: Vec<Pixels>,
    pub(super) metrics: Arc<TableColumnMetrics>,
}

impl TableVisualLayout {
    pub(super) fn position_for_index(&self, index: usize, line_height: Pixels) -> Point<Pixels> {
        let index = index.min(self.len);
        self.fragments
            .iter()
            .find(|fragment| fragment.display_range.contains(&index))
            .or_else(|| self.fragments.last())
            .map_or(Point::default(), |fragment| {
                fragment.position(index, line_height)
            })
    }

    pub(super) fn closest_index_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> usize {
        self.fragments
            .iter()
            .min_by(|a, b| {
                let distance = |f: &TableVisualFragment| {
                    let right = f.x + f.width;
                    f32::from((f.x - position.x).max(position.x - right).max(Pixels::ZERO))
                };
                distance(a).total_cmp(&distance(b))
            })
            .map_or(0, |fragment| fragment.closest_index(position, line_height))
    }

    /// A source range can occupy several columns with independent wrap positions.
    pub(super) fn range_bounds(
        &self,
        range: Range<usize>,
        line_height: Pixels,
    ) -> Vec<Bounds<Pixels>> {
        let mut bounds = Vec::new();
        for fragment in self.fragments.iter() {
            let start = range.start.max(fragment.display_range.start);
            let end = range.end.min(fragment.display_range.end);
            if start >= end {
                continue;
            }
            let first = fragment.position(start, line_height);
            let last = fragment.position(end, line_height);
            let first_row = (first.y / line_height).round() as usize;
            let last_row = (last.y / line_height).round() as usize;
            for row in first_row..=last_row {
                let left = if row == first_row {
                    first.x
                } else {
                    fragment.x
                };
                let right = if row == last_row {
                    last.x
                } else {
                    fragment.x + fragment.width
                };
                if right > left {
                    bounds.push(Bounds::from_corners(
                        point(left, line_height * row),
                        point(right, line_height * (row + 1)),
                    ));
                }
            }
        }
        bounds
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn shape(
        text: &gpui::SharedString,
        runs: &[TextRun],
        columns: &TableColumns,
        metrics: &Arc<TableColumnMetrics>,
        wrap_width: Option<Pixels>,
        preferred_widths_em: Option<&[f32]>,
        caret: Option<usize>,
        text_system: &gpui::WindowTextSystem,
    ) -> Option<Arc<TableVisualLayout>> {
        // Leave room to paint the caret after the final column boundary.
        let wrap_width = wrap_width.map(|width| (width - gpui::px(2.)).max(gpui::px(1.)));
        let delimiters = crate::document::table::delimiter_offsets(text, columns.format);
        if delimiters.len() < 2 || columns.widths.is_empty() {
            return None;
        }
        let mut base = runs.first()?.clone();
        base.font = metrics.font.clone();
        base.len = 1;
        let font_size = metrics.font_size;
        let advance = |glyph: &str| {
            text_system
                .shape_line(
                    glyph.to_owned().into(),
                    font_size,
                    std::slice::from_ref(&base),
                    None,
                )
                .width()
        };
        let space = metrics.space_width;
        let pipe = metrics.pipe_width;
        let indent = text_system
            .shape_line(
                text[..delimiters[0]].to_owned().into(),
                font_size,
                &slice_text_runs(runs, 0..delimiters[0]),
                None,
            )
            .width();
        let preferred = preferred_widths_em.filter(|widths| {
            widths.len() == columns.widths.len()
                && widths.iter().all(|width| width.is_finite() && *width > 0.)
        });
        let source_widths = columns.source_widths.as_ref();
        let mut widths = preferred.map_or_else(
            || {
                source_widths.map_or_else(
                    || metrics.natural_widths.clone(),
                    |widths| {
                        widths
                            .iter()
                            .zip(&columns.widths)
                            .enumerate()
                            .map(|(index, (width, natural))| {
                                metrics.column_width(index, *width, *natural)
                            })
                            .collect()
                    },
                )
            },
            |widths| widths.iter().map(|width| font_size * *width).collect(),
        );
        // Source spaces use normal glyph advances; only explicitly narrowed columns wrap them.
        for (column, width) in widths.iter_mut().enumerate() {
            if wrap_width.is_none() {
                *width = (*width).max(metrics.natural_widths[column]);
                if columns.format == DocumentFormat::Org {
                    *width = (*width).max(metrics.raw_widths[column]);
                }
            } else if preferred.is_none()
                && columns.format == DocumentFormat::Org
                && source_widths.is_none_or(|requested| requested[column] >= columns.widths[column])
            {
                *width = (*width).max(metrics.raw_widths[column]);
            }
        }
        let natural_width =
            indent + pipe * (columns.widths.len() + 1) + widths.iter().copied().sum::<Pixels>();
        let constrained = wrap_width.is_some_and(|width| natural_width > width);
        let markdown = columns.format == DocumentFormat::Markdown;
        if markdown
            && !columns.aligned
            && source_widths.is_none()
            && !constrained
            && preferred.is_none()
        {
            return None;
        }
        let column_layout = markdown
            || constrained
            || preferred.is_some()
            || wrap_width.is_some()
                && source_widths.is_some_and(|widths| {
                    widths
                        .iter()
                        .zip(&columns.widths)
                        .any(|(width, natural)| width < natural)
                });
        if let Some(width) = wrap_width.filter(|_| constrained) {
            let budget = (width - indent - pipe * (columns.widths.len() + 1))
                .max(px(columns.widths.len() as f32));
            fit_columns(&mut widths, budget);
        }
        let separator = crate::document::table::is_separator(text);
        let mut fragments = Vec::with_capacity(delimiters.len() * 2 + 1);
        let shape = |source: gpui::SharedString, source_runs: &[TextRun], width| {
            text_system
                .shape_text(source, font_size, source_runs, width, None)
                .ok()
                .and_then(|lines| lines.into_iter().next())
                .unwrap_or_default()
        };
        if delimiters[0] > 0 {
            let range = 0..delimiters[0];
            fragments.push(TableVisualFragment {
                preview_runs: slice_text_runs(runs, range.clone()),
                display_range: range.clone(),
                content_range: range.clone(),
                x: Pixels::ZERO,
                width: indent,
                layout: shape(
                    text[range.clone()].to_owned().into(),
                    &slice_text_runs(runs, range),
                    None,
                ),
                delimiter: false,
                scaled_source: false,
                entities: None,
            });
        }
        let mut x = indent;
        for (column, start) in delimiters.iter().copied().enumerate() {
            let range = start..start + 1;
            fragments.push(TableVisualFragment {
                preview_runs: slice_text_runs(runs, range.clone()),
                display_range: range.clone(),
                content_range: range.clone(),
                x,
                width: pipe,
                layout: shape(
                    text[range.clone()].to_owned().into(),
                    &slice_text_runs(runs, range),
                    None,
                ),
                delimiter: true,
                scaled_source: false,
                entities: None,
            });
            let end = delimiters.get(column + 1).copied().unwrap_or(text.len());
            let range = start + 1..end;
            let mut width = if column_layout {
                widths.get(column).copied().unwrap_or_default()
            } else {
                Pixels::ZERO
            };
            if !range.is_empty() {
                let raw = &text[range.clone()];
                let padding = space.min(width / 4.);
                let content_width = (width - padding * 2.).max(gpui::px(1.));
                let content_range = if markdown {
                    let trimmed = raw.trim();
                    let start = range.start + raw.len() - raw.trim_start().len();
                    start..start + trimmed.len()
                } else {
                    range.clone()
                };
                let content = &text[content_range.clone()];
                let text_width = if (separator && column_layout) || markdown {
                    content_width
                } else if column_layout {
                    // Keep the menu outside wrapped text, without dropping a source space.
                    (width - padding).max(px(1.))
                } else {
                    width.max(px(1.))
                };
                let entities = (!separator)
                    .then(|| {
                        cell_entities(
                            content,
                            columns.format,
                            caret
                                .filter(|caret| content_range.contains(caret))
                                .map(|caret| caret - content_range.start),
                        )
                    })
                    .flatten();
                let source_runs = slice_text_runs(runs, content_range.clone());
                let mut preview_runs = match &entities {
                    Some(entities) => entity_runs(&source_runs, entities),
                    None => source_runs,
                };
                let (layout, scaled_source) = if separator && column_layout {
                    let count = (f32::from(content_width) / f32::from(advance("-")))
                        .floor()
                        .max(1.) as usize;
                    let mut rule = "-".repeat(count);
                    if raw.trim().starts_with(':') {
                        rule.replace_range(..1, ":");
                    }
                    if raw.trim().ends_with(':') {
                        rule.replace_range(count - 1.., ":");
                    }
                    let mut run = base.clone();
                    run.len = rule.len();
                    (shape(rule.into(), &[run], None), true)
                } else {
                    (
                        shape(
                            entities
                                .as_ref()
                                .map_or_else(
                                    || content.to_owned(),
                                    |entities| entities.text.clone(),
                                )
                                .into(),
                            &preview_runs,
                            // Floating-point sums can round down an exact fit.
                            (column_layout && wrap_width.is_some())
                                .then_some(text_width + px(0.01)),
                        ),
                        false,
                    )
                };
                if scaled_source {
                    let mut run = runs.first().cloned().unwrap_or_default();
                    run.len = layout.text.len();
                    preview_runs = vec![run];
                }
                if !column_layout {
                    // Ordinary table editing paints every source space at its native advance.
                    // Only narrow columns and active resize gestures need a shared wrap grid.
                    width = layout.width();
                }
                let alignment_offset = if markdown
                    && !separator
                    && raw.len() - raw.trim_start().len() > raw.len() - raw.trim_end().len()
                    && layout.wrap_boundaries().is_empty()
                {
                    (text_width - layout.width()).max(Pixels::ZERO)
                } else {
                    Pixels::ZERO
                };
                fragments.push(TableVisualFragment {
                    preview_runs,
                    display_range: range,
                    content_range,
                    x: x + pipe
                        + if (separator && column_layout) || markdown {
                            padding
                        } else {
                            Pixels::ZERO
                        }
                        + alignment_offset,
                    width: if column_layout {
                        text_width - alignment_offset
                    } else {
                        width
                    },
                    layout,
                    delimiter: false,
                    scaled_source,
                    entities,
                });
            }
            if !column_layout && let Some(column_width) = widths.get_mut(column) {
                *column_width = width;
            }
            if delimiters.get(column + 1).is_some() {
                x += pipe + width;
            }
        }
        let visual_rows = fragments
            .iter()
            .map(|fragment| fragment.layout.wrap_boundaries().len() + 1)
            .max()
            .unwrap_or(1);
        let width = fragments
            .iter()
            .map(|fragment| fragment.x + fragment.width)
            .max()
            .unwrap_or_default();
        Some(Arc::new(TableVisualLayout {
            fragments,
            width,
            len: text.len(),
            visual_rows,
            column_widths: widths,
            metrics: metrics.clone(),
        }))
    }
}

fn cell_entities(source: &str, format: DocumentFormat, caret: Option<usize>) -> Option<EntityText> {
    (format == DocumentFormat::Org && source.contains('\\'))
        .then(|| EntityText::new(source, caret))
        .filter(EntityText::has_replacements)
}

fn entity_runs(runs: &[TextRun], entities: &EntityText) -> Vec<TextRun> {
    let mut offset = 0;
    runs.iter()
        .filter_map(|run| {
            let start = entities.source_to_display(offset);
            offset += run.len;
            let end = entities.source_to_display(offset);
            (start < end).then(|| {
                let mut run = run.clone();
                run.len = end - start;
                run
            })
        })
        .collect()
}

fn slice_text_runs(runs: &[TextRun], range: Range<usize>) -> Vec<TextRun> {
    let mut sliced = Vec::new();
    let mut offset = 0usize;
    for run in runs {
        let run_range = offset..offset + run.len;
        let start = run_range.start.max(range.start);
        let end = run_range.end.min(range.end);
        if start < end {
            let mut run = run.clone();
            run.len = end - start;
            sliced.push(run);
        }
        offset = run_range.end;
        if offset >= range.end {
            break;
        }
    }
    sliced
}

/// Cap long columns equally, retaining short columns at their natural width.
fn fit_columns(widths: &mut [Pixels], budget: Pixels) {
    let mut sorted = widths.to_vec();
    sorted.sort_by(|a, b| f32::from(*a).total_cmp(&f32::from(*b)));
    let mut remaining = budget;
    for (index, width) in sorted.iter().enumerate() {
        let cap = remaining / (sorted.len() - index) as f32;
        if *width >= cap {
            for width in widths {
                *width = (*width).min(cap);
            }
            break;
        }
        remaining -= *width;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_budget_preserves_short_columns_and_shares_remaining_space() {
        for (budget, expected) in [
            (300., [20., 80., 200.]),
            (180., [20., 80., 80.]),
            (100., [20., 40., 40.]),
            (30., [10., 10., 10.]),
        ] {
            let mut widths = [px(20.), px(80.), px(200.)];
            fit_columns(&mut widths, px(budget));
            assert_eq!(widths, expected.map(px));
        }
    }
}
