use super::*;
use crate::app::PaneSide;
use crate::document::{
    ByteRange, DocumentCommand, EditOrigin, EditTransaction, Selection, TextEdit, TextSnapshot,
};
use gpui::{EntityInputHandler, Modifiers, VisualTestContext};

fn draft(
    w: &Entity<WorkspaceWindow>,
    name: &str,
    text: &str,
    cx: &mut VisualTestContext,
) -> Entity<DocumentSession> {
    w.update(cx, |w, cx| {
        w.create_buffer(name.into(), None, cx);
        let session = w.document_session().unwrap().clone();
        session.update(cx, |doc, cx| {
            doc.edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        doc.revision(),
                        vec![TextEdit::new(ByteRange::new(0, 0), text)],
                    ),
                    Selection::default(),
                    Selection::default(),
                    EditOrigin::Other,
                ),
                cx,
            )
            .unwrap();
        });
        session
    })
}

fn open(w: &Entity<WorkspaceWindow>, cx: &mut VisualTestContext) {
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.open_file_overview(window, cx);
            w.file_overview_tick(window, cx);
        })
    });
    cx.run_until_parked();
}

fn complete_motion(
    w: &mut WorkspaceWindow,
    window: &mut Window,
    cx: &mut Context<WorkspaceWindow>,
) {
    for _ in 0..3 {
        if let Some(state) = &mut w.file_overview.session {
            if let Some(motion) = state.motion {
                let target = motion.sample(Instant::now() + Duration::from_secs(3)).value;
                state.motion = Some(Motion::settle(
                    Instant::now() - Duration::from_secs(3),
                    state.progress,
                    target,
                    0.,
                    false,
                ));
            }
        }
        w.file_overview_tick(window, cx);
    }
}

#[gpui::test]
fn cached_target_uses_current_layout_and_invalidates_safely(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let target = draft(&w, "Target.org", "* Target\n\nRetained text", cx);
    cx.run_until_parked();
    let current = draft(&w, "Current.org", "* Current", cx);
    cx.run_until_parked();
    let cached_size = w.read_with(cx, |w, cx| {
        w.file_overview
            .cache
            .get(target.read(cx).id(), target.read(cx).revision())
            .expect("the previously painted target is cached")
            .source
            .bounds
            .size
    });
    cx.simulate_resize(size(px(900.), px(1100.)));
    cx.run_until_parked();
    open(&w, cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            let state = w.file_overview.session.as_mut().unwrap();
            state.phase = Phase::Visible;
            state.progress = 1.;
            state.motion = None;
            w.select_overview_item(target.read(cx).id(), window, cx);
            let state = w.file_overview.session.as_ref().unwrap();
            assert!(state.is_preparing());
            assert!(state.target_frame.is_none());
            assert_eq!(w.document_session().unwrap(), &current);
        });
    });
    for _ in 0..4 {
        cx.update(|window, app| window.simulate_next_frame(app));
        cx.run_until_parked();
    }
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            let state = w.file_overview.session.as_ref().unwrap();
            assert!(!state.is_preparing());
            let frame = state.target_frame.as_ref().expect("fresh target layout");
            assert_ne!(frame.source.bounds.size, cached_size);
            assert_eq!(frame.source.bounds, w.overview_stage_bounds(window));
            assert_eq!(w.document_session().unwrap(), &current);
            w.invalidate_file_overview_frames();
            w.file_overview_tick(window, cx);
            let state = w.file_overview.session.as_ref().unwrap();
            assert!(matches!(state.phase, Phase::Rebounding | Phase::Visible));
            assert!(state.target_frame.is_none());
            assert_eq!(w.document_session().unwrap(), &current);
        });
    });
}

