use super::{
    GLOBAL_VISIBILITY_CYCLE_COMMAND, GlobalVisibility, MAX_EAGER_LAYOUT_ROWS, accept_generation,
    dired_command_items, preview_input, should_eagerly_measure_rows,
};

use crate::{
    document::{
        ByteOffset, ByteRange, EditOrigin, EditTransaction, Selection, SessionEdit, TextEdit,
        TextSnapshot,
    },
    input::EmacsOutcome,
    keymap::KeyStroke,
};
use gpui::AppContext;

fn visible_source_lines(
    app: &super::WorkspaceWindow,
    document: &super::PreviewSnapshot,
    cx: &gpui::App,
) -> Vec<u64> {
    app.preview_panel()
        .unwrap()
        .read(cx)
        .visible_rows()
        .iter()
        .map(|index| {
            document.text.line_of_byte(
                document
                    .projection
                    .rows
                    .get(*index)
                    .unwrap()
                    .source
                    .range
                    .start,
            ) + 1
        })
        .collect()
}

#[test]
fn eager_layout_is_limited_to_small_documents() {
    assert!(should_eagerly_measure_rows(MAX_EAGER_LAYOUT_ROWS));
    assert!(!should_eagerly_measure_rows(MAX_EAGER_LAYOUT_ROWS + 1));
}

#[gpui::test]
fn incremental_document_replacement_preserves_list_and_fold_state(cx: &mut gpui::TestAppContext) {
    let source = (0..400)
        .map(|index| format!("* Heading {index}\nbody {index}\n"))
        .collect::<String>();
    let mut buffer = crate::document::DocumentBuffer::from_utf8(source.into_bytes()).unwrap();
    let before = buffer.snapshot();
    let previous =
        super::loading::derive_preview(std::path::PathBuf::from("panel-state.org"), before.clone());
    let folded = previous
        .blocks
        .nodes()
        .iter()
        .position(|block| {
            matches!(block.kind, crate::org_syntax::BlockKind::Heading { .. })
                && previous.text.copy_range(block.content) == "Heading 200"
        })
        .unwrap() as crate::org_syntax::BlockId;
    let panel = cx.new(|_| super::PreviewPanel::new(std::sync::Arc::new(previous), 80.0));
    panel.update(cx, |panel, _| {
        panel.toggle_fold(folded);
        panel.list_state().scrollbar_drag_started();
    });

    let edit = before
        .copy_range(ByteRange::new(0, before.len_bytes()))
        .find("body 100")
        .unwrap() as u64
        + 5;
    let delta = buffer
        .commit(EditTransaction::new(
            before.revision(),
            vec![TextEdit::new(ByteRange::new(edit, edit + 3), "one hundred")],
        ))
        .unwrap();
    let next = cx.read(|cx| {
        let previous = panel.read(cx).document().clone();
        super::loading::derive_preview_incremental(
            std::path::PathBuf::from("panel-state.org"),
            buffer.snapshot(),
            Some(&previous),
            &[delta],
        )
    });
    panel.update(cx, |panel, cx| {
        panel.replace_document(std::sync::Arc::new(next), cx);
        assert!(panel.list_state().is_scrollbar_dragging());
        assert_eq!(panel.fold_markers().len(), 1);
        assert!(panel.fold_markers().contains(&folded));
    });
}

#[gpui::test]
fn right_preview_toggle_preserves_editor_state_and_publishes_only_latest_revision(
    cx: &mut gpui::TestAppContext,
) {
    let session = crate::document::DocumentSession::from_utf8(
        std::path::PathBuf::from("phase-d.org"),
        b"* Heading\nbody\n".to_vec(),
    )
    .unwrap();
    let workspace = cx.new(|_| super::WorkspaceWindow::with_right_preview(false));
    workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(
            0,
            Ok(super::WorkspaceLoadedDocument::Source(session)),
            cx,
        ));
    });
    let (session, editor) = cx.read(|cx| {
        let ready = workspace.read(cx).state.ready().unwrap();
        (ready.session.clone(), ready.editor.clone())
    });
    editor.update(cx, |editor, cx| {
        editor.set_selection(Selection::caret(ByteOffset(2)), cx)
    });
    let original = session.read_with(cx, |session, _| session.snapshot());

    workspace.update(cx, |workspace, cx| {
        workspace.toggle_right_preview(cx);
        assert!(workspace.preview_panel().is_none());
        assert!(
            workspace
                .document_status_snapshot(crate::navigation::PaneId(1), cx)
                .is_some()
        );
        workspace.toggle_right_preview(cx);
    });

    workspace.update(cx, |workspace, cx| {
        workspace.toggle_right_preview(cx);
        workspace.toggle_soft_wrap(cx);
    });
    assert!(!cx.read(|cx| editor.read(cx).soft_wrap()));
    assert_eq!(
        session.read_with(cx, |session, _| session.revision()),
        original.revision()
    );
    assert_eq!(
        cx.read(|cx| editor.read(cx).selection()),
        Selection::caret(ByteOffset(2))
    );

    let revision = session.read_with(cx, |session, _| session.revision());
    let end = original.len_bytes();
    session.update(cx, |session, cx| {
        session
            .edit(
                SessionEdit::new(
                    EditTransaction::new(
                        revision,
                        vec![TextEdit::new(ByteRange::new(end, end), "latest")],
                    ),
                    Selection::caret(ByteOffset(end)),
                    Selection::caret(ByteOffset(end + 6)),
                    EditOrigin::Typing,
                ),
                cx,
            )
            .unwrap();
    });
    workspace.update(cx, |workspace, cx| workspace.schedule_derived_update(cx));
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(25));
    cx.run_until_parked();

    let latest_revision = session.read_with(cx, |session, _| session.revision());
    let panel_revision = cx.read(|cx| {
        workspace
            .read(cx)
            .preview_panel()
            .unwrap()
            .read(cx)
            .document()
            .revision
    });
    assert_eq!(panel_revision, latest_revision);

    workspace.update(cx, |workspace, cx| {
        assert!(workspace.document_view.editor_focused());
        workspace.focus_right_preview(cx);
        assert!(!workspace.document_view.editor_focused());
        assert!(
            !workspace
                .document_status_snapshot(crate::navigation::PaneId(1), cx)
                .unwrap()
                .uses_editor_viewport()
        );
        workspace.return_to_editor(cx);
        assert!(workspace.document_view.editor_focused());
        assert!(workspace.document_view.right_preview_open());
        assert!(
            workspace
                .document_status_snapshot(crate::navigation::PaneId(1), cx)
                .unwrap()
                .uses_editor_viewport()
        );
    });

    workspace.update(cx, |workspace, cx| {
        workspace.toggle_right_preview(cx);
        workspace.toggle_right_preview(cx);
    });
    assert_eq!(
        session.read_with(cx, |session, _| session.revision()),
        latest_revision
    );
    assert_eq!(
        cx.read(|cx| editor.read(cx).selection()),
        Selection::caret(ByteOffset(2))
    );
}

