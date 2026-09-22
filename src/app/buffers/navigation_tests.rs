use super::*;
use crate::document::{DocumentCommand, EditOrigin, EditTransaction, TextEdit};
use gpui::{AppContext, TestAppContext};

fn insert(session: &Entity<DocumentSession>, offset: u64, text: &str, cx: &mut TestAppContext) {
    session.update(cx, |s, cx| {
        s.edit(
            DocumentCommand::new(
                EditTransaction::new(
                    s.revision(),
                    vec![TextEdit::new(ByteRange::new(offset, offset), text)],
                ),
                Selection::default(),
                Selection::default(),
                EditOrigin::Other,
            ),
            cx,
        )
        .unwrap();
    });
}

fn query(w: &mut WorkspaceWindow, text: &str, cx: &mut Context<WorkspaceWindow>) {
    let Some(Panel::Picker(p)) = &mut w.buffers.panel else {
        panic!("picker not open")
    };
    p.selected = 0;
    p.input.update(cx, |input, cx| input.sync(text, cx));
    cx.notify();
}

#[gpui::test]
fn quick_open_combines_recents_deduplicates_and_does_not_create_on_miss(cx: &mut TestAppContext) {
    let w = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    w.update(cx, |w, cx| {
        w.create_buffer(
            "notes.org".into(),
            Some("/tmp/navigation/notes.org".into()),
            cx,
        );
        w.recent_documents = vec![
            crate::recent_documents::RecentDocument {
                path: "/tmp/navigation/notes.org".into(),
                opened_at: 2,
            },
            crate::recent_documents::RecentDocument {
                path: "/tmp/navigation/research.md".into(),
                opened_at: 1,
            },
        ];
        w.open_buffer_picker(PickerIntent::Switch, cx);
        let items = w.buffer_candidates(cx);
        assert_eq!(items.len(), 2);
        assert!(items[0].id.is_some());
        assert!(items[1].id.is_none());
        query(w, "rsch", cx);
        assert_eq!(w.buffer_candidates(cx)[0].name, "research.md");
        query(w, "navigation", cx);
        assert_eq!(w.buffer_candidates(cx).len(), 2);
        query(w, "no-such-file", cx);
        w.accept_buffer_picker(cx);
        w.accept_buffer_picker(cx);
        assert_eq!(w.buffer_sessions().count(), 1);
        assert!(w.buffers.navigation.back.is_empty());
    });
}

