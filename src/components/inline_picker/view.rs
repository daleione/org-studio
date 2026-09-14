//! Floating shell and theme-consistent controls for each picker.
use super::*;

fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    div()
        .id(id)
        .h(px(26.))
        .px(px(8.))
        .flex_none()
        .flex()
        .items_center()
        .rounded(px(6.))
        .cursor_pointer()
        .border_1()
        .border_color(rgba(0))
        .bg(rgba(if selected {
            (SELECTED_BACKGROUND << 8) | 0xff
        } else {
            0
        }))
        .text_color(rgb(if selected {
            0xffffff
        } else {
            current_theme().foreground
        }))
        .hover(move |s| {
            s.bg(rgb(if selected {
                SELECTED_HOVER_BACKGROUND
            } else {
                HOVER_BACKGROUND
            }))
        })
        .when(!label.is_empty(), |s| s.child(label))
}
impl Render for InlinePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_on_render {
            self.focus_on_render = false;
            self.focus_input(window, cx);
        }
        let theme = current_theme();
        let content = div()
            .id("inline-picker")
            .debug_selector(|| "inline-picker".into())
            .key_context("InlinePicker")
            .track_focus(&self.focus)
            .w(px(self.width))
            .h(px(self.height))
            .p(px(4.))
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(theme.border))
            .bg(rgb(theme.background))
            .shadow_md()
            .font_family(".SystemUIFont")
            .font_weight(FontWeight::NORMAL)
            .text_size(px(12.))
            .line_height(px(19.))
            .text_color(rgb(theme.foreground))
            .cursor_default()
            .flex()
            .flex_col()
            .gap(px(8.))
            .overflow_y_scroll()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if !this.is_pinned() {
                        window.focus(&this.focus, cx);
                    }
                    cx.stop_propagation();
                }),
            )
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|this, event, _, cx| {
                if this.handle_key(event, cx) {
                    cx.stop_propagation();
                }
            }));
        match self.value.clone() {
            InlineValue::Priority(value) => self.render_priority(content, value, window, cx),
            InlineValue::Tags(value) => self.render_tags(content, value, cx),
            InlineValue::Link(value) => self.render_link(content, value, cx),
        }
    }
}
impl InlinePicker {
    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_none()
            .h(px(26.))
            .flex()
            .justify_end()
            .gap(px(6.))
            .child(
                button("inline-cancel", self.language.text("inline.cancel"), false)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(InlineEvent::Cancel))),
            )
            .child(
                button("inline-apply", self.language.text("inline.apply"), true)
                    .debug_selector(|| "inline-apply".into())
                    .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
            )
    }
}

fn icon(name: &str) -> Svg {
    svg()
        .path(format!("assets/icons/agenda/{name}.svg"))
        .size(px(14.))
        .flex_none()
}

