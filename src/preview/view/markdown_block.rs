use super::{
    Arc, DocumentFormat, FontWeight, InlineText, PreviewDocument, current_theme, div, img,
    markdown, parse_inline, px, render_code_row, render_table_row, resolve_image_path, rgb,
    styled_inline_runs,
};
use gpui::prelude::*;

pub(in crate::preview) fn parse_document_inline(
    format: DocumentFormat,
    source: &str,
) -> InlineText {
    match format {
        DocumentFormat::Org => parse_inline(source),
        DocumentFormat::Markdown => markdown::parse_markdown_inline(source),
    }
}

pub(super) fn render_markdown_block(
    document: &Arc<PreviewDocument>,
    display_row: usize,
    block: &markdown::MarkdownBlock,
    is_folded: bool,
    available_width: f32,
) -> gpui::Div {
    use markdown::MarkdownKind;
    let theme = current_theme();
    let display_map = document
        .display_map
        .as_ref()
        .expect("preview display map must exist after loading");
    let display_runs = display_map.runs(display_row);
    let row_layout = display_map.layout(display_row);
    let text = display_runs.text.clone();
    let inline = || styled_inline_runs(text.clone(), display_runs.inline_spans.clone());
    match &block.kind {
        MarkdownKind::Blank => {
            div().h(px(row_layout.fixed_height.unwrap_or(row_layout.min_height)))
        }
        MarkdownKind::Heading { level } => {
            let index = (*level as usize).saturating_sub(1).min(3);
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(row_layout.font_size))
                .line_height(px(row_layout.line_height))
                .font_weight(if *level <= 2 {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(rgb(theme.heading[index]))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(13.0))
                        .child(format!("{} ", theme.heading_bullets[index])),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .child(inline())
                        .when(is_folded, |element| {
                            element.child(
                                div()
                                    .flex_none()
                                    .text_color(rgb(theme.keyword))
                                    .child("..."),
                            )
                        }),
                )
        }
        MarkdownKind::Paragraph => div()
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .child(inline()),
        MarkdownKind::ListItem => div()
            .pl(px(row_layout.padding_left))
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .child(inline()),
        MarkdownKind::Quote => div()
            .pl(px(row_layout.padding_left))
            .pr(px(row_layout.padding_right))
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .border_l_2()
            .border_color(rgb(theme.heading[1]))
            .text_color(rgb(theme.quote))
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .child(inline()),
        MarkdownKind::Code { role, .. } => {
            render_code_row(text, display_runs.code_spans, row_layout, *role)
        }
        MarkdownKind::TableRow => render_table_row(
            &text,
            display_map
                .table_projection(display_row)
                .expect("markdown table row projection must exist"),
        ),
        MarkdownKind::HorizontalRule => div()
            .mt(px(row_layout.margin_top))
            .mb(px(row_layout.margin_bottom))
            .h(px(row_layout.fixed_height.unwrap_or(1.0)))
            .w_full()
            .bg(rgb(theme.border)),
        MarkdownKind::Image { path } => {
            let source = resolve_image_path(&document.path, path);
            let fitted = display_map.image_size(display_row, available_width);
            div()
                .w_full()
                .pt(px(row_layout.padding_top))
                .pb(px(row_layout.padding_bottom))
                .flex()
                .items_start()
                .child(if let Some((width, height)) = fitted {
                    img(source).w(px(width)).h(px(height))
                } else {
                    img(source).max_w(px(available_width.min(960.0)))
                })
        }
    }
}