#[gpui::test]
fn cold_document_paints_a_real_frame_before_opening_or_committing(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let target = draft(
        &w,
        "Cold.org",
        "* Cold target\n\nContent retained in the session",
        cx,
    );
    w.update(cx, |w, cx| w.show_split(cx));
    cx.run_until_parked();
    let current = draft(&w, "Current.org", "* Current", cx);
    w.update(cx, |w, cx| {
        w.show_editor(cx);
        w.file_overview.cache.clear();
    });
    open(&w, cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            let state = w.file_overview.session.as_mut().unwrap();
            state.phase = Phase::Visible;
            state.progress = 1.;
            state.motion = None;
            w.select_overview_item(target.read(cx).id(), window, cx);
            let state = w.file_overview.session.as_ref().unwrap();
            assert!(state.is_preparing());
            assert_eq!(state.progress, 0.);
            assert_eq!(w.document_session().unwrap(), &current);
            w.file_overview.session.as_mut().unwrap().motion = Some(Motion::settle(
                Instant::now() - Duration::from_millis(100),
                0.,
                1.,
                0.,
                false,
            ));
            w.cancel_file_overview(window, cx);
            let state = w.file_overview.session.as_ref().unwrap();
            assert_eq!(
                state.progress, 0.,
                "preparation never moved the displayed card"
            );
            assert!(w.overview_preparing_document().is_none());
            complete_motion(w, window, cx);
            w.select_overview_item(target.read(cx).id(), window, cx);
            let now = Instant::now();
            let mut gesture = Gesture::new(true, now);
            gesture.update(0.3, now);
            w.file_overview
                .session
                .as_mut()
                .unwrap()
                .begin_gesture(gesture, now);
        })
    });
    for _ in 0..4 {
        cx.run_until_parked();
        cx.update(|window, app| {
            window.simulate_next_frame(app);
        });
    }
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        let state = w.file_overview.session.as_ref().unwrap();
        assert!(!state.is_preparing());
        assert!(!state.is_preparing());
        assert!(
            state.follow.is_some(),
            "a held cold gesture must resume through its follow filter"
        );
        let frame = state
            .target_frame
            .as_ref()
            .expect("real original-layout frame");
        assert_eq!(frame.revision, target.read(cx).revision());
        assert_eq!(frame.source.bounds, state.target_to);
        assert_eq!(w.document_session().unwrap(), &current);
    });
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.cancel_file_overview(window, cx);
            complete_motion(w, window, cx);
        })
    });
    assert_eq!(
        w.read_with(cx, |w, _| w.document_session().unwrap().clone()),
        current
    );
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.file_overview.cache.clear();
            w.select_overview_item(target.read(cx).id(), window, cx);
            assert!(w.file_overview.session.as_ref().unwrap().is_preparing());
            target.update(cx, |doc, cx| {
                doc.edit(
                    DocumentCommand::new(
                        EditTransaction::new(
                            doc.revision(),
                            vec![TextEdit::new(ByteRange::new(0, 0), "Updated ")],
                        ),
                        Selection::default(),
                        Selection::default(),
                        EditOrigin::Other,
                    ),
                    cx,
                )
                .unwrap();
            });
            w.file_overview_tick(window, cx);
            let state = w.file_overview.session.as_ref().unwrap();
            assert!(!state.is_preparing());
            assert!(w.overview_preparing_document().is_none());
            assert!(state.notice.is_some());
            assert_eq!(w.document_session().unwrap(), &current);
        })
    });
}

