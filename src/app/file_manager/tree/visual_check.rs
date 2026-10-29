//! Opt-in native Metal captures, using production widgets and filesystem data.
use super::{MenuKind, WorkspaceWindow};
use crate::{
    command::FileTreeCommand,
    file_manager::{EntryId, TreeSelection},
    theme::ThemeMode,
};
use gpui::{
    AppContext, AsyncApp, Bounds, Context, WindowBounds, WindowHandle, WindowOptions, point, px,
    size,
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub fn run_visual_check(output: PathBuf) {
    let fixture = output.join("workspace");
    std::fs::create_dir_all(fixture.join("Notes/Archive")).unwrap();
    std::fs::create_dir_all(fixture.join("src")).unwrap();
    std::fs::create_dir_all(fixture.join("Empty")).unwrap();
    for (name, body) in [
        (
            "Notes/planning.org",
            "#+title: Project notes\n\n* Planning\n\n** TODO Review the next release\n\n- [X] Collect reference material\n- [ ] Review the navigation\n\n* Notes\n\nKeep the workspace calm and the document in focus.\n",
        ),
        (
            "Notes/reference.md",
            "# Reference\n\nA collection of ideas.\n",
        ),
        ("src/main.rs", "fn main() {}\n"),
        ("README.md", "# Workspace\n"),
        ("settings.toml", "theme = \"auto\"\n"),
        ("data.csv", "name,value\nitem,1\n"),
        ("paper.pdf", ""),
        ("recording.m4a", ""),
        ("clip.mp4", ""),
        ("backup.zip", ""),
        ("attachment.bin", ""),
        ("photo.png", ""),
        (".hidden", "value"),
    ] {
        std::fs::write(fixture.join(name), body).unwrap();
    }
    for index in 0..50 {
        std::fs::write(fixture.join(format!("z-extra-{index:02}.txt")), "text\n").unwrap();
    }
    for path in ["Notes/recent.org", "src/helper.rs"] {
        match std::fs::remove_file(fixture.join(path)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("reset diagnostic fixture: {error}"),
        }
    }
    gpui_platform::application().run(move |cx| {
        crate::editor::init(cx);
        crate::typography::init(Default::default(), cx.text_system());
        crate::theme::set_theme_mode(ThemeMode::Light);
        let loaded = crate::preview::load_document(fixture.join("Notes/planning.org")).unwrap();
        let root = fixture.clone();
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(30.), px(40.)),
                        size(px(1200.), px(850.)),
                    ))),
                    ..Default::default()
                },
                move |_, cx| {
                    cx.new(|cx| {
                        let mut workspace = WorkspaceWindow::new();
                        workspace.file_manager.tree = super::TreeHost::default();
                        workspace.language = crate::i18n::Language::Chinese;
                        workspace.theme_mode = ThemeMode::Light;
                        workspace.minimap_visible = false;
                        assert!(workspace.apply_load_result(0, Ok(loaded), cx));
                        workspace.file_manager.sidebar_visible = true;
                        workspace.set_file_tree_root(root, cx);
                        workspace.focus_sidebar(cx);
                        workspace
                    })
                },
            )
            .unwrap();
        cx.activate(true);
        cx.spawn(async move |cx| {
            if let Err(error) = scenario(window, &fixture, &output, cx).await {
                eprintln!("file-tree native check failed: {error}");
                std::process::exit(1);
            }
            eprintln!("file-tree native check passed: {}", output.display());
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}

async fn advance(
    window: WindowHandle<WorkspaceWindow>,
    cx: &mut AsyncApp,
    frames: usize,
) -> Result<(), String> {
    for _ in 0..frames {
        cx.update_window(window.into(), |_, window, cx| {
            window.simulate_next_frame(cx)
        })
        .map_err(|error| error.to_string())?;
        cx.background_executor()
            .timer(Duration::from_millis(16))
            .await;
    }
    Ok(())
}

fn capture(
    window: WindowHandle<WorkspaceWindow>,
    output: &Path,
    name: &str,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    window
        .update(cx, |_, window, _| {
            window.render_to_image().map_err(|error| error.to_string())
        })
        .map_err(|error| error.to_string())??
        .save(output.join(name))
        .map_err(|error| error.to_string())
}

fn select(workspace: &mut WorkspaceWindow, path: &Path, cx: &mut Context<WorkspaceWindow>) {
    workspace
        .file_manager
        .tree
        .model
        .select(EntryId::for_path(path), TreeSelection::Single);
    workspace.reveal_file_tree_cursor(cx);
}

async fn scenario(
    window: WindowHandle<WorkspaceWindow>,
    root: &Path,
    output: &Path,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    advance(window, cx, 40).await?;
    window
        .update(cx, |workspace, window, cx| {
            select(workspace, &root.join("Notes"), cx);
            workspace.dispatch_file_tree_command(FileTreeCommand::Expand, window, cx);
            select(workspace, &root.join("src"), cx);
            workspace.dispatch_file_tree_command(FileTreeCommand::Expand, window, cx);
        })
        .map_err(|error| error.to_string())?;
    advance(window, cx, 40).await?;
    std::fs::write(root.join("Notes/recent.org"), "* Recent\n")
        .map_err(|error| error.to_string())?;
    std::fs::write(root.join("src/helper.rs"), "pub fn helper() {}\n")
        .map_err(|error| error.to_string())?;
    advance(window, cx, 80).await?;
    window
        .update(cx, |workspace, _, _| {
            assert!(
                workspace
                    .file_manager
                    .tree
                    .model
                    .entry(EntryId::for_path(&root.join("Notes/recent.org")))
                    .is_some()
            );
            assert!(
                workspace
                    .file_manager
                    .tree
                    .model
                    .entry(EntryId::for_path(&root.join("src/helper.rs")))
                    .is_some()
            );
        })
        .map_err(|error| error.to_string())?;
    window
        .update(cx, |workspace, _, cx| {
            select(workspace, &root.join("Notes"), cx);
            workspace.focus_sidebar(cx);
        })
        .map_err(|error| error.to_string())?;
    advance(window, cx, 10).await?;
    capture(window, output, "folder-selected.png", cx)?;
    window
        .update(cx, |workspace, _, cx| {
            select(workspace, &root.join("Notes/planning.org"), cx);
            workspace.focus_document(cx);
            assert_eq!(workspace.file_manager.tree.model.root(), Some(root));
            assert!(workspace.file_manager.tree.model.rows().len() > 15);
        })
        .map_err(|error| error.to_string())?;
    advance(window, cx, 10).await?;
    capture(window, output, "light.png", cx)?;
    window
        .update(cx, |workspace, window, cx| {
            crate::theme::set_theme_mode(ThemeMode::Dark);
            workspace.theme_mode = ThemeMode::Dark;
            workspace.focus_sidebar(cx);
            select(workspace, &root.join("src/main.rs"), cx);
            window.activate_window();
            cx.notify();
        })
        .map_err(|error| error.to_string())?;
    advance(window, cx, 10).await?;
    capture(window, output, "dark.png", cx)?;
    window
        .update(cx, |workspace, window, cx| {
            workspace.dispatch_file_tree_command(FileTreeCommand::MoreMenu, window, cx)
        })
        .map_err(|error| error.to_string())?;
    advance(window, cx, 10).await?;
    capture(window, output, "menu.png", cx)?;
    capture_menu_variants(window, output, cx).await?;
    window
        .update(cx, |workspace, window, cx| {
            workspace.dispatch_file_tree_command(FileTreeCommand::NewFile, window, cx)
        })
        .map_err(|error| error.to_string())?;
    advance(window, cx, 10).await?;
    capture(window, output, "name.png", cx)?;
    window
        .update(cx, |workspace, window, cx| {
            workspace.dispatch_file_tree_command(FileTreeCommand::ReturnToEditor, window, cx)
        })
        .map_err(|error| error.to_string())?;
    crate::app::file_overview::benchmark::check_sidebar_round_trip(window, output, cx).await
}

async fn capture_menu_variants(
    window: WindowHandle<WorkspaceWindow>,
    output: &Path,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    for (mode, name) in [
        (ThemeMode::Light, "menu-en-light.png"),
        (ThemeMode::Dark, "menu-en-dark.png"),
    ] {
        window
            .update(cx, |workspace, window, cx| {
                workspace.file_manager.tree.menu = None;
                workspace.language = crate::i18n::Language::English;
                workspace.theme_mode = mode;
                crate::theme::set_theme_mode(mode);
                let trigger = workspace.file_manager.tree.toolbar_bounds.get()
                    [MenuKind::More.toolbar_index().unwrap()];
                workspace.open_file_tree_menu(MenuKind::More, Some(trigger.center()), window, cx);
            })
            .map_err(|error| error.to_string())?;
        advance(window, cx, 5).await?;
        capture(window, output, name, cx)?;
    }
    window
        .update(cx, |workspace, window, cx| {
            for key in ["s", "o", "r", "t", "right"] {
                workspace.file_tree_menu_key(
                    &gpui::KeyDownEvent {
                        keystroke: gpui::Keystroke::parse(key).unwrap(),
                        is_held: false,
                        prefer_character_input: false,
                    },
                    window,
                    cx,
                );
            }
        })
        .map_err(|error| error.to_string())?;
    advance(window, cx, 5).await?;
    capture(window, output, "menu-sort.png", cx)?;
    window
        .update(cx, |workspace, window, cx| {
            workspace.file_manager.tree.menu = None;
            window.resize(size(px(320.), px(300.)));
            cx.notify();
        })
        .map_err(|error| error.to_string())?;
    advance(window, cx, 5).await?;
    window
        .update(cx, |workspace, window, cx| {
            let trigger = workspace.file_manager.tree.toolbar_bounds.get()
                [MenuKind::More.toolbar_index().unwrap()];
            workspace.open_file_tree_menu(MenuKind::More, Some(trigger.center()), window, cx);
        })
        .map_err(|error| error.to_string())?;
    advance(window, cx, 5).await?;
    window
        .update(cx, |workspace, window, cx| {
            workspace.file_tree_menu_key(
                &gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("end").unwrap(),
                    is_held: false,
                    prefer_character_input: false,
                },
                window,
                cx,
            );
        })
        .map_err(|error| error.to_string())?;
    advance(window, cx, 5).await?;
    capture(window, output, "menu-compact.png", cx)?;
    window
        .update(cx, |workspace, window, cx| {
            workspace.file_manager.tree.menu = None;
            workspace.language = crate::i18n::Language::Chinese;
            window.resize(size(px(1200.), px(850.)));
            cx.notify();
        })
        .map_err(|error| error.to_string())?;
    advance(window, cx, 5).await
}
