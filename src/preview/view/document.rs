use super::{
    Arc, BlockKind, BlockNode, DocumentFormat, FoldDirection, FoldSegment, FontWeight, Instant,
    PreviewRow, PreviewSnapshot, ReadingActionDispatcher, ReadingInteraction,
    ReadingMinimapWidthDispatcher, ReadingRowContext, ReadingRowHost, div, img, minimap, px,
    render_code_row, render_markdown_block, render_table_row, resolve_image_path, rgb,
    styled_inline_runs,
};
use crate::preview::BlockId;
use crate::preview::{
    CodeRowRole, ReadingPreviewPanel, ReadingRenderState,
    diagram::DiagramProjection,
    display_map::{DisplayRuns, PreviewLineKind},
    layout::reading_content_width,
    org_line::CheckboxState,
    projection::{ReadingCodeRow, ReadingListMarker, VisualRowKind},
    style::CodeBlockVariant,
};
use gpui::{list, prelude::*};

pub(crate) struct ReadingRenderOptions {
    pub(crate) minimap_visible: bool,
    pub(crate) minimap_reveal: f32,
    pub(crate) pane_width: f32,
    pub(crate) minimap_width: f32,
    pub(crate) minimap_resize_preview: Option<f32>,
    pub(crate) minimap_thumb_visibility: crate::settings::MinimapThumbVisibility,
    pub(crate) generation: u64,
    pub(crate) opened_at: Instant,
    pub(crate) style: super::PreviewStyle,
    pub(crate) dispatch_action: ReadingActionDispatcher,
    pub(crate) change_minimap_width: ReadingMinimapWidthDispatcher,
}

