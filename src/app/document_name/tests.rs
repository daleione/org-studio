use super::*;
use crate::{
    app::PaneSide,
    document::{ByteOffset, ByteRange, Selection, TextSnapshot},
};
use gpui::{EntityInputHandler, Modifiers, VisualTestContext};

fn new_draft(cx: &mut gpui::TestAppContext) -> (Entity<WorkspaceWindow>, &mut VisualTestContext) {
    cx.update(crate::editor::init);
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    cx.update(|window, app| w.update(app, |w, cx| w.new_draft(window, cx)));
    cx.run_until_parked();
    (w, cx)
}

fn open_name(w: &Entity<WorkspaceWindow>, cx: &mut VisualTestContext) {
    let title = cx.debug_bounds("document-titlebar-filename").unwrap();
    cx.simulate_click(title.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(w.read_with(cx, |w, _| w.document_name.session.is_some()));
}

fn load_saved(w: &Entity<WorkspaceWindow>, path: &std::path::Path, cx: &mut VisualTestContext) {
    let loaded = crate::preview::load_document(path.to_path_buf()).unwrap();
    w.update(cx, |w, cx| {
        assert!(w.apply_load_result(w.generation, Ok(loaded), cx))
    });
    cx.run_until_parked();
}

fn saved_file<'a>(
    cx: &'a mut gpui::TestAppContext,
    name: &str,
    body: &str,
) -> (
    tempfile::TempDir,
    Entity<WorkspaceWindow>,
    &'a mut VisualTestContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(name);
    std::fs::write(&path, body).unwrap();
    let (workspace, cx) = new_draft(cx);
    load_saved(&workspace, &path, cx);
    (directory, workspace, cx)
}

fn bind_save(cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "cmd-s",
            crate::preview::SaveDocument,
            None,
        )])
    });
}

#[gpui::test]
fn title_naming_keeps_the_document_editor_selection_and_undo(cx: &mut gpui::TestAppContext) {
    let (w, cx) = new_draft(cx);
    cx.simulate_input("* Heading\n\nBody");
    cx.run_until_parked();
    let (session, editor, revision, hint) = w.read_with(cx, |w, cx| {
        let session = w.document_session().unwrap().clone();
        let doc = session.read(cx);
        (
            session.clone(),
            w.editor(PaneSide::Left).unwrap(),
            doc.revision(),
            doc.syntax_path().to_path_buf(),
        )
    });
    editor.update(cx, |editor, cx| {
        editor.set_selection(Selection::caret(ByteOffset(3)), cx)
    });
    open_name(&w, cx);
    cx.simulate_input("项目笔记.org");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        assert!(w.document_name.session.is_none());
        assert_eq!(w.document_session().unwrap(), &session);
        assert_eq!(w.editor(PaneSide::Left), Some(editor.clone()));
        assert_eq!(editor.read(cx).selection(), Selection::caret(ByteOffset(3)));
        let doc = session.read(cx);
        assert_eq!(doc.display_name(), "项目笔记.org");
        assert_eq!(doc.syntax_path(), hint);
        assert_eq!(doc.revision(), revision);
        assert!(doc.is_dirty() && doc.file_path().is_none());
        assert!(w.window_title(cx).contains(w.buffer_text("草稿", "Draft")));
    });
    session.update(cx, |doc, cx| {
        doc.undo(cx).unwrap();
    });
    cx.run_until_parked();
    session.read_with(cx, |doc, _| assert_eq!(doc.snapshot().len_bytes(), 0));
    cx.simulate_input("Restored focus");
    cx.run_until_parked();
    session.read_with(cx, |doc, _| assert_eq!(doc.snapshot().len_bytes(), 14));
}

