use super::*;
use crate::document::{DocumentSnapshot, TextSnapshot};

mod layout;
mod statistics;

fn prepared(source: &str) -> PlotPreparation {
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let arena = crate::org_syntax::parse(&snapshot);
    let candidates = collect_plots(&snapshot, &arena);
    assert_eq!(candidates.len(), 1);
    prepare_plot(&snapshot, &candidates[0])
}

#[test]
fn org_plot_binding_options_and_data_share_real_source_semantics() {
    let source = "#+begin_example\n#+PLOT: with:points\n| 0 | 1 |\n#+end_example\n\n#+PLOT: title:\"正确标题\" ind:1 deps:(3 2) labels:(nil \"本期\" \"上期\")\n#+PLOT: title:\"不能覆盖\" with:histograms\n#+name: demo\n| month | first | second |\n|-------+-------+--------|\n| a\\vert{}b | 120 | NULL |\n| 8月 | -20 | 80 |\n";
    let PlotPreparation::Ready(plot) = prepared(source) else {
        panic!("valid plot")
    };
    assert_eq!(plot.plan.kind, PlotKind::Histograms);
    assert_eq!(plot.plan.title.as_deref(), Some("正确标题"));
    assert_eq!(plot.plan.series[0].label, "本期");
    assert_eq!(plot.plan.series[1].label, "上期");
    assert_eq!(plot.plan.x_ticks[0].label, "a|b");
    assert!(plot.plan.series[1].points[0].is_none());
    assert!(plot.plan.zero > 0.0 && plot.plan.zero < 1.0);
    let PlotPreparation::Ready(escaped) = prepared(
        "#+PLOT: ind:1 with:histograms\n| label | value |\n|-------+-------|\n| \\ast{} | 2 |\n| \\under{} | 3 |\n",
    ) else {
        panic!("escaped labels are data, not control rows")
    };
    assert_eq!(
        escaped
            .plan
            .x_ticks
            .iter()
            .map(|tick| tick.label.as_str())
            .collect::<Vec<_>>(),
        ["*", "_"]
    );
    let PlotPreparation::Ready(unlabelled) =
        prepared("#+PLOT: ind:1 deps:nil labels:nil\n| x | y |\n|---+---|\n| a | 2 |\n")
    else {
        panic!("valid nil options")
    };
    assert_eq!(unlabelled.plan.series[0].label, "2");
    for (source, expected) in [
        ("#+PLOT: ind:1 deps:(2)\n| x | y |\n|---+---|\n", "empty"),
        (
            "#+PLOT: ind:1 deps:(2)\n| x | y |\n|---+---|\n| a | NULL |\n",
            "empty",
        ),
        (
            "#+PLOT: script:\"x.typ\"\n| 1 | 2 |\n",
            "unsupported-option",
        ),
        ("#+PLOT: deps:(3)\n| 1 | 2 |\n", "invalid-column"),
        ("#+PLOT: ind:1\n| a | 12x |\n", "invalid-number"),
        ("#+PLOT: ind:1\n| a | 1e-999 |\n", "invalid-number"),
        ("#+PLOT: deps:((2))\n| 1 | 2 |\n", "invalid-option"),
    ] {
        match prepared(source) {
            PlotPreparation::Empty => assert_eq!(expected, "empty"),
            PlotPreparation::Invalid(error) => assert_eq!(error.code, expected),
            PlotPreparation::Ready(_) => panic!("unexpected valid plot"),
        }
    }
}

