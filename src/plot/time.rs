use jiff::{Timestamp, civil::date, fmt::strtime::BrokenDownTime, tz::TimeZone};

use super::{PlotAxisSpec, PlotDiagnostic, PlotTick};
use crate::org_semantic::{TimestampKind, timestamp_edit::TimestampDraft};

fn org_datetime(value: &str) -> Option<jiff::civil::DateTime> {
    let draft = TimestampDraft::parse(value, TimestampKind::Plain)?;
    let stamp = draft.start.value;
    Some(
        stamp
            .start_date
            .to_datetime(stamp.start_time.unwrap_or(jiff::civil::Time::MIN)),
    )
}

pub(super) fn is_org_time_column(labels: &[String]) -> bool {
    let mut values = labels
        .iter()
        .filter(|value| !value.is_empty() && value.as_str() != "NULL")
        .peekable();
    values.peek().is_some() && values.all(|value| org_datetime(value).is_some())
}

pub(super) fn parse_column(
    values: &[String],
    format: Option<&str>,
) -> Result<Vec<Option<f64>>, PlotDiagnostic> {
    values
        .iter()
        .enumerate()
        .map(|(row, value)| {
            if value.is_empty() || value == "NULL" {
                return Ok(None);
            }
            let parsed = org_datetime(value)
                .and_then(|value| value.to_zoned(TimeZone::UTC).ok())
                .map(|value| value.timestamp())
                .or_else(|| {
                    let mut parsed =
                        BrokenDownTime::parse(format.unwrap_or("%Y-%m-%d-%H:%M:%S"), value).ok()?;
                    if parsed.year().is_some()
                        && parsed.day().is_none()
                        && parsed.day_of_year().is_none()
                        && parsed.iso_week().is_none()
                        && parsed.sunday_based_week().is_none()
                        && parsed.monday_based_week().is_none()
                    {
                        parsed.set_month(Some(parsed.month().unwrap_or(1))).ok()?;
                        parsed.set_day(Some(1)).ok()?;
                    }
                    parsed.to_timestamp().ok().or_else(|| {
                        if parsed.offset().is_some() {
                            return None;
                        }
                        let time = parsed.to_time().ok()?;
                        let calendar = parsed.year().is_some()
                            || parsed.iso_week_year().is_some()
                            || parsed.month().is_some()
                            || parsed.day().is_some()
                            || parsed.day_of_year().is_some();
                        let day = if calendar {
                            parsed.to_date().ok()?
                        } else if parsed.hour().is_some() {
                            date(1970, 1, 1)
                        } else {
                            return None;
                        };
                        day.to_datetime(time)
                            .to_zoned(TimeZone::UTC)
                            .ok()
                            .map(|value| value.timestamp())
                    })
                });
            parsed
                .map(|value| Some(value.as_millisecond() as f64 / 1000.0))
                .ok_or_else(|| {
                    PlotDiagnostic::new(
                        "invalid-time",
                        format!(
                            "Row {}: expected an Org timestamp or time matching {}",
                            row + 1,
                            format.unwrap_or("%Y-%m-%d-%H:%M:%S")
                        ),
                    )
                })
        })
        .collect()
}

pub(super) fn axis(
    values: &[f64],
    bounds: PlotAxisSpec,
    ticks: Option<usize>,
) -> Result<(f64, f64, Vec<PlotTick>), PlotDiagnostic> {
    let mut min = bounds
        .min
        .unwrap_or_else(|| values.iter().copied().reduce(f64::min).unwrap_or(0.0));
    let mut max = bounds
        .max
        .unwrap_or_else(|| values.iter().copied().reduce(f64::max).unwrap_or(1.0));
    if min == max && (bounds.min.is_none() || bounds.max.is_none()) {
        if bounds.min.is_none() {
            min -= 43200.0;
        }
        if bounds.max.is_none() {
            max += 43200.0;
        }
    }
    if !min.is_finite()
        || !max.is_finite()
        || max <= min
        || Timestamp::from_second(min.floor() as i64).is_err()
        || Timestamp::from_second(max.ceil() as i64).is_err()
    {
        return Err(PlotDiagnostic::new(
            "invalid-range",
            "The time axis must have valid, increasing timestamp bounds",
        ));
    }
    let count = ticks.unwrap_or(4).max(1);
    let rough = (max - min) / count as f64;
    if rough >= 2419200.0 {
        return calendar_axis(min, max, rough, ticks == Some(0));
    }
    let step = [
        0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1., 5., 15., 30., 60., 300., 900., 1800., 3600., 7200.,
        10800., 14400., 21600., 43200., 86400., 172800., 604800., 1209600., 2592000.,
    ]
    .into_iter()
    .find(|step| *step >= rough)
    .unwrap_or(2592000.0);
    let format = if max - min >= 172800. {
        "%Y-%m-%d"
    } else if min >= 0.0 && max < 86400.0 {
        if step >= 60.0 { "%H:%M" } else { "%H:%M:%S" }
    } else if max - min >= 3600. {
        "%m-%d %H:%M"
    } else {
        "%H:%M:%S"
    };
    let mut labels = Vec::new();
    let mut value = (min / step).ceil() * step;
    while ticks != Some(0) && value <= max && labels.len() < 49 {
        let stamp = Timestamp::from_millisecond((value * 1000.0).round() as i64).map_err(|_| {
            PlotDiagnostic::new(
                "invalid-time",
                "Time tick is outside the supported date range",
            )
        })?;
        let mut label = stamp.strftime(format).to_string();
        if step < 1.0 {
            label.push_str(&format!(".{:03}", stamp.as_millisecond().rem_euclid(1000)));
        }
        labels.push(PlotTick {
            position: (value - min) / (max - min),
            label,
        });
        value += step;
    }
    Ok((min, max, labels))
}

