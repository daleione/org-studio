use crate::app::{PanePair, PaneSide, ReadyDocument, WorkspaceLoadState};
use crate::document::{
    ByteOffset, ByteRange, DocumentCommand, DocumentSession, EditOrigin, EditTransaction,
    Selection, TextEdit,
};
use crate::{
    app::{WorkspaceWindow, echo_area::EchoMessage},
    babel::CancelToken,
};
use gpui::{AppContext, Focusable};
use std::{sync::Arc, time::Duration};

#[gpui::test]
fn cancelling_mysql_document_trust_never_launches_a_query(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    for dismissal in ["click", "escape", "tab-enter", "changed"] {
        let source = "#+begin_src sql :engine mysql :dbconnection mysql://reader:synthetic@127.0.0.1:9/test\nSELECT 1;\n#+end_src\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                "/tmp/org-studio-mysql-trust.org".into(),
                source.as_bytes().to_vec(),
            )
            .unwrap()
        });
        let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
        workspace.update(cx, |w, cx| {
            w.state = WorkspaceLoadState::Ready {
                document: ReadyDocument {
                    session: session.clone(),
                    editor_syntax: Arc::default(),
                    editors: PanePair {
                        left: None,
                        right: None,
                    },
                    readers: PanePair {
                        left: None,
                        right: None,
                    },
                },
            };
            w.ensure_editor_for(PaneSide::Left, cx);
        });
        cx.update(|window, app| {
            workspace.update(app, |w, cx| {
                let request = crate::babel::prepare_execution(
                    &session.read(cx).snapshot(),
                    session.read(cx).path(),
                    Selection::caret(ByteOffset(0)),
                    Vec::new(),
                )
                .unwrap();
                w.confirm_source_execution(
                    request,
                    session.clone(),
                    w.editor(PaneSide::Left).unwrap(),
                    window,
                    cx,
                );
                assert!(w.babel_run.is_none());
            })
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("mysql-query-dialog").is_some());
        cx.simulate_keystrokes("x");
        session.read_with(cx, |session, _| assert!(!session.is_dirty()));
        match dismissal {
            "click" => {
                let cancel = cx.debug_bounds("mysql-confirm-cancel").unwrap();
                cx.simulate_click(cancel.center(), gpui::Modifiers::default());
            }
            "escape" => cx.simulate_keystrokes("escape"),
            "tab-enter" => {
                cx.simulate_keystrokes("tab");
                cx.simulate_keystrokes("enter");
            }
            "changed" => session.update(cx, |doc, cx| {
                doc.edit(
                    DocumentCommand::new(
                        EditTransaction::new(
                            doc.revision(),
                            vec![TextEdit::new(ByteRange::new(0, 0), "Changed\n")],
                        ),
                        Selection::default(),
                        Selection::default(),
                        EditOrigin::Other,
                    ),
                    cx,
                )
                .unwrap();
            }),
            _ => unreachable!(),
        }
        cx.run_until_parked();
        assert!(cx.debug_bounds("mysql-query-dialog").is_none());
        workspace.read_with(cx, |w, _| {
            assert!(w.babel_task.is_none());
            assert!(w.babel_run.is_none());
            assert!(w.mysql.trusted.is_empty());
        });
        session.read_with(cx, |session, _| {
            assert_eq!(session.is_dirty(), dismissal == "changed")
        });
    }
}

#[gpui::test]
fn query_status_stops_the_matching_task_and_restores_the_statusline(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let cancel = CancelToken::default();
    let session = workspace.update(cx, |w, cx| {
        w.create_buffer("query.org".into(), None, cx);
        w.show_editor(cx);
        let session = w.document_session().unwrap().clone();
        w.babel_request = 7;
        w.start_babel_run(session.read(cx).id(), ByteOffset(0), cancel.clone(), cx);
        w.stop_babel_execution(6, cx);
        session
    });
    assert!(
        !cancel.cancelled(),
        "a stale control must not stop a newer query"
    );
    cx.update(|window, app| {
        window.resize(gpui::size(gpui::px(440.), gpui::px(660.)));
        workspace.update(app, |w, cx| {
            window.focus(
                &w.editor(PaneSide::Left).unwrap().read(cx).focus_handle(cx),
                cx,
            );
        });
    });
    cx.run_until_parked();
    let revision = session.read_with(cx, |doc, _| doc.revision());
    let stop = cx.debug_bounds("echo-action").unwrap();
    cx.simulate_click(stop.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cancel.cancelled());
    cx.update(|window, app| {
        workspace.read_with(app, |w, cx| {
            assert!(
                w.editor(PaneSide::Left)
                    .unwrap()
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            assert_eq!(session.read(cx).revision(), revision);
        });
    });
    workspace.update(cx, |w, cx| {
        w.babel_run = None;
        w.show_echo_message(EchoMessage::success("Query complete"), cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("echo-action").is_none());
    workspace.read_with(cx, |w, cx| assert!(w.displayed_echo_message(cx).is_some()));
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    workspace.read_with(cx, |w, cx| assert!(w.displayed_echo_message(cx).is_none()));
}
