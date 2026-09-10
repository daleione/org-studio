use gpui::{Entity, MouseButton, ParentElement, Styled, div, prelude::*, px, rgb};

use crate::{
    app::WorkspaceWindow, i18n::Language, settings::StatusLineSettings, theme::current_theme,
};

use super::{
    STATUS_LINE_HEIGHT, info_text,
    model::{
        CONFIGURABLE_SEGMENTS, StatusLineSnapshot, StatusPopover, StatusPopoverContent,
        StatusSegment,
    },
    render_document_statistics, segment_title,
};

pub(crate) fn render_status_popover(
    popover: StatusPopover,
    snapshot: Option<&StatusLineSnapshot>,
    settings: StatusLineSettings,
    entity: Entity<WorkspaceWindow>,
    language: Language,
    pane_width: f32,
    reading_style_anchor_left: f32,
) -> gpui::AnyElement {
    let theme = current_theme();
    let pane = popover.pane;
    let content = popover.content;
    let close_entity = entity.clone();
    let title = match &content {
        StatusPopoverContent::Outline { .. } => segment_title(StatusSegment::Outline, language),
        StatusPopoverContent::ReadingStyle => match language {
            Language::Chinese => "阅读主题",
            Language::English => "Reading Theme",
        },
        StatusPopoverContent::Customize => match language {
            Language::Chinese => "定制状态栏",
            Language::English => "Customize Status Line",
        },
        StatusPopoverContent::Info(segment) => segment_title(*segment, language),
        StatusPopoverContent::Overflow(_) => match language {
            Language::Chinese => "更多状态",
            Language::English => "More Status",
        },
    };
    let mut panel = div()
        .id("status-popover")
        .absolute()
        .bottom(px(STATUS_LINE_HEIGHT + 8.0))
        .py(px(7.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(rgb(theme.border))
        .bg(rgb(theme.background))
        .shadow_lg()
        .text_size(px(11.0))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .h(px(34.0))
                .px(px(12.0))
                .flex()
                .items_center()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(div().flex_1().child(title))
                .child(
                    div()
                        .id("status-popover-close")
                        .px(px(6.0))
                        .cursor_pointer()
                        .text_color(rgb(theme.foreground_dim))
                        .child("×")
                        .on_click(move |_, _, cx| {
                            close_entity.update(cx, |this, cx| {
                                this.status.popover = None;
                                cx.notify();
                            });
                        }),
                ),
        );
    panel = if matches!(content, StatusPopoverContent::Outline { .. }) {
        panel
            .left(px(8.0))
            .w(px(286.0_f32.min((pane_width - 16.0).max(1.0))))
    } else if matches!(content, StatusPopoverContent::ReadingStyle) {
        panel
            .left(px(
                reading_style_anchor_left.min((pane_width - 128.0).max(8.0))
            ))
            .w(px(190.0_f32.min((pane_width - 16.0).max(1.0))))
    } else {
        panel
            .right(px(8.0))
            .w(px(286.0_f32.min((pane_width - 16.0).max(1.0))))
    };
    match content {
        StatusPopoverContent::Outline { document, entries } => {
            let mut list = div()
                .id("status-outline-list")
                .max_h(px(280.0))
                .overflow_y_scroll();
            for (index, (title, source)) in entries.iter().enumerate() {
                let source = *source;
                let row_entity = entity.clone();
                list = list.child(
                    div()
                        .id(("status-outline-heading", index))
                        .px(px(12.0))
                        .py(px(8.0))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(theme.background_alt)))
                        .child(super::breadcrumb(title, false))
                        .on_click(move |_, _, cx| {
                            row_entity.update(cx, |this, cx| {
                                let mapped = this.document_session().and_then(|session| {
                                    let session = session.read(cx);
                                    (session.id() == document)
                                        .then(|| session.map_range_to_current(source).ok())
                                        .flatten()
                                });
                                if let Some(mapped) = mapped {
                                    let side = super::pane_side_for_status(pane);
                                    match this.document_workspace.surface(side) {
                                        crate::app::PaneSurface::Editor => {
                                            if let Some(editor) = this.editor(side) {
                                                editor.update(cx, |editor, cx| {
                                                    editor.set_selection(
                                                        crate::document::Selection::caret(
                                                            mapped.range.start,
                                                        ),
                                                        cx,
                                                    );
                                                    editor.scroll_to_source_offset(
                                                        mapped.range.start,
                                                        cx,
                                                    );
                                                });
                                            }
                                        }
                                        crate::app::PaneSurface::Reading => {
                                            if this.latest_preview_is_current(cx)
                                                && let Some(panel) = this.reading_panel_for(side)
                                            {
                                                panel.update(cx, |panel, _| {
                                                    panel
                                                        .scroll_to_source_offset(mapped.range.start)
                                                });
                                            }
                                        }
                                    }
                                }
                                this.status.popover = None;
                                cx.notify();
                            })
                        }),
                );
            }
            if entries.is_empty() {
                list = list.child(
                    div()
                        .px(px(12.0))
                        .py(px(8.0))
                        .text_color(rgb(theme.foreground_dim))
                        .child(match language {
                            Language::Chinese => "此文档暂无标题",
                            Language::English => "No headings in this document",
                        }),
                );
            }
            panel = panel.child(list);
        }

        StatusPopoverContent::ReadingStyle => {
            let selected = snapshot.and_then(|snapshot| snapshot.reading_style);
            for style_id in crate::preview::PreviewStyleId::ALL {
                let row_entity = entity.clone();
                let style = *crate::preview::preview_style(style_id);
                panel = panel.child(
                    div()
                        .id(("reading-style", style_id as usize))
                        .h(px(34.0))
                        .mx(px(7.0))
                        .px(px(8.0))
                        .rounded(px(6.0))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(theme.background_alt)))
                        .when(selected == Some(style_id), |row| {
                            row.bg(rgb(theme.background_alt))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                        })
                        .child(div().flex_1().child(style.name(language)))
                        .when(selected == Some(style_id), |row| {
                            row.child(div().text_color(rgb(theme.heading[0])).child("✓"))
                        })
                        .on_click(move |_, _, cx| {
                            row_entity
                                .update(cx, |this, cx| this.select_reading_style(style_id, cx));
                        }),
                );
            }
        }
        StatusPopoverContent::Customize => {
            for segment in CONFIGURABLE_SEGMENTS {
                let enabled = segment.enabled(settings);
                let toggle_entity = entity.clone();
                panel = panel.child(
                    div()
                        .id(("status-setting", segment as usize))
                        .h(px(34.0))
                        .mx(px(7.0))
                        .px(px(8.0))
                        .rounded(px(6.0))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(theme.background_alt)))
                        .child(div().flex_1().child(segment_title(segment, language)))
                        .child(
                            div()
                                .w(px(28.0))
                                .h(px(16.0))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .when(enabled, |switch| switch.justify_end())
                                .when(!enabled, |switch| switch.justify_start())
                                .px(px(3.0))
                                .bg(rgb(if enabled {
                                    theme.code_boundary_background
                                } else {
                                    theme.background_alt
                                }))
                                .child(div().size(px(10.0)).rounded_full().bg(rgb(if enabled {
                                    theme.heading[0]
                                } else {
                                    theme.foreground_dim
                                }))),
                        )
                        .on_click(move |_, _, cx| {
                            toggle_entity
                                .update(cx, |this, cx| this.toggle_status_segment(segment, cx));
                        }),
                );
            }
            panel = panel.child(
                div()
                    .mt(px(5.0))
                    .pt(px(8.0))
                    .mx(px(12.0))
                    .border_t_1()
                    .border_color(rgb(theme.border))
                    .text_size(px(9.0))
                    .text_color(rgb(theme.foreground_dim))
                    .child(match language {
                        Language::Chinese => "模式和更多入口始终保留；其余配置自动保存。",
                        Language::English => "Mode and More stay visible; other choices are saved.",
                    }),
            );
        }
        StatusPopoverContent::Overflow(segments) => {
            for segment in segments.iter().copied() {
                let row_entity = entity.clone();
                panel = panel.child(
                    div()
                        .id(("status-overflow", segment as usize))
                        .h(px(34.0))
                        .mx(px(7.0))
                        .px(px(8.0))
                        .rounded(px(6.0))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(theme.background_alt)))
                        .child(div().flex_1().child(segment_title(segment, language)))
                        .child(div().text_color(rgb(theme.foreground_dim)).child("›"))
                        .on_click(move |_, _, cx| {
                            row_entity.update(cx, |this, cx| {
                                this.activate_status_segment(pane, segment, None, cx)
                            });
                        }),
                );
            }
            let customize_entity = entity.clone();
            panel = panel.child(
                div()
                    .id("status-overflow-customize")
                    .h(px(34.0))
                    .mt(px(4.0))
                    .mx(px(7.0))
                    .px(px(8.0))
                    .border_t_1()
                    .border_color(rgb(theme.border))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(theme.background_alt)))
                    .child(match language {
                        Language::Chinese => "定制状态栏…",
                        Language::English => "Customize status line…",
                    })
                    .on_click(move |_, _, cx| {
                        customize_entity.update(cx, |this, cx| {
                            this.status.popover = Some(StatusPopover {
                                pane,
                                content: StatusPopoverContent::Customize,
                            });
                            cx.notify();
                        });
                    }),
            );
        }
        StatusPopoverContent::Info(segment) => {
            if segment == StatusSegment::Statistics
                && let Some(statistics) = snapshot.and_then(|value| value.document_statistics)
            {
                panel = panel.child(render_document_statistics(statistics, language));
            } else {
                panel = panel.child(
                    div()
                        .px(px(12.0))
                        .pb(px(9.0))
                        .text_color(rgb(theme.foreground_dim))
                        .child(info_text(segment, snapshot, language)),
                );
            }
        }
    }
    panel.into_any_element()
}