#[gpui::test]
fn covered_document_stops_rendering_and_resumes_the_same_editor(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let doc = draft(&w, "Current.org", "* Preserved text", cx);
    cx.run_until_parked();
    let editor = w.read_with(cx, |w, _| w.editor(PaneSide::Left).unwrap().clone());
    let frame = w.read_with(cx, |w, _| w.file_overview.drawn_frame.latest().unwrap());
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.open_file_overview(window, cx);
            let state = w.file_overview.session.as_mut().unwrap();
            state.phase = Phase::Visible;
            state.progress = 1.;
            state.motion = None;
            cx.notify();
        })
    });
    for _ in 0..3 {
        w.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert!(cx.debug_bounds("file-overview-paused-document").is_some());
        w.read_with(cx, |w, _| {
            assert!(Arc::ptr_eq(
                &frame,
                w.file_overview.drawn_frame.latest().as_ref().unwrap()
            ));
        });
    }
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.cancel_file_overview(window, cx);
            w.file_overview.session.as_mut().unwrap().motion =
                Some(Motion::settle(Instant::now(), 0., 0., 0., true));
            w.file_overview_tick(window, cx);
            let state = w.file_overview.session.as_ref().unwrap();
            assert_eq!(state.phase, Phase::Handoff);
            assert_eq!(state.handoff_frames, 0);
            assert_eq!(state.handoff_backdrop, 0.);
            assert!(Arc::ptr_eq(
                state.target_frame.as_ref().unwrap(),
                state.origin_frame.as_ref().unwrap()
            ));
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("file-overview-paused-document").is_none());
    w.read_with(cx, |w, _| {
        assert_eq!(w.document_session().unwrap(), &doc);
        assert_eq!(w.editor(PaneSide::Left).unwrap(), editor);
    });
    for _ in 0..2 {
        cx.update(|window, app| window.simulate_next_frame(app));
        cx.run_until_parked();
    }
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            assert!(w.file_overview.session.as_ref().unwrap().handoff_frames >= 2);
            cx.set_reduce_motion(true);
            w.file_overview_tick(window, cx);
            assert!(!w.file_overview_is_open());
        })
    });
    cx.simulate_input("X");
    cx.run_until_parked();
    assert!(doc.read_with(cx, |doc, _| {
        let snapshot = doc.snapshot();
        snapshot
            .copy_range(ByteRange::new(0, snapshot.len_bytes()))
            .contains('X')
    }));
}

#[gpui::test]
fn overview_search_and_cancel_preserve_sessions_undo_and_search(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let a = draft(&w, "项目.org", "* 未保存 A", cx);
    let b = draft(&w, "notes.md", "# Unsaved B", cx);
    w.update(cx, |w, cx| w.open_search(false, false, false, cx));
    cx.run_until_parked();
    open(&w, cx);
    assert!(cx.debug_bounds("file-overview").is_some());
    cx.simulate_input("项目");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        let items = w.overview_items(cx);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, a.read(cx).id());
        assert_eq!(w.document_session().unwrap(), &b);
        assert!(w.search_is_open());
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(!w.file_overview_is_open());
        assert!(w.search_is_open());
        assert_eq!(w.document_session().unwrap(), &b);
        assert!(a.read(cx).is_dirty() && b.read(cx).is_dirty());
        assert_eq!(w.buffer_sessions().count(), 2);
    });
    a.update(cx, |doc, cx| {
        doc.undo(cx).unwrap();
    });
    assert_eq!(a.read_with(cx, |doc, _| doc.snapshot().len_bytes()), 0);
    assert_eq!(b.read_with(cx, |doc, _| doc.snapshot().len_bytes()), 11);
}

#[gpui::test]
fn overview_only_lists_open_sessions_and_typing_never_edits_the_document(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let doc = draft(&w, "current.org", "* Text", cx);
    w.update(cx, |w, _| {
        w.recent_documents.clear();
        crate::recent_documents::record_success(
            &mut w.recent_documents,
            "/tmp/closed-only.org".into(),
        );
    });
    open(&w, cx);
    cx.simulate_input("closed-only");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(w.overview_items(cx).is_empty());
        assert_eq!(
            doc.read(cx).snapshot().copy_range(ByteRange::new(0, 6)),
            "* Text"
        );
    });
    cx.simulate_keystrokes("ctrl-g");
    cx.run_until_parked();
    assert!(!w.read_with(cx, |w, _| w.file_overview_is_open()));
}

#[gpui::test]
fn selecting_commits_once_and_restores_the_same_editor_entity(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let a = draft(&w, "A.org", "* A", cx);
    let editor = w.read_with(cx, |w, _| w.editor(super::super::PaneSide::Left).unwrap());
    let _b = draft(&w, "B.org", "* B", cx);
    open(&w, cx);
    cx.simulate_keystrokes("right enter");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(!w.file_overview_is_open());
        assert_eq!(w.document_session().unwrap(), &a);
        assert_eq!(w.editor(super::super::PaneSide::Left).unwrap(), editor);
        assert_eq!(w.buffer_sessions().count(), 2);
        assert!(a.read(cx).is_dirty());
    });
}