#[gpui::test]
fn quick_open_heading_keyboard_cancel_and_number_pick(cx: &mut TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| {
        cx.bind_keys([
            gpui::KeyBinding::new("cmd-p", crate::app::SwitchBuffer, None),
            gpui::KeyBinding::new("cmd-[", crate::app::NavigateBack, None),
            gpui::KeyBinding::new("cmd-]", crate::app::NavigateForward, None),
        ])
    });
    cx.update(|cx| cx.set_reduce_motion(true));
    for (name, text) in [
        (
            "outline.org",
            "* Notebook\n** Planning\ntext\n#+begin_src text\n* Not a heading\n#+end_src\n** Reading\nbody\n",
        ),
        (
            "outline.md",
            "# Notebook\n## Planning\ntext\n```text\n# Not a heading\n```\n## Reading\nbody\n",
        ),
    ] {
        let (w, view) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
        view.simulate_resize(gpui::size(gpui::px(390.), gpui::px(600.)));
        let session = w.update(view, |w, cx| {
            w.create_buffer(name.into(), None, cx);
            w.recent_documents.clear();
            w.document_session().unwrap().clone()
        });
        insert(&session, 0, text, view);
        view.run_until_parked();
        let editor = w.update(view, |w, _| {
            w.editor(w.document_workspace.active_pane).unwrap()
        });
        let before = editor.read_with(view, |e, cx| {
            (e.selection(), e.top_source_anchor(&e.snapshot(cx)))
        });
        view.simulate_keystrokes("cmd-p");
        view.simulate_input("@");
        view.run_until_parked();
        let shell = view.debug_bounds("floating-status-line").unwrap();
        let row = view.debug_bounds("quick-open-row-0").unwrap();
        assert!(row.left() >= shell.left() && row.right() <= shell.right());
        w.update(view, |w, cx| {
            let items = w.buffer_candidates(cx);
            assert_eq!(items.len(), 3, "literal code must not contribute headings");
            assert_eq!(items[1].path, "Notebook");
            assert_eq!(items[2].heading.as_ref().unwrap().line, 7);
        });
        view.simulate_keystrokes("down escape");
        view.run_until_parked();
        assert_eq!(
            before,
            editor.read_with(view, |e, cx| (
                e.selection(),
                e.top_source_anchor(&e.snapshot(cx))
            ))
        );
        view.simulate_keystrokes("cmd-p");
        view.simulate_input("@");
        view.simulate_keystrokes("ctrl-3");
        view.run_until_parked();
        w.update(view, |w, cx| {
            assert!(w.buffers.panel.is_none());
            let target = session
                .read(cx)
                .snapshot()
                .line_content_range(LineIndex(6))
                .unwrap()
                .start;
            assert_eq!(editor.read(cx).selection().head(), target);
            assert_eq!(w.buffers.navigation.back.len(), 1);
        });
        view.simulate_keystrokes("cmd-[");
        view.run_until_parked();
        assert_eq!(
            before,
            editor.read_with(view, |e, cx| (
                e.selection(),
                e.top_source_anchor(&e.snapshot(cx))
            ))
        );
        view.simulate_keystrokes("cmd-]");
        view.run_until_parked();
        w.update(view, |_, cx| {
            let target = session
                .read(cx)
                .snapshot()
                .line_content_range(LineIndex(6))
                .unwrap()
                .start;
            assert_eq!(editor.read(cx).selection().head(), target);
        });
        assert_eq!(
            session.read_with(view, |s, _| s
                .snapshot()
                .copy_range(ByteRange::new(0, s.snapshot().len_bytes()))),
            text
        );
    }
}

