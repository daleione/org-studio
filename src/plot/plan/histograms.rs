use super::{format_number, range_error};
use crate::plot::{HistogramStyle, MAX_SERIES, PlotDiagnostic, PlotInput, PlotSeries, PlotX};

pub(super) struct Stack {
    pub(super) tops: Vec<Option<f64>>,
    pub(super) bases: Vec<Option<f64>>,
}

pub(super) fn transpose_columns(input: &mut PlotInput) -> Result<(), PlotDiagnostic> {
    if input.histogram != HistogramStyle::ColumnStacked {
        return Ok(());
    }
    let labels = match &input.x {
        PlotX::Category(labels) => labels.clone(),
        PlotX::Numeric(values) | PlotX::Time(values) => values
            .iter()
            .enumerate()
            .map(|(row, value)| format_number(value.unwrap_or(row as f64), 1.0))
            .collect(),
    };
    if labels.len() > MAX_SERIES {
        return Err(PlotDiagnostic::new(
            "limit-exceeded",
            format!(
                "A column-stacked chart supports at most {MAX_SERIES} rows; each row becomes a series"
            ),
        ));
    }
    let source = std::mem::take(&mut input.series);
    input.x = PlotX::Category(source.iter().map(|series| series.label.clone()).collect());
    input.series = labels
        .into_iter()
        .enumerate()
        .map(|(row, label)| PlotSeries {
            column: row,
            label,
            values: source.iter().map(|series| series.values[row]).collect(),
        })
        .collect();
    Ok(())
}

pub(super) fn stack(input: &PlotInput, rows: usize) -> Result<Option<Vec<Stack>>, PlotDiagnostic> {
    if input.histogram == HistogramStyle::Clustered {
        return Ok(None);
    }
    let mut positive = vec![0.0; rows];
    let mut negative = vec![0.0; rows];
    input
        .series
        .iter()
        .map(|series| {
            let mut stack = Stack {
                tops: Vec::with_capacity(rows),
                bases: Vec::with_capacity(rows),
            };
            for (row, value) in series.values.iter().enumerate() {
                if let Some(value) = value {
                    let total = if *value >= 0.0 {
                        &mut positive[row]
                    } else {
                        &mut negative[row]
                    };
                    stack.bases.push(Some(*total));
                    *total += value;
                    if !total.is_finite() {
                        return Err(range_error());
                    }
                    stack.tops.push(Some(*total));
                } else {
                    stack.bases.push(None);
                    stack.tops.push(None);
                }
            }
            Ok(stack)
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}
