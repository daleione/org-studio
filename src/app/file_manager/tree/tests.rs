use super::*;
use crate::{
    document::{ByteRange, DocumentCommand, EditOrigin, EditTransaction, Selection, TextEdit},
    file_manager::{EntryId, OperationPlan, TreeRow},
    typography::ui_px,
};
use gpui::{
    Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent, TouchPhase, VisualTestContext, point,
};

const ROW_TARGETS: [&str; 4] = [
    "file-tree-current-name",
    "file-tree-current-icon",
    "file-tree-current-disclosure",
    "file-tree-current-row",
];

pub(super) fn fixture(
    cx: &mut gpui::TestAppContext,
) -> (
    tempfile::TempDir,
    Entity<WorkspaceWindow>,
    &mut VisualTestContext,
) {
    // These integration tests exercise real filesystem notifications in
    // addition to GPUI's deterministic input/clock simulation.
    cx.executor().allow_parking();
    cx.update(crate::editor::init);
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("chapter")).unwrap();
    std::fs::write(directory.path().join("chapter/entry.org"), "* Entry\n").unwrap();
    std::fs::write(directory.path().join("notes.org"), "* Notes\n").unwrap();
    let loaded = crate::preview::load_document(directory.path().join("notes.org")).unwrap();
    let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(0, Ok(loaded), cx));
        workspace.toggle_sidebar(cx);
        workspace.file_manager.sidebar_visibility_animation = None;
        workspace.focus_sidebar(cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(
            workspace
                .read(cx)
                .file_manager
                .tree
                .input
                .as_ref()
                .unwrap()
                .read(cx)
                .focus
                .is_focused(window),
            "file tree must own native input focus"
        );
    });
    (directory, workspace, cx)
}

fn select(workspace: &Entity<WorkspaceWindow>, path: &Path, cx: &mut VisualTestContext) {
    workspace.update(cx, |workspace, cx| {
        workspace
            .file_manager
            .tree
            .model
            .select(EntryId::for_path(path), TreeSelection::Single);
        workspace.reveal_file_tree_cursor(cx);
    });
    cx.run_until_parked();
}

fn click_row_target(
    cx: &mut VisualTestContext,
    selector: &'static str,
    click_count: usize,
    modifiers: Modifiers,
) {
    let bounds = cx.debug_bounds(selector).expect(selector);
    // Hit the start of the actual name, icon or disclosure, rather than
    // the empty space after a short name. The row target hits its padding.
    let position = point(bounds.left() + ui_px(2.), bounds.center().y);
    cx.simulate_event(gpui::MouseDownEvent {
        position,
        button: MouseButton::Left,
        modifiers,
        click_count,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position,
        button: MouseButton::Left,
        modifiers,
        click_count,
    });
    cx.run_until_parked();
}

fn trackpad(
    cx: &mut VisualTestContext,
    position: gpui::Point<gpui::Pixels>,
    x: f32,
    y: f32,
    phase: TouchPhase,
) {
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(x), px(y))),
        touch_phase: phase,
        ..Default::default()
    });
    cx.run_until_parked();
}

#[gpui::test]
fn name_hints_follow_truncation_when_the_sidebar_is_resized(cx: &mut gpui::TestAppContext) {
    let (directory, workspace, cx) = fixture(cx);
    let long_name = "a-somewhat-long-document-name.org";
    std::fs::write(directory.path().join(long_name), "* Heading\n").unwrap();
    workspace.update(cx, |workspace, cx| {
        workspace.request_file_directory(directory.path().to_path_buf(), true, cx)
    });
    cx.run_until_parked();
    for (name, width, truncated) in [
        ("notes.org", 200, false),
        (long_name, 200, true),
        (long_name, 420, false),
    ] {
        cx.simulate_mouse_move(point(px(700.), px(300.)), None, Modifiers::default());
        workspace.update(cx, |workspace, cx| {
            workspace.file_manager.sidebar_width = width;
            cx.notify();
        });
        select(&workspace, &directory.path().join(name), cx);
        let label = cx.debug_bounds("file-tree-current-name").unwrap();
        cx.simulate_mouse_move(
            point(label.left() + ui_px(2.), label.center().y),
            None,
            Modifiers::default(),
        );
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(501));
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds("file-tree-hint").is_some(),
            truncated,
            "{name}, width {width}"
        );
    }
}

