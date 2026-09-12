use super::{
    geometry::ShellShape,
    session::{SearchMode, SearchScope},
};
use crate::{
    app::{PanePair, ReadyDocument, WorkspaceLoadState},
    document::{DocumentSession, EditTransaction, TextEdit},
};
use crate::{
    app::{PaneSide, WorkspaceWindow},
    document::{ByteRange, DocumentCommand, EditOrigin, Selection, TextSnapshot},
};
use gpui::{Entity, Modifiers, prelude::*};
use std::{sync::Arc, time::Instant};
pub(super) fn setup(
    cx: &mut gpui::TestAppContext,
    text: &str,
) -> (Entity<WorkspaceWindow>, Entity<DocumentSession>) {
    let session = cx.new(|_| {
        DocumentSession::from_utf8("/tmp/document-search.org".into(), text.as_bytes().to_vec())
            .unwrap()
    });
    let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
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
    (workspace, session)
}
fn query(w: &Entity<WorkspaceWindow>, text: &str, cx: &mut gpui::TestAppContext) {
    w.update(cx, |w, cx| {
        let id = w.search.session.as_ref().unwrap().input.entity_id();
        w.search_input_changed(id, text.into(), cx);
    });
    cx.run_until_parked();
}
fn text(d: &Entity<DocumentSession>, cx: &gpui::TestAppContext) -> String {
    cx.read(|cx| {
        let s = d.read(cx).snapshot();
        s.copy_range(ByteRange::new(0, s.len_bytes()))
    })
}

