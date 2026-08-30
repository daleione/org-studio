use super::*;

impl SourceEditor {
    pub(super) fn replace_selection(
        &mut self,
        text: &str,
        origin: EditOrigin,
        cx: &mut Context<Self>,
    ) {
        let history_before = self.selection;
        self.replace_selection_recording(text, origin, history_before, cx);
    }

    fn replace_selection_recording(
        &mut self,
        text: &str,
        origin: EditOrigin,
        history_before: Selection,
        cx: &mut Context<Self>,
    ) {
        self.finish_composition(cx);
        self.vertical_goal_x = None;
        let range = self.selection.range();
        let after = Selection::caret(ByteOffset(range.start.0 + text.len() as u64));
        let after_utf16 = self.selection_utf16.start + text.encode_utf16().count();
        let revision = self.session.read(cx).revision();
        let result = self.session.update(cx, |session, cx| {
            session.edit(
                SessionEdit::new(
                    EditTransaction::new(revision, vec![TextEdit::new(range, text.to_owned())]),
                    history_before,
                    after,
                    origin,
                ),
                cx,
            )
        });
        if result.is_ok() {
            self.selection = after;
            self.selection_utf16 = after_utf16..after_utf16;
            self.selection_utf16_reversed = false;
            let snapshot = self.snapshot(cx);
            self.reveal_caret(&snapshot);
            cx.notify();
        }
    }

    fn delete_backward(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let history_before = self.selection;
        if self.selection.is_empty() {
            let snapshot = self.snapshot(cx);
            let previous = snapshot
                .previous_grapheme_boundary(self.selection.head())
                .unwrap_or(self.selection.head());
            if previous == self.selection.head() {
                return;
            }
            let deleted_utf16 = snapshot
                .copy_range(ByteRange::new(previous.0, self.selection.head().0))
                .encode_utf16()
                .count();
            self.selection_utf16 =
                self.selection_utf16.start.saturating_sub(deleted_utf16)..self.selection_utf16.end;
            self.selection = Selection::new(self.selection.head(), previous);
        }
        self.replace_selection_recording("", EditOrigin::DeleteBackward, history_before, cx);
    }

    fn delete_forward(&mut self, _: &DeleteForward, _: &mut Window, cx: &mut Context<Self>) {
        let history_before = self.selection;
        if self.selection.is_empty() {
            let snapshot = self.snapshot(cx);
            let next = snapshot
                .next_grapheme_boundary(self.selection.head())
                .unwrap_or(self.selection.head());
            if next == self.selection.head() {
                return;
            }
            let deleted_utf16 = snapshot
                .copy_range(ByteRange::new(self.selection.head().0, next.0))
                .encode_utf16()
                .count();
            self.selection_utf16 =
                self.selection_utf16.start..self.selection_utf16.end + deleted_utf16;
            self.selection = Selection::new(self.selection.head(), next);
        }
        self.replace_selection_recording("", EditOrigin::DeleteForward, history_before, cx);
    }

    fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(false, false, false, cx);
    }

    fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(true, false, false, cx);
    }

    fn move_word_left(&mut self, _: &MoveWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(false, false, true, cx);
    }

    fn move_word_right(&mut self, _: &MoveWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(true, false, true, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(false, true, false, cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(true, true, false, cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(false, true, true, cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(true, true, true, cx);
    }

    fn move_horizontal(
        &mut self,
        forward: bool,
        extend: bool,
        by_word: bool,
        cx: &mut Context<Self>,
    ) {
        self.finish_composition(cx);
        self.vertical_goal_x = None;
        let snapshot = self.snapshot(cx);
        let target = if !extend && !self.selection.is_empty() {
            if forward {
                self.selection.range().end
            } else {
                self.selection.range().start
            }
        } else if by_word {
            word_boundary(&snapshot, self.selection.head(), forward)
        } else if forward {
            snapshot
                .next_grapheme_boundary(self.selection.head())
                .unwrap_or(self.selection.head())
        } else {
            snapshot
                .previous_grapheme_boundary(self.selection.head())
                .unwrap_or(self.selection.head())
        };
        self.selection = if extend {
            self.selection.with_head(target)
        } else {
            Selection::caret(target)
        };
        self.sync_selection_utf16(&snapshot);
        self.reveal_caret(&snapshot);
        cx.notify();
    }

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(-1, false, cx);
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(1, false, cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(-1, true, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(1, true, cx);
    }

    fn move_vertical(&mut self, delta: i64, extend: bool, cx: &mut Context<Self>) {
        self.finish_composition(cx);
        let snapshot = self.snapshot(cx);
        let head = self.selection.head();
        if let Some(row) = self
            .hit_rows
            .iter()
            .find(|row| head >= row.range.start && head <= row.range.end)
        {
            let source_local = (head.0 - row.range.start.0).min(row.range.len()) as usize;
            let display_local = row.display.source_to_display(source_local);
            if let Some(position) = row
                .layout
                .position_for_index(display_local, px(LINE_HEIGHT))
            {
                let goal_x = *self
                    .vertical_goal_x
                    .get_or_insert_with(|| f32::from(position.x));
                let target = self.hit_test(gpui::point(
                    row.text_origin_x + px(goal_x),
                    row.origin_y + position.y + px(delta as f32 * LINE_HEIGHT + LINE_HEIGHT / 2.0),
                ));
                self.selection = if extend {
                    self.selection.with_head(target)
                } else {
                    Selection::caret(target)
                };
                self.sync_selection_utf16(&snapshot);
                self.reveal_caret(&snapshot);
                cx.notify();
                return;
            }
        }
        let Ok(line) = snapshot.line_index_at(head) else {
            return;
        };
        let Ok((_, column)) = snapshot.line_and_column_at(head) else {
            return;
        };
        let target_line = (line.0 as i128 + i128::from(delta))
            .clamp(0, snapshot.len_lines().saturating_sub(1) as i128)
            as u64;
        let target = snapshot
            .byte_at_line_column(LineIndex(target_line), column)
            .unwrap_or(head);
        self.selection = if extend {
            self.selection.with_head(target)
        } else {
            Selection::caret(target)
        };
        self.sync_selection_utf16(&snapshot);
        self.reveal_caret(&snapshot);
        cx.notify();
    }

    fn move_line_start(&mut self, _: &MoveLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to_line_edge(false, cx);
    }

    fn move_line_end(&mut self, _: &MoveLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to_line_edge(true, cx);
    }

    fn move_to_line_edge(&mut self, end: bool, cx: &mut Context<Self>) {
        self.finish_composition(cx);
        let snapshot = self.snapshot(cx);
        if let Ok(line) = snapshot.line_index_at(self.selection.head())
            && let Ok(range) = snapshot.line_content_range(line)
        {
            self.selection = Selection::caret(if end { range.end } else { range.start });
            self.sync_selection_utf16(&snapshot);
            self.reveal_caret(&snapshot);
            cx.notify();
        }
    }

    fn move_document_start(
        &mut self,
        _: &MoveDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_composition(cx);
        self.selection = Selection::default();
        self.selection_utf16 = 0..0;
        self.selection_utf16_reversed = false;
        self.scroll_y = 0.0;
        cx.notify();
    }

    fn move_document_end(&mut self, _: &MoveDocumentEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.finish_composition(cx);
        let snapshot = self.snapshot(cx);
        self.selection = Selection::caret(ByteOffset(snapshot.len_bytes()));
        self.sync_selection_utf16(&snapshot);
        self.reveal_caret(&snapshot);
        cx.notify();
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.finish_composition(cx);
        let snapshot = self.snapshot(cx);
        self.selection = Selection::new(ByteOffset(0), ByteOffset(snapshot.len_bytes()));
        self.sync_selection_utf16(&snapshot);
        cx.notify();
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        let newline = self.session.read(cx).newline_sequence();
        self.replace_selection(newline, EditOrigin::Newline, cx);
    }

    fn insert_tab(&mut self, _: &InsertTab, _: &mut Window, cx: &mut Context<Self>) {
        self.replace_selection("\t", EditOrigin::Typing, cx);
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.finish_composition(cx);
        if let Ok(HistoryOutcome::Applied(selection)) =
            self.session.update(cx, |session, cx| session.undo(cx))
        {
            self.selection = selection;
            let snapshot = self.snapshot(cx);
            self.sync_selection_utf16(&snapshot);
            self.reveal_caret(&snapshot);
            cx.notify();
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.finish_composition(cx);
        if let Ok(HistoryOutcome::Applied(selection)) =
            self.session.update(cx, |session, cx| session.redo(cx))
        {
            self.selection = selection;
            let snapshot = self.snapshot(cx);
            self.sync_selection_utf16(&snapshot);
            self.reveal_caret(&snapshot);
            cx.notify();
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            let snapshot = self.snapshot(cx);
            cx.write_to_clipboard(ClipboardItem::new_string(
                snapshot.copy_range(self.selection.range()),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            let snapshot = self.snapshot(cx);
            cx.write_to_clipboard(ClipboardItem::new_string(
                snapshot.copy_range(self.selection.range()),
            ));
            self.replace_selection("", EditOrigin::Cut, cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_selection(&text, EditOrigin::Paste, cx);
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.finish_composition(cx);
        self.vertical_goal_x = None;
        self.is_selecting = true;
        self.drag_position = Some(event.position);
        let target = self.hit_test(event.position);
        self.selection = if event.modifiers.shift {
            self.selection.with_head(target)
        } else {
            Selection::caret(target)
        };
        let snapshot = self.snapshot(cx);
        self.sync_selection_utf16(&snapshot);
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.drag_position = Some(event.position);
            self.autoscroll_selection(cx);
            if self.autoscroll_task.is_none() && self.drag_is_outside_viewport() {
                let executor = cx.background_executor().clone();
                self.autoscroll_task = Some(cx.spawn(async move |this, cx| {
                    loop {
                        executor.timer(Duration::from_millis(16)).await;
                        let Ok(keep_running) = this.update(cx, |this, cx| {
                            if !this.is_selecting || !this.drag_is_outside_viewport() {
                                this.autoscroll_task = None;
                                return false;
                            }
                            this.autoscroll_selection(cx);
                            true
                        }) else {
                            break;
                        };
                        if !keep_running {
                            break;
                        }
                    }
                }));
            }
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
        self.drag_position = None;
        self.autoscroll_task = None;
    }

    fn drag_is_outside_viewport(&self) -> bool {
        self.viewport
            .zip(self.drag_position)
            .is_some_and(|(viewport, position)| {
                position.y < viewport.top() || position.y > viewport.bottom()
            })
    }

    fn autoscroll_selection(&mut self, cx: &mut Context<Self>) {
        let Some((viewport, position)) = self.viewport.zip(self.drag_position) else {
            return;
        };
        let distance = if position.y < viewport.top() {
            f32::from(position.y - viewport.top())
        } else if position.y > viewport.bottom() {
            f32::from(position.y - viewport.bottom())
        } else {
            0.0
        };
        if distance != 0.0 {
            let max_scroll = (self.display_map.total_visual_rows() as f32 * LINE_HEIGHT
                - f32::from(viewport.size.height))
            .max(0.0);
            let speed = distance.signum() * (distance.abs() / 8.0).clamp(4.0, 64.0);
            self.scroll_y = (self.scroll_y + speed).clamp(0.0, max_scroll);
        }
        self.selection = self.selection.with_head(self.hit_test(position));
        let snapshot = self.snapshot(cx);
        self.sync_selection_utf16(&snapshot);
        cx.notify();
    }

    pub(super) fn hit_test(&self, position: gpui::Point<Pixels>) -> ByteOffset {
        let Some(first) = self.hit_rows.first() else {
            return ByteOffset(0);
        };
        let row = self
            .hit_rows
            .iter()
            .find(|row| {
                let top = row.origin_y;
                let height = px((row.layout.wrap_boundaries().len() + 1) as f32 * LINE_HEIGHT);
                position.y >= top && position.y < top + height
            })
            .unwrap_or_else(|| {
                if position.y < first.origin_y {
                    first
                } else {
                    self.hit_rows.last().expect("visible row exists")
                }
            });
        let display = row
            .layout
            .closest_index_for_position(
                gpui::point(position.x - row.text_origin_x, position.y - row.origin_y),
                px(LINE_HEIGHT),
            )
            .unwrap_or_else(|index| index);
        let local = row.display.display_to_source(display);
        ByteOffset(row.range.start.0 + local.min(row.range.len() as usize) as u64)
    }

    fn scroll(&mut self, delta_x: f32, delta_y: f32, cx: &mut Context<Self>) {
        let viewport_height = self
            .viewport
            .map_or(0.0, |bounds| f32::from(bounds.size.height));
        let max_scroll =
            (self.display_map.total_visual_rows() as f32 * LINE_HEIGHT - viewport_height).max(0.0);
        let previous_y = self.scroll_y;
        self.scroll_y = (self.scroll_y - delta_y).clamp(0.0, max_scroll);
        if !self.display_map.soft_wrap() {
            self.scroll_x = (self.scroll_x - delta_x).max(0.0);
        }
        if (self.scroll_y - previous_y).abs() > 0.5 {
            cx.emit(super::SourceScrollEvent);
        }
        cx.notify();
    }

    pub(super) fn reveal_caret(&mut self, snapshot: &DocumentSnapshot) {
        let Some(viewport) = self.viewport else {
            return;
        };
        let Ok(line) = snapshot.line_index_at(self.selection.head()) else {
            return;
        };
        let top = self
            .hit_rows
            .iter()
            .find(|row| {
                self.selection.head() >= row.range.start && self.selection.head() <= row.range.end
            })
            .and_then(|row| {
                let local =
                    (self.selection.head().0 - row.range.start.0).min(row.range.len()) as usize;
                row.layout
                    .position_for_index(row.display.source_to_display(local), px(LINE_HEIGHT))
                    .map(|position| {
                        self.scroll_y
                            + f32::from(row.origin_y - viewport.top())
                            + f32::from(position.y)
                    })
            })
            .unwrap_or_else(|| self.display_map.line_start_visual_row(line.0) as f32 * LINE_HEIGHT);
        let bottom = top + LINE_HEIGHT;
        let height = f32::from(viewport.size.height);
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if bottom > self.scroll_y + height {
            self.scroll_y = (bottom - height).max(0.0);
        }
        if !self.display_map.soft_wrap()
            && let Some(row) = self.hit_rows.iter().find(|row| {
                self.selection.head() >= row.range.start && self.selection.head() <= row.range.end
            })
        {
            let source_local =
                (self.selection.head().0 - row.range.start.0).min(row.range.len()) as usize;
            let display_local = row.display.source_to_display(source_local);
            if let Some(position) = row
                .layout
                .position_for_index(display_local, px(LINE_HEIGHT))
            {
                let caret_x = row.text_origin_x + position.x;
                let text_left = row.text_origin_x + px(self.scroll_x);
                if caret_x > viewport.right() - px(12.0) {
                    self.scroll_x += f32::from(caret_x - viewport.right() + px(12.0));
                } else if caret_x < text_left {
                    self.scroll_x = (self.scroll_x - f32::from(text_left - caret_x)).max(0.0);
                }
            }
        }
    }

    pub(crate) fn top_source_anchor(&self, snapshot: &DocumentSnapshot) -> (ByteOffset, f32) {
        let visual_row = (self.scroll_y.max(0.0) / LINE_HEIGHT).floor() as u64;
        let line = self.display_map.line_at_visual_row(visual_row);
        let line_start = self.display_map.line_start_visual_row(line);
        let line_end = self
            .display_map
            .line_start_visual_row(line.saturating_add(1));
        let rows = line_end.saturating_sub(line_start).max(1);
        let fraction = (visual_row.saturating_sub(line_start) as f32 / rows as f32).clamp(0.0, 1.0);
        let source = snapshot
            .line_content_range(LineIndex(line))
            .map_or(ByteOffset(0), |range| range.start);
        (source, fraction)
    }

    pub(crate) fn scroll_to_source_anchor(
        &mut self,
        offset: ByteOffset,
        fraction: f32,
        cx: &mut Context<Self>,
    ) {
        let snapshot = self.snapshot(cx);
        let Ok(line) = snapshot.line_index_at(offset) else {
            return;
        };
        let start = self.display_map.line_start_visual_row(line.0);
        let end = self
            .display_map
            .line_start_visual_row(line.0.saturating_add(1));
        let within =
            ((end.saturating_sub(start).max(1) as f32) * fraction.clamp(0.0, 1.0)).floor() as u64;
        let target = start.saturating_add(within) as f32 * LINE_HEIGHT;
        if (self.scroll_y - target).abs() > 0.5 {
            self.scroll_y = target;
            cx.notify();
        }
    }

    pub(super) fn sync_selection_utf16(&mut self, snapshot: &DocumentSnapshot) {
        let range = self.selection.range();
        let Some(start) = snapshot
            .byte_to_utf16(range.start)
            .ok()
            .and_then(|offset| usize::try_from(offset.0).ok())
        else {
            return;
        };
        let Some(end) = snapshot
            .byte_to_utf16(range.end)
            .ok()
            .and_then(|offset| usize::try_from(offset.0).ok())
        else {
            return;
        };
        self.selection_utf16 = start..end;
        self.selection_utf16_reversed = self.selection.is_reversed();
    }
}

impl Focusable for SourceEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SourceEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.autofocus {
            self.autofocus = false;
            window.focus(&self.focus_handle, cx);
        }
        if self.focus_lost_subscription.is_none() {
            self.focus_lost_subscription = Some(cx.on_focus_lost(window, |this, _, cx| {
                this.finish_composition(cx);
            }));
        }
        let entity = cx.entity();
        let scroll_entity = entity.clone();
        div()
            .id("source-editor")
            .key_context("SourceEditor")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .bg(rgb(current_theme().background))
            .font_family("Menlo")
            .text_size(px(14.0))
            .line_height(px(LINE_HEIGHT))
            .cursor(gpui::CursorStyle::IBeam)
            .on_action(cx.listener(Self::delete_backward))
            .on_action(cx.listener(Self::delete_forward))
            .on_action(cx.listener(Self::move_left))
            .on_action(cx.listener(Self::move_right))
            .on_action(cx.listener(Self::move_word_left))
            .on_action(cx.listener(Self::move_word_right))
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::move_line_start))
            .on_action(cx.listener(Self::move_line_end))
            .on_action(cx.listener(Self::move_document_start))
            .on_action(cx.listener(Self::move_document_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::insert_tab))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(move |event, _, cx| {
                let delta = event.delta.pixel_delta(px(LINE_HEIGHT));
                scroll_entity.update(cx, |this, cx| {
                    this.scroll(f32::from(delta.x), f32::from(delta.y), cx)
                });
                cx.stop_propagation();
            })
            .child(EditorElement::new(entity))
    }
}