#[gpui::test]
fn leaving_dired_for_home_cancels_pending_navigation(cx: &mut gpui::TestAppContext) {
    let (directory, workspace, cx) = fixture(cx);
    let child = directory.path().join("chapter");
    workspace.update(cx, |workspace, cx| {
        workspace.open_file_manager(child.clone(), cx);
        assert!(workspace.file_manager.dired_load.is_some());
        workspace.reload_file_manager(cx);
        workspace.show_home_now(cx);
    });
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.file_manager.dired_load.is_none());
        assert!(!workspace.file_manager.refresh_pending);
    });
    workspace.update(cx, |workspace, cx| {
        workspace.open_file_manager(child.clone(), cx)
    });
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        let session = workspace.file_manager.session.as_ref().unwrap();
        assert_eq!(session.directory(), child);
        assert!(
            session
                .entries()
                .iter()
                .any(|entry| entry.display_name.as_ref() == "entry.org")
        );
        assert!(workspace.file_manager.dired_load.is_none());
    });
}

#[gpui::test]
fn trackpad_swipes_toggle_the_sidebar_from_the_document_and_tree(cx: &mut gpui::TestAppContext) {
    let (directory, workspace, cx) = fixture(cx);
    cx.update(|_, cx| cx.set_reduce_motion(true));
    cx.simulate_resize(gpui::size(px(1000.), px(650.)));
    let document = workspace.read_with(cx, |workspace, _| {
        workspace.document_session().unwrap().clone()
    });
    let position = point(px(700.), px(180.));
    workspace.update(cx, |workspace, cx| {
        workspace.minimap_visible = false;
        workspace.show_editor(cx);
        workspace.focus_document(cx);
    });
    cx.run_until_parked();
    trackpad(cx, position, -10., 1., TouchPhase::Started);
    trackpad(cx, position, -40., 1., TouchPhase::Moved);
    trackpad(cx, position, 0., 0., TouchPhase::Ended);
    assert!(!workspace.read_with(cx, |workspace, _| workspace.sidebar_visible()));

    trackpad(cx, position, 10., 1., TouchPhase::Started);
    trackpad(cx, position, 40., 1., TouchPhase::Moved);
    trackpad(cx, position, 0., 0., TouchPhase::Ended);
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.sidebar_visible());
        assert_eq!(workspace.document_session().unwrap(), &document);
        assert_eq!(
            workspace.file_manager.tree.model.root(),
            Some(directory.path())
        );
    });
    let row = cx.debug_bounds("file-tree-current-row").unwrap();
    trackpad(cx, row.center(), -50., 0., TouchPhase::Started);
    trackpad(cx, position, 0., 0., TouchPhase::Ended);
    assert!(!workspace.read_with(cx, |workspace, _| workspace.sidebar_visible()));
}

#[gpui::test]
fn horizontal_document_scrolling_takes_priority_over_sidebar_swipes(cx: &mut gpui::TestAppContext) {
    let (_, workspace, cx) = fixture(cx);
    cx.update(|_, cx| cx.set_reduce_motion(true));
    cx.simulate_resize(gpui::size(px(1000.), px(650.)));
    let document = workspace.read_with(cx, |workspace, _| {
        workspace.document_session().unwrap().clone()
    });
    document.update(cx, |document, cx| {
        document
            .edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        document.revision(),
                        vec![TextEdit::new(
                            ByteRange::new(0, 0),
                            format!("{}\n", "sample ".repeat(200)),
                        )],
                    ),
                    Selection::default(),
                    Selection::default(),
                    EditOrigin::Other,
                ),
                cx,
            )
            .unwrap();
    });
    workspace.update(cx, |workspace, cx| {
        workspace.minimap_visible = false;
        workspace.show_editor(cx);
        workspace.toggle_soft_wrap(cx);
        workspace.focus_document(cx);
    });
    cx.run_until_parked();
    let position = point(px(700.), px(100.));
    trackpad(cx, position, -10., 0., TouchPhase::Started);
    trackpad(cx, position, -60., 0., TouchPhase::Moved);
    trackpad(cx, position, 0., 0., TouchPhase::Ended);
    workspace.read_with(cx, |workspace, cx| {
        assert!(workspace.sidebar_visible());
        assert!(
            workspace
                .editor(crate::app::PaneSide::Left)
                .unwrap()
                .read(cx)
                .scroll_x()
                > 0.
        );
    });
}

