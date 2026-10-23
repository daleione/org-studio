use std::collections::HashMap;

use super::{
    MAX_POINTS, MAX_SERIES, PlotCandidate, PlotDiagnostic, PlotInput, PlotKind, PlotLayout,
    PlotSeries, PlotX,
};
use crate::document::{DocumentFormat, TextSnapshot, table};

pub(super) fn prepare_input(
    snapshot: &dyn TextSnapshot,
    candidate: &PlotCandidate,
) -> Result<PlotInput, PlotDiagnostic> {
    let spec = super::parse::parse_spec(&candidate.options)?;
    if candidate.table.len() > 8 * 1024 * 1024 {
        return Err(PlotDiagnostic::new(
            "limit-exceeded",
            "The chart source table exceeds 8 MiB; select a smaller result",
        ));
    }
    let source = snapshot.copy_range(candidate.table);
    let mut rows = Vec::new();
    let mut header = None;
    let mut saw_separator = false;
    for line in source.lines() {
        let parsed = table::parse_line(line, DocumentFormat::Org);
        if parsed.separator {
            if !saw_separator && rows.len() == 1 {
                header = rows.pop();
            }
            saw_separator = true;
            continue;
        }
        let raw_cells = parsed
            .cells
            .iter()
            .map(|cell| &line[cell.text_range.clone()])
            .collect::<Vec<_>>();
        if raw_cells.iter().all(|cell| alignment_cookie(cell)) {
            continue;
        }
        if raw_cells
            .first()
            .is_some_and(|cell| matches!(*cell, "!" | "^" | "_" | "$" | "#" | "*" | "/"))
        {
            return Err(PlotDiagnostic::new(
                "unsupported-table",
                "Org table control columns are not supported for plotting",
            ));
        }
        rows.push(
            raw_cells
                .into_iter()
                .map(|cell| crate::org_syntax::inline::parse(cell).text)
                .collect::<Vec<_>>(),
        );
        if rows.len() > MAX_POINTS {
            return Err(PlotDiagnostic::new(
                "limit-exceeded",
                format!("A chart supports at most {MAX_POINTS} data points"),
            ));
        }
    }
    if spec.transpose {
        let had_header = header.is_some();
        let source = header.into_iter().chain(rows).collect::<Vec<_>>();
        let columns = source.first().map_or(0, Vec::len);
        if source.iter().any(|row| row.len() != columns) {
            return Err(PlotDiagnostic::new(
                "invalid-table",
                "Cannot transpose a table with inconsistent column counts",
            ));
        }
        rows = (0..columns)
            .map(|column| source.iter().map(|row| row[column].clone()).collect())
            .collect();
        header = if had_header && !rows.is_empty() {
            Some(rows.remove(0))
        } else {
            None
        };
    }
    let columns = header
        .as_ref()
        .map(Vec::len)
        .or_else(|| rows.first().map(Vec::len))
        .unwrap_or(0);
    if spec.x_column > columns
        || spec
            .columns
            .as_ref()
            .is_some_and(|deps| deps.iter().any(|&column| column == 0 || column > columns))
    {
        return Err(PlotDiagnostic::new(
            "invalid-column",
            format!("Org Plot column is outside this table's {columns} columns"),
        ));
    }
    if rows.iter().any(|row| row.len() != columns) {
        return Err(PlotDiagnostic::new(
            "invalid-table",
            "The chart table has inconsistent column counts",
        ));
    }
    let selected = (1..=columns)
        .filter(|column| {
            *column != spec.x_column
                && spec
                    .columns
                    .as_ref()
                    .is_none_or(|deps| deps.contains(column))
        })
        .collect::<Vec<_>>();
    let max_columns = if spec.layout == PlotLayout::Grid {
        128
    } else {
        MAX_SERIES
    };
    if selected.len() > max_columns || rows.len().saturating_mul(selected.len()) > MAX_POINTS {
        return Err(PlotDiagnostic::new(
            "limit-exceeded",
            format!(
                "This chart supports {max_columns} data columns and {MAX_POINTS} data points in total"
            ),
        ));
    }
    if columns > 0 && selected.is_empty() {
        return Err(PlotDiagnostic::new(
            "invalid-column",
            "No data columns remain after selecting the horizontal axis",
        ));
    }
    let kind = spec.kind.unwrap_or(PlotKind::Lines);
    if kind == PlotKind::BoxPlot
        && (spec.x_column != 0
            || spec.time
            || spec.time_format.is_some()
            || spec.x_axis.min.is_some()
            || spec.x_axis.max.is_some())
    {
        return Err(PlotDiagnostic::new(
            "invalid-option",
            "Boxplots summarize each selected numeric column; use ind:0 and deps, without x-axis options",
        ));
    }
    if spec.layout == PlotLayout::Radar && !rows.is_empty() && !(3..=16).contains(&rows.len()) {
        return Err(PlotDiagnostic::new(
            "invalid-table",
            "A radar chart requires 3 to 16 axes; use transpose:yes for axes stored in columns",
        ));
    }
    if spec.layout == PlotLayout::Radar && spec.x_column == 0 {
        return Err(PlotDiagnostic::new(
            "invalid-option",
            "Radar charts require a column of axis names; use ind:1",
        ));
    }
    if spec.layout != PlotLayout::Cartesian
        && (spec.time
            || spec.time_format.is_some()
            || spec.x_axis.min.is_some()
            || spec.x_axis.max.is_some())
    {
        return Err(PlotDiagnostic::new(
            "invalid-option",
            "Time and numeric x-axis options require a 2d chart",
        ));
    }
    if spec.layout == PlotLayout::Grid
        && (spec.y_axis.min.is_some() || spec.y_axis.max.is_some() || spec.ticks.is_some())
    {
        return Err(PlotDiagnostic::new(
            "invalid-option",
            "Grid charts use table row and column labels; numeric axis options do not apply",
        ));
    }
    if spec.time && (spec.x_column == 0 || kind == PlotKind::Histograms) {
        return Err(PlotDiagnostic::new(
            "invalid-option",
            "A time axis requires ind to select a column and a non-histogram 2d style",
        ));
    }
    let mut warnings = Vec::new();
    let x = if spec.x_column == 0 {
        PlotX::Numeric((0..rows.len()).map(|index| Some(index as f64)).collect())
    } else {
        let labels = rows
            .iter()
            .map(|row| row[spec.x_column - 1].clone())
            .collect::<Vec<_>>();
        if spec.layout != PlotLayout::Cartesian || kind == PlotKind::Histograms {
            PlotX::Category(labels)
        } else if spec.time || super::time::is_org_time_column(&labels) {
            PlotX::Time(super::time::parse_column(
                &labels,
                spec.time_format.as_deref(),
            )?)
        } else if labels
            .iter()
            .filter(|value| !missing(value))
            .all(|value| value.parse::<f64>().is_ok())
        {
            PlotX::Numeric(numeric_column(
                labels.iter().map(String::as_str),
                spec.x_column,
                &mut warnings,
            )?)
        } else {
            PlotX::Category(labels)
        }
    };
    if matches!(x, PlotX::Category(_)) && (spec.x_axis.min.is_some() || spec.x_axis.max.is_some()) {
        return Err(PlotDiagnostic::new(
            "invalid-option",
            "xmin and xmax require a numeric or time axis",
        ));
    }
    if spec.time_format.is_some() && !matches!(x, PlotX::Time(_)) {
        return Err(PlotDiagnostic::new(
            "invalid-option",
            "timefmt requires timeind:t or a column of Org timestamps",
        ));
    }
    let series = selected
        .into_iter()
        .map(|column| {
            let label = if let Some(labels) = &spec.labels {
                labels
                    .get(column - 1)
                    .and_then(Clone::clone)
                    .unwrap_or_else(|| column.to_string())
            } else {
                header
                    .as_ref()
                    .and_then(|header| header.get(column - 1))
                    .cloned()
                    .unwrap_or_else(|| column.to_string())
            };
            if label.len() > 2048 {
                return Err(PlotDiagnostic::new(
                    "limit-exceeded",
                    "A chart series label exceeds 2,048 bytes",
                ));
            }
            let values = numeric_column(
                rows.iter().map(|cells| cells[column - 1].as_str()),
                column,
                &mut warnings,
            )?;
            Ok(PlotSeries {
                column: column - 1,
                label,
                values,
            })
        })
        .collect::<Result<Vec<_>, PlotDiagnostic>>()?;
    if let PlotX::Category(labels) = &x
        && labels.iter().any(|label| label.len() > 2048)
    {
        return Err(PlotDiagnostic::new(
            "limit-exceeded",
            "A chart category label exceeds 2,048 bytes",
        ));
    }
    if spec.title.as_ref().is_some_and(|title| title.len() > 2048) {
        return Err(PlotDiagnostic::new(
            "limit-exceeded",
            "The chart title exceeds 2,048 bytes",
        ));
    }
    warnings.sort();
    warnings.dedup();
    Ok(PlotInput {
        layout: spec.layout,
        kind,
        title: spec.title,
        x,
        series,
        warnings,
        x_axis: spec.x_axis,
        y_axis: spec.y_axis,
        ticks: spec.ticks,
        fill_to_zero: spec.fill_to_zero,
        histogram: spec.histogram.unwrap_or_default(),
        box_width: spec.box_width,
        boxplot: spec.boxplot.unwrap_or_default(),
    })
}

