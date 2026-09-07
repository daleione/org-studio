use crate::{
    agenda::{BuiltinQuery, FileId},
    app::WorkspaceWindow,
};
use gpui::{
    Div, Entity, InteractiveElement, MouseButton, ParentElement, Styled, div, prelude::*, px, svg,
};
use std::sync::Arc;

use super::badge;

pub(crate) fn sidebar_section_header(
    workspace: Entity<WorkspaceWindow>,
    label: &'static str,
    section: super::super::state::SidebarSection,
    expanded: bool,
) -> Div {
    div()
        .h(px(29.))
        .px_5()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(11.))
        .text_color(gpui::rgb(0x8a8d93))
        .cursor_pointer()
        .child(
            svg()
                .data(&b"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 12 12'><path d='M4 2L9 6L4 10Z'/></svg>"[..])
                .size(px(10.))
                .flex_none()
                .text_color(gpui::rgb(0x777b82))
                .with_transformation(gpui::Transformation::rotate(gpui::radians(
                    if expanded { std::f32::consts::FRAC_PI_2 } else { 0. },
                ))),
        )
        .child(label)
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            workspace.update(cx, |this, cx| {
                this.dispatch_agenda_intent(
                    super::super::UiIntent::ToggleSidebarSection(section),
                    cx,
                )
            });
        })
}

pub(crate) struct StaticSidebarItem {
    pub language: crate::i18n::Language,
    pub workspace: Entity<WorkspaceWindow>,
    pub label: &'static str,
    pub icon: &'static str,
    pub count: Option<usize>,
    pub selected: bool,
    pub compact: bool,
    pub query: BuiltinQuery,
}

pub(crate) fn static_sidebar_item(props: StaticSidebarItem) -> Div {
    let StaticSidebarItem {
        language,
        workspace,
        label,
        icon,
        count,
        selected,
        compact,
        query,
    } = props;
    div()
        .relative()
        .h(px(39.))
        .px_3()
        .when(compact, |item| item.px_0().justify_center())
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(8.))
        .text_size(px(14.))
        .text_color(gpui::rgb(if selected { 0x6632bd } else { 0x2f3238 }))
        .when(selected, |item| {
            item.bg(gpui::rgb(super::super::style::PURPLE_SELECTION))
                .font_weight(gpui::FontWeight::SEMIBOLD)
        })
        .child(
            div()
                .w(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .data(super::super::icon::agenda_icon(icon))
                        .size(px(17.))
                        .text_color(gpui::rgb(if selected { 0x7c3bd1 } else { 0x51565e })),
                ),
        )
        .when(!compact, |item| {
            item.child(div().flex_1().child(language.text(match label {
                "收件箱" => "agenda.inbox",
                "Tasks" => "agenda.tasks",
                "Projects" => "agenda.projects",
                _ => "agenda.agenda",
            })))
        })
        .when_some(count, |item, count| {
            item.child(badge(count.to_string()).when(compact, |badge| {
                badge
                    .absolute()
                    .top(px(1.))
                    .right(px(0.))
                    .min_w(px(18.))
                    .h(px(18.))
                    .text_size(px(9.))
            }))
        })
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            workspace.update(cx, |this, cx| {
                this.dispatch_agenda_intent(
                    super::super::UiIntent::SelectNavigation(label, query),
                    cx,
                )
            });
        })
}

pub(crate) fn sidebar_filter_item(
    workspace: Entity<WorkspaceWindow>,
    label: String,
    count: usize,
    tag: Option<Arc<str>>,
    source: Option<FileId>,
    selected: bool,
    is_source: bool,
) -> Div {
    let context_workspace = workspace.clone();
    let context_source = source;
    let label = if is_source {
        div()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(label)
    } else {
        div()
            .h(px(23.))
            .px_3()
            .flex()
            .items_center()
            .rounded(px(12.))
            .bg(gpui::rgb(if selected { 0xe6d9ee } else { 0xf2eaf5 }))
            .text_size(px(11.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(gpui::rgb(0x7d438d))
            .child(label)
    };
    div()
        .min_h(px(if is_source { 32. } else { 35. }))
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(7.))
        .when(selected, |item| {
            item.bg(gpui::rgb(super::super::style::PURPLE_SELECTION))
        })
        .text_size(px(12.))
        .text_color(gpui::rgb(0x555960))
        .cursor_pointer()
        .child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .overflow_hidden()
                .child(label),
        )
        .child(
            div()
                .text_size(px(10.))
                .text_color(gpui::rgb(0x777b82))
                .child(count.to_string()),
        )
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            let tag = tag.clone();
            workspace.update(cx, |this, cx| {
                if is_source {
                    this.dispatch_agenda_intent(
                        super::super::UiIntent::SetSource(if selected { None } else { source }),
                        cx,
                    )
                } else {
                    this.dispatch_agenda_intent(super::super::UiIntent::SetTag(tag), cx)
                }
            });
        })
        .when(is_source && context_source.is_some(), |item| {
            item.on_mouse_down(MouseButton::Right, move |event, _, cx| {
                cx.stop_propagation();
                if let Some(file) = context_source {
                    context_workspace.update(cx, |this, cx| {
                        this.dispatch_agenda_intent(
                            super::super::UiIntent::ShowSourceContextMenu(file, event.position),
                            cx,
                        )
                    });
                }
            })
        })
}