#[gpui::test]
fn folder_root_navigation_supports_mouse_and_keyboard_and_preserves_the_document(
    cx: &mut gpui::TestAppContext,
) {
    let (directory, workspace, cx) = fixture(cx);
    let child = directory.path().join("chapter");
    let document = workspace.read_with(cx, |workspace, _| {
        workspace.document_session().unwrap().clone()
    });
    for activation in ["click", "shortcut", "arrows", "tabs"] {
        workspace.update(cx, |workspace, cx| {
            workspace.set_file_tree_root(directory.path().to_path_buf(), cx);
            workspace.file_manager.tree.model.collapse_all();
            workspace.rebuild_file_tree(cx);
            workspace.focus_sidebar(cx);
        });
        cx.run_until_parked();
        select(&workspace, &child, cx);
        click_row_target(cx, "file-tree-current-name", 1, Modifiers::default());
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(
                workspace.file_manager.tree.model.root(),
                Some(directory.path())
            );
            assert!(workspace.file_manager.tree.model.expanded(&child));
        });
        if activation == "shortcut" {
            // Cmd-toggle can leave the cursor on a deselected file. Entering
            // a folder must use the selection rather than the cursor.
            workspace.update(cx, |workspace, cx| {
                let tree = &mut workspace.file_manager.tree.model;
                let file = EntryId::for_path(&directory.path().join("notes.org"));
                tree.select(file, TreeSelection::Toggle);
                tree.select(file, TreeSelection::Toggle);
                cx.notify();
            });
            cx.run_until_parked();
        }
        let enter = cx.debug_bounds("file-tree-enter").unwrap();
        match activation {
            "click" => cx.simulate_click(enter.center(), Modifiers::default()),
            "shortcut" => cx.simulate_keystrokes("cmd-down"),
            "arrows" => cx.simulate_keystrokes("shift-tab left left enter"),
            _ => cx.simulate_keystrokes("shift-tab shift-tab shift-tab space"),
        }
        cx.run_until_parked();
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(
                workspace.file_manager.tree.model.root(),
                Some(child.as_path()),
                "{activation}"
            );
            assert_eq!(workspace.document_session().unwrap(), &document);
            assert!(workspace.file_manager.sidebar_focused);
        });
        let parent = cx.debug_bounds("file-tree-parent").unwrap();
        match activation {
            "click" => cx.simulate_click(parent.center(), Modifiers::default()),
            "shortcut" => cx.simulate_keystrokes("cmd-up"),
            "arrows" => cx.simulate_keystrokes("shift-tab left left enter"),
            _ => cx.simulate_keystrokes("shift-tab shift-tab shift-tab space"),
        }
        cx.run_until_parked();
        workspace.read_with(cx, |workspace, _| {
            let tree = &workspace.file_manager.tree;
            assert_eq!(tree.model.root(), Some(directory.path()), "{activation}");
            assert_eq!(tree.model.single_selected().unwrap().path.as_ref(), child);
            assert!(tree.model.expanded(&child));
            assert_eq!(workspace.document_session().unwrap(), &document);
            assert!(workspace.file_manager.sidebar_focused);
        });
    }
    cx.simulate_keystrokes("cmd-down");
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace
            .file_manager
            .tree
            .model
            .root()
            .map(Path::to_path_buf)),
        Some(child)
    );
    workspace.update(cx, |workspace, cx| {
        workspace.set_file_tree_root(directory.path().to_path_buf(), cx);
        workspace.focus_document(cx);
    });
    cx.run_until_parked();
    for shortcut in ["cmd-up", "cmd-down"] {
        cx.simulate_keystrokes(shortcut);
        cx.run_until_parked();
        assert_eq!(
            workspace.read_with(cx, |workspace, _| workspace
                .file_manager
                .tree
                .model
                .root()
                .map(Path::to_path_buf)),
            Some(directory.path().to_path_buf()),
            "{shortcut} in the editor must not navigate the tree"
        );
    }
}

