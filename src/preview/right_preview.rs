use gpui::Context;

use super::WorkspaceWindow;

pub(in crate::preview) const MIN_WIDTH_PX: f32 = 280.0;
pub(in crate::preview) const MAX_WIDTH_PX: f32 = 960.0;
pub(in crate::preview) const MAX_VIEWPORT_FRACTION: f32 = 0.65;
pub(in crate::preview) const RESIZE_HANDLE_PX: f32 = 6.0;
pub(in crate::preview) const MIN_EDITOR_WIDTH_PX: f32 = 320.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResizeSession {
    start_pointer_x: f32,
    start_width: f32,
    current_width: f32,
    viewport_width: f32,
}

pub(in crate::preview) fn width_for_viewport(viewport_width: f32, desired: f32) -> f32 {
    let available = (viewport_width - RESIZE_HANDLE_PX - MIN_EDITOR_WIDTH_PX).max(0.0);
    let maximum = (viewport_width * MAX_VIEWPORT_FRACTION)
        .min(available)
        .min(MAX_WIDTH_PX);
    desired.clamp(MIN_WIDTH_PX.min(maximum), maximum)
}

fn width_from_resize(session: ResizeSession, pointer_x: f32) -> f32 {
    width_for_viewport(
        session.viewport_width,
        session.start_width + session.start_pointer_x - pointer_x,
    )
}

impl WorkspaceWindow {
    pub(in crate::preview) fn rendered_right_preview_width(&self, viewport_width: f32) -> f32 {
        let desired = self
            .right_preview_resize
            .map(|resize| resize.current_width)
            .unwrap_or(f32::from(
                self.document_view_preferences.right_preview_width,
            ));
        width_for_viewport(viewport_width, desired)
    }

    pub(in crate::preview) fn begin_right_preview_resize(
        &mut self,
        pointer_x: f32,
        viewport_width: f32,
        cx: &mut Context<Self>,
    ) {
        let width = self.rendered_right_preview_width(viewport_width);
        self.right_preview_resize = Some(ResizeSession {
            start_pointer_x: pointer_x,
            start_width: width,
            current_width: width,
            viewport_width,
        });
        cx.notify();
    }

    pub(in crate::preview) fn update_right_preview_resize(
        &mut self,
        pointer_x: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(resize) = self.right_preview_resize else {
            return;
        };
        let width = width_from_resize(resize, pointer_x);
        if width != resize.current_width {
            self.right_preview_resize = Some(ResizeSession {
                current_width: width,
                ..resize
            });
            self.bump_preview_revision(cx);
            cx.notify();
        }
    }

    pub(in crate::preview) fn finish_right_preview_resize(&mut self, cx: &mut Context<Self>) {
        let Some(resize) = self.right_preview_resize.take() else {
            return;
        };
        let width = resize
            .current_width
            .round()
            .clamp(MIN_WIDTH_PX, MAX_WIDTH_PX) as u16;
        if self.document_view_preferences.right_preview_width != width {
            self.document_view_preferences.right_preview_width = width;
            self.save_preview_settings();
        }
        self.bump_preview_revision(cx);
        cx.notify();
    }

    pub(in crate::preview) fn cancel_right_preview_resize(&mut self) -> bool {
        self.right_preview_resize.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_preserves_editor_space_and_product_limits() {
        assert_eq!(width_for_viewport(1_200.0, 420.0), 420.0);
        assert_eq!(width_for_viewport(1_200.0, 100.0), 280.0);
        assert_eq!(width_for_viewport(2_000.0, 1_200.0), 960.0);
        assert_eq!(width_for_viewport(600.0, 420.0), 274.0);
    }

    #[test]
    fn dragging_left_grows_the_right_preview() {
        let session = ResizeSession {
            start_pointer_x: 800.0,
            start_width: 400.0,
            current_width: 400.0,
            viewport_width: 1_200.0,
        };
        assert_eq!(width_from_resize(session, 700.0), 500.0);
        assert_eq!(width_from_resize(session, 1_000.0), 280.0);
    }
}
