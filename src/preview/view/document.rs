use super::{
    Arc, BlockKind, BlockNode, DocumentFormat, FoldDirection, FoldSegment, FontWeight, Instant,
    PreviewRow, PreviewSnapshot, ReadingInteraction, ReadingRowContext, ReadingRowHost,
    WorkspaceWindow, current_theme, div, img, minimap, px, render_code_row, render_markdown_block,
    render_table_row, resolve_image_path, rgb, styled_inline_runs,
};
use crate::preview::BlockId;
use crate::preview::{
    CodeRowRole, ReadingPreviewPanel, ReadingRenderState,
    display_map::{DisplayRuns, PreviewLineKind},
    layout::{READING_FRAME_MAX_WIDTH, reading_content_width},
    org_line::{CheckboxState, parse_heading},
    projection::{ReadingCodeRow, ReadingListMarker, VisualRowKind},
};
use gpui::{list, prelude::*};

pub(in crate::preview) struct ReadingRenderOptions {
    pub(in crate::preview) minimap_visible: bool,
    pub(in crate::preview) pane_width: f32,
    pub(in crate::preview) minimap_width: f32,
    pub(in crate::preview) minimap_resize_preview: Option<f32>,
    pub(in crate::preview) minimap_thumb_visibility: crate::settings::MinimapThumbVisibility,
    pub(in crate::preview) generation: u64,
    pub(in crate::preview) opened_at: Instant,
}