#[gpui::test]
fn opening_right_preview_preserves_ime_until_preview_receives_focus(cx: &mut gpui::TestAppContext) {
    let loaded = super::load_document(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/preview-basics.org"),
    )
    .unwrap();
    let window = cx.open_window(gpui::size(gpui::px(900.0), gpui::px(700.0)), |_, _| {
        super::WorkspaceWindow::with_right_preview(false)
    });
    let editor = window
        .update(cx, |workspace, _, cx| {
            assert!(workspace.apply_load_result(0, Ok(loaded), cx));
            workspace.state.ready().unwrap().editor.clone()
        })
        .unwrap();
    let root = window.entity(cx).unwrap();
    cx.update(|cx| {
        cx.with_window(root.entity_id(), |window, cx| {
            editor.update(cx, |editor, cx| {
                <crate::editor::SourceEditor as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                    editor,
                    None,
                    "ni",
                    Some(2..2),
                    window,
                    cx,
                );
            });
        })
        .unwrap();
    });
    assert!(cx.read(|cx| editor.read(cx).has_active_composition()));

    window
        .update(cx, |workspace, _, cx| workspace.toggle_right_preview(cx))
        .unwrap();
    assert!(cx.read(|cx| editor.read(cx).has_active_composition()));

    window
        .update(cx, |workspace, _, cx| workspace.focus_right_preview(cx))
        .unwrap();
    assert!(!cx.read(|cx| editor.read(cx).has_active_composition()));
}

fn simulate_next_frame<V: gpui::Render + 'static>(
    window: &gpui::WindowHandle<V>,
    cx: &mut gpui::TestAppContext,
) -> usize {
    let root = window.entity(cx).unwrap();
    let callback_count = cx.update(|cx| {
        cx.with_window(root.entity_id(), |window, cx| {
            window.simulate_next_frame(cx)
        })
        .unwrap()
    });
    cx.run_until_parked();
    callback_count
}

fn finish_fold_animation<V: gpui::Render + 'static>(
    window: &gpui::WindowHandle<V>,
    cx: &mut gpui::TestAppContext,
) {
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION + std::time::Duration::from_millis(20));
    assert!(simulate_next_frame(window, cx) > 0);
    assert!(simulate_next_frame(window, cx) > 0);
}

fn ready_document(
    app: &super::WorkspaceWindow,
    cx: &gpui::App,
) -> std::sync::Arc<super::PreviewSnapshot> {
    match &app.state {
        super::PreviewLoadState::Ready { document } => document
            .panel
            .as_ref()
            .expect("preview panel should be published")
            .read(cx)
            .document()
            .clone(),
        _ => panic!("document should be ready"),
    }
}

fn apply_load_result(
    app: &mut super::WorkspaceWindow,
    generation: u64,
    result: Result<super::LoadedDocument, (std::path::PathBuf, String)>,
    cx: &mut gpui::TestAppContext,
) -> bool {
    cx.update(|cx| app.apply_load_result(generation, result, cx))
}

fn cycle_panel_global_visibility(app: &super::WorkspaceWindow, cx: &mut gpui::App) {
    app.preview_panel()
        .unwrap()
        .update(cx, |panel, _| panel.cycle_global_visibility());
}

fn toggle_panel_fold(
    app: &super::WorkspaceWindow,
    block_id: crate::org_syntax::BlockId,
    cx: &mut gpui::App,
) {
    app.preview_panel()
        .unwrap()
        .update(cx, |panel, _| panel.toggle_fold(block_id));
}

fn toggle_panel_fold_animated(
    app: &super::WorkspaceWindow,
    block_id: crate::org_syntax::BlockId,
    viewport_height: f32,
    available_width: f32,
    window: Option<&mut gpui::Window>,
    cx: &mut gpui::App,
) {
    app.preview_panel().unwrap().update(cx, |panel, cx| {
        panel.toggle_fold_animated(block_id, viewport_height, available_width, window, cx);
    });
}

fn heading_id(
    document: &super::PreviewSnapshot,
    level: u16,
    ordinal: usize,
) -> crate::org_syntax::BlockId {
    document
        .blocks
        .nodes()
        .iter()
        .enumerate()
        .filter(|(_, block)| {
            matches!(block.kind, crate::org_syntax::BlockKind::Heading { level: actual } if actual == level)
        })
        .nth(ordinal)
        .map(|(index, _)| index as crate::org_syntax::BlockId)
        .expect("requested heading should exist")
}

#[test]
fn stale_generations_are_rejected() {
    assert!(accept_generation(7, 7));
    assert!(!accept_generation(8, 7));
}

#[test]
fn shift_tab_dispatches_the_global_visibility_cycle() {
    let (commands, mut keyboard, context) = preview_input();
    let expected = commands.key(GLOBAL_VISIBILITY_CYCLE_COMMAND).unwrap();
    assert_eq!(
        keyboard.route(KeyStroke::new("tab", false, false, true, false), context),
        EmacsOutcome::Command {
            command: expected,
            prefix: crate::command::PrefixArgument::None,
        }
    );
}

#[test]
fn sidebar_keymap_activates_both_sidebar_and_dired_contexts() {
    let mut app = super::WorkspaceWindow::with_right_preview(true);
    app.install_sidebar_keymap();
    let contexts = super::built_in_contexts();
    assert!(app.key_context.contains(contexts.key("workspace").unwrap()));
    assert!(app.key_context.contains(contexts.key("sidebar").unwrap()));
    assert!(app.key_context.contains(contexts.key("dired").unwrap()));
    assert!(!app.key_context.contains(contexts.key("preview").unwrap()));
}