fn calendar_axis(
    min: f64,
    max: f64,
    rough: f64,
    hidden: bool,
) -> Result<(f64, f64, Vec<PlotTick>), PlotDiagnostic> {
    let months = [1, 2, 3, 6, 12, 24, 60, 120, 600, 1200, 6000, 12000]
        .into_iter()
        .find(|months| *months as f64 * 2592000.0 >= rough)
        .unwrap_or_else(|| (rough / 31536000.0).ceil() as i32 * 12);
    let start = Timestamp::from_second(min.floor() as i64)
        .unwrap()
        .to_zoned(TimeZone::UTC)
        .date();
    let ordinal = i32::from(start.year()) * 12 + i32::from(start.month()) - 1;
    let mut month = ordinal.div_euclid(months) * months;
    let mut labels = Vec::new();
    while !hidden && labels.len() < 49 {
        let Ok(day) = jiff::civil::Date::new(
            month.div_euclid(12) as i16,
            (month.rem_euclid(12) + 1) as i8,
            1,
        ) else {
            break;
        };
        let Ok(zoned) = day.at(0, 0, 0, 0).to_zoned(TimeZone::UTC) else {
            month += months;
            continue;
        };
        let value = zoned.timestamp().as_second() as f64;
        if value > max {
            break;
        }
        if value >= min {
            labels.push(PlotTick {
                position: (value - min) / (max - min),
                label: day
                    .strftime(if months >= 12 { "%Y" } else { "%Y-%m" })
                    .to_string(),
            });
        }
        month += months;
    }
    Ok((min, max, labels))
}

/// Rebuild labels for the canvas, keeping the prepared data coordinates intact.
pub(super) fn reflow(
    plan: &mut super::PlotPlan,
    width: u32,
    font_size: f32,
) -> Result<(), PlotDiagnostic> {
    let Some(domain) = plan.time_axis else {
        return Ok(());
    };
    plan.x_context.clear();
    plan.x_minor_ticks.clear();
    let span = domain.max - domain.min;
    let count = domain.ticks.unwrap_or_else(|| {
        ((width as f32 - 100.0).max(1.0) / (font_size * 5.0))
            .floor()
            .clamp(2.0, 24.0) as usize
    });
    let (_, _, mut ticks) = axis(
        &[domain.min, domain.max],
        PlotAxisSpec {
            min: Some(domain.min),
            max: Some(domain.max),
        },
        Some(count),
    )?;
    if count == 0 {
        plan.x_ticks.clear();
        return Ok(());
    }
    // Intraday charts put the date on a second row rather than repeating it.
    if span < 172800.0 && !(domain.min >= 0.0 && domain.max < 86400.0) {
        let format = if span >= 3600.0 { "%H:%M" } else { "%H:%M:%S" };
        let gap = font_size as f64 * 4.5 / (width as f64 - 100.0).max(1.0);
        ticks.retain(|tick| tick.position >= gap && tick.position <= 1.0 - gap);
        ticks.insert(
            0,
            PlotTick {
                position: 0.0,
                label: String::new(),
            },
        );
        ticks.push(PlotTick {
            position: 1.0,
            label: String::new(),
        });
        for tick in &mut ticks {
            let value = domain.min + tick.position * span;
            let stamp = Timestamp::from_millisecond((value * 1000.0).round() as i64).unwrap();
            tick.label = stamp.strftime(format).to_string();
            if span < 3600.0
                && (span / (count as f64) < 1.0
                    || domain.min.fract() != 0.0
                    || domain.max.fract() != 0.0)
            {
                tick.label
                    .push_str(&format!(".{:03}", stamp.as_millisecond().rem_euclid(1000)));
            }
        }
        let first = Timestamp::from_second(domain.min.floor() as i64).unwrap();
        plan.x_context.push(PlotTick {
            position: 0.0,
            label: first.strftime("%m-%d").to_string(),
        });
        let mut midnight = (domain.min / 86400.0).floor() * 86400.0 + 86400.0;
        while midnight <= domain.max {
            let position = (midnight - domain.min) / span;
            let label = Timestamp::from_second(midnight as i64)
                .unwrap()
                .strftime("%m-%d")
                .to_string();
            // Keep both dates readable when the first day occupies only a few pixels.
            if position * (width as f64 - 100.0) < font_size as f64 * 4.0 {
                plan.x_context[0].label.push_str(&format!(" / {label}"));
            } else {
                plan.x_context.push(PlotTick { position, label });
            }
            midnight += 86400.0;
        }
    }
    // Minor ticks indicate individual hours without repeating every label.
    if (7200.0..=172800.0).contains(&span) && (width as f64 - 100.0) / (span / 3600.0) >= 8.0 {
        let mut value = (domain.min / 3600.0).ceil() * 3600.0;
        while value <= domain.max {
            plan.x_minor_ticks.push((value - domain.min) / span);
            value += 3600.0;
        }
    }
    plan.x_ticks = ticks;
    Ok(())
}