#[gpui::test]
fn entering_requires_a_single_child_folder_and_disabled_toolbar_buttons_are_skipped(
    cx: &mut gpui::TestAppContext,
) {
    let (directory, workspace, cx) = fixture(cx);
    for selection in ["root", "file", "empty", "multiple"] {
        workspace.update(cx, |workspace, cx| {
            let tree = &mut workspace.file_manager.tree.model;
            match selection {
                "root" => tree.select(EntryId::for_path(directory.path()), TreeSelection::Single),
                "file" => tree.select(
                    EntryId::for_path(&directory.path().join("notes.org")),
                    TreeSelection::Single,
                ),
                "empty" => tree.clear_selection(),
                _ => {
                    tree.select(EntryId::for_path(directory.path()), TreeSelection::Single);
                    tree.select(
                        EntryId::for_path(&directory.path().join("chapter")),
                        TreeSelection::Toggle,
                    );
                }
            }
            workspace.focus_sidebar(cx);
        });
        cx.run_until_parked();
        assert!(workspace.read_with(cx, |workspace, _| {
            workspace
                .file_manager
                .tree
                .directory_root_target(None)
                .is_none()
        }));
        let enter = cx.debug_bounds("file-tree-enter").unwrap();
        cx.simulate_click(enter.center(), Modifiers::default());
        cx.simulate_keystrokes("cmd-down shift-tab left left");
        cx.run_until_parked();
        cx.update(|window, cx| {
            let tree = &workspace.read(cx).file_manager.tree;
            assert_eq!(tree.model.root(), Some(directory.path()), "{selection}");
            let index = TOOLBAR_COMMANDS
                .iter()
                .position(|command| *command == FileTreeCommand::ParentRoot)
                .unwrap();
            assert!(
                tree.toolbar.as_ref().unwrap()[index].is_focused(window),
                "{selection}"
            );
        });
    }
    workspace.update(cx, |workspace, cx| {
        workspace.set_file_tree_root(PathBuf::from("/"), cx);
        workspace.focus_sidebar(cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("file-tree-parent").is_none());
    cx.simulate_keystrokes("cmd-up shift-tab left left");
    cx.update(|window, cx| {
        let tree = &workspace.read(cx).file_manager.tree;
        assert_eq!(tree.model.root(), Some(Path::new("/")));
        assert!(
            tree.toolbar.as_ref().unwrap()[MenuKind::More.toolbar_index().unwrap()]
                .is_focused(window)
        );
    });
}

#[gpui::test]
fn horizontal_tree_scrolling_takes_priority_over_sidebar_swipes(cx: &mut gpui::TestAppContext) {
    let (directory, workspace, cx) = fixture(cx);
    cx.update(|_, cx| cx.set_reduce_motion(true));
    workspace.update(cx, |workspace, cx| {
        workspace.file_manager.sidebar_width = 200;
        cx.notify();
    });
    select(&workspace, &directory.path().join("chapter"), cx);
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    let scroll = workspace.read_with(cx, |workspace, _| {
        workspace.file_manager.tree.horizontal_scroll.clone()
    });
    assert!(scroll.max_offset().x > px(0.));
    let position = scroll.bounds().center();
    trackpad(cx, position, -10., 0., TouchPhase::Started);
    trackpad(cx, position, -60., 0., TouchPhase::Moved);
    trackpad(cx, position, 0., 0., TouchPhase::Ended);
    assert!(workspace.read_with(cx, |workspace, _| workspace.sidebar_visible()));
    assert!(scroll.offset().x < px(0.));
}

#[gpui::test]
fn horizontal_reading_table_scrolling_takes_priority_over_sidebar_swipes(
    cx: &mut gpui::TestAppContext,
) {
    let (directory, workspace, cx) = fixture(cx);
    cx.update(|_, cx| cx.set_reduce_motion(true));
    cx.simulate_resize(gpui::size(px(1000.), px(650.)));
    let header = (0..32)
        .map(|column| format!("Column {column}"))
        .collect::<Vec<_>>()
        .join(" | ");
    let divider = ["----"; 32].join("+");
    let values = ["value"; 32].join(" | ");
    let path = directory.path().join("table.org");
    std::fs::write(&path, format!("| {header} |\n|{divider}|\n| {values} |\n")).unwrap();
    let loaded = crate::preview::load_document(path).unwrap();
    workspace.update(cx, |workspace, cx| {
        workspace.minimap_visible = false;
        workspace.show_reading(cx);
        assert!(workspace.apply_load_result(0, Ok(loaded), cx));
    });
    cx.run_until_parked();
    let scroll = workspace
        .read_with(cx, |workspace, cx| {
            workspace
                .reading_panel()
                .unwrap()
                .read(cx)
                .render_state()
                .table_scroll_handles
                .values()
                .find(|scroll| scroll.max_offset().x > px(0.))
                .cloned()
        })
        .expect("wide table has a horizontal scroll viewport");
    let position = scroll.bounds().center();
    trackpad(cx, position, -10., 0., TouchPhase::Started);
    trackpad(cx, position, -60., 0., TouchPhase::Moved);
    trackpad(cx, position, 0., 0., TouchPhase::Ended);
    assert!(workspace.read_with(cx, |workspace, _| workspace.sidebar_visible()));
    assert!(scroll.offset().x < px(0.));

    let position = point(px(700.), px(350.));
    trackpad(cx, position, -50., 0., TouchPhase::Started);
    trackpad(cx, position, 0., 0., TouchPhase::Ended);
    assert!(!workspace.read_with(cx, |workspace, _| workspace.sidebar_visible()));
    trackpad(cx, position, 50., 0., TouchPhase::Started);
    trackpad(cx, position, 0., 0., TouchPhase::Ended);
    assert!(workspace.read_with(cx, |workspace, _| workspace.sidebar_visible()));
}

#[gpui::test]
fn arrows_expand_in_place_and_never_open_the_selected_document(cx: &mut gpui::TestAppContext) {
    let (directory, workspace, cx) = fixture(cx);
    let original = workspace.read_with(cx, |workspace, _| workspace.document_session().cloned());
    select(&workspace, &directory.path().join("chapter"), cx);
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        let tree = &workspace.file_manager.tree.model;
        assert_eq!(tree.root(), Some(directory.path()));
        assert!(tree.expanded(&directory.path().join("chapter")));
        assert_eq!(
            tree.current().unwrap().path.as_ref(),
            directory.path().join("chapter")
        );
        assert!(
            tree.rows()
                .iter()
                .filter_map(TreeRow::entry)
                .any(|entry| entry.display_name.as_ref() == "notes.org")
        );
        assert_eq!(workspace.document_session(), original.as_ref());
    });
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace
                .file_manager
                .tree
                .model
                .current()
                .unwrap()
                .display_name
                .as_ref(),
            "entry.org"
        );
        assert_eq!(workspace.document_session(), original.as_ref());
    });
    cx.simulate_keystrokes("left left");
    cx.run_until_parked();
    assert!(!workspace.read_with(cx, |workspace, _| {
        workspace
            .file_manager
            .tree
            .model
            .expanded(&directory.path().join("chapter"))
    }));
}