#[gpui::test]
fn cancelled_expansion_and_closed_target_cannot_activate_a_document(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let a = draft(&w, "A.org", "* A", cx);
    let b = draft(&w, "B.org", "* B", cx);
    open(&w, cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.file_overview.session.as_mut().unwrap().reduced_motion = false;
            w.select_overview_item(a.read(cx).id(), window, cx);
            assert_eq!(w.document_session().unwrap(), &b);
            w.cancel_file_overview(window, cx);
            complete_motion(w, window, cx);
            assert_eq!(
                w.file_overview.session.as_ref().unwrap().phase,
                Phase::Visible
            );
            assert_eq!(w.document_session().unwrap(), &b);
            w.select_overview_item(a.read(cx).id(), window, cx);
            w.remove_buffer(a.read(cx).id(), cx);
            complete_motion(w, window, cx);
            assert_eq!(w.document_session().unwrap(), &b);
            assert!(w.file_overview.session.as_ref().unwrap().target.is_none());
        })
    });
}

#[gpui::test]
fn keyboard_cancel_restores_home_and_the_tool_route(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    draft(&w, "A.org", "* A", cx);
    w.update(cx, |w, cx| w.show_home_now(cx));
    open(&w, cx);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(!w.file_overview_is_open());
        assert!(w.document_session().is_none());
        assert_eq!(w.buffer_sessions().count(), 1);
        w.content_route = ContentRoute::FileManager;
        cx.notify();
    });
    open(&w, cx);
    cx.simulate_keystrokes("ctrl-g");
    cx.run_until_parked();
    assert_eq!(
        w.read_with(cx, |w, _| w.content_route),
        ContentRoute::FileManager
    );
}

#[gpui::test]
fn pinch_is_continuous_and_escape_preserves_the_origin(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let doc = draft(&w, "A.org", "* A", cx);
    cx.run_until_parked();
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            let position = w.overview_stage_bounds(window).center();
            let event = |delta, phase| gpui::PinchEvent {
                position,
                delta,
                phase,
                modifiers: Modifiers::default(),
            };
            w.file_overview_pinch(&event(-0.015, TouchPhase::Started), window, cx);
            assert!(!w.file_overview_is_open());
            w.file_overview_pinch(&event(-0.08, TouchPhase::Moved), window, cx);
            let state = w.file_overview.session.as_ref().unwrap();
            assert!(state.progress > 0.0 && state.progress < 1.0);
            assert!(state.motion.is_none());
            // A duplicate Started during an owned stream must not reset it.
            let previous = state.progress;
            w.file_overview_pinch(&event(-0.02, TouchPhase::Started), window, cx);
            assert!(w.file_overview.session.as_ref().unwrap().progress > previous);
            w.overview_key_command("escape", Modifiers::default(), window, cx);
            complete_motion(w, window, cx);
            assert!(!w.file_overview_is_open());
            assert_eq!(w.document_session().unwrap(), &doc);
            w.content_route = ContentRoute::Image;
            w.file_overview_pinch(&event(-0.4, TouchPhase::Moved), window, cx);
            assert!(!w.file_overview_is_open());
            w.content_route = ContentRoute::Document;
        })
    });
}

#[gpui::test]
fn filtered_gesture_release_keeps_displayed_motion_and_raw_intent(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let doc = draft(&w, "A.org", "* A", cx);
    open(&w, cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            let state = w.file_overview.session.as_mut().unwrap();
            state.reduced_motion = false;
            state.phase = Phase::Entering;
            let now = Instant::now();
            let mut gesture = Gesture::new(false, now - Duration::from_millis(16));
            gesture.update(-0.25, now);
            assert!(gesture.commits(TouchPhase::Ended, now));
            state.begin_gesture(gesture, now);
            assert_eq!(state.progress, 0.);
            let target = state.gesture.as_ref().unwrap().progress();
            let scale = 1. + (state.gesture_scale() - 1.) * target;
            assert!((scale - 0.75 / 0.97).abs() < 0.00001);
            let release_at = now + Duration::from_millis(20);
            let displayed = state.follow.unwrap().sample(release_at);
            assert!(displayed.value > 0. && displayed.value < 0.45);
            state.settle(Phase::Entering, 1., release_at);
            let released = state.motion.unwrap().sample(release_at);
            assert!((released.value - displayed.value).abs() < 0.000001);
            assert!((released.velocity - displayed.velocity).abs() < 0.00001);
            assert!(state.follow.is_none() && state.gesture.is_none());
            complete_motion(w, window, cx);
            assert_eq!(
                w.file_overview.session.as_ref().unwrap().phase,
                Phase::Visible
            );
            assert_eq!(w.document_session().unwrap(), &doc);
            w.close_file_overview_now(window, cx);
        })
    });
}

