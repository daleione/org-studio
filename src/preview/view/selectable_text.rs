use std::{cell::Cell, ops::Range, rc::Rc, sync::Arc};

use gpui::{
    AnyElement, App, Bounds, CursorStyle, DispatchPhase, Element, ElementId, GlobalElementId,
    Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Style, Window, fill, point, px, relative, rgba,
};

use super::inline_text::{ReadingText, ReadingTextLayout};
use crate::{
    org_syntax::inline::{InlineKind, InlineSpan},
    preview::ReadingPreviewPanel,
};

type ClickListener = Box<dyn Fn(Arc<str>, &mut Window, &mut App)>;

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
        __inspector_id: Option<&InspectorElementId>,
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
        __inspector_id: Option<&InspectorElementId>,
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
        __inspector_id: Option<&InspectorElementId>,
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
    text: ReadingText,
    panel: gpui::Entity<ReadingPreviewPanel>,
    row: usize,
    text_offset: usize,
    row_text_len: Option<usize>,
    restrict_drag_to_bounds: bool,
    row_bounds: Option<ReadingRowBounds>,
    minimum_height: Option<f32>,
    selection: Option<(Range<usize>, bool)>,
    search_ranges: Vec<(Range<usize>, bool)>,
    inline_spans: Option<Arc<[InlineSpan]>>,
    click_listener: Option<ClickListener>,
}

impl SelectableReadingText {
    pub(in crate::preview) fn new(
        id: impl Into<ElementId>,
        text: crate::document_preview::RecordedText,
        panel: gpui::Entity<ReadingPreviewPanel>,
        row: usize,
        selection: Option<(Range<usize>, bool)>,
    ) -> Self {
        Self {
            id: id.into(),
            text: ReadingText::uniform(text),
            panel,
            row,
            text_offset: 0,
            row_text_len: None,
            restrict_drag_to_bounds: false,
            row_bounds: None,
            minimum_height: None,
            selection,
            search_ranges: Vec::new(),
            inline_spans: None,
            click_listener: None,
        }
    }

    pub(in crate::preview) fn with_search_ranges(
        mut self,
        ranges: Vec<(Range<usize>, bool)>,
    ) -> Self {
        self.search_ranges = ranges;
        self
    }

    pub(in crate::preview) fn with_inline_fonts(
        mut self,
        runs: &crate::preview::display_map::DisplayRuns,
        style: crate::preview::PreviewStyle,
        zoom: f32,
        math_slot: usize,
    ) -> Self {
        if crate::preview::inline_layout::needs_sized_inline(&runs.inline_spans, style) {
            self.text = ReadingText::sized(
                runs,
                style,
                zoom,
                Some((self.panel.clone(), self.row, math_slot)),
            );
        }
        self
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

    pub(in crate::preview) fn with_links(
        self,
        spans: Arc<[InlineSpan]>,
        interaction: &super::ReadingInteraction,
    ) -> Self {
        if !spans.iter().any(|span| span.kind == InlineKind::Link) {
            return self;
        }
        let Some(row) = interaction.document.projection.rows.get(self.row) else {
            return self;
        };
        let target = crate::preview::source_action_target(&interaction.document, row);
        let interaction = interaction.clone();
        self.on_double_click(spans, move |destination, window, cx| {
            (interaction.dispatch)(
                crate::preview::PreviewAction::OpenLink {
                    target: target.clone(),
                    destination,
                },
                interaction.panel.clone(),
                window,
                cx,
            );
        })
    }

    fn on_double_click(
        mut self,
        spans: Arc<[InlineSpan]>,
        listener: impl Fn(Arc<str>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.inline_spans = Some(spans);
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
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let text_layout_id = self.text.element.request_layout(window, cx);
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
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let text_bounds = window.layout_bounds(state.text_layout_id);
        self.text.element.prepaint(window, cx);
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
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_state: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.text.layout().clone();
        let hitbox = &state.hitbox;
        let text_bounds = state.text_bounds;

        if hitbox.is_hovered(window)
            && link_at_position(
                &layout,
                self.inline_spans.as_deref().unwrap_or_default(),
                window.mouse_position(),
            )
            .is_some()
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
        let move_hitbox = hitbox.clone();
        let spans = self.inline_spans.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if !event.dragging()
                && move_hitbox.is_hovered(window)
                && let Some(target) = link_at_position(
                    &move_layout,
                    spans.as_deref().unwrap_or_default(),
                    event.position,
                )
            {
                move_panel.update(cx, |panel, cx| {
                    panel.hover_link(Some(target.clone()), cx);
                });
                cx.stop_propagation();
            }
            if !event.dragging()
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
        let spans = self.inline_spans.clone();
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
            if event.click_count != 2 || suppress_click || !up_hitbox.is_hovered(window) {
                return;
            }
            if let Some(target) = link_at_position(
                &up_layout,
                spans.as_deref().unwrap_or_default(),
                event.position,
            ) && let Some(listener) = listener.as_ref()
            {
                cx.stop_propagation();
                listener(target.clone(), window, cx);
            }
        });

        self.text.element.paint(window, cx);
        for (range, current) in &self.search_ranges {
            paint_search(&layout, text_bounds, range.clone(), false, window, *current);
        }
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

fn nearest_index(layout: &ReadingTextLayout, position: gpui::Point<Pixels>) -> usize {
    match layout.index_for_position(position) {
        Ok(index) | Err(index) => index.min(layout.len()),
    }
}

fn link_at_position<'a>(
    layout: &ReadingTextLayout,
    spans: &'a [InlineSpan],
    position: gpui::Point<Pixels>,
) -> Option<&'a Arc<str>> {
    let bounds = layout.bounds();
    if !bounds.contains(&position) {
        return None;
    }
    let line_height = f32::from(layout.line_height());
    let line_at = |y: Pixels| (f32::from(y - bounds.top()) / line_height).floor() as usize;
    let line = line_at(position.y);
    spans
        .iter()
        .find(|link| {
            if link.kind != InlineKind::Link {
                return false;
            }
            let Some(start) = layout.position_for_index(link.range.start) else {
                return false;
            };
            let Some(end) = layout.position_for_index(link.range.end) else {
                return false;
            };
            let first_line = line_at(start.y);
            let last_line = line_at(end.y);
            let left = if line == first_line {
                start.x
            } else {
                bounds.left()
            };
            let right = if line == last_line {
                end.x
            } else {
                bounds.right()
            };
            first_line <= line && line <= last_line && position.x >= left && position.x < right
        })
        .and_then(|span| span.target.as_ref())
}

