use super::*;

const ROW_HEIGHT: f32 = 30.;
const HEADER_HEIGHT: f32 = 34.;
const MENU_WIDTH: f32 = 264.;

pub(super) fn menu_bounds(
    viewport: Bounds<Pixels>,
    anchor: Bounds<Pixels>,
    height: f32,
) -> Bounds<Pixels> {
    let width = px(MENU_WIDTH).min((viewport.size.width - px(16.)).max(px(1.)));
    let x = anchor
        .left()
        .min(viewport.right() - width - px(8.))
        .max(viewport.left() + px(8.));
    let y = anchor.bottom() + px(6.);
    let height = px(height).min((viewport.bottom() - y - px(8.)).max(px(0.)));
    Bounds::new(point(x, y), size(width, height))
}

impl SemanticEditor {
    pub(in crate::editor) fn table_overlays(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(viewport) = self.viewport else {
            return Vec::new();
        };
        if self
            .table_actions
            .popup
            .as_ref()
            .is_some_and(|popup| !popup.focus_pending && !popup.focus.is_focused(window))
        {
            self.table_actions = TableActions::default();
        }
        if self
            .table_actions
            .popup
            .as_ref()
            .is_some_and(|popup| popup.viewport != viewport)
        {
            self.dismiss_table_actions(cx);
        }
        let theme = current_theme();
        let mut overlays = Vec::new();
        let target = self
            .table_actions
            .popup
            .as_ref()
            .map(|p| &p.target)
            .or(self.table_actions.hover.as_ref());
        if let Some(target) = target.filter(|hit| {
            hit.button.size.width >= px(3.)
                && hit.button.intersect(&viewport) == hit.button
                && hit.button.bottom()
                    <= viewport.bottom() - px(self.display_map.bottom_overlay_clearance)
        }) {
            let hit = target.clone();
            let open = self.table_actions.popup.is_some();
            let icon_height = (hit.button.size.height * 0.85)
                .clamp(px(16.), px(24.))
                .min(hit.button.size.height);
            let button = div()
                .id("table-cell-menu-button")
                .debug_selector(|| "table-cell-menu-button".into())
                .w(hit.button.size.width)
                .h(hit.button.size.height)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if this.table_actions.popup.is_some() {
                            this.dismiss_table_actions(cx);
                        } else {
                            this.open_table_menu(hit.clone(), cx);
                        }
                    }),
                )
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .on_hover(cx.listener(|this, hovering: &bool, window, cx| {
                    if !hovering {
                        this.table_hover(window.mouse_position(), cx);
                    }
                }))
                .child(
                    gpui::svg()
                        .debug_selector(|| "table-cell-menu-icon".into())
                        .data(include_bytes!("../../../assets/editor/table-menu.svg"))
                        .w((icon_height * 0.3).min(target.button.size.width))
                        .h(icon_height)
                        .flex_shrink_0()
                        .text_color(rgb(if open { theme.accent } else { theme.foreground })),
                );
            overlays.push(
                deferred(anchored().position(target.button.origin).child(button))
                    .with_priority(9)
                    .into_any_element(),
            );
        }
        let Some(popup) = self.table_actions.popup.as_mut() else {
            return overlays;
        };
        if popup.focus_pending {
            popup.focus_pending = false;
            window.focus(&popup.focus, cx);
        }
        let sort_items = sort_items();
        let items: &[MenuItem] = if popup.sorting {
            &sort_items
        } else {
            &popup.items
        };
        let chinese = self.ui_language == crate::i18n::Language::Chinese;
        let mut last_group = items.first().map_or(0, |item| item.group);
        let mut height = HEADER_HEIGHT + 12.;
        let mut rows = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let separator = item.group != last_group;
            last_group = item.group;
            height += ROW_HEIGHT + if separator { 9. } else { 0. };
            let selected = index == popup.selected;
            let action = item.action;
            let destructive = matches!(
                action,
                MenuAction::Edit(TableEdit::KillRow | TableEdit::DeleteColumn)
            );
            let row = div()
                .id(format!("table-menu-item-{index}"))
                .debug_selector(move || format!("table-menu-item-{index}"))
                .when(separator, |row| {
                    row.mt(px(8.))
                        .border_t_1()
                        .border_color(rgb(theme.floating_border))
                })
                .child(
                    div()
                        .h(px(ROW_HEIGHT))
                        .px(px(10.))
                        .rounded(px(5.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .cursor_pointer()
                        .text_color(rgb(if destructive {
                            theme.link_warn
                        } else {
                            theme.foreground
                        }))
                        .when(selected, |row| row.bg(rgba((theme.foreground << 8) | 0x10)))
                        .on_mouse_move(cx.listener(move |this, _, _, cx| {
                            if let Some(popup) = &mut this.table_actions.popup
                                && popup.selected != index
                            {
                                popup.selected = index;
                                cx.notify();
                            }
                            cx.stop_propagation();
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.table_menu_choose(action, cx);
                            }),
                        )
                        .child(div().flex_1().min_w_0().truncate().child(if chinese {
                            item.zh
                        } else {
                            item.en
                        }))
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(rgb(theme.foreground_muted))
                                .child(item.shortcut),
                        ),
                );
            rows.push(row);
        }
        let menu_viewport = Bounds::new(
            viewport.origin,
            size(
                viewport.size.width,
                (viewport.size.height - px(self.display_map.bottom_overlay_clearance)).max(px(1.)),
            ),
        );
        let bounds = menu_bounds(menu_viewport, popup.target.button, height);
        let title = if popup.sorting {
            if chinese {
                "‹ 按当前列排序".to_owned()
            } else {
                "‹ Sort by this column".to_owned()
            }
        } else if chinese {
            format!(
                "源文件第 {} 行 · 第 {} 列",
                popup.target.line.0 + 1,
                popup.target.column + 1
            )
        } else {
            format!(
                "Source line {} · Column {}",
                popup.target.line.0 + 1,
                popup.target.column + 1
            )
        };
        let header = div()
            .id("table-menu-header")
            .h(px(HEADER_HEIGHT))
            .flex_shrink_0()
            .px(px(14.))
            .flex()
            .items_center()
            .text_size(px(11.))
            .text_color(rgb(theme.foreground_dim))
            .child(title)
            .when(popup.sorting, |header| {
                header.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(popup) = &mut this.table_actions.popup {
                            popup.sorting = false;
                            popup.selected = 0;
                            popup.scroll.set_offset(Point::default());
                            cx.notify();
                        }
                    }),
                )
            });
        let menu = div()
            .id("table-cell-menu")
            .debug_selector(|| "table-cell-menu".into())
            .track_focus(&popup.focus)
            .key_context("TableMenu")
            .w(bounds.size.width)
            .h(bounds.size.height)
            .flex()
            .flex_col()
            .overflow_hidden()
            .occlude()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(theme.floating_border))
            .bg(rgba(theme.floating_background))
            .shadow_lg()
            .font_family(".SystemUIFont")
            .text_size(px(13.))
            .line_height(px(18.))
            .on_key_down(cx.listener(Self::table_menu_key))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                if !this
                    .table_actions
                    .popup
                    .as_ref()
                    .is_some_and(|p| p.target.button.contains(&event.position))
                {
                    this.dismiss_table_actions(cx);
                }
            }))
            .on_mouse_move(|_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(header)
            .child(
                div()
                    .id("table-menu-scroll")
                    .flex_1()
                    .min_h_0()
                    .px(px(6.))
                    .pb(px(6.))
                    .overflow_y_scroll()
                    .track_scroll(&popup.scroll)
                    .children(rows),
            );
        overlays.push(
            deferred(anchored().position(bounds.origin).child(menu))
                .with_priority(10)
                .into_any_element(),
        );
        overlays
    }
}
