use std::{cell::RefCell, path::PathBuf, rc::Rc};

use gpui::{
    App, Bounds, KeyBinding, Menu, MenuItem, SharedString, SystemMenuType, TitlebarOptions,
    WindowBounds, WindowOptions, actions, prelude::*, px, size,
};
use org_studio::{
    perf_tracing,
    preview::{
        OpenDocument, OpenFileManager, PreviewApp, ReloadDocument, ReturnToDocument, ToggleMinimap,
        ToggleSidebar, preload_initial_document,
    },
};

actions!(org_studio, [Quit]);

const DISABLE_INACTIVE_THROTTLE_ENV: &str = "ORG_STUDIO_BENCH_DISABLE_INACTIVE_THROTTLE";

fn benchmark_disables_inactive_throttle() -> bool {
    std::env::var(DISABLE_INACTIVE_THROTTLE_ENV)
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "on"))
}

fn main() {
    let perf_trace = perf_tracing::PerfTrace::install();
    let initial_path = std::env::args_os().nth(1).map(PathBuf::from);
    let (open_sender, open_receiver) = async_channel::unbounded::<Vec<String>>();
    let application = gpui_platform::application();
    application.on_open_urls(move |urls| {
        let _ = open_sender.try_send(urls);
    });

    application.run(move |cx: &mut App| {
        // Give command-line documents a head start while menus, displays and the native window are
        // initialized. `open_initial` consumes an already-ready small document synchronously and
        // continues awaiting a large one without blocking the window.
        let initial_load = initial_path
            .clone()
            .map(|path| preload_initial_document(path, cx));
        let active_preview: Rc<RefCell<Option<gpui::Entity<PreviewApp>>>> =
            Rc::new(RefCell::new(None));
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-o", OpenDocument, None),
            KeyBinding::new("cmd-r", ReloadDocument, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        cx.set_menus(app_menus(org_studio::settings::initial_minimap_enabled()));
        let preview_for_minimap = active_preview.clone();
        cx.on_action(move |_: &ToggleMinimap, cx| {
            let Some(preview) = preview_for_minimap.borrow().clone() else {
                return;
            };
            let visible = preview.update(cx, |preview, cx| {
                preview.toggle_minimap(cx);
                preview.minimap_visible()
            });
            cx.set_menus(app_menus(visible));
        });
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
        let window_width = std::env::var("ORG_STUDIO_WINDOW_WIDTH")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite() && *value >= 320.0)
            .unwrap_or(920.0);
        let window_height = std::env::var("ORG_STUDIO_WINDOW_HEIGHT")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite() && *value >= 240.0)
            .unwrap_or(720.0);
        let bounds = Bounds::centered(
            requested_display,
            size(px(window_width), px(window_height)),
            cx,
        );
        let preview_for_window = active_preview.clone();

        let disable_inactive_throttle = benchmark_disables_inactive_throttle();
        let mut window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(SharedString::from("Org Studio")),
                appears_transparent: false,
                ..Default::default()
            }),
            ..Default::default()
        };
        if disable_inactive_throttle {
            // Synthetic on_next_frame benchmarks do not generate native input and macOS does not
            // guarantee that an unattended process becomes the active application. Disable only
            // GPUI's inactive-window energy throttle so the sample remains display-link paced.
            window_options.inactive_frame_interval = None;
            eprintln!(
                "org_studio_benchmark_frame_policy inactive_window_throttle=disabled env={DISABLE_INACTIVE_THROTTLE_ENV}"
            );
        }

        cx.open_window(
            window_options,
            move |_window, cx| {
                let preview = cx.new(|cx| {
                    let mut app = PreviewApp::new();
                    if let Some(load) = initial_load {
                        app.open_initial(load, cx);
                    }
                    app
                });
                *preview_for_window.borrow_mut() = Some(preview.clone());
                preview
            },
        )
        .expect("failed to open Org Studio window");

        cx.spawn(async move |cx| {
            while let Ok(urls) = open_receiver.recv().await {
                for url in urls {
                    let Ok(url) = url::Url::parse(&url) else {
                        continue;
                    };
                    let Ok(path) = url.to_file_path() else {
                        continue;
                    };
                    let preview = active_preview.borrow().clone();
                    if let Some(preview) = preview {
                        let _ = preview.update(cx, |app, cx| app.open(path, cx));
                    }
                }
            }
        })
        .detach();

        cx.activate(true);
    });
    if let Some(perf_trace) = perf_trace {
        perf_trace.report();
    }
}

fn app_menus(minimap_enabled: bool) -> Vec<Menu> {
    vec![
        Menu::new("Org Studio").items([
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Quit Org Studio", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("Open...", OpenDocument),
            MenuItem::action("Open File Manager...", OpenFileManager),
            MenuItem::action("Return to Document", ReturnToDocument),
            MenuItem::separator(),
            MenuItem::action("Reload", ReloadDocument),
        ]),
        Menu::new("View").items([
            MenuItem::action("Show/Hide Sidebar", ToggleSidebar),
            MenuItem::action(
                if minimap_enabled {
                    "✓ Minimap"
                } else {
                    "Minimap"
                },
                ToggleMinimap,
            ),
        ]),
    ]
}
