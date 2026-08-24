use std::path::PathBuf;

use gpui::{
    App, Application, Bounds, KeyBinding, Menu, MenuItem, SharedString, SystemMenuType,
    TitlebarOptions, WindowBounds, WindowOptions, actions, prelude::*, px, size,
};
use org_studio::{
    perf_tracing,
    preview::{OpenDocument, PreviewApp, ReloadDocument},
};

actions!(org_studio, [Quit]);

fn main() {
    let perf_trace = perf_tracing::PerfTrace::install();
    let initial_path = std::env::args_os().nth(1).map(PathBuf::from);

    Application::new().run(move |cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-o", OpenDocument, None),
            KeyBinding::new("cmd-r", ReloadDocument, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        cx.set_menus(vec![
            Menu {
                name: "Org Studio".into(),
                items: vec![
                    MenuItem::os_submenu("Services", SystemMenuType::Services),
                    MenuItem::separator(),
                    MenuItem::action("Quit Org Studio", Quit),
                ],
            },
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("Open...", OpenDocument),
                    MenuItem::action("Reload", ReloadDocument),
                ],
            },
        ]);
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
        let bounds = Bounds::centered(requested_display, size(px(920.0), px(720.0)), cx);
        let path = initial_path.clone();

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("Org Studio")),
                    appears_transparent: false,
                    ..Default::default()
                }),
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