#[gpui::test]
fn cancelling_title_naming_never_changes_the_name_or_body(cx: &mut gpui::TestAppContext) {
    let (w, cx) = new_draft(cx);
    cx.update(|window, app| {
        app.intercept_keystrokes(WorkspaceWindow::intercept_fullscreen_escape)
            .detach();
        window.toggle_fullscreen();
    });
    cx.run_until_parked();
    let session = w.read_with(cx, |w, _| w.document_session().unwrap().clone());
    let name = session.read_with(cx, |doc, _| doc.display_name());
    open_name(&w, cx);
    cx.simulate_input("Discard this");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!w.read_with(cx, |w, _| w.document_name.is_open()));
    cx.update(|window, _| assert!(window.is_fullscreen()));
    session.read_with(cx, |doc, _| {
        assert_eq!(doc.display_name(), name);
        assert_eq!(doc.snapshot().len_bytes(), 0);
        assert!(!doc.is_dirty());
    });
}

#[gpui::test]
fn naming_rejects_invalid_names_and_format_changes(cx: &mut gpui::TestAppContext) {
    let (w, cx) = new_draft(cx);
    open_name(&w, cx);
    for name in ["   ", "folder/name", "notes.md"] {
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input(name);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        w.read_with(cx, |w, cx| {
            let prompt = w.document_name.session.as_ref().unwrap();
            assert!(prompt.error.is_some());
            assert!(prompt.input.read(cx).invalid);
        });
    }
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Notes.v2.ORG");
    let done = cx.debug_bounds("document-name-done").unwrap();
    cx.simulate_click(done.center(), Modifiers::default());
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        assert_eq!(
            w.document_session().unwrap().read(cx).display_name(),
            "Notes.v2.org"
        )
    });
}

#[gpui::test]
fn naming_ime_confirmation_is_not_submission(cx: &mut gpui::TestAppContext) {
    let (w, cx) = new_draft(cx);
    open_name(&w, cx);
    let input = w.read_with(cx, |w, _| {
        w.document_name.session.as_ref().unwrap().input.clone()
    });
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.replace_and_mark_text_in_range(None, "笔记", Some(2..2), window, cx);
        })
    });
    // The test platform has no IME. Check the command guard without inserting
    // a literal newline into marked text, then emulate the native commit.
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            assert!(!w.confirm_document_name(AfterRename::Nothing, window, cx))
        })
    });
    cx.update(|window, app| input.update(app, |input, cx| input.unmark_text(window, cx)));
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            assert!(!w.confirm_document_name(AfterRename::Nothing, window, cx))
        })
    });
    for _ in 0..2 {
        cx.update(|window, app| window.simulate_next_frame(app));
        cx.run_until_parked();
    }
    assert!(
        !input.read_with(cx, |input, _| input.is_composing()),
        "IME commit guard should clear after a real frame"
    );
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        assert!(
            w.document_name.session.is_none(),
            "prompt: {:?}",
            w.document_name.session.as_ref().map(|p| (
                p.error.clone(),
                p.input.read(cx).text.clone(),
                p.input.read(cx).is_composing()
            ))
        );
        assert_eq!(
            w.document_session().unwrap().read(cx).display_name(),
            "笔记.org"
        );
    });
}

#[gpui::test]
fn saving_from_name_input_keeps_the_name_when_save_is_cancelled(cx: &mut gpui::TestAppContext) {
    let (w, cx) = new_draft(cx);
    cx.update(|_, cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "cmd-s",
            crate::preview::SaveDocument,
            None,
        )])
    });
    cx.simulate_input("Saved body");
    open_name(&w, cx);
    cx.simulate_input("My notes");
    cx.simulate_keystrokes("cmd-s");
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        assert!(w.document_name.session.is_none());
        assert_eq!(
            w.document_session().unwrap().read(cx).display_name(),
            "My notes.org"
        );
        assert!(matches!(
            w.save.interaction,
            super::super::save::SaveInteraction::Prompt
        ));
    });
    cx.cx.simulate_path_prompt_response(|options| {
        assert!(options.directories && !options.files && !options.multiple);
        assert!(options.prompt.as_ref().unwrap().contains("My notes.org"));
        None
    });
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        let doc = w.document_session().unwrap().read(cx);
        assert_eq!(doc.display_name(), "My notes.org");
        assert!(doc.file_path().is_none());
        assert_eq!(
            doc.snapshot()
                .copy_range(ByteRange::new(0, doc.snapshot().len_bytes())),
            "Saved body"
        );
    });
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("My notes.org");
    let session = w.read_with(cx, |w, _| w.document_session().unwrap().clone());
    cx.simulate_keystrokes("cmd-s");
    cx.cx
        .simulate_path_prompt_response(|_| Some(vec![directory.path().to_path_buf()]));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "Saved body");
    w.read_with(cx, |w, cx| {
        assert_eq!(w.document_session().unwrap(), &session);
        assert!(!session.read(cx).is_draft());
        assert!(!w.window_title(cx).contains(w.buffer_text("草稿", "Draft")));
    });
}

