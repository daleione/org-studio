use std::{cell::Cell, ops::Range, rc::Rc};

use gpui::{
    AnyElement, App, Bounds, CursorStyle, DispatchPhase, Element, ElementId, GlobalElementId,
    Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Style, StyledText, Window, fill, point, px, relative,
    rgba,
};

use crate::preview::ReadingPreviewPanel;

type ClickListener = Box<dyn Fn(usize, &mut Window, &mut App)>;

#[derive(Clone, Default)]
pub(in crate::preview) struct ReadingRowBounds(Rc<Cell<Option<LayoutId>>>);

impl ReadingRowBounds {
    fn get(&self, window: &mut Window) -> Option<Bounds<Pixels>> {
        self.0
            .get()
            .map(|layout_id| window.layout_bounds(layout_id))
    }
}

pub(in crate::preview) struct ReadingRowScope {
    bounds: ReadingRowBounds,
    content: AnyElement,
}

impl ReadingRowScope {
    pub(in crate::preview) fn new(bounds: ReadingRowBounds, content: impl IntoElement) -> Self {
        Self {
            bounds,
            content: content.into_any_element(),
        }
    }
}

impl Element for ReadingRowScope {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let content_layout_id = self.content.request_layout(window, cx);
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        let layout_id = window.request_layout(style, [content_layout_id], cx);
        self.bounds.0.set(Some(layout_id));
        (layout_id, content_layout_id)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.content.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_state: &mut Self::RequestLayoutState,
        _prepaint_state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.content.paint(window, cx);
    }
}

