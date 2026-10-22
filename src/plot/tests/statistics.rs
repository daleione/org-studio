use super::*;

fn ready(source: &str) -> std::sync::Arc<PreparedPlot> {
    match prepared(source) {
        PlotPreparation::Ready(plot) => plot,
        other => panic!("Expected a chart: {other:?}"),
    }
}

#[test]
fn row_stacks_keep_separate_positive_negative_and_missing_totals() {
    let plot = ready(
        "#+PLOT: ind:1 with:histograms set:\"style histogram rowstacked\" ymin:-20 ymax:20\n| category | A | B | C | D |\n|---+---+---+---+---|\n| first | 10 | 5 | -3 | -7 |\n| second | NULL | 2 | NULL | -4 |\n",
    );
    let expected = [(0.5, 0.75), (0.75, 0.875), (0.5, 0.425), (0.425, 0.25)];
    for (series, (base, top)) in plot.plan.series.iter().zip(expected) {
        assert!((series.bases[0].unwrap() - base).abs() < 1e-9);
        assert!((series.points[0].unwrap().1 - top).abs() < 1e-9);
    }
    assert!(plot.plan.series[0].points[1].is_none());
    assert!(plot.plan.series[0].bases[1].is_none());
    assert_eq!(plot.plan.series[1].bases[1], Some(plot.plan.zero));
    let auto = ready(
        "#+PLOT: ind:1 with:histograms set:\"style histogram rowstacked\"\n| row | 70 | 80 | -20 | -100 |\n",
    );
    assert!(
        auto.plan
            .series
            .iter()
            .flat_map(|series| series.points.iter().flatten())
            .all(|(_, y)| (0.0..=1.0).contains(y)),
        "auto range must include the stacked totals"
    );
    assert!(matches!(
        prepared("#+PLOT: with:histograms set:\"style histogram rowstacked\"\n| 1e308 | 1e308 |\n"),
        PlotPreparation::Invalid(_)
    ));
}

#[test]
fn column_stacks_exchange_categories_and_legend_series() {
    let plot = ready(
        "#+PLOT: ind:1 with:histograms set:\"style histogram columnstacked\"\n| month | retail | partner |\n|---+---+---|\n| July | 10 | 5 |\n| August | 20 | NULL |\n",
    );
    assert_eq!(
        plot.plan
            .x_ticks
            .iter()
            .map(|tick| tick.label.as_str())
            .collect::<Vec<_>>(),
        ["retail", "partner"]
    );
    assert_eq!(
        plot.plan
            .series
            .iter()
            .map(|series| series.label.as_str())
            .collect::<Vec<_>>(),
        ["July", "August"]
    );
    assert_eq!(
        plot.plan.series[1].bases[0],
        plot.plan.series[0].points[0].map(|(_, y)| y)
    );
    assert!(plot.plan.series[1].points[1].is_none());
    let source = format!(
        "#+PLOT: with:histograms set:\"style histogram columnstacked\"\n{}",
        "| 1 | 2 |\n".repeat(MAX_SERIES + 1)
    );
    assert!(
        matches!(prepared(&source), PlotPreparation::Invalid(error) if error.code == "limit-exceeded")
    );
}

#[test]
fn frequency_boxes_keep_exact_bin_widths_and_gaps() {
    let plot = ready(
        "#+PLOT: ind:1 with:boxes set:\"boxwidth 50 absolute\" xmin:0 xmax:200 ymin:0 ymax:5\n| midpoint | frequency |\n|---+---|\n| 25 | 3 |\n| 75 | 0 |\n| 175 | 2 |\n",
    );
    assert_eq!(
        plot.plan.box_bounds,
        [Some((0.0, 0.25)), Some((0.25, 0.5)), Some((0.75, 1.0))]
    );
    assert_eq!(
        plot.plan.series[0].points,
        [Some((0.125, 0.6)), Some((0.375, 0.0)), Some((0.875, 0.4))]
    );
    let auto = ready("#+PLOT: ind:1 with:boxes set:\"boxwidth 50\"\n| 25 | 1 |\n| 175 | 2 |\n");
    assert!(
        auto.plan
            .box_bounds
            .iter()
            .flatten()
            .all(|(a, b)| *a >= 0.0 && *b <= 1.0),
        "auto range must include the outer edges of the bins"
    );
    assert!(
        matches!(prepared("#+PLOT: ind:1 with:boxes set:\"boxwidth 1e-300\"\n| 1e100 | 1 |\n"), PlotPreparation::Invalid(error) if error.code == "invalid-range")
    );
}