pub(crate) fn source_context_menu(
    language: crate::i18n::Language,
    workspace: Entity<WorkspaceWindow>,
    menu: super::super::state::SourceContextMenu,
) -> impl gpui::IntoElement {
    div()
        .id("agenda-source-context-menu")
        .absolute()
        .left(menu.position.x)
        .top(menu.position.y)
        .w(px(168.))
        .py_1()
        .rounded(px(7.))
        .border_1()
        .border_color(gpui::rgb(0xdedfe2))
        .bg(gpui::rgb(0xffffff))
        .shadow_lg()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .h(px(30.))
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .rounded(px(5.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(gpui::rgb(0x2f3238))
                .hover(|style| style.bg(gpui::rgb(0xf1eff8)))
                .child(
                    svg()
                        .data(super::super::icon::agenda_icon(
                            "assets/icons/agenda/arrow-square-out.svg",
                        ))
                        .size(px(15.)),
                )
                .child(language.text("agenda.open_file"))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    workspace.update(cx, |this, cx| {
                        this.open_agenda_source_file(menu.file, window, cx)
                    });
                }),
        )
}

pub(crate) fn saved_view_item(
    workspace: Entity<WorkspaceWindow>,
    label: String,
    index: usize,
    selected: bool,
) -> Div {
    div()
        .h(px(36.))
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(7.))
        .when(selected, |item| {
            item.bg(gpui::rgb(super::super::style::PURPLE_SELECTION))
        })
        .text_size(px(12.))
        .text_color(gpui::rgb(if selected { 0x6632bd } else { 0x555960 }))
        .cursor_pointer()
        .child(
            svg()
                .data(super::super::icon::agenda_icon(
                    "assets/icons/agenda/star.svg",
                ))
                .size(px(15.)),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(label),
        )
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            workspace.update(cx, |this, cx| {
                this.dispatch_agenda_intent(super::super::UiIntent::SelectSavedView(index), cx)
            });
        })
}

pub(crate) fn sidebar_item(
    workspace: Entity<WorkspaceWindow>,
    query: BuiltinQuery,
    label: &'static str,
    icon: &'static str,
    count: usize,
    selected: bool,
    compact: bool,
) -> Div {
    let count_badge = badge(count.to_string()).when(compact, |badge| {
        badge
            .absolute()
            .top(px(1.))
            .right(px(0.))
            .min_w(px(18.))
            .h(px(18.))
            .text_size(px(9.))
    });
    div()
        .relative()
        .h(px(39.0))
        .px_3()
        .when(compact, |item| item.px_0().justify_center())
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(8.))
        .text_size(px(14.))
        .text_color(gpui::rgb(if selected { 0x6632bd } else { 0x2f3238 }))
        .when(selected, |item| {
            item.bg(gpui::rgb(super::super::style::PURPLE_SELECTION))
                .font_weight(gpui::FontWeight::SEMIBOLD)
        })
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            workspace.update(cx, |this, cx| {
                this.dispatch_agenda_intent(
                    super::super::UiIntent::SelectNavigation(label, query),
                    cx,
                )
            });
        })
        .child(
            div()
                .w(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .data(super::super::icon::agenda_icon(icon))
                        .size(px(17.))
                        .text_color(gpui::rgb(if selected { 0x7c3bd1 } else { 0x51565e })),
                ),
        )
        .when(!compact, |item| item.child(div().flex_1().child(label)))
        .child(count_badge)
}