impl InlinePicker {
    fn render_priority(
        &mut self,
        mut content: Stateful<Div>,
        value: PriorityValue,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = current_theme();
        let PriorityValue { current, choices } = value;
        let mut row = div()
            .id("priority-options")
            .track_scroll(&self.priority_scroll)
            .overflow_x_scroll()
            .flex()
            .gap(px(3.))
            .items_center()
            .h_full();
        let count = choices.len();
        for (index, choice) in choices.into_iter().enumerate() {
            let selected = choice == current;
            let label = choice.clone();
            row = row.child(
                button(("priority", index), "", selected)
                    .gap(px(6.))
                    .when(selected, |s| {
                        s.child(div().w(px(12.)).flex_none().child("✓"))
                    })
                    .child(label)
                    .debug_selector({
                        let choice = choice.clone();
                        move || format!("priority-{choice}")
                    })
                    .w(px(Self::priority_width(&choice, &current, window)))
                    .justify_center()
                    .border_color(rgba(if selected {
                        0
                    } else {
                        (theme.foreground_dim << 8) | 0x38
                    }))
                    .when(self.keyboard_navigation && self.selected == index, |s| {
                        s.border_color(rgb(theme.link))
                    })
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(InlineEvent::Priority(Some(choice.clone())))
                    })),
            );
        }
        content = content.child(
            row.child(
                div()
                    .w(px(1.))
                    .h(px(14.))
                    .mx(px(3.))
                    .flex_none()
                    .bg(rgb(theme.border)),
            )
            .child(
                button(
                    "priority-remove",
                    self.language.text("inline.remove"),
                    false,
                )
                .w(px(self.remove_width(window)))
                .justify_center()
                .text_size(px(11.))
                .text_color(rgb(theme.foreground_dim))
                .when(self.keyboard_navigation && self.selected == count, |s| {
                    s.bg(rgb(HOVER_BACKGROUND))
                })
                .debug_selector(|| "priority-remove".into())
                .on_click(cx.listener(|_, _, _, cx| cx.emit(InlineEvent::Priority(None)))),
            ),
        );
        content
    }
    fn render_tags(
        &mut self,
        mut content: Stateful<Div>,
        value: TagsValue,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = current_theme();
        let TagsValue {
            local,
            inherited,
            available,
        } = value;
        self.input.update(cx, |i, _| {
            i.config.placeholder = self.language.text("inline.tags_search").into()
        });
        let query = self.input.read(cx).text.trim().to_owned();
        let lower = query.to_lowercase();
        let mut list = div()
            .id("tag-options")
            .debug_selector(|| "tag-options".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(3.));
        for (index, tag) in available
            .iter()
            .filter(|t| {
                t.to_lowercase().contains(&lower) && (!inherited.contains(t) || local.contains(t))
            })
            .enumerate()
        {
            let selected = local.contains(tag);
            let tag = tag.clone();
            let label = tag.clone();
            list = list.child(
                button(("tag", index), "", selected)
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(12.))
                            .flex_none()
                            .child(if selected { "✓" } else { "" }),
                    )
                    .child(label)
                    .debug_selector({
                        let tag = tag.clone();
                        move || format!("tag-{tag}")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let InlineValue::Tags(TagsValue { local, .. }) = &mut this.value {
                            if local.contains(&tag) {
                                local.retain(|v| v != &tag);
                            } else {
                                local.push(tag.clone());
                            }
                        }
                        cx.notify();
                    })),
            );
        }
        if valid_tag(&query) && !available.contains(&query) {
            list = list.child(
                button(
                    "tag-add",
                    format!("＋ {} “{}”", self.language.text("inline.add"), query),
                    false,
                )
                .debug_selector(|| "tag-add".into())
                .on_click(cx.listener(|this, _, _, cx| this.add_tag(cx))),
            );
        }
        let inherited_section = (!inherited.is_empty()).then(|| {
            let mut chips = div().flex().flex_wrap().gap(px(4.));
            for (index, tag) in inherited.iter().enumerate() {
                chips = chips.child(
                    div()
                        .id(("inherited-tag", index))
                        .debug_selector({
                            let tag = tag.clone();
                            move || format!("inherited-tag-{tag}")
                        })
                        .h(px(22.))
                        .px(px(7.))
                        .max_w_full()
                        .flex()
                        .items_center()
                        .flex_none()
                        .rounded(px(5.))
                        .bg(rgb(theme.background_alt))
                        .text_color(rgb(theme.foreground_dim))
                        .text_size(px(11.))
                        .child(div().overflow_hidden().text_ellipsis().child(tag.clone())),
                );
            }
            div()
                .id("inherited-tags")
                .debug_selector(|| "inherited-tags".into())
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(5.))
                .border_t_1()
                .border_color(rgb(theme.border))
                .pt(px(8.))
                .child(
                    div()
                        .h(px(14.))
                        .line_height(px(14.))
                        .text_size(px(10.))
                        .text_color(rgb(theme.foreground_dim))
                        .child(self.language.text("inline.inherited_short")),
                )
                .child(chips)
        });
        content = content
            .p(px(10.))
            .child(
                div()
                    .flex_none()
                    .h(px(19.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.language.text("inline.tags")),
            )
            .child(div().h(px(30.)).flex_none().child(self.input.clone()))
            .child(list)
            .children(inherited_section)
            .child(self.footer(cx));
        content
    }
    fn render_link(
        &mut self,
        mut content: Stateful<Div>,
        value: LinkValue,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = current_theme();
        let LinkValue {
            target,
            title,
            preview,
            can_open,
            can_edit,
        } = value;
        if self.editing {
            content = content
                .p(px(10.))
                .gap(px(7.))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(theme.foreground_dim))
                        .child(self.language.text("inline.edit_link")),
                )
                .child(self.input.clone())
                .when(self.error, |s| {
                    s.child(
                        div()
                            .text_color(rgb(theme.heading[0]))
                            .child(self.language.text("inline.invalid_link")),
                    )
                })
                .child(self.footer(cx));
        } else {
            let has_title = title.is_some();
            let label = title.unwrap_or_else(|| target.clone());
            let header = div()
                .id("link-summary")
                .debug_selector(|| "link-summary".into())
                .h(px(if has_title { 50. } else { 42. }))
                .flex_none()
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .size(px(24.))
                        .rounded(px(6.))
                        .bg(rgb(HOVER_BACKGROUND))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            icon(if has_title {
                                "file-text"
                            } else {
                                "arrow-square-out"
                            })
                            .text_color(rgb(theme.link)),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .line_height(px(18.))
                                .font_weight(FontWeight::MEDIUM)
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(label),
                        )
                        .when(has_title, |s| {
                            s.child(
                                div()
                                    .text_size(px(10.))
                                    .line_height(px(14.))
                                    .text_color(rgb(theme.foreground_dim))
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(target),
                            )
                        }),
                );
            content = content.p_0().gap_0().child(header);
            if !preview.is_empty() {
                let mut excerpt = div().px(px(12.)).pb(px(10.)).flex_none().flex().flex_col();
                for line in preview {
                    excerpt = excerpt.child(
                        div()
                            .h(px(18.))
                            .line_height(px(18.))
                            .text_size(px(11.))
                            .text_color(rgb(theme.foreground_dim))
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(line),
                    );
                }
                content = content.child(excerpt);
            }
            let row = div()
                .id("link-actions")
                .debug_selector(|| "link-actions".into())
                .h(px(35.))
                .flex_none()
                .border_t_1()
                .border_color(rgb(theme.border))
                .bg(rgb(theme.background_alt))
                .px(px(5.))
                .flex()
                .items_center()
                .gap(px(2.))
                .child(
                    button("link-open", "", false)
                        .gap(px(5.))
                        .text_color(rgb(theme.link))
                        .child(icon("arrow-square-out").text_color(rgb(theme.link)))
                        .child(self.language.text("inline.open"))
                        .debug_selector(|| "link-open".into())
                        .when(!can_open, |s| s.opacity(0.4).cursor_default())
                        .on_click(cx.listener(move |_, _, _, cx| {
                            if can_open {
                                cx.emit(InlineEvent::Open);
                            }
                        })),
                )
                .child(div().flex_1())
                .child(
                    button("link-copy", self.language.text("inline.copy"), false)
                        .text_color(rgb(theme.foreground_dim))
                        .debug_selector(|| "link-copy".into())
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(InlineEvent::Copy))),
                )
                .when(can_edit, |s| {
                    s.child(
                        button("link-edit", self.language.text("inline.edit"), false)
                            .text_color(rgb(theme.foreground_dim))
                            .debug_selector(|| "link-edit".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let InlineValue::Link(LinkValue { target, .. }) = &this.value {
                                    this.input.update(cx, |i, cx| i.sync(target, cx));
                                }
                                this.editing = true;
                                this.focus_input(window, cx);
                                cx.notify();
                            })),
                    )
                });
            content = content.child(row);
        }
        content
    }
}
