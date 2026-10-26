use super::picker::match_score;
use super::*;
use crate::document::{
    ByteRange, DocumentCommand, EditOrigin, EditTransaction, Selection, TextEdit, TextSnapshot,
};
use gpui::AppContext;

fn edit(session: &Entity<DocumentSession>, text: &str, cx: &mut gpui::TestAppContext) {
    session.update(cx, |s, cx| {
        s.edit(
            DocumentCommand::new(
                EditTransaction::new(
                    s.revision(),
                    vec![TextEdit::new(
                        ByteRange::new(0, s.snapshot().len_bytes()),
                        text,
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
}

fn text(session: &Entity<DocumentSession>, cx: &gpui::TestAppContext) -> String {
    cx.read(|cx| {
        let snapshot = session.read(cx).snapshot();
        snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
    })
}

#[gpui::test]
fn switching_and_home_preserve_independent_sessions_and_undo(cx: &mut gpui::TestAppContext) {
    let w = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    let a = w.update(cx, |w, cx| {
        w.create_buffer("A".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    edit(&a, "中文 A", cx);
    let a_editor = w.update(cx, |w, _| w.editor(crate::app::PaneSide::Left).unwrap());
    let b = w.update(cx, |w, cx| {
        w.create_buffer("B".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    edit(&b, "English B", cx);
    w.update(cx, |w, cx| {
        w.activate_buffer(a.read(cx).id(), cx);
        assert_eq!(w.document_session().unwrap(), &a);
        assert_eq!(w.editor(crate::app::PaneSide::Left).unwrap(), a_editor);
        assert_eq!(w.buffer_sessions().count(), 2);
        w.show_home_now(cx);
        assert!(w.document_session().is_none());
        assert_eq!(w.buffer_sessions().count(), 2);
        w.activate_buffer(b.read(cx).id(), cx);
    });
    a.update(cx, |s, cx| {
        s.undo(cx).unwrap();
    });
    assert_eq!(text(&a, cx), "");
    assert_eq!(text(&b, cx), "English B");
}

#[gpui::test]
fn exit_review_includes_hidden_drafts_and_cancel_keeps_them(cx: &mut gpui::TestAppContext) {
    let w = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    for name in ["中文草稿", "English draft"] {
        let session = w.update(cx, |w, cx| {
            w.create_buffer(name.into(), None, cx);
            w.document_session().unwrap().clone()
        });
        edit(&session, name, cx);
    }
    w.update(cx, |w, cx| {
        w.show_home_now(cx);
        w.begin_buffer_review(ReviewKind::Quit, cx);
        assert_eq!(w.buffers.review().unwrap().entries.len(), 2);
        w.cancel_buffer_panel(cx);
        assert!(w.buffers.review().is_none());
        assert!(w.buffer_sessions().all(|s| s.read(cx).is_dirty()));
    });
}

#[gpui::test]
fn cc_co_opens_the_link_under_the_caret(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let session = w.update(cx, |w, cx| {
        w.create_buffer("links.org".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    edit(
        &session,
        "* Top\n:PROPERTIES:\n:CUSTOM_ID: plan\n:END:\n\nbody [[#plan][jump]]\n",
        cx,
    );
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        let editor = w
            .editor(w.document_workspace.active_pane)
            .expect("an editor pane");
        editor.update(cx, |editor, cx| {
            editor.set_selection(Selection::caret(crate::document::ByteOffset(50)), cx)
        });
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-c ctrl-o");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        let editor = w.editor(w.document_workspace.active_pane).unwrap();
        assert_eq!(
            editor.read(cx).selection().head().0,
            19,
            "C-c C-o must jump to the :CUSTOM_ID: line"
        );
    });
}

#[gpui::test]
fn emacs_picker_is_stable_and_does_not_type_into_document(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    w.update(cx, |w, cx| {
        w.create_buffer("notes.org".into(), None, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-x b");
    cx.run_until_parked();
    let original = cx.debug_bounds("floating-status-line").unwrap();
    w.update(cx, |w, cx| {
        assert!(matches!(w.buffers.panel, Some(Panel::Picker(_))));
        if let Some(Panel::Picker(p)) = &w.buffers.panel {
            p.input.update(cx, |i, cx| i.sync("不存在 / no match", cx));
        }
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(cx.debug_bounds("floating-status-line").unwrap(), original);
    cx.simulate_keystrokes("ctrl-g");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(w.buffers.panel.is_none());
        assert!(!w.document_session().unwrap().read(cx).is_dirty());
    });
    cx.simulate_keystrokes("ctrl-x ctrl-b");
    cx.run_until_parked();
    w.update(cx, |w, _| {
        assert!(matches!(w.buffers.panel, Some(Panel::Picker(_))))
    });
    cx.simulate_keystrokes("ctrl-g");
    cx.simulate_resize(gpui::size(gpui::px(390.), gpui::px(600.)));
    for language in [
        crate::i18n::Language::Chinese,
        crate::i18n::Language::English,
    ] {
        w.update(cx, |w, cx| {
            w.language = language;
            w.open_buffer_picker(PickerIntent::New, cx);
            if let Some(Panel::Picker(p)) = &mut w.buffers.panel {
                p.markdown = true;
            }
        });
        cx.run_until_parked();
        w.update(cx, |w, cx| {
            let Some(Panel::Picker(p)) = &w.buffers.panel else {
                panic!("missing input");
            };
            let input = p.input.read(cx);
            assert!(
                f32::from(input.painted_bounds().unwrap().size.width) >= input.content_width,
                "placeholder must fit in {language:?}"
            );
        });
        let bounds = cx.debug_bounds("floating-status-line").unwrap();
        assert!(f32::from(bounds.right()) <= 390.);
        assert_eq!(f32::from(bounds.size.height), 92.);
    }
}

#[cfg(unix)]
#[test]
fn aliases_and_hard_links_have_one_file_identity() {
    let directory = std::env::temp_dir().join(format!("buffer-identity-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("notes.org");
    let hard = directory.join("hard.org");
    let sym = directory.join("symbolic.org");
    std::fs::write(&file, "text").unwrap();
    std::fs::hard_link(&file, &hard).unwrap();
    std::os::unix::fs::symlink(&file, &sym).unwrap();
    assert!(same_file(&file, &hard));
    assert!(same_file(&file, &sym));
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
fn an_unnamed_picker_draft_requires_a_name_on_first_save(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    for markdown in [false, true] {
        w.update(cx, |w, cx| {
            w.open_buffer_picker(PickerIntent::New, cx);
            let Some(Panel::Picker(picker)) = &mut w.buffers.panel else {
                panic!("missing new-document input");
            };
            picker.markdown = markdown;
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        let session = w.read_with(cx, |w, cx| {
            assert!(w.buffers.panel.is_none());
            let session = w.document_session().unwrap();
            let doc = session.read(cx);
            assert!(doc.is_draft() && !doc.is_named_draft());
            assert_eq!(
                doc.syntax_path().extension().and_then(|e| e.to_str()),
                Some(if markdown { "md" } else { "org" })
            );
            session.clone()
        });
        cx.simulate_input("keep this draft");
        cx.run_until_parked();
        assert_eq!(text(&session, cx), "keep this draft");
        cx.update(|window, app| w.update(app, |w, cx| w.save_document(window, cx)));
        cx.cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        w.read_with(cx, |w, cx| {
            assert!(matches!(
                w.save.interaction,
                crate::app::save::SaveInteraction::Idle
            ));
            assert_eq!(w.document_session(), Some(&session));
            assert!(session.read(cx).file_path().is_none());
            assert!(session.read(cx).is_dirty());
        });
    }
}

#[gpui::test]
fn save_review_names_drafts_sequentially_and_keeps_skipped_edits(cx: &mut gpui::TestAppContext) {
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let root = std::env::temp_dir().join(format!("buffer-review-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut sessions = Vec::new();
    for name in ["A", "B", "跳过"] {
        let session = w.update(cx, |w, cx| {
            w.create_buffer(name.into(), None, cx);
            let s = w.document_session().unwrap().clone();
            s.update(cx, |s, cx| {
                s.apply_transient_edit(
                    EditTransaction::new(
                        s.revision(),
                        vec![TextEdit::new(ByteRange::new(0, 0), name)],
                    ),
                    cx,
                )
                .unwrap();
            });
            s
        });
        sessions.push(session);
    }
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.begin_buffer_review(ReviewKind::Save, cx);
            w.buffers.review_mut().unwrap().entries[0].save = false;
            w.process_buffer_review(window, cx);
        })
    });
    for name in ["B", "A"] {
        cx.cx.simulate_path_prompt_response(|options| {
            assert!(options.directories && !options.files && !options.multiple);
            assert!(options.prompt.as_ref().unwrap().contains(name));
            Some(vec![root.clone()])
        });
        cx.run_until_parked();
    }
    w.update(cx, |w, cx| {
        assert!(w.buffers.review().is_none());
        assert!(!sessions[0].read(cx).is_dirty());
        assert!(!sessions[1].read(cx).is_dirty());
        assert!(sessions[2].read(cx).is_dirty());
        assert!(sessions[2].read(cx).file_path().is_none());
    });
    assert_eq!(std::fs::read_to_string(root.join("A.org")).unwrap(), "A");
    assert_eq!(std::fs::read_to_string(root.join("B.org")).unwrap(), "B");
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn batch_open_keeps_every_success_and_focuses_the_first_file(cx: &mut gpui::TestAppContext) {
    let root = std::env::temp_dir().join(format!("buffer-batch-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let paths = [
        root.join("A.org"),
        root.join("B.md"),
        root.join("missing.org"),
    ];
    std::fs::write(&paths[0], "* A").unwrap();
    std::fs::write(&paths[1], "# B").unwrap();
    let w = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    w.update(cx, |w, cx| w.open_buffers(paths.to_vec(), cx));
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert_eq!(w.buffer_sessions().count(), 2);
        assert!(same_file(
            w.document_session().unwrap().read(cx).path(),
            &paths[0]
        ));
        let b = w.buffer_for_path(&paths[1], cx).unwrap();
        let a = w.document_session().unwrap().read(cx).id();
        w.open(paths[1].clone(), cx);
        assert_eq!(w.document_session().unwrap(), &b);
        assert_eq!(w.buffer_sessions().count(), 2);
        assert_eq!(
            w.buffers.opened_ids().collect::<Vec<_>>(),
            [a, b.read(cx).id()]
        );
        w.remove_buffer(a, cx);
        w.open(paths[0].clone(), cx);
    });
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        let b = w.buffer_for_path(&paths[1], cx).unwrap().read(cx).id();
        let reopened = w.document_session().unwrap().read(cx).id();
        assert_eq!(w.buffers.opened_ids().collect::<Vec<_>>(), [b, reopened]);
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn background_source_file_activates_in_editor(cx: &mut gpui::TestAppContext) {
    let root = std::env::temp_dir().join(format!("buffer-source-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("main.py");
    std::fs::write(&path, "def main():\n    return 42\n").unwrap();
    let w = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    w.update(cx, |w, cx| {
        w.create_buffer("notes.org".into(), None, cx);
        w.document_workspace
            .set_surface(crate::app::PaneSide::Left, crate::app::PaneSurface::Reading);
        w.open_background_buffer(path.clone(), cx);
    });
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        let session = w.buffer_for_path(&path, cx).unwrap();
        w.activate_buffer(session.read(cx).id(), cx);
        assert_eq!(
            w.document_workspace.active_surface(),
            crate::app::PaneSurface::Editor
        );
        assert!(w.editor(crate::app::PaneSide::Left).is_some());
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn save_callback_stays_with_its_document_after_switching(cx: &mut gpui::TestAppContext) {
    let root = std::env::temp_dir().join(format!("buffer-save-switch-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("A.org");
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let a = w.update(cx, |w, cx| {
        w.create_buffer("A".into(), Some(path.clone()), cx);
        let a = w.document_session().unwrap().clone();
        a.update(cx, |s, cx| {
            s.apply_transient_edit(
                EditTransaction::new(
                    s.revision(),
                    vec![TextEdit::new(ByteRange::new(0, 0), "saved A")],
                ),
                cx,
            )
            .unwrap();
        });
        a
    });
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.save_document(window, cx);
            w.create_buffer("B".into(), None, cx);
            let b = w.document_session().unwrap().clone();
            b.update(cx, |s, cx| {
                s.apply_transient_edit(
                    EditTransaction::new(
                        s.revision(),
                        vec![TextEdit::new(ByteRange::new(0, 0), "unsaved B")],
                    ),
                    cx,
                )
                .unwrap();
            });
        })
    });
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(!a.read(cx).is_dirty());
        let b = w.document_session().unwrap().read(cx);
        assert!(b.is_dirty());
        assert_eq!(b.display_name(), "B");
        assert!(b.file_path().is_none());
    });
    assert_eq!(std::fs::read_to_string(path).unwrap(), "saved A");
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn keyboard_close_reviews_before_discard_and_window_close_checks_hidden_edits(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    w.update(cx, |w, cx| {
        w.create_buffer("keep".into(), None, cx);
        let s = w.document_session().unwrap().clone();
        s.update(cx, |s, cx| {
            s.apply_transient_edit(
                EditTransaction::new(
                    s.revision(),
                    vec![TextEdit::new(ByteRange::new(0, 0), "keep this")],
                ),
                cx,
            )
            .unwrap();
        });
        w.create_buffer("discard".into(), None, cx);
        let s = w.document_session().unwrap().clone();
        s.update(cx, |s, cx| {
            s.apply_transient_edit(
                EditTransaction::new(
                    s.revision(),
                    vec![TextEdit::new(ByteRange::new(0, 0), "discard this")],
                ),
                cx,
            )
            .unwrap();
        });
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-x k enter");
    cx.run_until_parked();
    w.update(cx, |w, _| {
        assert_eq!(w.buffers.review().unwrap().entries.len(), 1)
    });
    cx.simulate_keystrokes("tab space");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert_eq!(w.buffer_sessions().count(), 1);
        assert_eq!(
            w.document_session().unwrap().read(cx).display_name(),
            "keep"
        );
        w.show_home_now(cx);
    });
    cx.run_until_parked();
    assert!(!cx.simulate_close());
    cx.run_until_parked();
    w.update(cx, |w, _| {
        assert_eq!(w.buffers.review().unwrap().kind, ReviewKind::Window)
    });
    cx.simulate_keystrokes("ctrl-g");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(w.buffer_sessions().next().unwrap().read(cx).is_dirty())
    });
}

#[gpui::test]
fn exiting_skips_clean_files_and_empty_drafts_but_reviews_cleared_saved_files(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.org");
    std::fs::write(&path, "saved text").unwrap();
    let loaded = crate::preview::load_document(path.clone()).unwrap();
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    cx.run_until_parked();
    assert!(cx.simulate_close());
    let saved = w.update(cx, |w, cx| {
        assert!(w.apply_load_result(w.generation, Ok(loaded), cx));
        w.document_session().unwrap().clone()
    });
    let draft = w.update(cx, |w, cx| {
        w.create_buffer("draft.org".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    edit(&draft, "temporary text", cx);
    edit(&draft, "", cx);
    assert!(draft.read_with(cx, |s, _| s.is_dirty()));
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.activate_buffer(saved.read(cx).id(), cx);
            w.request_quit(window, cx);
        });
    });
    cx.run_until_parked();
    assert!(w.read_with(cx, |w, _| w.buffers.review().is_none()));
    assert!(cx.simulate_close());

    edit(&saved, "", cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.show_home_now(cx);
            w.request_quit(window, cx);
        });
    });
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        let review = w.buffers.review().unwrap();
        assert_eq!(review.kind, ReviewKind::Quit);
        assert_eq!(review.entries.len(), 1);
        assert_eq!(review.entries[0].id, saved.read(cx).id());
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!cx.simulate_close());
    w.read_with(cx, |w, cx| {
        let review = w.buffers.review().unwrap();
        assert_eq!(review.kind, ReviewKind::Window);
        assert_eq!(review.entries.len(), 1);
        assert_eq!(review.entries[0].id, saved.read(cx).id());
    });
    assert_eq!(std::fs::read_to_string(path).unwrap(), "saved text");
}

#[gpui::test]
fn an_empty_draft_with_a_pending_save_still_blocks_exit(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let directory = tempfile::tempdir().unwrap();
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let draft = w.update(cx, |w, cx| {
        w.create_buffer("blank.org".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    let request = draft.update(cx, |s, _| {
        s.begin_save(Some(directory.path().join("blank.org")))
            .unwrap()
    });
    assert!(!draft.read_with(cx, |s, _| s.is_dirty()));
    cx.update(|window, app| w.update(app, |w, cx| w.request_quit(window, cx)));
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        let review = w.buffers.review().unwrap();
        assert_eq!(review.kind, ReviewKind::Quit);
        assert_eq!(review.entries.len(), 1);
        assert_eq!(review.entries[0].id, draft.read(cx).id());
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!cx.simulate_close());
    assert_eq!(
        w.read_with(cx, |w, _| w.buffers.review().unwrap().kind),
        ReviewKind::Window
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    draft.update(cx, |s, _| s.cancel_save(request.revision()));
    assert!(cx.simulate_close());
}

#[gpui::test]
fn quit_review_is_compact_and_tab_cycles_only_visible_buttons(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    cx.simulate_resize(gpui::size(gpui::px(900.), gpui::px(680.)));
    let session = w.update(cx, |w, cx| {
        w.create_buffer("notes.org".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    edit(&session, "work in progress", cx);
    w.update(cx, |w, cx| w.begin_buffer_review(ReviewKind::Quit, cx));
    cx.run_until_parked();
    let shell = cx.debug_bounds("floating-status-line").unwrap();
    assert!(shell.size.height < crate::typography::ui_px(250.));
    assert!(cx.debug_bounds("buffer-review-modal").is_none());
    assert!(cx.debug_bounds("buffer-review-cancel").is_none());
    let buttons = ["buffer-review-discard-all", "buffer-review-submit"];
    let before = buttons.map(|id| cx.debug_bounds(id).unwrap());
    assert!(before[0].right() <= before[1].left());
    assert!(before[0].left() >= shell.left() && before[1].right() <= shell.right());
    cx.update(|window, app| {
        assert!(
            w.read(app)
                .buffers
                .review()
                .unwrap()
                .focus(ReviewControl::Submit)
                .is_focused(window)
        );
    });
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        assert!(
            w.read(app)
                .buffers
                .review()
                .unwrap()
                .focus(ReviewControl::Discard)
                .is_focused(window)
        );
    });
    assert_eq!(before, buttons.map(|id| cx.debug_bounds(id).unwrap()));
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        assert!(
            w.read(app)
                .buffers
                .review()
                .unwrap()
                .focus(ReviewControl::Submit)
                .is_focused(window)
        );
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(w.read_with(cx, |w, _| w.buffers.review().is_none()));
    assert_eq!(text(&session, cx), "work in progress");
    assert!(session.read_with(cx, |s, _| s.is_dirty()));
}

#[gpui::test]
fn batch_review_choices_and_file_links_do_not_discard_edits(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    cx.simulate_resize(gpui::size(gpui::px(900.), gpui::px(680.)));
    for index in 0..8 {
        let session = w.update(cx, |w, cx| {
            w.create_buffer(format!("document-{index}.org"), None, cx);
            w.document_session().unwrap().clone()
        });
        edit(&session, &format!("draft {index}"), cx);
    }
    w.update(cx, |w, cx| {
        w.buffers
            .scroll
            .set_offset(gpui::point(gpui::px(0.), gpui::px(-200.)));
        w.begin_buffer_review(ReviewKind::Quit, cx);
    });
    cx.run_until_parked();
    let shell = cx.debug_bounds("floating-status-line").unwrap();
    let list = cx.debug_bounds("buffer-review-list").unwrap();
    assert!(cx.debug_bounds("buffer-review-row-0").unwrap().top() >= list.top());
    let submit = cx.debug_bounds("buffer-review-submit").unwrap();
    assert!(shell.size.height < gpui::px(600.));
    assert!(list.bottom() <= submit.top());
    assert!(cx.debug_bounds("buffer-review-discard-all").is_none());
    assert!(cx.debug_bounds("buffer-review-cancel").is_none());
    cx.simulate_keystrokes("tab space");
    cx.run_until_parked();
    cx.update(|window, app| {
        let review = w.read(app).buffers.review().unwrap();
        assert!(review.entries.iter().all(|e| !e.save));
        assert!(review.focus(ReviewControl::All).is_focused(window));
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        assert!(w.buffers.review().unwrap().entries.iter().all(|e| e.save));
        assert!(w.buffer_sessions().all(|s| s.read(cx).is_dirty()));
        assert!(w.save.task.is_none() && w.save.dialog_task.is_none());
    });
    cx.simulate_keystrokes("ctrl-g");
    cx.run_until_parked();
    assert!(w.read_with(cx, |w, _| w.buffers.review().is_none()));

    w.update(cx, |w, cx| w.begin_buffer_review(ReviewKind::Quit, cx));
    cx.run_until_parked();
    let target = w.read_with(cx, |w, _| w.buffers.review().unwrap().entries[1].id);
    let choice = cx.debug_bounds("buffer-review-choice-1").unwrap();
    cx.simulate_click(choice.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(w.read_with(cx, |w, _| !w.buffers.review().unwrap().entries[1].save));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        assert!(w.buffers.review().is_none());
        assert_eq!(w.document_session().unwrap().read(cx).id(), target);
        assert!(w.buffer_sessions().all(|s| s.read(cx).is_dirty()));
        assert!(w.save.task.is_none() && w.save.dialog_task.is_none());
    });

    w.update(cx, |w, cx| w.begin_buffer_review(ReviewKind::Quit, cx));
    cx.run_until_parked();
    let target = w.read_with(cx, |w, _| w.buffers.review().unwrap().entries[2].id);
    let file = cx.debug_bounds("buffer-review-document-2").unwrap();
    cx.simulate_click(file.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        assert!(w.buffers.review().is_none());
        assert_eq!(w.document_session().unwrap().read(cx).id(), target);
        assert!(w.buffer_sessions().all(|s| s.read(cx).is_dirty()));
    });
    cx.simulate_resize(gpui::size(gpui::px(390.), gpui::px(360.)));
    w.update(cx, |w, cx| w.begin_buffer_review(ReviewKind::Quit, cx));
    cx.run_until_parked();
    let shell = cx.debug_bounds("floating-status-line").unwrap();
    let list = cx.debug_bounds("buffer-review-list").unwrap();
    let button = cx.debug_bounds("buffer-review-submit").unwrap();
    assert!(button.left() >= shell.left() && button.right() <= shell.right());
    assert!(button.bottom() <= shell.bottom());
    assert!(list.bottom() <= button.top());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
}

#[gpui::test]
fn review_morph_retains_content_until_the_status_line_returns(cx: &mut gpui::TestAppContext) {
    use crate::app::status_line::shell::{ResizePolicy, ShellKind, ShellShape};
    use std::time::Instant;
    cx.update(|cx| cx.set_reduce_motion(false));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let session = w.update(cx, |w, cx| {
        w.create_buffer("draft.org".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    edit(&session, "pending changes", cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            for kind in [ReviewKind::Quit, ReviewKind::Close(session.read(cx).id())] {
                w.begin_buffer_review(kind, cx);
                let open = w.buffer_shell_request(window, cx).unwrap();
                assert_eq!(open.kind, ShellKind::Buffers);
                assert!(matches!(open.resize, ResizePolicy::Animated));
                w.status_shell_tick(window, cx);
                let opening = w.status.sample_shell(open.available, Instant::now());
                assert!(opening.height < open.target.height);
                assert!(opening.content_opacity < 1.);
                w.status.set_shell_shape_for_test(
                    open.target,
                    open.available,
                    Instant::now(),
                    false,
                );
                w.cancel_buffer_panel(cx);
                assert!(w.buffers.review().is_none());
                assert!(matches!(w.buffers.outgoing, Some(OutgoingPanel::Review(_))));
                let close = w.buffer_shell_request(window, cx).unwrap();
                assert!(close.returning);
                w.status_shell_tick(window, cx);
                let closing = w.status.sample_shell(close.available, Instant::now());
                assert!(closing.height > close.target.height);
                assert!(closing.content_opacity > 0.);
                w.status.set_shell_shape_for_test(
                    ShellShape::status(close.available),
                    close.available,
                    Instant::now(),
                    false,
                );
                w.status_shell_tick(window, cx);
                assert!(!w.buffers.returning());
            }
        })
    });
}

#[gpui::test]
fn close_return_keeps_the_document_visible_after_removal(cx: &mut gpui::TestAppContext) {
    use crate::app::status_line::shell::ShellShape;
    use std::time::{Duration, Instant};
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(false));
    let (w, view) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let current = w.update(view, |w, cx| {
        w.create_buffer("Earlier.org".into(), None, cx);
        w.create_buffer("Later.org".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    edit(&current, "pending changes", view);
    let id = current.read_with(view, |document, _| document.id());
    let available = view.update(|window, app| {
        w.update(app, |w, cx| {
            w.request_close_buffer(id, cx);
            let open = w.buffer_shell_request(window, cx).unwrap();
            w.status
                .set_shell_shape_for_test(open.target, open.available, Instant::now(), false);
            w.status_shell_tick(window, cx);
            open.available
        })
    });
    view.run_until_parked();
    assert!(view.debug_bounds("buffer-review-document-0").is_some());
    view.update(|window, app| {
        w.update(app, |w, cx| {
            w.discard_buffer_review(window, cx);
            assert!(w.buffer_session(id, cx).is_none());
            // Keep the return animation open independently of test execution speed.
            w.status.set_shell_shape_for_test(
                ShellShape::status(available),
                available,
                Instant::now() + Duration::from_secs(5),
                true,
            );
            cx.notify();
        })
    });
    drop(current);
    view.run_until_parked();
    assert!(
        view.debug_bounds("buffer-review-document-0").is_some(),
        "closing the file must not remove its name from the outgoing panel"
    );
    view.update(|window, app| {
        w.update(app, |w, cx| {
            w.status.set_shell_shape_for_test(
                ShellShape::status(available),
                available,
                Instant::now(),
                false,
            );
            w.status_shell_tick(window, cx);
            assert!(!w.buffers.returning());
        })
    });
}

#[test]
fn matching_handles_unicode_case_and_path_ranking() {
    assert_eq!(match_score("生活.org", "笔记/生活.org", "生活"), Some(800));
    assert_eq!(
        match_score("README.md", "project/README.md", "readme.md"),
        Some(1000)
    );
    assert_eq!(
        match_score("notes.org", "工作/notes.org", "工作"),
        Some(300)
    );
    assert_eq!(match_score("reading-list.md", "", "rdlst"), Some(100));
    assert_eq!(match_score("notes.org", "", "不存在"), None);
}

#[gpui::test]
fn cancelled_review_does_not_write_after_path_prompt_returns(cx: &mut gpui::TestAppContext) {
    let root = std::env::temp_dir().join(format!("buffer-cancel-prompt-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let target = root.join("草稿.org");
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let session = w.update(cx, |w, cx| {
        w.create_buffer("草稿".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    edit(&session, "尚未保存", cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.begin_buffer_review(ReviewKind::Save, cx);
            w.process_buffer_review(window, cx);
            w.cancel_buffer_panel(cx);
            w.begin_buffer_review(ReviewKind::Quit, cx);
        })
    });
    cx.cx
        .simulate_path_prompt_response(|_| Some(vec![root.clone()]));
    cx.run_until_parked();
    assert!(
        !target.exists(),
        "a cancelled review must not start a write"
    );
    w.update(cx, |w, cx| {
        assert!(session.read(cx).is_dirty());
        assert!(session.read(cx).file_path().is_none());
        let review = w.buffers.review().unwrap();
        assert_eq!(review.kind, ReviewKind::Quit);
        assert!(!review.running && !review.entries[0].done);
        w.open_buffer_picker(PickerIntent::Switch, cx);
        assert!(w.buffers.review().is_none());
        w.request_close_buffer(session.read(cx).id(), cx);
        assert!(matches!(
            w.buffers.review().unwrap().kind,
            ReviewKind::Close(_)
        ));
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn open_picker_includes_documents_loaded_in_the_background(cx: &mut gpui::TestAppContext) {
    let root = std::env::temp_dir().join(format!("buffer-live-picker-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("后台.md");
    std::fs::write(&path, "# 后台").unwrap();
    let w = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    w.update(cx, |w, cx| {
        w.create_buffer("当前.org".into(), None, cx);
        w.recent_documents.clear();
        w.open_buffer_picker(PickerIntent::Switch, cx);
        w.open_background_buffer(path.clone(), cx);
    });
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        let candidates = w.buffer_candidates(cx);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].name, "当前.org");
        assert_eq!(candidates[1].name, "后台.md");
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn latest_foreground_open_wins_without_losing_previous_requests(cx: &mut gpui::TestAppContext) {
    let root = std::env::temp_dir().join(format!("buffer-open-focus-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let a = root.join("A.org");
    let b = root.join("B.org");
    std::fs::write(&a, "* A").unwrap();
    std::fs::write(&b, "* B").unwrap();
    let w = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    w.update(cx, |w, cx| {
        w.open(a.clone(), cx);
        w.open(b.clone(), cx);
    });
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert_eq!(w.buffer_sessions().count(), 2);
        assert!(same_file(
            w.document_session().unwrap().read(cx).file_path().unwrap(),
            &b
        ));
        assert!(w.buffer_for_path(&a, cx).is_some());
        let first = w.buffer_for_path(&a, cx).unwrap().read(cx).id();
        let second = w.document_session().unwrap().read(cx).id();
        assert_eq!(w.buffers.opened_ids().collect::<Vec<_>>(), [first, second]);
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn home_picker_uses_window_width_and_leaves_no_statusline(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(true));
    cx.simulate_resize(gpui::size(gpui::px(900.), gpui::px(600.)));
    w.update(cx, |w, cx| {
        w.create_buffer("分屏.org".into(), None, cx);
        w.show_home_now(cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("floating-status-line").is_none());
    w.update(cx, |w, cx| w.open_buffer_picker(PickerIntent::New, cx));
    cx.run_until_parked();
    assert_eq!(
        f32::from(cx.debug_bounds("floating-status-line").unwrap().size.width),
        560.
    );
    w.update(cx, |w, cx| w.cancel_buffer_panel(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("floating-status-line").is_none());
}

#[gpui::test]
fn recent_panel_scroll_stays_inside_the_panel(cx: &mut gpui::TestAppContext) {
    use gpui::{point, px, size};
    let scroll = |position, delta| gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(delta))),
        touch_phase: gpui::TouchPhase::Moved,
        ..Default::default()
    };
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    cx.simulate_resize(size(px(900.), px(700.)));
    let session = w.update(cx, |w, cx| {
        w.create_buffer("正文.org".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    edit(&session, &"A scrollable line of text\n".repeat(200), cx);
    cx.run_until_parked();
    let editor = w.update(cx, |w, _| w.editor(crate::app::PaneSide::Left).unwrap());
    editor.update(cx, |e, cx| {
        e.scroll_to_source_offset(crate::document::ByteOffset(0), cx)
    });
    cx.run_until_parked();
    let origin = editor.update(cx, |e, cx| e.top_source_anchor(&e.snapshot(cx)));
    w.update(cx, |w, cx| {
        w.recent_documents = (0..10)
            .map(|index| crate::recent_documents::RecentDocument {
                path: PathBuf::from(format!("/tmp/recent-scroll-{index}.org")),
                opened_at: index,
            })
            .collect();
        w.open_buffer_picker(PickerIntent::Switch, cx);
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("floating-status-line").unwrap();
    let center = bounds.center();
    cx.simulate_event(scroll(center, -80.));
    cx.run_until_parked();
    w.update(cx, |w, _| assert!(w.buffers.scroll.offset().y < px(0.)));
    assert_eq!(
        editor.update(cx, |e, cx| e.top_source_anchor(&e.snapshot(cx))),
        origin
    );

    // Cover both ends of the list, the header, and the footer outside the scroll area.
    for (position, delta) in [
        (center, -10000.),
        (center, -80.),
        (center, 10000.),
        (center, 80.),
        (point(center.x, bounds.top() + px(20.)), -80.),
        (point(center.x, bounds.bottom() - px(20.)), -80.),
    ] {
        cx.simulate_event(scroll(position, delta));
        cx.run_until_parked();
        assert_eq!(
            editor.update(cx, |e, cx| e.top_source_anchor(&e.snapshot(cx))),
            origin
        );
    }
    w.update(cx, |w, cx| {
        if let Some(Panel::Picker(p)) = &w.buffers.panel {
            p.input.update(cx, |i, cx| i.sync("没有匹配", cx));
        }
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_event(scroll(center, -80.));
    cx.run_until_parked();
    assert_eq!(
        editor.update(cx, |e, cx| e.top_source_anchor(&e.snapshot(cx))),
        origin
    );

    w.update(cx, |w, cx| w.cancel_buffer_panel(cx));
    cx.run_until_parked();
    cx.simulate_event(scroll(center, -80.));
    cx.run_until_parked();
    assert_ne!(
        editor.update(cx, |e, cx| e.top_source_anchor(&e.snapshot(cx))),
        origin
    );
}

#[gpui::test]
fn single_close_keyboard_cancel_and_batch_skip_keep_edits(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let session = w.update(cx, |w, cx| {
        w.create_buffer("未保存的草稿.org".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    edit(&session, "keep until confirmed", cx);
    w.update(cx, |w, cx| {
        w.begin_buffer_review(ReviewKind::Close(session.read(cx).id()), cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(w.read_with(cx, |w, _| w.buffers.review().is_none()));
    assert_eq!(text(&session, cx), "keep until confirmed");

    w.update(cx, |w, cx| w.begin_buffer_review(ReviewKind::Save, cx));
    cx.run_until_parked();
    let skip = cx.debug_bounds("buffer-review-choice-0").unwrap().center();
    cx.simulate_click(skip, gpui::Modifiers::default());
    cx.run_until_parked();
    let submit = cx.debug_bounds("buffer-review-submit").unwrap().center();
    cx.simulate_click(submit, gpui::Modifiers::default());
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        assert!(w.buffers.review().is_none());
        assert_eq!(w.document_session(), Some(&session));
        assert!(session.read(cx).is_dirty());
    });
    assert_eq!(text(&session, cx), "keep until confirmed");
}

#[gpui::test]
fn quit_without_saving_includes_hidden_drafts_and_never_prompts(cx: &mut gpui::TestAppContext) {
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    for name in ["hidden", "visible"] {
        let session = w.update(cx, |w, cx| {
            w.create_buffer(name.into(), None, cx);
            w.document_session().unwrap().clone()
        });
        edit(&session, name, cx);
    }
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.begin_buffer_review(ReviewKind::Quit, cx);
            assert_eq!(w.buffers.review().unwrap().entries.len(), 2);
            w.discard_buffer_review(window, cx);
            assert!(w.buffers.review().is_none());
            assert!(matches!(
                w.save.interaction,
                crate::app::save::SaveInteraction::AllowCloseOnce
            ));
            assert!(w.save.task.is_none() && w.save.dialog_task.is_none());
            assert!(
                w.buffer_sessions()
                    .all(|s| s.read(cx).file_path().is_none())
            );
        })
    });
}

#[gpui::test]
fn discard_review_rejects_new_revisions_and_new_dirty_buffers(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let clean = w.update(cx, |w, cx| {
        w.create_buffer("clean".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    let dirty = w.update(cx, |w, cx| {
        w.create_buffer("dirty".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    edit(&dirty, "reviewed version", cx);
    w.update(cx, |w, cx| w.begin_buffer_review(ReviewKind::Quit, cx));
    edit(&dirty, "new version", cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.discard_buffer_review(window, cx);
            assert!(w.buffers.review().unwrap().error.is_some());
            assert!(matches!(
                w.save.interaction,
                crate::app::save::SaveInteraction::Idle
            ));
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        assert!(
            w.read(app)
                .buffers
                .review()
                .unwrap()
                .focus(ReviewControl::Discard)
                .is_focused(window)
        );
    });
    cx.simulate_keystrokes("ctrl-g");
    cx.run_until_parked();
    w.update(cx, |w, cx| w.begin_buffer_review(ReviewKind::Quit, cx));
    edit(&clean, "new unreviewed edits", cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.discard_buffer_review(window, cx);
            assert!(w.buffers.review().unwrap().error.is_some());
            assert!(matches!(
                w.save.interaction,
                crate::app::save::SaveInteraction::Idle
            ));
        })
    });
    assert_eq!(text(&dirty, cx), "new version");
    assert_eq!(text(&clean, cx), "new unreviewed edits");
}

#[gpui::test]
fn running_review_and_save_only_review_cannot_discard_all(cx: &mut gpui::TestAppContext) {
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let session = w.update(cx, |w, cx| {
        w.create_buffer("draft".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    edit(&session, "draft", cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.begin_buffer_review(ReviewKind::Save, cx);
            w.discard_buffer_review(window, cx);
            assert!(w.buffers.review().unwrap().entries[0].save);
            w.cancel_buffer_panel(cx);
            w.begin_buffer_review(ReviewKind::Quit, cx);
            w.buffers.review_mut().unwrap().running = true;
            w.discard_buffer_review(window, cx);
            w.set_buffer_review_choice(session.read(cx).id(), false, cx);
            assert!(w.buffers.review().unwrap().entries[0].save);
        })
    });
}
