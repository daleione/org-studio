use super::{
    Arc, CodeHighlightKind, CodeHighlightSpan, FontStyle, FontWeight, HighlightStyle, InlineKind,
    InlineSpan, PreviewStyle, StyledText, rgb,
};
use std::ops::Range;

pub(super) fn styled_code_runs(
    text: gpui::SharedString,
    spans: Arc<[CodeHighlightSpan]>,
    preview_style: PreviewStyle,
) -> StyledText {
    let highlights = spans.iter().map(|span| {
        (
            span.start..span.end,
            code_highlight_style(span.kind, preview_style),
        )
    });
    StyledText::new(text).with_highlights(highlights)
}

pub(in crate::preview) fn code_highlight_style(
    kind: CodeHighlightKind,
    preview_style: PreviewStyle,
) -> HighlightStyle {
    let palette = preview_style.palette;
    let color = match kind {
        CodeHighlightKind::Attribute => palette.attribute,
        CodeHighlightKind::Boolean | CodeHighlightKind::Constant => palette.constant,
        CodeHighlightKind::Comment => palette.comment,
        CodeHighlightKind::Function => palette.function,
        CodeHighlightKind::Keyword => palette.keyword,
        CodeHighlightKind::Number => palette.number,
        CodeHighlightKind::Operator | CodeHighlightKind::Punctuation => palette.operator,
        CodeHighlightKind::Property | CodeHighlightKind::Variable => palette.variable,
        CodeHighlightKind::String => palette.string,
        CodeHighlightKind::Type => palette.type_name,
    };
    HighlightStyle {
        color: Some(rgb(color).into()),
        font_style: matches!(kind, CodeHighlightKind::Comment).then_some(FontStyle::Italic),
        ..Default::default()
    }
}

pub(in crate::preview) fn styled_inline_runs(
    text: gpui::SharedString,
    spans: Arc<[InlineSpan]>,
    preview_style: PreviewStyle,
) -> StyledText {
    let highlights = inline_highlights(&text, &spans, preview_style);
    StyledText::new(text).with_highlights(highlights)
}

fn inline_highlights(
    text: &str,
    spans: &[InlineSpan],
    preview_style: PreviewStyle,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut boundaries = Vec::with_capacity(spans.len() * 2);
    for span in spans {
        let valid = span.range.start < span.range.end
            && span.range.end <= text.len()
            && text.is_char_boundary(span.range.start)
            && text.is_char_boundary(span.range.end);
        debug_assert!(valid, "inline span must be a non-empty UTF-8 range");
        if valid {
            boundaries.push(span.range.start);
            boundaries.push(span.range.end);
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    boundaries
        .windows(2)
        .filter_map(|boundary| {
            let range = boundary[0]..boundary[1];
            let mut style = HighlightStyle::default();
            let mut styled = false;
            for span in spans
                .iter()
                .filter(|span| span.range.start < range.end && span.range.end > range.start)
            {
                apply_inline_style(&mut style, span.kind, preview_style);
                styled = true;
            }
            styled.then_some((range, style))
        })
        .collect()
}

fn apply_inline_style(style: &mut HighlightStyle, kind: InlineKind, preview_style: PreviewStyle) {
    let palette = preview_style.palette;
    match kind {
        InlineKind::Bold => merge_font_weight(style, FontWeight::BOLD),
        InlineKind::Italic => style.font_style = Some(FontStyle::Italic),
        InlineKind::Underline => style.color = Some(rgb(palette.link).into()),
        InlineKind::Strike => style.fade_out = Some(0.55),
        InlineKind::Code | InlineKind::Verbatim => {
            style.color = Some(rgb(palette.inline_code).into());
            style.background_color = Some(rgb(palette.inline_code_background).into());
        }
        InlineKind::Link | InlineKind::FootnoteReference => {
            style.color = Some(rgb(palette.link).into());
            merge_font_weight(style, FontWeight::MEDIUM);
        }
        InlineKind::Target | InlineKind::RadioTarget => {
            style.color = Some(rgb(palette.attribute).into());
            merge_font_weight(style, FontWeight::MEDIUM);
        }
        InlineKind::Timestamp => style.color = Some(rgb(palette.date).into()),
        InlineKind::Entity | InlineKind::Latex => {
            style.color = Some(rgb(palette.function).into());
        }
    }
}

fn merge_font_weight(style: &mut HighlightStyle, weight: FontWeight) {
    let weight = match style.font_weight {
        Some(current) if current >= weight => current,
        _ => weight,
    };
    style.font_weight = Some(weight);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::{DocumentFormat, PreviewStyleId, parse_document_inline, preview_style};

    #[gpui::test]
    fn nested_markdown_styles_are_flattened_before_gpui_layout(cx: &mut gpui::TestAppContext) {
        use gpui::{IntoElement, point, px, size};

        let parsed =
            parse_document_inline(DocumentFormat::Markdown, "**逗号 `,` 拼接成单个字符串**");
        let highlights = inline_highlights(
            &parsed.text,
            &parsed.spans,
            *preview_style(PreviewStyleId::Base),
        );

        assert!(
            highlights
                .windows(2)
                .all(|pair| pair[0].0.end <= pair[1].0.start)
        );
        assert!(highlights.iter().all(|(range, _)| {
            parsed.text.is_char_boundary(range.start)
                && parsed.text.is_char_boundary(range.end)
                && range.end <= parsed.text.len()
        }));
        let (_, nested) = highlights
            .iter()
            .find(|(range, _)| &parsed.text[range.clone()] == ",")
            .expect("inline code remains represented after flattening");
        assert_eq!(nested.font_weight, Some(FontWeight::BOLD));
        assert!(nested.background_color.is_some());

        let text: gpui::SharedString = parsed.text.into();
        let spans: Arc<[InlineSpan]> = parsed.spans.into();
        let cx = cx.add_empty_window();
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(400.0), px(80.0)),
            |_, _| {
                styled_inline_runs(
                    text.clone(),
                    spans.clone(),
                    *preview_style(PreviewStyleId::Base),
                )
                .into_any_element()
            },
        );
    }
}