#[gpui::test]
fn navigation_restores_scrolled_origin_across_buffers_and_maps_edits(cx: &mut TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, view) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let a = w.update(view, |w, cx| {
        w.create_buffer("journal.org".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    insert(
        &a,
        0,
        &"An ordinary line of source text\n".repeat(160),
        view,
    );
    view.run_until_parked();
    let editor = w.update(view, |w, _| {
        w.editor(w.document_workspace.active_pane).unwrap()
    });
    editor.update(view, |e, cx| {
        let snapshot = e.snapshot(cx);
        let scroll = snapshot.line_content_range(LineIndex(45)).unwrap().start;
        e.set_selection(Selection::new(ByteOffset(3), ByteOffset(8)), cx);
        e.restore_scroll(scroll, 0.35, 0., cx);
    });
    view.run_until_parked();
    let before = editor.read_with(view, |e, cx| {
        (e.selection(), e.top_source_anchor(&e.snapshot(cx)))
    });
    let b = w.update(view, |w, cx| {
        w.create_buffer("draft.md".into(), None, cx);
        let b = w.document_session().unwrap().clone();
        w.activate_buffer(a.read(cx).id(), cx);
        w.buffers.navigation = NavigationHistory::default();
        w.activate_buffer(b.read(cx).id(), cx);
        b
    });
    insert(&a, 0, "New preface\n", view);
    w.update(view, |w, cx| w.navigate_history(false, cx));
    view.run_until_parked();
    w.update(view, |w, cx| {
        assert_eq!(w.document_session().unwrap(), &a);
        assert!(w.navigation_can_go(true, cx));
    });
    let after = editor.read_with(view, |e, cx| {
        (e.selection(), e.top_source_anchor(&e.snapshot(cx)))
    });
    assert_eq!(after.0.head().0, before.0.head().0 + 12);
    assert_eq!(after.0.anchor().0, before.0.anchor().0 + 12);
    assert_eq!(after.1.0.0, before.1.0.0 + 12);
    assert!((after.1.1 - before.1.1).abs() < 0.01);
    w.update(view, |w, cx| {
        w.navigate_history(true, cx);
        assert_eq!(w.document_session().unwrap(), &b);
        w.navigate_history(false, cx);
        w.navigate_to_source(ByteOffset(0), cx);
        assert!(
            !w.navigation_can_go(true, cx),
            "a new jump clears the forward branch"
        );
    });
}

#[gpui::test]
fn navigation_reading_history_restores_active_split_pane(cx: &mut TestAppContext) {
    use crate::app::{PaneSide, PaneSurface};
    cx.update(crate::editor::init);
    cx.update(|cx| {
        cx.bind_keys([
            gpui::KeyBinding::new("cmd-p", crate::app::SwitchBuffer, None),
            gpui::KeyBinding::new("cmd-[", crate::app::NavigateBack, None),
            gpui::KeyBinding::new("cmd-]", crate::app::NavigateForward, None),
        ])
    });
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, view) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(true));
    let doc = w.update(view, |w, cx| {
        w.create_buffer("chapters.org".into(), None, cx);
        w.document_workspace
            .set_surface(PaneSide::Right, PaneSurface::Reading);
        w.document_session().unwrap().clone()
    });
    insert(
        &doc,
        0,
        &(0..120)
            .map(|i| format!("* Chapter {i}\nNotes\n"))
            .collect::<String>(),
        view,
    );
    w.update(view, |w, cx| w.schedule_derived_update(cx));
    view.run_until_parked();
    view.executor()
        .advance_clock(std::time::Duration::from_millis(30));
    view.run_until_parked();
    let (editor, reader) = w.update(view, |w, cx| {
        w.activate_pane(PaneSide::Right, cx);
        (
            w.editor(PaneSide::Left).unwrap(),
            w.reading_panel_for(PaneSide::Right).unwrap(),
        )
    });
    let editor_before = editor.read_with(view, |e, cx| {
        (e.selection(), e.top_source_anchor(&e.snapshot(cx)))
    });
    let origin = reader.read_with(view, |r, _| r.top_source_anchor().unwrap());
    view.simulate_keystrokes("cmd-p");
    view.simulate_input("@ Chapter 80");
    view.simulate_keystrokes("enter");
    view.run_until_parked();
    w.update(view, |w, cx| {
        assert_eq!(w.document_workspace.active_pane, PaneSide::Right);
        assert_eq!(
            doc.read(cx)
                .snapshot()
                .line_index_at(reader.read(cx).top_source_offset().unwrap())
                .unwrap(),
            LineIndex(160),
        );
        w.navigate_history(false, cx);
    });
    view.run_until_parked();
    assert_eq!(
        reader.read_with(view, |r, _| r.top_source_anchor().unwrap()),
        origin
    );
    assert_eq!(
        editor.read_with(view, |e, cx| (
            e.selection(),
            e.top_source_anchor(&e.snapshot(cx))
        )),
        editor_before
    );
    insert(&doc, 0, "* Preface\nNew notes\n", view);
    w.update(view, |w, cx| {
        assert!(!w.latest_preview_is_current(cx));
        w.open_buffer_picker(PickerIntent::Switch, cx);
        query(w, "@ Chapter 80", cx);
        assert!(w.buffer_candidates(cx).is_empty());
        w.accept_buffer_picker(cx);
        assert!(
            w.buffers.panel.is_some(),
            "wait for the matching preview revision"
        );
        w.schedule_derived_update(cx);
    });
    view.run_until_parked();
    view.executor()
        .advance_clock(std::time::Duration::from_millis(30));
    view.run_until_parked();
    w.update(view, |w, cx| {
        assert_eq!(w.buffer_candidates(cx).len(), 1);
        w.accept_buffer_picker(cx);
        assert!(w.buffers.panel.is_none());
        assert_eq!(
            doc.read(cx)
                .snapshot()
                .line_index_at(reader.read(cx).top_source_offset().unwrap())
                .unwrap(),
            LineIndex(162),
        );
    });
}

