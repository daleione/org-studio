use super::*;
use gpui::{TestAppContext, VisualTestContext};

fn source(session: &Entity<DocumentSession>, view: &VisualTestContext) -> String {
    view.read(|cx| {
        let snapshot = session.read(cx).snapshot();
        snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
    })
}

#[gpui::test]
fn link_picker_edits_hidden_target_and_description_as_one_undo_step(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "See [[https://example.org/old][Guide]] next.\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("links.org".into(), original.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session.clone(), cx));
    view.run_until_parked();
    editor.update(view, |editor, cx| {
        editor.set_selection(
            Selection::caret(ByteOffset(original.find("Guide").unwrap() as u64 + 2)),
            cx,
        );
        editor.edit_link_at_caret(cx);
    });
    view.run_until_parked();
    assert!(view.debug_bounds("link-picker").is_some());
    view.simulate_input("https://example.org/new[section]");
    view.simulate_keystrokes("tab");
    view.run_until_parked();
    view.simulate_keystrokes("cmd-a");
    view.simulate_input("新指南 😀");
    view.simulate_keystrokes("enter");
    view.run_until_parked();
    assert!(view.debug_bounds("link-picker").is_none());
    let changed = "See [[https://example.org/new\\[section\\]][新指南 😀]] next.\n";
    assert_eq!(source(&session, view), changed);
    view.read(|cx| {
        assert!(
            editor.read(cx).hit_rows[0]
                .layout
                .text
                .contains("新指南 😀")
        );
        assert!(
            !editor.read(cx).hit_rows[0]
                .layout
                .text
                .contains("example.org")
        );
    });
    view.simulate_keystrokes("cmd-z");
    view.run_until_parked();
    assert_eq!(source(&session, view), original);
    view.simulate_keystrokes("cmd-shift-z");
    view.run_until_parked();
    assert_eq!(source(&session, view), changed);
}

#[gpui::test]
fn link_picker_inserts_from_selection_and_cancel_keeps_source(cx: &mut TestAppContext) {
    cx.update(init);
    let original = "Read the handbook.\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("links.org".into(), original.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session.clone(), cx));
    view.run_until_parked();
    editor.update(view, |editor, cx| {
        editor.set_selection(Selection::new(ByteOffset(9), ByteOffset(17)), cx);
        editor.edit_link_at_caret(cx);
    });
    view.run_until_parked();
    view.simulate_input("file:handbook.org");
    view.simulate_keystrokes("escape");
    view.run_until_parked();
    assert_eq!(source(&session, view), original);
    assert!(view.debug_bounds("link-picker").is_none());
    editor.update(view, |editor, cx| editor.edit_link_at_caret(cx));
    view.run_until_parked();
    view.simulate_input("file:handbook.org");
    let apply = view.debug_bounds("link-apply").unwrap().center();
    view.simulate_click(apply, gpui::Modifiers::default());
    view.run_until_parked();
    assert_eq!(
        source(&session, view),
        "Read the [[file:handbook.org][handbook]].\n"
    );
}

#[gpui::test]
fn link_picker_rejects_stale_revision_and_skips_noop_edits(cx: &mut TestAppContext) {
    let original = "[[file:handbook.org][Handbook]]";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("links.org".into(), original.as_bytes().to_vec()).unwrap()
    });
    let editor = cx.new(|cx| SemanticEditor::new(session.clone(), cx));
    editor.update(cx, |editor, cx| {
        let revision = editor.snapshot(cx).revision();
        editor.edit_link_at_caret(cx);
        editor.apply_link_edit(original, cx);
        assert_eq!(editor.snapshot(cx).revision(), revision);
        editor.edit_link_at_caret(cx);
        // A shared-session edit arrives before the editor handles its notification.
        let external = format!("{original}\n");
        session.update(cx, |session, cx| {
            session
                .edit(
                    DocumentCommand::new(
                        EditTransaction::new(
                            revision,
                            vec![TextEdit::new(
                                ByteRange::new(original.len() as u64, original.len() as u64),
                                "\n",
                            )],
                        ),
                        editor.selection,
                        editor.selection,
                        EditOrigin::Other,
                    ),
                    cx,
                )
                .unwrap();
        });
        let changed_revision = editor.snapshot(cx).revision();
        editor.apply_link_edit("[[file:other.org][Other]]", cx);
        let snapshot = editor.snapshot(cx);
        assert_eq!(snapshot.revision(), changed_revision);
        assert_eq!(
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
            external
        );
        assert_eq!(
            editor.command_feedback.as_deref(),
            Some(editor.ui_language.text("link.source_changed"))
        );
    });
}