fn nearest_document_index(
    layout: &ReadingTextLayout,
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
    layout: &ReadingTextLayout,
    bounds: Bounds<Pixels>,
    range: Range<usize>,
    include_newline: bool,
    window: &mut Window,
) {
    let range = layout.visual_range(range);
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
        crate::document_preview::quad(
            window,
            fill(
                Bounds::from_corners(position, point(position.x + px(8.0), position.y + height)),
                rgba((crate::theme::current_theme().accent << 8) | 0x4a),
            ),
        );
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
        crate::document_preview::quad(
            window,
            fill(
                Bounds::from_corners(point(left, top), point(right, top + px(line_height))),
                rgba((crate::theme::current_theme().accent << 8) | 0x4a),
            ),
        );
    }
}

fn paint_search(
    layout: &ReadingTextLayout,
    bounds: Bounds<Pixels>,
    range: Range<usize>,
    include_newline: bool,
    window: &mut Window,
    current: bool,
) {
    let range = layout.visual_range(range);
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
        crate::document_preview::quad(
            window,
            fill(
                Bounds::from_corners(position, point(position.x + px(8.0), position.y + height)),
                rgba(if current {
                    crate::theme::current_theme().search_current
                } else {
                    crate::theme::current_theme().search_match
                }),
            ),
        );
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
        crate::document_preview::quad(
            window,
            fill(
                Bounds::from_corners(point(left, top), point(right, top + px(line_height))),
                rgba(if current {
                    crate::theme::current_theme().search_current
                } else {
                    crate::theme::current_theme().search_match
                }),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, path::PathBuf, rc::Rc, sync::Arc};

    use gpui::{AppContext, IntoElement, Modifiers, MouseButton, div, point, prelude::*, px, size};

    use crate::{document::DocumentSnapshot, preview::ReadingPreviewPanel};

    use super::{ReadingRowBounds, ReadingRowScope, SelectableReadingText};

    #[gpui::test]
    fn wrapped_reading_link_hit_testing_includes_the_last_half_of_each_glyph(
        cx: &mut gpui::TestAppContext,
    ) {
        let caption = "Reference 文档 with a caption that wraps across several lines";
        let text = crate::document_preview::RecordedText::new(format!("{caption} tail"));
        let layout = super::ReadingTextLayout::Uniform(text.layout().clone());
        let links = [super::InlineSpan {
            kind: super::InlineKind::Link,
            source: 0..caption.len(),
            range: 0..caption.len(),
            target: Some("https://example.org/reference".into()),
        }];
        let view = cx.add_empty_window();
        view.draw(
            point(px(30.0), px(40.0)),
            size(px(140.0), px(300.0)),
            move |_, _| {
                div()
                    .w(px(140.0))
                    .text_size(px(14.0))
                    .line_height(px(23.8))
                    .child(text)
                    .into_any_element()
            },
        );
        for (index, character) in caption.char_indices() {
            if character.is_whitespace() {
                continue;
            }
            let start = layout.position_for_index(index).unwrap();
            let end = layout
                .position_for_index(index + character.len_utf8())
                .unwrap();
            let x = if start.y == end.y {
                start.x + (end.x - start.x) * 0.9
            } else {
                start.x + px(1.0)
            };
            let point = point(x, start.y + layout.line_height() * 0.5);
            assert_eq!(
                super::link_at_position(&layout, &links, point).map(|target| target.as_ref()),
                Some("https://example.org/reference"),
                "missing glyph {character:?} at {point:?}"
            );
        }
        let end = layout.position_for_index(caption.len()).unwrap();
        assert!(
            super::link_at_position(&layout, &links, point(end.x + px(2.0), end.y + px(5.0)))
                .is_none()
        );
    }

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
                    crate::document_preview::RecordedText::new("hello world"),
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
                                crate::document_preview::RecordedText::new("hello world"),
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
                                crate::document_preview::RecordedText::new("hello world"),
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
                                crate::document_preview::RecordedText::new("hello world"),
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
                            crate::document_preview::RecordedText::new("hello world"),
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
                            crate::document_preview::RecordedText::new("first"),
                            render_panel.clone(),
                            0,
                            None,
                        )
                        .with_minimum_height(20.0),
                    )
                    .child(SelectableReadingText::new(
                        ("selection-blank-start-test", 1usize),
                        crate::document_preview::RecordedText::new(""),
                        render_panel.clone(),
                        1,
                        None,
                    ))
                    .child(
                        SelectableReadingText::new(
                            ("selection-blank-start-test", 2usize),
                            crate::document_preview::RecordedText::new("third"),
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
                            crate::document_preview::RecordedText::new("first"),
                            render_panel.clone(),
                            0,
                            None,
                        )
                        .with_minimum_height(20.0),
                    )
                    .child(
                        SelectableReadingText::new(
                            ("selection-blank-end-test", 1usize),
                            crate::document_preview::RecordedText::new(""),
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
    fn a_later_text_row_opens_links_only_on_double_click_without_dragging(
        cx: &mut gpui::TestAppContext,
    ) {
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
                        crate::document_preview::RecordedText::new("first"),
                        render_panel.clone(),
                        0,
                        None,
                    ))
                    .child(
                        SelectableReadingText::new(
                            ("selection-test", 1usize),
                            crate::document_preview::RecordedText::new("second"),
                            render_panel.clone(),
                            1,
                            None,
                        )
                        .on_double_click(
                            Arc::from([super::InlineSpan {
                                kind: super::InlineKind::Link,
                                source: 0..6,
                                range: 0..6,
                                target: Some("https://example.org/".into()),
                            }]),
                            {
                                let clicked = render_clicked.clone();
                                move |_, _, _| clicked.set(true)
                            },
                        ),
                    )
                    .into_any_element()
            },
        );
        let position = point(px(8.0), px(30.0));
        cx.simulate_click(position, Modifiers::default());
        assert!(!clicked.get(), "a single click only places the text anchor");
        cx.read(|cx| assert!(!panel.read(cx).text_selection_pending()));
        cx.simulate_event(gpui::MouseDownEvent {
            position,
            button: MouseButton::Left,
            click_count: 2,
            ..Default::default()
        });
        cx.simulate_event(gpui::MouseUpEvent {
            position,
            button: MouseButton::Left,
            click_count: 2,
            ..Default::default()
        });
        assert!(clicked.get());

        clicked.set(false);
        cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(
            point(px(55.0), px(30.0)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_event(gpui::MouseUpEvent {
            position: point(px(55.0), px(30.0)),
            button: MouseButton::Left,
            click_count: 2,
            ..Default::default()
        });
        assert!(!clicked.get(), "dragging a link must only select text");
        cx.read(|cx| assert!(panel.read(cx).selected_text().is_some()));
    }
}
