use super::*;

#[test]
fn sql_detail_plot_does_not_include_the_independent_summary() {
    let source = include_str!("../../../tests/fixtures/sql-report-summary.org");
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let candidates = collect_plots(&snapshot, &crate::org_syntax::parse(&snapshot));
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert!(!snapshot.copy_range(candidate.table).contains("Total"));
    assert!(candidate.formulas.is_none());
    assert!(candidate.source_end().0 < snapshot.len_bytes());
    let PlotPreparation::Ready(plot) = prepare_plot(&snapshot, candidate) else {
        panic!("the independent summary is outside the chart source")
    };
    assert_eq!(plot.plan.rows, 3);
    assert_eq!(plot.plan.series[0].label, "Value");
    assert!(plot.plan.series[0].points.iter().all(Option::is_some));
    let points = &plot.plan.series[0].points;
    assert!(
        ((points[2].unwrap().0 - points[1].unwrap().0)
            / (points[1].unwrap().0 - points[0].unwrap().0)
            - 3.0)
            .abs()
            < 1e-9
    );
    assert!(matches!(
        prepared(&source.replace("2025-01-02", "2025-02-30")),
        PlotPreparation::Invalid(error) if error.code == "invalid-time"
    ));
}

#[test]
fn plot_table_formulas_stop_at_blank_lines_and_other_keywords() {
    for suffix in ["\n#+TBLFM: $2=$1\n", "#+CAPTION: next\n#+TBLFM: $2=$1\n"] {
        let source = format!("#+PLOT: ind:1\n| 1 | 2 |\n#+tblfm: $2=$1*2\n{suffix}");
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let candidates = collect_plots(&snapshot, &crate::org_syntax::parse(&snapshot));
        assert_eq!(
            snapshot.copy_range(candidates[0].formulas.unwrap()),
            "#+tblfm: $2=$1*2\n"
        );
    }
}

#[test]
fn plot_source_includes_all_attached_formula_lines() {
    let source = "#+PLOT: ind:1\n| 1 | 2 |\n#+TBLFM: $2=$1\n#+tblfm: $2=$1*2\n#+TBLFM: $2=$1*3\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let candidates = collect_plots(&snapshot, &crate::org_syntax::parse(&snapshot));
    assert_eq!(
        snapshot
            .copy_range(candidates[0].formulas.unwrap())
            .lines()
            .count(),
        3
    );
    assert_eq!(candidates[0].source_end().0, snapshot.len_bytes());
}

#[test]
fn plot_headers_end_at_the_first_horizontal_rule() {
    let PlotPreparation::Ready(plot) =
        prepared("#+PLOT: ind:1\n|---+---|\n| first | 2 |\n|---+---|\n| second | 4 |\n")
    else {
        panic!("a table beginning with a rule has no header")
    };
    assert_eq!(plot.plan.rows, 2);
    assert_eq!(plot.plan.x_ticks[0].label, "first");
}
