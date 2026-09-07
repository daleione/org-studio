use gpui::{Div, Entity, InteractiveElement, MouseButton, div, prelude::*, px, rgb, svg};

use super::super::{UiIntent, state::AgendaProjection};
use crate::app::WorkspaceWindow;

pub(crate) fn projection_switch(
    workspace: Entity<WorkspaceWindow>,
    projection: AgendaProjection,
) -> Div {
    [
        (
            AgendaProjection::List,
            "assets/icons/agenda/list-bullets.svg",
        ),
        (
            AgendaProjection::Calendar,
            "assets/icons/agenda/calendar-dots.svg",
        ),
        (
            AgendaProjection::Source,
            "assets/icons/agenda/file-code.svg",
        ),
    ]
    .into_iter()
    .fold(
        div()
            .flex_none()
            .h(px(38.))
            .p(px(2.))
            .flex()
            .items_center()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(0xdfe0e3))
            .bg(rgb(0xf4f4f5)),
        |control, (value, icon)| {
            let selected = value == projection;
            let workspace = workspace.clone();
            control.child(
                div()
                    .flex_none()
                    .w(px(40.))
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |button| button.bg(rgb(0xffffff)).shadow_sm())
                    .when(!selected, |button| {
                        button.hover(|style| style.bg(rgb(0xeaeaec)))
                    })
                    .child(
                        svg()
                            .data(super::super::icon::agenda_icon(icon))
                            .size(px(16.))
                            .text_color(rgb(if selected { 0x1688ff } else { 0x34373d })),
                    )
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        workspace.update(cx, |this, cx| {
                            this.dispatch_agenda_intent(UiIntent::SetProjection(value), cx);
                        });
                    }),
            )
        },
    )
}
