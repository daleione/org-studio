use std::ops::Range;

use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, Element, ElementId, GlobalElementId, Hitbox,
    HitboxBehavior, InspectorElementId, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, StyledText, Window, fill, point, px, rgba,
};

use crate::preview::ReadingPreviewPanel;

type ClickListener = Box<dyn Fn(usize, &mut Window, &mut App)>;

/// A `StyledText` wrapper that adds browser-like drag selection without changing the text's
/// semantic styling. Selection belongs to the reading panel, so it survives virtual-list row
/// recycling and can span multiple rendered rows.
pub(in crate::preview) struct SelectableReadingText {
    id: ElementId,
    text: StyledText,
    panel: gpui::Entity<ReadingPreviewPanel>,
    row: usize,
    text_offset: usize,
    restrict_drag_to_bounds: bool,
    selection: Option<(Range<usize>, bool)>,
    clickable_ranges: Vec<Range<usize>>,
    click_listener: Option<ClickListener>,
}

impl SelectableReadingText {
    pub(in crate::preview) fn new(
        id: impl Into<ElementId>,
        text: StyledText,
        panel: gpui::Entity<ReadingPreviewPanel>,
        row: usize,
        selection: Option<(Range<usize>, bool)>,
    ) -> Self {
        Self {
            id: id.into(),
            text,
            panel,
            row,
            text_offset: 0,
            restrict_drag_to_bounds: false,
            selection,
            clickable_ranges: Vec::new(),
            click_listener: None,
        }
    }

    pub(in crate::preview) fn with_text_offset(mut self, text_offset: usize) -> Self {
        self.text_offset = text_offset;
        self
    }

    pub(in crate::preview) fn restrict_drag_to_bounds(mut self) -> Self {
        self.restrict_drag_to_bounds = true;
        self
    }

    pub(super) fn on_click(
        mut self,
        ranges: Vec<Range<usize>>,
        listener: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.clickable_ranges = ranges;
        self.click_listener = Some(Box::new(listener));
        self
    }
}

impl Element for SelectableReadingText {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        self.text.request_layout(None, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.text
            .prepaint(None, inspector_id, bounds, state, window, cx);
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_state: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.text.layout().clone();
        if let Some((range, include_newline)) = self.selection.as_ref() {
            paint_selection(&layout, bounds, range.clone(), *include_newline, window);
        }

        let hovered_index = layout.index_for_position(window.mouse_position()).ok();
        if hitbox.is_hovered(window)
            && hovered_index.is_some_and(|index| {
                self.clickable_ranges
                    .iter()
                    .any(|range| range.contains(&index))
            })
        {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
        } else {
            window.set_cursor_style(CursorStyle::IBeam, hitbox);
        }

        let down_panel = self.panel.clone();
        let down_layout = layout.clone();
        let down_hitbox = hitbox.clone();
        let row = self.row;
        let text_offset = self.text_offset;
        let restrict_drag_to_bounds = self.restrict_drag_to_bounds;
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble
                && event.button == MouseButton::Left
                && down_hitbox.is_hovered(window)
            {
                let offset = text_offset + nearest_index(&down_layout, event.position);
                down_panel.update(cx, |panel, cx| {
                    panel.begin_text_selection(row, offset, event.modifiers.shift);
                    cx.notify();
                });
                window.prevent_default();
            }
        });

        let move_panel = self.panel.clone();
        let move_layout = layout.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble
                || !event.dragging()
                || event.position.y < bounds.top()
                || event.position.y > bounds.bottom()
                || (restrict_drag_to_bounds && !bounds.contains(&event.position))
                || !move_panel.read(cx).text_selection_pending()
            {
                return;
            }
            let offset = text_offset + nearest_index(&move_layout, event.position);
            move_panel.update(cx, |panel, cx| {
                panel.update_text_selection(row, offset);
                cx.notify();
            });
        });

        let up_panel = self.panel.clone();
        let up_layout = layout.clone();
        let up_hitbox = hitbox.clone();
        let ranges = std::mem::take(&mut self.clickable_ranges);
        let listener = self.click_listener.take();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || !up_panel.read(cx).text_selection_pending()
            {
                return;
            }
            let within_row = event.position.y >= bounds.top()
                && event.position.y <= bounds.bottom()
                && (!restrict_drag_to_bounds || bounds.contains(&event.position));
            if !within_row {
                return;
            }
            let offset = text_offset + nearest_index(&up_layout, event.position);
            let suppress_click = up_panel.update(cx, |panel, cx| {
                panel.update_text_selection(row, offset);
                panel.finish_text_selection();
                let suppress = panel.text_selection_suppresses_click();
                cx.notify();
                suppress
            });
            if suppress_click || !up_hitbox.is_hovered(window) {
                return;
            }
            let local_offset = offset.saturating_sub(text_offset);
            if let Some(index) = ranges
                .iter()
                .position(|range| range.contains(&local_offset))
                && let Some(listener) = listener.as_ref()
            {
                cx.stop_propagation();
                listener(index, window, cx);
            }
        });

        self.text
            .paint(None, inspector_id, bounds, &mut (), &mut (), window, cx);
    }
}

