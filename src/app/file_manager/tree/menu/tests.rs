use super::super::tests::fixture;
use super::*;
use crate::{
    file_manager::{SortDirection, SortKey},
    i18n::Language,
};
use gpui::Modifiers;

#[gpui::test]
fn toolbar_menu_tracks_the_trigger_and_toggles_without_reopening(cx: &mut gpui::TestAppContext) {
    let (_, workspace, cx) = fixture(cx);
    workspace.update(cx, |workspace, cx| {
        workspace.language = Language::English;
        cx.notify();
    });
    cx.run_until_parked();
    let trigger = cx.debug_bounds("file-tree-more").unwrap();
    cx.simulate_click(trigger.center(), Modifiers::default());
    cx.run_until_parked();
    let bounds = cx.debug_bounds("file-tree-menu").unwrap();
    assert_eq!(bounds.top(), (trigger.bottom() + ui_px(4.)).round());
    assert_eq!(
        bounds.left(),
        (trigger.right() - bounds.size.width)
            .max(ui_px(MARGIN))
            .round()
    );
    workspace.read_with(cx, |workspace, _| {
        assert!(
            workspace
                .file_manager
                .tree
                .menu
                .as_ref()
                .unwrap()
                .main
                .selected
                .is_none(),
            "mouse opening must not highlight the first command"
        );
    });
    let label = cx.debug_bounds("file-tree-menu-label-0-0").unwrap();
    let shortcut = cx.debug_bounds("file-tree-menu-shortcut-0-0").unwrap();
    let long_label = cx.debug_bounds("file-tree-menu-label-0-4").unwrap();
    let long_shortcut = cx.debug_bounds("file-tree-menu-shortcut-0-4").unwrap();
    assert_eq!(long_label.left(), label.left());
    assert_eq!(long_shortcut.right(), shortcut.right());
    assert!(long_label.right() < long_shortcut.left());
    // The same trigger receives the click, rather than dismissing on mouse down
    // and reopening on mouse up. Switching to the other trigger also works.
    cx.simulate_click(trigger.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |workspace, _| {
        workspace.file_manager.tree.menu.is_none()
    }));
    cx.simulate_click(trigger.center(), Modifiers::default());
    cx.run_until_parked();
    let new = cx.debug_bounds("file-tree-new").unwrap();
    cx.simulate_click(new.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |workspace, _| {
        workspace.file_manager.tree.menu.as_ref().unwrap().kind == MenuKind::New
    }));
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| {
        assert!(
            workspace
                .read(cx)
                .file_manager
                .tree
                .toolbar
                .as_ref()
                .unwrap()[MenuKind::New.toolbar_index().unwrap()]
            .is_focused(window)
        );
    });
    cx.simulate_keystrokes("right enter f down r e f");
    workspace.read_with(cx, |workspace, _| {
        let menu = workspace.file_manager.tree.menu.as_ref().unwrap();
        let item = &menu.main.items[menu.main.selected.unwrap()];
        assert!(item.action == MenuAction::Command(FileTreeCommand::Refresh));
    });
    cx.simulate_keystrokes("escape");
}

