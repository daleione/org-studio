//! Reading text elements and the shared geometry used by selection and hit testing.
use crate::preview::{
    PreviewStyle,
    display_map::DisplayRuns,
    inline_layout::{InlineLayout, InlineTextMetrics, needs_sized_inline, shape_inline},
};
use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Point, Style, Window, px,
};
use std::{
    cell::{Ref, RefCell},
    rc::Rc,
};

/// The layout is populated during measurement and shared by painting and mouse callbacks.
#[derive(Clone, Default)]
pub(in crate::preview) struct InlineLayoutHandle(Rc<RefCell<Option<InlineLayout>>>);

impl InlineLayoutHandle {
    fn read(&self) -> Ref<'_, InlineLayout> {
        Ref::map(self.0.borrow(), |layout| {
            layout
                .as_ref()
                .expect("inline text must be measured before use")
        })
    }
}

#[derive(Clone)]
pub(in crate::preview) enum ReadingTextLayout {
    Uniform(gpui::TextLayout),
    Sized(InlineLayoutHandle),
}

impl ReadingTextLayout {
    pub(super) fn visual_range(&self, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
        match self {
            Self::Uniform(_) => range,
            Self::Sized(layout) => layout.read().visual_range(range),
        }
    }
    pub(in crate::preview) fn index_for_position(
        &self,
        position: Point<Pixels>,
    ) -> Result<usize, usize> {
        match self {
            Self::Uniform(layout) => layout.index_for_position(position),
            Self::Sized(layout) => layout.read().index_for_position(position),
        }
    }
    pub(in crate::preview) fn position_for_index(&self, index: usize) -> Option<Point<Pixels>> {
        match self {
            Self::Uniform(layout) => layout.position_for_index(index),
            Self::Sized(layout) => layout.read().position_for_index(index),
        }
    }
    pub(in crate::preview) fn bounds(&self) -> Bounds<Pixels> {
        match self {
            Self::Uniform(layout) => layout.bounds(),
            Self::Sized(layout) => layout.read().bounds,
        }
    }
    pub(in crate::preview) fn len(&self) -> usize {
        match self {
            Self::Uniform(layout) => layout.len(),
            Self::Sized(layout) => layout.read().len,
        }
    }
    pub(in crate::preview) fn line_height(&self) -> Pixels {
        match self {
            Self::Uniform(layout) => layout.line_height(),
            Self::Sized(layout) => layout.read().line_height,
        }
    }
}

/// GPUI erases the element type; the matching layout remains available for interaction.
pub(in crate::preview) struct ReadingText {
    pub(super) element: AnyElement,
    layout: ReadingTextLayout,
}

type MathTarget = (
    gpui::Entity<crate::preview::ReadingPreviewPanel>,
    usize,
    usize,
);

impl ReadingText {
    pub(in crate::preview) fn uniform(text: crate::document_preview::RecordedText) -> Self {
        let layout = ReadingTextLayout::Uniform(text.layout().clone());
        Self {
            element: text.into_any_element(),
            layout,
        }
    }

    pub(in crate::preview) fn inline(
        text: crate::document_preview::RecordedText,
        runs: &DisplayRuns,
        style: PreviewStyle,
        zoom: f32,
    ) -> Self {
        if needs_sized_inline(&runs.inline_spans, style) {
            Self::sized(runs, style, zoom, None)
        } else {
            Self::uniform(text)
        }
    }

    pub(super) fn sized(
        runs: &DisplayRuns,
        style: PreviewStyle,
        zoom: f32,
        math_owner: Option<MathTarget>,
    ) -> Self {
        let layout = InlineLayoutHandle::default();
        let errors = runs.math.errors();
        let error_id = math_owner
            .as_ref()
            .filter(|_| !errors.is_empty())
            .map(|(_, row, slot)| gpui::SharedString::from(format!("formula-error-{row}-{slot}")));
        let element = SizedInlineText {
            runs: runs.clone(),
            style,
            zoom,
            layout: layout.clone(),
            math_owner,
        }
        .into_any_element();
        let element = if let Some(error_id) = error_id {
            use gpui::prelude::*;
            let hint = format!(
                "{}\n{}",
                crate::i18n::Language::system().text("math.render_failed"),
                errors
                    .iter()
                    .map(|s| s.as_ref())
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            gpui::div()
                .id(error_id)
                .w_full()
                .child(element)
                .tooltip(move |_, cx| cx.new(|_| FormulaErrorHint(hint.clone())).into())
                .into_any_element()
        } else {
            element
        };
        Self {
            element,
            layout: ReadingTextLayout::Sized(layout),
        }
    }

    pub(super) fn layout(&self) -> &ReadingTextLayout {
        &self.layout
    }
}

struct FormulaErrorHint(String);
impl gpui::Render for FormulaErrorHint {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        use crate::components::UiTextExt;
        use gpui::prelude::*;
        let theme = crate::theme::current_theme();
        gpui::div()
            .max_w(px(480.0))
            .p_2()
            .rounded_md()
            .bg(gpui::rgb(theme.background))
            .border_1()
            .border_color(gpui::rgb(theme.border))
            .text_color(gpui::rgb(theme.foreground))
            .text_size(crate::typography::ui_px(12.0))
            .child_text(self.0.clone())
    }
}