#[gpui::test]
fn folder_row_targets_toggle_on_every_click_in_a_rapid_sequence(cx: &mut gpui::TestAppContext) {
    let (directory, workspace, cx) = fixture(cx);
    let branch = directory.path().join("chapter");
    let child = branch.join("entry.org");
    let document = workspace.read_with(cx, |workspace, _| workspace.document_session().cloned());
    select(&workspace, &branch, cx);
    for selector in ROW_TARGETS {
        for click_count in 1..=4 {
            click_row_target(cx, selector, click_count, Modifiers::default());
            workspace.read_with(cx, |workspace, _| {
                let tree = &workspace.file_manager.tree.model;
                let expanded = click_count % 2 == 1;
                assert_eq!(
                    tree.expanded(&branch),
                    expanded,
                    "{selector}, click {click_count}"
                );
                assert_eq!(tree.entry(EntryId::for_path(&child)).is_some(), expanded);
                assert_eq!(tree.root(), Some(directory.path()));
                assert_eq!(tree.current().unwrap().path.as_ref(), branch);
                assert_eq!(workspace.document_session(), document.as_ref());
                assert!(workspace.file_manager.sidebar_focused);
            });
        }
    }
}

#[gpui::test]
fn row_targets_apply_modified_selection_without_opening_or_toggling(cx: &mut gpui::TestAppContext) {
    let (directory, workspace, cx) = fixture(cx);
    let branch = directory.path().join("chapter");
    let child = branch.join("entry.org");
    let document = workspace.read_with(cx, |workspace, _| workspace.document_session().cloned());
    select(&workspace, &branch, cx);
    cx.simulate_keystrokes("right");
    cx.run_until_parked();

    for path in [&branch, &child] {
        for selector in ROW_TARGETS {
            for (modifiers, selected) in [
                (
                    Modifiers {
                        platform: true,
                        ..Default::default()
                    },
                    false,
                ),
                (
                    Modifiers {
                        shift: true,
                        ..Default::default()
                    },
                    true,
                ),
            ] {
                select(&workspace, path, cx);
                click_row_target(cx, selector, 2, modifiers);
                workspace.read_with(cx, |workspace, _| {
                    let tree = &workspace.file_manager.tree.model;
                    assert!(tree.expanded(&branch), "{selector}");
                    assert_eq!(
                        tree.selected(EntryId::for_path(path)),
                        selected,
                        "{selector}"
                    );
                    assert_eq!(tree.current().unwrap().path.as_ref(), path);
                    assert_eq!(workspace.document_session(), document.as_ref());
                    assert!(workspace.file_manager.sidebar_focused);
                });
            }
        }
    }
}