#[gpui::test]
fn sorting_submenu_supports_mouse_keyboard_checks_and_focus_return(cx: &mut gpui::TestAppContext) {
    let (_, workspace, cx) = fixture(cx);
    cx.simulate_resize(size(px(1200.), px(800.)));
    workspace.update(cx, |workspace, cx| {
        workspace.language = Language::English;
        cx.notify();
    });
    cx.run_until_parked();
    let trigger = cx.debug_bounds("file-tree-more").unwrap();
    cx.simulate_click(trigger.center(), Modifiers::default());
    cx.run_until_parked();
    let sort = cx.debug_bounds("file-tree-menu-0-5").unwrap();
    cx.simulate_mouse_move(sort.center(), None, Modifiers::default());
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        let menu = workspace.file_manager.tree.menu.as_ref().unwrap();
        assert_eq!(menu.active, Page::Sort);
        assert!(menu.sort.selected.is_none());
        assert!(menu.sort.items[0].checked);
        assert!(menu.sort.items[3].checked);
    });
    let submenu = cx.debug_bounds("file-tree-sort-menu").unwrap();
    let main = cx.debug_bounds("file-tree-menu").unwrap();
    assert!(
        submenu.left() >= main.right() - ui_px(3.),
        "main: {main:?}, submenu: {submenu:?}"
    );
    let modified = cx.debug_bounds("file-tree-menu-1-1").unwrap();
    cx.simulate_mouse_move(modified.center(), None, Modifiers::default());
    cx.simulate_click(modified.center(), Modifiers::default());
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace.file_manager.tree.model.sort.key,
            SortKey::Modified
        );
        assert!(workspace.file_manager.tree.menu.is_none());
    });
    // Keyboard opening starts on the first enabled command; type-ahead opens
    // sorting, Right enters it, Left/Escape dismiss one level at a time.
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.dispatch_file_tree_command(FileTreeCommand::MoreMenu, window, cx)
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("s o r t right end up enter");
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace
            .file_manager
            .tree
            .model
            .sort
            .direction),
        SortDirection::Descending
    );
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.dispatch_file_tree_command(FileTreeCommand::MoreMenu, window, cx)
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("s o r t right left right escape");
    assert!(workspace.read_with(cx, |workspace, _| {
        let menu = workspace.file_manager.tree.menu.as_ref().unwrap();
        menu.main.selected == Some(5) && menu.active == Page::Main
    }));
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| {
        assert!(
            workspace
                .read(cx)
                .file_manager
                .tree
                .toolbar
                .as_ref()
                .unwrap()[MenuKind::More.toolbar_index().unwrap()]
            .is_focused(window)
        )
    });
    // The glyphs shown in the menu correspond to functioning registered keys.
    cx.simulate_keystrokes("enter cmd-shift-.");
    assert!(workspace.read_with(cx, |workspace, _| {
        workspace.file_manager.tree.model.show_hidden
    }));
    assert!(workspace.read_with(cx, |workspace, _| {
        workspace.file_manager.tree.menu.is_none()
    }));
}

#[gpui::test]
fn small_window_menu_scrolls_to_keyboard_selection_and_stays_inside_viewport(
    cx: &mut gpui::TestAppContext,
) {
    let (_, workspace, cx) = fixture(cx);
    cx.simulate_resize(size(px(300.), px(250.)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.language = Language::English;
            workspace.dispatch_file_tree_command(FileTreeCommand::MoreMenu, window, cx);
        })
    });
    cx.run_until_parked();
    let menu = cx.debug_bounds("file-tree-menu").unwrap();
    assert!(menu.left() >= ui_px(MARGIN) && menu.right() <= px(300.) - ui_px(MARGIN));
    assert!(menu.top() >= ui_px(MARGIN) && menu.bottom() <= px(250.) - ui_px(MARGIN));
    cx.simulate_keystrokes("end");
    cx.run_until_parked();
    let last = cx.debug_bounds("file-tree-menu-0-7").unwrap();
    assert!(
        last.top() >= menu.top() && last.bottom() <= menu.bottom(),
        "End must scroll the active command into view: {last:?}, {menu:?}"
    );
    cx.simulate_keystrokes("home");
    cx.run_until_parked();
    let first = cx.debug_bounds("file-tree-menu-0-0").unwrap();
    assert!(first.top() >= menu.top() && first.bottom() <= menu.bottom());
    cx.simulate_keystrokes("s o r t right");
    cx.simulate_resize(size(px(260.), px(200.)));
    cx.run_until_parked();
    let submenu = cx.debug_bounds("file-tree-sort-menu").unwrap();
    assert!(submenu.left() >= ui_px(MARGIN) && submenu.right() <= px(260.) - ui_px(MARGIN));
    assert!(submenu.top() >= ui_px(MARGIN) && submenu.bottom() <= px(200.) - ui_px(MARGIN));
}

