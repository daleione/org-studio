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
        let snapshot = self.snapshot(cx);
        let line_range = snapshot.line_content_range(row.line).ok()?;
        if line_range.len() > 64 * 1024 {
            return None;
        }
        let text = snapshot.copy_range(line_range);
        let offset = self.hit_test(position).0.saturating_sub(line_range.start.0) as usize;
        let query = syntax::SparseEditorStyleSnapshot::query_lines(
            self.session.read(cx).syntax_path(),
            &snapshot,
            &[row.line.0],
            &self.syntax_service,
        );
        let style = query.snapshot.line(row.line.0)?.id;
        if matches!(
            style,
            syntax::EditorStyleId::Code
                | syntax::EditorStyleId::CodeBoundary
                | syntax::EditorStyleId::Comment
        ) {
            return None;
        }
        let org =
            DocumentFormat::from_path(self.session.read(cx).syntax_path()) == DocumentFormat::Org;
        let mut token = None;
        if org && !self.is_read_only(cx) {
            if style == syntax::EditorStyleId::List {
                token = crate::org_syntax::command::checkbox_token(&text)
                    .map(|(r, _)| (r, Kind::Checkbox));
            } else if matches!(style, syntax::EditorStyleId::Heading(_)) {
                let tags = syntax::org_tag_ranges(&text);
                if let (Some(first), Some(last)) = (tags.first(), tags.last())
                    && offset >= first.start
                    && offset < last.end
                {
                    token = Some((first.start..last.end, Kind::Tags));
                } else {
                    token = priority_token(&text).map(|r| (r, Kind::Priority));
                }
            }
        }
        if token.as_ref().is_none_or(|(r, _)| !r.contains(&offset)) {
            token = self
                .link_at_position(position)
                .and_then(|i| self.link_hits.get(i))
                .map(|link| {
                    let start = row.range.start.0
                        + row.display.display_to_source(link.display_range.start) as u64;
                    let end = row.range.start.0
                        + row.display.display_to_source(link.display_range.end) as u64;
                    (
                        (start - line_range.start.0) as usize..(end - line_range.start.0) as usize,
                        Kind::Link(link.clone()),
                    )
                });
        }
        let (token, kind) = token?;
        if !matches!(kind, Kind::Link(_)) && !token.contains(&offset) {
            return None;
        }
        let start = row.position_for_display_index(row.display.source_to_display(
            token.start + line_range.start.0.saturating_sub(row.range.start.0) as usize,
        ))?;
        let end = row.position_for_display_index(row.display.source_to_display(
            token.end + line_range.start.0.saturating_sub(row.range.start.0) as usize,
        ))?;
        // Anchor a wrapped link to the visual segment actually under the pointer.
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
            pointer: position,
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
        let mut show_below = height <= below || below >= above;
        let mut x = token
            .left()
            .min(viewport.right() - px(width + 8.))
            .max(viewport.left() + px(8.));
        if matches!(popup.hit.kind, Kind::Link(_)) {
            let right = popup.hit.pointer.x + px(18.);
            let left = popup.hit.pointer.x - px(width + 18.);
            if right + px(width + 8.) <= viewport.right() {
                x = right;
            } else if left >= viewport.left() + px(8.) {
                x = left;
            } else if above >= 38. {
                show_below = false;
            }
        }
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
