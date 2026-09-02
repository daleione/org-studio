use gpui::Context;

use super::WorkspaceWindow;

pub(crate) const MIN_PANE_WIDTH_PX: f32 = 280.0;
pub(crate) const RESIZE_HANDLE_PX: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResizeSession {
    start_pointer_x: f32,
    start_left_width: f32,
    current_left_width: f32,
    viewport_width: f32,
}

pub(crate) fn left_width_for_viewport(viewport_width: f32, ratio: u16) -> f32 {
    let content_width = (viewport_width - RESIZE_HANDLE_PX).max(0.0);
    let desired = content_width * f32::from(ratio) / 10_000.0;
    desired.clamp(
        MIN_PANE_WIDTH_PX.min(content_width / 2.0),
        (content_width - MIN_PANE_WIDTH_PX).max(content_width / 2.0),
    )
}

fn width_from_resize(session: ResizeSession, pointer_x: f32) -> f32 {
    let content_width = (session.viewport_width - RESIZE_HANDLE_PX).max(0.0);
    (session.start_left_width + pointer_x - session.start_pointer_x).clamp(
        MIN_PANE_WIDTH_PX.min(content_width / 2.0),
        (content_width - MIN_PANE_WIDTH_PX).max(content_width / 2.0),
    )
}

impl WorkspaceWindow {
    pub(crate) fn rendered_left_pane_width(&self, viewport_width: f32) -> f32 {
        self.split_resize.map_or_else(
            || left_width_for_viewport(viewport_width, self.document_view_preferences.split_ratio),
            |resize| resize.current_left_width,
        )
    }

    pub(crate) fn begin_split_resize(
        &mut self,
        pointer_x: f32,
        viewport_width: f32,
        cx: &mut Context<Self>,
    ) {
        let width = self.rendered_left_pane_width(viewport_width);
        self.split_resize = Some(ResizeSession {
            start_pointer_x: pointer_x,
            start_left_width: width,
            current_left_width: width,
            viewport_width,
        });
        cx.notify();
    }

    pub(crate) fn update_split_resize(&mut self, pointer_x: f32, cx: &mut Context<Self>) {
        let Some(resize) = self.split_resize else {
            return;
        };
        let width = width_from_resize(resize, pointer_x);
        if width != resize.current_left_width {
            self.split_resize = Some(ResizeSession {
                current_left_width: width,
                ..resize
            });
            self.bump_preview_revision(cx);
            cx.notify();
        }
    }

    pub(crate) fn finish_split_resize(&mut self, cx: &mut Context<Self>) {
        let Some(resize) = self.split_resize.take() else {
            return;
        };
        let content_width = (resize.viewport_width - RESIZE_HANDLE_PX).max(1.0);
        let ratio = ((resize.current_left_width / content_width) * 10_000.0)
            .round()
            .clamp(1.0, 9_999.0) as u16;
        if self.document_view_preferences.split_ratio != ratio {
            self.document_view_preferences.split_ratio = ratio;
            self.save_preview_settings();
        }
        self.bump_preview_revision(cx);
        cx.notify();
    }

    pub(crate) fn cancel_split_resize(&mut self) -> bool {
        self.split_resize.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_preserves_space_for_both_panes() {
        assert_eq!(left_width_for_viewport(1_006.0, 5_000), 500.0);
        assert_eq!(left_width_for_viewport(1_006.0, 1_000), 280.0);
        assert_eq!(left_width_for_viewport(1_006.0, 9_000), 720.0);
        assert_eq!(left_width_for_viewport(506.0, 5_000), 250.0);
    }

    #[test]
    fn dragging_changes_the_left_pane_width() {
        let session = ResizeSession {
            start_pointer_x: 800.0,
            start_left_width: 400.0,
            current_left_width: 400.0,
            viewport_width: 1_200.0,
        };
        assert_eq!(width_from_resize(session, 700.0), 300.0);
        assert_eq!(width_from_resize(session, 1_000.0), 600.0);
    }

    #[test]
    fn every_drag_sample_stays_relative_to_the_original_width() {
        let mut session = ResizeSession {
            start_pointer_x: 600.0,
            start_left_width: 500.0,
            current_left_width: 500.0,
            viewport_width: 1_400.0,
        };

        session.current_left_width = width_from_resize(session, 620.0);
        assert_eq!(session.current_left_width, 520.0);

        session.current_left_width = width_from_resize(session, 640.0);
        assert_eq!(session.current_left_width, 540.0);

        session.current_left_width = width_from_resize(session, 610.0);
        assert_eq!(session.current_left_width, 510.0);
    }
}