#[test]
fn source_keymap_passes_text_keys_to_the_editor() {
    let mut app = super::WorkspaceWindow::with_right_preview(false);
    for key in ["g", "q", "space", "backspace"] {
        assert!(
            matches!(
                app.keyboard.route(
                    KeyStroke::new(key, false, false, false, false),
                    app.key_context,
                ),
                EmacsOutcome::PassThrough | EmacsOutcome::Undefined
            ),
            "{key} must remain available to the editor"
        );
    }
    assert!(matches!(
        app.keyboard.route(
            KeyStroke::new("tab", false, false, true, false),
            app.key_context,
        ),
        EmacsOutcome::PassThrough | EmacsOutcome::Undefined
    ));
    let contexts = super::built_in_contexts();
    assert!(app.key_context.contains(contexts.key("editor").unwrap()));
    assert!(!app.key_context.contains(contexts.key("preview").unwrap()));
}

#[gpui::test]
fn editor_only_workspace_load_does_not_build_a_hidden_preview(cx: &mut gpui::TestAppContext) {
    let path =
        std::env::temp_dir().join(format!("org-studio-source-load-{}.org", std::process::id()));
    std::fs::write(&path, "* Heading\nbody\n").unwrap();
    let loaded = super::load_workspace_document(path.clone(), false).unwrap();
    let _ = std::fs::remove_file(&path);
    let mut app = super::WorkspaceWindow::with_right_preview(false);
    app.generation = 1;
    assert!(cx.update(|cx| app.apply_load_result(1, Ok(loaded), cx)));
    assert!(app.document_session().is_some());
    assert!(app.preview_panel().is_none());
}

#[gpui::test]
fn opening_preview_while_a_source_load_finishes_schedules_the_new_document(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::path::PathBuf::from("open-during-load.org");
    let session = crate::document::DocumentSession::from_utf8(
        path.clone(),
        b"* Current document\nbody\n".to_vec(),
    )
    .unwrap();
    let workspace = cx.new(|_| super::WorkspaceWindow::with_right_preview(false));
    workspace.update(cx, |workspace, cx| {
        let generation = workspace.begin_open(path, std::time::Instant::now());
        workspace.toggle_right_preview(cx);
        assert!(workspace.apply_load_result(
            generation,
            Ok(super::WorkspaceLoadedDocument::Source(session)),
            cx,
        ));
        workspace.reconcile_derived_preview(cx);
    });

    cx.executor()
        .advance_clock(std::time::Duration::from_millis(25));
    cx.run_until_parked();

    cx.read(|cx| {
        let workspace = workspace.read(cx);
        let ready = workspace.state.ready().unwrap();
        let panel = ready
            .panel
            .as_ref()
            .expect("right preview should be published");
        assert_eq!(
            panel.read(cx).document().revision,
            ready.session.read(cx).revision()
        );
    });
}

#[gpui::test]
fn closing_preview_releases_projection_and_rejects_a_completed_preview_load(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-close-preview-{}.org",
        std::process::id()
    ));
    std::fs::write(&path, "* Heading\nbody\n").unwrap();
    let loaded = super::load_document(path.clone()).unwrap();
    let late_loaded = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let workspace = cx.new(|_| super::WorkspaceWindow::with_right_preview(true));

    workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(0, Ok(loaded), cx));
        assert!(workspace.preview_panel().is_some());
        workspace.toggle_right_preview(cx);
        assert!(workspace.preview_panel().is_none());
        assert_eq!(workspace.derived.published, None);
        assert!(
            workspace
                .derived
                .pending
                .lock()
                .expect("derived request slot")
                .is_none()
        );

        workspace.generation = 1;
        assert!(workspace.apply_load_result(1, Ok(late_loaded), cx));
        assert!(workspace.preview_panel().is_none());
        assert_eq!(workspace.derived.published, None);
    });
}

#[gpui::test]
fn lagging_preview_status_falls_back_to_one_coherent_editor_snapshot(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-lagging-preview-status-{}.org",
        std::process::id()
    ));
    std::fs::write(&path, "* Heading\nbody\n").unwrap();
    let loaded = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let workspace = cx.new(|_| super::WorkspaceWindow::with_right_preview(true));
    let session = workspace.update(cx, |workspace, cx| {
        assert!(workspace.apply_load_result(0, Ok(loaded), cx));
        workspace.focus_right_preview(cx);
        workspace.document_session().unwrap().clone()
    });
    session.update(cx, |session, cx| {
        let end = session.snapshot().len_bytes();
        session
            .edit(
                SessionEdit::new(
                    EditTransaction::new(
                        session.revision(),
                        vec![TextEdit::new(ByteRange::new(end, end), "new")],
                    ),
                    Selection::caret(ByteOffset(end)),
                    Selection::caret(ByteOffset(end + 3)),
                    EditOrigin::Typing,
                ),
                cx,
            )
            .unwrap();
    });

    cx.read(|cx| {
        let status = workspace
            .read(cx)
            .document_status_snapshot(crate::navigation::PaneId(1), cx)
            .unwrap();
        assert!(status.uses_editor_viewport());
    });
}

#[gpui::test]
fn export_source_uses_the_live_edited_session(cx: &mut gpui::TestAppContext) {
    let path =
        std::env::temp_dir().join(format!("org-studio-live-export-{}.org", std::process::id()));
    std::fs::write(&path, "old").unwrap();
    let loaded = super::load_workspace_document(path.clone(), false).unwrap();
    let _ = std::fs::remove_file(&path);
    let mut app = super::WorkspaceWindow::with_right_preview(false);
    app.generation = 1;
    assert!(cx.update(|cx| app.apply_load_result(1, Ok(loaded), cx)));
    let session = app.document_session().unwrap().clone();
    cx.update(|cx| {
        session.update(cx, |session, cx| {
            session
                .edit(
                    SessionEdit::new(
                        EditTransaction::new(
                            session.revision(),
                            vec![TextEdit::new(ByteRange::new(0, 3), "new")],
                        ),
                        Selection::new(ByteOffset(0), ByteOffset(3)),
                        Selection::caret(ByteOffset(3)),
                        EditOrigin::Other,
                    ),
                    cx,
                )
                .unwrap();
        });
    });
    let (snapshot, export_path, format) = cx.update(|cx| app.current_export_source(cx).unwrap());
    assert_eq!(snapshot.copy_range(ByteRange::new(0, 3)), "new");
    assert_eq!(export_path, path);
    assert_eq!(format, super::DocumentFormat::Org);
}

