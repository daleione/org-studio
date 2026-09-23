use std::{cell::RefCell, path::PathBuf, rc::Rc};

use gpui::{
    App, Bounds, KeyBinding, SharedString, TitlebarOptions, WindowAppearance, WindowBounds,
    WindowHandle, WindowOptions, prelude::*, px, size,
};
use org_studio::{
    app::WorkspaceWindow,
    perf_tracing,
    preview::{
        DOCUMENT_WORKSPACE_KEY_CONTEXT, DecreaseContentFontSize, ExportDocument,
        IncreaseContentFontSize, InitialDocumentLoad, OpenDocument, QuitApplication,
        ReloadDocument, ResetContentFontSize, SaveDocument, SaveDocumentAs, ShowEditor,
        ShowReading, ShowSplit, ToggleMinimap, ToggleSidebar, ToggleSoftWrap,
        preload_initial_document,
    },
    window_state,
};

const DISABLE_INACTIVE_THROTTLE_ENV: &str = "ORG_STUDIO_BENCH_DISABLE_INACTIVE_THROTTLE";

fn benchmark_disables_inactive_throttle() -> bool {
    std::env::var(DISABLE_INACTIVE_THROTTLE_ENV)
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "on"))
}

#[derive(Default)]
struct ApplicationController {
    main_window: Option<WindowHandle<WorkspaceWindow>>,
}

impl ApplicationController {
    fn reopen(&mut self, cx: &mut App) {
        self.open_or_activate(None, None, cx);
    }