#[gpui::test]
fn search_exit_drops_business_state_and_reopen_keeps_the_entire_shape(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| cx.set_reduce_motion(false));
    let (w, _) = setup(cx, "alpha alpha");
    w.update(cx, |w, cx| {
        w.open_search(false, false, true, cx);
        let s = w.search.session.as_ref().unwrap();
        let input = s.input.clone();
        let cancel = s.cancel.clone();
        let placeholder = input.read(cx).config.placeholder.to_string();
        s.replacement_input
            .update(cx, |i, cx| i.sync("中文\n🙂", cx));
        let p = w.search.presentation.as_mut().unwrap();
        p.available_width = 1200.;
        let expanded = ShellShape {
            width: 480.,
            height: 109.,
            replacement: 41.,
            search_opacity: 1.,
        };
        p.motion.update(expanded, 1200., Instant::now(), false);
        w.close_search(false, cx);
        assert!(!w.search_is_open());
        assert!(!w.search_owns_input(input.entity_id()));
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(w.keyboard.status(), None);
        let p = w.search.presentation.as_mut().unwrap();
        assert!(p.returning());
        let super::presentation::PresentationPhase::Returning(snapshot) = &p.phase else {
            unreachable!()
        };
        assert_eq!(snapshot.query, placeholder);
        assert_eq!(snapshot.replacement, "中文↵🙂");
        assert!(snapshot.query_empty && !snapshot.replacement_empty);
        let midway = ShellShape {
            width: 800.,
            height: 75.5,
            replacement: 20.5,
            search_opacity: 0.5,
        };
        p.motion.update(midway, 1200., Instant::now(), false);
        w.open_search(false, false, false, cx);
        let p = w.search.presentation.as_ref().unwrap();
        assert!(!p.returning());
        assert_eq!(p.motion.sample(1200., Instant::now()).0, midway);
        let s = w.search.session.as_ref().unwrap();
        assert_ne!(s.input.entity_id(), input.entity_id());
        w.search_input_changed(input.entity_id(), "stale".into(), cx);
        assert!(w.search.session.as_ref().unwrap().query.pattern.is_empty());
    });
    cx.update(|cx| cx.set_reduce_motion(true));
    w.update(cx, |w, cx| {
        w.close_search(false, cx);
        assert!(w.search.presentation.is_none());
    });
}
#[gpui::test]
fn search_skip_then_remaining_never_revisits_skipped_match(cx: &mut gpui::TestAppContext) {
    let (w, d) = setup(cx, "a a a");
    w.update(cx, |w, cx| w.open_search(false, false, true, cx));
    query(&w, "a", cx);
    w.update(cx, |w, cx| {
        w.search.session.as_mut().unwrap().replacement = "aa".into();
        w.search_navigate(false, cx);
        w.search_replace(true, cx);
    });
    cx.run_until_parked();
    assert_eq!(text(&d, cx), "a aa aa");
    w.update(cx, |w, cx| {
        assert!(w.search.session.as_ref().unwrap().current.is_none());
        w.search_replace(true, cx);
    });
    cx.run_until_parked();
    assert_eq!(text(&d, cx), "a aa aa");
    d.update(cx, |d, cx| d.undo(cx).unwrap());
    assert_eq!(text(&d, cx), "a a a");
}
#[gpui::test]
fn search_replacement_mode_preserves_input_without_leaking_scope_semantics(
    cx: &mut gpui::TestAppContext,
) {
    let (w, _) = setup(cx, "alpha alpha");
    w.update(cx, |w, cx| {
        w.open_search(false, false, false, cx);
        w.search_toggle_replacement(cx);
        let s = w.search.session.as_ref().unwrap();
        assert_eq!(s.mode, SearchMode::Replace);
        let input = s.replacement_input.entity_id();
        w.search_input_changed(input, "替换内容".into(), cx);
        w.search_toggle_replacement(cx);
        let s = w.search.session.as_ref().unwrap();
        assert_eq!(s.mode, SearchMode::Find);
        assert!(s.focus_pending && !s.replacement_focus_pending);
        assert_eq!(s.scope, SearchScope::WholeDocument);
        w.search_input_changed(input, "late event".into(), cx);
        w.search_toggle_replacement(cx);
        let s = w.search.session.as_ref().unwrap();
        assert_eq!(s.replacement.as_str(), "替换内容");
        assert_eq!(s.replacement_input.entity_id(), input);
        assert!(s.replacement_focus_pending);
    });
}
#[gpui::test]
fn search_replace_then_remaining_and_ordinary_replace_do_not_loop(cx: &mut gpui::TestAppContext) {
    for query_replace in [true, false] {
        let (w, d) = setup(cx, "a a a");
        w.update(cx, |w, cx| w.open_search(false, false, query_replace, cx));
        query(&w, "a", cx);
        w.update(cx, |w, cx| {
            w.search.session.as_mut().unwrap().replacement = "aa".into();
            w.search_replace(false, cx);
        });
        cx.run_until_parked();
        w.update(cx, |w, cx| {
            assert_eq!(
                w.search.session.as_ref().unwrap().current,
                Some(ByteRange::new(3, 4))
            );
            if query_replace {
                w.search_replace(true, cx)
            } else {
                w.search_replace(false, cx)
            }
        });
        cx.run_until_parked();
        assert_eq!(
            text(&d, cx),
            if query_replace { "aa aa aa" } else { "aa aa a" }
        );
    }
}
#[gpui::test]
fn search_close_and_old_input_cannot_reopen_or_mutate_new_session(cx: &mut gpui::TestAppContext) {
    let (w, d) = setup(cx, "a a a");
    let old = w.update(cx, |w, cx| {
        w.open_search(false, false, false, cx);
        let id = w.search.session.as_ref().unwrap().input.entity_id();
        w.search_input_changed(id, "a".into(), cx);
        w.close_search(true, cx);
        id
    });
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(!w.search_is_open());
        w.open_search(false, false, false, cx);
        w.search_input_changed(old, "old".into(), cx);
        assert!(w.search.session.as_ref().unwrap().query.pattern.is_empty());
    });
    assert_eq!(text(&d, cx), "a a a");
}
#[gpui::test]
fn search_preview_does_not_change_selection_or_dirty_and_cancel_restores(
    cx: &mut gpui::TestAppContext,
) {
    let (w, d) = setup(cx, "a x a");
    w.update(cx, |w, cx| w.open_search(true, false, false, cx));
    query(&w, "a", cx);
    w.update(cx, |w, cx| {
        w.search_navigate(false, cx);
        assert_eq!(
            w.editor(PaneSide::Left).unwrap().read(cx).selection(),
            Selection::default()
        );
        w.close_search(true, cx);
        assert_eq!(
            w.editor(PaneSide::Left).unwrap().read(cx).selection(),
            Selection::default()
        );
    });
    assert!(!cx.read(|cx| d.read(cx).is_dirty()));
}
#[gpui::test]
fn search_isearch_steps_restore_direction_match_and_query(cx: &mut gpui::TestAppContext) {
    let (w, _) = setup(cx, "a a a");
    w.update(cx, |w, cx| w.open_search(true, false, false, cx));
    query(&w, "a", cx);
    w.update(cx, |w, cx| {
        w.search_navigate(false, cx);
        w.search_navigate(true, cx);
        let s = w.search.session.as_ref().unwrap();
        assert_eq!(s.current, Some(ByteRange::new(2, 3)));
        assert!(s.backwards);
        w.search_key("backspace", Modifiers::default(), cx);
    });
    cx.run_until_parked();
    w.update(cx, |w, _| {
        let s = w.search.session.as_ref().unwrap();
        assert!(!s.backwards);
        assert_eq!(s.current, Some(ByteRange::new(2, 3)));
        assert_eq!(s.query.pattern, "a");
    });
}
#[gpui::test]
fn search_external_edit_blocks_fixed_range_and_clears_stale_results(cx: &mut gpui::TestAppContext) {
    let (w, d) = setup(cx, "a a a");
    w.update(cx, |w, cx| w.open_search(false, false, true, cx));
    query(&w, "a", cx);
    d.update(cx, |d, cx| {
        d.edit(
            DocumentCommand::new(
                EditTransaction::new(d.revision(), vec![TextEdit::new(ByteRange::new(2, 3), "b")]),
                Selection::default(),
                Selection::default(),
                EditOrigin::Other,
            ),
            cx,
        )
        .unwrap()
    });
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(w.search.session.as_ref().unwrap().range_blocked);
        assert!(w.search.session.as_ref().unwrap().current.is_none());
        w.search_replace(true, cx);
    });
    assert_eq!(text(&d, cx), "a b a");
}

