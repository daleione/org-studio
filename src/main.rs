use std::{cell::RefCell, path::PathBuf, rc::Rc};

use gpui::{
    App, Application, Bounds, KeyBinding, Menu, MenuItem, SharedString, SystemMenuType,
    TitlebarOptions, WindowBounds, WindowOptions, actions, prelude::*, px, size,
};
use org_studio::{
    perf_tracing,
    preview::{
        OpenDocument, OpenFileManager, PreviewApp, ReloadDocument, ReturnToDocument, ToggleMinimap,
        ToggleSidebar,
    },
};

actions!(org_studio, [Quit]);

fn main() {
    let perf_trace = perf_tracing::PerfTrace::install();
    let initial_path = std::env::args_os().nth(1).map(PathBuf::from);
    let (open_sender, open_receiver) = async_channel::unbounded::<Vec<String>>();
    let application = Application::new();
    application.on_open_urls(move |urls| {
        let _ = open_sender.try_send(urls);
    });

    application.run(move |cx: &mut App| {
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
        let path = initial_path.clone();
        let preview_for_window = active_preview.clone();

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
                let preview = cx.new(|cx| {
                    let mut app = PreviewApp::new();
                    if let Some(path) = path {
                        app.open(path, cx);
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
                MenuItem::action("Open File Manager...", OpenFileManager),
                MenuItem::action("Return to Document", ReturnToDocument),
                MenuItem::separator(),
                MenuItem::action("Reload", ReloadDocument),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Show/Hide Sidebar", ToggleSidebar),
                MenuItem::action(
                    if minimap_enabled {
                        "✓ Minimap"
                    } else {
                        "Minimap"
                    },
                    ToggleMinimap,
                ),
            ],
        },
    ]
}