    fn open_or_activate(
        &mut self,
        path: Option<PathBuf>,
        initial_load: Option<InitialDocumentLoad>,
        cx: &mut App,
    ) {
        if let Some(handle) = self.main_window {
            let update = handle.update(cx, |preview, window, cx| {
                if let Some(path) = path.clone()
                    && preview.current_open_path(cx) != Some(path.as_path())
                {
                    preview.open(path, cx);
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
                    let mut preview = WorkspaceWindow::new();
                    if let Some(load) = initial_load {
                        preview.open_initial(load, cx);
                    } else if let Some(path) = path {
                        preview.open(path, cx);
                    }
                    // Track the window's placement so the next launch reopens
                    // on the same display instead of always re-centering on
                    // the primary (menu-bar) display.
                    cx.observe_window_bounds(window, |_, window, cx| {
                        window_state::remember(
                            window.bounds(),
                            window.display(cx).map(|display| display.id()),
                        );
                    })
                    .detach();
                    // Follow system appearance changes so Auto mode repaints
                    // with the matching palette without a restart. Events that
                    // arrive while an explicit mode pins the native appearance
                    // are ignored by the theme layer (they reflect our pin).
                    window
                        .observe_window_appearance(|window, cx| {
                            org_studio::theme::note_system_appearance(is_dark_appearance(
                                window.appearance(),
                            ));
                            cx.refresh_windows();
                        })
                        .detach();
                    preview
                });
                window.activate_window();
                preview
            })
            .expect("failed to open Org Studio window");
        self.main_window = Some(handle);
        cx.activate(true);
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

    // Benchmark/determinism overrides win outright and never reuse persisted
    // placement state.
    let benchmark_overrides = std::env::var_os("ORG_STUDIO_DISPLAY_INDEX").is_some()
        || std::env::var_os("ORG_STUDIO_WINDOW_WIDTH").is_some()
        || std::env::var_os("ORG_STUDIO_WINDOW_HEIGHT").is_some();
    let (bounds, display_id) = if benchmark_overrides {
        (
            Bounds::centered(
                requested_display,
                size(px(window_width), px(window_height)),
                cx,
            ),
            requested_display,
        )
    } else {
        window_state::restored_placement(cx, size(px(window_width), px(window_height)))
    };

    let mut options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        // Centered bounds alone are ambiguous when multiple macOS displays share
        // the same logical origin. Preserve the selected display through native
        // window creation so benchmarks run on the requested panel and restored
        // windows reopen on the display they last lived on.
        display_id,
        titlebar: Some(TitlebarOptions {
            title: Some(SharedString::from("Org Studio")),
            appears_transparent: true,
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

fn is_dark_appearance(appearance: WindowAppearance) -> bool {
    matches!(
        appearance,
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    )
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
        // Restore the persisted theme mode (Auto/Light/Dark). Auto hands the
        // native chrome back to the system; explicit modes pin the matching
        // appearance so the traffic-light row stays in sync with the palette.
        let theme_mode = org_studio::settings::WorkspaceSettings::load().theme_mode;
        org_studio::theme::set_theme_mode(theme_mode);
        cx.set_window_appearance(org_studio::app::native_window_appearance(theme_mode));
        // Give command-line documents a head start while menus, displays and the native window are
        // initialized. `open_initial` consumes an already-ready small document synchronously and
        // continues awaiting a large one without blocking the window.
        let launch_path = open_receiver
            .try_recv()
            .ok()
            .into_iter()
            .flat_map(paths_from_urls)
            .last();
        let initial_path = initial_path.clone().or(launch_path);
        let initial_load = initial_path
            .clone()
            .filter(|path| !org_studio::preview::is_supported_image(path))
            .map(|path| preload_initial_document(path, cx));
        org_studio::editor::init(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-p", org_studio::app::SwitchBuffer, None),
            KeyBinding::new("cmd-[", org_studio::app::NavigateBack, None),
            KeyBinding::new("cmd-]", org_studio::app::NavigateForward, None),
            KeyBinding::new("cmd-o", OpenDocument, None),
            KeyBinding::new("cmd-s", SaveDocument, None),
            KeyBinding::new("cmd-shift-s", SaveDocumentAs, None),
            KeyBinding::new("cmd-r", ReloadDocument, None),
            KeyBinding::new("cmd-shift-e", ExportDocument, None),
            KeyBinding::new("cmd-q", QuitApplication, None),
            KeyBinding::new("cmd-1", ShowEditor, None),
            KeyBinding::new("cmd-2", ShowReading, None),
            KeyBinding::new("cmd-3", ShowSplit, None),
            KeyBinding::new("alt-z", ToggleSoftWrap, None),
            KeyBinding::new("cmd-b", ToggleSidebar, None),
            KeyBinding::new(
                "cmd-alt-m",
                ToggleMinimap,
                Some(DOCUMENT_WORKSPACE_KEY_CONTEXT),
            ),
            KeyBinding::new(
                "cmd-=",
                IncreaseContentFontSize,
                Some(DOCUMENT_WORKSPACE_KEY_CONTEXT),
            ),
            KeyBinding::new(
                "cmd-+",
                IncreaseContentFontSize,
                Some(DOCUMENT_WORKSPACE_KEY_CONTEXT),
            ),
            KeyBinding::new(
                "cmd--",
                DecreaseContentFontSize,
                Some(DOCUMENT_WORKSPACE_KEY_CONTEXT),
            ),
            KeyBinding::new(
                "cmd-0",
                ResetContentFontSize,
                Some(DOCUMENT_WORKSPACE_KEY_CONTEXT),
            ),
        ]);
        cx.set_menus(org_studio::app::application_menus(
            org_studio::settings::WorkspaceSettings::load().language,
        ));
        cx.intercept_keystrokes(WorkspaceWindow::intercept_fullscreen_escape)
            .detach();
        cx.intercept_keystrokes(WorkspaceWindow::intercept_quick_open_shortcuts)
            .detach();
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
            // Persist the final placement synchronously so the next launch
            // (or reopen after this window closed) lands on the same display.
            window_state::flush();
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let open_controller = controller.clone();
        cx.spawn(async move |cx| {
            while let Ok(urls) = open_receiver.recv().await {
                for path in paths_from_urls(urls) {
                    cx.update(|cx| {
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