fn external_edit(
    d: &Entity<DocumentSession>,
    range: ByteRange,
    value: &str,
    cx: &mut gpui::TestAppContext,
) {
    d.update(cx, |d, cx| {
        d.edit(
            DocumentCommand::new(
                EditTransaction::new(d.revision(), vec![TextEdit::new(range, value)]),
                Selection::default(),
                Selection::default(),
                EditOrigin::Other,
            ),
            cx,
        )
        .unwrap();
    });
    cx.run_until_parked();
}

#[gpui::test]
fn search_selection_survives_external_edits_with_or_without_cached_replacement(
    cx: &mut gpui::TestAppContext,
) {
    for cached in [false, true] {
        let (w, d) = setup(cx, "a a a");
        w.update(cx, |w, cx| {
            w.open_search(false, false, false, cx);
            if cached {
                w.search_toggle_replacement(cx);
                w.search_toggle_replacement(cx);
            }
            let s = w.search.session.as_mut().unwrap();
            s.scope = SearchScope::Selection;
            s.query.scope = ByteRange::new(0, 1);
        });
        query(&w, "a", cx);
        external_edit(&d, ByteRange::new(5, 5), " a", cx);
        external_edit(&d, ByteRange::new(0, 0), "z ", cx);
        w.update(cx, |w, cx| {
            let s = w.search.session.as_ref().unwrap();
            assert_eq!(s.query.scope, ByteRange::new(2, 3));
            assert_eq!(
                s.results.as_ref().unwrap().matches.as_ref(),
                &[ByteRange::new(2, 3)]
            );
            w.search_toggle_replacement(cx);
            let id = w
                .search
                .session
                .as_ref()
                .unwrap()
                .replacement_input
                .entity_id();
            w.search_input_changed(id, "x".into(), cx);
            w.search_replace(true, cx);
        });
        cx.run_until_parked();
        assert_eq!(
            text(&d, cx),
            "z x a a a",
            "replacement must stay inside the original selection"
        );
    }
}

