//! Generated text uses the normal editor's shaping, hit testing and selection.
use super::*;
use gpui::{HighlightStyle, TextRun};

pub(crate) type LineHighlights = Vec<(Range<usize>, HighlightStyle)>;

#[derive(Clone)]
pub(crate) struct GeneratedTextProjection<T> {
    pub(crate) version: u64,
    pub(crate) text: String,
    pub(crate) highlights: Vec<LineHighlights>,
    pub(crate) targets: Vec<Option<T>>,
}

pub(crate) struct GeneratedProjectionController<T> {
    next_version: u64,
    latest_request: u64,
    closed: bool,
    current: Option<std::sync::Arc<GeneratedTextProjection<T>>>,
}

/// Lifecycle and command boundary shared by generated text consumers.  The instance id is part
/// of every publication token, so a completion from a closed view can never be accepted by a
/// later view which happens to reuse the same provider.
pub(crate) struct GeneratedTextView<T> {
    instance: u64,
    visible: bool,
    controller: GeneratedProjectionController<T>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneratedRequest {
    instance: u64,
    request: u64,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneratedViewTarget {
    instance: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GeneratedCommand {
    Activate,
    Refresh,
    Save,
    Undo,
    Redo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandDisposition {
    Enabled,
    Disabled,
    Unhandled,
}

impl<T> Default for GeneratedTextView<T> {
    fn default() -> Self {
        Self {
            instance: 1,
            visible: false,
            controller: Default::default(),
        }
    }
}

impl<T> GeneratedTextView<T> {
    pub(crate) fn mount(&mut self) {
        if self.controller.closed {
            self.reopen();
        } else {
            self.visible = true;
        }
    }

    pub(crate) fn hide(&mut self) {
        self.visible = false;
    }

    pub(crate) fn close(&mut self) {
        self.visible = false;
        self.controller.close();
    }

    pub(crate) fn reopen(&mut self) {
        self.instance = self
            .instance
            .checked_add(1)
            .expect("generated view id exhausted");
        self.controller = Default::default();
        self.visible = true;
    }

    pub(crate) fn begin_request(&mut self) -> GeneratedRequest {
        GeneratedRequest {
            instance: self.instance,
            request: self.controller.begin_request(),
        }
    }

    pub(crate) fn publish_request(
        &mut self,
        token: GeneratedRequest,
        text: String,
        highlights: Vec<LineHighlights>,
        targets: Vec<Option<T>>,
    ) -> Option<std::sync::Arc<GeneratedTextProjection<T>>> {
        (token.instance == self.instance).then_some(())?;
        self.controller
            .publish_request(token.request, text, highlights, targets)
    }

    pub(crate) fn publish(
        &mut self,
        text: String,
        highlights: Vec<LineHighlights>,
        targets: Vec<Option<T>>,
    ) -> Option<std::sync::Arc<GeneratedTextProjection<T>>> {
        let request = self.begin_request();
        self.publish_request(request, text, highlights, targets)
    }

    pub(crate) fn resolve(&self, version: u64, line: usize) -> Option<&T> {
        self.visible.then_some(())?;
        self.controller.resolve(version, line)
    }

    pub(crate) fn command(&self, command: GeneratedCommand) -> CommandDisposition {
        match command {
            GeneratedCommand::Save | GeneratedCommand::Undo | GeneratedCommand::Redo => {
                CommandDisposition::Disabled
            }
            GeneratedCommand::Activate | GeneratedCommand::Refresh => CommandDisposition::Enabled,
        }
    }

    #[cfg(test)]
    pub(crate) fn capture_target(&self) -> Option<GeneratedViewTarget> {
        self.visible.then_some(GeneratedViewTarget {
            instance: self.instance,
        })
    }

    #[cfg(test)]
    pub(crate) fn command_for(
        &self,
        target: GeneratedViewTarget,
        command: GeneratedCommand,
    ) -> CommandDisposition {
        if !self.visible || target.instance != self.instance {
            CommandDisposition::Unhandled
        } else {
            self.command(command)
        }
    }
}

impl<T> Default for GeneratedProjectionController<T> {
    fn default() -> Self {
        Self {
            next_version: 0,
            latest_request: 0,
            closed: false,
            current: None,
        }
    }
}

impl<T> GeneratedProjectionController<T> {
    pub(crate) fn begin_request(&mut self) -> u64 {
        self.latest_request = self
            .latest_request
            .checked_add(1)
            .expect("projection request exhausted");
        self.latest_request
    }

    pub(crate) fn publish_request(
        &mut self,
        request: u64,
        text: String,
        highlights: Vec<LineHighlights>,
        targets: Vec<Option<T>>,
    ) -> Option<std::sync::Arc<GeneratedTextProjection<T>>> {
        if self.closed || request != self.latest_request {
            return None;
        }
        self.next_version = self
            .next_version
            .checked_add(1)
            .expect("projection version exhausted");
        let projection = std::sync::Arc::new(GeneratedTextProjection {
            version: self.next_version,
            text,
            highlights,
            targets,
        });
        self.current = Some(projection.clone());
        Some(projection)
    }

    pub(crate) fn resolve(&self, version: u64, line: usize) -> Option<&T> {
        let projection = self.current.as_ref()?;
        if projection.version != version {
            return None;
        }
        projection.targets.get(line)?.as_ref()
    }

    pub(crate) fn close(&mut self) {
        self.closed = true;
        self.current = None;
    }
}

impl SemanticEditor {
    #[cfg(test)]
    pub(crate) fn new_read_only(
        text: String,
        highlights: Vec<LineHighlights>,
        cx: &mut Context<Self>,
    ) -> Self {
        let session = cx.new(|_| DocumentSession::read_only_text(text));
        let mut editor = Self::new_with_autofocus(session, false, cx);
        editor.generated_highlights = Some(highlights);
        editor.display_map.set_soft_wrap(false);
        editor.minimap.visible = false;
        editor.minimap.reveal = 0.0;
        editor.inline_image_previews = false;
        editor
    }

    pub(crate) fn new_activatable_read_only(
        text: String,
        highlights: Vec<LineHighlights>,
        display_name: &'static str,
        producer: &'static str,
        cx: &mut Context<Self>,
    ) -> Self {
        let session = cx.new(|_| DocumentSession::generated_text(text, display_name, producer));
        let mut editor = Self::new_with_autofocus(session, false, cx);
        editor.generated_highlights = Some(highlights);
        editor.display_map.set_soft_wrap(false);
        editor.minimap.visible = false;
        editor.minimap.reveal = 0.0;
        editor.inline_image_previews = false;
        editor.activate_read_only_lines = true;
        editor
    }

    pub(crate) fn update_read_only(
        &mut self,
        text: String,
        highlights: Vec<LineHighlights>,
        cx: &mut Context<Self>,
    ) {
        let snapshot = self.snapshot(cx);
        let changed = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())) != text;
        self.generated_highlights = Some(highlights);
        self.shape_cache.clear();
        if changed {
            let selection = self.selection;
            let scroll_x = self.scroll_x;
            let scroll_y = self.scroll_y;
            self.session
                .update(cx, |session, cx| session.publish_generated_text(text, cx));
            let len = self.snapshot(cx).len_bytes();
            self.set_selection(
                Selection::new(
                    ByteOffset(selection.anchor().0.min(len)),
                    ByteOffset(selection.head().0.min(len)),
                ),
                cx,
            );
            self.scroll_x = scroll_x;
            self.scroll_y = scroll_y;
        }
        cx.notify();
    }

    pub(crate) fn is_read_only(&self, cx: &App) -> bool {
        self.session.read(cx).is_read_only()
    }

    pub(crate) fn selected_line(&self, cx: &App) -> u64 {
        self.snapshot(cx)
            .line_index_at(self.selection.head())
            .map_or(0, |line| line.0)
    }

    pub(crate) fn select_read_only_line(&mut self, line: usize, cx: &mut Context<Self>) {
        let snapshot = self.snapshot(cx);
        let offset = snapshot
            .line_range(crate::document::LineIndex(line as u64))
            .map_or(snapshot.len_bytes(), |range| range.start.0);
        self.set_selection(Selection::caret(ByteOffset(offset)), cx);
    }
}

pub(super) fn highlighted_runs(base: TextRun, highlights: &LineHighlights) -> Vec<TextRun> {
    let mut boundaries = vec![0, base.len];
    for (range, _) in highlights {
        boundaries.extend([range.start.min(base.len), range.end.min(base.len)]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
        .windows(2)
        .map(|bounds| {
            let mut run = base.clone();
            run.len = bounds[1] - bounds[0];
            for (range, style) in highlights {
                if range.start <= bounds[0] && range.end >= bounds[1] {
                    if let Some(color) = style.color {
                        run.color = color;
                    }
                    if let Some(weight) = style.font_weight {
                        run.font.weight = weight;
                    }
                }
            }
            run
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::EntityInputHandler;

    #[test]
    fn projection_version_changes_when_only_targets_change() {
        let mut controller = GeneratedProjectionController::default();
        let first_request = controller.begin_request();
        let first = controller
            .publish_request(first_request, "same".into(), vec![], vec![Some("old")])
            .unwrap();
        let second_request = controller.begin_request();
        let second = controller
            .publish_request(second_request, "same".into(), vec![], vec![Some("new")])
            .unwrap();
        assert_ne!(first.version, second.version);
        assert!(controller.resolve(first.version, 0).is_none());
        assert_eq!(controller.resolve(second.version, 0), Some(&"new"));
    }

    #[test]
    fn non_agenda_result_fixture_rejects_out_of_order_and_closed_publications() {
        #[derive(Clone, Debug, Eq, PartialEq)]
        struct DiagnosticTarget {
            path: &'static str,
            byte: u64,
        }
        let mut controller = GeneratedProjectionController::default();
        let old = controller.begin_request();
        let new = controller.begin_request();
        let current = controller
            .publish_request(
                new,
                "warning\n".into(),
                vec![],
                vec![Some(DiagnosticTarget {
                    path: "/tmp/a.org",
                    byte: 7,
                })],
            )
            .unwrap();
        assert!(
            controller
                .publish_request(old, "stale\n".into(), vec![], vec![None])
                .is_none()
        );
        assert_eq!(controller.resolve(current.version, 0).unwrap().byte, 7);
        controller.close();
        assert!(
            controller
                .publish_request(new, "late\n".into(), vec![], vec![None])
                .is_none()
        );
        assert!(controller.resolve(current.version, 0).is_none());
    }

    #[test]
    fn generic_fixture_uses_the_full_view_lifecycle_and_command_boundary() {
        #[derive(Clone, Debug, Eq, PartialEq)]
        struct DiagnosticTarget(&'static str, u64);
        let mut view = GeneratedTextView::default();
        view.mount();
        let old = view.begin_request();
        let new = view.begin_request();
        let current = view
            .publish_request(
                new,
                "same warning\n".into(),
                vec![vec![]],
                vec![Some(DiagnosticTarget("a.org", 4))],
            )
            .unwrap();
        assert!(
            view.publish_request(old, "old\n".into(), vec![], vec![])
                .is_none()
        );
        assert_eq!(
            view.resolve(current.version, 0),
            Some(&DiagnosticTarget("a.org", 4))
        );
        assert_eq!(
            view.command(GeneratedCommand::Refresh),
            CommandDisposition::Enabled
        );
        assert_eq!(
            view.command(GeneratedCommand::Save),
            CommandDisposition::Disabled
        );
        let menu_target = view.capture_target().unwrap();

        let closed_request = view.begin_request();
        view.close();
        assert_eq!(
            view.command_for(menu_target, GeneratedCommand::Save),
            CommandDisposition::Unhandled
        );
        view.reopen();
        assert_eq!(
            view.command_for(menu_target, GeneratedCommand::Activate),
            CommandDisposition::Unhandled
        );
        assert!(
            view.publish_request(closed_request, "late\n".into(), vec![], vec![])
                .is_none()
        );
        let changed_map = view
            .publish(
                "same warning\n".into(),
                vec![vec![]],
                vec![Some(DiagnosticTarget("b.org", 9))],
            )
            .unwrap();
        assert_eq!(
            view.resolve(changed_map.version, 0),
            Some(&DiagnosticTarget("b.org", 9))
        );
    }

    #[test]
    fn mounting_a_closed_view_creates_a_new_instance_before_sync_publication() {
        let mut view = GeneratedTextView::<u64>::default();
        view.mount();
        let old_target = view.capture_target().unwrap();
        view.close();
        view.mount();
        let projection = view
            .publish("reopened\n".into(), vec![vec![]], vec![Some(7)])
            .unwrap();
        assert_eq!(view.resolve(projection.version, 0), Some(&7));
        assert_eq!(
            view.command_for(old_target, GeneratedCommand::Activate),
            CommandDisposition::Unhandled
        );
    }

    #[gpui::test]
    fn generated_buffer_selects_and_copies_but_rejects_input_and_saving(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(super::super::init);
        let text = "Monday 7 September 2026 W37\n  工作: Scheduled: TODO 中文任务\n";
        let (editor, cx) =
            cx.add_window_view(|_, cx| SemanticEditor::new_read_only(text.into(), vec![], cx));
        cx.update(|window, app| {
            let focus = editor.read(app).focus_handle.clone();
            window.focus(&focus, app);
        });
        cx.simulate_keystrokes("cmd-a cmd-c");
        cx.run_until_parked();
        cx.update(|_, app| assert_eq!(app.read_from_clipboard().unwrap().text().unwrap(), text));
        cx.simulate_keystrokes("x backspace delete cmd-v cmd-x cmd-z cmd-shift-z tab shift-tab");
        cx.update(|window, app| {
            editor.update(app, |editor, cx| {
                editor.replace_and_mark_text_in_range(None, "输入", Some(2..2), window, cx);
                editor.replace_text_in_range(None, "替换", window, cx);
                let session = editor.session.read(cx);
                assert_eq!(
                    session
                        .snapshot()
                        .copy_range(ByteRange::new(0, session.snapshot().len_bytes())),
                    text
                );
                assert!(!session.is_dirty());
                assert!(session.is_generated());
                assert!(session.file_path().is_none());
                assert_eq!(
                    session.generated_source(),
                    Some(&crate::document::GeneratedSource {
                        producer: std::sync::Arc::from("editor.generated"),
                        display_name: std::sync::Arc::from("Generated"),
                    })
                );
                assert!(matches!(
                    session.save_request(None),
                    Err(crate::document::SaveStartError::ReadOnly)
                ));
                assert!(matches!(
                    session.save_request(Some("/tmp/should-not-save.org".into())),
                    Err(crate::document::SaveStartError::ReadOnly)
                ));
                assert!(editor.marked.is_none());
                assert!(!editor.soft_wrap());
                assert!(!editor.minimap.visible);
            });
        });
        cx.simulate_keystrokes("cmd-up down");
        cx.run_until_parked();
        editor.update(cx, |editor, cx| assert_eq!(editor.selected_line(cx), 1));
        editor.update(cx, |editor, cx| {
            editor.update_read_only(text.into(), vec![], cx)
        });
        cx.run_until_parked();
        editor.update(cx, |editor, cx| assert_eq!(editor.selected_line(cx), 1));
        editor.update(cx, |editor, cx| {
            editor.update_read_only("Changed\n".into(), vec![], cx)
        });
        cx.run_until_parked();
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.selected_line(cx), 1);
            assert!(!editor.session.read(cx).is_dirty());
        });
    }

    #[gpui::test]
    fn generated_session_rejects_direct_edits(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| DocumentSession::read_only_text("original".into()));
        session.update(cx, |session, cx| {
            let transaction = EditTransaction::new(
                session.revision(),
                vec![TextEdit::new(ByteRange::new(0, 0), "bad")],
            );
            assert!(matches!(
                session.apply_transient_edit(transaction.clone(), cx),
                Err(crate::document::EditError::ReadOnly)
            ));
            let command = DocumentCommand::new(
                transaction,
                Selection::default(),
                Selection::default(),
                EditOrigin::Typing,
            );
            assert!(matches!(
                session.edit(command, cx),
                Err(crate::document::EditError::ReadOnly)
            ));
            assert!(session.undo(cx).is_err());
            assert!(session.redo(cx).is_err());
            assert!(matches!(
                session.begin_force_save(),
                Err(crate::document::SaveStartError::ReadOnly)
            ));
            assert_eq!(session.newline_sequence(), "\n");
            assert_eq!(
                session.observe_disk(None),
                crate::document::DiskChangeAction::Ignore
            );
            assert!(matches!(
                session.reload_request(),
                Err(crate::document::ReloadError::NoFileBackend)
            ));
            assert!(
                session
                    .retarget_moved_file(
                        "/tmp/generated-must-not-exist".into(),
                        crate::document::FileStamp::detached(b""),
                        None,
                        cx,
                    )
                    .is_err()
            );
        });
    }

    #[gpui::test]
    fn enter_activates_the_caret_line_without_inserting_a_newline(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let activated = std::rc::Rc::new(std::cell::Cell::new(None));
        let editor = cx.new(|cx| {
            SemanticEditor::new_activatable_read_only(
                "Header\nTask\n".into(),
                vec![],
                "Test result",
                "test.activatable",
                cx,
            )
        });
        struct Harness(
            Entity<SemanticEditor>,
            std::rc::Rc<std::cell::Cell<Option<u64>>>,
        );
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let activated = self.1.clone();
                div()
                    .size_full()
                    .on_action(move |action: &ActivateReadOnlyLine, _, _| {
                        activated.set(Some(action.line))
                    })
                    .child(self.0.clone())
            }
        }
        let (_, cx) = cx.add_window_view(|_, _| Harness(editor.clone(), activated.clone()));
        cx.update(|window, app| {
            let focus = editor.read(app).focus_handle.clone();
            window.focus(&focus, app);
        });
        cx.simulate_keystrokes("down enter");
        cx.run_until_parked();
        assert_eq!(activated.get(), Some(1));
        editor.update(cx, |editor, cx| {
            assert!(!editor.session.read(cx).is_dirty())
        });
    }

    #[gpui::test]
    fn generic_generated_buffer_does_not_assign_enter_a_business_action(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(super::super::init);
        let activated = std::rc::Rc::new(std::cell::Cell::new(false));
        let editor = cx.new(|cx| SemanticEditor::new_read_only("Result\n".into(), vec![], cx));
        struct Harness(Entity<SemanticEditor>, std::rc::Rc<std::cell::Cell<bool>>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let activated = self.1.clone();
                div()
                    .on_action(move |_: &ActivateReadOnlyLine, _, _| activated.set(true))
                    .child(self.0.clone())
            }
        }
        let (_, cx) = cx.add_window_view(|_, _| Harness(editor.clone(), activated.clone()));
        cx.update(|window, app| {
            let focus = editor.read(app).focus_handle.clone();
            window.focus(&focus, app);
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(!activated.get());
        editor.update(cx, |editor, cx| {
            assert_eq!(
                editor.snapshot(cx).copy_range(ByteRange::new(0, 7)),
                "Result\n"
            );
        });
    }
}
