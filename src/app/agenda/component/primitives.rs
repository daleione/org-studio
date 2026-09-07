use crate::app::WorkspaceWindow;
use gpui::{
    Div, Entity, InteractiveElement, MouseButton, ParentElement, Stateful,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px, rgb, svg,
};

pub(crate) fn badge(text: impl Into<String>) -> Div {
    div()
        .min_w(px(23.))
        .h(px(23.))
        .px_2()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgb(0xdeedff))
        .text_color(rgb(0x0a6ed1))
        .text_size(px(11.))
        .child(text.into())
}
pub(crate) fn pill(text: impl Into<String>) -> Div {
    div()
        .h(px(24.))
        .px_3()
        .flex()
        .items_center()
        .rounded_full()
        .bg(rgb(0xf3edf5))
        .text_color(rgb(0x754c7d))
        .text_size(px(11.))
        .child(text.into())
}
pub(crate) fn icon_button(path: &'static str) -> Div {
    div()
        .flex_none()
        .size(px(38.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(0xdfe0e3))
        .bg(rgb(0xffffff))
        .text_color(rgb(0x34373d))
        .child(
            svg()
                .data(super::super::icon::agenda_icon(path))
                .size(px(16.))
                .text_color(rgb(0x34373d)),
        )
}
pub(crate) fn action_icon_button(
    workspace: Entity<WorkspaceWindow>,
    path: &'static str,
    intent: super::super::UiIntent,
    selected: bool,
) -> Stateful<Div> {
    icon_button(path)
        .id(format!("agenda-action-icon-{path}"))
        .cursor_pointer()
        .when(selected, |button| {
            button.bg(rgb(0xf1eafb)).text_color(rgb(0x7540c4))
        })
        .hover(move |style| {
            if selected {
                style.bg(rgb(0xe5d9f7)).border_color(rgb(0xa98ad5))
            } else {
                style.bg(rgb(0xe9eaed)).border_color(rgb(0xbfc2c8))
            }
        })
        .active(|style| style.opacity(0.72))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            let intent = intent.clone();
            workspace.update(cx, |this, cx| this.dispatch_agenda_intent(intent, cx));
        })
}
pub(crate) fn text_action_button(
    workspace: Entity<WorkspaceWindow>,
    label: &'static str,
    intent: super::super::UiIntent,
) -> Stateful<Div> {
    div()
        .id(format!("agenda-text-action-{label}"))
        .h(px(28.))
        .px_3()
        .flex()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(rgb(0xd9c18e))
        .bg(rgb(0xffffff))
        .text_size(px(10.))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0xfff8e9)).border_color(rgb(0xcaa75e)))
        .active(|style| style.opacity(0.72))
        .child(label)
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            let intent = intent.clone();
            workspace.update(cx, |this, cx| this.dispatch_agenda_intent(intent, cx));
        })
}
pub(crate) fn field(value: impl gpui::IntoElement) -> Stateful<Div> {
    div()
        .id("agenda-search-field")
        .w(px(250.))
        .h(px(38.))
        .px_3()
        .flex()
        .items_center()
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(0xdfe0e3))
        .bg(rgb(0xffffff))
        .text_color(rgb(0x858990))
        .text_size(px(12.))
        .gap_2()
        .cursor_text()
        .hover(|style| style.bg(rgb(0xf8fbff)).border_color(rgb(0x8eb9df)))
        .active(|style| style.border_color(rgb(0x1688ff)))
        .child(
            svg()
                .data(super::super::icon::agenda_icon(
                    "assets/icons/agenda/search.svg",
                ))
                .size(px(15.))
                .text_color(rgb(0x777b82)),
        )
        .child(div().flex_1().min_w_0().child(value))
        .child(
            div()
                .text_size(px(10.))
                .text_color(rgb(0xa2a5aa))
                .child("⌘K"),
        )
}
pub(crate) fn empty_state(text: impl Into<String>) -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(0x7f8998))
        .child(text.into())
}
