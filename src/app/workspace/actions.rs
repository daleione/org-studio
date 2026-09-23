use crate::links::{resolve_file_link, split_link_target};
use gpui::{ClipboardItem, Context, Window};

use crate::app::WorkspaceWindow;
use crate::document::{
    ByteOffset, ByteRange, DocumentCommand, DocumentSession, EditOrigin, EditTransaction,
    Selection, TextSnapshot,
};
use crate::preview::{CopyFeedbackState, PreviewAction, PreviewActionTarget, ReadingPreviewPanel};

impl WorkspaceWindow {
    pub(crate) fn dispatch_preview_action(
        &mut self,
        action: PreviewAction,
        panel: gpui::Entity<ReadingPreviewPanel>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !target_belongs_to_panel(panel.read(cx).document(), action.target()) {
            self.set_document_notice(Some("Preview action is no longer valid".into()));
            cx.notify();
            return;
        }
        match action {
            PreviewAction::ToggleCheckbox { target, expected } => {
                let Some(action_id) =
                    panel.update(cx, |panel, _| panel.begin_action(target.identity()))
                else {
                    return;
                };
                let result = self.toggle_reading_checkbox(&target, &expected, cx);
                panel.update(cx, |panel, _| match result {
                    Ok(revision) => panel.commit_action(action_id, revision),
                    Err(_) => panel.fail_action(action_id),
                });
                if let Err(message) = result {
                    self.set_document_notice(Some(message.into()));
                }
                cx.notify();
            }
            PreviewAction::OpenLink {
                target,
                destination,
            } => {
                if let Err(message) =
                    self.open_reading_link(&target, &destination, panel, _window, cx)
                {
                    self.set_document_notice(Some(message.into()));
                    cx.notify();
                }
            }
            PreviewAction::CopyCode(target) => {
                let result = self.copy_reading_code(&target, cx);
                panel.update(cx, |panel, cx| {
                    panel.show_copy_feedback(
                        target.source_range,
                        if result.is_ok() {
                            CopyFeedbackState::Succeeded
                        } else {
                            CopyFeedbackState::Failed
                        },
                        cx,
                    );
                });
                if let Err(message) = result {
                    self.set_document_notice(Some(message.into()));
                }
            }
            PreviewAction::OpenImage { target, path } => {
                if let Err(message) = self.open_reading_image(&target, &path, cx) {
                    self.set_document_notice(Some(message.into()));
                    cx.notify();
                }
            }
        }
    }

