//! Source hit testing and viewport-constrained popup placement.
use super::*;

impl SemanticEditor {
    pub(super) fn inline_hit(&self, position: Point<Pixels>, cx: &App) -> Option<Hit> {
        if self.viewport.is_none_or(|v| !v.contains(&position)) {
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
        let path = self.session.read(cx).syntax_path();
        if DocumentFormat::from_path(path) != DocumentFormat::Org
            || crate::syntax_highlighting::language_for_path(path).is_some()
            || self.is_read_only(cx)
        {
            return None;
        }
        let snapshot = self.snapshot(cx);
        let line_range = snapshot.line_content_range(row.line).ok()?;
        if line_range.len() > 64 * 1024 {
            return None;
        }
        let text = snapshot.copy_range(line_range);
        let offset = self.hit_test(position).0.saturating_sub(line_range.start.0) as usize;
        let query = syntax::SparseEditorStyleSnapshot::query_lines(
            path,
            &snapshot,
            &[row.line.0],
            &self.syntax_service,
        );
        let style = query.snapshot.line(row.line.0)?.id;
        let (token, kind) = match style {
            syntax::EditorStyleId::List => crate::org_syntax::command::checkbox_token(&text)
                .map(|(range, _)| (range, Kind::Checkbox)),
            syntax::EditorStyleId::Heading(_) => {
                let tags = syntax::org_tag_ranges(&text);
                if let (Some(first), Some(last)) = (tags.first(), tags.last())
                    && offset >= first.start
                    && offset < last.end
                {
                    Some((first.start..last.end, Kind::Tags))
                } else {
                    priority_token(&text).map(|range| (range, Kind::Priority))
                }
            }
            _ => None,
        }?;
        if !token.contains(&offset) {
            return None;
        }
        let start = row.position_for_display_index(row.display.source_to_display(
            token.start + line_range.start.0.saturating_sub(row.range.start.0) as usize,
        ))?;
        let end = row.position_for_display_index(row.display.source_to_display(
            token.end + line_range.start.0.saturating_sub(row.range.start.0) as usize,
        ))?;
        // Anchor a wrapped token to the visual segment under the pointer.
        let y = if start.y == end.y {
            row.origin_y + start.y
        } else {
            row.origin_y
                + px((f32::from(position.y - row.origin_y) / f32::from(row.line_height)).floor())
                    * f32::from(row.line_height)
        };
        let x = if start.y == end.y {
            row.text_origin_x + start.x
        } else {
            position.x
        };
        Some(Hit {
            range: ByteRange::new(
                line_range.start.0 + token.start as u64,
                line_range.start.0 + token.end as u64,
            ),
            source: text.get(token)?.to_owned(),
            kind,
            bounds: Bounds::new(
                gpui::point(x, y),
                gpui::size((end.x - start.x).max(px(10.)), row.line_height),
            ),
        })
    }
    pub(in crate::editor) fn inline_overlay(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let viewport = self.viewport?;
        let popup = self.inline_actions.popup.as_mut()?;
        let (width, height) = popup.picker.read(cx).preferred_size(window);
        let width = width.min(f32::from(viewport.size.width) - 16.).max(40.);
        let token = popup.hit.bounds;
        let below = f32::from(viewport.bottom() - token.bottom()) - 10.;
        let above = f32::from(token.top() - viewport.top()) - 10.;
        let show_below = height <= below || below >= above;
        let x = token
            .left()
            .min(viewport.right() - px(width + 8.))
            .max(viewport.left() + px(8.));
        let height = height.min(if show_below { below } else { above }).max(38.);
        let y = if show_below {
            token.bottom()
        } else {
            (token.top() - px(height + 6.)).max(viewport.top() + px(4.))
        };
        let bounds = Bounds::new(gpui::point(x, y), gpui::size(px(width), px(height + 6.)));
        popup.bounds = Some(bounds);
        popup.picker.update(cx, |p, _| {
            p.width = width;
            p.height = height;
        });
        Some(
            gpui::deferred(
                gpui::anchored()
                    .position(bounds.origin)
                    .snap_to_window_with_margin(px(4.))
                    .child(
                        div()
                            .id("editor-inline-popup")
                            .occlude()
                            .py(px(3.))
                            .on_mouse_down_out(
                                cx.listener(|this, _, _, cx| this.dismiss_inline(cx)),
                            )
                            .on_mouse_move(cx.listener(|this, _, _, cx| {
                                this.inline_actions.closing = None;
                                cx.stop_propagation();
                            }))
                            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                                if *hovered {
                                    this.inline_actions.closing = None;
                                } else {
                                    this.inline_hover(window.mouse_position(), cx);
                                }
                            }))
                            .child(popup.picker.clone()),
                    ),
            )
            .with_priority(10)
            .into_any_element(),
        )
    }
}
