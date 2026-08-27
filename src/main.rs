use std::{cell::RefCell, path::PathBuf, rc::Rc};

use gpui::{
    App, Bounds, KeyBinding, Menu, MenuItem, SharedString, SystemMenuType, TitlebarOptions,
    WindowBounds, WindowHandle, WindowOptions, actions, prelude::*, px, size,
};
use org_studio::{
    perf_tracing,
    preview::{
        InitialDocumentLoad, OpenDocument, PreviewApp, ReloadDocument, ToggleMinimap,
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

#[derive(Default)]
struct ApplicationController {
    main_window: Option<WindowHandle<PreviewApp>>,
}

impl ApplicationController {
    fn reopen(&mut self, cx: &mut App) {
        let path = self
            .active_preview(cx)
            .is_none()
            .then(org_studio::settings::last_document_path)
            .flatten();
        self.open_or_activate(path, None, cx);
    }

    fn open_or_activate(
        &mut self,
        path: Option<PathBuf>,
        initial_load: Option<InitialDocumentLoad>,
        cx: &mut App,
    ) {
        if let Some(handle) = self.main_window {
            let update = handle.update(cx, |preview, window, cx| {
                if let Some(path) = path.clone() {
                    if preview.current_document_path() != Some(path.as_path()) {
                        preview.open(path, cx);
                    }
                }
                window.activate_window();
            });
            if update.is_ok() {
                cx.activate(true);
                return;
            }
            self.main_window = None;
        }

        let options = preview_window_options(cx);
        let handle = cx
            .open_window(options, move |window, cx| {
                let preview = cx.new(|cx| {
                    let mut preview = PreviewApp::new();
                    if let Some(load) = initial_load {
                        preview.open_initial(load, cx);
                    } else if let Some(path) = path {
                        preview.open(path, cx);
                    }
                    preview
                });
                window.activate_window();
                preview
            })
            .expect("failed to open Org Studio window");
        self.main_window = Some(handle);
        cx.activate(true);
    }

    fn active_preview(&mut self, cx: &mut App) -> Option<WindowHandle<PreviewApp>> {
        let handle = self.main_window?;
        if handle.read(cx).is_ok() {
            Some(handle)
        } else {
            self.main_window = None;
            None
        }
    }
}

fn preview_window_options(cx: &mut App) -> WindowOptions {
    let displays = cx.displays();
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
    let mut options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(SharedString::from("Org Studio")),
            appears_transparent: false,
            ..Default::default()
        }),
        ..Default::default()
    };
    if benchmark_disables_inactive_throttle() {
        options.inactive_frame_interval = None;
        eprintln!(
            "org_studio_benchmark_frame_policy inactive_window_throttle=disabled env={DISABLE_INACTIVE_THROTTLE_ENV}"
        );
    }
    options
}

fn paths_from_urls(urls: Vec<String>) -> Vec<PathBuf> {
    urls.into_iter()
        .filter_map(|url| url::Url::parse(&url).ok())
        .filter_map(|url| url.to_file_path().ok())
        .collect()
}

fn main() {
    let perf_trace = perf_tracing::PerfTrace::install();
    let initial_path = std::env::args_os().nth(1).map(PathBuf::from);
    let (open_sender, open_receiver) = async_channel::unbounded::<Vec<String>>();
    let application = gpui_platform::application();
    let controller = Rc::new(RefCell::new(ApplicationController::default()));
    application.on_open_urls(move |urls| {
        let _ = open_sender.try_send(urls);
    });
    let reopen_controller = controller.clone();
    application.on_reopen(move |cx| {
        reopen_controller.borrow_mut().reopen(cx);
    });

    application.run(move |cx: &mut App| {
        // Give command-line documents a head start while menus, displays and the native window are
        // initialized. `open_initial` consumes an already-ready small document synchronously and
        // continues awaiting a large one without blocking the window.
        let launch_path = open_receiver
            .try_recv()
            .ok()
            .into_iter()
            .flat_map(paths_from_urls)
            .last();
        let initial_path = initial_path
            .clone()
            .or(launch_path)
            .or_else(org_studio::settings::last_document_path);
        let initial_load = initial_path
            .clone()
            .map(|path| preload_initial_document(path, cx));
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-o", OpenDocument, None),
            KeyBinding::new("cmd-r", ReloadDocument, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        cx.set_menus(app_menus());
        let displays = cx.displays();
        for (index, display) in displays.iter().enumerate() {
            eprintln!(
                "org_studio_display index={} id={:?} bounds={:?}",
                index,
                display.id(),
                display.bounds()
            );
        }
        controller
            .borrow_mut()
            .open_or_activate(initial_path, initial_load, cx);

        let closed_controller = controller.clone();
        cx.on_window_closed(move |cx, window_id| {
            let mut controller = closed_controller.borrow_mut();
            if controller
                .main_window
                .is_some_and(|handle| handle.window_id() == window_id)
            {
                controller.main_window = None;
            }
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let open_controller = controller.clone();
        cx.spawn(async move |cx| {
            while let Ok(urls) = open_receiver.recv().await {
                for path in paths_from_urls(urls) {
                    let _ = cx.update(|cx| {
                        open_controller
                            .borrow_mut()
                            .open_or_activate(Some(path), None, cx);
                    });
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

fn app_menus() -> Vec<Menu> {
    vec![
        Menu::new("Org Studio").items([
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Quit Org Studio", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("Open...", OpenDocument),
            MenuItem::separator(),
            MenuItem::action("Reload", ReloadDocument),
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Toggle Minimap", ToggleMinimap),
        ]),
    ]
}