    fn toggle_reading_checkbox(
        &mut self,
        target: &PreviewActionTarget,
        expected: &str,
        cx: &mut Context<Self>,
    ) -> Result<crate::document::Revision, &'static str> {
        let Some(session) = self.document_session().cloned() else {
            return Err("Document is no longer open");
        };
        session.update(cx, |session, cx| {
            let (range, source) = validate_target(session, target)?;
            validate_checkbox_source(&source, expected)?;
            let revision = session.revision();
            let snapshot = session.snapshot();
            let Some(edits) = crate::org_syntax::command::checkbox_transaction(&snapshot, range)
            else {
                return Err("Checkbox is no longer present");
            };
            let before = Selection::caret(range.start);
            let after = Selection::caret(ByteOffset(range.end.0));
            session
                .edit(
                    DocumentCommand::new(
                        EditTransaction::new(revision, edits),
                        before,
                        after,
                        EditOrigin::Other,
                    ),
                    cx,
                )
                .map(|delta| delta.after)
                .map_err(|_| "Could not update the checkbox")
        })
    }

    fn open_reading_link(
        &mut self,
        target: &PreviewActionTarget,
        destination: &str,
        panel: gpui::Entity<ReadingPreviewPanel>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let Some(session) = self.document_session().cloned() else {
            return Err("Document is no longer open");
        };
        let document_path = {
            let session = session.read(cx);
            validate_target(session, target)?;
            session.path().to_path_buf()
        };
        if destination.starts_with("http://")
            || destination.starts_with("https://")
            || destination.starts_with("mailto:")
        {
            cx.open_url(destination);
            return Ok(());
        }

        let local_target = destination.strip_prefix("file:").unwrap_or(destination);
        let (path_or_anchor, search) = split_link_target(local_target);
        if path_or_anchor.is_empty()
            || path_or_anchor.starts_with('#')
            || path_or_anchor.starts_with('*')
        {
            let anchor = search.unwrap_or(path_or_anchor);
            return panel
                .update(cx, |panel, _| panel.jump_to_destination(anchor))
                .then_some(())
                .ok_or("Link target was not found");
        }
        if search.is_none()
            && panel.update(cx, |panel, _| panel.jump_to_destination(path_or_anchor))
        {
            return Ok(());
        }

        let (path, anchor) =
            resolve_file_link(&document_path, destination).ok_or("Invalid file link")?;
        if !path.exists() {
            return Err("Linked file does not exist");
        }
        if crate::preview::is_supported_image(&path) {
            self.open_image_viewer(path, cx)?;
        } else if crate::preview::is_supported_document(&path) {
            self.open_document_link(path, anchor, cx);
        } else {
            cx.open_with_system(&path);
        }
        Ok(())
    }

    pub(crate) fn open_document_link(
        &mut self,
        path: std::path::PathBuf,
        anchor: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.buffer_busy() {
            return;
        }
        let surface = if crate::document::DocumentFormat::detect(&path).is_some() {
            crate::app::PaneSurface::Reading
        } else {
            crate::app::PaneSurface::Editor
        };
        self.open(path, cx);
        if matches!(self.state, crate::app::WorkspaceLoadState::Loading { .. }) {
            self.pending_link_surface = Some((self.generation, surface));
        } else {
            self.set_active_surface(surface, cx);
        }
        // A newly loading document starts at its own destination, not the source pane's anchor.
        if matches!(self.state, crate::app::WorkspaceLoadState::Loading { .. }) {
            *self
                .pending_surface_anchors
                .get_mut(self.document_workspace.active_pane) = None;
        }
        if let Some(anchor) = anchor.filter(|_| surface == crate::app::PaneSurface::Reading) {
            self.pending_navigation = Some((self.generation, anchor.into()));
            if matches!(self.state, crate::app::WorkspaceLoadState::Ready { .. }) {
                self.apply_pending_navigation(self.generation, cx);
            }
        }
    }

    fn copy_reading_code(
        &self,
        target: &PreviewActionTarget,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let Some(session) = self.document_session() else {
            return Err("Document is no longer open");
        };
        let (_, source) = validate_target(session.read(cx), target)?;
        cx.write_to_clipboard(ClipboardItem::new_string(source));
        Ok(())
    }

    fn open_reading_image(
        &mut self,
        target: &PreviewActionTarget,
        image_path: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let Some(session) = self.document_session() else {
            return Err("Document is no longer open");
        };
        let document_path = {
            let session = session.read(cx);
            validate_target(session, target)?;
            session.path().to_path_buf()
        };
        if image_path.starts_with("http://") || image_path.starts_with("https://") {
            cx.open_url(image_path);
            return Ok(());
        }
        let (path, _) =
            resolve_file_link(&document_path, image_path).ok_or("Invalid image link")?;
        if !path.exists() {
            return Err("Image file does not exist");
        }
        self.open_image_viewer(path, cx)
    }

    pub(crate) fn open_image_viewer(
        &mut self,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        if !crate::preview::is_supported_image(&path) {
            return Err("Image format is not supported");
        }
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        let size =
            crate::preview::image_dimensions(&path).map_err(|_| "Image could not be loaded")?;
        gpui::ImageSource::from(path.clone()).remove_asset(cx);
        self.close_command_line(cx);
        self.close_search(false, cx);
        self.dismiss_buffer_panel(cx);
        if self.content_route != crate::app::ContentRoute::Image {
            self.image_viewer.return_route = self.content_route;
        }
        crate::recent_documents::record_success(&mut self.recent_documents, path.clone());
        self.image_viewer.clear(cx);
        self.image_viewer.path = Some(path);
        self.image_viewer.size = Some(size);
        self.image_viewer.zoom = 1.0;
        self.image_viewer.raster_zoom = 1.0;
        self.image_viewer
            .scroll
            .set_offset(gpui::point(gpui::px(0.), gpui::px(0.)));
        self.content_route = crate::app::ContentRoute::Image;
        cx.notify();
        Ok(())
    }

    pub(crate) fn close_image_viewer(&mut self, cx: &mut Context<Self>) {
        self.image_viewer.clear(cx);
        self.content_route = self.image_viewer.return_route;
        cx.notify();
    }

    pub(crate) fn zoom_image(
        &mut self,
        factor: f32,
        focal_point: Option<gpui::Point<gpui::Pixels>>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.zoom_image_with_refinement(factor, focal_point, true, window, cx);
    }

    fn zoom_image_with_refinement(
        &mut self,
        factor: f32,
        focal_point: Option<gpui::Point<gpui::Pixels>>,
        refine: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.content_route != crate::app::ContentRoute::Image || !factor.is_finite() {
            return;
        }
        let Some(image_size) = self.image_viewer.size else {
            return;
        };
        let old_zoom = self.image_viewer.zoom;
        self.image_viewer.zoom = (old_zoom * factor).clamp(0.1, 16.0);
        if self.image_viewer.zoom == old_zoom {
            return;
        }
        let viewport = window.viewport_size();
        let viewport_width = f32::from(viewport.width);
        let viewport_height = (f32::from(viewport.height) - crate::app::TITLEBAR_HEIGHT).max(1.0);
        let focal_x = focal_point
            .map(|point| f32::from(point.x))
            .unwrap_or(viewport_width / 2.0)
            .clamp(0.0, viewport_width);
        let focal_y = focal_point
            .map(|point| f32::from(point.y) - crate::app::TITLEBAR_HEIGHT)
            .unwrap_or(viewport_height / 2.0)
            .clamp(0.0, viewport_height);
        let old_offset = self.image_viewer.scroll.offset();
        let ratio = self.image_viewer.zoom / old_zoom;
        let old_origin = crate::app::image_viewer::image_origin(
            image_size,
            viewport_width,
            viewport_height,
            old_zoom,
        );
        let new_origin = crate::app::image_viewer::image_origin(
            image_size,
            viewport_width,
            viewport_height,
            self.image_viewer.zoom,
        );
        self.image_viewer.scroll.set_offset(gpui::point(
            gpui::px(crate::app::image_viewer::zoomed_scroll_offset(
                f32::from(old_offset.x),
                focal_x,
                ratio,
                old_origin.0,
                new_origin.0,
            )),
            gpui::px(crate::app::image_viewer::zoomed_scroll_offset(
                f32::from(old_offset.y),
                focal_y,
                ratio,
                old_origin.1,
                new_origin.1,
            )),
        ));
        if refine {
            self.image_viewer.raster_zoom = self.image_viewer.zoom;
        }
        cx.notify();
    }

    pub(crate) fn zoom_image_gesture(
        &mut self,
        factor: f32,
        focal_point: gpui::Point<gpui::Pixels>,
        phase: gpui::TouchPhase,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        // Reuse the current texture while the fingers are moving. Only request
        // a sharper raster after the gesture settles.
        self.zoom_image_with_refinement(factor, Some(focal_point), false, window, cx);
        if matches!(phase, gpui::TouchPhase::Ended | gpui::TouchPhase::Cancelled) {
            self.image_viewer.raster_zoom = self.image_viewer.zoom;
            cx.notify();
        }
    }

    pub(crate) fn reset_image_zoom(&mut self, cx: &mut Context<Self>) {
        self.image_viewer.zoom = 1.0;
        self.image_viewer.raster_zoom = 1.0;
        self.image_viewer
            .scroll
            .set_offset(gpui::point(gpui::px(0.), gpui::px(0.)));
        cx.notify();
    }
}