#[gpui::test]
fn unchanged_viewport_does_not_cancel_an_active_sidebar_resize(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-sidebar-resize-{}.org",
        std::process::id()
    ));
    std::fs::write(&path, "* Heading\nbody\n").unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let window = cx.open_window(gpui::size(gpui::px(900.0), gpui::px(700.0)), |_, _| {
        super::WorkspaceWindow::with_right_preview(true)
    });
    window
        .update(cx, |app, _, cx| {
            app.generation = 1;
            assert!(app.apply_load_result(1, Ok(document), cx));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |app, _, cx| {
            app.begin_sidebar_resize(240.0, 900.0, cx);
            assert!(app.file_manager.is_resizing_sidebar());
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |app, _, _| {
            assert!(app.file_manager.is_resizing_sidebar());
        })
        .unwrap();
}

#[gpui::test]
fn app_global_visibility_cycle_updates_list_fold_and_minimap_projection_together(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-global-visibility-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "preamble\n* One\nbody\n** Child\nchild body\n* Two\nvisible\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let app = cx.new(|_| super::WorkspaceWindow::with_right_preview(true));
    app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document), cx));
        cycle_panel_global_visibility(app, cx);
        assert_eq!(
            app.preview_panel().unwrap().read(cx).global_visibility(),
            GlobalVisibility::Overview
        );
        assert_eq!(
            app.preview_panel().unwrap().read(cx).visible_rows().len(),
            2
        );
        assert_eq!(
            app.preview_panel().unwrap().read(cx).fold_markers().len(),
            2
        );
        assert_eq!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .item_count(),
            app.preview_panel().unwrap().read(cx).visible_rows().len()
        );
        cycle_panel_global_visibility(app, cx);
        assert_eq!(
            app.preview_panel().unwrap().read(cx).global_visibility(),
            GlobalVisibility::Contents
        );
        assert_eq!(
            app.preview_panel().unwrap().read(cx).visible_rows().len(),
            3
        );
        assert_eq!(
            app.preview_panel().unwrap().read(cx).fold_markers().len(),
            2
        );
        assert_eq!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .item_count(),
            app.preview_panel().unwrap().read(cx).visible_rows().len()
        );
        cycle_panel_global_visibility(app, cx);
        assert_eq!(
            app.preview_panel().unwrap().read(cx).global_visibility(),
            GlobalVisibility::All
        );
        assert!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .is_empty()
        );
        assert_eq!(
            app.preview_panel().unwrap().read(cx).visible_rows().len(),
            7
        );
        assert_eq!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .item_count(),
            app.preview_panel().unwrap().read(cx).visible_rows().len()
        );
    });
}

#[gpui::test]
fn shift_tab_animates_all_three_global_visibility_transitions(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-animated-global-visibility-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "preamble\n* One\nbody\n** Child\nchild body\n* Two\nvisible\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let window = cx.open_window(gpui::size(gpui::px(900.0), gpui::px(700.0)), |_, _| {
        super::WorkspaceWindow::with_right_preview(true)
    });
    window
        .update(cx, |app, _, cx| {
            app.generation = 1;
            assert!(app.apply_load_result(1, Ok(document), cx));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    for (visibility, direction) in [
        (GlobalVisibility::Overview, super::FoldDirection::Collapse),
        (GlobalVisibility::Contents, super::FoldDirection::Expand),
        (GlobalVisibility::All, super::FoldDirection::Expand),
    ] {
        window
            .update(cx, |app, window, cx| {
                app.cycle_global_visibility_animated(window, cx);
                assert_eq!(
                    app.preview_panel().unwrap().read(cx).global_visibility(),
                    visibility
                );
                let animation = app
                    .preview_panel()
                    .unwrap()
                    .read(cx)
                    .fold_animation()
                    .expect("every Shift-Tab state should animate");
                assert_eq!(animation.direction, direction);
                if visibility == GlobalVisibility::Overview {
                    assert!(animation.segments.len() > 1);
                }
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        assert!(simulate_next_frame(&window, cx) > 0);
        finish_fold_animation(&window, cx);
        window
            .update(cx, |app, _, cx| {
                assert!(
                    app.preview_panel()
                        .unwrap()
                        .read(cx)
                        .fold_animation()
                        .is_none()
                );
                assert_eq!(
                    app.preview_panel()
                        .unwrap()
                        .read(cx)
                        .list_state()
                        .item_count(),
                    app.preview_panel().unwrap().read(cx).visible_rows().len()
                );
            })
            .unwrap();
    }
}

#[gpui::test]
fn expanding_a_child_from_contents_does_not_collapse_its_parent(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-contents-child-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "preamble\n* One\nparent body\n** Child\nchild body\n* Two\nsecond body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let app = cx.new(|_| super::WorkspaceWindow::with_right_preview(true));
    app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document), cx));
        cycle_panel_global_visibility(app, cx);
        cycle_panel_global_visibility(app, cx);

        let document = ready_document(app, cx);
        let parent = heading_id(&document, 1, 0);
        let child = heading_id(&document, 2, 0);

        assert!(
            !app.preview_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&parent)
        );
        assert!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&child)
        );

        toggle_panel_fold(app, child, cx);
        assert_eq!(
            app.preview_panel().unwrap().read(cx).global_visibility(),
            GlobalVisibility::Contents
        );
        assert!(
            !app.preview_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&parent)
        );
        assert!(
            !app.preview_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&child)
        );
        assert_eq!(visible_source_lines(app, &document, cx), vec![2, 4, 5, 6]);

        toggle_panel_fold(app, child, cx);
        assert_eq!(
            app.preview_panel().unwrap().read(cx).global_visibility(),
            GlobalVisibility::Contents
        );
        assert_eq!(
            app.preview_panel().unwrap().read(cx).visible_rows().len(),
            3
        );
        assert!(
            !app.preview_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&parent)
        );
        assert!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&child)
        );

        cycle_panel_global_visibility(app, cx);
        assert_eq!(
            app.preview_panel().unwrap().read(cx).global_visibility(),
            GlobalVisibility::Overview
        );
        assert_eq!(
            app.preview_panel().unwrap().read(cx).visible_rows().len(),
            2
        );
    });
}

