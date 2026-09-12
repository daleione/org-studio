use super::*;
use gpui::{AnyElement, MouseButton, div, prelude::*, px, rgb};

fn button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
) -> gpui::Stateful<gpui::Div> {
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
        .text_color(rgb(0x71829a))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0xeaf0f8)))
        .child(label.into())
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
            .shell
            .owns(crate::app::status_line::shell::ShellKind::Buffers, pane)
        {
            return None;
        }
        let available = (width - 2. * crate::app::status_line::FLOATING_STATUS_INSET).max(1.);
        let (shape, _) = self
            .status
            .shell
            .motion
            .sample(available, std::time::Instant::now());
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
                            .text_color(rgb(0x71829a))
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
                            .child(div().text_color(rgb(0x9aa7b6)).child("↵")),
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
                        .text_color(rgb(0x71829a))
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
                .child(div().text_color(rgb(0x9aa7b6)).child("↵"))
                .child(close)
                .into_any_element();
        }
        let items = self.buffer_candidates(cx);
        let total = items.len();
        let heading = if p.intent == PickerIntent::Close {
            self.buffer_text("关闭文档", "Close document")
        } else if p.intent == PickerIntent::File {
            self.buffer_text("打开 / 新建文件", "Open / create file")
        } else if p.recent {
            self.buffer_text("最近", "Recent")
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
                    .when(selected, |s| s.bg(rgb(0xebf1fd)))
                    .hover(|s| s.bg(rgb(0xeff3f9)))
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
                                    .text_color(rgb(0x48596e))
                                    .child(c.name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(10.))
                                    .text_color(rgb(0x98a5b6))
                                    .child(path),
                            ),
                    )
                    .child(
                        div()
                            .w(px(66.))
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(rgb(if c.dirty { 0xad8a4e } else { 0x94a2b4 }))
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
        let tabs = [
            (false, self.buffer_text("已打开", "Open")),
            (true, self.buffer_text("最近", "Recent")),
        ];
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
                    .text_color(rgb(0x596b81))
                    .child(heading)
                    .child(div().text_color(rgb(0x9ca9b9)).child(total.to_string()))
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
                                .text_color(rgb(0x96a4b6))
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
                    .text_color(rgb(0x97a4b6))
                    .child(hint),
            )
            .child(
                div()
                    .h(px(60.))
                    .flex_none()
                    .px(px(12.))
                    .border_t_1()
                    .border_color(rgb(0xe5ebf3))
                    .bg(rgb(0xf3f6fa))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .when(p.intent != PickerIntent::File, |footer| {
                        footer.child(
                            div()
                                .flex()
                                .flex_none()
                                .p(px(2.))
                                .rounded(px(6.))
                                .bg(rgb(0xeaf0f5))
                                .children(tabs.into_iter().map(|(recent, label)| {
                                    let entity = entity.clone();
                                    button(
                                        if recent {
                                            "buffer-recent"
                                        } else {
                                            "buffer-open"
                                        },
                                        label,
                                    )
                                    .when(recent == p.recent, |b| {
                                        b.bg(rgb(0xffffff)).text_color(rgb(0x5278b5))
                                    })
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        move |_, _, cx| {
                                            cx.stop_propagation();
                                            entity.update(cx, |w, cx| {
                                                if let Some(Panel::Picker(p)) = &mut w.buffers.panel
                                                    && p.intent != PickerIntent::Close
                                                {
                                                    p.recent = recent;
                                                    p.selected = 0;
                                                }
                                                w.buffers.focus_pending = true;
                                                cx.notify();
                                            });
                                        },
                                    )
                                })),
                        )
                    })
                    .child(div().flex_1().min_w_0().h(px(34.)).child(p.input.clone())),
            )
            .into_any_element()
    }

    fn review_view(&self, r: &SaveReview, entity: Entity<Self>, cx: &App) -> AnyElement {
        let title = match r.kind {
            ReviewKind::Save => self.buffer_text("保存未完成的工作", "Save your work"),
            ReviewKind::Close(_) => {
                self.buffer_text("关闭前，处理这份修改", "Save before closing?")
            }
            _ => self.buffer_text("退出前，处理这些修改", "Before you go, review your changes"),
        };
        let cancel_entity = entity.clone();
        let submit_entity = entity.clone();
        let rows = r
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, e)| {
                let session = self.buffer_session(e.id, cx)?;
                let s = session.read(cx);
                let choice_entity = entity.clone();
                let label = if e.done {
                    self.buffer_text("已保存", "Saved")
                } else if e.save {
                    self.buffer_text("保存", "Save")
                } else if r.kind == ReviewKind::Save {
                    self.buffer_text("暂不保存", "Skip for now")
                } else {
                    self.buffer_text("不保存 · 丢弃", "Discard changes")
                };
                Some(
                    div()
                        .h(px(54.))
                        .when(index == r.selected, |row| {
                            row.bg(rgb(0xebf1fd)).rounded(px(6.))
                        })
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .border_b_1()
                        .border_color(rgb(0xe9eef5))
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
                                        .text_color(rgb(0x506178))
                                        .child(s.display_name()),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(10.))
                                        .text_color(rgb(0x9ca9ba))
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
                        .child(
                            button(("buffer-save-choice", index), label)
                                .w(px(128.))
                                .border_1()
                                .border_color(rgb(0xe0e7f0))
                                .bg(rgb(0xffffff))
                                .text_color(rgb(if e.save { 0x71829a } else { 0xab6970 }))
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    cx.stop_propagation();
                                    choice_entity.update(cx, |w, cx| {
                                        if let Some(r) = w.buffers.review_mut()
                                            && !r.running
                                            && !r.entries[index].done
                                        {
                                            r.entries[index].save = !r.entries[index].save;
                                            r.selected = index;
                                            cx.notify();
                                        }
                                    });
                                }),
                        ),
                )
            })
            .collect::<Vec<_>>();
        let label = if r.running {
            self.buffer_text("正在保存…", "Saving…")
        } else {
            match r.kind {
                ReviewKind::Save => self.buffer_text("确认", "Confirm"),
                ReviewKind::Close(_) => self.buffer_text("确认并关闭", "Confirm & close"),
                _ => self.buffer_text("确认并退出", "Confirm & quit"),
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
                    .text_color(rgb(0x576a82))
                    .child(title),
            )
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
                        0xad5960
                    } else {
                        0x97a4b5
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
                    .h(px(60.))
                    .flex_none()
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(10.))
                    .border_t_1()
                    .border_color(rgb(0xe4ebf4))
                    .child(
                        button("buffer-review-cancel", self.buffer_text("取消", "Cancel"))
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                                cancel_entity.update(cx, |w, cx| w.cancel_buffer_panel(cx));
                            }),
                    )
                    .child(
                        button("buffer-review-submit", label)
                            .bg(rgb(0x4977cf))
                            .text_color(rgb(0xffffff))
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
