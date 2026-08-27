use super::{
    Arc, BlockId, BlockKind, BlockNode, DocumentFormat, FontWeight, HashSet, Instant, ListState,
    PreviewApp, PreviewDocument, PreviewRow, StyledText, accept_generation, centered_message,
    current_theme, div, img, minimap, px, render_markdown_block, render_table_row,
    resolve_image_path, rgb, styled_code_runs, styled_inline_runs,
};
use gpui::{list, prelude::*};

#[allow(clippy::too_many_arguments)]
pub(in crate::preview) fn render_document(
    document: Arc<PreviewDocument>,
    list_state: ListState,
    visible_rows: Arc<Vec<usize>>,
    folded: Arc<HashSet<BlockId>>,
    entity: gpui::Entity<PreviewApp>,
    minimap_visible: bool,
    editor_width: f32,
    minimap_width: f32,
    minimap_resize_preview: Option<f32>,
    minimap_thumb_visibility: crate::settings::MinimapThumbVisibility,
    generation: u64,
    presentation_revision: u64,
    opened_at: Instant,
) -> gpui::Div {
    let theme = current_theme();
    let preview_display_map = document.display_map.clone();
    let defer_initial_reveal = minimap_visible
        && !document
            .minimap
            .initial_visible_batch_ready
            .load(std::sync::atomic::Ordering::Acquire);
    let opening_path = document.path.display().to_string();
    let minimap_list_state = list_state.clone();
    let minimap_entity = entity.clone();
    let minimap_resize_entity = entity.clone();
    div()
        .size_full()
        .flex()
        .relative()
        .bg(rgb(theme.background))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .relative()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(px(50.0))
                        .bg(rgb(theme.background_alt))
                        .border_r_1()
                        .border_color(rgb(theme.border)),
                )
                .child({
                    let document = document.clone();
                    let visible_rows = visible_rows.clone();
                    let folded = folded.clone();
                    list(list_state, move |index, _window, _cx| {
                        let actual_index = visible_rows[index];
                        let row = document
                            .projection
                            .source_row(actual_index)
                            .expect("visible preview row maps to the current revision");
                        let display_map = document
                            .display_map
                            .as_ref()
                            .expect("preview display map must exist after loading");
                        let is_heading = display_map.is_heading(actual_index);
                        let is_table_row = display_map.is_table(actual_index);
                        let is_folded = is_heading && folded.contains(&row.block_id);
                        let document_for_click = document.clone();
                        let entity_for_click = entity.clone();
                        div()
                            .id(("preview-row", actual_index))
                            .w_full()
                            .min_h(px(24.0))
                            .when(index == 0, |element| element.pt_1())
                            .when(index + 1 == visible_rows.len(), |element| element.pb_2())
                            .when(is_heading, |element| {
                                element.cursor_pointer().on_click(move |_, _, cx| {
                                    entity_for_click.update(cx, |this, cx| {
                                        this.toggle_fold(row.block_id, &document_for_click);
                                        cx.notify();
                                    });
                                })
                            })
                            .flex()
                            .items_start()
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(50.0))
                                    .pr_3()
                                    .h(px(24.0))
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .text_right()
                                    .font_family("Menlo")
                                    .text_size(px(10.0))
                                    .text_color(rgb(theme.foreground_dim))
                                    .child(if row.show_line_number {
                                        (document.text.line_of_byte(row.content.range.start) + 1)
                                            .to_string()
                                    } else {
                                        String::new()
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .min_h(px(24.0))
                                    .flex()
                                    .items_center()
                                    .pl_3()
                                    .pr_8()
                                    .when(is_table_row, |element| {
                                        element.bg(rgb(current_theme().background_alt))
                                    })
                                    .child(div().w_full().child(
                                        if document.format == DocumentFormat::Markdown {
                                            render_markdown_block(
                                                &document,
                                                actual_index,
                                                &document.markdown_blocks[row.block_id as usize],
                                                {
                                                    let minimap = if minimap_visible {
                                                        minimap_width
                                                    } else {
                                                        0.0
                                                    };
                                                    (editor_width - 110.0 - minimap).max(120.0)
                                                },
                                            )
                                        } else {
                                            render_block(
                                                &document,
                                                actual_index,
                                                row,
                                                &document.blocks.nodes()[row.block_id as usize],
                                                is_folded,
                                                {
                                                    let minimap = if minimap_visible {
                                                        minimap_width
                                                    } else {
                                                        0.0
                                                    };
                                                    (editor_width - 110.0 - minimap).max(120.0)
                                                },
                                            )
                                        },
                                    )),
                            )
                            .into_any()
                    })
                    .flex_1()
                    .w_full()
                }),
        )
        .when_some(
            minimap_visible.then_some(preview_display_map).flatten(),
            |layout, display_map| {
                layout.child(minimap::render(
                    display_map,
                    document.minimap.clone(),
                    visible_rows.clone(),
                    folded.clone(),
                    minimap_list_state,
                    editor_width,
                    minimap_width,
                    minimap_thumb_visibility,
                    generation,
                    presentation_revision,
                    opened_at,
                    move |_source_target, ratio, center, window, cx| {
                        minimap_entity.update(cx, |this, cx| {
                            this.minimap_pending_seek =
                                Some((presentation_revision, ratio, center));
                            if this.minimap_seek_scheduled {
                                return;
                            }
                            this.minimap_seek_scheduled = true;
                            cx.on_next_frame(window, |this, _, cx| {
                                this.minimap_seek_scheduled = false;
                                if let Some((revision, ratio, center)) =
                                    this.minimap_pending_seek.take()
                                    && accept_generation(this.presentation_revision, revision)
                                {
                                    minimap::seek_to_ratio(&this.list_state, ratio, center);
                                    cx.notify();
                                }
                            });
                        });
                    },
                    move |change, _, cx| {
                        minimap_resize_entity.update(cx, |this, cx| {
                            this.change_minimap_width(change, cx);
                        });
                    },
                ))
            },
        )
        .when_some(minimap_resize_preview, |layout, width| {
            layout.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(width))
                    .w(px(1.0))
                    .bg(rgb(theme.heading[0])),
            )
        })
        .when(defer_initial_reveal, |layout| {
            layout.child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .left_0()
                    .child(centered_message("OPENING DOCUMENT", &opening_path)),
            )
        })
}