#[gpui::test]
fn new_contact_can_take_over_each_settling_direction(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let doc = draft(&w, "A.org", "* A", cx);
    open(&w, cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            let position = w.overview_stage_bounds(window).center();
            for phase in [
                Phase::Entering,
                Phase::Returning,
                Phase::Opening,
                Phase::Rebounding,
            ] {
                let state = w.file_overview.session.as_mut().unwrap();
                state.reduced_motion = false;
                state.phase = phase;
                state.gesture = None;
                state.follow = None;
                state.target = state.origin;
                state.target_frame = state.origin_frame.clone();
                state.target_from = state.origin_card;
                state.target_to = state.origin_bounds;
                let motion = Motion::settle(
                    Instant::now() - Duration::from_millis(80),
                    0.2,
                    1.,
                    0.,
                    false,
                );
                let before = motion.sample(Instant::now());
                state.progress = before.value;
                state.motion = Some(motion);
                let event = |delta, phase| gpui::PinchEvent {
                    position,
                    delta,
                    phase,
                    modifiers: Default::default(),
                };
                w.file_overview_pinch(&event(0., TouchPhase::Started), window, cx);
                let state = w.file_overview.session.as_ref().unwrap();
                assert!(state.motion.is_none());
                assert!(state.follow.is_some() && state.gesture.is_some());
                assert!((state.progress - before.value).abs() < 0.005);
                let grabbed = state.follow.unwrap().sample(Instant::now());
                assert!((grabbed.velocity - before.velocity).abs() < 0.1);
                let opening = matches!(phase, Phase::Opening | Phase::Rebounding);
                w.file_overview_pinch(
                    &event(if opening { -0.08 } else { 0.08 }, TouchPhase::Moved),
                    window,
                    cx,
                );
                assert!(
                    w.file_overview
                        .session
                        .as_ref()
                        .unwrap()
                        .gesture
                        .as_ref()
                        .unwrap()
                        .progress()
                        < before.value
                );
                w.file_overview_pinch(&event(0., TouchPhase::Ended), window, cx);
                assert_eq!(
                    w.file_overview.session.as_ref().unwrap().phase,
                    if opening {
                        Phase::Rebounding
                    } else {
                        Phase::Returning
                    }
                );
                assert_eq!(w.document_session().unwrap(), &doc);
            }
            let state = w.file_overview.session.as_mut().unwrap();
            state.phase = Phase::Rebounding;
            state.target_frame = None;
            state.gesture = None;
            state.follow = None;
            state.motion = Some(Motion::settle(Instant::now(), 0.5, 0., 0., false));
            w.file_overview_pinch(
                &gpui::PinchEvent {
                    position,
                    delta: 0.1,
                    phase: TouchPhase::Started,
                    modifiers: Default::default(),
                },
                window,
                cx,
            );
            assert!(
                w.file_overview.session.as_ref().unwrap().gesture.is_none(),
                "an invalidated target cannot be revived by a new contact"
            );
            w.close_file_overview_now(window, cx);
        })
    });
}

#[gpui::test]
fn composition_confirm_is_not_a_file_selection(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    draft(&w, "中文.org", "* A", cx);
    let b = draft(&w, "B.org", "* B", cx);
    open(&w, cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            let input = w.file_overview.session.as_ref().unwrap().query.clone();
            input.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "中文", Some(2..2), window, cx)
            });
            w.overview_key_command("enter", Modifiers::default(), window, cx);
            assert_eq!(w.document_session().unwrap(), &b);
            assert_eq!(
                w.file_overview.session.as_ref().unwrap().phase,
                Phase::Visible
            );
        })
    });
}