pub(in crate::preview) fn render_reading_document(
    state: ReadingRenderState,
    panel_entity: gpui::Entity<ReadingPreviewPanel>,
    workspace_entity: gpui::Entity<WorkspaceWindow>,
    options: ReadingRenderOptions,
) -> gpui::Div {
    let ReadingRenderState {
        document,
        minimap_state,
        table_scroll_handles,
        list_state,
        visible_rows,
        fold_markers,
        fold_animation,
        geometry_revision,
        zoom,
        action_states,
        copy_feedback,
    } = state;
    let ReadingRenderOptions {
        minimap_visible,
        pane_width,
        minimap_width,
        minimap_resize_preview,
        minimap_thumb_visibility,
        generation,
        opened_at,
    } = options;
    let theme = current_theme();
    let reading_display_map = document.display_map.clone();
    let minimap_list_state = list_state.clone();
    let minimap_entity = panel_entity.clone();
    let minimap_resize_entity = workspace_entity.clone();
    let rendered_item_count = list_state.item_count();
    let allow_minimap_refinement = fold_animation.is_none();
    let interaction = ReadingInteraction {
        panel: panel_entity.clone(),
        workspace: workspace_entity.clone(),
        action_states,
        copy_feedback,
    };
    // The minimap represents the final semantic projection. The temporary flow segments belong
    // only to the main list; exposing them here causes a second tile refresh when the animation
    // completes and the segment is removed.
    let minimap_rows = visible_rows.clone();
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
                .child({
                    let document = document.clone();
                    let visible_rows = visible_rows.clone();
                    let fold_markers = fold_markers.clone();
                    let fold_animation = fold_animation.clone();
                    let interaction = interaction.clone();
                    list(list_state, move |index, _window, _cx| {
                        let available_width = {
                            let minimap = if minimap_visible { minimap_width } else { 0.0 };
                            reading_content_width(pane_width, minimap)
                        };
                        if let Some(animation) = fold_animation.as_ref()
                            && let Some(shell) = animation.segment_at(index)
                        {
                            return render_fold_shell(
                                document.clone(),
                                animation.direction,
                                animation.progress,
                                shell.clone(),
                                available_width,
                                zoom,
                                table_scroll_handles.clone(),
                            )
                            .into_any_element();
                        }
                        let row_index = fold_animation
                            .as_ref()
                            .map_or(index, |transition| transition.target_index_for_item(index));
                        let actual_index = visible_rows[row_index];
                        let row = document
                            .projection
                            .source_row(actual_index)
                            .expect("visible reading row maps to the current revision");
                        let display_map = document
                            .display_map
                            .as_ref()
                            .expect("reading display map must exist after loading");
                        let is_heading = display_map.is_heading(actual_index);
                        let is_suppressed_marker =
                            fold_animation.as_ref().is_some_and(|animation| {
                                animation.suppressed_markers.contains(&row.block_id)
                            });
                        let is_folded = is_heading
                            && fold_markers.contains(&row.block_id)
                            && !is_suppressed_marker;
                        let entity_for_click = panel_entity.clone();
                        let row_element = render_reading_row(
                            &document,
                            actual_index,
                            row,
                            is_folded,
                            ReadingRowHost {
                                available_width,
                                zoom,
                                table_scroll_handles: &table_scroll_handles,
                                interaction: Some(&interaction),
                            },
                        )
                        .id(("reading-row", actual_index))
                        .when(index == 0, |element| element.pt_1())
                        .when(index + 1 == rendered_item_count, |element| element.pb_2())
                        .when(is_heading, |element| {
                            element.cursor_pointer().on_click(move |_, window, cx| {
                                let viewport_height = f32::from(window.viewport_size().height);
                                entity_for_click.update(cx, |this, cx| {
                                    this.toggle_fold_animated(
                                        row.block_id,
                                        viewport_height,
                                        available_width,
                                        Some(window),
                                        cx,
                                    );
                                    cx.notify();
                                });
                            })
                        });

                        row_element.into_any_element()
                    })
                    .flex_1()
                    .w_full()
                }),
        )
        .when_some(
            minimap_visible.then_some(reading_display_map).flatten(),
            |layout, display_map| {
                layout.child(minimap::render(
                    display_map,
                    minimap_state.clone(),
                    minimap_rows.clone(),
                    fold_markers.clone(),
                    minimap_list_state,
                    pane_width,
                    minimap_width,
                    minimap_thumb_visibility,
                    generation,
                    geometry_revision,
                    allow_minimap_refinement,
                    opened_at,
                    move |_source_target, offset, window, cx| {
                        minimap_entity.update(cx, |this, cx| {
                            this.seek_minimap(geometry_revision, offset, window, cx);
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
}

fn render_fold_shell(
    document: Arc<PreviewSnapshot>,
    direction: FoldDirection,
    progress: f32,
    shell: FoldSegment,
    available_width: f32,
    zoom: f32,
    table_scroll_handles: Arc<std::collections::HashMap<BlockId, gpui::ScrollHandle>>,
) -> gpui::Div {
    let distance = shell.distance;
    let rows = shell.rendered_rows;
    let (shell_height, body_offset) = fold_shell_geometry(direction, progress, distance);
    let rendered_rows = rows
        .iter()
        .copied()
        .map(|row| {
            render_fold_shell_row(&document, row, available_width, zoom, &table_scroll_handles)
        })
        .collect::<Vec<_>>();
    div()
        .relative()
        .w_full()
        .h(px(shell_height))
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(body_offset))
                .children(rendered_rows),
        )
}

fn fold_shell_geometry(direction: FoldDirection, delta: f32, distance: f32) -> (f32, f32) {
    match direction {
        FoldDirection::Collapse => (distance * (1.0 - delta), -distance * delta),
        FoldDirection::Expand => (distance * delta, -distance * (1.0 - delta)),
    }
}

fn render_fold_shell_row(
    document: &Arc<PreviewSnapshot>,
    actual_index: usize,
    available_width: f32,
    zoom: f32,
    table_scroll_handles: &std::collections::HashMap<BlockId, gpui::ScrollHandle>,
) -> gpui::Div {
    let row = document
        .projection
        .source_row(actual_index)
        .expect("fold shell row maps to the current revision");
    render_reading_row(
        document,
        actual_index,
        row,
        false,
        ReadingRowHost {
            available_width,
            zoom,
            table_scroll_handles,
            interaction: None,
        },
    )
}

fn render_reading_row(
    document: &Arc<PreviewSnapshot>,
    actual_index: usize,
    row: PreviewRow,
    is_folded: bool,
    host: ReadingRowHost<'_>,
) -> gpui::Div {
    let ReadingRowHost {
        available_width,
        zoom,
        table_scroll_handles,
        interaction,
    } = host;
    let minimum_height = match &document
        .projection
        .rows
        .get(actual_index)
        .expect("reading row index is in bounds")
        .kind
    {
        VisualRowKind::Table(table) if table.is_separator() => 2.0,
        VisualRowKind::Code(ReadingCodeRow::End) => 0.0,
        _ => 24.0,
    };
    let table_scroll = match &document
        .projection
        .rows
        .get(actual_index)
        .expect("reading row index is in bounds")
        .kind
    {
        VisualRowKind::Table(table) => table_scroll_handles.get(&table.group_id()),
        _ => None,
    };
    let context = ReadingRowContext {
        available_width,
        zoom,
        table_scroll,
    };
    div()
        .w_full()
        .min_h(px(minimum_height))
        .flex()
        .justify_center()
        .items_start()
        .child(
            div()
                .w_full()
                .max_w(px(READING_FRAME_MAX_WIDTH))
                .min_w_0()
                .min_h(px(minimum_height))
                .flex()
                .items_center()
                .px_6()
                .child(
                    div()
                        .w_full()
                        .child(if document.format == DocumentFormat::Markdown {
                            render_markdown_block(
                                document,
                                actual_index,
                                &document.markdown_blocks[row.block_id as usize],
                                is_folded,
                                context,
                                interaction,
                            )
                        } else {
                            render_block(
                                document,
                                actual_index,
                                row,
                                &document.blocks.nodes()[row.block_id as usize],
                                is_folded,
                                context,
                                interaction,
                            )
                        }),
                ),
        )
}

fn render_block(
    document: &Arc<PreviewSnapshot>,
    display_row: usize,
    row: PreviewRow,
    block: &BlockNode,
    is_folded: bool,
    context: ReadingRowContext<'_>,
    interaction: Option<&ReadingInteraction>,
) -> gpui::Div {
    let theme = current_theme();
    let display_map = document
        .display_map
        .as_ref()
        .expect("reading display map must exist after loading");
    let display_runs = display_map.runs(display_row);
    let row_layout = display_map.layout(display_row).scaled(context.zoom);
    if row.blank {
        return div().h(px(row_layout.fixed_height.unwrap_or(row_layout.min_height)));
    }
    let text = display_runs.text.clone();
    let inline = || reading_inline(document, display_row, &display_runs, interaction);
    if matches!(display_map.row_kind(display_row), PreviewLineKind::Caption) {
        return div()
            .w_full()
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .text_center()
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.meta))
            .child(inline());
    }

    match &block.kind {
        BlockKind::BlankLine => {
            div().h(px(row_layout.fixed_height.unwrap_or(row_layout.min_height)))
        }
        BlockKind::Heading { level } => {
            let heading_index = (*level as usize).saturating_sub(1);
            let source = document.text.copy_range(row.content.range);
            let parts = parse_heading(source.trim_end_matches(['\r', '\n']));
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(row_layout.font_size))
                .line_height(px(row_layout.line_height))
                .font_weight(if *level <= 2 {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(rgb(theme.heading[heading_index.min(3)]))
                .children(parts.todo.map(|todo| reading_chip(todo, theme.keyword)))
                .children(
                    parts
                        .priority
                        .map(|priority| reading_chip(format!("P{priority}"), theme.attribute)),
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
                .children(parts.cookie.map(|cookie| reading_chip(cookie, theme.meta)))
                .children(
                    parts
                        .tags
                        .into_iter()
                        .map(|tag| reading_chip(tag, theme.link)),
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
                .and_then(|map| map.image_size(display_row, context.available_width))
                .unwrap_or_else(|| (context.available_width.min(640.0), 240.0));
            let action = document
                .projection
                .rows
                .get(display_row)
                .map(|visual| crate::preview::image_action(document, visual, path.clone()));
            let interaction = interaction.cloned();
            div().w_full().child(
                div()
                    .id(("reading-image", display_row))
                    .pt(px(row_layout.padding_top))
                    .pb(px(row_layout.padding_bottom))
                    .flex()
                    .items_start()
                    .justify_start()
                    .child(img(source).w(px(width)).h(px(height)))
                    .when_some(action.zip(interaction), |element, (action, interaction)| {
                        element.cursor_pointer().on_click(move |_, window, cx| {
                            interaction.workspace.update(cx, |workspace, cx| {
                                workspace.dispatch_preview_action(
                                    action.clone(),
                                    interaction.panel.clone(),
                                    window,
                                    cx,
                                );
                            });
                        })
                    }),
            )
        }
        BlockKind::Planning => div()
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.date))
            .child(text),
        BlockKind::ListItem => render_list_item(
            document,
            display_row,
            inline(),
            row_layout.font_size,
            row_layout.line_height,
            interaction,
        ),
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
        BlockKind::TableRow => display_map.table_projection(display_row).map_or_else(
            || reading_fallback(text.clone(), row_layout.font_size, row_layout.line_height),
            |projection| {
                render_table_row(
                    &text,
                    projection,
                    DocumentFormat::Org,
                    context.available_width,
                    context.zoom,
                    display_row,
                    context.table_scroll,
                )
            },
        ),
        BlockKind::SourceBlock { language } => div()
            .w_full()
            .when(!row.continuation, |element| {
                element.child(reading_code_label(
                    language.as_deref(),
                    crate::preview::code_action(document, display_row),
                    interaction,
                    display_row,
                ))
            })
            .child(render_code_row(
                text,
                display_runs.code_spans,
                row_layout,
                CodeRowRole::Body,
            )),
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

fn reading_chip(text: impl Into<gpui::SharedString>, color: u32) -> gpui::Div {
    let theme = current_theme();
    div()
        .flex_none()
        .px_1()
        .rounded_sm()
        .bg(rgb(theme.background_alt))
        .font_family("Menlo")
        .text_size(px(10.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .child(text.into())
}

pub(super) fn render_list_item(
    document: &Arc<PreviewSnapshot>,
    display_row: usize,
    content: gpui::AnyElement,
    font_size: f32,
    line_height: f32,
    interaction: Option<&ReadingInteraction>,
) -> gpui::Div {
    let theme = current_theme();
    let marker = match &document
        .projection
        .rows
        .get(display_row)
        .expect("reading row index is in bounds")
        .kind
    {
        VisualRowKind::List(marker) => marker,
        _ => {
            return reading_fallback(content, font_size, line_height);
        }
    };
    div()
        .w_full()
        .pl(px(f32::from(marker.indent).min(96.0)))
        .flex()
        .items_start()
        .gap_2()
        .text_size(px(font_size))
        .line_height(px(line_height))
        .text_color(rgb(theme.foreground))
        .child(reading_list_marker(
            document,
            display_row,
            marker,
            interaction,
        ))
        .child(div().flex_1().min_w_0().child(content))
}

fn reading_list_marker(
    document: &PreviewSnapshot,
    display_row: usize,
    marker: &ReadingListMarker,
    interaction: Option<&ReadingInteraction>,
) -> gpui::Stateful<gpui::Div> {
    let theme = current_theme();
    if let Some(state) = &marker.checkbox {
        let action = document
            .projection
            .rows
            .get(display_row)
            .and_then(|row| crate::preview::checkbox_action(document, row));
        let action_state = action.as_ref().and_then(|action| {
            interaction.and_then(|interaction| {
                let identity = action.target().identity();
                interaction
                    .action_states
                    .iter()
                    .find_map(|(candidate, state)| (*candidate == identity).then_some(*state))
            })
        });
        let (label, foreground, background) = match action_state {
            Some(crate::preview::PreviewActionVisualState::Pending) => {
                ("…", theme.foreground_dim, theme.background_alt)
            }
            Some(crate::preview::PreviewActionVisualState::Failed) => {
                ("!", theme.background, theme.keyword)
            }
            None => match state {
                CheckboxState::Empty => ("", theme.foreground_dim, theme.background),
                CheckboxState::Partial => ("−", theme.background, theme.attribute),
                CheckboxState::Checked => ("✓", theme.background, theme.heading[1]),
            },
        };
        let interaction = interaction.cloned();
        return div()
            .id(("reading-checkbox", display_row))
            .mt(px(3.0))
            .size(px(16.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .border_1()
            .border_color(rgb(foreground))
            .bg(rgb(background))
            .font_family("Menlo")
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(foreground))
            .child(label)
            .when_some(action.zip(interaction), |element, (action, interaction)| {
                element.cursor_pointer().on_click(move |_, window, cx| {
                    interaction.workspace.update(cx, |workspace, cx| {
                        workspace.dispatch_preview_action(
                            action.clone(),
                            interaction.panel.clone(),
                            window,
                            cx,
                        );
                    });
                })
            });
    }
    let ordered = marker.marker.ends_with('.') || marker.marker.ends_with(')');
    let label = if ordered {
        marker.marker.to_string()
    } else {
        "•".to_owned()
    };
    let width = reading_marker_width(&label, ordered);
    div()
        .id(("reading-list-marker", display_row))
        .w(px(width))
        .flex_none()
        .whitespace_nowrap()
        .when(ordered, |element| element.pr_1().text_right())
        .when(!ordered, |element| element.text_center())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme.heading[2]))
        .child(label)
}

fn reading_marker_width(label: &str, ordered: bool) -> f32 {
    if ordered {
        (label.chars().count() as f32 * 9.0 + 8.0).max(30.0)
    } else {
        18.0
    }
}

pub(super) fn reading_code_label(
    language: Option<&str>,
    action: Option<crate::preview::PreviewAction>,
    interaction: Option<&ReadingInteraction>,
    display_row: usize,
) -> gpui::Stateful<gpui::Div> {
    let theme = current_theme();
    let copy_feedback = action.as_ref().and_then(|action| {
        interaction.and_then(|interaction| {
            interaction
                .copy_feedback
                .filter(|(range, _)| *range == action.target().source_range)
                .map(|(_, state)| state)
        })
    });
    let (copy_label, copy_color) = match copy_feedback {
        Some(crate::preview::CopyFeedbackState::Succeeded) => ("Copied ✓", theme.heading[1]),
        Some(crate::preview::CopyFeedbackState::Failed) => ("Copy failed", theme.keyword),
        None => ("Copy", theme.meta),
    };
    let interaction = interaction.cloned();
    div()
        .id(("reading-code-label", display_row))
        .w_full()
        .h(px(22.0))
        .px_3()
        .flex()
        .items_center()
        .border_l_2()
        .border_color(rgb(theme.code_block_accent))
        .bg(rgb(theme.code_boundary_background))
        .font_family("Menlo")
        .text_size(px(10.0))
        .text_color(rgb(theme.meta))
        .justify_between()
        .child(language.unwrap_or("code").to_owned())
        .when_some(action.zip(interaction), |element, (action, interaction)| {
            element.child(
                div()
                    .id(("reading-copy-code", display_row))
                    .w(px(76.0))
                    .px_2()
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(rgb(copy_color))
                    .hover(|element| element.bg(rgb(theme.background_alt)))
                    .active(|element| element.bg(rgb(theme.code_block_accent)).opacity(0.62))
                    .when(copy_feedback.is_some(), |element| {
                        element.bg(rgb(theme.background_alt))
                    })
                    .child(copy_label)
                    .on_click(move |_, window, cx| {
                        interaction.workspace.update(cx, |workspace, cx| {
                            workspace.dispatch_preview_action(
                                action.clone(),
                                interaction.panel.clone(),
                                window,
                                cx,
                            );
                        });
                    }),
            )
        })
}

pub(super) fn reading_inline(
    document: &PreviewSnapshot,
    display_row: usize,
    runs: &DisplayRuns,
    interaction: Option<&ReadingInteraction>,
) -> gpui::AnyElement {
    let styled = styled_inline_runs(runs.text.clone(), runs.inline_spans.clone());
    let Some(interaction) = interaction.cloned() else {
        return styled.into_any_element();
    };
    if runs.links.is_empty() {
        return styled.into_any_element();
    }
    let Some(row) = document.projection.rows.get(display_row) else {
        return styled.into_any_element();
    };
    let target = crate::preview::source_action_target(document, row);
    let ranges = runs
        .links
        .iter()
        .map(|link| link.range.clone())
        .collect::<Vec<_>>();
    let actions = runs
        .links
        .iter()
        .map(|link| crate::preview::PreviewAction::OpenLink {
            target: target.clone(),
            destination: link.destination.clone(),
        })
        .collect::<Vec<_>>();
    gpui::InteractiveText::new(("reading-inline", display_row), styled)
        .on_click(ranges, move |index, window, cx| {
            cx.stop_propagation();
            let Some(action) = actions.get(index).cloned() else {
                return;
            };
            interaction.workspace.update(cx, |workspace, cx| {
                workspace.dispatch_preview_action(action, interaction.panel.clone(), window, cx);
            });
        })
        .into_any_element()
}

pub(super) fn reading_fallback(
    content: impl IntoElement,
    font_size: f32,
    line_height: f32,
) -> gpui::Div {
    div()
        .text_size(px(font_size))
        .line_height(px(line_height))
        .text_color(rgb(current_theme().foreground))
        .child(content)
}

#[cfg(test)]
mod animation_tests {
    use super::{FoldDirection, fold_shell_geometry, reading_marker_width};

    #[test]
    fn shell_height_and_body_offset_share_one_linear_progress() {
        assert_eq!(
            fold_shell_geometry(FoldDirection::Collapse, 0.0, 96.0),
            (96.0, 0.0)
        );
        assert_eq!(
            fold_shell_geometry(FoldDirection::Collapse, 0.5, 96.0),
            (48.0, -48.0)
        );
        assert_eq!(
            fold_shell_geometry(FoldDirection::Expand, 0.0, 96.0),
            (0.0, -96.0)
        );
        assert_eq!(
            fold_shell_geometry(FoldDirection::Expand, 0.5, 96.0),
            (48.0, -48.0)
        );
        assert_eq!(
            fold_shell_geometry(FoldDirection::Expand, 1.0, 96.0),
            (96.0, 0.0)
        );
    }

    #[test]
    fn ordered_list_marker_has_room_for_punctuation_without_wrapping() {
        assert_eq!(reading_marker_width("•", false), 18.0);
        assert!(reading_marker_width("1.", true) >= 30.0);
        assert!(reading_marker_width("100.", true) > reading_marker_width("1.", true));
    }
}
