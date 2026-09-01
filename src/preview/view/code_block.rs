use std::{ops::Range, sync::Arc};

use gpui::{FontWeight, HighlightStyle, SharedString, StyledText, div, prelude::*, px, rgb};

use crate::preview::{
    CodeHighlightSpan, CodeRowRole, PreviewStyle, display_map::RowLayout, style::CodeBlockVariant,
    view::styled_text::styled_code_runs,
};

pub(super) fn render_code_row(
    text: SharedString,
    code_spans: Arc<[CodeHighlightSpan]>,
    layout: RowLayout,
    role: CodeRowRole,
    card_bottom: bool,
    style: PreviewStyle,
) -> gpui::Div {
    let palette = style.palette;
    let content = if role.is_boundary() {
        styled_code_boundary(text, role, style)
    } else {
        styled_code_runs(text, code_spans, style)
    };
    let background = if role.is_boundary() {
        rgb(palette.code_background).blend(rgb(palette.code_boundary_background).alpha(0.62))
    } else {
        rgb(palette.code_background)
    };

    div()
        .min_h(px(layout.min_height))
        .pl(px(layout.padding_left))
        .pr(px(layout.padding_right))
        .pt(px(layout.padding_top))
        .pb(px(layout.padding_bottom))
        .when(
            style.variants.code_block == CodeBlockVariant::AccentBar,
            |element| {
                element
                    .border_l_2()
                    .border_color(rgb(palette.code_block_accent))
            },
        )
        .when(
            style.variants.code_block == CodeBlockVariant::Card,
            |element| {
                element
                    .border_l_1()
                    .border_r_1()
                    .border_color(rgb(palette.border))
            },
        )
        .when(
            style.variants.code_block == CodeBlockVariant::Card && card_bottom,
            |element| element.border_b_1().rounded_b(px(style.spacing.radius)),
        )
        .bg(background)
        .text_color(rgb(if role.is_boundary() {
            palette.code_boundary
        } else {
            palette.code_foreground
        }))
        .font_family(style.typography.code_family)
        .text_size(px(layout.font_size))
        .line_height(px(layout.line_height))
        .child(content)
}

fn styled_code_boundary(text: SharedString, role: CodeRowRole, style: PreviewStyle) -> StyledText {
    let palette = style.palette;
    let mut highlights = Vec::new();
    if role == CodeRowRole::Open
        && let Some((language, arguments)) = fence_metadata_ranges(&text)
    {
        highlights.push((
            language,
            HighlightStyle {
                color: Some(rgb(palette.date).into()),
                font_weight: Some(FontWeight::SEMIBOLD),
                ..Default::default()
            },
        ));
        if let Some(arguments) = arguments {
            highlights.push((
                arguments,
                HighlightStyle {
                    color: Some(rgb(palette.meta).into()),
                    ..Default::default()
                },
            ));
        }
    }
    StyledText::new(text).with_highlights(highlights)
}

fn fence_metadata_ranges(source: &str) -> Option<(Range<usize>, Option<Range<usize>>)> {
    let trimmed = source.trim_start();
    let leading = source.len() - trimmed.len();
    let marker_end = if let Some(marker @ (b'`' | b'~')) = trimmed.as_bytes().first().copied() {
        trimmed.bytes().take_while(|byte| *byte == marker).count()
    } else if trimmed
        .get(.."#+begin_src".len())
        .is_some_and(|marker| marker.eq_ignore_ascii_case("#+begin_src"))
    {
        "#+begin_src".len()
    } else {
        return None;
    };
    let metadata = &trimmed[marker_end..];
    let language_offset = metadata.len() - metadata.trim_start().len();
    let language_start = marker_end + language_offset;
    let language_len = metadata[language_offset..]
        .find(char::is_whitespace)
        .unwrap_or(metadata.len() - language_offset);
    if language_len == 0 {
        return None;
    }
    let language = leading + language_start..leading + language_start + language_len;
    let after_language = language_start + language_len;
    let arguments_start = after_language + trimmed[after_language..].len()
        - trimmed[after_language..].trim_start().len();
    let arguments = (arguments_start < trimmed.len())
        .then_some(leading + arguments_start..leading + trimmed.len());
    Some((language, arguments))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locates_markdown_and_org_language_metadata() {
        assert_eq!(fence_metadata_ranges("```rust"), Some((3..7, None)));
        assert_eq!(
            fence_metadata_ranges("#+begin_src rust :results silent"),
            Some((12..16, Some(17..32)))
        );
        assert_eq!(fence_metadata_ranges("```"), None);
    }
}
