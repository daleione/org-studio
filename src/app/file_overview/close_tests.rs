use super::{
    tests::{complete_motion, draft, finish_opening, open},
    *,
};
use crate::{
    app::{PaneSide, buffers::ReviewKind},
    document::{
        ByteRange, DocumentCommand, EditOrigin, EditTransaction, Selection, TextEdit, TextSnapshot,
    },
};
use gpui::{Modifiers, VisualTestContext};

fn window(cx: &mut gpui::TestAppContext) -> (Entity<WorkspaceWindow>, &mut VisualTestContext) {
    cx.update(crate::editor::init);
    cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false))
}

fn open_grid(w: &Entity<WorkspaceWindow>, cx: &mut VisualTestContext) {
    open(w, cx);
    cx.update(|window, app| {
        w.update(app, |w, cx| {
            complete_motion(w, window, cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
}

fn click(w: &Entity<WorkspaceWindow>, id: DocumentId, cx: &mut VisualTestContext) {
    let index = w.read_with(cx, |w, cx| {
        w.overview_items(cx)
            .iter()
            .position(|item| item.id == id)
            .unwrap()
    });
    let selector = ["file-overview-close-0", "file-overview-close-1"][index];
    let bounds = cx.debug_bounds(selector).unwrap();
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
}

fn press(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx.debug_bounds(selector).unwrap();
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
}

fn clear(session: &Entity<DocumentSession>, cx: &mut VisualTestContext) {
    session.update(cx, |doc, cx| {
        doc.edit(
            DocumentCommand::new(
                EditTransaction::new(
                    doc.revision(),
                    vec![TextEdit::new(
                        ByteRange::new(0, doc.snapshot().len_bytes()),
                        "",
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
    cx.run_until_parked();
}

#[gpui::test]
fn an_empty_draft_closes_directly_and_leaves_the_new_file_card(cx: &mut gpui::TestAppContext) {
    let (w, cx) = window(cx);
    cx.update(|window, app| w.update(app, |w, cx| w.new_draft(window, cx)));
    cx.run_until_parked();
    let id = w.read_with(cx, |w, cx| w.document_session().unwrap().read(cx).id());
    open_grid(&w, cx);
    click(&w, id, cx);
    w.read_with(cx, |w, cx| {
        assert!(w.buffers.review().is_none());
        assert_eq!(w.buffer_sessions().count(), 0);
        let state = w.file_overview.session.as_ref().unwrap();
        assert_eq!(state.phase, Phase::Visible);
        assert_eq!(state.origin, None);
        assert!(w.overview_items(cx).is_empty());
    });
    assert!(cx.debug_bounds("file-overview-new-file").is_some());
    assert!(cx.debug_bounds("file-overview-close-0").is_none());
}

#[gpui::test]
fn a_draft_typed_then_cleared_closes_and_done_returns_to_the_remaining_document(
    cx: &mut gpui::TestAppContext,
) {
    let (w, cx) = window(cx);
    let keep = draft(&w, "Keep.org", "* Keep", cx);
    let empty = draft(&w, "Empty.org", "temporary text", cx);
    clear(&empty, cx);
    assert!(empty.read_with(cx, |doc, _| doc.is_dirty()));
    open_grid(&w, cx);
    click(&w, empty.read_with(cx, |doc, _| doc.id()), cx);
    w.read_with(cx, |w, cx| {
        assert!(w.buffers.review().is_none());
        assert_eq!(w.buffer_sessions().count(), 1);
        assert_eq!(w.document_session(), Some(&keep));
        let state = w.file_overview.session.as_ref().unwrap();
        assert_eq!(state.phase, Phase::Visible);
        assert_eq!(state.origin, Some(keep.read(cx).id()));
        assert!(state.origin_frame.is_none());
    });
    cx.simulate_keystrokes("escape");
    cx.update(|window, app| w.update(app, |w, cx| complete_motion(w, window, cx)));
    cx.run_until_parked();
    assert!(!w.read_with(cx, |w, _| w.file_overview_is_open()));
    assert_eq!(
        w.read_with(cx, |w, _| w.document_session().cloned()),
        Some(keep)
    );
}

#[gpui::test]
fn closing_a_dirty_background_card_can_cancel_without_opening_or_editing_it(
    cx: &mut gpui::TestAppContext,
) {
    let (w, cx) = window(cx);
    let target = draft(&w, "Background.org", "keep background", cx);
    let current = draft(&w, "Current.org", "keep current", cx);
    open_grid(&w, cx);
    cx.simulate_input("org");
    cx.run_until_parked();
    let (query, offset) = w.read_with(cx, |w, _| {
        let state = w.file_overview.session.as_ref().unwrap();
        (state.query.clone(), state.scroll.offset())
    });
    click(&w, target.read_with(cx, |doc, _| doc.id()), cx);
    assert!(
        cx.debug_bounds("file-overview-close-confirmation")
            .is_some()
    );
    w.read_with(cx, |w, cx| {
        assert_eq!(w.document_session(), Some(&current));
        assert_eq!(
            w.buffers.review().unwrap().kind,
            ReviewKind::Close(target.read(cx).id())
        );
        assert_eq!(
            w.file_overview.session.as_ref().unwrap().phase,
            Phase::Visible
        );
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("file-overview-close-confirmation")
            .is_none()
    );
    w.read_with(cx, |w, cx| {
        assert_eq!(w.document_session(), Some(&current));
        assert_eq!(w.buffer_sessions().count(), 2);
        let state = w.file_overview.session.as_ref().unwrap();
        assert_eq!(state.query, query);
        assert_eq!(state.scroll.offset(), offset);
        assert_eq!(query.read(cx).text, "org");
    });
    cx.simulate_input("x");
    cx.run_until_parked();
    assert_eq!(query.read_with(cx, |query, _| query.text.clone()), "orgx");
    assert_eq!(
        current.read_with(cx, |doc, _| doc
            .snapshot()
            .copy_range(ByteRange::new(0, doc.snapshot().len_bytes()))),
        "keep current"
    );
}

#[gpui::test]
fn discard_closes_only_the_chosen_card_and_keeps_the_overview(cx: &mut gpui::TestAppContext) {
    let (w, cx) = window(cx);
    let target = draft(&w, "Discard.org", "discard this", cx);
    let keep = draft(&w, "Keep.org", "keep this", cx);
    open_grid(&w, cx);
    let id = target.read_with(cx, |doc, _| doc.id());
    click(&w, id, cx);
    press("buffer-review-discard-all", cx);
    w.read_with(cx, |w, cx| {
        assert!(w.buffers.review().is_none());
        assert!(w.buffer_session(id, cx).is_none());
        assert_eq!(w.document_session(), Some(&keep));
        assert_eq!(w.buffer_sessions().count(), 1);
        assert_eq!(
            w.file_overview.session.as_ref().unwrap().phase,
            Phase::Visible
        );
        assert!(keep.read(cx).is_dirty());
    });
}

#[gpui::test]
fn a_new_draft_stays_open_if_save_is_cancelled_and_closes_after_success(
    cx: &mut gpui::TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Saved.org");
    let (w, cx) = window(cx);
    cx.update(|window, app| w.update(app, |w, cx| w.new_draft(window, cx)));
    cx.run_until_parked();
    cx.simulate_input("* Save this\n");
    cx.run_until_parked();
    let target = w.read_with(cx, |w, _| w.document_session().unwrap().clone());
    let id = target.read_with(cx, |doc, _| doc.id());
    open_grid(&w, cx);
    click(&w, id, cx);
    press("buffer-review-submit", cx);
    cx.cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        assert!(w.buffer_session(id, cx).is_some());
        assert!(w.buffers.review().is_some());
        assert!(target.read(cx).is_dirty());
        assert!(target.read(cx).file_path().is_none());
    });
    press("buffer-review-cancel", cx);
    click(&w, id, cx);
    press("buffer-review-submit", cx);
    cx.cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "* Save this\n");
    w.read_with(cx, |w, cx| {
        assert!(w.buffer_session(id, cx).is_none());
        assert!(w.buffers.review().is_none());
        assert!(w.file_overview_is_open());
        assert_eq!(w.buffer_sessions().count(), 0);
        assert!(!target.read(cx).is_dirty());
    });
    assert!(cx.debug_bounds("file-overview-new-file").is_some());
}

#[gpui::test]
fn clearing_a_saved_file_still_requires_saving_and_can_save_in_the_background(
    cx: &mut gpui::TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Existing.org");
    std::fs::write(&path, "saved text").unwrap();
    let (w, cx) = window(cx);
    let loaded = crate::preview::load_document(path.clone()).unwrap();
    w.update(cx, |w, cx| {
        assert!(w.apply_load_result(w.generation, Ok(loaded), cx))
    });
    cx.run_until_parked();
    let target = w.read_with(cx, |w, _| w.document_session().unwrap().clone());
    clear(&target, cx);
    let keep = draft(&w, "Keep.org", "keep these unsaved edits", cx);
    open_grid(&w, cx);
    let id = target.read_with(cx, |doc, _| doc.id());
    click(&w, id, cx);
    assert!(
        cx.debug_bounds("file-overview-close-confirmation")
            .is_some()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "saved text");
    press("buffer-review-submit", cx);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    w.read_with(cx, |w, cx| {
        assert!(w.buffer_session(id, cx).is_none());
        assert_eq!(w.document_session(), Some(&keep));
        assert!(w.file_overview_is_open());
        assert_eq!(w.editor(PaneSide::Left).unwrap().read(cx).session(), &keep);
    });
}

#[gpui::test]
fn a_failed_save_keeps_the_card_and_shows_the_review_error(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing").join("Unsaved.org");
    let (w, cx) = window(cx);
    let target = draft(&w, "Unsaved.org", "must survive", cx);
    let id = target.read_with(cx, |doc, _| doc.id());
    open_grid(&w, cx);
    click(&w, id, cx);
    press("buffer-review-submit", cx);
    cx.cx
        .simulate_path_prompt_response(|_| Some(vec![path.parent().unwrap().to_path_buf()]));
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        assert!(w.file_overview_is_open());
        assert!(w.buffer_session(id, cx).is_some());
        let review = w.buffers.review().unwrap();
        assert!(!review.running && review.error.is_some());
        let snapshot = target.read(cx).snapshot();
        assert_eq!(
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
            "must survive"
        );
        assert!(target.read(cx).is_dirty());
    });
    assert!(!path.exists());
    assert!(
        cx.debug_bounds("file-overview-close-confirmation")
            .is_some()
    );
}

#[gpui::test]
fn a_named_draft_selects_only_a_folder_and_can_retry_a_name_collision(
    cx: &mut gpui::TestAppContext,
) {
    let occupied = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let name = "未命名 3.org"; // An explicitly chosen name can look like an automatic name.
    let existing = occupied.path().join(name);
    std::fs::write(&existing, "existing disk text").unwrap();
    let (w, cx) = window(cx);
    let target = draft(&w, name, "keep this draft", cx);
    let id = target.read_with(cx, |doc, _| doc.id());
    open_grid(&w, cx);
    click(&w, id, cx);
    press("buffer-review-submit", cx);
    assert!(cx.cx.did_prompt_for_paths() && !cx.cx.did_prompt_for_new_path());
    cx.cx.simulate_path_prompt_response(|options| {
        assert!(options.directories && !options.files && !options.multiple);
        assert!(options.prompt.as_ref().unwrap().contains(name));
        None
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("file-overview-close-confirmation")
            .is_some()
    );
    assert_eq!(target.read_with(cx, |doc, _| doc.display_name()), name);
    press("buffer-review-submit", cx);
    cx.cx
        .simulate_path_prompt_response(|_| Some(vec![occupied.path().to_path_buf()]));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(&existing).unwrap(),
        "existing disk text"
    );
    assert!(cx.debug_bounds("buffer-review-error").is_some());
    target.read_with(cx, |doc, _| assert!(doc.is_named_draft() && doc.is_dirty()));
    press("buffer-review-submit", cx);
    cx.cx
        .simulate_path_prompt_response(|_| Some(vec![destination.path().to_path_buf()]));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(destination.path().join(name)).unwrap(),
        "keep this draft"
    );
    w.read_with(cx, |w, cx| {
        assert!(w.buffer_session(id, cx).is_none());
        assert!(w.buffers.review().is_none());
        assert!(w.file_overview_is_open());
    });
}

#[gpui::test]
fn a_named_draft_cannot_replace_an_open_document(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Notes.org");
    std::fs::write(&path, "existing open file").unwrap();
    let (w, cx) = window(cx);
    let loaded = crate::preview::load_document(path.clone()).unwrap();
    w.update(cx, |w, cx| {
        assert!(w.apply_load_result(w.generation, Ok(loaded), cx))
    });
    cx.run_until_parked();
    let existing = w.read_with(cx, |w, _| w.document_session().unwrap().clone());
    let target = draft(&w, "Notes.org", "new draft content", cx);
    let id = target.read_with(cx, |doc, _| doc.id());
    open_grid(&w, cx);
    click(&w, id, cx);
    press("buffer-review-submit", cx);
    cx.cx
        .simulate_path_prompt_response(|_| Some(vec![directory.path().to_path_buf()]));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "existing open file"
    );
    w.read_with(cx, |w, cx| {
        assert!(w.buffers.review().unwrap().error.is_some());
        assert_eq!(w.buffer_sessions().count(), 2);
        assert_eq!(w.buffer_for_path(&path, cx), Some(existing.clone()));
        assert!(target.read(cx).is_named_draft() && target.read(cx).is_dirty());
    });
}

#[gpui::test]
fn discarding_saved_changes_leaves_the_disk_file_untouched(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Existing.org");
    std::fs::write(&path, "saved text").unwrap();
    let (w, cx) = window(cx);
    let loaded = crate::preview::load_document(path.clone()).unwrap();
    w.update(cx, |w, cx| {
        assert!(w.apply_load_result(w.generation, Ok(loaded), cx))
    });
    cx.run_until_parked();
    let target = w.read_with(cx, |w, _| w.document_session().unwrap().clone());
    clear(&target, cx);
    let id = target.read_with(cx, |doc, _| doc.id());
    open_grid(&w, cx);
    click(&w, id, cx);
    press("buffer-review-discard-all", cx);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "saved text");
    assert!(!cx.cx.did_prompt_for_paths() && !cx.cx.did_prompt_for_new_path());
    w.read_with(cx, |w, cx| {
        assert!(w.buffer_session(id, cx).is_none());
        assert!(w.file_overview_is_open());
    });
}

#[gpui::test]
fn confirmation_only_covers_its_card_and_other_close_buttons_still_work(
    cx: &mut gpui::TestAppContext,
) {
    let (w, cx) = window(cx);
    let background = draft(&w, "Background.org", "background text", cx);
    let current = draft(&w, "Current.org", "current text", cx);
    open_grid(&w, cx);
    click(&w, current.read_with(cx, |doc, _| doc.id()), cx);
    let mask = cx.debug_bounds("file-overview-close-confirmation").unwrap();
    assert_eq!(mask, cx.debug_bounds("file-overview-preview-1").unwrap());
    assert!(cx.debug_bounds("buffer-review-modal").is_none());
    assert!(cx.debug_bounds("file-overview-close-1").is_none());
    assert!(cx.debug_bounds("file-overview-close-0").is_some());
    click(&w, background.read_with(cx, |doc, _| doc.id()), cx);
    assert_eq!(
        cx.debug_bounds("file-overview-close-confirmation").unwrap(),
        cx.debug_bounds("file-overview-preview-0").unwrap()
    );
    w.read_with(cx, |w, cx| {
        assert_eq!(
            w.buffers.inline_close_target(),
            Some(background.read(cx).id())
        );
        assert_eq!(w.document_session(), Some(&current));
    });
    cx.simulate_click(point(px(5.), px(200.)), Modifiers::default());
    cx.run_until_parked();
    assert!(w.read_with(cx, |w, _| w.buffers.review().is_none()));
    assert!(
        cx.debug_bounds("file-overview-close-confirmation")
            .is_none()
    );
    assert_eq!(w.read_with(cx, |w, _| w.buffer_sessions().count()), 2);
}

#[gpui::test]
fn pending_close_yields_to_opening_another_card_and_creating_a_draft(
    cx: &mut gpui::TestAppContext,
) {
    let (w, cx) = window(cx);
    let target = draft(&w, "Background.org", "keep this content", cx);
    let current = draft(&w, "Current.org", "current content", cx);
    open_grid(&w, cx);
    click(&w, target.read_with(cx, |doc, _| doc.id()), cx);
    let card = cx.debug_bounds("file-overview-preview-1").unwrap();
    cx.simulate_click(card.center(), Modifiers::default());
    finish_opening(&w, cx);
    w.read_with(cx, |w, cx| {
        assert_eq!(w.document_session(), Some(&current));
        assert!(w.buffers.review().is_none());
        assert!(target.read(cx).is_dirty());
    });
    open_grid(&w, cx);
    click(&w, target.read_with(cx, |doc, _| doc.id()), cx);
    cx.simulate_keystrokes("cmd-n");
    finish_opening(&w, cx);
    w.read_with(cx, |w, cx| {
        assert_eq!(w.buffer_sessions().count(), 3);
        assert!(w.buffers.review().is_none());
        let created = w.document_session().unwrap().read(cx);
        assert!(created.is_draft() && created.snapshot().len_bytes() == 0);
        assert!(w.buffer_session(target.read(cx).id(), cx).is_some());
    });
}

#[gpui::test]
fn a_committed_card_save_survives_navigation_and_only_closes_its_original_document(
    cx: &mut gpui::TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let (w, cx) = window(cx);
    let target = draft(&w, "Saving.org", "save this original document", cx);
    draft(&w, "Keep.org", "keep this document", cx);
    let id = target.read_with(cx, |doc, _| doc.id());
    open_grid(&w, cx);
    click(&w, id, cx);
    press("buffer-review-submit", cx);
    let opened = w.update(cx, |w, cx| {
        w.create_buffer("Opened meanwhile.org".into(), None, cx);
        w.document_session().unwrap().clone()
    });
    cx.run_until_parked();
    assert!(!w.read_with(cx, |w, _| w.file_overview_is_open()));
    assert!(cx.debug_bounds("buffer-review-modal").is_none());
    w.read_with(cx, |w, _| {
        assert!(w.buffers.review().is_some_and(|r| r.running && r.inline))
    });
    cx.cx
        .simulate_path_prompt_response(|_| Some(vec![directory.path().to_path_buf()]));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(directory.path().join("Saving.org")).unwrap(),
        "save this original document"
    );
    w.read_with(cx, |w, cx| {
        assert_eq!(w.document_session(), Some(&opened));
        assert!(w.buffer_session(id, cx).is_none());
        assert!(w.buffers.review().is_none());
        assert_eq!(w.buffer_sessions().count(), 2);
    });
}

#[gpui::test]
fn inline_close_yields_to_filtering_and_batch_save_review(cx: &mut gpui::TestAppContext) {
    let (w, cx) = window(cx);
    let background = draft(&w, "Background.org", "keep background text", cx);
    let current = draft(&w, "Current.org", "keep current text", cx);
    open_grid(&w, cx);
    click(&w, background.read_with(cx, |doc, _| doc.id()), cx);
    let query = w.read_with(cx, |w, _| {
        w.file_overview.session.as_ref().unwrap().query_bounds
    });
    cx.simulate_click(query.center(), Modifiers::default());
    cx.simulate_input("Current");
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        assert!(w.buffers.review().is_none());
        assert_eq!(
            w.file_overview
                .session
                .as_ref()
                .unwrap()
                .query
                .read(cx)
                .text,
            "Current"
        );
        assert_eq!(w.document_session(), Some(&current));
        assert_eq!(w.buffer_sessions().count(), 2);
    });
    w.update(cx, |w, cx| {
        w.request_close_buffer(current.read(cx).id(), cx)
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("file-overview-close-confirmation")
            .is_some()
    );
    w.update(cx, |w, cx| w.begin_buffer_review(ReviewKind::Save, cx));
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        let review = w.buffers.review().unwrap();
        assert_eq!(review.kind, ReviewKind::Save);
        assert!(!review.inline);
        assert_eq!(review.entries.len(), 2);
        assert!(current.read(cx).is_dirty() && background.read(cx).is_dirty());
    });
    assert!(
        cx.debug_bounds("file-overview-close-confirmation")
            .is_none()
    );
}

#[gpui::test]
fn scrolling_a_pending_close_out_of_view_cancels_it_without_losing_the_document(
    cx: &mut gpui::TestAppContext,
) {
    let (w, cx) = window(cx);
    cx.simulate_resize(size(px(390.), px(600.)));
    for i in 0..8 {
        draft(&w, &format!("Background {i}.org"), "keep", cx);
    }
    let target = draft(&w, "Current.org", "keep this too", cx);
    let id = target.read_with(cx, |doc, _| doc.id());
    open_grid(&w, cx);
    let close = cx.debug_bounds("file-overview-close-8").unwrap();
    cx.simulate_click(close.center(), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        w.read_with(cx, |w, _| w.buffers.inline_close_target()),
        Some(id)
    );
    w.update(cx, |w, cx| {
        w.file_overview
            .session
            .as_ref()
            .unwrap()
            .scroll
            .set_offset(point(px(0.), px(0.)));
        cx.notify();
    });
    cx.run_until_parked();
    w.read_with(cx, |w, cx| {
        assert!(w.buffers.review().is_none());
        assert!(w.buffer_session(id, cx).is_some());
        assert_eq!(w.buffer_sessions().count(), 9);
    });
    assert!(
        cx.debug_bounds("file-overview-close-confirmation")
            .is_none()
    );
}
