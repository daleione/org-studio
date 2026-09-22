//! Source table shaping and geometry shared by the editor and minimap.
use std::{ops::Range, sync::Arc};

use gpui::{Bounds, Pixels, Point, TextRun, WrappedLine, point, px};

use super::org_commands::TableColumns;

pub(super) struct TableVisualFragment {
    pub(super) display_range: Range<usize>,
    pub(super) content_range: Range<usize>,
    pub(super) x: Pixels,
    pub(super) width: Pixels,
    pub(super) layout: WrappedLine,
    pub(super) delimiter: bool,
    // Separator glyphs are shortened to fit the column, while their source stays intact.
    pub(super) scaled_source: bool,
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
        self.content_range.start + local.min(self.content_range.len())
    }
}

pub(crate) struct TableVisualLayout {
    pub(super) fragments: Vec<TableVisualFragment>,
    pub(super) width: Pixels,
    pub(super) len: usize,
    pub(super) visual_rows: usize,
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
        font_size: Pixels,
        columns: &TableColumns,
        format: crate::document::DocumentFormat,
        wrap_width: Option<Pixels>,
        text_system: &gpui::WindowTextSystem,
    ) -> Option<Arc<TableVisualLayout>> {
        // Leave room to paint the caret after the final column boundary.
        let wrap_width = wrap_width.map(|width| (width - gpui::px(2.)).max(gpui::px(1.)));
        let delimiters = crate::document::table::delimiter_offsets(text, format);
        if delimiters.len() < 2 || columns.widths.is_empty() {
            return None;
        }
        let mut base = runs.first()?.clone();
        base.len = 1;
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
        let space = advance(" ");
        let pipe = advance("|");
        let indent = text_system
            .shape_line(
                text[..delimiters[0]].to_owned().into(),
                font_size,
                &slice_text_runs(runs, 0..delimiters[0]),
                None,
            )
            .width();
        let mut widths = columns
            .widths
            .iter()
            .map(|width| space * (width + 2))
            .collect::<Vec<_>>();
        let natural_width =
            indent + pipe * (columns.widths.len() + 1) + widths.iter().copied().sum::<Pixels>();
        let wrapped = wrap_width.is_some_and(|width| natural_width > width);
        if !columns.aligned && !wrapped {
            return None;
        }
        if let Some(width) = wrap_width.filter(|_| wrapped) {
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
            });
        }
        let mut x = indent;
        for (column, start) in delimiters.iter().copied().enumerate() {
            let range = start..start + 1;
            fragments.push(TableVisualFragment {
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
            });
            let end = delimiters.get(column + 1).copied().unwrap_or(text.len());
            let range = start + 1..end;
            let width = widths.get(column).copied().unwrap_or_default();
            if !range.is_empty() {
                let raw = &text[range.clone()];
                let padding = if wrapped {
                    space.min(width / 4.)
                } else {
                    Pixels::ZERO
                };
                let content_width = if wrapped {
                    (width - padding * 2.).max(gpui::px(1.))
                } else {
                    width
                };
                let content_range = if wrapped {
                    let trimmed = raw.trim();
                    let start = range.start + raw.len() - raw.trim_start().len();
                    start..start + trimmed.len()
                } else {
                    range.clone()
                };
                let (layout, scaled_source) = if wrapped && separator {
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
                            text[content_range.clone()].to_owned().into(),
                            &slice_text_runs(runs, content_range.clone()),
                            wrapped.then_some(content_width),
                        ),
                        false,
                    )
                };
                fragments.push(TableVisualFragment {
                    display_range: range,
                    content_range,
                    x: x + pipe + padding,
                    width: if wrapped {
                        content_width
                    } else {
                        layout.width()
                    },
                    layout,
                    delimiter: false,
                    scaled_source,
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
        }))
    }
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