#[gpui::test]
fn file_rows_keep_tree_focus_until_double_click_or_return(cx: &mut gpui::TestAppContext) {
    let (directory, workspace, cx) = fixture(cx);
    let branch = directory.path().join("chapter");
    let child = branch.join("entry.org");
    select(&workspace, &branch, cx);
    cx.simulate_keystrokes("right");
    cx.run_until_parked();

    for selector in ROW_TARGETS {
        workspace.update(cx, |workspace, cx| {
            workspace.open(directory.path().join("notes.org"), cx);
            workspace.focus_sidebar(cx);
        });
        cx.run_until_parked();
        select(&workspace, &child, cx);
        click_row_target(cx, selector, 1, Modifiers::default());
        let document = cx.update(|window, cx| {
            let workspace = workspace.read(cx);
            let tree = &workspace.file_manager.tree;
            assert_eq!(
                workspace.document_path(cx),
                Some(child.as_path()),
                "{selector}"
            );
            assert!(workspace.file_manager.sidebar_focused, "{selector}");
            assert!(
                tree.input
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .focus
                    .is_focused(window)
            );
            workspace.document_session().cloned()
        });
        if selector == ROW_TARGETS[0] {
            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            cx.simulate_input("Text");
            cx.run_until_parked();
            workspace.read_with(cx, |workspace, cx| {
                assert!(!workspace.file_manager.sidebar_focused);
                assert!(workspace.document_session().unwrap().read(cx).is_dirty());
            });
            workspace.update(cx, |workspace, cx| workspace.focus_sidebar(cx));
            cx.run_until_parked();
        }
        click_row_target(cx, selector, 2, Modifiers::default());
        cx.update(|window, cx| {
            let workspace = workspace.read(cx);
            let tree = &workspace.file_manager.tree;
            assert_eq!(
                workspace.document_path(cx),
                Some(child.as_path()),
                "{selector}"
            );
            assert_eq!(workspace.document_session(), document.as_ref());
            assert!(!workspace.file_manager.sidebar_focused, "{selector}");
            assert!(
                !tree
                    .input
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .focus
                    .is_focused(window)
            );
        });
    }
}

