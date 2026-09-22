use super::*;
use gpui::{AnyElement, MouseButton, div, prelude::*, px, rgb};

impl WorkspaceWindow {
    pub(super) fn quick_picker_view(
        &self,
        p: &Picker,
        entity: Entity<Self>,
        close: AnyElement,
        cx: &App,
    ) -> AnyElement {
        let theme = crate::theme::current_theme();
        let headings = p.input.read(cx).text.trim_start().starts_with('@');
        let narrow = self.buffers.available < 480.;
        let items = self.buffer_candidates(cx);
        let total = items.len();
        let scope = if headings {
            self.document_session()
                .map(|s| s.read(cx).display_name())
                .unwrap_or_default()
        } else {
            self.buffer_text("已打开 + 最近", "Open + Recent")
                .to_owned()
        };
        let home = std::env::var_os("HOME");
        let mut rows = Vec::new();
        for (index, item) in items.into_iter().enumerate() {
            let entity = entity.clone();
            let state = if let Some(heading) = &item.heading {
                format!("{} {}", self.buffer_text("行", "Ln"), heading.line)
            } else {
                if item.dirty {
                    self.buffer_text("已修改", "Modified")
                } else if item.current {
                    self.buffer_text("当前", "Current")
                } else if item.id.is_some() {
                    self.buffer_text("已打开", "Open")
                } else {
                    self.buffer_text("最近", "Recent")
                }
                .to_owned()
            };
            let detail = if headings {
                item.path
            } else {
                let parent = Path::new(&item.path).parent().unwrap_or(Path::new(""));
                home.as_ref()
                    .and_then(|home| {
                        parent
                            .strip_prefix(home)
                            .ok()
                            .map(|p| format!("~/{}", p.display()))
                    })
                    .unwrap_or_else(|| parent.display().to_string())
            };
            rows.push(
                div()
                    .id(("quick-open-row", index))
                    .debug_selector(move || format!("quick-open-row-{index}"))
                    .h(px(34.))
                    .flex_none()
                    .px(px(10.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .cursor_pointer()
                    .when(index == p.selected, |row| row.bg(rgb(theme.accent_bg)))
                    .hover(|row| row.bg(rgb(theme.hover)))
                    .child(
                        div()
                            .w(px(18.))
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(rgb(theme.accent))
                            .child(if index < 10 {
                                ((index + 1) % 10).to_string()
                            } else {
                                String::new()
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(rgb(theme.foreground))
                            .child(item.name),
                    )
                    .when(!narrow, |row| {
                        row.child(
                            div()
                                .w(px(200.))
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(rgb(theme.foreground_muted))
                                .child(detail),
                        )
                    })
                    .child(
                        div()
                            .w(px(if narrow { 54. } else { 68. }))
                            .flex_none()
                            .text_right()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(if item.dirty {
                                theme.warning
                            } else {
                                theme.foreground_dim
                            }))
                            .child(state),
                    )
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        entity.update(cx, |w, cx| {
                            if let Some(Panel::Picker(p)) = &mut w.buffers.panel {
                                p.selected = index;
                            }
                            w.accept_buffer_picker(cx);
                        });
                    }),
            );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(40.))
                    .flex_none()
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .text_size(px(12.))
                    .text_color(rgb(theme.foreground_dim))
                    .child(if headings {
                        self.buffer_text("当前文档标题", "Document headings")
                    } else {
                        self.buffer_text("快速打开", "Quick open")
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_right()
                            .text_color(rgb(theme.foreground_muted))
                            .child(scope),
                    )
                    .child(close),
            )
            .child(
                div()
                    .id("quick-open-candidates")
                    .flex_1()
                    .min_h_0()
                    .px(px(8.))
                    .overflow_y_scroll()
                    .track_scroll(&self.buffers.scroll)
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                    .when(total == 0, |list| {
                        list.child(
                            div()
                                .h(px(120.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(12.))
                                .text_color(rgb(theme.foreground_muted))
                                .child(if headings {
                                    self.buffer_text("没有匹配的标题", "No matching headings")
                                } else {
                                    self.buffer_text("没有匹配的文档", "No matching documents")
                                }),
                        )
                    })
                    .children(rows),
            )
            .child(
                div()
                    .h(px(28.))
                    .flex_none()
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .text_size(px(10.))
                    .text_color(rgb(theme.foreground_muted))
                    .child(self.buffer_text(
                        "↑↓ 选择 · ↵ 确认 · ⌘1–9 / ⌘0 快选 · Esc 取消",
                        "↑↓ Select · ↵ Confirm · ⌘1–9 / ⌘0 Pick · Esc Cancel",
                    )),
            )
            .child(
                div()
                    .h(px(48.))
                    .flex_none()
                    .px(px(14.))
                    .border_t_1()
                    .border_color(rgb(theme.floating_border))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .debug_selector(|| "quick-open-input-label".into())
                            .h(px(22.))
                            .line_height(px(22.))
                            .flex_none()
                            .text_size(px(13.))
                            .text_color(rgb(theme.accent))
                            .child(if headings {
                                self.buffer_text("跳转", "Go")
                            } else {
                                self.buffer_text("打开", "Open")
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(34.))
                            .flex()
                            .items_center()
                            .child(p.input.clone()),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn navigation_status_width(
        &self,
        pane: crate::app::PaneSide,
        width: f32,
        cx: &App,
    ) -> f32 {
        if pane != self.document_workspace.active_pane
            || !self.navigation_can_go(false, cx) && !self.navigation_can_go(true, cx)
        {
            return 0.;
        }
        if width >= 1000. { 230. } else { 60. }
    }

    pub(crate) fn with_navigation_status(
        &self,
        content: gpui::AnyElement,
        pane: crate::app::PaneSide,
        width: f32,
        entity: Entity<Self>,
        cx: &App,
    ) -> gpui::AnyElement {
        let width = self.navigation_status_width(pane, width, cx);
        if width == 0. {
            return content;
        }
        let theme = crate::theme::current_theme();
        let mut controls = div()
            .w(px(width))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(4.));
        for forward in [false, true] {
            let enabled = self.navigation_can_go(forward, cx);
            let entity = entity.clone();
            controls = controls.child(
                div()
                    .id(if forward {
                        "navigation-forward"
                    } else {
                        "navigation-back"
                    })
                    .debug_selector(move || {
                        if forward {
                            "navigation-forward"
                        } else {
                            "navigation-back"
                        }
                        .into()
                    })
                    .size(px(28.))
                    .flex_none()
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(theme.foreground_dim))
                    .text_size(px(16.))
                    .opacity(if enabled { 1. } else { 0.3 })
                    .when(enabled, |button| {
                        button.cursor_pointer().hover(|s| s.bg(rgb(theme.hover)))
                    })
                    .child(if forward { "→" } else { "←" })
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        if enabled {
                            entity.update(cx, |w, cx| w.navigate_history(forward, cx));
                        }
                    }),
            );
        }
        if width > 60.
            && let Some(label) = self.navigation_back_label(cx)
        {
            let entity = entity.clone();
            controls = controls.child(
                div()
                    .id("navigation-return")
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .px(px(8.))
                    .text_size(px(11.))
                    .text_color(rgb(theme.accent))
                    .cursor_pointer()
                    .child(format!("{} {label}", self.buffer_text("返回", "Back to")))
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        entity.update(cx, |w, cx| w.navigate_history(false, cx));
                    }),
            );
        }
        div()
            .size_full()
            .flex()
            .items_center()
            .child(controls)
            .child(div().flex_1().min_w_0().child(content))
            .into_any_element()
    }
}
