use super::*;
use std::sync::Arc;

fn hourly_plot() -> Arc<PreparedPlot> {
    let mut source = "#+PLOT: title:\"最近24小时每小时订单量\" ind:1 timeind:t timefmt:\"%Y-%m-%d %H:%M:%S\" with:linespoints\n| 小时 | 订单数量 |\n|---+---|\n".to_owned();
    for hour in 0..=24 {
        let day = 22 + (hour + 2) / 24;
        let time = (hour + 2) % 24;
        source.push_str(&format!(
            "| 2026-10-{day} {time:02}:00:00 | {} |\n",
            (hour * 37 + 4) % 100
        ));
    }
    match prepared(&source) {
        PlotPreparation::Ready(plot) => plot,
        other => panic!("hourly plot: {other:?}"),
    }
}

#[test]
fn time_labels_reflow_without_changing_data_and_render_at_the_requested_aspect() {
    let plot = hourly_plot();
    let mut narrow = plot.plan.clone();
    let mut wide = plot.plan.clone();
    time::reflow(&mut narrow, 480, 12.0).unwrap();
    time::reflow(&mut wide, 960, 12.0).unwrap();
    assert!(wide.x_ticks.len() > narrow.x_ticks.len());
    assert_eq!(wide.x_ticks.first().unwrap().label, "02:00");
    assert_eq!(wide.x_ticks.last().unwrap().label, "02:00");
    assert!(wide.x_ticks.iter().all(|tick| tick.label.len() == 5));
    assert_eq!(
        wide.x_context
            .iter()
            .map(|tick| tick.label.as_str())
            .collect::<Vec<_>>(),
        ["10-22", "10-23"]
    );
    assert_eq!(wide.x_minor_ticks.len(), 25);
    assert_eq!(wide.series[0].points, plot.plan.series[0].points);
    wide.time_axis.as_mut().unwrap().ticks = Some(0);
    time::reflow(&mut wide, 960, 12.0).unwrap();
    assert!(wide.x_ticks.is_empty() && wide.x_context.is_empty() && wide.x_minor_ticks.is_empty());
    let domain = narrow.time_axis.as_mut().unwrap();
    domain.max = domain.min + 0.002;
    time::reflow(&mut narrow, 480, 12.0).unwrap();
    assert!(
        narrow
            .x_ticks
            .windows(2)
            .all(|ticks| ticks[0].label != ticks[1].label),
        "subsecond ticks must keep their precision"
    );

    let engine = crate::typst_runtime::TypstEngine::default();
    let style = PlotStyle::for_theme(&crate::theme::ORG_STUDIO_LIGHT, 12.0);
    for (width, height) in [
        (480.0, 320.0),
        (720.0, 320.0),
        (960.0, 240.0),
        (360.0, 480.0),
    ] {
        let request = PlotRenderRequest::sized(plot.clone(), style.clone(), width, height);
        let artifact = render_plot(&engine, &request).unwrap();
        let svg = std::str::from_utf8(&artifact.svg).unwrap();
        let svg = roxmltree::Document::parse(svg).unwrap();
        let viewbox = svg
            .root_element()
            .attribute("viewBox")
            .unwrap()
            .split_whitespace()
            .map(|value| value.parse::<f32>().unwrap())
            .collect::<Vec<_>>();
        assert!((viewbox[2] / viewbox[3] - width / height).abs() < 0.0001);
    }
}

#[test]
fn dragging_one_axis_preserves_the_other_dimension() {
    assert_eq!(
        PlotResizeAxis::Width.resize((180.0, 250.0), (1.0, 0.0), 800.0),
        (181.0, 250.0)
    );
    assert_eq!(
        PlotResizeAxis::Height.resize((400.0, 1400.0), (0.0, -50.0), 800.0),
        (400.0, 1350.0)
    );
    assert_eq!(
        PlotResizeAxis::Height.resize((180.0, 250.0), (80.0, 40.0), 800.0),
        (180.0, 290.0)
    );
    assert_eq!(
        PlotResizeAxis::Width.resize((400.0, 1400.0), (80.0, 40.0), 800.0),
        (480.0, 1400.0)
    );
    assert_eq!(
        PlotResizeAxis::Both.resize((400.0, 300.0), (1000.0, -1000.0), 800.0),
        (800.0, 180.0)
    );
}

#[test]
fn display_and_render_limits_match_without_resizing_to_viewport_height() {
    let sizing = crate::preview::ImageSizing {
        auto_width: 10000.0,
        max_width: 10000.0,
        max_height: 200.0,
        font_size: 32.0,
    };
    let attributes = crate::org_syntax::attributes::parse_attr_org_line(
        "#+ATTR_ORG: :width 10000 :height 10000",
    )
    .unwrap();
    let plot = hourly_plot();
    let size = super::super::layout::display_size(plot.natural_size, &attributes, &sizing);
    let request = PlotRenderRequest::for_display(
        plot,
        PlotStyle::for_theme(&crate::theme::ORG_STUDIO_LIGHT, 12.0),
        size,
        sizing.font_size,
    );
    assert_eq!(size, (4096.0, 3200.0));
    assert_eq!(
        size,
        (
            request.key.size.0 as f32 * 2.0,
            request.key.size.1 as f32 * 2.0
        ),
        "the renderer must not silently stretch a smaller canvas"
    );
    assert_eq!(
        super::super::layout::display_size(
            (480, 250),
            &attributes,
            &crate::preview::ImageSizing {
                max_height: 80.0,
                ..sizing
            }
        ),
        size
    );
}
