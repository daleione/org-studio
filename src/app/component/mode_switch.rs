use gpui::{
    AnimationExt, Div, ElementId, InteractiveElement, IntoElement, ParentElement, Pixels, Rgba,
    SharedString, SpringAnimation, SpringConfig, Stateful, StatefulInteractiveElement, Styled, div,
    prelude::FluentBuilder, px, rgb, rgba,
};

const MODE_SWITCH_SPRING: SpringConfig = SpringConfig::new(500.0, 45.0, 1.0);

#[derive(Clone)]
pub(crate) struct ModeSwitch {
    id: SharedString,
    selected_index: usize,
    item_width: Pixels,
    motion_enabled: bool,
}

impl ModeSwitch {
    pub(crate) fn new(
        id: impl Into<SharedString>,
        selected_index: usize,
        item_width: Pixels,
    ) -> Self {
        Self {
            id: id.into(),
            selected_index,
            item_width,
            motion_enabled: true,
        }
    }

    pub(crate) fn motion_enabled(mut self, enabled: bool) -> Self {
        self.motion_enabled = enabled;
        self
    }

    pub(crate) fn item<E>(
        &self,
        index: usize,
        id: impl Into<ElementId>,
        content: impl FnOnce(Rgba) -> E,
    ) -> Stateful<Div>
    where
        E: IntoElement,
    {
        let theme = crate::theme::current_theme();
        let selected = index == self.selected_index;
        let foreground = foreground(selected);
        div()
            .id(id)
            .flex_none()
            .w(self.item_width)
            .h(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .text_color(foreground)
            .cursor_pointer()
            .when(!selected, |item| {
                // Keep the hover wash translucent so the moving indicator stays visible.
                item.hover(|style| style.bg(rgba((theme.hover << 8) | 0x80)))
            })
            .active(|style| style.opacity(0.72))
            .child(content(foreground))
    }

    pub(crate) fn render<I, E>(&self, items: I) -> Div
    where
        I: IntoIterator<Item = E>,
        E: IntoElement,
    {
        let theme = crate::theme::current_theme();
        let items = items.into_iter().collect::<Vec<_>>();
        let mut control = div()
            .flex_none()
            .h(px(30.))
            .p(px(2.))
            .flex()
            .items_center()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(theme.border))
            .bg(rgb(theme.surface))
            .relative();

        if self.selected_index < items.len() {
            let animation_id = self.id.clone();
            let indicator_selector = format!("{}-indicator", self.id);
            let indicator_left = indicator_left(self.selected_index, self.item_width);
            let indicator = div()
                .debug_selector(move || indicator_selector.clone())
                .absolute()
                .top(px(2.))
                .w(self.item_width)
                .h(px(26.))
                .rounded(px(6.))
                .bg(rgb(theme.elevated))
                .shadow_sm();
            control = if self.motion_enabled {
                control.child(
                    indicator.with_spring(
                        animation_id,
                        SpringAnimation::new(MODE_SWITCH_SPRING)
                            .to(indicator_left)
                            .with_epsilon(0.1),
                        |indicator, left| indicator.left(left),
                    ),
                )
            } else {
                control.child(indicator.left(indicator_left))
            };
        }

        control.children(items)
    }
}

fn foreground(selected: bool) -> Rgba {
    let theme = crate::theme::current_theme();
    rgb(if selected {
        theme.accent
    } else {
        theme.foreground
    })
}

fn indicator_left(selected_index: usize, item_width: Pixels) -> Pixels {
    px(2.) + item_width * selected_index as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::SpringState;

    #[test]
    fn selected_and_unselected_content_have_visible_foregrounds() {
        let theme = crate::theme::current_theme();
        assert_eq!(foreground(true), rgb(theme.accent));
        assert_eq!(foreground(false), rgb(theme.foreground));
        assert_ne!(foreground(true), foreground(false));
    }

    #[test]
    fn indicator_target_advances_by_one_item_width() {
        let item_width = px(44.);
        assert_eq!(indicator_left(0, item_width), px(2.));
        assert_eq!(indicator_left(1, item_width), px(46.));
        assert_eq!(indicator_left(2, item_width), px(90.));
    }

    #[test]
    fn motion_can_be_disabled_for_accessibility() {
        let control = ModeSwitch::new("test", 0, px(44.)).motion_enabled(false);
        assert!(!control.motion_enabled);
    }

    #[test]
    fn spring_moves_the_indicator_through_an_intermediate_position() {
        let start = f32::from(indicator_left(0, px(44.)));
        let target = f32::from(indicator_left(2, px(44.)));
        let after_fifty_ms = MODE_SWITCH_SPRING.step(
            SpringState {
                position: start,
                velocity: 0.,
            },
            target,
            0.05,
        );
        assert!(after_fifty_ms.position > start);
        assert!(after_fifty_ms.position < target);
    }
}