pub(crate) fn render_reading_document(
    state: ReadingRenderState,
    panel_entity: gpui::Entity<ReadingPreviewPanel>,
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
        text_selection,
    } = state;
    let ReadingRenderOptions {
        minimap_visible,
        minimap_reveal,
        pane_width,
        minimap_width,
        minimap_resize_preview,
        minimap_thumb_visibility,
        generation,
        opened_at,
        style,
        dispatch_action,
        change_minimap_width,
    } = options;
    let (minimap_layout_width, minimap_visual_width) =
        crate::motion::sliding_panel_widths(minimap_width, minimap_visible, minimap_reveal);
    let minimap_interactive = minimap_visible && minimap_reveal >= 1.0;
    let palette = style.palette;
    let reading_display_map = document.display_map.clone();
    let minimap_list_state = list_state.clone();
    let minimap_entity = panel_entity.clone();
    let allow_minimap_refinement = fold_animation.is_none();
    let interaction = ReadingInteraction {
        panel: panel_entity.clone(),
        dispatch: dispatch_action,
        action_states,
        copy_feedback,
        text_selection,
        row_bounds: None,
    };
    // The minimap represents the final semantic projection. The temporary flow segments belong
    // only to the main list; exposing them here causes a second tile refresh when the animation
    // completes and the segment is removed.
    let minimap_rows = visible_rows.clone();
    let clear_selection_panel = panel_entity.clone();
    let finish_selection_panel = panel_entity.clone();
    div()
        .size_full()
        .flex()
        .relative()
        .bg(rgb(palette.background))
        .font_family(style.typography.body_family)
        .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
            clear_selection_panel.update(cx, |panel, cx| {
                if panel.clear_text_selection_unless_dragging() {
                    cx.notify();
                }
            });
        })
        .on_mouse_up(gpui::MouseButton::Left, move |_, _, cx| {
            finish_selection_panel.update(cx, |panel, cx| {
                if panel.text_selection_pending() {
                    panel.finish_text_selection();
                    cx.notify();
                }
            });
        })
        .child(
            div()
                .flex_none()
                .w(px((pane_width - minimap_layout_width).max(0.0)))
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
                        let available_width =
                            { reading_content_width(pane_width, minimap_layout_width, style) };
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
                                style,
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
                        let row_interaction =
                            interaction.scoped_to_row(super::ReadingRowBounds::default());
                        let row_element = render_reading_row(
                            &document,
                            actual_index,
                            row,
                            is_folded,
                            ReadingRowHost {
                                available_width,
                                zoom,
                                style,
                                table_scroll_handles: &table_scroll_handles,
                                interaction: Some(&row_interaction),
                                extra_bottom_padding: row_index + 1 == visible_rows.len()
                                    && actual_index + 1 < document.projection.rows.len(),
                            },
                        )
                        .id(("reading-row", actual_index))
                        .when(is_heading, |element| {
                            element.cursor_pointer().on_click(move |_, window, cx| {
                                let viewport_height = f32::from(window.viewport_size().height);
                                entity_for_click.update(cx, |this, cx| {
                                    if this.text_selection_suppresses_click() {
                                        return;
                                    }
                                    this.toggle_fold_animated(
                                        row.block_id,
                                        viewport_height,
                                        available_width,
                                        style,
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
            (minimap_visual_width > 0.0)
                .then_some(reading_display_map)
                .flatten(),
            |layout, display_map| {
                layout.child(
                    div()
                        .absolute()
                        .right_0()
                        .top_0()
                        .bottom_0()
                        .w(px(minimap_visual_width))
                        .overflow_hidden()
                        .child(minimap::render(
                            document.clone(),
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
                            zoom,
                            style,
                            allow_minimap_refinement,
                            opened_at,
                            move |_source_target, offset, window, cx| {
                                minimap_entity.update(cx, |this, cx| {
                                    this.seek_minimap(geometry_revision, offset, window, cx);
                                });
                            },
                            move |change, _, cx| change_minimap_width(change, cx),
                        ))
                        .when(!minimap_interactive, |minimap| {
                            minimap.child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .right_0()
                                    .bottom_0()
                                    .left_0()
                                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation();
                                    })
                                    .on_scroll_wheel(|_, _, cx| {
                                        cx.stop_propagation();
                                    }),
                            )
                        }),
                )
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
                    .bg(rgb(palette.accent)),
            )
        })
}

#[allow(clippy::too_many_arguments)]
fn render_fold_shell(
    document: Arc<PreviewSnapshot>,
    direction: FoldDirection,
    progress: f32,
    shell: FoldSegment,
    available_width: f32,
    zoom: f32,
    style: super::PreviewStyle,
    table_scroll_handles: Arc<std::collections::HashMap<BlockId, gpui::ScrollHandle>>,
) -> gpui::Div {
    let distance = shell.distance;
    let rows = shell.rendered_rows;
    let (shell_height, body_offset) = fold_shell_geometry(direction, progress, distance);
    let rendered_rows = rows
        .iter()
        .copied()
        .map(|row| {
            render_fold_shell_row(
                &document,
                row,
                available_width,
                zoom,
                style,
                &table_scroll_handles,
            )
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
    let delta = crate::motion::FOLD_MOTION.ease(delta);
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
    style: super::PreviewStyle,
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
            style,
            table_scroll_handles,
            interaction: None,
            extra_bottom_padding: false,
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
        style,
        table_scroll_handles,
        interaction,
        extra_bottom_padding,
    } = host;
    let display_map = document
        .display_map
        .as_ref()
        .expect("reading display map must exist after loading");
    let mut row_layout = display_map.layout(actual_index, style);
    if extra_bottom_padding {
        row_layout.margin_bottom += style.spacing.content_padding_bottom;
    }
    let row_layout = row_layout.scaled(zoom);
    let content_minimum_height = match &document
        .projection
        .rows
        .get(actual_index)
        .expect("reading row index is in bounds")
        .kind
    {
        VisualRowKind::Table(table) if table.is_separator() => 2.0,
        VisualRowKind::Code(ReadingCodeRow::End) => 0.0,
        _ => row_layout.min_height,
    };
    let minimum_height = content_minimum_height + row_layout.margin_top + row_layout.margin_bottom;
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
        style,
        table_scroll,
    };
    let row_bounds = interaction.and_then(|interaction| interaction.row_bounds.clone());
    let content = div()
        .w_full()
        .min_h(px(minimum_height))
        .flex()
        .justify_center()
        .items_start()
        .child(
            div()
                .w_full()
                .max_w(px(available_width + style.spacing.horizontal_padding))
                .min_w_0()
                .min_h(px(minimum_height))
                .flex()
                .items_center()
                .px(px(style.spacing.horizontal_padding * 0.5))
                .pt(px(row_layout.margin_top))
                .pb(px(row_layout.margin_bottom))
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
        );
    match row_bounds {
        Some(row_bounds) => div()
            .w_full()
            .child(super::ReadingRowScope::new(row_bounds, content)),
        None => content,
    }
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
    let style = context.style;
    let palette = style.palette;
    let display_map = document
        .display_map
        .as_ref()
        .expect("reading display map must exist after loading");
    let display_runs = display_map.runs(display_row);
    let row_layout = display_map.layout(display_row, style).scaled(context.zoom);
    if row.blank {
        return selectable_blank_row(
            display_row,
            row_layout.fixed_height.unwrap_or(row_layout.min_height),
            interaction,
        );
    }
    let text = display_runs.text.clone();
    let inline = || reading_inline(document, display_row, &display_runs, style, interaction);
    if matches!(display_map.row_kind(display_row), PreviewLineKind::Caption) {
        return div()
            .w_full()
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .text_center()
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(palette.meta))
            .child(inline());
    }

    match &block.kind {
        BlockKind::BlankLine => selectable_blank_row(
            display_row,
            row_layout.fixed_height.unwrap_or(row_layout.min_height),
            interaction,
        ),
        BlockKind::Heading { level } => {
            let heading_index = (*level as usize).saturating_sub(1);
            let source = document.text.copy_range(row.content.range);
            let parts = super::super::org_line::parse_heading_with_config(
                source.trim_end_matches(['\r', '\n']),
                document
                    .semantic
                    .as_ref()
                    .map(|semantic| semantic.config.as_ref()),
            );
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
                .text_color(rgb(palette.heading[heading_index.min(3)]))
                .children(
                    parts
                        .todo
                        .map(|todo| reading_chip(todo, palette.keyword, style, context.zoom)),
                )
                .children(parts.priority.map(|priority| {
                    reading_chip(
                        format!("P{priority}"),
                        palette.attribute,
                        style,
                        context.zoom,
                    )
                }))
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
                                    .text_color(rgb(palette.keyword))
                                    .child("..."),
                            )
                        }),
                )
                .children(
                    parts
                        .cookie
                        .map(|cookie| reading_chip(cookie, palette.meta, style, context.zoom)),
                )
                .children(
                    parts
                        .tags
                        .into_iter()
                        .map(|tag| reading_chip(tag, palette.link, style, context.zoom)),
                )
        }
        BlockKind::Paragraph => {
            let paragraph = div()
                .text_size(px(row_layout.font_size))
                .line_height(px(row_layout.line_height))
                .text_color(rgb(palette.foreground))
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
                    .child(
                        img(source)
                            .w(px(width))
                            .h(px(height))
                            .rounded(px(style.spacing.radius)),
                    )
                    .when_some(action.zip(interaction), |element, (action, interaction)| {
                        element.cursor_pointer().on_click(move |_, window, cx| {
                            (interaction.dispatch)(
                                action.clone(),
                                interaction.panel.clone(),
                                window,
                                cx,
                            );
                        })
                    }),
            )
        }
        BlockKind::Planning => div()
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(palette.date))
            .child(selectable_plain_text(text, display_row, interaction)),
        BlockKind::ListItem => render_list_item(
            document,
            display_row,
            inline(),
            row_layout.font_size,
            row_layout.line_height,
            style,
            interaction,
        ),
        BlockKind::FixedWidth => div()
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(palette.code_foreground))
            .child(selectable_plain_text(text, display_row, interaction)),
        BlockKind::FootnoteDefinition => div()
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(palette.link))
            .child(selectable_plain_text(text, display_row, interaction)),
        BlockKind::TableRow => display_map.table_projection(display_row).map_or_else(
            || {
                reading_fallback(
                    text.clone(),
                    row_layout.font_size,
                    row_layout.line_height,
                    style,
                )
            },
            |projection| {
                let table_layout = display_map
                    .reading_table_layout(display_row, context.available_width, context.zoom, style)
                    .expect("table row has a resolved layout");
                render_table_row(
                    &text,
                    projection,
                    DocumentFormat::Org,
                    &table_layout,
                    context.zoom,
                    display_row,
                    context.table_scroll,
                    style,
                    interaction,
                )
            },
        ),
        BlockKind::SourceBlock { language } => {
            if let Some(VisualRowKind::Diagram(diagram)) = document
                .projection
                .rows
                .get(display_row)
                .map(|row| &row.kind)
            {
                render_diagram(
                    document,
                    display_row,
                    language.as_deref(),
                    diagram,
                    context,
                    interaction,
                )
            } else {
                div()
                    .w_full()
                    .when(!row.continuation, |element| {
                        element.child(reading_code_label(
                            language.as_deref(),
                            crate::preview::code_action(document, display_row),
                            interaction,
                            display_row,
                            false,
                            style,
                        ))
                    })
                    .child(render_code_row(
                        text,
                        display_runs.code_spans,
                        row_layout,
                        CodeRowRole::Body,
                        document
                            .projection
                            .rows
                            .get(display_row + 1)
                            .is_none_or(|next| next.block_id != row.block_id),
                        style,
                        interaction.map(|interaction| (display_row, interaction)),
                    ))
            }
        }
        BlockKind::ExampleBlock | BlockKind::Raw | BlockKind::ExportBlock { .. } => div()
            .min_h(px(row_layout.min_height))
            .pl(px(row_layout.padding_left))
            .pr(px(row_layout.padding_right))
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .bg(rgb(palette.code_background))
            .font_family(style.typography.code_family)
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(palette.code_foreground))
            .child(selectable_plain_text(text, display_row, interaction)),
        BlockKind::QuoteBlock => div()
            .pl(px(row_layout.padding_left))
            .pr(px(row_layout.padding_right))
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .when(style.spacing.quote_line_width >= 4.0, |element| {
                element.border_l_4()
            })
            .when(style.spacing.quote_line_width < 4.0, |element| {
                element.border_l_2()
            })
            .border_color(rgb(palette.quote_border))
            .text_color(rgb(palette.quote))
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .child(selectable_plain_text(text, display_row, interaction)),
        BlockKind::VerseBlock => div()
            .pl(px(row_layout.padding_left))
            .font_family(style.typography.code_family)
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(palette.quote))
            .child(selectable_plain_text(text, display_row, interaction)),
        BlockKind::CenterBlock => div()
            .w_full()
            .text_center()
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(palette.foreground))
            .child(selectable_plain_text(text, display_row, interaction)),
        BlockKind::SpecialBlock { name } => div()
            .min_h(px(row_layout.min_height))
            .pl(px(row_layout.padding_left))
            .pr(px(row_layout.padding_right))
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .bg(rgb(palette.code_background))
            .text_color(rgb(palette.attribute))
            .font_family(style.typography.code_family)
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .child(format!("{name}: {text}")),
        BlockKind::Drawer { .. } => div()
            .pl(px(row_layout.padding_left))
            .pr(px(row_layout.padding_right))
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .bg(rgb(palette.surface))
            .font_family(style.typography.code_family)
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(palette.meta))
            .child(selectable_plain_text(text, display_row, interaction)),
        BlockKind::Keyword => div()
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .font_family(style.typography.code_family)
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(palette.meta))
            .child(selectable_plain_text(text, display_row, interaction)),
        BlockKind::Comment | BlockKind::CommentBlock => div(),
        BlockKind::HorizontalRule => div()
            .h(px(row_layout.fixed_height.unwrap_or(1.0)))
            .w_full()
            .bg(rgb(palette.border)),
    }
}