fn target_belongs_to_panel(
    document: &crate::preview::PreviewSnapshot,
    target: &PreviewActionTarget,
) -> bool {
    if document.document_id != target.document_id || document.revision != target.base_revision {
        return false;
    }
    let Some(syntax_id) = target.syntax_id else {
        return true;
    };
    document
        .blocks
        .block_for_syntax_id(syntax_id)
        .and_then(|block_id| document.blocks.nodes().get(block_id as usize))
        .is_some_and(|block| {
            block.source.start <= target.source_range.start
                && block.source.end >= target.source_range.end
        })
}

fn validate_target(
    session: &DocumentSession,
    target: &PreviewActionTarget,
) -> Result<(ByteRange, String), &'static str> {
    if session.id() != target.document_id {
        return Err("The action belongs to another document");
    }
    let mapped = session
        .map_range_to_current(target.revision_range())
        .map_err(|_| "The source changed; action cancelled")?;
    let snapshot = session.snapshot();
    if mapped.range.end.0 > snapshot.len_bytes() {
        return Err("The action target is no longer valid");
    }
    let source = snapshot.copy_range(mapped.range);
    Ok((mapped.range, source))
}

fn validate_checkbox_source(source: &str, expected: &str) -> Result<(), &'static str> {
    if source != expected {
        return Err("Checkbox changed before the action completed");
    }
    if !crate::org_syntax::command::is_checkbox_state_token(source) {
        return Err("Checkbox target is ambiguous");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{split_link_target, validate_checkbox_source};

    #[test]
    fn link_target_preserves_cross_file_anchors() {
        assert_eq!(
            split_link_target("notes.org::*Heading"),
            ("notes.org", Some("*Heading"))
        );
        assert_eq!(
            split_link_target("notes.md#section"),
            ("notes.md", Some("section"))
        );
        assert_eq!(split_link_target("#section"), ("#section", None));
    }

    #[test]
    fn exact_checkbox_tokens_are_valid_action_targets() {
        for token in ["[ ]", "[-]", "[X]", "[x]"] {
            assert_eq!(validate_checkbox_source(token, token), Ok(()));
        }
        assert_eq!(
            validate_checkbox_source("literal [ ]", "literal [ ]"),
            Err("Checkbox target is ambiguous")
        );
        assert_eq!(
            validate_checkbox_source("[X]", "[ ]"),
            Err("Checkbox changed before the action completed")
        );
    }
}
