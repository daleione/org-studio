use super::*;
use crate::{
    components::UiTextExt,
    theme::current_theme,
    typography::{ui_font, ui_px},
};
use gpui::{MouseButton, Role, div, prelude::*, px, rgb};

impl WorkspaceWindow {
    pub(crate) fn render_document_name(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let prompt = self.document_name.session.as_ref()?;
        let busy = self.document_name_busy();
        let theme = current_theme();
        let width = f32::from(window.viewport_size().width).min(392.) - 32.;
        let message = prompt.error.as_deref().unwrap_or_else(|| {
            if busy {
                self.buffer_text("正在重命名…", "Renaming…")
            } else if prompt.document.read(cx).is_draft() {
                self.buffer_text("尚未保存到磁盘", "Not saved to disk")
            } else {
                self.buffer_text("在当前文件夹中重命名", "Rename in the current folder")
            }
        });
        let buttons = [
            (
                "document-name-cancel",
                self.buffer_text("取消", "Cancel"),
                &prompt.cancel_focus,
                false,
            ),
            (
                "document-name-done",
                self.buffer_text("完成", "Done"),
                &prompt.done_focus,
                true,
            ),
        ]
        .into_iter()
        .fold(
            div().mt(px(14.)).flex().justify_end().gap(px(8.)),
            |row, (id, label, focus, confirm)| {
                row.child(
                    div()
                        .id(id)
                        .debug_selector(move || id.to_owned())
                        .role(Role::Button)
                        .aria_label(label)
                        .track_focus(focus)
                        .tab_index(0)
                        .px(px(14.))
                        .py(px(6.))
                        .rounded(px(5.))
                        .bg(rgb(if confirm {
                            theme.accent
                        } else {
                            theme.background_alt
                        }))
                        .text_color(rgb(if confirm { 0xffffff } else { theme.foreground }))
                        .cursor_pointer()
                        .when(busy, |button| button.opacity(0.5).cursor_default())
                        .focus(|s| s.border_1().border_color(rgb(theme.accent)))
                        .child_text(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if confirm {
                                this.confirm_document_name(AfterRename::Nothing, window, cx);
                            } else {
                                this.cancel_document_name(cx);
                            }
                        })),
                )
            },
        );
        Some(
            div()
                .id("document-name-popover")
                .debug_selector(|| "document-name-popover".into())
                .role(Role::Dialog)
                .aria_label(self.buffer_text("命名或重命名文档", "Name or rename document"))
                .occlude()
                .absolute()
                .top(px(crate::app::TITLEBAR_HEIGHT + 6.))
                .left(
                    (self.document_name.anchor.center().x - px(width / 2.))
                        .clamp(px(16.), window.viewport_size().width - px(width + 16.)),
                )
                .w(px(width))
                .p(px(14.))
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(theme.border))
                .bg(rgb(theme.elevated))
                .shadow_lg()
                .font(ui_font(".SystemUIFont"))
                .text_size(ui_px(12.))
                .text_color(rgb(theme.foreground))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down_out(cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if !this.document_name.anchor.contains(&event.position) {
                        this.cancel_document_name(cx);
                    }
                }))
                .child(
                    div()
                        .mb(px(7.))
                        .child_text(self.buffer_text("名称", "Name")),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h(px(34.))
                                .child(prompt.input.clone()),
                        )
                        .when(!prompt.extension.is_empty(), |row| {
                            row.child(
                                div()
                                    .text_color(rgb(theme.foreground_dim))
                                    .child_text(format!(".{}", prompt.extension)),
                            )
                        }),
                )
                .child(
                    div()
                        .mt(px(7.))
                        .text_size(ui_px(11.))
                        .text_color(rgb(if prompt.error.is_some() {
                            theme.error
                        } else {
                            theme.foreground_dim
                        }))
                        .child_text(message.to_owned()),
                )
                .child(buttons)
                .into_any_element(),
        )
    }
}
