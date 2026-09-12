use gpui::{AnyElement, Entity, MouseButton, div, prelude::*, px, rgb};

use super::{
    Presentation,
    labels::{Item, text},
    layout::*,
};
use crate::app::{PaneSide, WorkspaceWindow, status_line};

fn keycap(key: &str, active: bool) -> gpui::Div {
    let label = key_label(key);
    div()
        .flex_none()
        .h(px(KEY_HEIGHT))
        .min_w(px(32.))
        .px(px(KEY_PADDING))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .font_family(KEY_FONT)
        .text_size(px(KEY_SIZE))
        .bg(rgb(if active { 0x4977cf } else { 0xeaf0fb }))
        .text_color(rgb(if active { 0xffffff } else { 0x4977cf }))
        .child(label.to_owned())
}

impl WorkspaceWindow {
    pub(crate) fn prefix_hint_panel(
        &self,
        pane: PaneSide,
        width: f32,
        status: Option<AnyElement>,
        entity: Entity<Self>,
    ) -> Option<AnyElement> {
        let p = self
            .prefix_hint
            .presentation
            .as_ref()
            .filter(|p| p.pane == pane)?;
        let available = (width - 2. * status_line::FLOATING_STATUS_INSET).max(1.);
        let (shape, _) = self
            .status
            .shell
            .motion
            .sample(available, std::time::Instant::now());
        Some(
            status_line::floating_status_container(shape.height)
                .occlude()
                .left(px((width - shape.width) / 2.))
                .right(gpui::auto())
                .w(px(shape.width))
                .when(status.is_none(), |shell| {
                    shell.opacity(shape.content_opacity)
                })
                .child(
                    div()
                        .size_full()
                        .opacity(shape.content_opacity)
                        .child(self.prefix_hint_body(p, entity)),
                )
                .when(shape.content_opacity < 1., |shell| {
                    shell.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .w_full()
                            .h(px(status_line::FLOATING_STATUS_HEIGHT))
                            .opacity(1. - shape.content_opacity)
                            .children(status),
                    )
                })
                .into_any_element(),
        )
    }

    fn prefix_hint_body(&self, p: &Presentation, entity: Entity<Self>) -> AnyElement {
        let body = div()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .children(p.layout.rows.iter().map(|row| {
                div()
                    .h(px(row.height))
                    .flex_none()
                    .flex()
                    .gap(px(GAP))
                    .children(row.columns.iter().map(|column| {
                        div()
                            .w(px(p.layout.column_width))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .when_some(column.group, |view, group| {
                                view.child(
                                    div()
                                        .h(px(HEADER_HEIGHT))
                                        .flex_none()
                                        .px(px(ROW_PADDING))
                                        .text_size(px(KEY_SIZE))
                                        .text_color(rgb(0x8796a8))
                                        .child(group.title(self.language)),
                                )
                            })
                            .children(
                                p.layout.items[column.items.clone()]
                                    .iter()
                                    .map(|item| self.prefix_item_row(item, p, entity.clone())),
                            )
                    }))
            }));
        let prefix_keys = p.prefix.split_whitespace().collect::<Vec<_>>();
        let mut breadcrumb = div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .overflow_hidden();
        for (index, key) in prefix_keys.iter().enumerate() {
            if index > 0 {
                breadcrumb = breadcrumb.child(div().text_color(rgb(0x9aa7b6)).child("›"));
            }
            breadcrumb = breadcrumb.child(keycap(key, index + 1 == prefix_keys.len()));
        }
        let caption = super::labels::prefix_group(&p.prefix)
            .map_or(text(self.language, "继续按键", "Next key"), |group| {
                group.title(self.language)
            });
        let cancel = entity.clone();
        div()
            .size_full()
            .font_family(FONT)
            .text_size(px(FONT_SIZE))
            .text_color(rgb(0x53657b))
            .flex()
            .flex_col()
            .child(
                div()
                    .id("prefix-hint-items")
                    .debug_selector(|| "prefix-hint-items".into())
                    .flex_1()
                    .min_h_0()
                    .px(px(PADDING))
                    .py(px(PADDING))
                    .overflow_y_scroll()
                    .track_scroll(&p.scroll)
                    .child(body),
            )
            .child(
                div()
                    .h(px(FOOTER_HEIGHT))
                    .flex_none()
                    .mx(px(PADDING))
                    .border_t_1()
                    .border_color(rgb(0xdce4ee))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(breadcrumb.when(p.layout.width > 360., |row| {
                        row.child(div().ml(px(5.)).child(caption))
                    }))
                    .child(
                        div()
                            .id("prefix-hint-cancel")
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .cursor_pointer()
                            .child(keycap("C-g", false))
                            .child(text(self.language, "取消", "Cancel"))
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                cx.stop_propagation();
                                cancel.update(cx, |w, cx| w.cancel_prefix_input(window, cx));
                            }),
                    ),
            )
            .into_any_element()
    }

    fn prefix_item_row(&self, item: &Item, p: &Presentation, entity: Entity<Self>) -> AnyElement {
        let key = item.key.clone();
        let selector = format!("prefix-hint-key-{}", item.key);
        let prefix = p.prefix.clone();
        let enabled = !item.disabled && !p.returning;
        div()
            .id(format!("prefix-hint-key-{}", item.key))
            .h(px(ROW_HEIGHT))
            .flex_none()
            .w_full()
            .min_w_0()
            .debug_selector(move || selector.clone())
            .px(px(ROW_PADDING))
            .flex()
            .items_center()
            .gap(px(ITEM_GAP))
            .rounded(px(6.))
            .when(!enabled, |row| row.opacity(0.45))
            .when(enabled, |row| {
                row.cursor_pointer().hover(|s| s.bg(rgb(0xeaf1fc)))
            })
            .child(keycap(&item.key, false).w(px(p.layout.key_width)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(item.title.clone()),
            )
            .when(item.prefix, |row| {
                row.child(div().text_color(rgb(0x8796a8)).child("›"))
            })
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                if !enabled {
                    return;
                }
                entity.update(cx, |w, cx| {
                    if w.keyboard.pending_keys() != Some(prefix.as_str()) {
                        return;
                    }
                    let Ok(stroke) = crate::keymap::KeyStroke::parse(&key) else {
                        return;
                    };
                    w.route_stroke(stroke, window, cx);
                });
            })
            .into_any_element()
    }
}
