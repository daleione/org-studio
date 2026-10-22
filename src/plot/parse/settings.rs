use super::{Value, atom, syntax};
use crate::plot::{HistogramStyle, PlotDiagnostic, PlotKind, PlotLayout, PlotSpec};

pub(super) fn apply(values: &[Value], spec: &mut PlotSpec) -> Result<(), PlotDiagnostic> {
    for value in values {
        let text = atom(value)?;
        let tokens = text.split_whitespace().collect::<Vec<_>>();
        match tokens.as_slice() {
            ["style", "histogram", mode] => {
                spec.histogram = Some(match *mode {
                    "clustered" => HistogramStyle::Clustered,
                    "rowstacked" => HistogramStyle::RowStacked,
                    "columnstacked" => HistogramStyle::ColumnStacked,
                    _ => return Err(unsupported(text)),
                });
            }
            ["boxwidth", width] | ["boxwidth", width, "absolute"] => {
                spec.box_width = Some(positive(width)?);
            }
            ["style", "boxplot", options @ ..] if !options.is_empty() => {
                let style = spec.boxplot.get_or_insert_default();
                let mut options = options;
                while !options.is_empty() {
                    match options {
                        ["range", range, rest @ ..] => {
                            style.range = positive(range)?;
                            options = rest;
                        }
                        ["outliers", rest @ ..] => {
                            style.outliers = true;
                            options = rest;
                        }
                        ["nooutliers", rest @ ..] => {
                            style.outliers = false;
                            options = rest;
                        }
                        _ => return Err(unsupported(text)),
                    }
                }
            }
            _ => return Err(unsupported(text)),
        }
    }
    let kind = spec.kind.unwrap_or(PlotKind::Lines);
    if (spec.histogram.is_some() && kind != PlotKind::Histograms)
        || (spec.boxplot.is_some() && kind != PlotKind::BoxPlot)
        || (spec.box_width.is_some() && !matches!(kind, PlotKind::Boxes | PlotKind::BoxPlot))
        || (!values.is_empty() && spec.layout != PlotLayout::Cartesian)
    {
        return Err(syntax("The set option does not apply to this chart style"));
    }
    Ok(())
}

fn positive(text: &str) -> Result<f64, PlotDiagnostic> {
    text.parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| syntax("Box width and boxplot range must be finite positive numbers"))
}

fn unsupported(text: &str) -> PlotDiagnostic {
    PlotDiagnostic::new(
        "unsupported-option",
        format!("Unsupported Org Plot set option: {text}"),
    )
}