pub(super) fn render_diagram(
    document: &PreviewSnapshot,
    display_row: usize,
    language: Option<&str>,
    diagram: &DiagramProjection,
    context: ReadingRowContext<'_>,
    interaction: Option<&ReadingInteraction>,
) -> gpui::Div {
    let style = context.style;
    let palette = style.palette;
    let label = reading_code_label(
        language,
        crate::preview::code_action(document, display_row),
        interaction,
        display_row,
        false,
        style,
    );
    let body = match diagram {
        DiagramProjection::Ready {
            image, warnings, ..
        } => {
            let (width, height) = document
                .display_map
                .as_ref()
                .and_then(|map| map.image_size(display_row, context.available_width))
                .unwrap_or((context.available_width.min(640.0), 240.0));
            div()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .p_3()
                .bg(rgb(palette.surface_elevated))
                .border_1()
                .border_color(rgb(palette.border))
                .rounded_b(px(style.spacing.radius))
                .child(img(image.clone()).w(px(width)).h(px(height)))
                .children(warnings.first().map(|warning| {
                    div()
                        .w_full()
                        .text_size(px(11.0))
                        .text_color(rgb(palette.meta))
                        .child(warning.to_string())
                }))
        }
        DiagramProjection::Error { diagnostics } => div()
            .w_full()
            .min_h(px(72.0))
            .p_3()
            .flex()
            .flex_col()
            .gap_1()
            .bg(rgb(palette.code_background))
            .border_1()
            .border_color(rgb(palette.keyword))
            .rounded_b(px(style.spacing.radius))
            .text_size(px(12.0))
            .line_height(px(18.0))
            .text_color(rgb(palette.keyword))
            .child("PlantUML render failed")
            .children(
                diagnostics
                    .iter()
                    .take(3)
                    .map(|diagnostic| div().child(diagnostic.to_string())),
            ),
    };
    div().w_full().child(label).child(body)
}

