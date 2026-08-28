use std::{ops::Range, sync::Arc};

use gpui::{FontWeight, HighlightStyle, SharedString, StyledText, div, prelude::*, px, rgb};

use crate::{
    preview::{
        CodeHighlightSpan, CodeRowRole, display_map::RowLayout, view::styled_text::styled_code_runs,
    },
    theme::current_theme,
};

pub(super) fn render_code_row(
    text: SharedString,
    code_spans: Arc<[CodeHighlightSpan]>,
    layout: RowLayout,
    role: CodeRowRole,
) -> gpui::Div {
    let theme = current_theme();
    let content = if role.is_boundary() {
        styled_code_boundary(text, role)
    } else {
        styled_code_runs(text, code_spans)
    };
    let background = if role.is_boundary() {
        rgb(theme.code_background).blend(rgb(theme.code_boundary_background).alpha(0.62))
    } else {
        rgb(theme.code_background)
    };

    div()
        .min_h(px(layout.min_height))
        .pl(px(layout.padding_left))
        .pr(px(layout.padding_right))
        .pt(px(layout.padding_top))
        .pb(px(layout.padding_bottom))
        .border_l_2()
        .border_color(rgb(theme.code_block_accent))
        .bg(background)
        .text_color(rgb(if role.is_boundary() {
            theme.code_boundary
        } else {
            theme.code_foreground
        }))
        .font_family("Menlo")
        .text_size(px(layout.font_size))
        .line_height(px(layout.line_height))
        .child(content)
}

pub(super) fn org_code_row_role(source: &str, continuation: bool) -> CodeRowRole {
    if !continuation {
        CodeRowRole::Open
    } else if source.trim().eq_ignore_ascii_case("#+end_src") {
        CodeRowRole::Close
    } else {
        CodeRowRole::Body
    }
}

fn styled_code_boundary(text: SharedString, role: CodeRowRole) -> StyledText {
    let theme = current_theme();
    let mut highlights = Vec::new();
    if role == CodeRowRole::Open
        && let Some((language, arguments)) = fence_metadata_ranges(&text)
    {
        highlights.push((
            language,
            HighlightStyle {
                color: Some(rgb(theme.date).into()),
                font_weight: Some(FontWeight::SEMIBOLD),
                ..Default::default()
            },
        ));
        if let Some(arguments) = arguments {
            highlights.push((
                arguments,
                HighlightStyle {
                    color: Some(rgb(theme.meta).into()),
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
    fn recognizes_org_source_boundaries_without_hiding_source_rows() {
        assert_eq!(
            org_code_row_role("  #+BEGIN_SRC rust", false),
            CodeRowRole::Open
        );
        assert_eq!(
            org_code_row_role("#+begin_src nested", true),
            CodeRowRole::Body
        );
        assert_eq!(org_code_row_role("let x = 1;", true), CodeRowRole::Body);
        assert_eq!(org_code_row_role("#+end_src", true), CodeRowRole::Close);
    }

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
