use super::*;
use crate::components::UiTextExt;
use crate::components::selection_style::HOVER_BACKGROUND;
use crate::typography::{ui_family, ui_px};
use gpui::{AnyElement, MouseButton, div, prelude::*, px, rgb};

fn button_base(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
) -> gpui::Stateful<gpui::Div> {
    let theme = crate::theme::current_theme();
    div()
        .id(id)
        .flex_none()
        .px(px(9.))
        .h(ui_px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .text_size(ui_px(12.))
        .text_color(rgb(theme.foreground_dim))
        .cursor_pointer()
        .child_text(label)
}

fn button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
) -> gpui::Stateful<gpui::Div> {
    button_base(id, label).hover(|s| s.bg(rgb(HOVER_BACKGROUND())))
}

impl WorkspaceWindow {
    pub(crate) fn buffer_panel(
        &self,
        pane: crate::app::PaneSide,
        entity: Entity<Self>,
        width: f32,
        status: Option<AnyElement>,
        cx: &App,
    ) -> Option<AnyElement> {
        if !self
            .status
            .shell_owns(crate::app::status_line::shell::ShellKind::Buffers, pane)
        {
            return None;
        }
        let available = (width - 2. * crate::app::status_line::FLOATING_STATUS_INSET).max(1.);
        let shape = self
            .status
            .sample_shell(available, std::time::Instant::now());
        let review = self
            .buffers
            .review()
            .or_else(|| match &self.buffers.outgoing {
                Some(OutgoingPanel::Review(review)) => Some(review.as_ref()),
                _ => None,
            });
        let body = if let Some(review) = review {
            Some(self.review_view(review, entity.clone(), self.buffers.available, cx))
        } else if let Some(Panel::Picker(p)) = &self.buffers.panel {
            Some(self.picker_view(p, entity.clone(), cx))
        } else {
            None
        };
        Some(
            crate::app::status_line::floating_status_container(shape.height)
                .occlude()
                .when(review.is_some(), |shell| {
                    shell.bg(rgb(crate::theme::current_theme().elevated))
                })
                .when(status.is_none(), |shell| {
                    shell.opacity(shape.content_opacity)
                })
                .left(px((width - shape.width) / 2.))
                .right(gpui::auto())
                .w(px(shape.width))
                .font_family(ui_family(".SystemUIFont"))
                .children(
                    body.map(|body| div().size_full().opacity(shape.content_opacity).child(body)),
                )
                .when(self.buffers.returning() && status.is_some(), |shell| {
                    shell.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .w_full()
                            .h(ui_px(crate::app::status_line::FLOATING_STATUS_HEIGHT))
                            .opacity(1. - shape.content_opacity)
                            .children(status),
                    )
                })
                .into_any_element(),
        )
    }

    fn picker_view(&self, p: &Picker, entity: Entity<Self>, cx: &App) -> AnyElement {
        let theme = crate::theme::current_theme();
        let close_entity = entity.clone();
        let close = button("buffer-panel-close", "×")
            .text_size(ui_px(19.))
            .w(ui_px(28.))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                close_entity.update(cx, |w, cx| w.cancel_buffer_panel(cx));
            });
        if p.intent == PickerIntent::New {
            let format_entity = entity.clone();
            if self.buffers.available < 420. {
                return div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .px(px(12.))
                    .py(px(10.))
                    .gap(px(8.))
                    .child(
                        div()
                            .h(ui_px(24.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_size(ui_px(12.))
                            .text_color(rgb(theme.foreground_dim))
                            .child_text(self.buffer_text("新建", "New"))
                            .child(close),
                    )
                    .child(
                        div()
                            .h(ui_px(34.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().flex_1().min_w_0().h_full().child(p.input.clone()))
                            .child(
                                button(
                                    "buffer-draft-format",
                                    if p.markdown { "Markdown" } else { "Org" },
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    move |_, _, cx| {
                                        cx.stop_propagation();
                                        format_entity.update(cx, |w, cx| {
                                            if let Some(Panel::Picker(p)) = &mut w.buffers.panel {
                                                p.markdown = !p.markdown;
                                            }
                                            cx.notify();
                                        });
                                    },
                                ),
                            )
                            .child(
                                div()
                                    .text_color(rgb(theme.foreground_muted))
                                    .child_text("↵"),
                            ),
                    )
                    .into_any_element();
            }
            return div()
                .size_full()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(12.))
                .child(
                    div()
                        .flex_none()
                        .text_size(ui_px(12.))
                        .text_color(rgb(theme.foreground_dim))
                        .child_text(self.buffer_text("新建", "New")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h(ui_px(34.))
                        .child(p.input.clone()),
                )
                .child(
                    button(
                        "buffer-draft-format",
                        if p.markdown { "Markdown" } else { "Org" },
                    )
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        format_entity.update(cx, |w, cx| {
                            if let Some(Panel::Picker(p)) = &mut w.buffers.panel {
                                p.markdown = !p.markdown;
                            }
                            cx.notify();
                        });
                    }),
                )
                .child(
                    div()
                        .text_color(rgb(theme.foreground_muted))
                        .child_text("↵"),
                )
                .child(close)
                .into_any_element();
        }
        if p.intent == PickerIntent::Switch {
            return self.quick_picker_view(p, entity, close.into_any_element(), cx);
        }
        let items = self.buffer_candidates(cx);
        let total = items.len();
        let heading = if p.intent == PickerIntent::Close {
            self.buffer_text("关闭文档", "Close document")
        } else if p.intent == PickerIntent::File {
            self.buffer_text("打开 / 新建文件", "Open / create file")
        } else {
            self.buffer_text("已打开", "Open documents")
        };
        let mut rows = Vec::new();
        for (index, c) in items.into_iter().enumerate() {
            let row_entity = entity.clone();
            let close_entity = entity.clone();
            let id = c.id;
            let selected = index == p.selected;
            let state = if c.dirty {
                self.buffer_text("已修改", "Modified")
            } else if c.current {
                self.buffer_text("当前", "Current")
            } else {
                ""
            };
            let path = if c.path.is_empty() {
                self.buffer_text("草稿", "Draft").to_owned()
            } else {
                Path::new(&c.path)
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            rows.push(
                div()
                    .id(("buffer-row", index))
                    .h(ui_px(46.))
                    .flex_none()
                    .px(px(10.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .cursor_pointer()
                    .when(selected, |s| s.bg(rgb(theme.accent_bg)))
                    .hover(|s| s.bg(rgb(theme.hover)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(ui_px(13.))
                                    .text_color(rgb(theme.foreground))
                                    .child_text(c.name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(ui_px(10.))
                                    .text_color(rgb(theme.foreground_muted))
                                    .child_text(path),
                            ),
                    )
                    .child(
                        div()
                            .w(px(66.))
                            .flex_none()
                            .text_size(ui_px(11.))
                            .text_color(rgb(if c.dirty {
                                theme.warning
                            } else {
                                theme.foreground_muted
                            }))
                            .child_text(state),
                    )
                    .when_some(id, |row, id| {
                        row.child(
                            button(("buffer-row-close", index), "×")
                                .w(ui_px(28.))
                                .opacity(if selected { 1. } else { 0.35 })
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    cx.stop_propagation();
                                    close_entity.update(cx, |w, cx| w.request_close_buffer(id, cx));
                                }),
                        )
                    })
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        row_entity.update(cx, |w, cx| {
                            if let Some(Panel::Picker(p)) = &mut w.buffers.panel {
                                p.selected = index;
                            }
                            w.accept_buffer_picker(cx);
                        });
                    }),
            );
        }
        let hint = p.message.clone().unwrap_or_else(|| {
            if p.pending_confirm {
                self.buffer_text("再次按 Enter 确认新建", "Press Enter again to create")
                    .to_owned()
            } else if total == 0
                && !p.input.read(cx).text.trim().is_empty()
                && p.intent != PickerIntent::Close
            {
                self.buffer_text("Enter 新建 · C-g 取消", "Enter to create · C-g to cancel")
                    .to_owned()
            } else {
                self.buffer_text(
                    "↑ ↓ 选择 · Enter 确认 · C-g 取消",
                    "↑ ↓ Select · Enter Confirm · C-g Cancel",
                )
                .to_owned()
            }
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(ui_px(50.))
                    .flex_none()
                    .px(px(18.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .text_size(ui_px(12.))
                    .text_color(rgb(theme.foreground_dim))
                    .child(heading)
                    .child(
                        div()
                            .text_color(rgb(theme.foreground_muted))
                            .child_text(total.to_string()),
                    )
                    .child(div().flex_1())
                    .child(close),
            )
            .child(
                div()
                    .id("buffer-candidates")
                    .flex_1()
                    .min_h_0()
                    .px(px(10.))
                    .overflow_y_scroll()
                    .track_scroll(&self.buffers.scroll)
                    .when(total == 0, |d| {
                        d.child(
                            div()
                                .h(px(160.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(ui_px(12.))
                                .text_color(rgb(theme.foreground_muted))
                                .child_text(
                                    self.buffer_text("没有匹配的文档", "No matching documents"),
                                ),
                        )
                    })
                    .children(rows),
            )
            .child(
                div()
                    .h(ui_px(30.))
                    .flex_none()
                    .px(px(18.))
                    .flex()
                    .items_center()
                    .text_size(ui_px(10.))
                    .text_color(rgb(theme.foreground_muted))
                    .child_text(hint),
            )
            .child(
                div()
                    .h(px(60.))
                    .flex_none()
                    .px(px(12.))
                    .border_t_1()
                    .border_color(rgb(theme.border))
                    .bg(rgb(theme.surface))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(ui_px(34.))
                            .child(p.input.clone()),
                    ),
            )
            .into_any_element()
    }
}