#[test]
fn menus_flip_and_fit_at_every_window_edge() {
    let viewport = size(px(640.), px(360.));
    let desired = size(px(250.), px(240.));
    for (x, y) in [(0., 0.), (620., 0.), (0., 340.), (620., 340.), (310., 170.)] {
        let anchor = Bounds::new(point(px(x), px(y)), size(px(20.), px(20.)));
        let menu = popup_bounds(desired, anchor, viewport, true);
        let limits = viewport_limits(viewport);
        assert!(menu.left() >= limits.left() && menu.right() <= limits.right());
        assert!(menu.top() >= limits.top() && menu.bottom() <= limits.bottom());
        assert!(menu.bottom() <= anchor.top() || menu.top() >= anchor.bottom());
        let submenu = submenu_bounds(size(px(230.), px(125.)), menu, anchor, viewport);
        assert!(submenu.left() >= limits.left() && submenu.right() <= limits.right());
        assert!(submenu.top() >= limits.top() && submenu.bottom() <= limits.bottom());
    }
    let near_bottom = Bounds::new(point(px(400.), px(320.)), size(px(28.), px(28.)));
    assert!(popup_bounds(desired, near_bottom, viewport, true).bottom() < near_bottom.top());
}

#[gpui::test]
fn enter_folder_menus_keep_the_captured_target_when_selection_changes(
    cx: &mut gpui::TestAppContext,
) {
    let (directory, workspace, cx) = fixture(cx);
    let child = directory.path().join("chapter");
    let document = workspace.read_with(cx, |workspace, _| {
        workspace.document_session().unwrap().clone()
    });
    for kind in [MenuKind::Context, MenuKind::More] {
        cx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.language = Language::English;
                workspace.set_file_tree_root(directory.path().to_path_buf(), cx);
                workspace.file_manager.tree.model.select(
                    crate::file_manager::EntryId::for_path(&child),
                    crate::file_manager::TreeSelection::Single,
                );
                workspace.open_file_tree_menu(kind, None, window, cx);
            });
        });
        cx.run_until_parked();
        workspace.read_with(cx, |workspace, _| {
            let items = &workspace
                .file_manager
                .tree
                .menu
                .as_ref()
                .unwrap()
                .main
                .items;
            let index = items
                .iter()
                .position(|item| item.action == MenuAction::Command(FileTreeCommand::EnterRoot))
                .unwrap();
            assert!(items[index].enabled);
            assert_eq!(items[index].shortcut.as_ref(), "⌘↓");
        });
        workspace.update(cx, |workspace, cx| {
            workspace.file_manager.tree.model.select(
                crate::file_manager::EntryId::for_path(&directory.path().join("notes.org")),
                crate::file_manager::TreeSelection::Single,
            );
            cx.notify();
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("e n t e r enter");
        cx.run_until_parked();
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(
                workspace.file_manager.tree.model.root(),
                Some(child.as_path())
            );
            assert_eq!(workspace.document_session().unwrap(), &document);
            assert!(workspace.file_manager.sidebar_focused);
            assert!(workspace.file_manager.tree.menu.is_none());
        });
    }
}

#[gpui::test]
fn a_directory_refresh_cannot_redirect_an_open_menu_to_a_different_file(
    cx: &mut gpui::TestAppContext,
) {
    let (directory, workspace, cx) = fixture(cx);
    let target = directory.path().join("target.org");
    std::fs::write(&target, "* Target\n").unwrap();
    std::fs::write(directory.path().join("other.org"), "* Other\n").unwrap();
    workspace.update(cx, |workspace, cx| {
        workspace.request_file_directory(directory.path().to_path_buf(), true, cx)
    });
    cx.run_until_parked();
    let document = workspace.read_with(cx, |workspace, _| workspace.document_session().cloned());
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.file_manager.tree.model.select(
                crate::file_manager::EntryId::for_path(&target),
                crate::file_manager::TreeSelection::Single,
            );
            workspace.dispatch_file_tree_command(FileTreeCommand::ContextMenu, window, cx);
        })
    });
    cx.run_until_parked();
    std::fs::remove_file(&target).unwrap();
    workspace.update(cx, |workspace, cx| {
        workspace.request_file_directory(directory.path().to_path_buf(), true, cx)
    });
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        let tree = &workspace.file_manager.tree;
        assert!(
            tree.model
                .current()
                .is_some_and(|entry| entry.path.as_ref() != target)
        );
        assert!(
            tree.operation_entry().is_none(),
            "the removed menu target must not become the nearest surviving row"
        );
    });
    cx.simulate_keystrokes("enter");
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.file_manager.tree.menu.is_none());
        assert_eq!(workspace.document_session(), document.as_ref());
    });
}