#[gpui::test]
fn visible_grid_is_virtualized_and_responsive(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    for index in 0..50 {
        draft(&w, &format!("file-{index}.org"), "* text", cx);
    }
    open(&w, cx);
    assert!(cx.debug_bounds("file-overview-card-0").is_some());
    assert!(cx.debug_bounds("file-overview-card-49").is_none());
    let grid = cx.debug_bounds("file-overview-grid").unwrap();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: grid.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-20_000.))),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("file-overview-card-0").is_none());
    assert!(
        cx.debug_bounds("file-overview-card-49").is_some(),
        "scroll offset: {:?}",
        w.read_with(cx, |w, _| w
            .file_overview
            .session
            .as_ref()
            .unwrap()
            .scroll
            .offset())
    );
    cx.simulate_resize(size(px(390.), px(800.)));
    cx.run_until_parked();
    // Resizing exits the transient view before the underlying document reflows.
    assert!(!w.read_with(cx, |w, _| w.file_overview_is_open()));
    open(&w, cx);
    let a = cx.debug_bounds("file-overview-card-0").unwrap();
    let b = cx.debug_bounds("file-overview-card-1").unwrap();
    assert_eq!(a.center().x, b.center().x);
    assert!(b.top() > a.bottom());
}

#[gpui::test]
fn titlebar_and_card_clicks_work_and_emacs_picker_stays_available(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let a = draft(&w, "A.org", "* A", cx);
    draft(&w, "B.org", "* B", cx);
    cx.run_until_parked();
    let button = cx.debug_bounds("document-titlebar-file-overview").unwrap();
    cx.simulate_click(button.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(w.read_with(cx, |w, _| w.file_overview_is_open()));
    let card = cx.debug_bounds("file-overview-card-1").unwrap();
    cx.simulate_click(card.center(), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        w.read_with(cx, |w, _| w.document_session().unwrap().clone()),
        a
    );
    open(&w, cx);
    cx.simulate_keystrokes("ctrl-x b");
    cx.run_until_parked();
    w.update(cx, |w, _| {
        assert!(!w.file_overview_is_open());
        assert!(w.buffers.panel.is_some());
    });
}

#[gpui::test]
fn command_palette_entry_and_cancel_restore_document_input(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let doc = draft(&w, "A.org", "* A", cx);
    cx.run_until_parked();
    cx.simulate_keystrokes("alt-x");
    cx.simulate_input("file-overview");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(w.read_with(cx, |w, _| w.file_overview_is_open()));
    cx.simulate_input("A");
    cx.simulate_keystrokes("cmd-1");
    cx.run_until_parked();
    assert!(w.read_with(cx, |w, _| w.file_overview_is_open()));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.simulate_input("X");
    cx.run_until_parked();
    let text = doc.read_with(cx, |doc, _| {
        let snapshot = doc.snapshot();
        snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
    });
    assert!(text.contains('X'));
    assert_eq!(text.chars().filter(|c| *c == 'A').count(), 1);
}

#[gpui::test]
fn cold_reading_preparation_is_cancellable_and_only_commits_when_ready(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let a = draft(&w, "A.org", "* A\n\nReading content", cx);
    w.update(cx, |w, cx| w.show_reading(cx));
    let b = draft(&w, "B.org", "* B", cx);
    w.update(cx, |w, cx| {
        w.show_editor(cx);
        assert!(w.overview_preview_request(a.read(cx).id(), cx).is_some());
    });
    open(&w, cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.select_overview_item(a.read(cx).id(), window, cx);
            assert!(w.file_overview.session.as_ref().unwrap().is_preparing());
            assert_eq!(w.document_session().unwrap(), &b);
            w.cancel_file_overview(window, cx);
            complete_motion(w, window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(
        w.read_with(cx, |w, _| w.document_session().unwrap().clone()),
        b
    );
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.select_overview_item(a.read(cx).id(), window, cx)
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| w.update(app, |w, cx| complete_motion(w, window, cx)));
    cx.run_until_parked();
    w.update(cx, |w, _| {
        assert!(!w.file_overview_is_open());
        assert_eq!(w.document_session().unwrap(), &a);
        assert!(w.document_workspace.needs_reading());
        assert!(w.derived.latest.is_some());
    });
}

#[gpui::test]
fn stale_preparation_cannot_replace_newer_unsaved_text(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let a = draft(&w, "A.org", "* A", cx);
    w.update(cx, |w, cx| w.show_reading(cx));
    let b = draft(&w, "B.org", "* B", cx);
    w.update(cx, |w, cx| w.show_editor(cx));
    open(&w, cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.select_overview_item(a.read(cx).id(), window, cx);
            a.update(cx, |doc, cx| {
                doc.edit(
                    DocumentCommand::new(
                        EditTransaction::new(
                            doc.revision(),
                            vec![TextEdit::new(ByteRange::new(2, 3), "New")],
                        ),
                        Selection::default(),
                        Selection::default(),
                        EditOrigin::Other,
                    ),
                    cx,
                )
                .unwrap();
            });
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| w.update(app, |w, cx| complete_motion(w, window, cx)));
    w.update(cx, |w, _| {
        let state = w.file_overview.session.as_ref().unwrap();
        assert_eq!(state.phase, Phase::Visible);
        assert!(state.notice.is_some());
        assert_eq!(w.document_session().unwrap(), &b);
    });
    assert_eq!(
        a.read_with(cx, |doc, _| doc.snapshot().copy_range(ByteRange::new(0, 5))),
        "* New"
    );
}

#[gpui::test]
fn outward_pinch_ignores_header_and_noise_and_can_be_cancelled(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let a = draft(&w, "A.org", "* A", cx);
    let b = draft(&w, "B.org", "* B", cx);
    open(&w, cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            let event = |position, delta, phase| gpui::PinchEvent {
                position,
                delta,
                phase,
                modifiers: Modifiers::default(),
            };
            w.file_overview_pinch(
                &event(point(px(100.), px(70.)), 0.3, TouchPhase::Started),
                window,
                cx,
            );
            assert_eq!(
                w.file_overview.session.as_ref().unwrap().phase,
                Phase::Visible
            );
            let items = w.overview_items(cx);
            let grid = w.overview_grid(window, &items);
            let position = grid
                .document_preview(1, px(0.), w.overview_item_size(&items[1], window))
                .center();
            w.file_overview_pinch(&event(position, 0.01, TouchPhase::Moved), window, cx);
            assert_eq!(
                w.file_overview.session.as_ref().unwrap().phase,
                Phase::Visible
            );
            w.file_overview_pinch(&event(position, 0.15, TouchPhase::Moved), window, cx);
            let state = w.file_overview.session.as_ref().unwrap();
            assert_eq!(state.phase, Phase::Opening);
            assert_eq!(state.target, Some(a.read(cx).id()));
            assert!(state.progress > 0.0);
            assert!(state.motion.is_none());
            assert_eq!(w.document_session().unwrap(), &b);
            w.overview_key_command("escape", Modifiers::default(), window, cx);
            complete_motion(w, window, cx);
            assert_eq!(
                w.file_overview.session.as_ref().unwrap().phase,
                Phase::Visible
            );
            assert_eq!(w.document_session().unwrap(), &b);
        });
    });
}