fn missing(value: &str) -> bool {
    value.trim().is_empty() || value.trim() == "NULL"
}
fn integer(value: &str) -> Option<(bool, &str)> {
    let negative = value.starts_with('-');
    let digits = value.strip_prefix(['-', '+']).unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let digits = digits.trim_start_matches('0');
    Some((negative && !digits.is_empty(), digits))
}
fn numeric_column<'a>(
    values: impl Iterator<Item = &'a str>,
    column: usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<Option<f64>>, PlotDiagnostic> {
    let mut coordinates = HashMap::new();
    let mut collision = false;
    let values = values
        .enumerate()
        .map(|(row, text)| {
            let value = number(text, row, column)?;
            if let Some(value) = value
                && let Some(exact) = integer(text)
            {
                collision |= coordinates
                    .insert(value.to_bits(), exact)
                    .is_some_and(|previous| previous != exact);
            }
            Ok(value)
        })
        .collect::<Result<_, PlotDiagnostic>>()?;
    if collision {
        warnings.push(format!("Column {column}: distinct integer values share a floating-point coordinate; exact values remain in the table"));
    }
    Ok(values)
}
fn number(value: &str, row: usize, column: usize) -> Result<Option<f64>, PlotDiagnostic> {
    if missing(value) {
        return Ok(None);
    }
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .filter(|number| {
            *number != 0.0
                || !value
                    .split(['e', 'E'])
                    .next()
                    .unwrap_or(value)
                    .bytes()
                    .any(|byte| matches!(byte, b'1'..=b'9'))
        })
        .map(|value| Some(if value == 0.0 { 0.0 } else { value }))
        .ok_or_else(|| {
            PlotDiagnostic::new(
                "invalid-number",
                format!(
                    "Row {}, column {column}: expected a finite number, got '{}'",
                    row + 1,
                    value.chars().take(64).collect::<String>()
                ),
            )
        })
}
fn alignment_cookie(value: &str) -> bool {
    let Some(inner) = value
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
    else {
        return false;
    };
    !inner.is_empty()
        && inner
            .chars()
            .all(|c| matches!(c, 'l' | 'c' | 'r' | '0'..='9'))
}
