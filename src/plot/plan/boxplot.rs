use super::{axis, content_key, normalize, range_error};
use crate::plot::{
    PlotBoxSummary, PlotDiagnostic, PlotInput, PlotPlan, PlotPlanSeries, PlotTick, PreparedPlot,
};

pub(super) fn build(input: PlotInput) -> Result<Option<PreparedPlot>, PlotDiagnostic> {
    let count = input.series.len();
    let mut summaries = input
        .series
        .iter()
        .enumerate()
        .map(|(index, series)| {
            let mut values = series.values.iter().copied().flatten().collect::<Vec<_>>();
            values.sort_by(f64::total_cmp);
            if values.is_empty() {
                return None;
            }
            let statistics = if values.len() < 4 {
                None
            } else {
                let q1 = quantile(&values, 1);
                let q3 = quantile(&values, 3);
                let range = input.boxplot.range * (q3 - q1);
                let low = values
                    .iter()
                    .copied()
                    .find(|value| *value >= q1 - range)
                    .unwrap();
                let high = values
                    .iter()
                    .copied()
                    .rev()
                    .find(|value| *value <= q3 + range)
                    .unwrap();
                Some([low, q1, quantile(&values, 2), q3, high])
            };
            let outliers = if input.boxplot.outliers {
                values
                    .into_iter()
                    .filter(|value| statistics.is_none_or(|q| *value < q[0] || *value > q[4]))
                    .collect()
            } else {
                Vec::new()
            };
            Some(PlotBoxSummary {
                position: (index as f64 + 0.5) / count as f64,
                width: input.box_width.map(|width| width / count as f64),
                statistics,
                outliers,
            })
        })
        .collect::<Vec<_>>();
    let values = summaries
        .iter()
        .flatten()
        .flat_map(|summary| {
            summary
                .statistics
                .into_iter()
                .flatten()
                .chain(summary.outliers.iter().copied())
        })
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(None);
    }
    let (min, max, y_ticks) = axis(&values, false, input.y_axis, input.ticks)?;
    for summary in summaries.iter_mut().flatten() {
        if let Some(statistics) = &mut summary.statistics {
            for value in statistics {
                *value = normalize(*value, min, max);
            }
        }
        for value in &mut summary.outliers {
            *value = normalize(*value, min, max);
        }
        if summary
            .statistics
            .iter()
            .flatten()
            .chain(&summary.outliers)
            .any(|value| !value.is_finite())
        {
            return Err(range_error());
        }
    }
    let content_key = content_key(&input)?;
    let series = input
        .series
        .into_iter()
        .zip(summaries)
        .map(|(series, boxplot)| PlotPlanSeries {
            label: series.label,
            points: Vec::new(),
            bases: Vec::new(),
            boxplot,
        })
        .collect::<Vec<_>>();
    let x_ticks = series
        .iter()
        .enumerate()
        .map(|(index, series)| PlotTick {
            position: (index as f64 + 0.5) / count as f64,
            label: series.label.clone(),
        })
        .collect();
    Ok(Some(PreparedPlot::new(
        content_key,
        PlotPlan {
            layout: input.layout,
            kind: input.kind,
            title: input.title,
            x_ticks,
            y_ticks,
            series,
            zero: normalize(0.0, min, max),
            rows: count,
            warnings: input.warnings,
            radar_ticks: Vec::new(),
            box_bounds: Vec::new(),
            fill_to_zero: false,
            clip: input.y_axis.min.is_some()
                || input.y_axis.max.is_some()
                || input.box_width.is_some(),
            stacked: false,
            time_axis: None,
            x_minor_ticks: Vec::new(),
            x_context: Vec::new(),
        },
    )))
}

// Gnuplot averages adjacent ranks only when n*p is integral (p = 1/4, 1/2, 3/4).
fn quantile(sorted: &[f64], quarter: usize) -> f64 {
    let rank = sorted.len() * quarter;
    let index = rank / 4;
    if rank.is_multiple_of(4) {
        sorted[index - 1] / 2.0 + sorted[index] / 2.0
    } else {
        sorted[index]
    }
}
