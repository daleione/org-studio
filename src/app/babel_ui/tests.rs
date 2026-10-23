use crate::{
    app::{PaneSide, WorkspaceWindow},
    document::{
        ByteOffset, ByteRange, DocumentCommand, DocumentSession, EditOrigin, EditTransaction,
        Selection, TextEdit, TextSnapshot,
    },
};
use gpui::Context;

fn code_document(
    workspace: &mut WorkspaceWindow,
    source: &str,
    cx: &mut Context<WorkspaceWindow>,
) -> gpui::Entity<DocumentSession> {
    workspace.create_buffer("analysis-notes.org".into(), None, cx);
    workspace.show_editor(cx);
    let session = workspace.document_session().unwrap().clone();
    session.update(cx, |document, cx| {
        document
            .edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        document.revision(),
                        vec![TextEdit::new(ByteRange::new(0, 0), source)],
                    ),
                    Selection::default(),
                    Selection::default(),
                    EditOrigin::Other,
                ),
                cx,
            )
            .unwrap()
    });
    session
}

#[gpui::test]
fn code_confirmation_cancel_and_source_changes_never_approve_execution(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(crate::editor::init);
    let source = "#+name: calculation\n#+begin_src python\nraise RuntimeError('this code must not run')\n#+end_src\n";
    for dismissal in ["click", "escape", "control-g", "tab-enter", "changed"] {
        let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
        let session = workspace.update(cx, |w, cx| code_document(w, source, cx));
        let revision = session.read_with(cx, |s, _| s.revision());
        cx.update(|window, app| {
            workspace.update(app, |w, cx| {
                let plan = crate::babel::prepare_execution(
                    &session.read(cx).snapshot(),
                    session.read(cx).path(),
                    Selection::caret(ByteOffset(source.find("#+begin_src").unwrap() as u64)),
                    Vec::new(),
                )
                .unwrap();
                w.confirm_source_execution(
                    plan,
                    session.clone(),
                    w.editor(PaneSide::Left).unwrap(),
                    window,
                    cx,
                );
            })
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("code-execution-dialog").is_some());
        assert!(cx.debug_bounds("code-confirm-preview").is_some());
        assert!(cx.debug_bounds("mysql-query-dialog").is_none());
        cx.simulate_keystrokes("x");
        assert_eq!(session.read_with(cx, |s, _| s.revision()), revision);
        match dismissal {
            "click" => {
                let cancel = cx.debug_bounds("code-confirm-cancel").unwrap().center();
                cx.simulate_click(cancel, gpui::Modifiers::default());
            }
            "escape" => cx.simulate_keystrokes("escape"),
            "control-g" => cx.simulate_keystrokes("ctrl-g"),
            "tab-enter" => {
                cx.simulate_keystrokes("tab");
                cx.simulate_keystrokes("enter");
            }
            "changed" => {
                session.update(cx, |s, cx| {
                    s.edit(
                        DocumentCommand::new(
                            EditTransaction::new(
                                s.revision(),
                                vec![TextEdit::new(ByteRange::new(0, 0), "Changed\n")],
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
            _ => unreachable!(),
        }
        cx.run_until_parked();
        assert!(cx.debug_bounds("code-execution-dialog").is_none());
        workspace.read_with(cx, |w, _| {
            assert!(w.babel_task.is_none());
            assert!(w.babel_run.is_none());
            assert!(w.babel_trusted_documents.is_empty());
        });
    }
}

#[gpui::test]
fn code_confirmation_keeps_actions_visible_and_restores_editor_focus(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::Focusable;
    cx.update(crate::editor::init);
    let previous_theme = crate::theme::theme_mode();
    let source = "#+name: prepare-input\n#+begin_src python\nreturn [1, 2, 3]\n#+end_src\n\n#+name: calculation\n#+begin_src python :var rows=prepare-input\n# Work with the supplied rows.\nvalues = [n * 2 for n in rows]\ncount = len(values)\ntotal = sum(values)\naverage = total / count\nreturn [[count, total, average]]\n#+end_src\n";
    for theme in [
        crate::theme::ThemeMode::Light,
        crate::theme::ThemeMode::Dark,
    ] {
        crate::theme::set_theme_mode(theme);
        let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
        let session = workspace.update(cx, |w, cx| code_document(w, source, cx));
        cx.update(|window, app| {
            window.resize(gpui::size(gpui::px(440.), gpui::px(540.)));
            workspace.update(app, |w, cx| {
                let plan = crate::babel::prepare_execution(
                    &session.read(cx).snapshot(),
                    session.read(cx).path(),
                    Selection::caret(ByteOffset(source.rfind("#+begin_src").unwrap() as u64)),
                    Vec::new(),
                )
                .unwrap();
                w.confirm_source_execution(
                    plan,
                    session.clone(),
                    w.editor(PaneSide::Left).unwrap(),
                    window,
                    cx,
                );
            });
        });
        cx.run_until_parked();
        let dialog = cx.debug_bounds("code-execution-dialog").unwrap();
        let run = cx.debug_bounds("code-confirm-run").unwrap();
        assert!(
            run.bottom() <= dialog.bottom(),
            "run button must remain inside the visible dialog"
        );
        assert!(cx.debug_bounds("code-confirm-dependencies").is_some());
        if theme == crate::theme::ThemeMode::Dark {
            cx.simulate_click(run.center(), gpui::Modifiers::default());
        } else {
            cx.simulate_keystrokes("escape");
        }
        cx.run_until_parked();
        cx.update(|window, app| {
            workspace.read_with(app, |w, cx| {
                assert!(
                    w.editor(PaneSide::Left)
                        .unwrap()
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                )
            })
        });
        let snapshot = session.read_with(cx, |s, _| s.snapshot());
        let text = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()));
        if theme == crate::theme::ThemeMode::Dark {
            assert!(
                text.contains("#+RESULTS: calculation"),
                "approved code should execute and publish its result"
            );
            workspace.read_with(cx, |w, _| assert!(!w.babel_trusted_documents.is_empty()));
        } else {
            assert_eq!(text, source);
        }
    }
    crate::theme::set_theme_mode(previous_theme);
}