fn reading_chip(
    text: impl Into<gpui::SharedString>,
    color: u32,
    style: super::PreviewStyle,
    zoom: f32,
) -> gpui::Div {
    div()
        .flex_none()
        .px(px(4.0 * zoom))
        .rounded(px((style.spacing.radius * zoom).clamp(2.0, 18.0)))
        .bg(rgb(style.palette.surface))
        .font_family(style.typography.code_family)
        .text_size(px(10.0 * zoom))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .child(text.into())
}

pub(crate) fn render_list_item(
    document: &Arc<PreviewSnapshot>,
    display_row: usize,
    content: gpui::AnyElement,
    font_size: f32,
    line_height: f32,
    style: super::PreviewStyle,
    interaction: Option<&ReadingInteraction>,
) -> gpui::Div {
    let palette = style.palette;
    let marker = match &document
        .projection
        .rows
        .get(display_row)
        .expect("reading row index is in bounds")
        .kind
    {
        VisualRowKind::List(marker) => marker,
        _ => {
            return reading_fallback(content, font_size, line_height, style);
        }
    };
    let scale = font_size / style.typography.body_size.max(1.0);
    div()
        .w_full()
        .pl(px(f32::from(marker.indent).min(96.0) * scale))
        .flex()
        .items_start()
        .gap(px(8.0 * scale))
        .text_size(px(font_size))
        .line_height(px(line_height))
        .text_color(rgb(palette.foreground))
        .child(reading_list_marker(
            document,
            display_row,
            marker,
            style,
            scale,
            interaction,
        ))
        .child(div().flex_1().min_w_0().child(content))
}

