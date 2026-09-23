//! TODO hit geometry and quick bar placement.
use super::*;

pub(super) fn quick_bar_bounds(
    token: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
    width: f32,
) -> Bounds<Pixels> {
    let x = token
        .left()
        .min(viewport.right() - px(width + 8.))
        .max(viewport.left() + px(8.));
    let below = token.bottom() + px(3.);
    let above = token.top() - px(QUICK_BAR_HEIGHT + 3.);
    let y = if below + px(QUICK_BAR_HEIGHT + 3.) <= viewport.bottom() || above < viewport.top() {
        below
    } else {
        above
    };
    Bounds::new(
        gpui::point(x, y),
        gpui::size(px(width), px(QUICK_BAR_HEIGHT)),
    )
}

impl SemanticEditor {
    pub(super) fn todo_hit(&self, position: Point<Pixels>, cx: &App) -> Option<TodoHit> {
        if self
            .viewport
            .is_none_or(|bounds| !bounds.contains(&position))
            || self.is_read_only(cx)
            || DocumentFormat::from_path(self.session.read(cx).syntax_path()) != DocumentFormat::Org
            || crate::syntax_highlighting::language_for_path(self.session.read(cx).syntax_path())
                .is_some()
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
        if let Some((revision, config)) = &self.todo.config
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
    pub(in crate::editor) fn todo_overlay(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let viewport = self.viewport?;
        let popup = self.todo.popup.as_ref()?;
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
        if let Some((_, config)) = &self.todo.config {
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
        let popup = self.todo.popup.as_mut()?;
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
            gpui::size(px(width), px(QUICK_BAR_HEIGHT + 6.) + extra),
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
                                    this.todo.dismiss_task = None;
                                } else {
                                    this.todo_hover(window.mouse_position(), cx);
                                }
                            }))
                            .on_mouse_move(cx.listener(|this, _, _, cx| {
                                this.todo.dismiss_task = None;
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