fn render_block(
    document: &Arc<PreviewDocument>,
    display_row: usize,
    row: PreviewRow,
    block: &BlockNode,
    is_folded: bool,
    available_width: f32,
) -> gpui::Div {
    let theme = current_theme();
    let display_map = document
        .display_map
        .as_ref()
        .expect("preview display map must exist after loading");
    let display_runs = display_map.runs(display_row);
    let row_layout = display_map.layout(display_row);
    if row.blank {
        return div().h(px(row_layout.fixed_height.unwrap_or(row_layout.min_height)));
    }
    let text = display_runs.text.clone();
    let inline = || styled_inline_runs(text.clone(), display_runs.inline_spans.clone());

    match &block.kind {
        BlockKind::BlankLine => {
            div().h(px(row_layout.fixed_height.unwrap_or(row_layout.min_height)))
        }
        BlockKind::Heading { level } => {
            let heading_index = (*level as usize).saturating_sub(1);
            let marker = format!(
                "{} ",
                theme.heading_bullets[heading_index % theme.heading_bullets.len()]
            );
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
                .text_color(rgb(theme.heading[heading_index.min(3)]))
                .child(
                    div()
                        .flex_none()
                        .font_family("Menlo")
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme.heading[heading_index.min(3)]))
                        .child(marker),
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
        BlockKind::Paragraph => {
            let paragraph = div()
                .text_size(px(row_layout.font_size))
                .line_height(px(row_layout.line_height))
                .text_color(rgb(theme.foreground))
                .child(inline());
            #[allow(clippy::if_same_then_else)]
            if row.continuation {
                paragraph
            } else {
                paragraph
            }
        }
        BlockKind::Image { path } => {
            let source = resolve_image_path(&document.path, path);
            let (width, height) = document
                .display_map
                .as_ref()
                .and_then(|map| map.image_size(display_row, available_width))
                .unwrap_or_else(|| (available_width.min(640.0), 240.0));
            div()
                .w_full()
                .pt(px(row_layout.padding_top))
                .pb(px(row_layout.padding_bottom))
                .flex()
                .items_start()
                .justify_start()
                .child(img(source).w(px(width)).h(px(height)))
        }
        BlockKind::Planning => div()
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.date))
            .child(text),
        BlockKind::ListItem => div()
            .pl(px(row_layout.padding_left))
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.foreground))
            .child(inline()),
        BlockKind::FixedWidth => div()
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.code_foreground))
            .child(text),
        BlockKind::FootnoteDefinition => div()
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.link))
            .child(text),
        BlockKind::TableRow => render_table_row(
            &text,
            display_map
                .table_projection(display_row)
                .expect("table row projection must exist"),
        ),
        BlockKind::SourceBlock { .. } => {
            let marker = text.trim_start().to_ascii_lowercase();
            let is_boundary = marker.starts_with("#+begin_") || marker.starts_with("#+end_");
            let content = if is_boundary {
                StyledText::new(text.clone())
            } else {
                styled_code_runs(text.clone(), display_runs.code_spans.clone())
            };
            div()
                .min_h(px(row_layout.min_height))
                .pl(px(row_layout.padding_left))
                .pr(px(row_layout.padding_right))
                .pt(px(row_layout.padding_top))
                .pb(px(row_layout.padding_bottom))
                .bg(rgb(if is_boundary {
                    theme.code_boundary_background
                } else {
                    theme.code_background
                }))
                .text_color(if is_boundary {
                    rgb(theme.code_boundary)
                } else {
                    rgb(theme.code_foreground)
                })
                .font_family("Menlo")
                .text_size(px(row_layout.font_size))
                .line_height(px(row_layout.line_height))
                .child(content)
        }
        BlockKind::ExampleBlock | BlockKind::Raw | BlockKind::ExportBlock { .. } => div()
            .min_h(px(row_layout.min_height))
            .pl(px(row_layout.padding_left))
            .pr(px(row_layout.padding_right))
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .bg(rgb(theme.code_background))
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.code_foreground))
            .child(text),
        BlockKind::QuoteBlock => div()
            .pl(px(row_layout.padding_left))
            .pr(px(row_layout.padding_right))
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .border_l_2()
            .border_color(rgb(theme.heading[1]))
            .text_color(rgb(theme.quote))
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .child(text),
        BlockKind::VerseBlock => div()
            .pl(px(row_layout.padding_left))
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.quote))
            .child(text),
        BlockKind::CenterBlock => div()
            .w_full()
            .text_center()
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.foreground))
            .child(text),
        BlockKind::SpecialBlock { name } => div()
            .min_h(px(row_layout.min_height))
            .pl(px(row_layout.padding_left))
            .pr(px(row_layout.padding_right))
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .bg(rgb(theme.code_background))
            .text_color(rgb(theme.attribute))
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .child(format!("{name}: {text}")),
        BlockKind::Drawer { .. } => div()
            .pl(px(row_layout.padding_left))
            .pr(px(row_layout.padding_right))
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .bg(rgb(theme.background_alt))
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.meta))
            .child(text),
        BlockKind::Keyword => div()
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.meta))
            .child(text),
        BlockKind::Comment | BlockKind::CommentBlock => div(),
        BlockKind::HorizontalRule => div()
            .mt(px(row_layout.margin_top))
            .mb(px(row_layout.margin_bottom))
            .h(px(row_layout.fixed_height.unwrap_or(1.0)))
            .w_full()
            .bg(rgb(theme.border)),
    }
}