fn reading_list_marker(
    document: &PreviewSnapshot,
    display_row: usize,
    marker: &ReadingListMarker,
    style: super::PreviewStyle,
    scale: f32,
    interaction: Option<&ReadingInteraction>,
) -> gpui::Stateful<gpui::Div> {
    let palette = style.palette;
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
                ("…", palette.foreground_dim, palette.surface)
            }
            Some(crate::preview::PreviewActionVisualState::Failed) => {
                ("!", palette.accent_contrast, palette.keyword)
            }
            None => match state {
                CheckboxState::Empty => ("", palette.foreground_dim, palette.background),
                CheckboxState::Partial => ("−", palette.accent_contrast, palette.attribute),
                CheckboxState::Checked => ("✓", palette.accent_contrast, palette.accent),
            },
        };
        let interaction = interaction.cloned();
        let checkbox_size = (style.spacing.checkbox_size * scale).max(14.0);
        return div()
            .id(("reading-checkbox", display_row))
            .mt(px(3.0 * scale))
            .size(px(checkbox_size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px((style.spacing.radius * scale).clamp(2.0, 12.0)))
            .border_1()
            .border_color(rgb(foreground))
            .bg(rgb(background))
            .font_family(style.typography.code_family)
            .text_size(px((11.0 * scale).max(5.0)))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(foreground))
            .child(label)
            .when_some(action.zip(interaction), |element, (action, interaction)| {
                element.cursor_pointer().on_click(move |_, window, cx| {
                    (interaction.dispatch)(action.clone(), interaction.panel.clone(), window, cx);
                })
            });
    }
    let ordered = marker.marker.ends_with('.') || marker.marker.ends_with(')');
    let prefix = marker
        .selection_prefix()
        .expect("non-checkbox list markers have a selectable prefix");
    let label = prefix.trim_end();
    let width = reading_marker_width(label, ordered) * scale;
    let marker_content = interaction.map_or_else(
        || gpui::StyledText::new(prefix.clone()).into_any_element(),
        |interaction| {
            let content_len = document
                .display_map
                .as_ref()
                .map_or(0, |display_map| display_map.runs(display_row).text.len());
            let row_text_len = prefix.len() + content_len;
            let selection =
                reading_segment_selection(interaction, display_row, row_text_len, 0..prefix.len());
            super::SelectableReadingText::new(
                ("reading-list-prefix", display_row),
                gpui::StyledText::new(prefix.clone()),
                interaction.panel.clone(),
                display_row,
                selection,
            )
            .with_row_text_len(row_text_len)
            .restrict_drag_to_bounds()
            .into_any_element()
        },
    );
    div()
        .id(("reading-list-marker", display_row))
        .w(px(width))
        .flex_none()
        .whitespace_nowrap()
        .when(ordered, |element| element.pr_1().text_right())
        .when(!ordered, |element| element.text_center())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.accent_text))
        .child(marker_content)
}