impl IntoElement for ReadingRowScope {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// A `StyledText` wrapper that adds browser-like drag selection without changing the text's
/// semantic styling. Selection belongs to the reading panel, so it survives virtual-list row
/// recycling and can span multiple rendered rows.
pub(in crate::preview) struct SelectableReadingText {
    id: ElementId,
    text: StyledText,
    panel: gpui::Entity<ReadingPreviewPanel>,
    row: usize,
    text_offset: usize,
    row_text_len: Option<usize>,
    restrict_drag_to_bounds: bool,
    row_bounds: Option<ReadingRowBounds>,
    minimum_height: Option<f32>,
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
            row_text_len: None,
            restrict_drag_to_bounds: false,
            row_bounds: None,
            minimum_height: None,
            selection,
            clickable_ranges: Vec::new(),
            click_listener: None,
        }
    }

    pub(in crate::preview) fn with_text_offset(mut self, text_offset: usize) -> Self {
        self.text_offset = text_offset;
        self
    }

    pub(in crate::preview) fn with_row_text_len(mut self, text_len: usize) -> Self {
        self.row_text_len = Some(text_len.max(self.text_offset));
        self
    }

    pub(in crate::preview) fn restrict_drag_to_bounds(mut self) -> Self {
        self.restrict_drag_to_bounds = true;
        self
    }

    pub(in crate::preview) fn with_row_bounds(mut self, bounds: Option<ReadingRowBounds>) -> Self {
        self.row_bounds = bounds;
        self
    }

    pub(in crate::preview) fn with_minimum_height(mut self, height: f32) -> Self {
        self.minimum_height = Some(height.max(1.0));
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

pub(in crate::preview) struct SelectableLayoutState {
    text_layout_id: LayoutId,
}

pub(in crate::preview) struct SelectablePrepaintState {
    hitbox: Hitbox,
    text_bounds: Bounds<Pixels>,
}

impl Element for SelectableReadingText {
    type RequestLayoutState = SelectableLayoutState;
    type PrepaintState = SelectablePrepaintState;

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
        let (text_layout_id, ()) = self.text.request_layout(None, inspector_id, window, cx);
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let inherited_line_height = text_style
            .line_height
            .to_pixels(font_size.into(), window.rem_size());
        style.min_size.height = self
            .minimum_height
            .map(px)
            .unwrap_or(inherited_line_height)
            .into();
        let layout_id = window.request_layout(style, [text_layout_id], cx);
        (layout_id, SelectableLayoutState { text_layout_id })
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
        let text_bounds = window.layout_bounds(state.text_layout_id);
        self.text
            .prepaint(None, inspector_id, text_bounds, &mut (), window, cx);
        let hit_bounds = self
            .row_bounds
            .as_ref()
            .and_then(|row_bounds| row_bounds.get(window))
            .unwrap_or(bounds);
        SelectablePrepaintState {
            hitbox: window.insert_hitbox(hit_bounds, HitboxBehavior::Normal),
            text_bounds,
        }
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_state: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.text.layout().clone();
        let hitbox = &state.hitbox;
        let text_bounds = state.text_bounds;

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
        let row_text_len = self.row_text_len;
        let restrict_drag_to_bounds = self.restrict_drag_to_bounds;
        let interaction_bounds = hitbox.bounds;
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble
                && event.button == MouseButton::Left
                && down_hitbox.is_hovered(window)
            {
                let offset =
                    nearest_document_index(&down_layout, event.position, text_offset, row_text_len);
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
                || event.position.y < interaction_bounds.top()
                || event.position.y > interaction_bounds.bottom()
                || (restrict_drag_to_bounds && !bounds.contains(&event.position))
                || !move_panel.read(cx).text_selection_pending()
            {
                return;
            }
            let offset =
                nearest_document_index(&move_layout, event.position, text_offset, row_text_len);
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
            let within_row = event.position.y >= interaction_bounds.top()
                && event.position.y <= interaction_bounds.bottom()
                && (!restrict_drag_to_bounds || bounds.contains(&event.position));
            if !within_row {
                return;
            }
            let offset =
                nearest_document_index(&up_layout, event.position, text_offset, row_text_len);
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

        self.text.paint(
            None,
            inspector_id,
            text_bounds,
            &mut (),
            &mut (),
            window,
            cx,
        );
        // StyledText paints inline-code and other semantic backgrounds as part of its own paint
        // pass. Paint the translucent selection afterwards so those backgrounds cannot hide it.
        if let Some((range, include_newline)) = self.selection.as_ref() {
            let selection_bounds = if range.is_empty() {
                bounds
            } else {
                text_bounds
            };
            paint_selection(
                &layout,
                selection_bounds,
                range.clone(),
                *include_newline,
                window,
            );
        }
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

fn nearest_document_index(
    layout: &gpui::TextLayout,
    position: gpui::Point<Pixels>,
    text_offset: usize,
    row_text_len: Option<usize>,
) -> usize {
    if text_offset > 0 {
        let bounds = layout.bounds();
        if position.x < bounds.left() && position.y <= bounds.top() + layout.line_height() {
            return 0;
        }
    }
    let offset = text_offset + nearest_index(layout, position);
    row_text_len.map_or(offset, |text_len| offset.min(text_len))
}

fn paint_selection(
    layout: &gpui::TextLayout,
    bounds: Bounds<Pixels>,
    range: Range<usize>,
    include_newline: bool,
    window: &mut Window,
) {
    if range.start > range.end || range.end > layout.len() {
        return;
    }
    if range.is_empty() {
        if !include_newline {
            return;
        }
        let position = layout
            .position_for_index(range.start)
            .unwrap_or(bounds.origin);
        let height = layout.line_height().min(bounds.size.height).max(px(1.0));
        window.paint_quad(fill(
            Bounds::from_corners(position, point(position.x + px(8.0), position.y + height)),
            rgba(0x3a81c34a),
        ));
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

    use super::{ReadingRowBounds, ReadingRowScope, SelectableReadingText};

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
    fn drag_can_start_in_the_rows_right_hand_whitespace(cx: &mut gpui::TestAppContext) {
        let preview = crate::preview::loading::derive_preview(
            PathBuf::from("selection.md"),
            DocumentSnapshot::from_utf8(b"hello world\n".to_vec()).unwrap(),
        );
        let panel = cx.new(|_| ReadingPreviewPanel::new(Arc::new(preview), 80.0));
        let render_panel = panel.clone();
        let cx = cx.add_empty_window();
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(240.0), px(40.0)),
            move |_, _| {
                let row_bounds = ReadingRowBounds::default();
                ReadingRowScope::new(
                    row_bounds.clone(),
                    div().w_full().flex().justify_center().child(
                        div().w(px(100.0)).child(
                            SelectableReadingText::new(
                                ("selection-whitespace-test", 0usize),
                                gpui::StyledText::new("hello world"),
                                render_panel.clone(),
                                0,
                                None,
                            )
                            .with_row_bounds(Some(row_bounds)),
                        ),
                    ),
                )
            },
        );
        cx.simulate_mouse_down(
            point(px(236.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(px(8.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            point(px(8.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );

        assert!(cx.read(|cx| panel.read(cx).selected_text().is_some()));
    }

    #[gpui::test]
    fn drag_can_start_in_the_rows_left_hand_whitespace(cx: &mut gpui::TestAppContext) {
        let preview = crate::preview::loading::derive_preview(
            PathBuf::from("selection.md"),
            DocumentSnapshot::from_utf8(b"hello world\n".to_vec()).unwrap(),
        );
        let panel = cx.new(|_| ReadingPreviewPanel::new(Arc::new(preview), 80.0));
        let render_panel = panel.clone();
        let cx = cx.add_empty_window();
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(240.0), px(40.0)),
            move |_, _| {
                let row_bounds = ReadingRowBounds::default();
                ReadingRowScope::new(
                    row_bounds.clone(),
                    div().w_full().flex().justify_center().child(
                        div().w(px(100.0)).child(
                            SelectableReadingText::new(
                                ("selection-left-whitespace-test", 0usize),
                                gpui::StyledText::new("hello world"),
                                render_panel.clone(),
                                0,
                                None,
                            )
                            .with_row_bounds(Some(row_bounds)),
                        ),
                    ),
                )
            },
        );
        cx.simulate_mouse_down(
            point(px(4.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(px(125.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            point(px(125.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );

        assert!(cx.read(|cx| panel.read(cx).selected_text().is_some()));
    }

    #[gpui::test]
    fn indented_text_uses_the_actual_row_bounds_for_selection(cx: &mut gpui::TestAppContext) {
        let preview = crate::preview::loading::derive_preview(
            PathBuf::from("selection.md"),
            DocumentSnapshot::from_utf8(b"hello world\n".to_vec()).unwrap(),
        );
        let panel = cx.new(|_| ReadingPreviewPanel::new(Arc::new(preview), 80.0));
        let render_panel = panel.clone();
        let cx = cx.add_empty_window();
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(240.0), px(40.0)),
            move |_, _| {
                let row_bounds = ReadingRowBounds::default();
                ReadingRowScope::new(
                    row_bounds.clone(),
                    div().w_full().pl(px(100.0)).child(
                        div().w(px(100.0)).child(
                            SelectableReadingText::new(
                                ("selection-indented-whitespace-test", 0usize),
                                gpui::StyledText::new("hello world"),
                                render_panel.clone(),
                                0,
                                None,
                            )
                            .with_row_bounds(Some(row_bounds)),
                        ),
                    ),
                )
            },
        );
        cx.simulate_mouse_down(
            point(px(4.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(px(150.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            point(px(150.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );

        assert!(cx.read(|cx| panel.read(cx).selected_text().is_some()));
    }

    #[gpui::test]
    fn dragging_from_list_whitespace_selects_the_marker_prefix(cx: &mut gpui::TestAppContext) {
        let preview = crate::preview::loading::derive_preview(
            PathBuf::from("selection.md"),
            DocumentSnapshot::from_utf8(b"1. hello world\n".to_vec()).unwrap(),
        );
        let panel = cx.new(|_| ReadingPreviewPanel::new(Arc::new(preview), 80.0));
        let render_panel = panel.clone();
        let cx = cx.add_empty_window();
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(240.0), px(40.0)),
            move |_, _| {
                let row_bounds = ReadingRowBounds::default();
                ReadingRowScope::new(
                    row_bounds.clone(),
                    div().w_full().pl(px(50.0)).child(
                        SelectableReadingText::new(
                            ("selection-list-prefix-test", 0usize),
                            gpui::StyledText::new("hello world"),
                            render_panel.clone(),
                            0,
                            None,
                        )
                        .with_text_offset(3)
                        .with_row_text_len("1. hello world".len())
                        .with_row_bounds(Some(row_bounds)),
                    ),
                )
            },
        );
        cx.simulate_mouse_down(
            point(px(4.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(px(130.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            point(px(130.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );

        let selected = cx.read(|cx| panel.read(cx).selected_text()).unwrap();
        assert!(selected.starts_with("1. "));
        assert!(selected.len() > 3);
    }

    #[gpui::test]
    fn drag_can_start_on_a_blank_row_and_continue_into_text(cx: &mut gpui::TestAppContext) {
        let preview = crate::preview::loading::derive_preview(
            PathBuf::from("selection.md"),
            DocumentSnapshot::from_utf8(b"first\n\nthird\n".to_vec()).unwrap(),
        );
        let panel = cx.new(|_| ReadingPreviewPanel::new(Arc::new(preview), 80.0));
        let render_panel = panel.clone();
        let cx = cx.add_empty_window();
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(200.0), px(60.0)),
            move |_, _| {
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .text_size(px(14.0))
                    .line_height(px(20.0))
                    .child(
                        SelectableReadingText::new(
                            ("selection-blank-start-test", 0usize),
                            gpui::StyledText::new("first"),
                            render_panel.clone(),
                            0,
                            None,
                        )
                        .with_minimum_height(20.0),
                    )
                    .child(SelectableReadingText::new(
                        ("selection-blank-start-test", 1usize),
                        gpui::StyledText::new(""),
                        render_panel.clone(),
                        1,
                        None,
                    ))
                    .child(
                        SelectableReadingText::new(
                            ("selection-blank-start-test", 2usize),
                            gpui::StyledText::new("third"),
                            render_panel.clone(),
                            2,
                            None,
                        )
                        .with_minimum_height(20.0),
                    )
            },
        );
        cx.simulate_mouse_down(
            point(px(100.0), px(30.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(px(36.0), px(50.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            point(px(36.0), px(50.0)),
            MouseButton::Left,
            Modifiers::default(),
        );

        let selected = cx.read(|cx| panel.read(cx).selected_text()).unwrap();
        assert!(selected.starts_with('\n'));
        assert!(selected.len() > 1);
    }

    #[gpui::test]
    fn drag_can_finish_on_a_blank_row(cx: &mut gpui::TestAppContext) {
        let preview = crate::preview::loading::derive_preview(
            PathBuf::from("selection.md"),
            DocumentSnapshot::from_utf8(b"first\n\nthird\n".to_vec()).unwrap(),
        );
        let panel = cx.new(|_| ReadingPreviewPanel::new(Arc::new(preview), 80.0));
        let render_panel = panel.clone();
        let cx = cx.add_empty_window();
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(200.0), px(40.0)),
            move |_, _| {
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .text_size(px(14.0))
                    .line_height(px(20.0))
                    .child(
                        SelectableReadingText::new(
                            ("selection-blank-end-test", 0usize),
                            gpui::StyledText::new("first"),
                            render_panel.clone(),
                            0,
                            None,
                        )
                        .with_minimum_height(20.0),
                    )
                    .child(
                        SelectableReadingText::new(
                            ("selection-blank-end-test", 1usize),
                            gpui::StyledText::new(""),
                            render_panel.clone(),
                            1,
                            None,
                        )
                        .with_minimum_height(20.0),
                    )
            },
        );
        cx.simulate_mouse_down(
            point(px(1.0), px(10.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(px(100.0), px(30.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            point(px(100.0), px(30.0)),
            MouseButton::Left,
            Modifiers::default(),
        );

        let selected = cx.read(|cx| panel.read(cx).selected_text()).unwrap();
        assert!(selected.starts_with("first"));
        assert!(selected.ends_with('\n'));
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
