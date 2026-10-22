use super::{
    PlotAxisSpec, PlotDiagnostic, PlotInput, PlotKind, PlotLayout, PlotPlan, PlotPlanSeries,
    PlotTick, PlotX, PreparedPlot,
};

pub(super) fn build_plan(input: PlotInput) -> Result<Option<PreparedPlot>, PlotDiagnostic> {
    if input.layout == PlotLayout::Radar {
        return radar(input);
    }
    let rows = match &input.x {
        PlotX::Numeric(x) | PlotX::Time(x) => x.len(),
        PlotX::Category(x) => x.len(),
    };
    let valid_x = |index: usize| match &input.x {
        PlotX::Numeric(x) | PlotX::Time(x) => x[index].is_some(),
        PlotX::Category(_) => true,
    };
    let y = input
        .series
        .iter()
        .flat_map(|series| {
            series
                .values
                .iter()
                .enumerate()
                .filter_map(|(i, value)| valid_x(i).then_some(*value).flatten())
        })
        .collect::<Vec<_>>();
    if y.is_empty() {
        return Ok(None);
    }
    let bars = input.kind == PlotKind::Histograms;
    let zero =
        bars || matches!(input.kind, PlotKind::Boxes | PlotKind::Impulses) || input.fill_to_zero;
    let (y_min, y_max, y_ticks) = axis(&y, zero, input.y_axis, input.ticks)?;
    let raw_boxes = if input.kind == PlotKind::Boxes {
        box_bounds(&input.x, rows)
    } else {
        Vec::new()
    };
    let cells = bars
        || input.layout == PlotLayout::Grid
        || (input.kind == PlotKind::Boxes && matches!(input.x, PlotX::Category(_)));
    let numeric_axis = if cells {
        None
    } else if let PlotX::Time(x) = &input.x {
        let mut values = x.iter().copied().flatten().collect::<Vec<_>>();
        values.extend(raw_boxes.iter().flatten().flat_map(|(a, b)| [*a, *b]));
        Some(super::time::axis(&values, input.x_axis, input.ticks)?)
    } else if let PlotX::Numeric(x) = &input.x {
        let mut values = x.iter().copied().flatten().collect::<Vec<_>>();
        values.extend(raw_boxes.iter().flatten().flat_map(|(a, b)| [*a, *b]));
        Some(axis(&values, false, input.x_axis, input.ticks)?)
    } else {
        None
    };
    let position = |index: usize| {
        if let Some((min, max, _)) = &numeric_axis {
            match &input.x {
                PlotX::Numeric(x) | PlotX::Time(x) => {
                    x[index].map(|value| normalize(value, *min, *max))
                }
                _ => unreachable!(),
            }
        } else if cells {
            Some((index as f64 + 0.5) / rows.max(1) as f64)
        } else {
            Some(if rows <= 1 {
                0.5
            } else {
                index as f64 / (rows - 1) as f64
            })
        }
    };
    let x_ticks = if let Some((_, _, ticks)) = &numeric_axis {
        ticks.clone()
    } else {
        (0..rows)
            .map(|index| PlotTick {
                position: position(index).unwrap_or(0.0),
                label: match &input.x {
                    PlotX::Category(labels) => labels[index].clone(),
                    PlotX::Numeric(values) | PlotX::Time(values) => {
                        format_number(values[index].unwrap_or(index as f64), 1.0)
                    }
                },
            })
            .collect()
    };
    let series = input
        .series
        .iter()
        .map(|series| PlotPlanSeries {
            label: series.label.clone(),
            points: series
                .values
                .iter()
                .enumerate()
                .map(|(index, y)| {
                    position(index)
                        .zip(*y)
                        .map(|(x, y)| (x, normalize(y, y_min, y_max)))
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    if series
        .iter()
        .flat_map(|series| series.points.iter().flatten())
        .any(|(x, y)| !x.is_finite() || !y.is_finite())
    {
        return Err(range_error());
    }
    let content_key = serde_json::to_vec(&input)
        .map_err(|error| PlotDiagnostic::new("invalid-number", error.to_string()))?
        .into();
    Ok(Some(PreparedPlot {
        content_key,
        plan: PlotPlan {
            layout: input.layout,
            kind: input.kind,
            title: input.title,
            x_ticks,
            y_ticks,
            series,
            zero: normalize(0.0, y_min, y_max),
            rows,
            warnings: input.warnings,
            radar_ticks: Vec::new(),
            box_bounds: raw_boxes
                .into_iter()
                .map(|bounds| {
                    bounds.map(|(a, b)| {
                        if let Some((min, max, _)) = &numeric_axis {
                            (normalize(a, *min, *max), normalize(b, *min, *max))
                        } else {
                            (a / rows.max(1) as f64, b / rows.max(1) as f64)
                        }
                    })
                })
                .collect(),
            fill_to_zero: input.fill_to_zero,
            clip: input.x_axis.min.is_some()
                || input.x_axis.max.is_some()
                || input.y_axis.min.is_some()
                || input.y_axis.max.is_some()
                || input.kind == PlotKind::HiSteps,
        },
    }))
}

fn box_bounds(x: &PlotX, rows: usize) -> Vec<Option<(f64, f64)>> {
    let values = match x {
        PlotX::Numeric(values) | PlotX::Time(values) => values.clone(),
        PlotX::Category(_) => (0..rows).map(|index| Some(index as f64 + 0.5)).collect(),
    };
    let mut sorted = values.iter().copied().flatten().collect::<Vec<_>>();
    sorted.sort_by(f64::total_cmp);
    sorted.dedup();
    values
        .iter()
        .map(|value| {
            let value = (*value)?;
            let index = sorted
                .binary_search_by(|probe| probe.total_cmp(&value))
                .ok()?;
            let left = index
                .checked_sub(1)
                .map(|index| (sorted[index] + value) / 2.0)
                .unwrap_or_else(|| {
                    value - sorted.get(index + 1).map_or(1.0, |next| next - value) / 2.0
                });
            let right = sorted
                .get(index + 1)
                .map(|next| (value + next) / 2.0)
                .unwrap_or_else(|| {
                    value
                        + index
                            .checked_sub(1)
                            .map_or(1.0, |index| value - sorted[index])
                            / 2.0
                });
            Some((left, right))
        })
        .collect()
}

fn radar(input: PlotInput) -> Result<Option<PreparedPlot>, PlotDiagnostic> {
    let PlotX::Category(labels) = &input.x else {
        unreachable!("radar axes are categorical")
    };
    if input
        .series
        .iter()
        .all(|series| series.values.iter().all(Option::is_none))
    {
        return Ok(None);
    }
    let mut axes = Vec::new();
    let mut radar_ticks = Vec::new();
    let count = input.ticks.unwrap_or(4).max(1);
    for row in 0..labels.len() {
        let values = input
            .series
            .iter()
            .filter_map(|series| series.values[row])
            .collect::<Vec<_>>();
        let (min, max, _) = axis(&values, false, input.y_axis, Some(count))?;
        axes.push((min, max));
        radar_ticks.push(if input.ticks == Some(0) {
            Vec::new()
        } else {
            (0..=count)
                .map(|index| PlotTick {
                    position: index as f64 / count as f64,
                    label: format_number(
                        min + (max - min) * index as f64 / count as f64,
                        (max - min) / count as f64,
                    ),
                })
                .collect()
        });
    }
    let series = input
        .series
        .iter()
        .map(|series| PlotPlanSeries {
            label: series.label.clone(),
            points: series
                .values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    value.map(|value| {
                        (
                            index as f64 / labels.len() as f64,
                            normalize(value, axes[index].0, axes[index].1).clamp(0.0, 1.0),
                        )
                    })
                })
                .collect(),
        })
        .collect();
    let content_key = serde_json::to_vec(&input)
        .map_err(|error| PlotDiagnostic::new("invalid-number", error.to_string()))?
        .into();
    Ok(Some(PreparedPlot {
        content_key,
        plan: PlotPlan {
            layout: PlotLayout::Radar,
            kind: input.kind,
            title: input.title,
            x_ticks: labels
                .iter()
                .enumerate()
                .map(|(index, label)| PlotTick {
                    position: index as f64 / labels.len() as f64,
                    label: label.clone(),
                })
                .collect(),
            y_ticks: Vec::new(),
            series,
            zero: 0.0,
            rows: labels.len(),
            warnings: input.warnings,
            radar_ticks,
            box_bounds: Vec::new(),
            fill_to_zero: false,
            clip: false,
        },
    }))
}

fn normalize(value: f64, min: f64, max: f64) -> f64 {
    (value - min) / (max - min)
}

fn axis(
    values: &[f64],
    include_zero: bool,
    bounds: PlotAxisSpec,
    ticks: Option<usize>,
) -> Result<(f64, f64, Vec<PlotTick>), PlotDiagnostic> {
    if bounds
        .min
        .zip(bounds.max)
        .is_some_and(|(min, max)| min >= max)
    {
        return Err(PlotDiagnostic::new(
            "invalid-range",
            "Axis minimum must be smaller than its maximum",
        ));
    }
    let mut min = values.iter().copied().reduce(f64::min).unwrap_or(0.0);
    let mut max = values.iter().copied().reduce(f64::max).unwrap_or(1.0);
    if include_zero {
        min = min.min(0.0);
        max = max.max(0.0);
    }
    if min == max {
        let pad = (min.abs() * 0.05).max(1.0);
        min -= pad;
        max += pad;
        if include_zero && values.iter().all(|value| *value >= 0.0) {
            min = 0.0;
        }
    }
    min = bounds.min.unwrap_or(min);
    max = bounds.max.unwrap_or(max);
    let span = max - min;
    if !span.is_finite() || span <= 0.0 {
        return Err(range_error());
    }
    let rough = span / ticks.unwrap_or(3).max(1) as f64;
    let unit = 10_f64.powf(rough.log10().floor());
    let fraction = rough / unit;
    let step = [1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .find(|step| *step >= fraction)
        .unwrap_or(10.0)
        * unit;
    if !step.is_finite() || step <= 0.0 {
        return Err(range_error());
    }
    let low = bounds.min.unwrap_or_else(|| (min / step).floor() * step);
    let high = bounds.max.unwrap_or_else(|| (max / step).ceil() * step);
    if !low.is_finite() || !high.is_finite() || !(high - low).is_finite() || high <= low {
        return Err(range_error());
    }
    let first = (low / step).ceil() * step;
    let count = ((high - first) / step).floor().max(0.0) as usize;
    if count > 12 {
        return Err(range_error());
    }
    let ticks = (0..=count)
        .filter(|_| ticks != Some(0))
        .map(|index| {
            let value = first + index as f64 * step;
            PlotTick {
                position: normalize(value, low, high),
                label: format_number(value, step),
            }
        })
        .collect();
    Ok((low, high, ticks))
}

fn range_error() -> PlotDiagnostic {
    PlotDiagnostic::new(
        "invalid-range",
        "The numeric range cannot be represented safely; scale the values in the source table",
    )
}

fn format_number(value: f64, step: f64) -> String {
    let value = if value.abs() < step.abs() * 1e-8 {
        0.0
    } else {
        value
    };
    if value != 0.0 && (value.abs() >= 1e7 || value.abs() < 0.0001) {
        let precision =
            (value.abs().log10().floor() - step.abs().log10().floor()).clamp(2.0, 16.0) as usize;
        return format!("{value:.precision$e}");
    }
    let decimals = if step >= 1.0 {
        if step.fract() == 0.0 { 0 } else { 1 }
    } else {
        (-step.log10().floor() as i32 + 1).clamp(0, 12) as usize
    };
    let number = format!("{value:.decimals$}");
    if number.contains('.') {
        number
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    } else {
        number
    }
}