#[gpui::test]
fn type_ahead_is_text_input_and_menus_restore_tree_focus(cx: &mut gpui::TestAppContext) {
    let (directory, workspace, cx) = fixture(cx);
    assert!(cx.debug_bounds("file-tree-name-query").is_none());
    cx.simulate_input("chap");
    cx.run_until_parked();
    assert!(cx.debug_bounds("file-tree-name-query").is_some());
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace
                .file_manager
                .tree
                .model
                .current()
                .unwrap()
                .display_name
                .as_ref(),
            "chapter"
        );
        assert!(!workspace.document_session().unwrap().read(cx).is_dirty());
        assert!(
            !workspace
                .file_manager
                .tree
                .model
                .expanded(&directory.path().join("chapter"))
        );
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(801));
    cx.run_until_parked();
    assert!(cx.debug_bounds("file-tree-name-query").is_none());
    cx.simulate_keystrokes("shift-f10");
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |workspace, _| {
        workspace.file_manager.tree.menu.is_some()
    }));
    cx.simulate_keystrokes("escape right");
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |workspace, _| {
        workspace.file_manager.tree.menu.is_none()
            && workspace
                .file_manager
                .tree
                .model
                .expanded(&directory.path().join("chapter"))
    }));
}

#[gpui::test]
fn name_lookup_works_after_blank_selection_and_only_after_ime_commit(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::EntityInputHandler;
    let (_, workspace, cx) = fixture(cx);
    workspace.update(cx, |workspace, cx| {
        workspace.file_manager.tree.model.clear_selection();
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_input("ch");
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace
            .file_manager
            .tree
            .model
            .current()
            .unwrap()
            .display_name
            .to_string()),
        "chapter"
    );
    let input = workspace.read_with(cx, |workspace, _| {
        workspace.file_manager.tree.input.clone().unwrap()
    });
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.sync("", cx);
            input.replace_and_mark_text_in_range(None, "notes", Some(5..5), window, cx);
        })
    });
    // The test platform has no IME and would insert a literal newline. Feed
    // the navigation guard directly, then emulate the native commit below.
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            for key in ["right", "enter"] {
                input.key_down(
                    &gpui::KeyDownEvent {
                        keystroke: gpui::Keystroke::parse(key).unwrap(),
                        is_held: false,
                        prefer_character_input: false,
                    },
                    window,
                    cx,
                );
            }
        });
    });
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace
            .file_manager
            .tree
            .model
            .current()
            .unwrap()
            .display_name
            .to_string()),
        "chapter"
    );
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.replace_text_in_range(None, "notes", window, cx)
        })
    });
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace
            .file_manager
            .tree
            .model
            .current()
            .unwrap()
            .display_name
            .to_string()),
        "notes.org"
    );
}

