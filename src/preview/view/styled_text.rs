use super::{
    Arc, CodeHighlightKind, CodeHighlightSpan, FontStyle, FontWeight, HighlightStyle, InlineKind,
    InlineSpan, StyledText, current_theme, rgb,
};

pub(super) fn styled_code_runs(
    text: gpui::SharedString,
    spans: Arc<[CodeHighlightSpan]>,
) -> StyledText {
    let highlights = spans
        .iter()
        .map(|span| (span.start..span.end, code_highlight_style(span.kind)));
    StyledText::new(text).with_highlights(highlights)
}

pub(in crate::preview) fn code_highlight_style(kind: CodeHighlightKind) -> HighlightStyle {
    let theme = current_theme();
    let color = match kind {
        CodeHighlightKind::Attribute => theme.attribute,
        CodeHighlightKind::Boolean | CodeHighlightKind::Constant => theme.constant,
        CodeHighlightKind::Comment => theme.comment,
        CodeHighlightKind::Function => theme.function,
        CodeHighlightKind::Keyword => theme.keyword,
        CodeHighlightKind::Number => theme.number,
        CodeHighlightKind::Operator | CodeHighlightKind::Punctuation => theme.operator,
        CodeHighlightKind::Property | CodeHighlightKind::Variable => theme.variable,
        CodeHighlightKind::String => theme.string,
        CodeHighlightKind::Type => theme.type_name,
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
) -> StyledText {
    let theme = current_theme();
    let highlights = spans.iter().map(|span| {
        let style = match span.kind {
            InlineKind::Bold => HighlightStyle {
                font_weight: Some(FontWeight::BOLD),
                ..Default::default()
            },
            InlineKind::Italic => HighlightStyle {
                font_style: Some(FontStyle::Italic),
                ..Default::default()
            },
            InlineKind::Underline => HighlightStyle {
                color: Some(rgb(theme.link).into()),
                ..Default::default()
            },
            InlineKind::Strike => HighlightStyle {
                fade_out: Some(0.55),
                ..Default::default()
            },
            InlineKind::Code | InlineKind::Verbatim => HighlightStyle {
                color: Some(rgb(theme.inline_code).into()),
                background_color: Some(rgb(theme.inline_code_background).into()),
                ..Default::default()
            },
            InlineKind::Link | InlineKind::FootnoteReference => HighlightStyle {
                color: Some(rgb(theme.link).into()),
                font_weight: Some(FontWeight::MEDIUM),
                ..Default::default()
            },
            InlineKind::Target | InlineKind::RadioTarget => HighlightStyle {
                color: Some(rgb(theme.attribute).into()),
                font_weight: Some(FontWeight::MEDIUM),
                ..Default::default()
            },
            InlineKind::Timestamp => HighlightStyle {
                color: Some(rgb(theme.date).into()),
                ..Default::default()
            },
            InlineKind::Entity | InlineKind::Latex => HighlightStyle {
                color: Some(rgb(theme.function).into()),
                ..Default::default()
            },
        };
        (span.range.clone(), style)
    });
    StyledText::new(text).with_highlights(highlights)
}