#[test]
fn numeric_spacing_and_missing_values_do_not_invent_points() {
    let PlotPreparation::Ready(plot) = prepared(
        "#+PLOT: ind:1 deps:(2) with:linespoints\n| 10 | 20 |\n| 15 | NULL |\n| 30 | 40 |\n",
    ) else {
        panic!("valid plot")
    };
    assert!(plot.plan.series[0].points[1].is_none());
    let first = plot.plan.series[0].points[0].unwrap();
    let last = plot.plan.series[0].points[2].unwrap();
    assert!(last.0 > first.0 && last.1 > first.1);
    let snapshot = DocumentSnapshot::from_utf8(
        b"#+PLOT: ind:1 with:points\n| 10 | 20 |\n| 15 | 24 |\n| 30 | 41 |\n".to_vec(),
    )
    .unwrap();
    let candidate = collect_plots(&snapshot, &crate::org_syntax::parse(&snapshot)).remove(0);
    let PlotPreparation::Ready(points) = prepare_plot(&snapshot, &candidate) else {
        panic!("valid points")
    };
    let points = &points.plan.series[0].points;
    let a = points[0].unwrap().0;
    let b = points[1].unwrap().0;
    let c = points[2].unwrap().0;
    assert!(((c - b) / (b - a) - 3.0).abs() < 1e-10);
    assert_eq!(snapshot.revision(), crate::document::Revision(0));
    let PlotPreparation::Ready(large) = prepared(
        "#+PLOT: ind:1 with:points\n| 9007199254740992 | 9007199254740992 |\n| 9007199254740993 | 9007199254740993 |\n",
    ) else {
        panic!("large integers keep approximate coordinates")
    };
    assert_eq!(large.plan.warnings.len(), 2);
    let PlotPreparation::Ready(decimal) = prepared(
        "#+PLOT: ind:1\n| 0 | 1000000000000000000000000000000000000000 |\n| 1 | 1000000000000000000000000000000000000001 |\n",
    ) else {
        panic!("integers beyond the machine integer range")
    };
    assert_eq!(decimal.plan.warnings.len(), 1);
    let PlotPreparation::Ready(offset) =
        prepared("#+PLOT: ind:1\n| 0 | 1000000001 |\n| 1 | 1000000002 |\n")
    else {
        panic!("large offset with a small range")
    };
    assert!(
        offset
            .plan
            .y_ticks
            .windows(2)
            .all(|ticks| ticks[0].label != ticks[1].label)
    );
}

#[test]
fn time_axes_and_transposition_preserve_table_meaning() {
    let ready = |source: &str| match prepared(source) {
        PlotPreparation::Ready(plot) => plot,
        other => panic!("expected a plot: {other:?}"),
    };
    let plot = ready(
        "#+PLOT: ind:1 timeind:t timefmt:\"%Y-%m-%d\"\n| date | value |\n|---+---|\n| 2026-10-01 | 1 |\n| 2026-10-02 | 2 |\n| 2026-10-05 | 3 |\n",
    );
    let points = &plot.plan.series[0].points;
    assert!(
        ((points[2].unwrap().0 - points[1].unwrap().0)
            / (points[1].unwrap().0 - points[0].unwrap().0)
            - 3.0)
            .abs()
            < 1e-9
    );
    assert!(
        plot.plan
            .x_ticks
            .iter()
            .all(|tick| tick.label.starts_with("2026-10-"))
    );
    let auto = ready(
        "#+PLOT: ind:1\n| date | value |\n|---+---|\n| <2026-10-01 Thu> | 1 |\n| NULL | 2 |\n| [2026-10-05 Mon 12:00] | 3 |\n",
    );
    assert!(auto.plan.series[0].points[1].is_none());
    let monthly = ready(
        "#+PLOT: timeind:1 timefmt:\"%Y-%m\"\n| 2026-01 | 1 |\n| 2026-04 | 2 |\n| 2026-12 | 3 |\n",
    );
    assert_eq!(
        monthly
            .plan
            .x_ticks
            .iter()
            .map(|tick| tick.label.as_str())
            .collect::<Vec<_>>(),
        ["2026-01", "2026-04", "2026-07", "2026-10"]
    );
    let offsets = ready(
        "#+PLOT: ind:1 timeind:t timefmt:\"%Y-%m-%d %H:%M %z\"\n| 2026-10-01 08:00 +0800 | 1 |\n| 2026-10-01 00:00 +0000 | 2 |\n",
    );
    assert_eq!(
        offsets.plan.series[0].points[0].unwrap().0,
        offsets.plan.series[0].points[1].unwrap().0
    );
    let transposed = ready(
        "#+PLOT: trans:yes ind:1 deps:(2 3)\n| metric | A | B |\n|---+---+---|\n| Jan | 1 | 2 |\n| Feb | 3 | 4 |\n",
    );
    assert_eq!(
        transposed
            .plan
            .x_ticks
            .iter()
            .map(|tick| tick.label.as_str())
            .collect::<Vec<_>>(),
        ["A", "B"]
    );
    assert_eq!(
        transposed
            .plan
            .series
            .iter()
            .map(|series| series.label.as_str())
            .collect::<Vec<_>>(),
        ["Jan", "Feb"]
    );
    let radar = ready(
        "#+PLOT: transpose:yes type:radar min:0 max:5\n| method | speed | cost | quality |\n|---+---+---+---|\n| A | 2 | 3 | 4 |\n| B | 4 | 3 | 5 |\n",
    );
    assert_eq!(radar.plan.rows, 3);
    assert_eq!(radar.plan.series[0].label, "A");
    assert_eq!(radar.plan.series[0].points[0].unwrap().1, 0.4);
    assert!(
        matches!(prepared("#+PLOT: ind:1 timeind:t timefmt:\"%Y-%m-%d\"\n| 2026-02-30 | 1 |\n"), PlotPreparation::Invalid(error) if error.code == "invalid-time")
    );
}