impl IntoElement for SelectableReadingText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

fn nearest_index(layout: &gpui::TextLayout, position: gpui::Point<Pixels>) -> usize {
    match layout.index_for_position(position) {
        Ok(index) | Err(index) => index.min(layout.len()),
    }
}

fn paint_selection(
    layout: &gpui::TextLayout,
    bounds: Bounds<Pixels>,
    range: Range<usize>,
    include_newline: bool,
    window: &mut Window,
) {
    if range.start >= range.end || range.end > layout.len() {
        return;
    }
    let Some(start) = layout.position_for_index(range.start) else {
        return;
    };
    let Some(end) = layout.position_for_index(range.end) else {
        return;
    };
    let line_height = f32::from(layout.line_height()).max(1.0);
    let first_line = ((f32::from(start.y - bounds.top())) / line_height)
        .round()
        .max(0.0) as usize;
    let last_line = ((f32::from(end.y - bounds.top())) / line_height)
        .round()
        .max(0.0) as usize;
    for line in first_line..=last_line {
        let left = if line == first_line {
            start.x
        } else {
            bounds.left()
        };
        let mut right = if line == last_line {
            end.x
        } else {
            bounds.right()
        };
        if include_newline && line == last_line {
            right = (right + px(8.0)).min(bounds.right() + px(8.0));
        }
        if right <= left {
            continue;
        }
        let top = bounds.top() + px(line as f32 * line_height);
        window.paint_quad(fill(
            Bounds::from_corners(point(left, top), point(right, top + px(line_height))),
            rgba(0x3a81c34a),
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, path::PathBuf, rc::Rc, sync::Arc};

    use gpui::{AppContext, IntoElement, Modifiers, MouseButton, div, point, prelude::*, px, size};

    use crate::{document::DocumentSnapshot, preview::ReadingPreviewPanel};

    use super::SelectableReadingText;

    #[gpui::test]
    fn drag_selects_rendered_text(cx: &mut gpui::TestAppContext) {
        let preview = crate::preview::loading::derive_preview(
            PathBuf::from("selection.md"),
            DocumentSnapshot::from_utf8(b"hello world\n".to_vec()).unwrap(),
        );
        let panel = cx.new(|_| ReadingPreviewPanel::new(Arc::new(preview), 80.0));
        let render_panel = panel.clone();
        let cx = cx.add_empty_window();
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(200.0), px(40.0)),
            move |_, _| {
                SelectableReadingText::new(
                    ("selection-test", 0usize),
                    gpui::StyledText::new("hello world"),
                    render_panel.clone(),
                    0,
                    None,
                )
                .into_any_element()
            },
        );
        cx.simulate_mouse_down(
            point(px(1.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(px(42.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            point(px(42.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );

        assert!(cx.read(|cx| panel.read(cx).selected_text().is_some()));
    }

    #[gpui::test]
    fn a_later_text_row_receives_its_own_mouse_up(cx: &mut gpui::TestAppContext) {
        let preview = crate::preview::loading::derive_preview(
            PathBuf::from("selection.md"),
            DocumentSnapshot::from_utf8(b"first\nsecond\n".to_vec()).unwrap(),
        );
        let panel = cx.new(|_| ReadingPreviewPanel::new(Arc::new(preview), 80.0));
        let render_panel = panel.clone();
        let clicked = Rc::new(Cell::new(false));
        let render_clicked = clicked.clone();
        let cx = cx.add_empty_window();
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(200.0), px(60.0)),
            move |_, _| {
                div()
                    .flex()
                    .flex_col()
                    .text_size(px(14.0))
                    .line_height(px(20.0))
                    .child(SelectableReadingText::new(
                        ("selection-test", 0usize),
                        gpui::StyledText::new("first"),
                        render_panel.clone(),
                        0,
                        None,
                    ))
                    .child(
                        SelectableReadingText::new(
                            ("selection-test", 1usize),
                            gpui::StyledText::new("second"),
                            render_panel.clone(),
                            1,
                            None,
                        )
                        .on_click(std::iter::once(0..6).collect(), {
                            let clicked = render_clicked.clone();
                            move |_, _, _| clicked.set(true)
                        }),
                    )
                    .into_any_element()
            },
        );
        cx.simulate_mouse_down(
            point(px(8.0), px(30.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            point(px(8.0), px(30.0)),
            MouseButton::Left,
            Modifiers::default(),
        );

        assert!(clicked.get());
    }
}
