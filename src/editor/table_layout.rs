//! Source table shaping and geometry shared by the editor and minimap.
use std::{ops::Range, sync::Arc};

use gpui::{Bounds, Pixels, Point, TextRun, WrappedLine, point, px};

use super::{org_commands::TableColumns, syntax};
use crate::document::DocumentFormat;
use crate::document::TextProjection;
use crate::org_semantic::display::{InlineDisplayContext, OrgDisplayOptions, project_inline};

mod metrics;
pub(super) use metrics::TableColumnMetrics;

#[derive(Default)]
pub(super) struct TableLayoutOptions<'a> {
    pub(super) wrap_width: Option<Pixels>,
    pub(super) preferred_widths_em: Option<&'a [f32]>,
    pub(super) caret: Option<usize>,
    pub(super) display_options: OrgDisplayOptions,
    pub(super) reveal: &'a [Range<usize>],
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
    projection: Option<TextProjection>,
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
            .projection
            .as_ref()
            .map_or(local, |projection| projection.source_to_display(local));
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
            .projection
            .as_ref()
            .map_or(local, |projection| projection.display_to_source(local));
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

    pub(super) fn shape(
        text: &gpui::SharedString,
        runs: &[TextRun],
        columns: &TableColumns,
        metrics: &Arc<TableColumnMetrics>,
        text_system: &gpui::WindowTextSystem,
        options: TableLayoutOptions<'_>,
    ) -> Option<Arc<TableVisualLayout>> {
        let TableLayoutOptions {
            wrap_width,
            preferred_widths_em,
            caret,
            display_options,
            reveal,
        } = options;
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
                    || match columns.format {
                        DocumentFormat::Org => metrics.raw_widths.clone(),
                        DocumentFormat::Markdown => metrics.natural_widths.clone(),
                    },
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
                *width = (*width).max(match columns.format {
                    DocumentFormat::Org => metrics.raw_widths[column],
                    DocumentFormat::Markdown => metrics.natural_widths[column],
                });
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
        // Every row uses the table's measured columns, including unconstrained Org rows.
        // Once a cell hides source bytes, its source padding cannot define visual borders.
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
                projection: None,
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
                projection: None,
            });
            let end = delimiters.get(column + 1).copied().unwrap_or(text.len());
            let range = start + 1..end;
            let width = widths.get(column).copied().unwrap_or_default();
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
                let text_width = if separator || markdown {
                    content_width
                } else if metrics
                    .raw_widths
                    .get(column)
                    .is_some_and(|raw| width < *raw)
                {
                    // Wrapped content leaves a menu gutter; fitting source spaces retain
                    // their native positions instead of wrapping the last space early.
                    (width - padding).max(px(1.))
                } else {
                    width.max(px(1.))
                };
                let cell_reveal = reveal
                    .iter()
                    .filter_map(|range| {
                        let start = range.start.max(content_range.start);
                        let end = range.end.min(content_range.end);
                        (start < end)
                            .then(|| start - content_range.start..end - content_range.start)
                    })
                    .collect::<Vec<_>>();
                let projection = (!separator)
                    .then(|| {
                        cell_projection(
                            content,
                            columns.format,
                            caret
                                .filter(|caret| content_range.contains(caret))
                                .map(|caret| caret - content_range.start),
                            display_options,
                            &cell_reveal,
                        )
                    })
                    .flatten();
                let source_runs = slice_text_runs(runs, content_range.clone());
                let mut preview_runs = match &projection {
                    Some(projection) => project_runs(&source_runs, projection),
                    None => source_runs,
                };
                let (layout, scaled_source) = if separator {
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
                            projection
                                .as_ref()
                                .map_or_else(
                                    || content.to_owned(),
                                    |projection| projection.text.clone(),
                                )
                                .into(),
                            &preview_runs,
                            // Floating-point sums can round down an exact fit.
                            wrap_width.is_some().then_some(text_width + px(0.01)),
                        ),
                        false,
                    )
                };
                if scaled_source {
                    let mut run = runs.first().cloned().unwrap_or_default();
                    run.len = layout.text.len();
                    preview_runs = vec![run];
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
                        + if separator || markdown {
                            padding
                        } else {
                            Pixels::ZERO
                        }
                        + alignment_offset,
                    width: text_width - alignment_offset,
                    layout,
                    delimiter: false,
                    scaled_source,
                    projection,
                });
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

pub(super) fn cell_projection(
    source: &str,
    format: DocumentFormat,
    caret: Option<usize>,
    options: OrgDisplayOptions,
    reveal: &[Range<usize>],
) -> Option<TextProjection> {
    (format == DocumentFormat::Org)
        .then(|| {
            project_inline(
                source,
                options,
                InlineDisplayContext {
                    table: true,
                    caret,
                    reveal,
                    ..Default::default()
                },
            )
        })
        .filter(TextProjection::has_replacements)
}

fn project_runs(runs: &[TextRun], projection: &TextProjection) -> Vec<TextRun> {
    let mut offset = 0;
    runs.iter()
        .filter_map(|run| {
            let start = projection.source_to_display(offset);
            offset += run.len;
            let end = projection.source_to_display(offset);
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