#[test]
fn explicit_ranges_radar_scales_and_grid_cells_are_planned_independently() {
    let ready = |source: &str| match prepared(source) {
        PlotPreparation::Ready(plot) => plot,
        other => panic!("expected a plot: {other:?}"),
    };
    let bounded = ready(
        "#+PLOT: ind:1 xmin:0 xmax:20 ymin:0 ymax:10 ticks:4\n| 0 | -5 |\n| 10 | 5 |\n| 20 | 15 |\n",
    );
    assert_eq!(bounded.plan.series[0].points[1], Some((0.5, 0.5)));
    assert_eq!(
        bounded.plan.series[0].points[0],
        Some((0.0, -0.5)),
        "clipping must preserve the original coordinates"
    );
    assert!(bounded.plan.clip);
    let radar = ready(
        "#+PLOT: type:radar ind:1 ticks:4\n| axis | A | B |\n|---+---+---|\n| speed | 10 | 20 |\n| cost | 1000 | 2000 |\n| quality | 1 | 2 |\n",
    );
    assert_eq!(radar.plan.layout, PlotLayout::Radar);
    assert_eq!(radar.plan.radar_ticks.len(), 3);
    assert!(
        radar.plan.series[0]
            .points
            .iter()
            .all(|point| point.unwrap().1 == 0.0)
    );
    assert!(
        radar.plan.series[1]
            .points
            .iter()
            .all(|point| point.unwrap().1 == 1.0)
    );
    let grid = ready(
        "#+PLOT: type:grid ind:1\n| row | A | B |\n|---+---+---|\n| first | 0 | NULL |\n| second | 10 | 20 |\n",
    );
    assert_eq!(grid.plan.layout, PlotLayout::Grid);
    assert_eq!(grid.plan.rows, 2);
    assert!(grid.plan.series[1].points[0].is_none());
    assert_eq!(grid.plan.series[0].points[0].unwrap().0, 0.25);
    for source in [
        "#+PLOT: ind:1 ymin:5 ymax:1\n| a | 2 |\n",
        "#+PLOT: ind:1 xmin:0\n| category | 2 |\n",
        "#+PLOT: type:radar ind:1\n| a | 2 |\n| b | 3 |\n",
        "#+PLOT: type:grid ind:1 map:nil\n| a | 2 |\n",
    ] {
        assert!(matches!(prepared(source), PlotPreparation::Invalid(_)));
    }
}