#[gpui::test]
fn clicking_a_second_level_heading_in_contents_never_folds_its_parent(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-contents-second-level-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "* Parent\nparent body\n** Child\nchild body\n*** Grandchild\ngrandchild body\n* Other\nother body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let app = cx.new(|_| super::WorkspaceWindow::with_right_preview(true));
    app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document), cx));
        cycle_panel_global_visibility(app, cx);
        cycle_panel_global_visibility(app, cx);

        let document = ready_document(app, cx);
        let parent = heading_id(&document, 1, 0);
        let child = heading_id(&document, 2, 0);

        assert_eq!(visible_source_lines(app, &document, cx), vec![1, 3, 5, 7]);
        assert!(
            !app.preview_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&parent)
        );

        toggle_panel_fold(app, child, cx);
        assert_eq!(visible_source_lines(app, &document, cx), vec![1, 3, 7]);
        assert!(
            !app.preview_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&parent)
        );
        assert!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&child)
        );

        toggle_panel_fold(app, child, cx);
        assert_eq!(
            visible_source_lines(app, &document, cx),
            vec![1, 3, 4, 5, 7]
        );
        assert!(
            !app.preview_panel()
                .unwrap()
                .read(cx)
                .fold_markers()
                .contains(&parent)
        );
    });
}

#[gpui::test]
fn heading_click_cycles_only_its_subtree_through_official_local_states(
    cx: &mut gpui::TestAppContext,
) {
    let path =
        std::env::temp_dir().join(format!("org-studio-local-cycle-{}.org", std::process::id()));
    std::fs::write(
        &path,
        "* Parent\nparent body\n** Child\nchild body\n** Sibling\nsibling body\n* Other\nother body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let app = cx.new(|_| super::WorkspaceWindow::with_right_preview(true));
    app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document), cx));
        cycle_panel_global_visibility(app, cx);
        let document = ready_document(app, cx);
        let parent = heading_id(&document, 1, 0);

        toggle_panel_fold(app, parent, cx);
        assert_eq!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .local_cycle_continuation(),
            Some((parent, super::LocalVisibility::Children))
        );
        assert_eq!(
            visible_source_lines(app, &document, cx),
            vec![1, 2, 3, 5, 7]
        );

        toggle_panel_fold(app, parent, cx);
        assert_eq!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .local_cycle_continuation(),
            Some((parent, super::LocalVisibility::Subtree))
        );
        assert_eq!(
            visible_source_lines(app, &document, cx),
            vec![1, 2, 3, 4, 5, 6, 7]
        );

        toggle_panel_fold(app, parent, cx);
        assert_eq!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .local_cycle_continuation(),
            Some((parent, super::LocalVisibility::Folded))
        );
        assert_eq!(visible_source_lines(app, &document, cx), vec![1, 7]);
    });
}

#[gpui::test]
fn animated_collapse_inserts_one_flow_shell_and_fast_reclick_finishes_it(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-animated-local-cycle-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "* Parent\nparent body\n** Child\nchild body\n* Other\nother body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let app = cx.new(|_| super::WorkspaceWindow::with_right_preview(true));

    app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document), cx));
        let document = ready_document(app, cx);
        let parent = heading_id(&document, 1, 0);

        toggle_panel_fold_animated(app, parent, 700.0, 790.0, None, cx);
        assert_eq!(visible_source_lines(app, &document, cx), vec![1, 5, 6]);
        assert_eq!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .map(|animation| animation.segments[0].transition_index),
            Some(1)
        );
        assert_eq!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .item_count(),
            app.preview_panel().unwrap().read(cx).visible_rows().len() + 1
        );

        toggle_panel_fold_animated(app, parent, 700.0, 790.0, None, cx);
        assert_eq!(
            visible_source_lines(app, &document, cx),
            vec![1, 2, 3, 5, 6]
        );
        let expansion = app
            .preview_panel()
            .unwrap()
            .read(cx)
            .fold_animation()
            .unwrap();
        assert!(matches!(expansion.direction, super::FoldDirection::Expand));
        assert_eq!(expansion.segments[0].target_len, 2);
        assert_eq!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .item_count(),
            app.preview_panel().unwrap().read(cx).visible_rows().len() - 1
        );
        app.preview_panel()
            .unwrap()
            .update(cx, |panel, _| panel.discard_fold_animation());
    });

    app.update(cx, |app, cx| {
        assert!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .is_none()
        );
        assert_eq!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .item_count(),
            app.preview_panel().unwrap().read(cx).visible_rows().len()
        );
    });
}

#[gpui::test]
fn reduced_motion_applies_local_fold_without_a_delayed_projection(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let path = std::env::temp_dir().join(format!(
        "org-studio-reduced-motion-fold-{}.org",
        std::process::id()
    ));
    std::fs::write(&path, "* Parent\nbody\n** Child\nchild body\n").unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let app = cx.new(|_| super::WorkspaceWindow::with_right_preview(true));

    app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document), cx));
        let document = ready_document(app, cx);
        let parent = heading_id(&document, 1, 0);

        toggle_panel_fold_animated(app, parent, 700.0, 790.0, None, cx);
        assert_eq!(visible_source_lines(app, &document, cx), vec![1]);
        assert!(
            app.preview_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .is_none()
        );
    });
}