fn reading_marker_width(label: &str, ordered: bool) -> f32 {
    if ordered {
        (label.chars().count() as f32 * 9.0 + 8.0).max(30.0)
    } else {
        18.0
    }
}

pub(crate) fn reading_code_label(
    language: Option<&str>,
    action: Option<crate::preview::PreviewAction>,
    interaction: Option<&ReadingInteraction>,
    display_row: usize,
    card_bottom: bool,
    style: super::PreviewStyle,
) -> gpui::Stateful<gpui::Div> {
    let palette = style.palette;
    let copy_feedback = action.as_ref().and_then(|action| {
        interaction.and_then(|interaction| {
            interaction
                .copy_feedback
                .filter(|(range, _)| *range == action.target().source_range)
                .map(|(_, state)| state)
        })
    });
    let (copy_label, copy_color) = match copy_feedback {
        Some(crate::preview::CopyFeedbackState::Succeeded) => ("Copied ✓", palette.accent),
        Some(crate::preview::CopyFeedbackState::Failed) => ("Copy failed", palette.keyword),
        None => ("Copy", palette.meta),
    };
    let interaction = interaction.cloned();
    div()
        .id(("reading-code-label", display_row))
        .w_full()
        .h(px(22.0))
        .px_3()
        .flex()
        .items_center()
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
                    .border_1()
                    .border_color(rgb(palette.border))
                    .when(!card_bottom, |element| {
                        element.border_b_0().rounded_t(px(style.spacing.radius))
                    })
                    .when(card_bottom, |element| {
                        element.rounded(px(style.spacing.radius))
                    })
            },
        )
        .bg(rgb(
            if style.variants.code_block == CodeBlockVariant::Card {
                palette.surface_elevated
            } else {
                palette.code_boundary_background
            },
        ))
        .font_family(style.typography.code_family)
        .text_size(px(10.0))
        .text_color(rgb(palette.meta))
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
                    .hover(|element| element.bg(rgb(palette.surface)))
                    .active(|element| element.bg(rgb(palette.code_block_accent)).opacity(0.62))
                    .when(copy_feedback.is_some(), |element| {
                        element.bg(rgb(palette.surface))
                    })
                    .child(copy_label)
                    .on_click(move |_, window, cx| {
                        (interaction.dispatch)(
                            action.clone(),
                            interaction.panel.clone(),
                            window,
                            cx,
                        );
                    }),
            )
        })
}

