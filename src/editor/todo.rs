//! Heading-only hover host for the reusable TODO menu.
use super::*;
use crate::{
    components::todo_picker::{BAR_HEIGHT, MORE_HEIGHT, TodoPicker, TodoPickerEvent},
    document::DocumentFormat,
    org_semantic::{OrgFileConfig, extract_file_config},
};

struct TodoHit {
    range: ByteRange,
    remove_end: ByteOffset,
    keyword: Arc<str>,
    bounds: Bounds<Pixels>,
}
pub(super) struct TodoPopup {
    picker: Entity<TodoPicker>,
    _subscription: Subscription,
    _observation: Subscription,
    interaction_bounds: Option<Bounds<Pixels>>,
    hit: TodoHit,
    revision: Revision,
}

fn heading_keyword(text: &str) -> Option<Range<usize>> {
    let trimmed = text.trim_start();
    let stars = trimmed.bytes().take_while(|b| *b == b'*').count();
    if stars == 0 || trimmed.as_bytes().get(stars) != Some(&b' ') {
        return None;
    }
    let after_stars = &trimmed[stars..];
    let start =
        text.len() - trimmed.len() + stars + after_stars.len() - after_stars.trim_start().len();
    let end = start
        + text[start..]
            .find(char::is_whitespace)
            .unwrap_or(text.len() - start);
    (start < end).then_some(start..end)
}

fn quick_bar_bounds(token: Bounds<Pixels>, viewport: Bounds<Pixels>, width: f32) -> Bounds<Pixels> {
    let x = token
        .left()
        .min(viewport.right() - px(width + 8.))
        .max(viewport.left() + px(8.));
    let below = token.bottom() + px(3.);
    let above = token.top() - px(BAR_HEIGHT + 3.);
    let y = if below + px(BAR_HEIGHT + 3.) <= viewport.bottom() || above < viewport.top() {
        below
    } else {
        above
    };
    Bounds::new(gpui::point(x, y), gpui::size(px(width), px(BAR_HEIGHT)))
}