#[test]
fn bundled_cetz_renders_all_basic_marks_offline_with_literal_unicode_labels() {
    let engine = crate::typst_runtime::TypstEngine::default();
    for (name, options) in [
        ("lines", "with:lines"),
        ("points", "with:points"),
        ("linespoints", "with:linespoints"),
        ("histograms", "with:histograms"),
        (
            "stacked",
            "with:histograms set:\"style histogram rowstacked\"",
        ),
        (
            "stacked-dark",
            "with:histograms set:\"style histogram rowstacked\"",
        ),
        (
            "column-stacked",
            "with:histograms set:\"style histogram columnstacked\"",
        ),
        ("frequency", "with:boxes set:\"boxwidth 50\""),
        ("boxplot", "with:boxplot"),
        ("boxplot-dark", "with:boxplot"),
        ("boxplot-small", "with:boxplot"),
        ("steps", "with:steps"),
        ("fsteps", "with:fsteps"),
        ("histeps", "with:histeps"),
        ("filledcurves", "with:filledcurves"),
        ("area", "with:\"filledcurves x1\""),
        ("impulses", "with:impulses"),
        ("boxes", "with:boxes"),
        ("radar", "type:radar min:0 max:5 ticks:5 transpose:yes"),
        ("radar-dark", "type:radar min:0 max:5 ticks:5 transpose:yes"),
        ("grid", "type:grid map:t"),
        ("grid-dark", "type:grid map:t"),
        ("time", "timeind:t timefmt:\"%Y-%m-%d\" with:linespoints"),
        (
            "clipped",
            "with:lines xmin:0 xmax:20 ymin:0 ymax:10 ticks:4",
        ),
    ] {
        let table = if name == "frequency" {
            "| midpoint | count |\n|---+---|\n| 25 | 3 |\n| 75 | 0 |\n| 175 | 2 |\n"
        } else if name == "boxplot-small" {
            "| 少量样本 | 常量样本 | 全部缺失 |\n|---+---+---|\n| 10 | 20 | NULL |\n| 15 | 20 | NULL |\n| NULL | 20 | NULL |\n| NULL | 20 | NULL |\n"
        } else if name.starts_with("boxplot") {
            "| 直营 | 合作渠道 |\n|---+---|\n| 10 | 12 |\n| 12 | 15 |\n| 14 | 18 |\n| 16 | 22 |\n| 18 | 25 |\n| 20 | 30 |\n| 22 | 35 |\n| 100 | NULL |\n"
        } else if name.starts_with("radar") {
            "| method | 性能 | 易用性 | 兼容性 | 维护性 | 扩展性 |\n|---+---+---+---+---+---|\n| A | 4 | 5 | 3 | 4 | 3 |\n| B | 5 | 3 | 4 | 3 | 5 |\n"
        } else if name == "time" {
            "| date | orders |\n|---+---|\n| 2026-10-01 | 120 |\n| 2026-10-02 | 145 |\n| 2026-10-05 | 210 |\n| 2026-10-08 | 185 |\n"
        } else if name == "clipped" {
            "| 0 | -5 |\n| 10 | 5 |\n| 20 | 15 |\n"
        } else {
            "| month | 本期 | 上期 |\n|-------+------+------|\n| 七月 | 120 | 100 |\n| 八月 | 180 | NULL |\n| 九月 | -20 | 130 |\n"
        };
        let ind = if name.starts_with("boxplot") { 0 } else { 1 };
        let source = format!("#+PLOT: title:\"中文与 #literal()\" ind:{ind} {options}\n{table}");
        let PlotPreparation::Ready(plot) = prepared(&source) else {
            panic!("valid plot")
        };
        let style = PlotStyle::for_theme(
            if name.ends_with("-dark") {
                &crate::theme::ORG_STUDIO_DARK
            } else {
                &crate::theme::ORG_STUDIO_LIGHT
            },
            12.0,
        );
        let request = PlotRenderRequest::natural(plot, style);
        let artifact =
            render_plot(&engine, &request).unwrap_or_else(|error| panic!("{name}: {error}"));
        let svg = std::str::from_utf8(&artifact.svg).unwrap();
        assert!(svg.contains("<svg"));
        let svg_document = roxmltree::Document::parse(svg).unwrap();
        let view_box = svg_document
            .root_element()
            .attribute("viewBox")
            .unwrap()
            .split_whitespace()
            .map(|number| number.parse::<f64>().unwrap())
            .collect::<Vec<_>>();
        let expected_ratio = request.key.size.0 as f64 / request.key.size.1 as f64;
        assert!((view_box[2] / view_box[3] - expected_ratio).abs() < 1e-5);
    }
    let PlotPreparation::Ready(plot) = prepared(
        "#+PLOT: ind:1 with:linespoints\n| month | a | b | c | d | e | f | g | h |\n|-------+---+---+---+---+---+---+---+---|\n| first | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |\n| last | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |\n",
    ) else {
        panic!("eight series")
    };
    render_plot(
        &engine,
        &PlotRenderRequest::natural(
            plot,
            PlotStyle::for_theme(&crate::theme::ORG_STUDIO_DARK, 12.0),
        ),
    )
    .unwrap();
}