pub(crate) fn reading_inline(
    document: &PreviewSnapshot,
    display_row: usize,
    runs: &DisplayRuns,
    style: super::PreviewStyle,
    interaction: Option<&ReadingInteraction>,
) -> gpui::AnyElement {
    let styled = styled_inline_runs(runs.text.clone(), runs.inline_spans.clone(), style);
    let Some(interaction) = interaction.cloned() else {
        return styled.into_any_element();
    };
    let text_offset = document
        .projection
        .rows
        .get(display_row)
        .and_then(|row| match &row.kind {
            VisualRowKind::List(marker) => marker.selection_prefix(),
            _ => None,
        })
        .map_or(0, |prefix| prefix.len());
    let row_text_len = text_offset + runs.text.len();
    let selection = reading_segment_selection(
        &interaction,
        display_row,
        row_text_len,
        text_offset..row_text_len,
    );
    let selectable = super::SelectableReadingText::new(
        ("reading-inline", display_row),
        styled,
        interaction.panel.clone(),
        display_row,
        selection,
    )
    .with_text_offset(text_offset)
    .with_row_text_len(row_text_len)
    .with_row_bounds(interaction.row_bounds.clone());
    if runs.links.is_empty() {
        return selectable.into_any_element();
    }
    let Some(row) = document.projection.rows.get(display_row) else {
        return selectable.into_any_element();
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
    selectable
        .on_click(ranges, move |index, window, cx| {
            let Some(action) = actions.get(index).cloned() else {
                return;
            };
            (interaction.dispatch)(action, interaction.panel.clone(), window, cx);
        })
        .into_any_element()
}

pub(in crate::preview) fn reading_row_selection(
    interaction: &ReadingInteraction,
    row: usize,
    text_len: usize,
) -> Option<(std::ops::Range<usize>, bool)> {
    let (start, end) = interaction.text_selection?;
    if row < start.row || row > end.row {
        return None;
    }
    let range_start = if row == start.row { start.offset } else { 0 }.min(text_len);
    let range_end = if row == end.row { end.offset } else { text_len }.min(text_len);
    let include_newline = row < end.row;
    (range_start < range_end || include_newline)
        .then_some((range_start..range_end, include_newline))
}

fn reading_segment_selection(
    interaction: &ReadingInteraction,
    row: usize,
    row_text_len: usize,
    segment: std::ops::Range<usize>,
) -> Option<(std::ops::Range<usize>, bool)> {
    reading_row_selection(interaction, row, row_text_len).and_then(
        |(selection, include_newline)| {
            let start = selection.start.max(segment.start);
            let end = selection.end.min(segment.end);
            let owns_newline =
                include_newline && selection.end == row_text_len && segment.end == row_text_len;
            (start < end || (owns_newline && start == end))
                .then_some((start - segment.start..end - segment.start, owns_newline))
        },
    )
}

pub(super) fn selectable_blank_row(
    display_row: usize,
    height: f32,
    interaction: Option<&ReadingInteraction>,
) -> gpui::Div {
    let Some(interaction) = interaction else {
        return div().w_full().h(px(height));
    };
    let selection = reading_row_selection(interaction, display_row, 0);
    div().w_full().h(px(height)).child(
        super::SelectableReadingText::new(
            ("reading-blank", display_row),
            gpui::StyledText::new(""),
            interaction.panel.clone(),
            display_row,
            selection,
        )
        .with_row_bounds(interaction.row_bounds.clone())
        .with_minimum_height(height),
    )
}

fn selectable_plain_text(
    text: gpui::SharedString,
    display_row: usize,
    interaction: Option<&ReadingInteraction>,
) -> gpui::AnyElement {
    let Some(interaction) = interaction else {
        return gpui::StyledText::new(text).into_any_element();
    };
    let selection = reading_row_selection(interaction, display_row, text.len());
    super::SelectableReadingText::new(
        ("reading-plain", display_row),
        gpui::StyledText::new(text),
        interaction.panel.clone(),
        display_row,
        selection,
    )
    .with_row_bounds(interaction.row_bounds.clone())
    .into_any_element()
}

pub(crate) fn reading_fallback(
    content: impl IntoElement,
    font_size: f32,
    line_height: f32,
    style: super::PreviewStyle,
) -> gpui::Div {
    div()
        .text_size(px(font_size))
        .line_height(px(line_height))
        .text_color(rgb(style.palette.foreground))
        .child(content)
}

#[cfg(test)]
mod animation_tests {
    use super::{FoldDirection, fold_shell_geometry, reading_marker_width};

    #[test]
    fn shell_height_and_body_offset_share_one_eased_progress() {
        assert_eq!(
            fold_shell_geometry(FoldDirection::Collapse, 0.0, 96.0),
            (96.0, 0.0)
        );
        let halfway = crate::motion::FOLD_MOTION.ease(0.5);
        assert_eq!(
            fold_shell_geometry(FoldDirection::Collapse, 0.5, 96.0),
            (96.0 * (1.0 - halfway), -96.0 * halfway)
        );
        assert_eq!(
            fold_shell_geometry(FoldDirection::Expand, 0.0, 96.0),
            (0.0, -96.0)
        );
        assert_eq!(
            fold_shell_geometry(FoldDirection::Expand, 0.5, 96.0),
            (96.0 * halfway, -96.0 * (1.0 - halfway))
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