#[gpui::test]
fn retry_is_keyboard_reachable_and_releases_focus_when_the_branch_recovers(
    cx: &mut gpui::TestAppContext,
) {
    let (directory, workspace, cx) = fixture(cx);
    let branch = directory.path().join("chapter");
    workspace.update(cx, |workspace, cx| {
        let request = workspace
            .file_manager
            .directories
            .request(branch.clone(), true)
            .unwrap();
        workspace.file_manager.directories.commit(
            &request,
            Err(crate::file_manager::ScanError {
                directory: branch.clone(),
                source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied"),
            }),
        );
        workspace
            .file_manager
            .tree
            .model
            .select(EntryId::for_path(&branch), TreeSelection::Single);
        workspace
            .file_manager
            .tree
            .model
            .set_expanded(&branch, true);
        workspace.rebuild_file_tree(cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(workspace.read(cx).file_manager.tree.retry_focus[&branch].is_focused(window))
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, cx| {
        let tree = &workspace.read(cx).file_manager.tree;
        assert!(!tree.retry_focus.contains_key(&branch));
        assert!(
            tree.input
                .as_ref()
                .unwrap()
                .read(cx)
                .focus
                .is_focused(window)
        );
    });
}

#[gpui::test]
fn inline_creation_reports_conflicts_and_escape_never_writes(cx: &mut gpui::TestAppContext) {
    let (directory, workspace, cx) = fixture(cx);
    select(&workspace, directory.path(), cx);
    cx.simulate_keystrokes("cmd-alt-n");
    cx.run_until_parked();
    assert!(
        workspace.read_with(cx, |workspace, _| workspace
            .file_manager
            .tree
            .edit
            .is_some()),
        "new file command must open the name input"
    );
    cx.simulate_input("notes.org");
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace
                .file_manager
                .tree
                .edit
                .as_ref()
                .unwrap()
                .input
                .read(cx)
                .text,
            "notes.org"
        )
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(
        workspace.read_with(cx, |workspace, _| {
            workspace
                .file_manager
                .tree
                .edit
                .as_ref()
                .is_some_and(|edit| edit.error.is_some())
        }),
        "name state: {:?}",
        workspace.read_with(cx, |workspace, cx| (
            workspace.file_manager.operation_busy,
            workspace
                .file_manager
                .tree
                .edit
                .as_ref()
                .map(|edit| (edit.input.read(cx).text.clone(), edit.error.clone())),
            workspace
                .file_manager
                .status
                .as_ref()
                .map(|status| status.message().to_string())
        ))
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("notes.org")).unwrap(),
        "* Notes\n"
    );
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("new.org");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!directory.path().join("new.org").exists());
    cx.simulate_keystrokes("cmd-alt-n");
    cx.run_until_parked();
    cx.simulate_input("new.org");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(directory.path().join("new.org").exists());
    assert!(workspace.read_with(cx, |workspace, _| {
        workspace.file_manager.tree.edit.is_none()
    }));
}

#[gpui::test]
fn folder_rename_retargets_parked_documents_without_accepting_external_changes(
    cx: &mut gpui::TestAppContext,
) {
    let (directory, workspace, cx) = fixture(cx);
    let source = directory.path().join("chapter");
    let entry = source.join("entry.org");
    workspace.update(cx, |workspace, cx| workspace.open(entry.clone(), cx));
    cx.run_until_parked();
    let session = workspace.read_with(cx, |workspace, _| {
        workspace.document_session().unwrap().clone()
    });
    session.update(cx, |session, cx| {
        session
            .edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        session.revision(),
                        vec![TextEdit::new(ByteRange::new(0, 0), "Local\n")],
                    ),
                    Selection::default(),
                    Selection::default(),
                    EditOrigin::Other,
                ),
                cx,
            )
            .unwrap();
    });
    std::fs::write(&entry, "* External\n").unwrap();
    workspace.update(cx, |workspace, cx| {
        workspace.open(directory.path().join("notes.org"), cx)
    });
    cx.run_until_parked();
    let destination = directory.path().join("section");
    workspace.update(cx, |workspace, cx| {
        workspace.execute_dired_operation(
            OperationPlan::rename(source.into(), destination.clone()),
            cx,
        )
    });
    cx.run_until_parked();
    session.read_with(cx, |session, _| {
        assert_eq!(
            session.file_path(),
            Some(destination.join("entry.org").as_path())
        );
        assert!(session.is_dirty());
        assert_ne!(
            session.sync_state().base().content_hash,
            crate::document::FileStamp::read(&destination.join("entry.org"))
                .unwrap()
                .content_hash
        );
    });
}