impl IntoElement for ReadingText {
    type Element = AnyElement;
    fn into_element(self) -> AnyElement {
        self.element
    }
    fn into_any_element(self) -> AnyElement {
        self.element
    }
}

struct SizedInlineText {
    runs: DisplayRuns,
    style: PreviewStyle,
    zoom: f32,
    layout: InlineLayoutHandle,
    math_owner: Option<MathTarget>,
}

impl Element for SizedInlineText {
    type RequestLayoutState = crate::math::MathStyle;
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let text_style = window.text_style();
        let text_align = text_style.text_align;
        let font_size = f32::from(text_style.font_size.to_pixels(window.rem_size()));
        let line_height = f32::from(
            text_style
                .line_height
                .to_pixels(px(font_size).into(), window.rem_size()),
        );
        let runs = super::styled_text::inline_text_runs(
            &self.runs.text,
            &self.runs.inline_spans,
            self.style,
            &text_style,
        );
        let display = self.runs.clone();
        let code_font_size = self
            .style
            .typography
            .inline_code_font_size(font_size, self.zoom);
        let layout = self.layout.clone();
        let foreground = self.style.palette.foreground;
        let id =
            window.request_measured_layout(Style::default(), move |known, available, window, _| {
                let width = known
                    .width
                    .or(match available.width {
                        AvailableSpace::Definite(width) => Some(width),
                        _ => None,
                    })
                    .map_or(f32::MAX, f32::from);
                let mut shaped = shape_inline(
                    &display.text,
                    &runs,
                    &display.inline_spans,
                    &display.math,
                    InlineTextMetrics {
                        font_size,
                        code_font_size,
                        line_height,
                        wrap_width: width,
                        foreground,
                    },
                    window.text_system(),
                );
                shaped.set_alignment(text_align);
                let size = shaped.size;
                layout.0.replace(Some(shaped));
                size
            });
        (id, crate::math::MathStyle::new(font_size, foreground))
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        math_style: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.layout
            .0
            .borrow_mut()
            .as_mut()
            .unwrap()
            .set_bounds(bounds);
        if bounds.intersects(&window.content_mask().bounds)
            && let Some((panel, row, slot)) = &self.math_owner
            && !self.runs.math.is_empty()
        {
            let visible = self
                .layout
                .read()
                .visible_range(window.content_mask().bounds);
            panel.update(cx, |panel, cx| {
                panel.request_math(*row, *slot, self.runs.math.slice(visible), *math_style, cx)
            });
        }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        math_style: &mut Self::RequestLayoutState,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.layout.read();
        layout.paint(window, cx);
        self.runs.math.trace_paint(
            *math_style,
            layout.formula_count(window.content_mask().bounds),
        );
    }
}