#[gpui::test]
fn measured_fold_travel_renders_through_real_list_animation_frames(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-rendered-fold-travel-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "* Parent\nparent body\n** Child One\nchild one body\n** Child Two\nchild two body\n* Other\nother body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let window = cx.open_window(gpui::size(gpui::px(900.0), gpui::px(700.0)), |_, _| {
        super::WorkspaceWindow::with_right_preview(true)
    });
    window
        .update(cx, |app, _window, cx| {
            app.generation = 1;
            assert!(app.apply_load_result(1, Ok(document), cx));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |app, window, cx| {
            let document = ready_document(app, cx);
            let parent = heading_id(&document, 1, 0);
            let first_hidden = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(1)
                .unwrap();
            let following = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(6)
                .unwrap();
            let expected_distance = f32::from(following.top() - first_hidden.top());

            toggle_panel_fold_animated(app, parent, 700.0, 790.0, Some(window), cx);
            let animation = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            let shell = &animation.segments[0];
            assert_eq!(shell.transition_index, 1);
            assert!((shell.distance - expected_distance).abs() < 0.01);
            assert_eq!(
                app.preview_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .item_count(),
                app.preview_panel().unwrap().read(cx).visible_rows().len() + 1
            );
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    // Even if preparing the first Shell frame exceeds the whole nominal duration, animation
    // time starts only after that frame is presented.
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION + std::time::Duration::from_millis(20));
    assert!(simulate_next_frame(&window, cx) > 0);
    let first_gap = window
        .update(cx, |app, _, cx| {
            let animation = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            assert_eq!(animation.progress, 0.0);
            assert!(animation.started_at.is_some());
            let heading = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap();
            let peer = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(2)
                .unwrap();
            f32::from(peer.top() - heading.bottom())
        })
        .unwrap();
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION / 3);
    assert!(simulate_next_frame(&window, cx) > 0);
    let later_gap = window
        .update(cx, |app, _, cx| {
            let animation = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            assert!(animation.progress > 0.0);
            let heading = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap();
            let shell = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(1)
                .unwrap();
            let peer = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(2)
                .unwrap();
            assert!((f32::from(shell.top() - heading.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - shell.bottom())).abs() < 0.01);
            let gap = f32::from(peer.top() - heading.bottom());
            let expected_gap = animation.segments[0].distance * (1.0 - animation.progress);
            assert!(
                (gap - expected_gap).abs() < 1.0,
                "gap={gap} expected_gap={expected_gap} progress={}",
                animation.progress
            );
            gap
        })
        .unwrap();
    assert!(later_gap < first_gap);

    finish_fold_animation(&window, cx);
    window
        .update(cx, |app, _, cx| {
            assert!(
                app.preview_panel()
                    .unwrap()
                    .read(cx)
                    .fold_animation()
                    .is_none()
            );
            assert_eq!(
                app.preview_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .item_count(),
                app.preview_panel().unwrap().read(cx).visible_rows().len()
            );
        })
        .unwrap();

    window
        .update(cx, |app, window, cx| {
            let document = ready_document(app, cx);
            let parent = heading_id(&document, 1, 0);
            toggle_panel_fold_animated(app, parent, 700.0, 790.0, Some(window), cx);
            let animation = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            assert!(matches!(animation.direction, super::FoldDirection::Expand));
            assert_eq!(animation.segments[0].transition_index, 1);
            assert_eq!(animation.segments[0].target_len, 3);
            assert_eq!(
                app.preview_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .item_count(),
                app.preview_panel().unwrap().read(cx).visible_rows().len() - 2
            );
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    assert!(simulate_next_frame(&window, cx) > 0);
    let first_gap = window
        .update(cx, |app, _, cx| {
            let animation = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            assert_eq!(animation.progress, 0.0);
            let heading = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap();
            let shell = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(1)
                .unwrap();
            let peer = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(2)
                .unwrap();
            assert!((f32::from(shell.top() - heading.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - shell.bottom())).abs() < 0.01);
            f32::from(peer.top() - heading.bottom())
        })
        .unwrap();
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION / 3);
    assert!(simulate_next_frame(&window, cx) > 0);
    let later_gap = window
        .update(cx, |app, _, cx| {
            let animation = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            assert!(animation.progress > 0.0);
            let heading = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap();
            let shell = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(1)
                .unwrap();
            let peer = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(2)
                .unwrap();
            assert!((f32::from(shell.top() - heading.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - shell.bottom())).abs() < 0.01);
            let gap = f32::from(peer.top() - heading.bottom());
            let expected_gap = animation.segments[0].distance * animation.progress;
            assert!(
                (gap - expected_gap).abs() < 1.0,
                "gap={gap} expected_gap={expected_gap} progress={}",
                animation.progress
            );
            gap
        })
        .unwrap();
    assert!(later_gap > first_gap);

    finish_fold_animation(&window, cx);
    window
        .update(cx, |app, _, cx| {
            assert!(
                app.preview_panel()
                    .unwrap()
                    .read(cx)
                    .fold_animation()
                    .is_none()
            );
            assert_eq!(
                app.preview_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .item_count(),
                app.preview_panel().unwrap().read(cx).visible_rows().len()
            );
            let heading = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap();
            let peer = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(4)
                .unwrap();
            assert!(peer.top() > heading.bottom());
        })
        .unwrap();

    window
        .update(cx, |app, window, cx| {
            let document = ready_document(app, cx);
            let parent = heading_id(&document, 1, 0);
            toggle_panel_fold_animated(app, parent, 700.0, 790.0, Some(window), cx);
            let animation = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            assert!(matches!(animation.direction, super::FoldDirection::Expand));
            assert_eq!(animation.segments.len(), 2);
            assert_eq!(animation.segments[0].transition_index, 3);
            assert_eq!(animation.segments[1].transition_index, 5);
            assert!(animation.segments.iter().all(|shell| shell.target_len == 1));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    assert!(simulate_next_frame(&window, cx) > 0);
    window
        .update(cx, |app, _, cx| {
            let first_shell = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(3)
                .unwrap();
            let second_shell = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(5)
                .unwrap();
            assert!(f32::from(first_shell.size.height).abs() < 0.01);
            assert!(f32::from(second_shell.size.height).abs() < 0.01);
        })
        .unwrap();
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION / 3);
    assert!(simulate_next_frame(&window, cx) > 0);
    window
        .update(cx, |app, _, cx| {
            let animation = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            assert!(animation.progress > 0.0);
            let first_child = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(2)
                .unwrap();
            let first_shell = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(3)
                .unwrap();
            let second_child = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(4)
                .unwrap();
            let second_shell = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(5)
                .unwrap();
            let peer = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(6)
                .unwrap();
            assert!((f32::from(first_shell.top() - first_child.bottom())).abs() < 0.01);
            assert!((f32::from(second_child.top() - first_shell.bottom())).abs() < 0.01);
            assert!((f32::from(second_shell.top() - second_child.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - second_shell.bottom())).abs() < 0.01);
            for (index, shell) in animation.segments.iter().enumerate() {
                let bounds = app
                    .preview_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .bounds_for_item(shell.transition_index)
                    .unwrap();
                let expected = shell.distance * animation.progress;
                assert!(
                    (f32::from(bounds.size.height) - expected).abs() < 1.0,
                    "shell {index} height did not share the expansion progress"
                );
            }
        })
        .unwrap();
    finish_fold_animation(&window, cx);
    window
        .update(cx, |app, _, cx| {
            assert!(
                app.preview_panel()
                    .unwrap()
                    .read(cx)
                    .fold_animation()
                    .is_none()
            );
            assert_eq!(
                app.preview_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .item_count(),
                app.preview_panel().unwrap().read(cx).visible_rows().len()
            );
            let first_child_body = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(3)
                .unwrap();
            let second_child = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(4)
                .unwrap();
            let second_child_body = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(5)
                .unwrap();
            let peer = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(6)
                .unwrap();
            assert!(first_child_body.size.height > gpui::px(0.0));
            assert!(second_child.top() >= first_child_body.bottom());
            assert!(second_child_body.size.height > gpui::px(0.0));
            assert!(peer.top() >= second_child_body.bottom());
        })
        .unwrap();
}

#[gpui::test]
fn collapse_keeps_an_offscreen_peer_behind_a_bounded_flow_shell(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-offscreen-fold-peer-{}.org",
        std::process::id()
    ));
    let mut source = String::from("* Parent\n");
    for index in 0..48 {
        source.push_str(&format!("** Child {index}\nbody {index}\n"));
    }
    source.push_str("* Other\nother body\n");
    std::fs::write(&path, source).unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let window = cx.open_window(gpui::size(gpui::px(900.0), gpui::px(320.0)), |_, _| {
        super::WorkspaceWindow::with_right_preview(true)
    });
    window
        .update(cx, |app, _window, cx| {
            app.generation = 1;
            assert!(app.apply_load_result(1, Ok(document), cx));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |app, window, cx| {
            let document = ready_document(app, cx);
            let parent = heading_id(&document, 1, 0);
            let peer = heading_id(&document, 1, 1);
            let peer_row = document
                .projection
                .rows
                .iter()
                .position(|row| row.block_id == peer)
                .unwrap();
            let old_peer_position = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .visible_rows()
                .binary_search(&peer_row)
                .unwrap();
            assert!(
                app.preview_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .bounds_for_item(old_peer_position)
                    .unwrap()
                    .top()
                    > gpui::px(320.0)
            );

            toggle_panel_fold_animated(app, parent, 320.0, 790.0, Some(window), cx);
            let animation = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .fold_animation()
                .unwrap();
            let shell = &animation.segments[0];
            assert_eq!(shell.transition_index, 1);
            assert!(shell.distance <= 320.0);
            assert!(shell.rendered_rows.len() < 20);
            assert_eq!(
                app.preview_panel().unwrap().read(cx).visible_rows()[1],
                peer_row
            );
            assert_eq!(
                app.preview_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .item_count(),
                app.preview_panel().unwrap().read(cx).visible_rows().len() + 1
            );
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    assert!(simulate_next_frame(&window, cx) > 0);

    window
        .update(cx, |app, _, cx| {
            let heading = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap();
            let shell = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(1)
                .unwrap();
            let peer = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(2)
                .unwrap();
            assert!((f32::from(shell.top() - heading.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - shell.bottom())).abs() < 0.01);
            assert!(f32::from(peer.top() - heading.bottom()) > 24.0);
            assert!(
                app.preview_panel()
                    .unwrap()
                    .read(cx)
                    .fold_animation()
                    .is_some()
            );
        })
        .unwrap();
    finish_fold_animation(&window, cx);
    window
        .update(cx, |app, _, cx| {
            let heading = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap();
            let peer = app
                .preview_panel()
                .unwrap()
                .read(cx)
                .list_state()
                .bounds_for_item(1)
                .unwrap();
            assert!((f32::from(peer.top() - heading.bottom())).abs() < 0.01);
            assert_eq!(
                app.preview_panel()
                    .unwrap()
                    .read(cx)
                    .list_state()
                    .item_count(),
                app.preview_panel().unwrap().read(cx).visible_rows().len()
            );
            assert!(
                app.preview_panel()
                    .unwrap()
                    .read(cx)
                    .fold_animation()
                    .is_none()
            );
        })
        .unwrap();
}

#[gpui::test]
fn open_failure_retains_previous_session_and_rejects_stale_completion(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-reload-state-{}.org",
        std::process::id()
    ));
    std::fs::write(&path, "* retained\n").unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(&path);

    let mut app = super::WorkspaceWindow::with_right_preview(true);
    app.generation = 1;
    assert!(apply_load_result(&mut app, 1, Ok(document), cx));
    assert!(app.state.ready().is_some());
    let session = app.document_session().unwrap().clone();

    app.generation = 2;
    assert!(apply_load_result(
        &mut app,
        2,
        Err((path.clone(), "reload failed".to_owned())),
        cx,
    ));
    assert!(matches!(app.state, super::PreviewLoadState::Failed { .. }));
    assert!(app.state.ready().is_some());
    assert_eq!(app.document_session(), Some(&session));

    let stale_path = path.with_extension("md");
    std::fs::write(&stale_path, "# stale\n").unwrap();
    let stale = super::load_document(stale_path.clone()).unwrap();
    let _ = std::fs::remove_file(stale_path);
    assert!(!apply_load_result(&mut app, 1, Ok(stale), cx));
    assert!(matches!(app.state, super::PreviewLoadState::Failed { .. }));
    assert!(app.state.ready().is_some());
    assert_eq!(app.document_session(), Some(&session));
}

#[gpui::test]
fn reload_keeps_session_identity_and_publishes_a_coherent_preview(cx: &mut gpui::TestAppContext) {
    use std::sync::{Arc, Mutex};

    let path = std::env::temp_dir().join(format!(
        "org-studio-stable-session-reload-{}.org",
        std::process::id()
    ));
    std::fs::write(&path, "* Before\nold\n").unwrap();
    let loaded = super::load_document(path.clone()).unwrap();
    let app = cx.new(|_| super::WorkspaceWindow::with_right_preview(true));
    let (session_before, document_id, panel, events) = app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(loaded), cx));
        let session = app.document_session().unwrap().clone();
        let panel = app.preview_panel().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        panel.update(cx, |_, cx| {
            let events = events.clone();
            cx.subscribe_self(move |_, event, _| events.lock().unwrap().push(*event))
                .detach();
        });
        let document_id = session.read(cx).id();
        (session, document_id, panel, events)
    });

    std::fs::write(&path, "* After\nnew\n").unwrap();
    app.update(cx, |app, cx| app.reload_current(cx));
    cx.run_until_parked();

    app.update(cx, |app, cx| {
        let session_after = app.document_session().unwrap();
        assert_eq!(session_after, &session_before);
        assert_eq!(session_after.read(cx).id(), document_id);
        assert_eq!(
            session_after.read(cx).revision(),
            crate::document::Revision(1)
        );
        let preview = panel.read(cx).document();
        assert_eq!(preview.document_id, document_id);
        assert_eq!(preview.revision, crate::document::Revision(1));
        assert_eq!(
            preview
                .text
                .copy_range(crate::document::ByteRange::new(0, preview.text.len_bytes())),
            "* After\nnew\n"
        );
    });
    assert_eq!(
        events.lock().unwrap().as_slice(),
        [super::DerivedEvent::Published {
            document_id,
            revision: crate::document::Revision(1),
        }]
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn dired_help_lists_every_command_and_groups_alias_keys() {
    let (commands, _, _) = preview_input();
    let items = dired_command_items(&commands);
    let labels = items
        .iter()
        .map(|(keys, title)| (keys.as_ref(), title.as_ref()))
        .collect::<Vec<_>>();

    assert_eq!(items.len(), 25);
    assert!(labels.contains(&("n / j", "Next Line")));
    assert!(labels.contains(&("p / k", "Previous Line")));
    assert!(labels.contains(&("^ / h", "Up Directory")));
    assert!(labels.contains(&("RET / l", "Open")));
    assert!(labels.contains(&("H", "History Back")));
    assert!(labels.contains(&("L", "History Forward")));
    assert!(labels.contains(&("g", "Refresh Directory")));
    assert!(labels.contains(&("C-x d", "Open Dired")));
    assert!(labels.contains(&("C-x C-b", "Home")));
    assert!(labels.contains(&("C-x C-d", "Toggle Sidebar")));
    assert!(labels.contains(&("?", "Dired Help")));
    assert!(labels.contains(&("N", "Create File")));
    assert!(labels.contains(&("+", "Create Directory")));
    assert!(labels.contains(&("R", "Rename")));
    assert!(labels.contains(&("C", "Copy")));
    assert!(labels.contains(&("M", "Move")));
    assert!(labels.contains(&("D", "Move to Trash")));
    assert!(labels.contains(&("C-g", "Close command list")));
}

#[test]
fn loads_markdown_without_sending_it_through_the_org_parser() {
    let path = std::env::temp_dir().join(format!("org-studio-markdown-{}.md", std::process::id()));
    std::fs::write(&path, "# Markdown\n\n- **native** preview\n").unwrap();
    let document = super::load_document(path.clone()).unwrap().into_preview();
    let _ = std::fs::remove_file(path);

    assert_eq!(document.format, super::DocumentFormat::Markdown);
    assert!(document.blocks.nodes().is_empty());
    assert!(!document.markdown_blocks.is_empty());
    assert_eq!(document.projection.rows.len(), 3);
}

#[test]
fn markdown_parent_and_minimap_share_identical_display_runs() {
    let path = std::env::temp_dir().join(format!(
        "org-studio-shared-display-runs-{}.md",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "# **标题** and *italic*\n\n```rust\nlet answer = 42;\n```\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap().into_preview();
    let _ = std::fs::remove_file(path);
    let display_map = document.display_map.as_ref().unwrap();
    let heading_layout = display_map.layout(0);
    assert_eq!(heading_layout.font_size, 22.0);
    assert_eq!(heading_layout.line_height, 24.0);
    let blank_layout = display_map.layout(1);
    assert_eq!(blank_layout.fixed_height, Some(24.0));

    let heading =
        super::parse_document_inline(super::DocumentFormat::Markdown, "**标题** and *italic*");
    let heading_runs = display_map.runs(0);
    assert_eq!(heading_runs.text.as_ref(), heading.text);
    assert_eq!(heading_runs.inline_spans.as_ref(), heading.spans);

    let code_row = document
        .projection
        .rows
        .iter()
        .position(|row| {
            document
                .text
                .copy_range(row.source.range)
                .contains("answer")
        })
        .expect("code row");
    assert!(!display_map.runs(code_row).code_spans.is_empty());
    let code_layout = display_map.layout(code_row);
    assert_eq!(code_layout.padding_left, 16.0);
    assert_eq!(code_layout.padding_right, 16.0);
    assert_eq!(code_layout.min_height, 24.0);
}

#[test]
fn org_parent_and_minimap_share_identical_display_runs() {
    let path = std::env::temp_dir().join(format!(
        "org-studio-shared-display-runs-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "* *粗体* and /italic/\n\n#+begin_src rust\nlet n = 7;\n#+end_src\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap().into_preview();
    let _ = std::fs::remove_file(path);
    let display_map = document.display_map.as_ref().unwrap();
    let heading_layout = display_map.layout(0);
    assert_eq!(heading_layout.font_size, 22.0);
    assert_eq!(heading_layout.line_height, 24.0);
    let blank_layout = display_map.layout(1);
    assert_eq!(blank_layout.fixed_height, Some(24.0));
    let source = document
        .text
        .copy_range(document.projection.rows.get(0).unwrap().source.range)
        .trim_end_matches(['\r', '\n'])
        .to_owned();
    let expected = super::parse_document_inline(super::DocumentFormat::Org, &source);
    let heading_runs = display_map.runs(0);
    assert_eq!(heading_runs.text.as_ref(), expected.text);
    assert_eq!(heading_runs.inline_spans.as_ref(), expected.spans);

    let code_row = document
        .projection
        .rows
        .iter()
        .position(|row| document.text.copy_range(row.source.range).contains("let n"))
        .expect("code row");
    assert!(!display_map.runs(code_row).code_spans.is_empty());
    let code_layout = display_map.layout(code_row);
    assert_eq!(code_layout.padding_left, 16.0);
    assert_eq!(code_layout.padding_right, 16.0);
    assert_eq!(code_layout.line_height, 19.0);
    assert_eq!(code_layout.min_height, 24.0);
}