#[gpui::test]
fn navigation_preserves_selection_direction_and_labels_the_caret_line(cx: &mut TestAppContext) {
    let w = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    let doc = w.update(cx, |w, cx| {
        w.create_buffer("selection.org".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    insert(&doc, 0, "First\nSecond\nThird\n", cx);
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        let editor = w.editor(w.document_workspace.active_pane).unwrap();
        for (anchor, head, line) in [(1, 8, 2), (8, 1, 1)] {
            let selection = Selection::new(ByteOffset(anchor), ByteOffset(head));
            editor.update(cx, |e, cx| e.set_selection(selection, cx));
            w.navigate_to_source(ByteOffset(13), cx);
            assert_eq!(
                w.navigation_back_label(cx).unwrap(),
                format!("selection.org · {line}")
            );
            w.navigate_history(false, cx);
            assert_eq!(editor.read(cx).selection(), selection);
        }
    });
}

#[gpui::test]
fn navigation_commits_file_history_after_success_and_preserves_it_on_failure(
    cx: &mut TestAppContext,
) {
    let root = std::env::temp_dir().join(format!("navigation-load-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("destination.org");
    std::fs::write(&path, "* Destination\nNew document\n").unwrap();
    let w = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    let origin = w.update(cx, |w, cx| {
        w.create_buffer("origin.org".into(), None, cx);
        let origin = w.document_session().unwrap().clone();
        w.open(path.clone(), cx);
        assert!(w.buffers.navigation.back.is_empty());
        origin
    });
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert_eq!(w.buffers.navigation.back.len(), 1);
        w.navigate_history(false, cx);
        assert_eq!(w.document_session().unwrap(), &origin);
        assert!(w.navigation_can_go(true, cx));
        w.open(root.join("missing.org"), cx);
    });
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert_eq!(w.document_session().unwrap(), &origin);
        assert!(w.buffers.navigation.back.is_empty());
        assert!(w.navigation_can_go(true, cx));
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn quick_open_reselects_current_file_aligns_input_and_accepts_control_zero(
    cx: &mut TestAppContext,
) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, view) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    view.simulate_resize(gpui::size(gpui::px(800.), gpui::px(600.)));
    let current = w.update(view, |w, cx| {
        for index in 0..12 {
            w.create_buffer(format!("note-{index}.org"), None, cx);
        }
        w.recent_documents.clear();
        w.document_session().unwrap().read(cx).id()
    });
    view.run_until_parked();
    view.simulate_keystrokes("ctrl-x b");
    view.run_until_parked();
    let label = view.debug_bounds("quick-open-input-label").unwrap();
    let input = view.debug_bounds("buffer-query").unwrap();
    assert_eq!(label.top(), input.top());
    assert_eq!(label.bottom(), input.bottom());
    w.update(view, |w, cx| {
        if let Some(Panel::Picker(p)) = &mut w.buffers.panel {
            p.selected = 11;
        }
        w.buffers.scroll.scroll_to_item(11);
        cx.notify();
    });
    view.run_until_parked();
    assert!(w.read_with(view, |w, _| w.buffers.scroll.offset().y) < gpui::px(0.));
    view.simulate_keystrokes("escape ctrl-x b");
    view.run_until_parked();
    let tenth = w.update(view, |w, cx| {
        let Some(Panel::Picker(p)) = &w.buffers.panel else {
            panic!("picker not open")
        };
        let items = w.buffer_candidates(cx);
        assert_eq!(p.selected, 0);
        assert_eq!(items[p.selected].id, Some(current));
        assert_eq!(w.buffers.scroll.offset().y, gpui::px(0.));
        items[9].id.unwrap()
    });
    let row = view.debug_bounds("quick-open-row-0").unwrap();
    let shell = view.debug_bounds("floating-status-line").unwrap();
    assert!(row.top() >= shell.top() && row.bottom() < input.top());
    view.simulate_keystrokes("ctrl-0");
    view.run_until_parked();
    w.update(view, |w, cx| {
        assert!(w.buffers.panel.is_none());
        assert_eq!(w.document_session().unwrap().read(cx).id(), tenth);
    });
}

