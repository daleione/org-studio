use std::path::PathBuf;

use gpui::{App, Application, Bounds, WindowBounds, WindowOptions, prelude::*, px, size};
use org_studio::{perf_tracing, preview::PreviewApp};

fn main() {
    let perf_trace = perf_tracing::PerfTrace::install();
    let initial_path = std::env::args_os().nth(1).map(PathBuf::from);

    Application::new().run(move |cx: &mut App| {
        let displays = cx.displays();
        for (index, display) in displays.iter().enumerate() {
            eprintln!(
                "org_studio_display index={} id={:?} bounds={:?}",
                index,
                display.id(),
                display.bounds()
            );
        }
        let requested_display = std::env::var("ORG_STUDIO_DISPLAY_INDEX")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .and_then(|index| displays.get(index).map(|display| display.id()));
        let bounds = Bounds::centered(requested_display, size(px(1100.0), px(760.0)), cx);
        let path = initial_path.clone();

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_window, cx| {
                cx.new(|cx| {
                    let mut app = PreviewApp::new();
                    if let Some(path) = path {
                        app.open(path, cx);
                    }
                    app
                })
            },
        )
        .expect("failed to open Org Studio window");

        cx.activate(true);
    });
    if let Some(perf_trace) = perf_trace {
        perf_trace.report();
    }
}