#[gpui::test]
fn document_frames_and_opening_use_the_original_aspect_ratio(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let a = draft(&w, "portrait.org", "* Portrait", cx);
    draft(&w, "current.org", "* Current", cx);
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        // Exercise size metadata after the retained paint frame was evicted.
        w.file_overview.cache.clear();
        w.file_overview
            .frame_sizes
            .insert(a.read(cx).id(), size(px(300.), px(900.)));
    });
    open(&w, cx);
    let preview = cx.debug_bounds("file-overview-preview-1").unwrap();
    let card = cx.debug_bounds("file-overview-card-1").unwrap();
    assert!((f32::from(preview.size.width / preview.size.height) - 1. / 3.).abs() < 0.0001);
    assert_eq!(card.size.width, preview.size.width);
    assert_eq!(card.size.height, preview.size.height + px(layout::CAPTION));
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.select_overview_item(a.read(cx).id(), window, cx);
            let state = w.file_overview.session.as_ref().unwrap();
            assert_eq!(state.target_from, preview);
            assert!(
                (f32::from(state.target_to.size.width / state.target_to.size.height) - 1. / 3.)
                    .abs()
                    < 0.0001
            );
        })
    });
}

#[gpui::test]
fn handoff_holds_the_frame_until_the_live_target_has_painted(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let target = draft(&w, "target.org", "* Target", cx);
    draft(&w, "origin.org", "* Origin", cx);
    open(&w, cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.select_overview_item(target.read(cx).id(), window, cx);
            cx.set_reduce_motion(false);
            let state = w.file_overview.session.as_mut().unwrap();
            state.motion = Some(Motion::settle(Instant::now(), 1., 1., 0., true));
            w.file_overview_tick(window, cx);
            assert_eq!(w.document_session().unwrap(), &target);
            for paints in [0, 1] {
                w.file_overview.session.as_mut().unwrap().handoff_frames = paints;
                w.file_overview_tick(window, cx);
                let state = w.file_overview.session.as_ref().unwrap();
                assert_eq!(state.phase, Phase::Handoff);
                assert_eq!(state.progress, 0.);
                assert!(state.motion.is_none());
            }
            w.file_overview.session.as_mut().unwrap().handoff_frames = 2;
            w.file_overview_tick(window, cx);
            assert!(w.file_overview.session.as_ref().unwrap().motion.is_some());
            cx.set_reduce_motion(true);
            w.file_overview_tick(window, cx);
            assert!(!w.file_overview_is_open());
        })
    });
    cx.run_until_parked();
    cx.simulate_input("X");
    cx.run_until_parked();
    assert!(target.read_with(cx, |doc, _| {
        let snapshot = doc.snapshot();
        snapshot
            .copy_range(ByteRange::new(0, snapshot.len_bytes()))
            .contains('X')
    }));
}