#[gpui::test]
fn quick_open_command_shortcuts_override_editor_modes_only_while_open(cx: &mut TestAppContext) {
    use crate::app::{PaneSurface, WorkspaceLayout};
    cx.update(crate::editor::init);
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        cx.bind_keys([
            gpui::KeyBinding::new("cmd-1", crate::preview::ShowEditor, None),
            gpui::KeyBinding::new("cmd-2", crate::preview::ShowReading, None),
            gpui::KeyBinding::new("cmd-3", crate::preview::ShowSplit, None),
        ]);
        cx.intercept_keystrokes(WorkspaceWindow::intercept_quick_open_shortcuts)
            .detach();
    });
    let (w, view) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    w.update(view, |w, cx| {
        for index in 0..12 {
            w.create_buffer(format!("switch-{index}.org"), None, cx);
        }
        w.recent_documents.clear();
    });
    view.run_until_parked();
    view.simulate_keystrokes("cmd-2");
    view.run_until_parked();
    let current = w.update(view, |w, cx| {
        assert_eq!(w.document_workspace.active_surface(), PaneSurface::Reading);
        w.open_buffer_picker(PickerIntent::Switch, cx);
        w.document_session().unwrap().read(cx).id()
    });
    view.run_until_parked();
    view.simulate_keystrokes("cmd-1");
    view.run_until_parked();
    w.update(view, |w, cx| {
        assert!(w.buffers.panel.is_none());
        assert_eq!(w.document_session().unwrap().read(cx).id(), current);
        assert_eq!(w.document_workspace.active_surface(), PaneSurface::Reading);
    });
    for (key, index) in [("cmd-2", 1), ("cmd-3", 2), ("cmd-0", 9)] {
        let target = w.update(view, |w, cx| {
            w.open_buffer_picker(PickerIntent::Switch, cx);
            w.buffer_candidates(cx)[index].id.unwrap()
        });
        view.run_until_parked();
        view.simulate_keystrokes(key);
        view.run_until_parked();
        w.update(view, |w, cx| {
            assert!(w.buffers.panel.is_none());
            assert_eq!(w.document_session().unwrap().read(cx).id(), target);
            assert_eq!(w.document_workspace.active_surface(), PaneSurface::Editor);
            assert_eq!(w.document_workspace.layout, WorkspaceLayout::Single);
        });
    }
    for key in ["cmd-tab", "ctrl-tab"] {
        let current = w.update(view, |w, cx| {
            w.open_buffer_picker(PickerIntent::Switch, cx);
            w.document_session().unwrap().read(cx).id()
        });
        view.run_until_parked();
        view.simulate_keystrokes(key);
        view.run_until_parked();
        w.update(view, |w, cx| {
            assert!(w.buffers.panel.is_some());
            assert_eq!(w.document_session().unwrap().read(cx).id(), current);
        });
    }
    w.update(view, |w, cx| {
        w.open_buffer_picker(PickerIntent::Switch, cx);
        query(w, "@ no heading", cx);
    });
    view.run_until_parked();
    view.simulate_keystrokes("cmd-2 cmd-3");
    view.run_until_parked();
    w.update(view, |w, _| {
        assert!(w.buffers.panel.is_some());
        assert_eq!(w.document_workspace.active_surface(), PaneSurface::Editor);
        assert_eq!(w.document_workspace.layout, WorkspaceLayout::Single);
    });
    view.simulate_keystrokes("escape cmd-2");
    view.run_until_parked();
    w.update(view, |w, _| {
        assert_eq!(w.document_workspace.active_surface(), PaneSurface::Reading)
    });
}
