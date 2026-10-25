use super::*;
use crate::{components::UiTextExt, theme, typography::ui_px};
use gpui::{AnyElement, MouseButton, Role, div, prelude::*, rgb};

impl WorkspaceWindow {
    pub(super) fn close_overview_file(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        if self.buffers.panel.is_some() && self.buffers.inline_close_target().is_none()
            || !matches!(
                self.save.interaction,
                super::super::save::SaveInteraction::Idle
            )
            || !self
                .file_overview
                .session
                .as_ref()
                .is_some_and(|s| s.phase == Phase::Visible)
        {
            return;
        }
        self.cancel_overview_close(cx);
        self.request_close_buffer(id, cx);
        if self.buffers.review().is_none() {
            self.focus_file_overview(cx);
        } else if let Some(state) = &mut self.file_overview.session {
            state.focus_query_pending = false;
        }
    }

    pub(crate) fn cancel_overview_close(&mut self, cx: &mut Context<Self>) {
        if self.buffers.inline_close_target().is_some() && !self.buffer_busy() {
            self.cancel_buffer_panel(cx);
        }
    }

    pub(super) fn overview_close_overlay(
        &self,
        id: DocumentId,
        preview: Size<Pixels>,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if self.buffers.inline_close_target() != Some(id) {
            return None;
        }
        let review = self.buffers.review()?;
        let theme = theme::current_theme();
        Some(
            div()
                .id("file-overview-close-confirmation")
                .debug_selector(|| "file-overview-close-confirmation".into())
                .role(Role::Group)
                .aria_label(self.buffer_text("关闭文件", "Close document"))
                .absolute()
                .inset_0()
                .occlude()
                .cursor_default()
                .bg(rgb(if theme::effective_theme_is_dark() {
                    theme.overview_background
                } else {
                    theme.overview_edge
                })
                .opacity(0.84))
                .p(px(8.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(|_, _, cx| cx.stop_propagation())
                .on_mouse_down_out(cx.listener(|this, _, _, cx| this.cancel_overview_close(cx)))
                .when_some(review.error.clone(), |view, error| {
                    view.child(
                        div()
                            .id("buffer-review-error")
                            .debug_selector(|| "buffer-review-error".into())
                            .aria_description(error.clone())
                            .text_size(ui_px(11.))
                            .text_color(rgb(theme.foreground))
                            .line_clamp(2)
                            .tooltip(move |_, cx| {
                                cx.new(|_| super::view::Hint(error.clone().into())).into()
                            })
                            .child_text(
                                self.buffer_text("保存失败，请重试", "Save failed. Try again."),
                            ),
                    )
                })
                .child(self.review_buttons(review, preview.width < ui_px(230.), cx.entity()))
                .into_any_element(),
        )
    }

    pub(crate) fn focus_file_overview(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = &mut self.file_overview.session {
            state.focus_query_pending = true;
            self.cancel_pending_document_focus(cx);
            cx.notify();
        }
    }
}