#[gpui::test]
fn switching_documents_discards_unconfirmed_names(cx: &mut gpui::TestAppContext) {
    let (w, cx) = new_draft(cx);
    let original = w.read_with(cx, |w, _| w.document_session().unwrap().clone());
    let name = original.read_with(cx, |doc, _| doc.display_name());
    open_name(&w, cx);
    cx.simulate_input("Unconfirmed");
    cx.update(|window, app| w.update(app, |w, cx| w.new_draft(window, cx)));
    cx.run_until_parked();
    assert!(w.read_with(cx, |w, _| w.document_name.session.is_none()));
    original.read_with(cx, |doc, _| assert_eq!(doc.display_name(), name));
}

#[gpui::test]
fn save_as_still_selects_a_name_and_path_for_a_named_draft(cx: &mut gpui::TestAppContext) {
    let (w, cx) = new_draft(cx);
    cx.simulate_input("Saved elsewhere");
    open_name(&w, cx);
    cx.simulate_input("My notes");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, app| w.update(app, |w, cx| w.save_document_as(window, cx)));
    assert!(cx.cx.did_prompt_for_new_path() && !cx.cx.did_prompt_for_paths());
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("Other name.org");
    cx.cx.simulate_new_path_selection(|_| Some(target.clone()));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "Saved elsewhere");
    w.read_with(cx, |w, cx| {
        assert_eq!(
            w.document_session().unwrap().read(cx).file_path(),
            Some(target.as_path())
        )
    });
}

#[gpui::test]
fn saved_file_rename_preserves_unsaved_text_selection_and_undo(cx: &mut gpui::TestAppContext) {
    let (directory, w, cx) = saved_file(cx, "Original.org", "* Heading\nSaved body\n");
    let source = directory.path().join("Original.org");
    let target = directory.path().join("项目笔记.org");

    cx.simulate_input("Local edit\n");
    cx.run_until_parked();
    let (session, editor) = w.read_with(cx, |w, _| {
        (
            w.document_session().unwrap().clone(),
            w.editor(PaneSide::Left).unwrap(),
        )
    });
    editor.update(cx, |editor, cx| {
        editor.set_selection(Selection::caret(ByteOffset(3)), cx)
    });
    let (revision, text, baseline, scroll) = session.read_with(cx, |doc, cx| {
        (
            doc.revision(),
            doc.snapshot()
                .copy_range(ByteRange::new(0, doc.snapshot().len_bytes())),
            doc.sync_state().clone(),
            editor.read(cx).top_source_anchor(&doc.snapshot()),
        )
    });
    w.update(cx, |w, _| {
        w.recent_documents = vec![crate::recent_documents::RecentDocument {
            path: source.clone(),
            opened_at: 1,
        }];
    });
    open_name(&w, cx);
    cx.simulate_input("项目笔记.ORG");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!source.exists());
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "* Heading\nSaved body\n"
    );
    w.read_with(cx, |w, cx| {
        assert!(w.document_name.session.is_none());
        assert_eq!(w.document_session(), Some(&session));
        assert_eq!(w.editor(PaneSide::Left), Some(editor.clone()));
        let doc = session.read(cx);
        assert_eq!(doc.file_path(), Some(target.as_path()));
        assert_eq!(doc.revision(), revision);
        assert_eq!(doc.sync_state(), &baseline);
        assert!(doc.is_dirty());
        assert_eq!(
            doc.snapshot()
                .copy_range(ByteRange::new(0, doc.snapshot().len_bytes())),
            text
        );
        assert_eq!(editor.read(cx).selection(), Selection::caret(ByteOffset(3)));
        assert_eq!(editor.read(cx).top_source_anchor(&doc.snapshot()), scroll);
        assert_eq!(w.recent_documents[0].path, target);
        assert!(!w.recent_documents.iter().any(|d| d.path == source));
        assert!(w.window_title(cx).contains("项目笔记.org"));
    });
    session.update(cx, |doc, cx| {
        doc.undo(cx).unwrap();
    });
    cx.run_until_parked();
    session.read_with(cx, |doc, _| {
        assert!(!doc.is_dirty());
        assert_eq!(
            doc.snapshot()
                .copy_range(ByteRange::new(0, doc.snapshot().len_bytes())),
            "* Heading\nSaved body\n"
        );
    });
}

