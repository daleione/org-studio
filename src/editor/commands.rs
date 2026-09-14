use super::*;
use crate::motion::FOLD_MOTION;

impl SemanticEditor {
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
        if self.is_read_only(cx) {
            return;
        }
        self.finish_composition(cx);
        self.vertical_goal_x = None;
        let range = self.selection.range();
        let after = Selection::caret(ByteOffset(range.start.0 + text.len() as u64));
        let after_utf16 = self.selection_utf16.start + text.encode_utf16().count();
        let revision = self.session.read(cx).revision();
        let result = self.session.update(cx, |session, cx| {
            session.edit(
                DocumentCommand::new(
                    EditTransaction::new(revision, vec![TextEdit::new(range, text.to_owned())]),
                    history_before,
                    after,
                    origin,
                ),
                cx,
            )
        });
        if result.is_ok() {
            self.emacs_mark_active = false;
            self.hit_rows = Arc::from([]);
            self.pending_reveal_caret = true;
            self.command_feedback = None;
            self.selection = after;
            self.sync_selection_revision(cx);
            self.selection_utf16 = after_utf16..after_utf16;
            self.selection_utf16_reversed = false;
            cx.notify();
        }
    }

    fn delete_backward(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_read_only(cx) {
            return;
        }
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
        if self.is_read_only(cx) {
            return;
        }
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
        let extend = extend || self.emacs_mark_active;
        let snapshot = self.snapshot(cx);
        let inline_image_edge = self.selection.is_empty().then(|| {
            self.hit_rows
                .iter()
                .find(|row| {
                    row.inline_image_preview
                        && self.selection.head() >= row.range.start
                        && self.selection.head() <= row.range.end
                })
                .and_then(|row| {
                    if forward && self.selection.head() < row.range.end {
                        Some(row.range.end)
                    } else if !forward && self.selection.head() > row.range.start {
                        Some(row.range.start)
                    } else {
                        None
                    }
                })
        });
        let target = if !extend && !self.selection.is_empty() {
            if forward {
                self.selection.range().end
            } else {
                self.selection.range().start
            }
        } else if let Some(target) = inline_image_edge.flatten() {
            target
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

    pub(super) fn move_vertical(&mut self, delta: i64, extend: bool, cx: &mut Context<Self>) {
        self.finish_composition(cx);
        let extend = extend || self.emacs_mark_active;
        let snapshot = self.snapshot(cx);
        let head = self.selection.head();
        let head_line = snapshot.line_index_at(head).ok();
        if let Some(row) = self.hit_rows.iter().find(|row| {
            Some(row.line) == head_line && head >= row.range.start && head <= row.range.end
        }) {
            let source_local = (head.0 - row.range.start.0).min(row.range.len()) as usize;
            let display_local = row.display.source_to_display(source_local);
            if let Some(position) = row.position_for_display_index(display_local) {
                let goal_x = *self
                    .vertical_goal_x
                    .get_or_insert_with(|| f32::from(position.x));
                let target = self.hit_test(gpui::point(
                    row.text_origin_x + px(goal_x),
                    row.origin_y
                        + position.y
                        + px(delta as f32 * f32::from(row.line_height)
                            + f32::from(row.line_height) / 2.0),
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
            let target = if end { range.end } else { range.start };
            self.selection = if self.emacs_mark_active {
                self.selection.with_head(target)
            } else {
                Selection::caret(target)
            };
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
        if self.scroll_y != 0.0 {
            self.scroll_y = 0.0;
            self.minimap.note_viewport_changed();
        }
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
        self.emacs_mark_active = false;
        let snapshot = self.snapshot(cx);
        self.selection = Selection::new(ByteOffset(0), ByteOffset(snapshot.len_bytes()));
        self.sync_selection_utf16(&snapshot);
        cx.notify();
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_read_only(cx) {
            if self.activate_read_only_lines {
                window.dispatch_action(
                    Box::new(ActivateReadOnlyLine {
                        line: self.selected_line(cx),
                    }),
                    cx,
                );
            }
            return;
        }
        let snapshot = self.snapshot(cx);
        let path = self.session.read(cx).syntax_path().to_path_buf();
        let table_candidate = crate::document::DocumentFormat::detect(&path)
            .and_then(|format| {
                let line = snapshot.line_index_at(self.selection.head()).ok()?;
                let range = snapshot.line_content_range(line).ok()?;
                Some(crate::document::table::is_table_row(
                    &snapshot.copy_range(range),
                    format,
                ))
            })
            .unwrap_or(false);
        if !table_candidate {
            let newline = self.session.read(cx).newline_sequence();
            self.replace_selection(newline, EditOrigin::Newline, cx);
            return;
        }
        if let Some(context) =
            super::org_commands::EditorCommandContext::at(&path, &snapshot, self.selection.head())
            && matches!(
                context.kind,
                super::org_commands::EditorCommandKind::TableCell { .. }
            )
        {
            self.align_table_from_context(
                &snapshot,
                &context,
                super::org_commands::TableNavigation::NextRow,
                cx,
            );
            return;
        }
        let newline = self.session.read(cx).newline_sequence();
        self.replace_selection(newline, EditOrigin::Newline, cx);
    }

    fn insert_tab(&mut self, _: &InsertTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_read_only(cx) {
            return;
        }
        let snapshot = self.snapshot(cx);
        let path = self.session.read(cx).syntax_path().to_path_buf();
        let headings = crate::document::DocumentFormat::detect(&path)
            .map(|_| self.folds.heading_index(&path, &snapshot));
        let Some(context) = super::org_commands::EditorCommandContext::at_with_headings(
            &path,
            &snapshot,
            self.selection.head(),
            headings.as_deref(),
        ) else {
            return;
        };
        match context.kind {
            super::org_commands::EditorCommandKind::TableCell { .. } => {
                self.align_table_from_context(
                    &snapshot,
                    &context,
                    super::org_commands::TableNavigation::NextCell,
                    cx,
                );
            }
            super::org_commands::EditorCommandKind::List => {
                self.selection = Selection::caret(context.line_range.start);
                self.sync_selection_utf16(&snapshot);
                self.replace_selection("\t", EditOrigin::Other, cx);
            }
            super::org_commands::EditorCommandKind::Heading => {
                self.finish_fold_animation();
                let previous = self.folds.projection(&path, &snapshot);
                if self.folds.toggle_heading(&path, &snapshot, context.line.0) {
                    let target = self.folds.projection(&path, &snapshot);
                    self.animate_fold_layout(previous, target, context.line.0, window, cx);
                }
            }
            _ => {
                if !super::folding::is_block_boundary_candidate(&path, &snapshot, context.line.0) {
                    self.replace_selection("\t", EditOrigin::Typing, cx);
                    return;
                }
                self.finish_fold_animation();
                let previous = self.folds.projection(&path, &snapshot);
                if self.folds.toggle_block(&path, &snapshot, context.line.0) {
                    let target = self.folds.projection(&path, &snapshot);
                    self.animate_fold_layout(previous, target, context.line.0, window, cx);
                } else {
                    self.replace_selection("\t", EditOrigin::Typing, cx);
                }
            }
        }
    }

    pub(super) fn finish_fold_animation(&mut self) {
        let Some(animation) = self.fold_animation.take() else {
            return;
        };
        self.fold_animation_revision = self.fold_animation_revision.wrapping_add(1);
        self.apply_fold_projection(
            super::folding::EditorFoldProjection {
                hidden_ranges: animation.target_hidden_ranges.to_vec(),
                marker_lines: animation.target_marker_lines.as_ref().clone(),
            },
            animation.anchor_line,
            animation.anchor_viewport_y,
        );
    }

    fn complete_fold_animation(&mut self) {
        let Some(animation) = self.fold_animation.take() else {
            return;
        };
        self.fold_animation_revision = self.fold_animation_revision.wrapping_add(1);
        self.fold_markers = animation.target_marker_lines;
        self.display_map
            .set_hidden_ranges(animation.target_hidden_ranges.to_vec());
        if let Some(viewport) = self.viewport {
            let max_scroll =
                (self.display_map.total_height() - f32::from(viewport.size.height)).max(0.0);
            self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);
        }
        self.minimap.invalidate_raster();
        self.minimap.note_viewport_changed();
    }

    fn stabilize_fold_animation_anchor(&mut self) {
        let Some(animation) = self.fold_animation.as_ref() else {
            return;
        };
        let Some(viewport) = self.viewport else {
            return;
        };
        let anchor_line = animation.anchor_line;
        let anchor_viewport_y = animation.anchor_viewport_y;
        self.scroll_y = fold_anchor_scroll_y(
            self.animated_line_start_y(anchor_line),
            self.animated_line_height_px(anchor_line),
            anchor_viewport_y,
            f32::from(viewport.size.height),
            self.animated_document_height(),
        );
        self.minimap.note_viewport_changed();
    }

    fn animate_fold_layout(
        &mut self,
        previous: super::folding::EditorFoldProjection,
        target: super::folding::EditorFoldProjection,
        anchor_line: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if previous == target {
            return;
        }
        let anchor_viewport_y = self.display_map.line_start_y(anchor_line) - self.scroll_y;
        let newly_hidden =
            super::folding::subtract_ranges(&target.hidden_ranges, &previous.hidden_ranges);
        let newly_visible =
            super::folding::subtract_ranges(&previous.hidden_ranges, &target.hidden_ranges);
        let (changed_ranges, collapsing) = match (newly_hidden.is_empty(), newly_visible.is_empty())
        {
            (false, true) => (newly_hidden, true),
            (true, false) => (newly_visible, false),
            _ => {
                self.apply_fold_projection(target, anchor_line, anchor_viewport_y);
                cx.notify();
                return;
            }
        };
        if cx.reduce_motion() {
            self.apply_fold_projection(target, anchor_line, anchor_viewport_y);
            cx.notify();
            return;
        }

        self.fold_animation_revision = self.fold_animation_revision.wrapping_add(1);
        let revision = self.fold_animation_revision;
        if !collapsing {
            self.apply_fold_projection(target.clone(), anchor_line, anchor_viewport_y);
        } else {
            self.fold_markers = Arc::new(target.marker_lines.clone());
        }
        self.fold_animation = Some(EditorFoldAnimation {
            revision,
            changed_ranges: changed_ranges.into(),
            target_hidden_ranges: target.hidden_ranges.into(),
            target_marker_lines: Arc::new(target.marker_lines),
            collapsing,
            anchor_line,
            anchor_viewport_y,
            started_at: None,
            progress: 0.0,
        });
        self.stabilize_fold_animation_anchor();
        self.minimap.invalidate_raster();
        self.schedule_fold_animation_frame(revision, window, cx);
        cx.notify();
    }

    fn apply_fold_projection(
        &mut self,
        projection: super::folding::EditorFoldProjection,
        anchor_line: u64,
        anchor_viewport_y: f32,
    ) {
        self.fold_markers = Arc::new(projection.marker_lines);
        self.display_map.set_hidden_ranges(projection.hidden_ranges);
        if let Some(viewport) = self.viewport {
            self.scroll_y = fold_anchor_scroll_y(
                self.display_map.line_start_y(anchor_line),
                self.display_map.line_height_px(anchor_line),
                anchor_viewport_y,
                f32::from(viewport.size.height),
                self.display_map.total_height(),
            );
        }
        self.minimap.invalidate_raster();
        self.minimap.note_viewport_changed();
    }

    fn schedule_fold_animation_frame(
        &mut self,
        revision: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.on_next_frame(window, move |this, window, cx| {
            let Some(animation) = this.fold_animation.as_mut() else {
                return;
            };
            if animation.revision != revision {
                return;
            }
            let Some(started_at) = animation.started_at else {
                animation.started_at = Some(Instant::now());
                this.schedule_fold_animation_frame(revision, window, cx);
                return;
            };
            animation.progress = FOLD_MOTION.sample(started_at, Instant::now()).progress;
            let complete = animation.progress >= 1.0;
            this.stabilize_fold_animation_anchor();
            cx.notify();
            if complete {
                cx.on_next_frame(window, move |this, _, cx| {
                    if this.fold_animation.as_ref().is_some_and(|animation| {
                        animation.revision == revision && animation.progress >= 1.0
                    }) {
                        this.complete_fold_animation();
                        cx.notify();
                    }
                });
            } else {
                this.schedule_fold_animation_frame(revision, window, cx);
            }
        });
    }

    fn shift_tab(&mut self, _: &ShiftTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_read_only(cx) {
            return;
        }
        let snapshot = self.snapshot(cx);
        let path = self.session.read(cx).syntax_path().to_path_buf();
        let headings = crate::document::DocumentFormat::detect(&path)
            .map(|_| self.folds.heading_index(&path, &snapshot));
        let Some(context) = super::org_commands::EditorCommandContext::at_with_headings(
            &path,
            &snapshot,
            self.selection.head(),
            headings.as_deref(),
        ) else {
            return;
        };
        if matches!(
            context.kind,
            super::org_commands::EditorCommandKind::TableCell { .. }
        ) {
            self.align_table_from_context(
                &snapshot,
                &context,
                super::org_commands::TableNavigation::PreviousCell,
                cx,
            );
        } else if matches!(context.kind, super::org_commands::EditorCommandKind::NonOrg) {
            self.replace_selection("\t", EditOrigin::Typing, cx);
        } else {
            self.finish_fold_animation();
            let previous = self.folds.projection(&path, &snapshot);
            if self.folds.cycle_global(&path, &snapshot) {
                let target = self.folds.projection(&path, &snapshot);
                let anchor_line = self.normalize_selection_for_fold(&snapshot, &target);
                self.animate_fold_layout(previous, target, anchor_line, window, cx);
            }
        }
    }

    fn normalize_selection_for_fold(
        &mut self,
        snapshot: &DocumentSnapshot,
        projection: &super::folding::EditorFoldProjection,
    ) -> u64 {
        let line = snapshot
            .line_index_at(self.selection.head())
            .map_or(0, |line| line.0);
        if !projection
            .hidden_ranges
            .iter()
            .any(|range| range.contains(&line))
        {
            return line;
        }
        let visible = (0..=line)
            .rev()
            .find(|candidate| {
                !projection
                    .hidden_ranges
                    .iter()
                    .any(|range| range.contains(candidate))
            })
            .unwrap_or(0);
        if let Ok(range) = snapshot.line_content_range(LineIndex(visible)) {
            self.selection = Selection::caret(range.end);
            self.sync_selection_utf16(snapshot);
        }
        visible
    }

    fn align_table(&mut self, _: &AlignTable, _: &mut Window, cx: &mut Context<Self>) {
        if !self.align_table_at_selection(cx) {
            self.command_feedback = Some("当前光标不在表格中".into());
            cx.notify();
        }
    }

    pub(crate) fn align_table_at_selection(&mut self, cx: &mut Context<Self>) -> bool {
        if self.is_read_only(cx) {
            return false;
        }
        let snapshot = self.snapshot(cx);
        let path = self.session.read(cx).syntax_path().to_path_buf();
        let Some(context) =
            super::org_commands::EditorCommandContext::at(&path, &snapshot, self.selection.head())
        else {
            return false;
        };
        if !matches!(
            context.kind,
            super::org_commands::EditorCommandKind::TableCell { .. }
        ) {
            return false;
        }
        self.align_table_from_context(
            &snapshot,
            &context,
            super::org_commands::TableNavigation::Stay,
            cx,
        )
    }

    pub(super) fn align_table_from_context(
        &mut self,
        snapshot: &DocumentSnapshot,
        context: &super::org_commands::EditorCommandContext,
        navigation: super::org_commands::TableNavigation,
        cx: &mut Context<Self>,
    ) -> bool {
        let newline = self.session.read(cx).newline_sequence().to_owned();
        let Some(alignment) =
            super::org_commands::align_table(snapshot, context, &newline, navigation)
        else {
            return false;
        };
        let before = self.selection;
        let after = Selection::caret(alignment.caret);
        let revision = snapshot.revision();
        if snapshot.copy_range(alignment.range) == alignment.replacement {
            self.command_feedback = None;
            self.selection = after;
            self.selection_revision = revision;
            self.sync_selection_utf16(snapshot);
            self.pending_reveal_caret = true;
            cx.notify();
            return true;
        }
        if self
            .session
            .update(cx, |session, cx| {
                session.edit(
                    DocumentCommand::new(
                        EditTransaction::new(
                            revision,
                            vec![TextEdit::new(alignment.range, alignment.replacement)],
                        ),
                        before,
                        after,
                        EditOrigin::Other,
                    ),
                    cx,
                )
            })
            .is_ok()
        {
            self.command_feedback = None;
            self.selection = after;
            let snapshot = self.snapshot(cx);
            self.selection_revision = snapshot.revision();
            self.sync_selection_utf16(&snapshot);
            self.hit_rows = Arc::from([]);
            self.pending_reveal_caret = true;
            cx.notify();
            true
        } else {
            false
        }
    }

    fn toggle_todo(&mut self, _: &ToggleTodo, _: &mut Window, cx: &mut Context<Self>) {
        self.apply_line_cycle(super::org_commands::cycle_todo, cx);
    }

    fn toggle_inline_image_previews(
        &mut self,
        _: &ToggleInlineImagePreviews,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = self.snapshot(cx);
        let image_line = snapshot
            .line_index_at(self.selection.head())
            .ok()
            .and_then(|line| snapshot.line_content_range(line).ok())
            .filter(|range| {
                crate::org_syntax::standalone_image_path(&snapshot.copy_range(*range)).is_some()
            });

        if let Some(range) = image_line {
            let visible = self.previews_inline_image_at(range.start);
            self.inline_image_preview_overrides
                .insert(range.start.0, !visible);
        } else {
            self.inline_image_previews = !self.inline_image_previews;
            self.inline_image_preview_overrides.clear();
        }
        self.display_map.invalidate_layout();
        self.hit_rows = Arc::from([]);
        self.minimap.invalidate_raster();
        cx.notify();
    }

    fn toggle_checkbox(&mut self, _: &ToggleCheckbox, _: &mut Window, cx: &mut Context<Self>) {
        let snapshot = self.snapshot(cx);
        if self
            .session
            .read(cx)
            .path()
            .extension()
            .and_then(|value| value.to_str())
            != Some("org")
        {
            self.command_feedback = Some("此命令仅适用于 Org 文档".into());
            cx.notify();
            return;
        }
        let Some(target) = snapshot
            .line_index_at(self.selection.head())
            .ok()
            .and_then(|line| snapshot.line_content_range(line).ok())
            .and_then(|line_range| {
                let text = snapshot.copy_range(line_range);
                crate::org_syntax::command::checkbox_token(&text).map(|(token, _)| {
                    ByteRange::new(
                        line_range.start.0 + token.start as u64,
                        line_range.start.0 + token.end as u64,
                    )
                })
            })
        else {
            self.command_feedback = Some("当前行不适用此命令".into());
            cx.notify();
            return;
        };
        let Some(edits) = crate::org_syntax::command::checkbox_transaction(&snapshot, target)
        else {
            self.command_feedback = Some("当前行不适用此命令".into());
            cx.notify();
            return;
        };
        let before = self.selection;
        let after = Selection::caret(map_offset_through_edits(before.head(), &edits));
        if self
            .session
            .update(cx, |session, cx| {
                session.edit(
                    DocumentCommand::new(
                        EditTransaction::new(snapshot.revision(), edits),
                        before,
                        after,
                        EditOrigin::Other,
                    ),
                    cx,
                )
            })
            .is_ok()
        {
            self.command_feedback = None;
            self.selection = after;
            let snapshot = self.snapshot(cx);
            self.selection_revision = snapshot.revision();
            self.sync_selection_utf16(&snapshot);
            self.hit_rows = Arc::from([]);
            self.pending_reveal_caret = true;
            cx.notify();
        }
    }

    fn apply_line_cycle(&mut self, cycle: super::org_commands::LineCycle, cx: &mut Context<Self>) {
        let snapshot = self.snapshot(cx);
        if self
            .session
            .read(cx)
            .path()
            .extension()
            .and_then(|value| value.to_str())
            != Some("org")
        {
            self.command_feedback = Some("此命令仅适用于 Org 文档".into());
            cx.notify();
            return;
        }
        let Ok(line) = snapshot.line_index_at(self.selection.head()) else {
            return;
        };
        let Ok(line_range) = snapshot.line_content_range(line) else {
            return;
        };
        let text = snapshot.copy_range(line_range);
        let Some((local, replacement)) = cycle(&text) else {
            self.command_feedback = Some("当前行不适用此命令".into());
            cx.notify();
            return;
        };
        let range = ByteRange::new(
            line_range.start.0 + local.start as u64,
            line_range.start.0 + local.end as u64,
        );
        let head = self.selection.head();
        let shift = replacement.len() as i128 - range.len() as i128;
        let mapped_head = if head <= range.start {
            head
        } else if head >= range.end {
            ByteOffset((head.0 as i128 + shift).max(0) as u64)
        } else {
            ByteOffset(range.start.0 + replacement.len() as u64)
        };
        let after = Selection::caret(mapped_head);
        let before = self.selection;
        if self
            .session
            .update(cx, |session, cx| {
                session.edit(
                    DocumentCommand::new(
                        EditTransaction::new(
                            snapshot.revision(),
                            vec![TextEdit::new(range, replacement)],
                        ),
                        before,
                        after,
                        EditOrigin::Other,
                    ),
                    cx,
                )
            })
            .is_ok()
        {
            self.command_feedback = None;
            self.selection = after;
            let snapshot = self.snapshot(cx);
            self.selection_revision = snapshot.revision();
            self.sync_selection_utf16(&snapshot);
            self.hit_rows = Arc::from([]);
            self.pending_reveal_caret = true;
            cx.notify();
        }
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.finish_composition(cx);
        self.emacs_mark_active = false;
        if let Ok(HistoryOutcome::Applied(selection)) =
            self.session.update(cx, |session, cx| session.undo(cx))
        {
            self.hit_rows = Arc::from([]);
            self.pending_reveal_caret = true;
            self.selection = selection;
            let snapshot = self.snapshot(cx);
            self.selection_revision = snapshot.revision();
            self.sync_selection_utf16(&snapshot);
            cx.notify();
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.finish_composition(cx);
        self.emacs_mark_active = false;
        if let Ok(HistoryOutcome::Applied(selection)) =
            self.session.update(cx, |session, cx| session.redo(cx))
        {
            self.hit_rows = Arc::from([]);
            self.pending_reveal_caret = true;
            self.selection = selection;
            let snapshot = self.snapshot(cx);
            self.selection_revision = snapshot.revision();
            self.sync_selection_utf16(&snapshot);
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

    fn kill_line(&mut self, _: &KillLine, _: &mut Window, cx: &mut Context<Self>) {
        let snapshot = self.snapshot(cx);
        let point = self.selection.head();
        let Some(range) = snapshot.line_index_at(point).ok().and_then(|line| {
            let content = snapshot.line_content_range(line).ok()?;
            let complete = snapshot.line_range(line).ok()?;
            if point < content.end {
                Some(ByteRange::new(point.0, content.end.0))
            } else if point < complete.end {
                Some(ByteRange::new(point.0, complete.end.0))
            } else {
                None
            }
        }) else {
            return;
        };
        self.kill_range(&snapshot, range, cx);
    }

    fn kill_word(&mut self, _: &KillWord, _: &mut Window, cx: &mut Context<Self>) {
        let snapshot = self.snapshot(cx);
        let point = self.selection.head();
        self.kill_range(
            &snapshot,
            ByteRange::new(point.0, word_boundary(&snapshot, point, true).0),
            cx,
        );
    }

    fn backward_kill_word(&mut self, _: &BackwardKillWord, _: &mut Window, cx: &mut Context<Self>) {
        let snapshot = self.snapshot(cx);
        let point = self.selection.head();
        self.kill_range(
            &snapshot,
            ByteRange::new(word_boundary(&snapshot, point, false).0, point.0),
            cx,
        );
    }

    fn kill_range(
        &mut self,
        snapshot: &DocumentSnapshot,
        range: ByteRange,
        cx: &mut Context<Self>,
    ) {
        if range.is_empty() || self.is_read_only(cx) {
            return;
        }
        let history_before = self.selection;
        cx.write_to_clipboard(ClipboardItem::new_string(snapshot.copy_range(range)));
        self.selection = Selection::new(range.start, range.end);
        self.sync_selection_utf16(snapshot);
        self.replace_selection_recording("", EditOrigin::Cut, history_before, cx);
    }

    fn open_line(&mut self, _: &OpenLine, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_read_only(cx) {
            return;
        }
        self.finish_composition(cx);
        self.vertical_goal_x = None;
        self.emacs_mark_active = false;
        let before = self.selection;
        let point = before.head();
        let after = Selection::caret(point);
        let newline = self.session.read(cx).newline_sequence().to_owned();
        let revision = self.session.read(cx).revision();
        let result = self.session.update(cx, |session, cx| {
            session.edit(
                DocumentCommand::new(
                    EditTransaction::new(
                        revision,
                        vec![TextEdit::new(ByteRange::new(point.0, point.0), newline)],
                    ),
                    before,
                    after,
                    EditOrigin::Newline,
                ),
                cx,
            )
        });
        if result.is_ok() {
            self.hit_rows = Arc::from([]);
            self.pending_reveal_caret = true;
            self.selection = after;
            let snapshot = self.snapshot(cx);
            self.selection_revision = snapshot.revision();
            self.sync_selection_utf16(&snapshot);
            cx.notify();
        }
    }

    fn transpose_chars(&mut self, _: &TransposeChars, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_read_only(cx) || !self.selection.is_empty() {
            return;
        }
        let snapshot = self.snapshot(cx);
        let point = self.selection.head();
        let Ok(line) = snapshot.line_index_at(point) else {
            return;
        };
        let Ok(line_range) = snapshot.line_content_range(line) else {
            return;
        };
        let (start, middle, end) = if point >= line_range.end {
            let middle = snapshot.previous_grapheme_boundary(point).unwrap_or(point);
            let start = snapshot
                .previous_grapheme_boundary(middle)
                .unwrap_or(middle);
            (start, middle, point)
        } else {
            if point <= line_range.start {
                return;
            }
            let start = snapshot.previous_grapheme_boundary(point).unwrap_or(point);
            let end = snapshot.next_grapheme_boundary(point).unwrap_or(point);
            (start, point, end)
        };
        if start == middle || middle == end {
            return;
        }
        let mut replacement = snapshot.copy_range(ByteRange::new(middle.0, end.0));
        replacement.push_str(&snapshot.copy_range(ByteRange::new(start.0, middle.0)));
        let history_before = self.selection;
        self.selection = Selection::new(start, end);
        self.sync_selection_utf16(&snapshot);
        self.replace_selection_recording(&replacement, EditOrigin::Other, history_before, cx);
    }

    fn set_mark(&mut self, _: &SetMark, _: &mut Window, cx: &mut Context<Self>) {
        self.finish_composition(cx);
        let point = self.selection.head();
        self.emacs_mark_active = true;
        self.selection = Selection::caret(point);
        let snapshot = self.snapshot(cx);
        self.sync_selection_utf16(&snapshot);
        cx.notify();
    }

    fn keyboard_quit(&mut self, _: &KeyboardQuit, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_inline(cx);
        self.dismiss_todo(cx);
        self.todo.hover_range = None;
        self.dismiss_timestamp(cx);
        self.finish_composition(cx);
        self.emacs_mark_active = false;
        self.command_feedback = None;
        self.selection = Selection::caret(self.selection.head());
        let snapshot = self.snapshot(cx);
        self.sync_selection_utf16(&snapshot);
        cx.notify();
    }

    fn scroll_page_down(&mut self, _: &ScrollPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_page(true, cx);
    }

    fn scroll_page_up(&mut self, _: &ScrollPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_page(false, cx);
    }

    fn scroll_page(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.finish_composition(cx);
        self.vertical_goal_x = None;
        let Some(viewport) = self.viewport else {
            return;
        };
        let viewport_height = f32::from(viewport.size.height);
        let amount = (viewport_height - self.base_line_height()).max(1.0);
        let max_scroll = (self.animated_document_height() - viewport_height).max(0.0);
        let previous_scroll = self.scroll_y;
        self.scroll_y = if forward {
            self.scroll_y + amount
        } else {
            self.scroll_y - amount
        }
        .clamp(0.0, max_scroll);

        let snapshot = self.snapshot(cx);
        let point = self.selection.head();
        if let Ok(line) = snapshot.line_index_at(point) {
            let caret_top = self.animated_line_start_y(line.0);
            let caret_bottom = caret_top
                + self
                    .animated_line_height_px(line.0)
                    .min(self.base_line_height());
            if caret_bottom <= self.scroll_y || caret_top >= self.scroll_y + viewport_height {
                let target_y = if forward {
                    self.scroll_y + 0.5
                } else {
                    self.scroll_y + viewport_height - self.base_line_height()
                };
                let target_line = self.animated_line_at_y(target_y.max(0.0));
                let column = snapshot
                    .line_and_column_at(point)
                    .map_or(0, |(_, column)| column);
                if let Ok(target) = snapshot.byte_at_line_column(LineIndex(target_line), column) {
                    self.selection = if self.emacs_mark_active {
                        self.selection.with_head(target)
                    } else {
                        Selection::caret(target)
                    };
                    self.sync_selection_utf16(&snapshot);
                }
            }
        }
        let scroll_delta = self.scroll_y - previous_scroll;
        if scroll_delta.abs() > 0.5 {
            self.minimap.note_viewport_scrolled(scroll_delta);
        }
        cx.notify();
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
        self.emacs_mark_active = false;
        if self.inline_mouse_down(event, cx) {
            return;
        }
        if let Some(source_offset) = self
            .source_run_buttons
            .iter()
            .find(|button| button.bounds.contains(&event.position))
            .map(|button| button.source_offset)
        {
            self.show_source_run_feedback(source_offset, super::SourceRunPhase::Running, cx);
            window.dispatch_action(Box::new(super::RunSourceBlockAt { source_offset }), cx);
            return;
        }
        if (event.modifiers.platform || event.modifiers.control)
            && let Some(link) = self
                .link_at_position(event.position)
                .and_then(|index| self.link_hits.get(index))
                .cloned()
        {
            self.open_link(&link, cx);
            return;
        }
        if let Some(bounds) = self.minimap.bounds {
            let x = f32::from(event.position.x);
            if (x - f32::from(bounds.left())).abs() <= super::minimap::RESIZE_HANDLE {
                self.minimap.resizing = Some((x, self.minimap.width));
                cx.stop_propagation();
                return;
            }
            if bounds.contains(&event.position) {
                let pointer = f32::from(event.position.y - bounds.top());
                let mut geometry = self.minimap_viewport_geometry(bounds);
                let clicked_thumb = pointer >= geometry.thumb_top
                    && pointer <= geometry.thumb_top + geometry.thumb_height;
                if !clicked_thumb {
                    self.seek_from_minimap(event.position.y, cx);
                    geometry = self.minimap_viewport_geometry(bounds);
                }
                let start_top = if clicked_thumb {
                    geometry.thumb_top
                } else {
                    (pointer - geometry.thumb_height / 2.0).clamp(
                        0.0,
                        (geometry.interaction_height - geometry.thumb_height).max(0.0),
                    )
                };
                self.minimap.drag = Some(crate::minimap::DragSession {
                    start_pointer_y: pointer,
                    start_thumb_top: start_top,
                    start_ratio: geometry.scroll_ratio,
                    current_thumb_top: start_top,
                });
                cx.stop_propagation();
                return;
            }
        }
        if let Some((line, text_x)) = self
            .hit_rows
            .iter()
            .min_by(|left, right| {
                vertical_distance(event.position.y, left)
                    .total_cmp(&vertical_distance(event.position.y, right))
            })
            .map(|row| (row.line.0, row.text_origin_x))
            && event.position.x < text_x
            && self.fold_markers.contains(&line)
        {
            let snapshot = self.snapshot(cx);
            let path = self.session.read(cx).syntax_path().to_path_buf();
            self.finish_fold_animation();
            let previous = self.folds.projection(&path, &snapshot);
            self.folds.expand_at(&snapshot, line);
            let target = self.folds.projection(&path, &snapshot);
            self.animate_fold_layout(previous, target, line, window, cx);
            return;
        }
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
        self.inline_hover(event.position, cx);
        self.todo_hover(event.position, cx);
        self.timestamp_hover(event.position, cx);
        let source_run_button_hovered = self
            .source_run_buttons
            .iter()
            .any(|button| button.bounds.contains(&event.position));
        if source_run_button_hovered != self.source_run_button_hovered {
            self.source_run_button_hovered = source_run_button_hovered;
            cx.notify();
        }
        let hovered_link = self
            .link_at_position(event.position)
            .and_then(|index| self.link_hits.get(index))
            .map(|hit| (hit.line, hit.display_range.clone()));
        if hovered_link.as_ref() != self.hovered_link.as_ref() {
            self.hovered_link = hovered_link.clone();
            self.hover_position = hovered_link.map(|_| event.position);
            cx.notify();
        }
        if let Some((start_x, start_width)) = self.minimap.resizing {
            self.minimap.width = (start_width + start_x - f32::from(event.position.x))
                .clamp(super::minimap::MIN_WIDTH, super::minimap::MAX_WIDTH);
            self.shape_cache.clear();
            cx.notify();
            return;
        }
        if self.minimap.drag.is_some() {
            self.seek_from_minimap(event.position.y, cx);
            return;
        }
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

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.minimap.drag = None;
        if self.minimap.resizing.take().is_some() {
            cx.emit(super::EditorMinimapWidthEvent(self.minimap.width));
        }
        self.is_selecting = false;
        self.drag_position = None;
        self.autoscroll_task = None;
        self.inline_mouse_up(event.position, cx);
    }

    pub(super) fn seek_from_minimap(&mut self, pointer_y: Pixels, cx: &mut Context<Self>) {
        let Some(bounds) = self.minimap.bounds else {
            return;
        };
        let geometry = self.minimap_viewport_geometry(bounds);
        let pointer = f32::from(pointer_y - bounds.top());
        let viewport_height = self
            .viewport
            .map_or(0.0, |viewport| f32::from(viewport.size.height));
        let (total_units, visible_top, visible_bottom) =
            self.minimap_source_viewport(viewport_height);
        let viewport_units = (visible_bottom - visible_top).max(0.0);
        let max_scroll_pixels = (self.animated_document_height() - viewport_height).max(0.0);
        let target_y = if let Some(session) = self.minimap.drag {
            let (ratio, thumb_top) = crate::minimap::drag_target(
                pointer,
                session,
                geometry.thumb_height,
                geometry.interaction_height,
            );
            if let Some(session) = self.minimap.drag.as_mut() {
                session.current_thumb_top = thumb_top;
            }
            ratio * max_scroll_pixels
        } else {
            let density = crate::minimap::Density::for_width(f32::from(bounds.size.width));
            let line_height = self.minimap.line_height(density);
            let clicked_unit =
                geometry.content_top + ((pointer - density.edge_padding()) / line_height).max(0.0);
            let max_scroll_units = (total_units - viewport_units).max(0.0);
            let ratio = if max_scroll_units > 0.0 {
                (clicked_unit / max_scroll_units).clamp(0.0, 1.0)
            } else {
                0.0
            };
            ratio * max_scroll_pixels
        };
        let previous_y = self.scroll_y;
        self.scroll_y = target_y.clamp(0.0, max_scroll_pixels);
        if (self.scroll_y - previous_y).abs() > 0.5 {
            self.minimap.note_viewport_changed();
        }
        cx.notify();
    }

    pub(super) fn minimap_viewport_geometry(
        &self,
        bounds: Bounds<Pixels>,
    ) -> super::minimap::ViewportGeometry {
        let viewport_height = f32::from(bounds.size.height);
        let (_, scroll_top, visible_bottom) = self.minimap_source_viewport(viewport_height);
        self.minimap_viewport_geometry_for_source_range(bounds, scroll_top, visible_bottom)
    }

    pub(super) fn minimap_viewport_geometry_for_source_range(
        &self,
        bounds: Bounds<Pixels>,
        visible_top: f32,
        visible_bottom: f32,
    ) -> super::minimap::ViewportGeometry {
        let viewport_height = f32::from(bounds.size.height);
        let active_layout = self.minimap.active_layout();
        let minimap_layout = active_layout.as_deref().unwrap_or(&self.display_map);
        let total_units =
            minimap_layout.total_height() / minimap_layout.base_line_height().max(1.0);
        let density = crate::minimap::Density::for_width(f32::from(bounds.size.width));
        let line_height = self.minimap.line_height(density);
        let viewport_units = (visible_bottom - visible_top).max(0.0);
        let max_scroll_units = (total_units - viewport_units).max(0.0);
        let scroll_ratio = if visible_bottom >= total_units - f32::EPSILON {
            1.0
        } else if max_scroll_units > 0.0 {
            (visible_top / max_scroll_units).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let viewport = crate::minimap::projection_viewport_with_line_height(
            total_units,
            visible_top,
            visible_bottom,
            scroll_ratio,
            viewport_height,
            density,
            line_height,
        );
        if self.minimap.active_frame_uses_prepared_layout() {
            // The complete Editor layout is now the same immutable geometry used by the active
            // minimap frame. Follow the shared Editor/Reading projection directly; the legacy
            // stabilization camera is only needed during the short sparse-layout bootstrap.
            viewport
        } else {
            self.minimap.stabilize_viewport(
                viewport,
                total_units,
                visible_top,
                visible_bottom,
                viewport_height,
                density,
            )
        }
    }

    pub(super) fn minimap_source_viewport(&self, viewport_height: f32) -> (f32, f32, f32) {
        let active_frame = self.minimap.active_frame();
        let minimap_layout = active_frame
            .as_ref()
            .map_or(&self.display_map, |frame| frame.layout.as_ref());
        let base_line_height = minimap_layout.base_line_height().max(1.0);
        let minimap_document_height = minimap_layout.total_height();
        let total_units = minimap_document_height / base_line_height;
        if total_units <= 0.0 {
            return (0.0, 0.0, 0.0);
        }
        let live_document_height = self.animated_document_height();
        let live_max_scroll = (live_document_height - viewport_height).max(0.0);
        let live_scroll_y = self.scroll_y.clamp(0.0, live_max_scroll);
        let at_end = live_scroll_y + viewport_height + 0.5 >= live_document_height;
        let prepared_max_scroll = (minimap_document_height - viewport_height).max(0.0);
        let minimap_scroll_y = active_frame.as_ref().map_or_else(
            || {
                super::minimap::source_viewport_for_layout(
                    minimap_layout,
                    &self.display_map,
                    live_scroll_y,
                    live_document_height,
                    viewport_height,
                )
                .visible_top
                    * base_line_height
            },
            |frame| {
                if self.minimap.active_frame_uses_prepared_layout() {
                    return super::minimap::source_viewport_for_layout(
                        frame.layout.as_ref(),
                        &self.display_map,
                        live_scroll_y,
                        live_document_height,
                        viewport_height,
                    )
                    .visible_top
                        * base_line_height;
                }
                self.minimap.project_scroll_camera(
                    &self.display_map,
                    frame,
                    live_scroll_y,
                    live_max_scroll,
                    prepared_max_scroll,
                )
            },
        );
        let visible_top = minimap_scroll_y / base_line_height;
        let visible_bottom = if at_end {
            total_units
        } else {
            ((minimap_scroll_y + viewport_height).min(minimap_document_height) / base_line_height)
                .clamp(visible_top, total_units)
        };
        (total_units, visible_top, visible_bottom)
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
            let max_scroll =
                (self.animated_document_height() - f32::from(viewport.size.height)).max(0.0);
            let speed = distance.signum() * (distance.abs() / 8.0).clamp(4.0, 64.0);
            let previous_y = self.scroll_y;
            self.scroll_y = (self.scroll_y + speed).clamp(0.0, max_scroll);
            let scroll_delta = self.scroll_y - previous_y;
            if scroll_delta.abs() > 0.5 {
                self.minimap.note_viewport_scrolled(scroll_delta);
            }
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
            .find(|row| position.y >= row.visible_top && position.y < row.visible_bottom)
            .unwrap_or_else(|| {
                self.hit_rows
                    .iter()
                    .min_by(|left, right| {
                        vertical_distance(position.y, left)
                            .total_cmp(&vertical_distance(position.y, right))
                    })
                    .unwrap_or(first)
            });
        if row.inline_image_preview {
            return if position.x < row.text_origin_x + px(8.0) {
                row.range.start
            } else {
                row.range.end
            };
        }
        let display = row.closest_display_index(gpui::point(
            position.x - row.text_origin_x,
            position.y - row.origin_y,
        ));
        let local = row.display.display_to_source(display);
        ByteOffset(row.range.start.0 + local.min(row.range.len() as usize) as u64)
    }

    pub(super) fn scroll(&mut self, delta_x: f32, delta_y: f32, cx: &mut Context<Self>) {
        self.dismiss_inline(cx);
        self.dismiss_todo(cx);
        self.todo.hover_range = None;
        self.dismiss_timestamp(cx);
        self.timestamp.hover_range = None;
        let viewport_height = self
            .viewport
            .map_or(0.0, |bounds| f32::from(bounds.size.height));
        let max_scroll = (self.animated_document_height() - viewport_height).max(0.0);
        let previous_y = self.scroll_y;
        self.scroll_y = (self.scroll_y - delta_y).clamp(0.0, max_scroll);
        if !self.display_map.soft_wrap() {
            self.scroll_x = (self.scroll_x - delta_x).clamp(0.0, self.max_horizontal_scroll());
        }
        let scroll_delta = self.scroll_y - previous_y;
        if scroll_delta.abs() > 0.5 {
            self.minimap.note_viewport_scrolled(scroll_delta);
        }
        cx.notify();
    }

    pub(super) fn max_horizontal_scroll(&self) -> f32 {
        if self.display_map.soft_wrap() {
            return 0.0;
        }
        let Some(viewport) = self.viewport else {
            return 0.0;
        };
        if self.hit_rows.is_empty() {
            return 0.0;
        }
        let text_left = self
            .hit_rows
            .iter()
            .map(|row| f32::from(row.text_origin_x) + self.scroll_x)
            .fold(f32::INFINITY, f32::min);
        let text_right = f32::from(
            self.minimap
                .bounds
                .map_or(viewport.right(), |bounds| bounds.left()),
        );
        let available_width = (text_right - text_left).max(1.0);
        let content_right = self
            .hit_rows
            .iter()
            .map(|row| f32::from(row.text_origin_x) + self.scroll_x + f32::from(row.visual_width()))
            .fold(text_left, f32::max);
        let content_width = (content_right - text_left).max(0.0);
        horizontal_scroll_limit(content_width, available_width)
    }

    pub(super) fn reveal_caret(&mut self, snapshot: &DocumentSnapshot) {
        let Some(viewport) = self.viewport else {
            return;
        };
        let Ok(line) = snapshot.line_index_at(self.selection.head()) else {
            return;
        };
        let measured = self
            .hit_rows
            .iter()
            .find(|row| {
                row.line == line
                    && self.selection.head() >= row.range.start
                    && self.selection.head() <= row.range.end
            })
            .and_then(|row| {
                let local =
                    (self.selection.head().0 - row.range.start.0).min(row.range.len()) as usize;
                row.position_for_display_index(row.display.source_to_display(local))
                    .map(|position| {
                        (
                            self.scroll_y
                                + f32::from(row.origin_y - viewport.top())
                                + f32::from(position.y),
                            f32::from(row.line_height),
                        )
                    })
            });
        let (top, caret_height) = measured.unwrap_or_else(|| {
            (
                self.animated_line_start_y(line.0),
                self.animated_line_height_px(line.0)
                    .min(self.base_line_height()),
            )
        });
        let bottom = top + caret_height;
        let height =
            (f32::from(viewport.size.height) - self.display_map.bottom_overlay_clearance).max(1.0);
        let previous_y = self.scroll_y;
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if bottom > self.scroll_y + height {
            self.scroll_y = (bottom - height).max(0.0);
        }
        if (self.scroll_y - previous_y).abs() > 0.5 {
            self.minimap.note_viewport_changed();
        }
        if let Some(row) = self.hit_rows.iter().find(|row| {
            row.line == line
                && self.selection.head() >= row.range.start
                && self.selection.head() <= row.range.end
        }) && !self.display_map.soft_wrap()
        {
            let source_local =
                (self.selection.head().0 - row.range.start.0).min(row.range.len()) as usize;
            let display_local = row.display.source_to_display(source_local);
            if let Some(position) = row.position_for_display_index(display_local) {
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
        let line = self.animated_line_at_y(self.scroll_y);
        let line_start = self.animated_line_start_y(line);
        let line_height = self.animated_line_height_px(line).max(1.0);
        let fraction = ((self.scroll_y - line_start) / line_height).clamp(0.0, 1.0);
        let source = snapshot
            .line_content_range(LineIndex(line))
            .map_or(ByteOffset(0), |range| range.start);
        (source, fraction)
    }

    pub(crate) fn scroll_to_source_offset(
        &mut self,
        source: ByteOffset,
        cx: &mut Context<Self>,
    ) -> bool {
        self.finish_fold_animation();
        let snapshot = self.snapshot(cx);
        let Ok(line) = snapshot.line_index_at(source) else {
            return false;
        };
        let line = (0..=line.0)
            .rev()
            .find(|line| !self.display_map.is_hidden(*line))
            .unwrap_or(0);
        let viewport_height = self
            .viewport
            .map_or(0.0, |viewport| f32::from(viewport.size.height));
        let max_scroll = (self.display_map.total_height() - viewport_height).max(0.0);
        self.scroll_y = self.display_map.line_start_y(line).clamp(0.0, max_scroll);
        self.minimap.note_viewport_changed();
        cx.notify();
        true
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

fn fold_anchor_scroll_y(
    anchor_y: f32,
    anchor_height: f32,
    previous_viewport_y: f32,
    viewport_height: f32,
    document_height: f32,
) -> f32 {
    let max_anchor_viewport_y = (viewport_height - anchor_height.max(1.0)).max(0.0);
    let anchor_viewport_y = previous_viewport_y.clamp(0.0, max_anchor_viewport_y);
    let max_scroll = (document_height - viewport_height).max(0.0);
    (anchor_y - anchor_viewport_y).clamp(0.0, max_scroll)
}

fn horizontal_scroll_limit(content_width: f32, available_width: f32) -> f32 {
    const CARET_PADDING: f32 = 12.0;
    (content_width + CARET_PADDING - available_width).max(0.0)
}

impl Focusable for SemanticEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SemanticEditor {
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
        let timestamp_overlay = self.timestamp_overlay(cx);
        let todo_overlay = self.todo_overlay(window, cx);
        let inline_overlay = self.inline_overlay(window, cx);
        div()
            .id("semantic-editor")
            .key_context(if self.inline_actions.popup.is_some() {
                "InlinePicker"
            } else if self.todo.popup.is_some() {
                "TodoPicker"
            } else if self.timestamp.popup.is_some() {
                "TimestampPicker"
            } else {
                "SemanticEditor"
            })
            .on_action(cx.listener(
                |this, _: &crate::components::timestamp_picker::CancelTimestamp, _, cx| {
                    this.dismiss_timestamp(cx)
                },
            ))
            .on_action(cx.listener(
                |this, _: &crate::components::todo_picker::CancelTodo, _, cx| {
                    this.dismiss_todo(cx);
                    this.autofocus = true;
                },
            ))
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .bg(rgb(current_theme().background))
            .font_family(super::EDITOR_FONT_FAMILY)
            .font_weight(gpui::FontWeight::NORMAL)
            .text_size(px(self.font_size_px()))
            .line_height(px(self.base_line_height()))
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
            .on_action(cx.listener(Self::shift_tab))
            .on_action(cx.listener(Self::align_table))
            .on_action(cx.listener(Self::toggle_todo))
            .on_action(cx.listener(Self::toggle_checkbox))
            .on_action(cx.listener(Self::toggle_inline_image_previews))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::kill_line))
            .on_action(cx.listener(Self::kill_word))
            .on_action(cx.listener(Self::backward_kill_word))
            .on_action(cx.listener(Self::open_line))
            .on_action(cx.listener(Self::transpose_chars))
            .on_action(cx.listener(Self::set_mark))
            .on_action(cx.listener(Self::keyboard_quit))
            .on_action(cx.listener(Self::scroll_page_down))
            .on_action(cx.listener(Self::scroll_page_up))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if this.inline_key(event, cx) {
                    cx.stop_propagation();
                    return;
                }
                if this.todo_key(event, cx) {
                    cx.stop_propagation();
                    return;
                }
                if event.keystroke.key == "escape" && this.timestamp.popup.is_some() {
                    this.dismiss_timestamp(cx);
                    cx.stop_propagation();
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                if !hovered {
                    this.inline_hover(window.mouse_position(), cx);
                    this.todo_hover(window.mouse_position(), cx);
                }
                if !hovered && this.todo.popup.is_none() {
                    this.todo.hover_task = None;
                    this.todo.hover_range = None;
                }
                if !hovered && this.timestamp.popup.is_none() {
                    this.timestamp.hover_task = None;
                    if this.timestamp.hover_range.take().is_some() {
                        cx.notify();
                    }
                }
            }))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(move |event, _, cx| {
                let line_height = scroll_entity.read(cx).base_line_height();
                let delta = event.delta.pixel_delta(px(line_height));
                scroll_entity.update(cx, |this, cx| {
                    this.scroll(f32::from(delta.x), f32::from(delta.y), cx)
                });
                cx.stop_propagation();
            })
            .child(EditorElement::new(entity))
            .children(timestamp_overlay)
            .children(todo_overlay)
            .children(inline_overlay)
            .when_some(self.command_feedback.clone(), |editor, message| {
                editor.child(
                    div()
                        .absolute()
                        .left(px(12.0))
                        .bottom(px(10.0))
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(rgb(current_theme().background_alt))
                        .border_1()
                        .border_color(rgb(current_theme().border))
                        .text_color(rgb(current_theme().foreground_dim))
                        .child(message),
                )
            })
    }
}

pub(super) fn map_offset_through_edits(offset: ByteOffset, edits: &[TextEdit]) -> ByteOffset {
    let mut shift = 0_i128;
    for edit in edits {
        if offset <= edit.range.start {
            continue;
        }
        if offset < edit.range.end {
            return ByteOffset(
                (i128::from(edit.range.start.0) + shift + edit.replacement.len() as i128).max(0)
                    as u64,
            );
        }
        shift += edit.replacement.len() as i128 - i128::from(edit.range.len());
    }
    ByteOffset((i128::from(offset.0) + shift).max(0) as u64)
}

#[cfg(test)]
mod horizontal_scroll_tests {
    use super::*;

    #[test]
    fn horizontal_scroll_is_zero_until_content_exceeds_the_viewport() {
        assert_eq!(horizontal_scroll_limit(500.0, 600.0), 0.0);
        assert_eq!(horizontal_scroll_limit(700.0, 600.0), 112.0);
    }

    #[test]
    fn fold_anchor_stays_inside_the_viewport_when_the_document_shrinks() {
        let scroll = fold_anchor_scroll_y(900.0, 22.0, 190.0, 200.0, 930.0);
        assert_eq!(scroll, 722.0);
        assert_eq!(900.0 - scroll, 178.0);
    }

    #[gpui::test]
    fn emacs_keys_move_mark_kill_and_yank(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("emacs.org"),
                b"alpha beta\nsecond\n".to_vec(),
            )
            .unwrap()
        });
        let session_for_view = session.clone();
        let (editor, cx) =
            cx.add_window_view(move |_, cx| SemanticEditor::new(session_for_view.clone(), cx));
        cx.run_until_parked();

        editor.update(cx, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(2)), cx)
        });
        cx.simulate_keystrokes("ctrl-e");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.selection.head()),
            ByteOffset(10)
        );
        cx.simulate_keystrokes("ctrl-a alt-f");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.selection.head()),
            ByteOffset(5)
        );
        cx.simulate_keystrokes("alt-f");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.selection.head()),
            ByteOffset(10)
        );
        cx.simulate_keystrokes("ctrl-a escape f");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.selection.head()),
            ByteOffset(5)
        );

        cx.simulate_keystrokes("ctrl-a ctrl-space ctrl-f ctrl-f");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.selection.range()),
            ByteRange::new(0, 2)
        );
        cx.simulate_keystrokes("ctrl-w");
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "al");
        assert_eq!(
            session.read_with(cx, |session, _| {
                let snapshot = session.snapshot();
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
            }),
            "pha beta\nsecond\n"
        );
        cx.simulate_keystrokes("ctrl-y");
        assert_eq!(
            session.read_with(cx, |session, _| {
                let snapshot = session.snapshot();
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
            }),
            "alpha beta\nsecond\n"
        );

        editor.update(cx, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(0)), cx)
        });
        cx.simulate_keystrokes("ctrl-space ctrl-f ctrl-g");
        assert!(editor.read_with(cx, |editor, _| editor.selection.is_empty()));
        assert!(!editor.read_with(cx, |editor, _| editor.emacs_mark_active));
    }

    #[gpui::test]
    fn floating_toolbar_clearance_keeps_the_final_caret_visible(cx: &mut gpui::TestAppContext) {
        let source = "line\n".repeat(100);
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("floating.org"),
                source.into_bytes(),
            )
            .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session, cx));
        editor.update(cx, |editor, cx| {
            editor.set_bottom_overlay_clearance(66.0);
            editor.viewport = Some(Bounds::new(
                gpui::point(px(0.0), px(0.0)),
                gpui::size(px(800.0), px(220.0)),
            ));
            let snapshot = editor.snapshot(cx);
            editor.display_map.configure(snapshot.len_lines(), 740.0);
            editor.selection = Selection::caret(ByteOffset(snapshot.len_bytes()));
            editor.reveal_caret(&snapshot);
            let caret_bottom = editor.display_map.line_start_y(snapshot.len_lines());
            assert!(caret_bottom - editor.scroll_y <= 220.0 - 66.0 + 0.5);
            assert!(editor.scroll_y <= editor.display_map.total_height() - 220.0 + 0.5);
        });
    }

    #[gpui::test]
    fn page_commands_scroll_and_keep_the_caret_visible(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let source = (0..100)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();
        let session = cx.new(|_| {
            DocumentSession::from_utf8(std::path::PathBuf::from("pages.org"), source.into_bytes())
                .unwrap()
        });
        let (editor, cx) = cx.add_window_view(move |_, cx| SemanticEditor::new(session, cx));
        cx.run_until_parked();
        editor.update(cx, |editor, _| {
            editor.viewport = Some(Bounds::new(
                gpui::point(px(0.0), px(0.0)),
                gpui::size(px(800.0), px(220.0)),
            ));
        });

        cx.simulate_keystrokes("ctrl-v");
        let (scroll_y, caret_line) = editor.read_with(cx, |editor, cx| {
            let line = editor
                .snapshot(cx)
                .line_index_at(editor.selection.head())
                .unwrap();
            (editor.scroll_y, line.0)
        });
        assert!(scroll_y > 0.0);
        assert!(caret_line > 0);

        cx.simulate_keystrokes("alt-v");
        assert_eq!(editor.read_with(cx, |editor, _| editor.scroll_y), 0.0);

        cx.simulate_keystrokes("pagedown");
        assert!(editor.read_with(cx, |editor, _| editor.scroll_y) > 0.0);
        cx.simulate_keystrokes("pageup");
        assert_eq!(editor.read_with(cx, |editor, _| editor.scroll_y), 0.0);
    }

    #[gpui::test]
    fn emacs_editing_keys_kill_open_and_transpose(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::init);
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("emacs.org"),
                b"ab cd\nnext\n".to_vec(),
            )
            .unwrap()
        });
        let session_for_view = session.clone();
        let (editor, cx) =
            cx.add_window_view(move |_, cx| SemanticEditor::new(session_for_view.clone(), cx));
        cx.run_until_parked();

        editor.update(cx, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(1)), cx)
        });
        cx.simulate_keystrokes("ctrl-t");
        assert_eq!(
            session.read_with(cx, |session, _| {
                let snapshot = session.snapshot();
                snapshot.copy_range(snapshot.line_content_range(LineIndex(0)).unwrap())
            }),
            "ba cd"
        );

        editor.update(cx, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(2)), cx)
        });
        cx.simulate_keystrokes("ctrl-o");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.selection.head()),
            ByteOffset(2)
        );
        assert_eq!(
            session.read_with(cx, |session, _| {
                let snapshot = session.snapshot();
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
            }),
            "ba\n cd\nnext\n"
        );

        editor.update(cx, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(0)), cx)
        });
        cx.simulate_keystrokes("ctrl-k");
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "ba");
        assert_eq!(
            session.read_with(cx, |session, _| {
                let snapshot = session.snapshot();
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
            }),
            "\n cd\nnext\n"
        );
    }

    #[gpui::test]
    fn inline_image_preview_toggle_preserves_the_org_link_source(cx: &mut gpui::TestAppContext) {
        let source = "#+RESULTS:\n[[file:images/result.svg]]\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("test.org"),
                source.as_bytes().to_vec(),
            )
            .unwrap()
        });
        let window = cx.open_window(gpui::size(px(800.0), px(500.0)), |_, cx| {
            SemanticEditor::new(session, cx)
        });

        window
            .update(cx, |editor, window, cx| {
                let link_start = ByteOffset("#+RESULTS:\n".len() as u64);
                editor.set_selection(Selection::caret(link_start), cx);
                editor.toggle_inline_image_previews(&ToggleInlineImagePreviews, window, cx);
                assert_eq!(
                    editor.inline_image_preview_overrides.get(&link_start.0),
                    Some(&false)
                );
                assert_eq!(
                    editor
                        .snapshot(cx)
                        .copy_range(ByteRange::new(0, source.len() as u64)),
                    source
                );

                editor.toggle_inline_image_previews(&ToggleInlineImagePreviews, window, cx);
                assert_eq!(
                    editor.inline_image_preview_overrides.get(&link_start.0),
                    Some(&true)
                );

                editor.set_selection(Selection::caret(ByteOffset(0)), cx);
                editor.toggle_inline_image_previews(&ToggleInlineImagePreviews, window, cx);
                assert!(!editor.inline_image_previews);
                editor.set_selection(Selection::caret(link_start), cx);
                editor.toggle_inline_image_previews(&ToggleInlineImagePreviews, window, cx);
                assert_eq!(
                    editor.inline_image_preview_overrides.get(&link_start.0),
                    Some(&true)
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn fold_animation_uses_next_frame_callbacks_outside_paint(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("test.org"),
                b"* Heading\nbody one\nbody two\n* Next\n".to_vec(),
            )
            .unwrap()
        });
        let window = cx.open_window(gpui::size(px(800.0), px(500.0)), |_, cx| {
            SemanticEditor::new(session, cx)
        });
        cx.run_until_parked();

        window
            .update(cx, |editor, window, cx| {
                editor.animate_fold_layout(
                    super::super::folding::EditorFoldProjection::default(),
                    super::super::folding::EditorFoldProjection {
                        hidden_ranges: std::iter::once(1..3).collect(),
                        marker_lines: HashSet::from([0]),
                    },
                    0,
                    window,
                    cx,
                );
            })
            .unwrap();
        cx.run_until_parked();

        let root = window.entity(cx).unwrap();
        root.update(cx, |editor, _| {
            editor.fold_animation.as_mut().unwrap().started_at =
                Some(Instant::now() - FOLD_MOTION.duration() / 2);
        });
        let simulate_frame = |cx: &mut gpui::TestAppContext| {
            cx.update(|cx| {
                cx.with_window(root.entity_id(), |window, cx| {
                    window.simulate_next_frame(cx)
                })
                .unwrap()
            })
        };
        assert!(simulate_frame(cx) > 0);
        cx.run_until_parked();
        root.update(cx, |editor, _| {
            editor.fold_animation.as_mut().unwrap().started_at =
                Some(Instant::now() - FOLD_MOTION.duration());
        });
        assert!(simulate_frame(cx) > 0);
        cx.run_until_parked();
        assert!(simulate_frame(cx) > 0);
        cx.run_until_parked();

        cx.read(|cx| {
            let editor = root.read(cx);
            assert!(editor.fold_animation.is_none());
            assert!(editor.display_map.is_hidden(1));
            assert!(editor.display_map.is_hidden(2));
        });
    }

    #[gpui::test]
    fn markdown_tab_runs_the_real_heading_fold_action_and_starts_animation(
        cx: &mut gpui::TestAppContext,
    ) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("test.md"),
                b"# Heading\nbody\n## Child\nchild body\n# Next\n".to_vec(),
            )
            .unwrap()
        });
        let window = cx.open_window(gpui::size(px(800.0), px(500.0)), |_, cx| {
            SemanticEditor::new(session, cx)
        });
        cx.run_until_parked();

        window
            .update(cx, |editor, window, cx| {
                editor.insert_tab(&InsertTab, window, cx);
                let animation = editor
                    .fold_animation
                    .as_ref()
                    .expect("Markdown heading Tab starts a fold transition");
                assert_eq!(
                    animation.target_hidden_ranges.as_ref(),
                    std::slice::from_ref(&(1..4))
                );
                assert_eq!(animation.target_marker_lines.as_ref(), &HashSet::from([0]));
            })
            .unwrap();
    }

    #[gpui::test]
    fn tab_on_block_boundary_folds_but_tab_in_body_still_indents(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("test.org"),
                b"#+begin_src rust\nfn main() {}\n#+end_src\nafter\n".to_vec(),
            )
            .unwrap()
        });
        let window = cx.open_window(gpui::size(px(800.0), px(500.0)), |_, cx| {
            SemanticEditor::new(session, cx)
        });
        cx.run_until_parked();

        window
            .update(cx, |editor, window, cx| {
                editor.insert_tab(&InsertTab, window, cx);
                let animation = editor
                    .fold_animation
                    .as_ref()
                    .expect("source block Tab starts a fold transition");
                assert_eq!(
                    animation.target_hidden_ranges.as_ref(),
                    std::slice::from_ref(&(1..3))
                );
                editor.finish_fold_animation();
                editor.set_selection(Selection::caret(ByteOffset(17)), cx);
                editor.insert_tab(&InsertTab, window, cx);
                assert_eq!(editor.snapshot(cx).copy_range(ByteRange::new(17, 18)), "\t");
            })
            .unwrap();
    }

    #[gpui::test]
    fn table_tab_shift_tab_and_return_realign_and_navigate_cells(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("test.org"),
                b"| a| bb|\n|--+---|\n| ccc |d|\n".to_vec(),
            )
            .unwrap()
        });
        let window = cx.open_window(gpui::size(px(800.0), px(500.0)), |_, cx| {
            SemanticEditor::new(session, cx)
        });
        cx.run_until_parked();

        window
            .update(cx, |editor, window, cx| {
                let text_after_caret = |editor: &SemanticEditor, cx: &gpui::App| {
                    let snapshot = editor.snapshot(cx);
                    let caret = editor.selection().head();
                    let line = snapshot.line_index_at(caret).unwrap();
                    let range = snapshot.line_content_range(line).unwrap();
                    snapshot.copy_range(ByteRange::new(caret.0, range.end.0))
                };

                editor.set_selection(Selection::caret(ByteOffset(2)), cx);
                editor.insert_tab(&InsertTab, window, cx);
                assert!(text_after_caret(editor, cx).starts_with("bb"));

                editor.shift_tab(&ShiftTab, window, cx);
                assert!(text_after_caret(editor, cx).starts_with('a'));

                editor.newline(&Newline, window, cx);
                assert!(text_after_caret(editor, cx).starts_with("ccc"));
                assert_eq!(editor.snapshot(cx).revision().0, 1);
            })
            .unwrap();
    }

    #[gpui::test]
    fn navigating_an_aligned_table_does_not_create_edits(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("test.org"),
                b"| alpha | beta  |\n|-------+-------|\n| gamma | delta |\n".to_vec(),
            )
            .unwrap()
        });
        let window = cx.open_window(gpui::size(px(800.0), px(500.0)), |_, cx| {
            SemanticEditor::new(session, cx)
        });
        cx.run_until_parked();

        window
            .update(cx, |editor, window, cx| {
                editor.set_selection(Selection::caret(ByteOffset(3)), cx);
                assert!(editor.align_table_at_selection(cx));
                assert_eq!(editor.selection().head(), ByteOffset(3));
                assert_eq!(editor.snapshot(cx).revision().0, 0);
                assert!(!editor.session.read(cx).is_dirty());

                editor.insert_tab(&InsertTab, window, cx);
                let snapshot = editor.snapshot(cx);
                let caret = editor.selection().head();
                let line = snapshot.line_index_at(caret).unwrap();
                let range = snapshot.line_content_range(line).unwrap();
                assert!(
                    snapshot
                        .copy_range(ByteRange::new(caret.0, range.end.0))
                        .starts_with("beta")
                );
                assert_eq!(snapshot.revision().0, 0);
                assert!(!editor.session.read(cx).is_dirty());
            })
            .unwrap();
    }

    #[gpui::test]
    fn markdown_table_tab_realigns_and_navigates_without_losing_separator_markers(
        cx: &mut gpui::TestAppContext,
    ) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("test.md"),
                b"| a| value |\n| :- | -: |\n| longer | x |\n".to_vec(),
            )
            .unwrap()
        });
        let window = cx.open_window(gpui::size(px(800.0), px(500.0)), |_, cx| {
            SemanticEditor::new(session, cx)
        });
        cx.run_until_parked();

        window
            .update(cx, |editor, window, cx| {
                editor.set_selection(Selection::caret(ByteOffset(2)), cx);
                editor.insert_tab(&InsertTab, window, cx);
                let snapshot = editor.snapshot(cx);
                let source = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()));
                assert!(source.contains("| :----- | ----: |"), "{source}");
                let caret = editor.selection().head();
                let line = snapshot.line_index_at(caret).unwrap();
                let range = snapshot.line_content_range(line).unwrap();
                assert!(
                    snapshot
                        .copy_range(ByteRange::new(caret.0, range.end.0))
                        .starts_with("value")
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn tabbing_past_the_last_table_cell_appends_rows_without_crashing(
        cx: &mut gpui::TestAppContext,
    ) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("test.md"),
                b"| a | value |\n| :- | -: |\n| longer | x |\n".to_vec(),
            )
            .unwrap()
        });
        let window = cx.open_window(gpui::size(px(800.0), px(500.0)), |_, cx| {
            SemanticEditor::new(session, cx)
        });
        cx.run_until_parked();

        window
            .update(cx, |editor, window, cx| {
                editor.set_selection(Selection::caret(ByteOffset(2)), cx);
                for _ in 0..20 {
                    editor.insert_tab(&InsertTab, window, cx);
                    let snapshot = editor.snapshot(cx);
                    let caret = editor.selection().head();
                    let line = snapshot.line_index_at(caret).unwrap();
                    let range = snapshot.line_content_range(line).unwrap();
                    let text = snapshot.copy_range(range);
                    assert!(
                        text.starts_with('|'),
                        "caret landed outside the table on {text:?}"
                    );
                }
            })
            .unwrap();
    }

    #[gpui::test]
    fn org_context_command_aligns_at_a_table_and_declines_plain_text(
        cx: &mut gpui::TestAppContext,
    ) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("test.org"),
                b"| a|long |\n| wider|b|\n\nplain\n".to_vec(),
            )
            .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session, cx));

        editor.update(cx, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(2)), cx);
            assert!(editor.align_table_at_selection(cx));
            let snapshot = editor.snapshot(cx);
            assert_eq!(
                snapshot.copy_range(snapshot.line_content_range(LineIndex(0)).unwrap()),
                "| a     | long |"
            );

            let plain = snapshot.line_content_range(LineIndex(3)).unwrap().start;
            editor.set_selection(Selection::caret(plain), cx);
            assert!(!editor.align_table_at_selection(cx));
        });
    }

    #[gpui::test]
    fn shift_tab_on_headingless_org_is_a_stable_no_op(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("plain.org"),
                b"plain\ncontent\n".to_vec(),
            )
            .unwrap()
        });
        let window = cx.open_window(gpui::size(px(800.0), px(500.0)), |_, cx| {
            SemanticEditor::new(session, cx)
        });
        cx.run_until_parked();

        window
            .update(cx, |editor, window, cx| {
                editor.shift_tab(&ShiftTab, window, cx);
                assert_eq!(
                    editor.folds.global,
                    super::super::folding::GlobalVisibility::All
                );
                assert!(editor.fold_animation.is_none());
                assert_eq!(editor.display_map.visible_line_count(), 3);
            })
            .unwrap();
    }

    #[gpui::test]
    fn shift_tab_keeps_a_full_animation_for_large_documents(cx: &mut gpui::TestAppContext) {
        let mut source = String::from("* Heading\n");
        for line in 0..600 {
            source.push_str(&format!("body {line}\n"));
        }
        source.push_str("* Next\n");
        let session = cx.new(|_| {
            DocumentSession::from_utf8(std::path::PathBuf::from("large.org"), source.into_bytes())
                .unwrap()
        });
        let window = cx.open_window(gpui::size(px(800.0), px(500.0)), |_, cx| {
            SemanticEditor::new(session, cx)
        });
        cx.run_until_parked();

        window
            .update(cx, |editor, window, cx| {
                editor.shift_tab(&ShiftTab, window, cx);
                let animation = editor
                    .fold_animation
                    .as_ref()
                    .expect("large global folds retain the animated transition");
                assert!(animation.changed_line_count() > 256);
                assert!(animation.collapsing);
            })
            .unwrap();
    }

    #[gpui::test]
    fn expanding_fold_animates_document_extent_and_commits_without_an_end_jump(
        cx: &mut gpui::TestAppContext,
    ) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                std::path::PathBuf::from("test.org"),
                b"* Heading\nbody one\nbody two\n* Next\n".to_vec(),
            )
            .unwrap()
        });
        let window = cx.open_window(gpui::size(px(800.0), px(500.0)), |_, cx| {
            SemanticEditor::new(session, cx)
        });
        cx.run_until_parked();

        window
            .update(cx, |editor, window, cx| {
                let previous = super::super::folding::EditorFoldProjection {
                    hidden_ranges: std::iter::once(1..3).collect(),
                    marker_lines: HashSet::from([0]),
                };
                editor
                    .display_map
                    .set_hidden_ranges(previous.hidden_ranges.clone());
                editor.fold_markers = Arc::new(previous.marker_lines.clone());
                let collapsed_height = editor.display_map.total_height();
                let collapsed_lines = editor.display_map.visible_line_count() as f32;

                editor.animate_fold_layout(
                    previous,
                    super::super::folding::EditorFoldProjection::default(),
                    0,
                    window,
                    cx,
                );
                assert!((editor.animated_document_height() - collapsed_height).abs() < 0.01);
                assert!((editor.animated_visible_line_count() - collapsed_lines).abs() < 0.01);

                editor.fold_animation.as_mut().unwrap().progress = 0.5;
                let middle_height = editor.animated_document_height();
                assert!(middle_height > collapsed_height);
                assert!(middle_height < editor.display_map.total_height());

                editor.fold_animation.as_mut().unwrap().progress = 1.0;
                let next_heading_y = editor.animated_line_start_y(3);
                let final_height = editor.animated_document_height();
                let scroll_y = editor.scroll_y;
                editor.complete_fold_animation();

                assert!((editor.display_map.line_start_y(3) - next_heading_y).abs() < 0.01);
                assert!((editor.display_map.total_height() - final_height).abs() < 0.01);
                assert!((editor.scroll_y - scroll_y).abs() < 0.01);
            })
            .unwrap();
    }
}
