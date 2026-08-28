use gpui::Context;

use super::PreviewApp;

pub(in crate::preview) const DEFAULT_WIDTH_PX: f32 = 240.0;
pub(in crate::preview) const MIN_WIDTH_PX: f32 = 180.0;
pub(in crate::preview) const MAX_WIDTH_PX: f32 = 420.0;
pub(in crate::preview) const MAX_WINDOW_FRACTION: f32 = 0.40;
pub(in crate::preview) const RESIZE_HANDLE_PX: f32 = 6.0;
pub(in crate::preview) const MIN_DOCUMENT_WIDTH_PX: f32 = 120.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::preview) struct ResizeSession {
    pub(in crate::preview) start_pointer_x: f32,
    pub(in crate::preview) start_width: f32,
    pub(in crate::preview) current_width: f32,
}

pub(in crate::preview) fn width_for_viewport(viewport_width: f32, desired: f32) -> f32 {
    let fraction_max = viewport_width * MAX_WINDOW_FRACTION;
    let document_max = viewport_width - RESIZE_HANDLE_PX - MIN_DOCUMENT_WIDTH_PX;
    let maximum = fraction_max.min(document_max).clamp(0.0, MAX_WIDTH_PX);
    let minimum = MIN_WIDTH_PX.min(maximum);
    desired.clamp(minimum, maximum)
}

pub(in crate::preview) fn width_from_resize(
    viewport_width: f32,
    session: ResizeSession,
    pointer_x: f32,
) -> f32 {
    width_for_viewport(
        viewport_width,
        session.start_width + pointer_x - session.start_pointer_x,
    )
}

impl PreviewApp {
    pub(super) fn focus_sidebar(&mut self, cx: &mut Context<Self>) {
        if self.sidebar_visible && !self.sidebar_focused {
            self.sidebar_focused = true;
            self.install_sidebar_keymap();
            cx.notify();
        }
    }

    pub(super) fn focus_document(&mut self, cx: &mut Context<Self>) {
        if self.sidebar_focused {
            self.sidebar_focused = false;
            self.install_preview_keymap();
            cx.notify();
        }
    }

    pub(super) fn rendered_sidebar_width(&self, viewport_width: f32) -> f32 {
        let desired = self
            .sidebar_resize
            .map(|resize| resize.current_width)
            .unwrap_or(f32::from(self.sidebar_width));
        width_for_viewport(viewport_width, desired)
    }

    pub(super) fn begin_sidebar_resize(
        &mut self,
        pointer_x: f32,
        viewport_width: f32,
        cx: &mut Context<Self>,
    ) {
        let width = self.rendered_sidebar_width(viewport_width);
        self.sidebar_resize = Some(ResizeSession {
            start_pointer_x: pointer_x,
            start_width: width,
            current_width: width,
        });
        cx.notify();
    }

    pub(super) fn update_sidebar_resize(
        &mut self,
        pointer_x: f32,
        viewport_width: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(resize) = self.sidebar_resize else {
            return;
        };
        let width = width_from_resize(viewport_width, resize, pointer_x);
        if self
            .sidebar_resize
            .as_ref()
            .is_some_and(|current| current.current_width != width)
        {
            self.sidebar_resize = Some(ResizeSession {
                current_width: width,
                ..resize
            });
            self.presentation_revision = self.presentation_revision.wrapping_add(1);
            cx.notify();
        }
    }

    pub(super) fn finish_sidebar_resize(&mut self, cx: &mut Context<Self>) {
        let Some(resize) = self.sidebar_resize.take() else {
            return;
        };
        let width = resize
            .current_width
            .round()
            .clamp(MIN_WIDTH_PX, MAX_WIDTH_PX) as u16;
        if self.sidebar_width != width {
            self.sidebar_width = width;
            self.save_preview_settings();
        }
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        cx.notify();
    }

    pub(super) fn reset_sidebar_width(&mut self, cx: &mut Context<Self>) {
        self.sidebar_resize = None;
        let width = DEFAULT_WIDTH_PX as u16;
        if self.sidebar_width != width {
            self.sidebar_width = width;
            self.save_preview_settings();
        }
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        cx.notify();
    }

    pub(super) fn cancel_sidebar_resize(&mut self) -> bool {
        self.sidebar_resize.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_width_honors_product_and_window_constraints() {
        assert_eq!(width_for_viewport(1_200.0, 240.0), 240.0);
        assert_eq!(width_for_viewport(1_200.0, 80.0), 180.0);
        assert_eq!(width_for_viewport(2_000.0, 900.0), 420.0);
        assert_eq!(width_for_viewport(400.0, 240.0), 160.0);
    }

    #[test]
    fn resize_uses_the_pointer_delta_without_losing_the_drag_origin() {
        let session = ResizeSession {
            start_pointer_x: 240.0,
            start_width: 240.0,
            current_width: 240.0,
        };
        assert_eq!(width_from_resize(1_200.0, session, 300.0), 300.0);
        assert_eq!(width_from_resize(1_200.0, session, 100.0), 180.0);
    }
}