impl SemanticEditor {
    pub(super) fn todo_highlight(&self) -> Option<ByteRange> {
        self.todo_popup.as_ref().map(|popup| popup.hit.range)
    }
    fn todo_hit(&self, position: Point<Pixels>, cx: &App) -> Option<TodoHit> {
        if self
            .viewport
            .is_none_or(|bounds| !bounds.contains(&position))
            || self.is_read_only(cx)
            || DocumentFormat::from_path(self.session.read(cx).syntax_path()) != DocumentFormat::Org
        {
            return None;
        }
        let row = self
            .hit_rows
            .iter()
            .find(|r| position.y >= r.visible_top && position.y < r.visible_bottom)?;
        if row.inline_image_preview
            || position.x < row.text_origin_x
            || position.x > row.text_origin_x + row.visual_width()
        {
            return None;
        }
        let snapshot = self.snapshot(cx);
        let line_range = snapshot.line_content_range(row.line).ok()?;
        if line_range.len() > 64 * 1024 {
            return None;
        }
        let text = snapshot.copy_range(line_range);
        let token = heading_keyword(&text)?;
        let (start, end) = (token.start, token.end);
        let offset = self.hit_test(position).0.saturating_sub(line_range.start.0) as usize;
        if offset < start || offset >= end {
            return None;
        }
        let query = syntax::SparseEditorStyleSnapshot::query_lines(
            self.session.read(cx).syntax_path(),
            &snapshot,
            &[row.line.0],
            &self.syntax_service,
        );
        if !matches!(
            query.snapshot.line(row.line.0)?.id,
            syntax::EditorStyleId::Heading(_)
        ) {
            return None;
        }
        let keyword: Arc<str> = Arc::from(&text[start..end]);
        if let Some((revision, config)) = &self.todo_config
            && *revision == snapshot.revision()
            && config.todo_state(&keyword).is_none()
        {
            return None;
        }
        let remove_end = end + text[end..].len() - text[end..].trim_start().len();
        Some(TodoHit {
            range: ByteRange::new(
                line_range.start.0 + start as u64,
                line_range.start.0 + end as u64,
            ),
            remove_end: ByteOffset(line_range.start.0 + remove_end as u64),
            keyword,
            bounds: {
                let start = row.position_for_display_index(row.display.source_to_display(
                    (line_range.start.0 + start as u64).saturating_sub(row.range.start.0) as usize,
                ))?;
                let end = row.position_for_display_index(row.display.source_to_display(
                    (line_range.start.0 + end as u64).saturating_sub(row.range.start.0) as usize,
                ))?;
                Bounds::from_corners(
                    gpui::point(
                        row.text_origin_x + start.x,
                        (row.origin_y + start.y).max(row.visible_top),
                    ),
                    gpui::point(
                        row.text_origin_x + end.x,
                        (row.origin_y + start.y + row.line_height).min(row.visible_bottom),
                    ),
                )
            },
        })
    }
    pub(super) fn todo_hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.timestamp_popup.is_some()
            || self.is_selecting
            || self.minimap.drag.is_some()
            || self.minimap.resizing.is_some()
        {
            return;
        }
        if self
            .todo_popup
            .as_ref()
            .is_some_and(|p| p.interaction_bounds.is_some_and(|b| b.contains(&position)))
        {
            self.todo_dismiss_task = None;
            return;
        }
        let hit = self.todo_hit(position, cx);
        if hit.is_some() {
            self.todo_dismiss_task = None;
        } else {
            self.todo_pointer_left(cx);
        }
        let range = hit.as_ref().map(|h| h.range);
        if range != self.todo_dismissed {
            self.todo_dismissed = None;
        }
        if range == self.todo_hover_range {
            return;
        }
        self.todo_hover_task = None;
        self.todo_hover_range = range;
        let Some(hit) = hit else {
            return;
        };
        if self.todo_dismissed == Some(hit.range) {
            return;
        }
        let snapshot = self.snapshot(cx);
        let revision = snapshot.revision();
        let cached = self
            .todo_config
            .as_ref()
            .filter(|(r, _)| *r == revision)
            .map(|(_, c)| c.clone());
        if let Some(popup) = &self.todo_popup {
            if popup.hit.range == hit.range {
                return;
            }
            // An already-open menu follows another visible status immediately.
            // Moving through ordinary text or into the menu keeps it usable.
            if let Some(config) = cached {
                self.open_todo_picker(hit, config, cx);
                return;
            }
        }
        let executor = cx.background_executor().clone();
        self.todo_hover_task = Some(cx.spawn(async move |this, cx| {
            executor.timer(Duration::from_millis(350)).await;
            // File-wide configuration is parsed off the UI thread, once per revision.
            let config = if let Some(config) = cached {
                config
            } else {
                executor
                    .spawn(async move { Arc::new(extract_file_config(&snapshot)) })
                    .await
            };
            let _ = this.update(cx, |this, cx| {
                if this.snapshot(cx).revision() != revision {
                    return;
                }
                this.todo_config = Some((revision, config.clone()));
                if this.todo_hover_range == Some(hit.range)
                    && !this.is_selecting
                    && this.timestamp_popup.is_none()
                    && this.todo_popup.is_none()
                    && !this.is_read_only(cx)
                    && config.todo_state(&hit.keyword).is_some()
                {
                    this.open_todo_picker(hit, config, cx);
                }
            });
        }));
    }
    pub(super) fn todo_pointer_left(&mut self, cx: &mut Context<Self>) {
        if self.todo_popup.is_none() || self.todo_dismiss_task.is_some() {
            return;
        }
        let executor = cx.background_executor().clone();
        self.todo_dismiss_task = Some(cx.spawn(async move |this, cx| {
            executor.timer(Duration::from_millis(300)).await;
            let _ = this.update(cx, |this, cx| {
                this.dismiss_todo(cx);
                this.todo_hover_range = None;
                this.autofocus = true;
            });
        }));
    }

    fn open_todo_picker(
        &mut self,
        hit: TodoHit,
        config: Arc<OrgFileConfig>,
        cx: &mut Context<Self>,
    ) {
        self.todo_dismiss_task = None;
        self.todo_config = Some((self.snapshot(cx).revision(), config.clone()));
        let states = config
            .todo_keywords()
            .iter()
            .filter_map(|k| config.todo_state(k).cloned())
            .collect();
        let picker =
            cx.new(|cx| TodoPicker::new(states, hit.keyword.clone(), self.ui_language, cx));
        let subscription = cx.subscribe(&picker, |this, _, event: &TodoPickerEvent, cx| {
            match event {
                TodoPickerEvent::Selected(state) => this.apply_todo(state.as_deref(), cx),
                TodoPickerEvent::Cancelled => this.dismiss_todo(cx),
                TodoPickerEvent::Customize => this.customize_todo(cx),
            }
            this.autofocus = true;
            cx.notify();
        });
        self.timestamp_hover_task = None;
        self.timestamp_hover_range = None;
        self.hovered_link = None;
        self.hover_position = None;
        let observation = cx.observe(&picker, |_, _, cx| cx.notify());
        self.todo_popup = Some(TodoPopup {
            picker,
            _subscription: subscription,
            _observation: observation,
            interaction_bounds: None,
            hit,
            revision: self.snapshot(cx).revision(),
        });
        cx.notify();
    }
    pub(super) fn dismiss_todo(&mut self, cx: &mut Context<Self>) {
        self.todo_dismiss_task = None;
        if let Some(popup) = self.todo_popup.take() {
            self.todo_dismissed = Some(popup.hit.range);
            cx.notify();
        }
        self.todo_hover_task = None;
    }
    pub(super) fn todo_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) -> bool {
        self.todo_popup.as_ref().is_some_and(|popup| {
            popup
                .picker
                .update(cx, |picker, cx| picker.handle_key(event, cx))
        })
    }
    pub(super) fn set_todo_language(
        &mut self,
        language: crate::i18n::Language,
        cx: &mut Context<Self>,
    ) {
        if let Some(popup) = &self.todo_popup {
            popup
                .picker
                .update(cx, |picker, cx| picker.set_language(language, cx));
        }
    }
    fn apply_todo(&mut self, state: Option<&str>, cx: &mut Context<Self>) {
        let Some(popup) = self.todo_popup.take() else {
            return;
        };
        self.todo_dismissed = Some(popup.hit.range);
        self.todo_hover_task = None;
        self.todo_dismiss_task = None;
        let snapshot = self.snapshot(cx);
        if self.is_read_only(cx)
            || snapshot.revision() != popup.revision
            || snapshot.copy_range(popup.hit.range) != popup.hit.keyword.as_ref()
        {
            self.command_feedback = Some(self.ui_language.text("todo.source_changed").into());
            cx.notify();
            return;
        }
        if state == Some(popup.hit.keyword.as_ref()) {
            cx.notify();
            return;
        }
        // Only states from the revision-coherent menu may be written.
        if state.is_some_and(|s| {
            self.todo_config
                .as_ref()
                .is_none_or(|(r, c)| *r != popup.revision || c.todo_state(s).is_none())
        }) {
            cx.notify();
            return;
        }
        self.finish_composition(cx);
        let range = if state.is_some() {
            popup.hit.range
        } else {
            ByteRange::new(popup.hit.range.start.0, popup.hit.remove_end.0)
        };
        let replacement = state.unwrap_or("");
        // Keep both endpoints attached to the same text, including reversed selections.
        let map = |offset: ByteOffset| {
            let value = if offset.0 <= range.start.0 {
                offset.0
            } else if offset.0 >= range.end.0 {
                offset.0 - range.len() + replacement.len() as u64
            } else {
                let mut local = ((offset.0 - range.start.0) as usize).min(replacement.len());
                while !replacement.is_char_boundary(local) {
                    local -= 1;
                }
                range.start.0 + local as u64
            };
            ByteOffset(value)
        };
        let after = Selection::new(map(self.selection.anchor()), map(self.selection.head()));
        let result = self.session.update(cx, |session, cx| {
            session.edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        popup.revision,
                        vec![TextEdit::new(range, replacement.to_owned())],
                    ),
                    self.selection,
                    after,
                    EditOrigin::Other,
                ),
                cx,
            )
        });
        if result.is_ok() {
            self.selection = after;
            self.sync_selection_revision(cx);
            self.sync_selection_utf16(&self.snapshot(cx));
            self.hit_rows = Arc::from([]);
            self.pending_reveal_caret = false;
            self.command_feedback = None;
        } else {
            self.command_feedback = Some(self.ui_language.text("todo.failed").into());
        }
        cx.notify();
    }
    fn customize_todo(&mut self, cx: &mut Context<Self>) {
        let Some(popup) = self.todo_popup.as_ref() else {
            return;
        };
        let revision = popup.revision;
        let keyword = popup.hit.keyword.clone();
        let snapshot = self.snapshot(cx);
        self.dismiss_todo(cx);
        if self.is_read_only(cx) || snapshot.revision() != revision {
            return;
        }
        // Use the file's native Org directive as the customization surface. Scan off-thread.
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let target = executor
                .spawn(async move {
                    let mut first = None;
                    let mut lines = crate::document::LineCursor::within(
                        &snapshot,
                        ByteRange::new(0, snapshot.len_bytes()),
                    )?;
                    while let Some(line) = lines.next_line() {
                        if let Some(sequence) =
                            crate::org_semantic::parse_todo_directive(line.text.trim_start())
                        {
                            let start = line.text.find(':')? + 1;
                            let end = line.text.trim_end_matches(['\r', '\n']).len();
                            let start = start + line.text[start..end].len()
                                - line.text[start..end].trim_start().len();
                            let range = ByteRange::new(
                                line.range.start.0 + start as u64,
                                line.range.start.0 + end as u64,
                            );
                            first.get_or_insert(range);
                            if sequence.states.iter().any(|s| s.keyword == keyword) {
                                return Some(range);
                            }
                        }
                    }
                    first
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.is_read_only(cx) || this.snapshot(cx).revision() != revision {
                    return;
                }
                if let Some(range) = target {
                    this.selection = Selection::new(range.start, range.end);
                    this.sync_selection_revision(cx);
                    this.sync_selection_utf16(&this.snapshot(cx));
                } else {
                    let newline = this.session.read(cx).newline_sequence();
                    let directive = "#+TODO: TODO | DONE";
                    let text = format!("{directive}{newline}");
                    let after = Selection::new(ByteOffset(8), ByteOffset(directive.len() as u64));
                    let result = this.session.update(cx, |session, cx| {
                        session.edit(
                            DocumentCommand::new(
                                EditTransaction::new(
                                    revision,
                                    vec![TextEdit::new(ByteRange::new(0, 0), text)],
                                ),
                                this.selection,
                                after,
                                EditOrigin::Other,
                            ),
                            cx,
                        )
                    });
                    if result.is_err() {
                        this.command_feedback = Some(this.ui_language.text("todo.failed").into());
                        return;
                    }
                    this.selection = after;
                    this.sync_selection_revision(cx);
                    this.sync_selection_utf16(&this.snapshot(cx));
                }
                this.pending_reveal_caret = true;
                this.autofocus = true;
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn todo_overlay(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let viewport = self.viewport?;
        let popup = self.todo_popup.as_ref()?;
        let width = popup
            .picker
            .read(cx)
            .preferred_width(window)
            .min(f32::from(viewport.size.width) - 16.)
            .max(40.);
        let mut layout = quick_bar_bounds(popup.hit.bounds, viewport, width);
        // When headings are consecutive there is no real interline space. Keep
        // their status column exposed, so moving down to another token still works.
        let snapshot = self.snapshot(cx);
        let mut status_right = layout.left();
        if let Some((_, config)) = &self.todo_config {
            for row in self
                .hit_rows
                .iter()
                .filter(|r| r.visible_top < layout.bottom() && r.visible_bottom > layout.top())
            {
                let Ok(range) = snapshot.line_content_range(row.line) else {
                    continue;
                };
                if range.len() > 64 * 1024 {
                    continue;
                }
                let text = snapshot.copy_range(range);
                let Some(token) = heading_keyword(&text) else {
                    continue;
                };
                if config.todo_state(&text[token.clone()]).is_none() {
                    continue;
                }
                if let Some(end) = row.position_for_display_index(row.display.source_to_display(
                    (range.start.0 + token.end as u64).saturating_sub(row.range.start.0) as usize,
                )) {
                    status_right = status_right.max(row.text_origin_x + end.x + px(8.));
                }
            }
        }
        if status_right + px(width + 8.) <= viewport.right() {
            layout.origin.x = status_right;
        }
        let popup = self.todo_popup.as_mut()?;
        let more_open = popup.picker.read(cx).more_open;
        let more_above = if layout.bottom() <= popup.hit.bounds.top() {
            layout.top() - viewport.top() >= px(MORE_HEIGHT + 3.)
        } else {
            viewport.bottom() - layout.bottom() < px(MORE_HEIGHT + 3.)
        };
        popup.picker.update(cx, |p, _| {
            p.width = width;
            p.more_above = more_above;
        });
        let extra = if more_open { px(MORE_HEIGHT) } else { px(0.) };
        let position = gpui::point(
            layout.left(),
            layout.top() - if more_above { extra } else { px(0.) },
        );
        let bounds = Bounds::new(
            position - gpui::point(px(0.), px(3.)),
            gpui::size(px(width), px(BAR_HEIGHT + 6.) + extra),
        );
        popup.interaction_bounds = Some(bounds);
        Some(
            gpui::deferred(
                gpui::anchored()
                    .position(bounds.origin)
                    .snap_to_window_with_margin(px(4.))
                    .child(
                        div()
                            .id("editor-todo-popup")
                            .occlude()
                            .py(px(3.))
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.dismiss_todo(cx);
                            }))
                            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                                if *hovered {
                                    this.todo_dismiss_task = None;
                                } else {
                                    this.todo_hover(window.mouse_position(), cx);
                                }
                            }))
                            .on_mouse_move(cx.listener(|this, _, _, cx| {
                                this.todo_dismiss_task = None;
                                cx.stop_propagation();
                            }))
                            .child(popup.picker.clone()),
                    ),
            )
            .with_priority(10)
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};

    fn point(
        editor: &Entity<SemanticEditor>,
        line: u64,
        local: usize,
        cx: &mut VisualTestContext,
    ) -> Point<Pixels> {
        cx.read(|cx| {
            let row = editor
                .read(cx)
                .hit_rows
                .iter()
                .find(|r| r.line.0 == line)
                .unwrap();
            let position = row
                .position_for_display_index(row.display.source_to_display(local))
                .unwrap();
            gpui::point(
                row.text_origin_x + position.x + px(2.),
                row.origin_y + position.y + px(5.),
            )
        })
    }
    fn hover(editor: &Entity<SemanticEditor>, line: u64, local: usize, cx: &mut VisualTestContext) {
        // Leave the previous token so deliberate reopening is possible after undo.
        cx.simulate_mouse_move(gpui::point(px(650.), px(600.)), None, Modifiers::default());
        let position = point(editor, line, local, cx);
        cx.simulate_mouse_move(position, None, Modifiers::default());
        cx.executor().advance_clock(Duration::from_millis(400));
        cx.run_until_parked();
    }
    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let p = cx.debug_bounds(selector).unwrap().center();
        cx.simulate_click(p, Modifiers::default());
        cx.run_until_parked();
    }
    fn source(editor: &Entity<SemanticEditor>, cx: &mut VisualTestContext) -> String {
        cx.read(|cx| {
            let s = editor.read(cx).snapshot(cx);
            s.copy_range(ByteRange::new(0, s.len_bytes()))
        })
    }

    #[gpui::test]
    fn todo_menu_follows_another_heading_without_closing_first(cx: &mut TestAppContext) {
        cx.update(init);
        let original = "#+TODO: TODO(t) DOING(i) | DONE(d)\n* TODO First\n* DOING Second\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("follow.org"), original.as_bytes().to_vec())
                .unwrap()
        });
        let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        cx.simulate_resize(gpui::size(px(800.), px(1000.)));
        cx.run_until_parked();
        hover(&editor, 1, 3, cx);
        let first_menu = cx.debug_bounds("todo-picker").unwrap();
        let first_range = cx.read(|cx| editor.read(cx).todo_highlight().unwrap());
        assert!(cx.debug_bounds("todo-current-TODO").is_some());
        assert!(cx.debug_bounds("todo-current-DOING").is_none());

        // Entering the options must retain the current heading.
        let option = cx.debug_bounds("todo-option-DONE").unwrap().center();
        cx.simulate_mouse_move(option, None, Modifiers::default());
        cx.run_until_parked();
        cx.read(|cx| assert_eq!(editor.read(cx).todo_highlight(), Some(first_range)));

        let second = point(&editor, 2, 3, cx) + gpui::point(px(0.), px(8.));
        assert!(!first_menu.contains(&second));
        cx.simulate_mouse_move(second, None, Modifiers::default());
        cx.run_until_parked();
        cx.read(|cx| {
            let e = editor.read(cx);
            let popup = e.todo_popup.as_ref().unwrap();
            assert_ne!(popup.hit.range, first_range);
            assert_eq!(popup.hit.keyword.as_ref(), "DOING");
        });

        assert!(cx.debug_bounds("todo-current-DOING").is_some());
        assert!(cx.debug_bounds("todo-current-TODO").is_none());

        // Repeated switching must not leave a delayed callback aimed at an old heading.
        let first = point(&editor, 1, 3, cx);
        cx.simulate_mouse_move(first, None, Modifiers::default());
        cx.run_until_parked();
        cx.read(|cx| assert_eq!(editor.read(cx).todo_highlight(), Some(first_range)));
        cx.simulate_mouse_move(second, None, Modifiers::default());
        cx.executor().advance_clock(Duration::from_millis(400));
        cx.run_until_parked();
        click(cx, "todo-option-DONE");
        assert_eq!(
            source(&editor, cx),
            original.replace("* DOING Second", "* DONE Second")
        );
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(source(&editor, cx), original);
    }

    #[gpui::test]
    fn todo_menu_custom_states_click_remove_and_undo(cx: &mut TestAppContext) {
        cx.update(init);
        let original = "#+TODO: TODO(t) DOING(i) WAITING(w) | DONE(d) CANCELLED(c)\r\n* TODO [#A] 完成日历 :work:\r\n<2026-09-15 Tue 14:00>\r\n正文保持不变\r\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("states.org"), original.as_bytes().to_vec())
                .unwrap()
        });
        let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        cx.run_until_parked();
        hover(&editor, 1, 3, cx);
        assert!(cx.debug_bounds("todo-picker").is_some());
        assert!(cx.debug_bounds("timestamp-picker").is_none());
        assert!(cx.debug_bounds("todo-option-WAITING").is_some());
        click(cx, "todo-option-DOING");
        assert_eq!(source(&editor, cx), original.replace("* TODO", "* DOING"));
        assert!(cx.debug_bounds("todo-picker").is_none());
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(source(&editor, cx), original);
        hover(&editor, 1, 3, cx);
        assert!(cx.debug_bounds("todo-remove").is_none());
        click(cx, "todo-more");
        click(cx, "todo-remove");
        assert_eq!(source(&editor, cx), original.replace("* TODO ", "* "));
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(source(&editor, cx), original);
        hover(&editor, 1, 3, cx);
        click(cx, "todo-option-TODO");
        assert_eq!(source(&editor, cx), original);
    }

    #[gpui::test]
    fn todo_menu_keys_and_dismissal_preserve_document(cx: &mut TestAppContext) {
        cx.update(init);
        let original = "#+SEQ_TODO: PLAN(p) WAIT(w) | SHIPPED(s)\n* PLAN Build\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("keys.org"), original.as_bytes().to_vec())
                .unwrap()
        });
        let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        cx.run_until_parked();
        hover(&editor, 1, 3, cx);
        for language in [
            crate::i18n::Language::English,
            crate::i18n::Language::Chinese,
        ] {
            editor.update(cx, |e, cx| e.set_ui_language(language, cx));
            cx.run_until_parked();
            assert!(cx.debug_bounds("todo-option-SHIPPED").is_some());
        }
        cx.simulate_keystrokes("s");
        assert_eq!(source(&editor, cx), original.replace("* PLAN", "* SHIPPED"));
        cx.simulate_keystrokes("cmd-z");
        hover(&editor, 1, 3, cx);
        cx.simulate_keystrokes("down enter");
        assert_eq!(source(&editor, cx), original.replace("* PLAN", "* WAIT"));
        cx.simulate_keystrokes("cmd-z");
        for keys in ["escape", "ctrl-g"] {
            hover(&editor, 1, 3, cx);
            cx.simulate_keystrokes(keys);
            cx.run_until_parked();
            assert!(cx.debug_bounds("todo-picker").is_none());
            assert_eq!(source(&editor, cx), original);
        }
        hover(&editor, 1, 3, cx);
        cx.simulate_click(gpui::point(px(650.), px(600.)), Modifiers::default());
        assert!(cx.debug_bounds("todo-picker").is_none());
        assert_eq!(source(&editor, cx), original);
    }

    #[gpui::test]
    fn todo_hover_default_states_code_guard_and_stale_revision(cx: &mut TestAppContext) {
        cx.update(init);
        let original = "* TODO Title\nTODO is body text\n#+begin_src org\n* TODO Example\n#+end_src\n* Ordinary heading\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("guards.org"), original.as_bytes().to_vec())
                .unwrap()
        });
        let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        cx.run_until_parked();
        for (line, local) in [(1, 1), (3, 3), (5, 3)] {
            hover(&editor, line, local, cx);
            assert!(cx.debug_bounds("todo-picker").is_none(), "line {line}");
        }
        hover(&editor, 0, 3, cx);
        assert!(cx.debug_bounds("todo-option-TODO").is_some());
        assert!(cx.debug_bounds("todo-option-DONE").is_some());
        assert!(cx.debug_bounds("todo-option-DOING").is_none());
        editor.update(cx, |e, cx| {
            e.todo_popup.as_mut().unwrap().revision = Revision(u64::MAX);
            e.apply_todo(Some("DONE"), cx);
            assert!(e.command_feedback.is_some());
        });
        assert_eq!(source(&editor, cx), original);
        hover(&editor, 0, 3, cx);
        editor.update(cx, |e, cx| e.scroll(0., -10., cx));
        cx.run_until_parked();
        assert!(cx.debug_bounds("todo-picker").is_none());
        assert_eq!(source(&editor, cx), original);
    }
    #[gpui::test]
    fn todo_quick_bar_bridge_and_delayed_dismissal(cx: &mut TestAppContext) {
        cx.update(init);
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                PathBuf::from("bridge.org"),
                b"* TODO First\n\n\n* TODO Second\n".to_vec(),
            )
            .unwrap()
        });
        let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        cx.run_until_parked();
        let before = point(&editor, 3, 3, cx);
        hover(&editor, 0, 3, cx);
        let bar = cx.debug_bounds("todo-bar").unwrap();
        cx.read(|cx| {
            let token = editor.read(cx).todo_popup.as_ref().unwrap().hit.bounds;
            assert!(bar.top() >= token.bottom());
        });
        assert_eq!(
            before,
            point(&editor, 3, 3, cx),
            "floating UI must not relayout source"
        );
        // Traverse the transparent 3px bridge, then linger on a real option.
        let bridge = gpui::point(bar.left() + px(10.), bar.top() - px(2.));
        cx.simulate_mouse_move(bridge, None, Modifiers::default());
        cx.executor().advance_clock(Duration::from_millis(500));
        cx.run_until_parked();
        assert!(cx.debug_bounds("todo-bar").is_some());
        let option = cx.debug_bounds("todo-option-DONE").unwrap().center();
        cx.simulate_mouse_move(option, None, Modifiers::default());
        cx.executor().advance_clock(Duration::from_millis(500));
        cx.run_until_parked();
        assert!(cx.debug_bounds("todo-bar").is_some());
        let outside = gpui::point(px(600.), px(500.));
        cx.simulate_mouse_move(outside, None, Modifiers::default());
        cx.executor().advance_clock(Duration::from_millis(150));
        cx.run_until_parked();
        assert!(cx.debug_bounds("todo-bar").is_some());
        // Reentry cancels the pending timer, rather than leaving a stale dismissal.
        cx.simulate_mouse_move(option, None, Modifiers::default());
        cx.executor().advance_clock(Duration::from_millis(400));
        cx.run_until_parked();
        assert!(cx.debug_bounds("todo-bar").is_some());
        cx.simulate_mouse_move(outside, None, Modifiers::default());
        cx.executor().advance_clock(Duration::from_millis(350));
        cx.run_until_parked();
        assert!(cx.debug_bounds("todo-bar").is_none());
    }

    #[gpui::test]
    fn todo_quick_bar_preserves_reversed_selection_and_scroll(cx: &mut TestAppContext) {
        cx.update(init);
        let original = format!(
            "#+TODO: TODO(t) DOING(i) | DONE(d)\n{}",
            "* TODO 中文任务\n".repeat(100)
        );
        let selected_start = original.rfind("中文任务").unwrap() as u64;
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("caret.org"), original.as_bytes().to_vec())
                .unwrap()
        });
        let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        cx.simulate_resize(gpui::size(px(800.), px(400.)));
        cx.run_until_parked();
        let before = Selection::new(ByteOffset(selected_start + 12), ByteOffset(selected_start));
        editor.update(cx, |e, cx| {
            e.selection = before;
            e.sync_selection_revision(cx);
            e.sync_selection_utf16(&e.snapshot(cx));
            e.scroll_y = 150.;
            e.pending_reveal_caret = false;
            cx.notify();
        });
        cx.run_until_parked();
        let (line, scroll) = cx.read(|cx| {
            let e = editor.read(cx);
            (
                e.hit_rows
                    .iter()
                    .find(|r| r.visible_top > e.viewport.unwrap().top() + px(20.))
                    .unwrap()
                    .line
                    .0,
                (e.scroll_x, e.scroll_y),
            )
        });
        hover(&editor, line, 3, cx);
        click(cx, "todo-option-DOING");
        cx.read(|cx| {
            let e = editor.read(cx);
            assert_eq!(
                e.selection,
                Selection::new(
                    ByteOffset(before.anchor().0 + 1),
                    ByteOffset(before.head().0 + 1)
                )
            );
            assert_eq!((e.scroll_x, e.scroll_y), scroll);
            assert!(!e.pending_reveal_caret);
            assert_eq!(e.snapshot(cx).copy_range(e.selection.range()), "中文任务");
        });
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(source(&editor, cx), original);
        cx.read(|cx| assert_eq!(editor.read(cx).selection, before));
    }

    #[gpui::test]
    fn todo_quick_bar_narrow_bottom_and_keyboard_overflow(cx: &mut TestAppContext) {
        cx.update(init);
        let original = format!(
            "#+TODO: TODO(t) DOING(i) WAITING(w) | DONE(d) CANCELLED(c)\n{}",
            "* TODO Task\n".repeat(20)
        );
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("narrow.org"), original.as_bytes().to_vec())
                .unwrap()
        });
        let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        cx.simulate_resize(gpui::size(px(320.), px(280.)));
        cx.run_until_parked();
        let line = cx.read(|cx| {
            let e = editor.read(cx);
            e.hit_rows
                .iter()
                .rev()
                .find(|r| r.origin_y + r.line_height <= e.viewport.unwrap().bottom())
                .unwrap()
                .line
                .0
        });
        hover(&editor, line, 3, cx);
        let bar = cx.debug_bounds("todo-bar").unwrap();
        cx.read(|cx| {
            let e = editor.read(cx);
            let token = e.todo_popup.as_ref().unwrap().hit.bounds;
            let viewport = e.viewport.unwrap();
            assert!(bar.bottom() <= token.top());
            assert!(bar.left() >= viewport.left() && bar.right() <= viewport.right());
        });
        assert!(bar.contains(&cx.debug_bounds("todo-more").unwrap().center()));
        // Left goes to More; another Left reveals the last overflowed status.
        cx.simulate_keystrokes("left left");
        cx.run_until_parked();
        assert!(bar.contains(&cx.debug_bounds("todo-option-CANCELLED").unwrap().center()));
        cx.simulate_keystrokes("space");
        assert_eq!(source(&editor, cx).matches("* CANCELLED Task").count(), 1);
        cx.simulate_keystrokes("cmd-z");
        // Undo reveals the caret; reopen a visible heading to check the More submenu.
        hover(&editor, 1, 3, cx);
        click(cx, "todo-more");
        let menu = cx.debug_bounds("todo-more-menu").unwrap();
        assert!(menu.top() >= px(0.) && menu.bottom() <= px(280.));
        assert!(menu.right() <= px(320.));
        click(cx, "todo-remove");
        assert_eq!(source(&editor, cx).matches("* Task").count(), 1);
    }

    #[gpui::test]
    fn todo_quick_bar_customize_edits_native_directive(cx: &mut TestAppContext) {
        cx.update(init);
        let original = "#+TODO: TODO(t) | DONE(d)\r\n#+SEQ_TODO: PLAN(p!) WAIT(w@) | SHIPPED(s)\r\n* PLAN Build\r\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("customize.org"), original.as_bytes().to_vec())
                .unwrap()
        });
        let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        cx.run_until_parked();
        hover(&editor, 2, 3, cx);
        click(cx, "todo-more");
        click(cx, "todo-customize");
        assert_eq!(source(&editor, cx), original);
        cx.read(|cx| {
            let e = editor.read(cx);
            assert_eq!(
                e.snapshot(cx).copy_range(e.selection.range()),
                "PLAN(p!) WAIT(w@) | SHIPPED(s)"
            );
        });
    }

    #[gpui::test]
    fn todo_quick_bar_customize_defaults_is_undoable(cx: &mut TestAppContext) {
        cx.update(init);
        let original = "* TODO Build\r\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("defaults.org"), original.as_bytes().to_vec())
                .unwrap()
        });
        let (editor, cx) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        cx.run_until_parked();
        hover(&editor, 0, 3, cx);
        click(cx, "todo-more");
        click(cx, "todo-customize");
        assert_eq!(
            source(&editor, cx),
            format!("#+TODO: TODO | DONE\r\n{original}")
        );
        cx.read(|cx| {
            let e = editor.read(cx);
            assert_eq!(
                e.snapshot(cx).copy_range(e.selection.range()),
                "TODO | DONE"
            );
        });
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(source(&editor, cx), original);
    }

    #[test]
    fn todo_quick_bar_clamps_to_editor_pane_and_flips_at_bottom() {
        let viewport = Bounds::new(
            gpui::point(px(200.), px(100.)),
            gpui::size(px(600.), px(400.)),
        );
        let token = |x, y| Bounds::new(gpui::point(px(x), px(y)), gpui::size(px(50.), px(24.)));
        let top = quick_bar_bounds(token(730., 100.), viewport, 420.);
        assert_eq!(top.top(), px(127.));
        assert_eq!(top.right(), px(792.));
        let bottom = quick_bar_bounds(token(210., 470.), viewport, 420.);
        assert_eq!(bottom.bottom(), px(467.));
        assert!(bottom.left() >= viewport.left());
    }
}