#[gpui::test]
fn search_whole_document_keeps_following_edits_after_leaving_replacement(
    cx: &mut gpui::TestAppContext,
) {
    let (w, d) = setup(cx, "a a");
    w.update(cx, |w, cx| {
        w.open_search(false, false, false, cx);
        w.search_toggle_replacement(cx);
        w.search_toggle_replacement(cx);
    });
    query(&w, "a", cx);
    external_edit(&d, ByteRange::new(0, 1), "aa", cx);
    w.update(cx, |w, _| {
        let s = w.search.session.as_ref().unwrap();
        assert!(!s.range_blocked);
        assert_eq!(s.query.scope, ByteRange::new(0, 4));
        assert_eq!(s.results.as_ref().unwrap().matches.len(), 3);
    });
}

#[gpui::test]
fn search_source_transition_preserves_query_scope_replacement_and_shared_results(
    cx: &mut gpui::TestAppContext,
) {
    let (w, d) = setup(cx, "a A a");
    w.update(cx, |w, cx| {
        w.set_active_surface(crate::app::PaneSurface::Reading, cx);
        w.open_search(false, false, false, cx);
        w.search_toggle_replacement(cx);
        let s = w.search.session.as_mut().unwrap();
        s.query.case = crate::search::CaseMode::Sensitive;
        s.scope = SearchScope::Selection;
        s.query.scope = ByteRange::new(0, 3);
        let id = s.replacement_input.entity_id();
        w.search_input_changed(id, "中文🙂".into(), cx);
    });
    query(&w, "A", cx);
    w.update(cx, |w, cx| {
        let s = w.search.session.as_ref().unwrap();
        let id = s.id;
        let input = s.input.entity_id();
        let matches = s.results.as_ref().unwrap().matches.clone();
        w.search_show_source(cx);
        let s = w.search.session.as_ref().unwrap();
        assert_eq!(s.id, id);
        assert_eq!(s.input.entity_id(), input);
        assert_eq!(s.query.pattern, "A");
        assert_eq!(s.query.case, crate::search::CaseMode::Sensitive);
        assert_eq!(s.query.scope, ByteRange::new(0, 3));
        assert_eq!(s.mode, SearchMode::Replace);
        assert!(Arc::ptr_eq(&matches, &s.results.as_ref().unwrap().matches));
        assert!(w.search.history.is_empty());
        w.search_replace(false, cx);
    });
    cx.run_until_parked();
    assert_eq!(text(&d, cx), "a 中文🙂 a");
}

#[gpui::test]
fn search_navigation_shares_matches_and_finds_the_current_index(cx: &mut gpui::TestAppContext) {
    let (w, _) = setup(cx, &"a ".repeat(10_000));
    w.update(cx, |w, cx| w.open_search(false, false, false, cx));
    query(&w, "a", cx);
    w.update(cx, |w, cx| {
        let matches = w
            .search
            .session
            .as_ref()
            .unwrap()
            .results
            .as_ref()
            .unwrap()
            .matches
            .clone();
        for _ in 0..5 {
            w.search_navigate(false, cx);
        }
        let s = w.search.session.as_ref().unwrap();
        assert_eq!(s.current_index(), Some(5));
        assert!(Arc::ptr_eq(&matches, &s.results.as_ref().unwrap().matches));
        assert!(
            Arc::strong_count(&matches) >= 3,
            "session and editor preview must share the match storage"
        );
    });
}

#[gpui::test]
fn search_refinement_keeps_current_location_and_failed_steps_return_to_success(
    cx: &mut gpui::TestAppContext,
) {
    let (w, _) = setup(cx, "ab ab");
    w.update(cx, |w, cx| w.open_search(true, false, false, cx));
    query(&w, "a", cx);
    w.update(cx, |w, cx| w.search_navigate(false, cx));
    query(&w, "ab", cx);
    w.update(cx, |w, _| {
        assert_eq!(
            w.search.session.as_ref().unwrap().current,
            Some(ByteRange::new(3, 5))
        )
    });
    query(&w, "abx", cx);
    query(&w, "abxx", cx);
    w.update(cx, |w, cx| {
        w.search_key(
            "g",
            Modifiers {
                control: true,
                ..Default::default()
            },
            cx,
        )
    });
    cx.run_until_parked();
    w.update(cx, |w, _| {
        let s = w.search.session.as_ref().unwrap();
        assert_eq!(s.query.pattern, "ab");
        assert_eq!(s.current, Some(ByteRange::new(3, 5)));
    });
}
