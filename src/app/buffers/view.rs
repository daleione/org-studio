use super::*;
use crate::components::selection_style::{
    HOVER_BACKGROUND, SELECTED_BACKGROUND, SELECTED_HOVER_BACKGROUND,
};
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
        .h(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .text_size(px(12.))
        .text_color(rgb(theme.foreground_dim))
        .cursor_pointer()
        .child(label.into())
}

fn button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
) -> gpui::Stateful<gpui::Div> {
    button_base(id, label).hover(|s| s.bg(rgb(HOVER_BACKGROUND())))
}

fn review_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    selected: bool,
    disabled: bool,
) -> gpui::Stateful<gpui::Div> {
    let theme = crate::theme::current_theme();
    let background = if selected {
        SELECTED_BACKGROUND()
    } else {
        theme.background
    };
    // White on the saturated selected blue stays fixed for contrast.
    let foreground = if selected {
        0xffffff
    } else {
        theme.foreground_dim
    };
    button_base(id, label)
        .bg(rgb(background))
        .text_color(rgb(foreground))
        .when(disabled, |s| s.opacity(0.5).cursor_default())
        .hover(move |s| {
            s.bg(rgb(if disabled {
                background
            } else if selected {
                SELECTED_HOVER_BACKGROUND()
            } else {
                HOVER_BACKGROUND()
            }))
            .text_color(rgb(foreground))
        })
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
        let body = match &self.buffers.panel {
            Some(Panel::Picker(p)) => Some(self.picker_view(p, entity.clone(), cx)),
            Some(Panel::Review(review)) => Some(self.review_view(review, entity.clone(), cx)),
            None => None,
        };
        Some(
            crate::app::status_line::floating_status_container(shape.height)
                .occlude()
                .when(status.is_none(), |shell| {
                    shell.opacity(shape.content_opacity)
                })
                .left(px((width - shape.width) / 2.))
                .right(gpui::auto())
                .w(px(shape.width))
                .font_family(".SystemUIFont")
                .children(
                    body.map(|body| div().size_full().opacity(shape.content_opacity).child(body)),
                )
                .when(self.buffers.returning && status.is_some(), |shell| {
                    shell.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .w_full()
                            .h(px(crate::app::status_line::FLOATING_STATUS_HEIGHT))
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
            .text_size(px(19.))
            .w(px(28.))
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
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_size(px(12.))
                            .text_color(rgb(theme.foreground_dim))
                            .child(self.buffer_text("新建", "New"))
                            .child(close),
                    )
                    .child(
                        div()
                            .h(px(34.))
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
                            .child(div().text_color(rgb(theme.foreground_muted)).child("↵")),
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
                        .text_size(px(12.))
                        .text_color(rgb(theme.foreground_dim))
                        .child(self.buffer_text("新建", "New")),
                )
                .child(div().flex_1().min_w_0().h(px(34.)).child(p.input.clone()))
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
                .child(div().text_color(rgb(theme.foreground_muted)).child("↵"))
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
                    .h(px(46.))
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
                                    .text_size(px(13.))
                                    .text_color(rgb(theme.foreground))
                                    .child(c.name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(10.))
                                    .text_color(rgb(theme.foreground_muted))
                                    .child(path),
                            ),
                    )
                    .child(
                        div()
                            .w(px(66.))
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(rgb(if c.dirty {
                                theme.warning
                            } else {
                                theme.foreground_muted
                            }))
                            .child(state),
                    )
                    .when_some(id, |row, id| {
                        row.child(
                            button(("buffer-row-close", index), "×")
                                .w(px(28.))
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
                    .h(px(50.))
                    .flex_none()
                    .px(px(18.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .text_size(px(12.))
                    .text_color(rgb(theme.foreground_dim))
                    .child(heading)
                    .child(
                        div()
                            .text_color(rgb(theme.foreground_muted))
                            .child(total.to_string()),
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
                                .text_size(px(12.))
                                .text_color(rgb(theme.foreground_muted))
                                .child(self.buffer_text("没有匹配的文档", "No matching documents")),
                        )
                    })
                    .children(rows),
            )
            .child(
                div()
                    .h(px(30.))
                    .flex_none()
                    .px(px(18.))
                    .flex()
                    .items_center()
                    .text_size(px(10.))
                    .text_color(rgb(theme.foreground_muted))
                    .child(hint),
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
                    .child(div().flex_1().min_w_0().h(px(34.)).child(p.input.clone())),
            )
            .into_any_element()
    }

    fn review_view(&self, r: &SaveReview, entity: Entity<Self>, cx: &App) -> AnyElement {
        let theme = crate::theme::current_theme();
        let title = match r.kind {
            ReviewKind::Save => self.buffer_text("保存未完成的工作", "Save your work"),
            ReviewKind::Close(_) => {
                self.buffer_text("关闭前，处理这份修改", "Save before closing?")
            }
            _ => self.buffer_text("退出前保存更改？", "Save changes before quitting?"),
        };
        let cancel_entity = entity.clone();
        let submit_entity = entity.clone();
        let discard_entity = entity.clone();
        let has_save = r.entries.iter().any(|e| e.save && !e.done);
        let has_discard = r.entries.iter().any(|e| !e.save && !e.done);
        let rows = r
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, e)| {
                let session = self.buffer_session(e.id, cx)?;
                let s = session.read(cx);
                let choices = if e.done {
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(theme.foreground_dim))
                        .child(self.buffer_text("已保存", "Saved"))
                } else {
                    div()
                        .flex()
                        .flex_none()
                        .gap(px(3.))
                        .p(px(2.))
                        .rounded(px(6.))
                        .bg(rgb(theme.hover))
                        .children([true, false].into_iter().map(|save| {
                            let choice_entity = entity.clone();
                            let id = e.id;
                            let label = if save {
                                self.buffer_text("保存", "Save")
                            } else if r.kind == ReviewKind::Save {
                                self.buffer_text("跳过", "Skip")
                            } else {
                                self.buffer_text("丢弃", "Discard")
                            };
                            review_button(
                                (
                                    if save {
                                        "buffer-save-choice"
                                    } else {
                                        "buffer-discard-choice"
                                    },
                                    index,
                                ),
                                label,
                                e.save == save,
                                r.running,
                            )
                            .debug_selector(move || {
                                format!(
                                    "buffer-review-{}-{index}",
                                    if save { "save" } else { "discard" }
                                )
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                move |_, _, cx| {
                                    cx.stop_propagation();
                                    choice_entity.update(cx, |w, cx| {
                                        w.set_buffer_review_choice(id, save, cx)
                                    });
                                },
                            )
                        }))
                };
                Some(
                    div()
                        .h(px(54.))
                        .when(index == r.selected, |row| {
                            row.bg(rgb(theme.accent_bg)).rounded(px(6.))
                        })
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .border_b_1()
                        .border_color(rgb(theme.divider))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(13.))
                                        .text_color(rgb(theme.foreground_dim))
                                        .child(s.display_name()),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(10.))
                                        .text_color(rgb(theme.foreground_muted))
                                        .child(
                                            s.file_path()
                                                .map(|p| p.to_string_lossy().into_owned())
                                                .unwrap_or_else(|| {
                                                    self.buffer_text(
                                                        "保存时选择位置",
                                                        "Choose a location when saving",
                                                    )
                                                    .to_owned()
                                                }),
                                        ),
                                ),
                        )
                        .child(choices),
                )
            })
            .collect::<Vec<_>>();
        let label = if r.running {
            self.buffer_text("正在保存…", "Saving…")
        } else {
            match r.kind {
                ReviewKind::Save if has_save => self.buffer_text("保存所选", "Save selected"),
                ReviewKind::Save => self.buffer_text("完成", "Done"),
                ReviewKind::Close(_) if has_save => self.buffer_text("保存并关闭", "Save & close"),
                ReviewKind::Close(_) => self.buffer_text("不保存关闭", "Close without saving"),
                _ if has_save && has_discard => {
                    self.buffer_text("确认选择并退出", "Apply choices & quit")
                }
                _ if has_save => self.buffer_text("保存并退出", "Save & quit"),
                _ => self.buffer_text("不保存退出", "Quit without saving"),
            }
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(52.))
                    .flex_none()
                    .px(px(18.))
                    .flex()
                    .items_center()
                    .text_size(px(13.))
                    .text_color(rgb(theme.foreground_dim))
                    .child(title),
            )
            .child(div().px(px(18.)).pb(px(12.)).flex_none().text_size(px(12.)).text_color(rgb(theme.foreground_dim))
                .child(if r.kind == ReviewKind::Save {
                    self.buffer_text("选择要保存的文档；跳过的修改会保留。", "Choose documents to save. Skipped edits will be kept.")
                } else {
                    self.buffer_text("为每份文档选择保存或丢弃。丢弃的修改无法恢复。", "Choose Save or Discard for each document. Discarded edits cannot be recovered.")
                }))
            .child(
                div()
                    .id("buffer-review-list")
                    .track_scroll(&self.buffers.scroll)
                    .flex_1()
                    .min_h_0()
                    .px(px(18.))
                    .overflow_y_scroll()
                    .children(rows),
            )
            .child(
                div()
                    .h(px(34.))
                    .px(px(18.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .text_size(px(10.))
                    .text_color(rgb(if r.error.is_some() {
                        theme.error
                    } else {
                        theme.foreground_muted
                    }))
                    .child(r.error.clone().unwrap_or_else(|| {
                        self.buffer_text(
                            "↑ ↓ 选择 · Space 切换 · Enter 确认 · C-g 取消",
                            "↑ ↓ Select · Space Toggle · Enter Confirm · C-g Cancel",
                        )
                        .to_owned()
                    })),
            )
            .child(
                div()
                    .min_h(px(60.))
                    .py(px(10.))
                    .flex_wrap()
                    .flex_none()
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(10.))
                    .border_t_1()
                    .border_color(rgb(theme.border))
                    .when(r.kind != ReviewKind::Save && has_save, |footer| footer.child(
                        review_button("buffer-review-discard-all", if matches!(r.kind, ReviewKind::Close(_)) {
                            self.buffer_text("不保存关闭", "Close without saving")
                        } else { self.buffer_text("不保存退出", "Quit without saving") }, false, r.running)
                            .debug_selector(|| "buffer-review-discard-all".into())
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                cx.stop_propagation();
                                discard_entity.update(cx, |w, cx| w.discard_buffer_review(window, cx));
                            })
                    ))
                    .child(div().flex_1())
                    .child(
                        button("buffer-review-cancel", self.buffer_text("取消", "Cancel"))
                            .debug_selector(|| "buffer-review-cancel".into())
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                                cancel_entity.update(cx, |w, cx| w.cancel_buffer_panel(cx));
                            }),
                    )
                    .child(
                        review_button("buffer-review-submit", label, true, r.running)
                            .debug_selector(|| "buffer-review-submit".into())
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                cx.stop_propagation();
                                submit_entity
                                    .update(cx, |w, cx| w.process_buffer_review(window, cx));
                            }),
                    ),
            )
            .into_any_element()
    }
}