impl IntoElement for SizedInlineText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::{InlineKind, InlineSpan};
    use gpui::{TextAlign, div, point, prelude::*, px, size};
    use std::{collections::HashSet, path::PathBuf};
    use unicode_segmentation::UnicodeSegmentation;

    #[gpui::test]
    fn independent_inline_size_matches_virtual_measurement_and_hit_testing(
        cx: &mut gpui::TestAppContext,
    ) {
        let preview = crate::preview::loading::derive_preview(
            PathBuf::from("fonts.md"),
            crate::document::DocumentSnapshot::from_utf8(
                "small `abcdefgh中文🙂ijklmnop` tail\n".as_bytes().to_vec(),
            )
            .unwrap(),
        );
        let map = preview.display_map.as_ref().unwrap().clone();
        let display = map.runs(0);
        let mut style = *crate::preview::preview_style(crate::preview::PreviewStyleId::Base);
        style.typography.code_size = 28.0;
        style.typography.inline_code_size = Some(28.0);
        let observed = InlineLayoutHandle::default();
        let painted = observed.clone();
        let cx = cx.add_empty_window();
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(120.0), px(400.0)),
            move |_, _| {
                div()
                    .w(px(120.0))
                    .font(crate::typography::font_with_fallbacks(
                        style.typography.body_family,
                        style.typography.body_fallbacks,
                    ))
                    .text_size(px(14.0))
                    .line_height(px(22.0))
                    .child(SizedInlineText {
                        runs: display.clone(),
                        style,
                        zoom: 1.0,
                        layout: painted.clone(),
                        math_owner: None,
                    })
                    .into_any_element()
            },
        );
        let measured =
            cx.update(|window, _| map.display_lines(0, 120.0, 1.0, style, window.text_system()));
        let layout = observed.read();
        assert!(
            layout
                .fragments
                .iter()
                .any(|fragment| fragment.line.font_size == px(28.0))
        );
        assert!(
            layout
                .fragments
                .iter()
                .any(|fragment| fragment.line.font_size == px(14.0))
        );
        assert!(layout.line_starts.len() > 1);
        assert_eq!(
            measured
                .ranges
                .iter()
                .map(|range| range.start)
                .collect::<Vec<_>>(),
            layout.line_starts
        );
        let row = map.layout(0, style);
        let expected_height =
            (f32::from(layout.size.height) + row.padding_top + row.padding_bottom)
                .max(row.min_height)
                + row.margin_top
                + row.margin_bottom;
        assert!((measured.parent_height - expected_height).abs() < 0.01);
        assert_eq!(
            layout
                .fragments
                .iter()
                .map(|fragment| fragment.range.len())
                .sum::<usize>(),
            layout.len
        );
        let text = map.runs(0).text;
        for (index, _) in text.char_indices() {
            let position = layout.position_for_index(index).unwrap();
            let nearest = layout.index_for_position(position);
            assert_eq!(
                nearest.unwrap_or_else(|index| index),
                index,
                "hit test at byte {index}"
            );
        }
    }

    #[gpui::test]
    fn narrow_mixed_size_wrapping_preserves_unicode_graphemes(cx: &mut gpui::TestAppContext) {
        let text = "Ae\u{301}中文👩‍💻Z";
        let spans = vec![InlineSpan {
            source: 0..text.len(),
            range: 0..text.len(),
            kind: InlineKind::Code,
            target: None,
        }];
        let cx = cx.add_empty_window();
        let layout = cx.update(|window, _| {
            shape_inline(
                text,
                &[gpui::TextRun {
                    len: text.len(),
                    font: gpui::font("Menlo"),
                    ..Default::default()
                }],
                &spans,
                &crate::preview::math::MathRow::default(),
                InlineTextMetrics {
                    font_size: 14.0,
                    code_font_size: 28.0,
                    line_height: 22.0,
                    wrap_width: 1.0,
                    foreground: 0x202020,
                },
                window.text_system(),
            )
        });
        let boundaries = text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain([text.len()])
            .collect::<HashSet<_>>();
        assert!(!layout.fragments.is_empty());
        assert_eq!(
            layout
                .fragments
                .iter()
                .map(|fragment| fragment.range.len())
                .sum::<usize>(),
            text.len()
        );
        for fragment in &layout.fragments {
            assert!(boundaries.contains(&fragment.range.start));
            assert!(boundaries.contains(&fragment.range.end));
        }
    }

    #[gpui::test]
    fn inline_code_keeps_its_size_and_alignment_in_different_text_contexts(
        cx: &mut gpui::TestAppContext,
    ) {
        let text = "body code";
        let spans = vec![InlineSpan {
            source: 5..9,
            range: 5..9,
            kind: InlineKind::Code,
            target: None,
        }];
        let mut style = *crate::preview::preview_style(crate::preview::PreviewStyleId::Base);
        style.typography.inline_code_size = Some(28.0);
        let inherited = gpui::TextStyle::default();
        let runs =
            crate::preview::view::styled_text::inline_text_runs(text, &spans, style, &inherited);
        let cx = cx.add_empty_window();
        for (body_size, line_height, zoom) in
            [(24.0, 32.0, 1.0), (13.0, 19.0, 1.0), (26.0, 38.0, 2.0)]
        {
            let mut layout = cx.update(|window, _| {
                shape_inline(
                    text,
                    &runs,
                    &spans,
                    &crate::preview::math::MathRow::default(),
                    InlineTextMetrics {
                        font_size: body_size,
                        code_font_size: style.typography.inline_code_font_size(body_size, zoom),
                        line_height,
                        wrap_width: 400.0,
                        foreground: style.palette.foreground,
                    },
                    window.text_system(),
                )
            });
            assert!(
                (f32::from(layout.fragments.last().unwrap().line.font_size) - 28.0 * zoom).abs()
                    < 0.0001
            );
            assert_eq!(
                layout.fragments.first().unwrap().line.font_size,
                px(body_size)
            );
            for align in [TextAlign::Center, TextAlign::Right] {
                layout.set_alignment(align);
                layout.set_bounds(Bounds::new(
                    point(px(10.0), px(20.0)),
                    size(px(400.0), layout.size.height),
                ));
                let start = layout.position_for_index(0).unwrap();
                assert!(start.x > layout.bounds.left());
                for (index, _) in text.char_indices() {
                    let position = layout.position_for_index(index).unwrap();
                    assert_eq!(layout.index_for_position(position).unwrap(), index);
                }
            }
        }
    }
}