#[gpui::test]
fn save_from_a_saved_file_name_input_renames_then_saves(cx: &mut gpui::TestAppContext) {
    let (directory, w, cx) = saved_file(cx, "Original.md", "# Heading\n");
    let source = directory.path().join("Original.md");
    let target = directory.path().join("Renamed.md");
    bind_save(cx);
    cx.simulate_input("New text\n");
    cx.run_until_parked();
    let session = w.read_with(cx, |w, _| w.document_session().unwrap().clone());
    let text = session.read_with(cx, |doc, _| {
        doc.snapshot()
            .copy_range(ByteRange::new(0, doc.snapshot().len_bytes()))
    });
    open_name(&w, cx);
    cx.simulate_input("Renamed");
    cx.simulate_keystrokes("cmd-s");
    cx.run_until_parked();
    assert!(!source.exists());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), text);
    w.read_with(cx, |w, cx| {
        assert!(w.document_name.session.is_none());
        assert_eq!(w.document_session(), Some(&session));
        assert!(!session.read(cx).is_dirty());
        assert!(matches!(w.save.interaction, SaveInteraction::Idle));
    });
}

#[gpui::test]
fn saved_name_cancel_no_op_and_collision_do_not_change_disk(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("Original.org");
    let taken = directory.path().join("Taken.org");
    std::fs::write(&source, "original").unwrap();
    std::fs::write(&taken, "do not overwrite").unwrap();
    let (w, cx) = new_draft(cx);
    load_saved(&w, &source, cx);
    open_name(&w, cx);
    cx.simulate_input("Cancelled");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!directory.path().join("Cancelled.org").exists());
    open_name(&w, cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(w.read_with(cx, |w, _| w.document_name.session.is_none()));
    open_name(&w, cx);
    cx.simulate_input("Taken");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "original");
    assert_eq!(std::fs::read_to_string(&taken).unwrap(), "do not overwrite");
    w.read_with(cx, |w, cx| {
        assert_eq!(
            w.document_name.session.as_ref().unwrap().error,
            Some(
                w.buffer_text(
                    "已有同名文件，请使用其他名称",
                    "A file with this name already exists; choose another name"
                )
                .to_owned()
            )
        );
        assert_eq!(
            w.document_session().unwrap().read(cx).file_path(),
            Some(source.as_path())
        );
    });
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Available");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(w.read_with(cx, |w, _| w.document_name.session.is_none()));
    assert!(!source.exists());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("Available.org")).unwrap(),
        "original"
    );
}

#[gpui::test]
fn failed_file_rename_keeps_the_name_and_input_for_retry(cx: &mut gpui::TestAppContext) {
    let (directory, w, cx) = saved_file(cx, "Original.org", "original");
    let source = directory.path().join("Original.org");
    let target = directory.path().join("Renamed.org");

    open_name(&w, cx);
    std::fs::remove_file(&source).unwrap();
    cx.simulate_input("Renamed");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!target.exists());
    w.read_with(cx, |w, cx| {
        let prompt = w.document_name.session.as_ref().unwrap();
        assert!(prompt.error.as_ref().is_some_and(|message| {
            message.starts_with(w.buffer_text("无法重命名", "Could not rename"))
        }));
        assert_eq!(prompt.input.read(cx).text, "Renamed");
        assert_eq!(
            w.document_session().unwrap().read(cx).file_path(),
            Some(source.as_path())
        );
        assert!(matches!(w.save.interaction, SaveInteraction::Idle));
    });
}