#[test]
fn plots_are_exported_from_the_frozen_org_table_without_image_files() {
    let directory = tempfile::tempdir().unwrap();
    for (file, link, expected_ratio) in [
        ("", "", 480.0 / 250.0),
        (
            " file:\"images/orders.svg\"",
            "\nAfter the table.\n\n#+ATTR_ORG: :width 320 :height 400\n[[file:images/orders.svg]]\n",
            320.0 / 400.0,
        ),
        (
            " file:\"images/orders.png\"",
            "\n#+ATTR_ORG: :width 320\n[[file:images/orders.png]]\n",
            320.0 / 250.0,
        ),
    ] {
        let source = format!(
            "#+title: Chart export\n#+PLOT: title:\"Orders\" ind:1 with:histograms{file}\n| month | orders |\n|-------+--------|\n| July | 120 |\n| August | 180 |\n{link}"
        );
        let snapshot = DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
        let output = crate::export::export_snapshot(
            crate::typst_runtime::shared_engine(),
            &snapshot,
            crate::export::ExportSourceFormat::Org,
            &directory.path().join("plot.org"),
            &crate::export::ExportOptions {
                format: crate::export::ExportFormat::Svg,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!output.files.is_empty());
        assert!(
            !output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "image-unavailable")
        );
        let svg = std::str::from_utf8(&output.files[0]).unwrap();
        let document = roxmltree::Document::parse(svg).unwrap();
        let images = document
            .descendants()
            .filter(|node| {
                node.is_element()
                    && node.tag_name().name() == "image"
                    && node.attributes().any(|attribute| {
                        attribute.name() == "href" && attribute.value().starts_with("data:image/")
                    })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            images.len(),
            1,
            "a linked chart must replace the image at its source row, without a second attached chart"
        );
        let width = images[0]
            .attribute("width")
            .unwrap()
            .parse::<f64>()
            .unwrap();
        let height = images[0]
            .attribute("height")
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert!(
            (width / height - expected_ratio).abs() < 0.0001,
            "export must preserve the authored canvas proportions"
        );
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            0,
            "export embeds the frozen table and never writes plot files"
        );
    }
}

#[test]
fn plot_outputs_are_explicit_and_display_fit_preserves_requested_width() {
    let options = |text: &str| super::parse::parse_spec(&[text.into()]);
    assert_eq!(options("ind:1").unwrap().output, PlotOutputTarget::Memory);
    assert!(options(r#"file:"""#).is_err());
    assert!(options(r#"file:"chart.jpg""#).is_err());
    let target = options(r#"file:"images/chart.svg""#).unwrap().output;
    assert_eq!(
        output::destination(&target, None).unwrap_err().code,
        "unsaved-output"
    );
    assert_eq!(
        output::destination(&target, Some(std::path::Path::new("plot.org"))).unwrap(),
        Some(std::env::current_dir().unwrap().join("images/chart.svg"))
    );
    assert_eq!(
        output::destination(&target, Some(std::path::Path::new("/tmp/plot/doc.org"))).unwrap(),
        Some("/tmp/plot/images/chart.svg".into())
    );
    let sizing = |width: f32, height: f32| crate::preview::ImageSizing {
        auto_width: width,
        max_width: width,
        max_height: height * 2.0,
        font_size: 20.0,
    };
    let dimensions = PlotLayout::Cartesian.dimensions();
    let spec = Default::default();
    assert_eq!(
        super::layout::display_size(dimensions, &spec, &sizing(1200.0, 900.0)),
        (600.0, 312.5)
    );
    let (width, height) = super::layout::display_size(dimensions, &spec, &sizing(1200.0, 400.0));
    assert_eq!(
        (width, height),
        (600.0, 312.5),
        "a shorter viewport must not shrink the plot"
    );
    let requested = crate::org_syntax::attributes::ImageAttributeSpec::from_width_px(640.0);
    assert_eq!(
        super::layout::display_size(dimensions, &requested, &sizing(300.0, 900.0)).0,
        300.0
    );
    assert_eq!(
        super::layout::display_size(dimensions, &requested, &sizing(1000.0, 900.0)).0,
        640.0
    );
    let em = crate::org_syntax::attributes::parse_attr_org_line("#+ATTR_ORG: :width 10em").unwrap();
    assert_eq!(
        super::layout::display_size(dimensions, &em, &sizing(1200.0, 900.0)).0,
        200.0
    );
    let radar = PlotLayout::Radar.dimensions();
    assert_eq!(
        super::layout::display_size(radar, &spec, &sizing(1200.0, 900.0)),
        (450.0, 400.0)
    );
    let (width, height) = super::layout::display_size(radar, &requested, &sizing(1000.0, 900.0));
    assert_eq!(
        (width, height),
        (640.0, 400.0),
        "chart widths reflow independently of height"
    );
}