#[gpui::test]
fn handoff_does_not_count_loading_placeholders_as_ready_paints(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let target = draft(&w, "target.org", "* Target", cx);
    draft(&w, "origin.org", "* Origin", cx);
    open(&w, cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            w.select_overview_item(target.read(cx).id(), window, cx);
            cx.set_reduce_motion(false);
            w.file_overview.session.as_mut().unwrap().motion =
                Some(Motion::settle(Instant::now(), 1., 1., 0., true));
            w.file_overview_tick(window, cx);
            // Simulate a target whose real reading projection is not ready.
            w.document_workspace
                .set_surface(PaneSide::Left, super::super::PaneSurface::Reading);
            w.state.ready_mut().unwrap().readers.left = None;
            cx.notify();
        });
    });
    for _ in 0..3 {
        cx.update(|window, app| window.simulate_next_frame(app));
        cx.run_until_parked();
        w.read_with(cx, |w, _| {
            let state = w.file_overview.session.as_ref().unwrap();
            assert_eq!(state.phase, Phase::Handoff);
            assert_eq!(state.handoff_frames, 0);
            assert_eq!(state.progress, 0.);
            assert!(state.motion.is_none());
        });
    }
    w.update(cx, |w, cx| {
        w.document_workspace
            .set_surface(PaneSide::Left, super::super::PaneSurface::Editor);
        cx.notify();
    });
    for _ in 0..3 {
        cx.update(|window, app| window.simulate_next_frame(app));
        cx.run_until_parked();
    }
    w.read_with(cx, |w, _| {
        let state = w.file_overview.session.as_ref().unwrap();
        assert!(state.handoff_frames >= 2);
        assert!(state.motion.is_some());
    });
}

#[gpui::test]
fn opening_files_during_overview_keeps_the_origin_and_adds_a_background_session(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("added.org");
    std::fs::write(&path, "* Added").unwrap();
    let path = std::fs::canonicalize(path).unwrap();
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    let origin = draft(&w, "A.org", "* A", cx);
    open(&w, cx);
    w.update(cx, |w, cx| w.open(path.clone(), cx));
    cx.run_until_parked();
    w.update(cx, |w, cx| {
        assert!(w.file_overview_is_open());
        assert_eq!(w.document_session().unwrap(), &origin);
        assert!(
            w.overview_items(cx)
                .iter()
                .any(|item| item.path == path.to_string_lossy())
        );
    });
}