#[test]
fn boxplots_follow_gnuplot_quartile_ranks_and_iqr_whiskers() {
    for (n, expected) in [
        (4, [1.5, 2.5, 3.5]),
        (5, [2.0, 3.0, 4.0]),
        (6, [2.0, 3.5, 5.0]),
        (7, [2.0, 4.0, 6.0]),
        (8, [2.5, 4.5, 6.5]),
    ] {
        let rows = (1..=n)
            .map(|value| format!("| {value} |\n"))
            .collect::<String>();
        let plot = ready(&format!("#+PLOT: with:boxplot ymin:0 ymax:10\n{rows}"));
        let q = plot.plan.series[0]
            .boxplot
            .as_ref()
            .unwrap()
            .statistics
            .unwrap();
        for (actual, expected) in q[1..4].iter().zip(expected) {
            assert!((actual * 10.0 - expected).abs() < 1e-9, "n={n}");
        }
    }
    let plot = ready(
        "#+PLOT: with:boxplot ymin:0 ymax:100\n| direct | missing | small |\n|---+---+---|\n| 10 | NULL | 9 |\n| 12 | NULL | NULL |\n| 14 | NULL | NULL |\n| 16 | NULL | NULL |\n| 18 | NULL | NULL |\n| 20 | NULL | NULL |\n| 22 | NULL | NULL |\n| 100 | NULL | NULL |\n",
    );
    let summary = plot.plan.series[0].boxplot.as_ref().unwrap();
    assert_eq!(summary.statistics.unwrap(), [0.1, 0.13, 0.17, 0.21, 0.22]);
    assert_eq!(summary.outliers, [1.0]);
    assert!(
        plot.plan.series[1].boxplot.is_none(),
        "NULL is not a zero-valued sample"
    );
    let small = plot.plan.series[2].boxplot.as_ref().unwrap();
    assert!(
        small.statistics.is_none(),
        "fewer than four samples must not invent a box"
    );
    assert_eq!(small.outliers, [0.09]);
}

#[test]
fn repeated_set_values_preserve_org_order_and_boxplot_autoscaling() {
    let source = "#+PLOT: with:boxplot set:\"style boxplot range 1.5\" set:\"boxwidth 0.4\"\n#+PLOT: set:\"style boxplot nooutliers\"\n| 10 |\n| 12 |\n| 14 |\n| 16 |\n| 18 |\n| 20 |\n| 22 |\n| 100 |\n";
    let hidden = ready(source);
    let visible = ready(&source.replace("nooutliers", "outliers"));
    assert_ne!(hidden.content_key, visible.content_key);
    assert!(
        hidden.plan.series[0]
            .boxplot
            .as_ref()
            .unwrap()
            .outliers
            .is_empty()
    );
    assert_eq!(hidden.plan.y_ticks.last().unwrap().label, "25");
    assert_eq!(visible.plan.y_ticks.last().unwrap().label, "100");
    let stacked = ready(
        "#+PLOT: with:histograms set:\"style histogram rowstacked\" set:\"style histogram clustered\"\n| 10 | 20 |\n",
    );
    assert!(
        stacked.plan.stacked,
        "Org applies settings right to left within a declaration"
    );
}

#[test]
fn statistical_styles_reject_unsupported_or_inapplicable_options() {
    for options in [
        "with:lines set:\"style histogram rowstacked\"",
        "with:boxes set:\"boxwidth 0\"",
        "with:boxes set:\"boxwidth NaN\"",
        "with:boxes set:\"boxwidth 1 relative\"",
        "with:histograms set:\"style histogram rowstacked; unset grid\"",
        "with:boxplot set:\"style boxplot range -1\"",
        "with:boxplot ind:1",
        "with:boxplot xmin:0",
        "type:grid set:\"boxwidth 1\"",
    ] {
        assert!(
            matches!(
                prepared(&format!("#+PLOT: {options}\n| 1 | 2 |\n")),
                PlotPreparation::Invalid(_)
            ),
            "{options}"
        );
    }
}