#[gpui::test]
fn renaming_a_conflicted_file_keeps_the_conflict_and_external_content(
    cx: &mut gpui::TestAppContext,
) {
    let (directory, w, cx) = saved_file(cx, "Original.org", "original");
    let source = directory.path().join("Original.org");
    let target = directory.path().join("Renamed.org");

    cx.simulate_input("local edit");
    let session = w.read_with(cx, |w, _| w.document_session().unwrap().clone());
    std::fs::write(&source, "external edit").unwrap();
    let stamp = crate::document::FileStamp::read(&source).unwrap();
    session.update(cx, |doc, _| {
        assert_eq!(
            doc.observe_disk(Some(stamp)),
            crate::document::DiskChangeAction::Conflict
        );
    });
    let conflict = session.read_with(cx, |doc, _| doc.sync_state().clone());
    open_name(&w, cx);
    cx.simulate_input("Renamed");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!source.exists());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "external edit");
    session.update(cx, |doc, _| {
        assert_eq!(doc.sync_state(), &conflict);
        assert!(matches!(
            doc.begin_save(None),
            Err(crate::document::SaveStartError::Conflict)
        ));
    });
}

#[gpui::test]
fn pending_rename_reserves_its_target_and_follows_the_same_background_document(
    cx: &mut gpui::TestAppContext,
) {
    let (directory, w, cx) = saved_file(cx, "Original.org", "original");
    let source = directory.path().join("Original.org");
    let target = directory.path().join("Renamed.org");

    let session = w.read_with(cx, |w, _| w.document_session().unwrap().clone());
    open_name(&w, cx);
    cx.simulate_input("Renamed");
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            assert!(!w.confirm_document_name(AfterRename::Nothing, window, cx));
            assert!(matches!(
                w.save.interaction,
                SaveInteraction::Renaming { .. }
            ));
            assert_eq!(w.buffer_for_path(&target, cx), Some(session.clone()));
            w.new_draft(window, cx);
        })
    });
    cx.run_until_parked();
    assert!(!source.exists());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
    w.read_with(cx, |w, cx| {
        assert_ne!(w.document_session(), Some(&session));
        assert!(w.document_session().unwrap().read(cx).is_draft());
        assert_eq!(session.read(cx).file_path(), Some(target.as_path()));
        assert!(w.document_name.session.is_none());
    });
}

#[gpui::test]
fn saved_file_can_change_only_the_case_of_its_name(cx: &mut gpui::TestAppContext) {
    let (directory, w, cx) = saved_file(cx, "Original.org", "original");
    let target = directory.path().join("original.org");

    open_name(&w, cx);
    cx.simulate_input("original");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        assert!(
            w.document_name.session.is_none(),
            "{:?}",
            w.document_name.session.as_ref().map(|p| &p.error)
        );
        assert_eq!(
            w.document_session().unwrap().read(cx).file_path(),
            Some(target.as_path())
        );
    });
    let names = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(names, [std::ffi::OsString::from("original.org")]);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
}

#[gpui::test]
fn opening_overview_dismisses_the_name_ui_while_the_submitted_rename_finishes(
    cx: &mut gpui::TestAppContext,
) {
    let (directory, w, cx) = saved_file(cx, "Original.org", "original");
    let target = directory.path().join("Renamed.org");
    open_name(&w, cx);
    cx.simulate_input("Renamed");
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            assert!(!w.confirm_document_name(AfterRename::Nothing, window, cx));
            w.open_file_overview(window, cx);
            w.document_name_tick(cx);
            assert!(w.file_overview_is_open());
            assert!(!w.document_name.is_open());
            assert!(matches!(
                w.save.interaction,
                SaveInteraction::Renaming { .. }
            ));
        })
    });
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
    w.read_with(cx, |w, cx| {
        assert!(!w.document_name.is_open());
        assert!(w.file_overview_is_open());
        assert_eq!(
            w.document_session().unwrap().read(cx).file_path(),
            Some(target.as_path())
        );
    });
}
