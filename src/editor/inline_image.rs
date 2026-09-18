//! Inline images in the editor: how big they are drawn, the bottom-right grip
//! drag, and the `#+ATTR_ORG:` width edit it commits.
//!
//! This module owns the display rules (`image_sizing`, `resolved_image_size`)
//! and the row-height write-back the paint pass and the minimap consume.

use crate::org_syntax::attributes::{
    ImageAttributeSpec, attr_org_line_above, attr_org_line_with_width, attr_org_line_without_width,
    image_attributes_at,
};

use super::*;

/// Our own inline-image attribute edit, pending its document event.
#[derive(Clone, Debug)]
pub(super) struct PendingImageEdit {
    /// The image line before the edit, in pre-splice numbering.
    previous_line: u64,
    /// Lines the edit adds above the image; negative when it removes one.
    line_delta: i64,
    /// Image metrics captured before the edit.
    metrics: Option<InlineImageMetrics>,
    /// Viewport correction for the inserted or removed line height.
    scroll_correction: f32,
}

/// Vertical padding above and below an inline image row.
pub(super) const INLINE_IMAGE_VERTICAL_PADDING: f32 = 6.0;
/// Legacy cap for the auto fit; an explicit `:width` or a drag may exceed it.
pub(super) const INLINE_IMAGE_MAX_WIDTH: f32 = 640.0;

/// Sizing limits for the editor column: the auto fit keeps its legacy cap,
/// while an explicit `:width` or a drag may use the full column width.
pub(super) fn image_sizing(
    column_width: f32,
    viewport_height: f32,
    font_size: f32,
) -> crate::preview::ImageSizing {
    crate::preview::ImageSizing {
        auto_width: column_width.min(INLINE_IMAGE_MAX_WIDTH),
        max_width: column_width,
        max_height: viewport_height * 2.0,
        font_size,
    }
}

/// Painted size for one cached image, honouring authored attributes and any
/// in-flight drag on that line.
pub(super) fn resolved_image_size(
    editor: &SemanticEditor,
    metrics: &InlineImageMetrics,
    sizing: &crate::preview::ImageSizing,
) -> (f32, f32) {
    let spec = editor.effective_inline_image_spec(ByteOffset(metrics.line_start), &metrics.spec);
    crate::preview::resolve_image_size(metrics.source, Some(&spec), sizing)
}

/// The closest `#+ATTR_ORG:` line above an image line.
struct ImageAttributeLine {
    /// Line content, without the newline.
    content: ByteRange,
    /// Whole line, including the newline.
    full: ByteRange,
    text: String,
}

impl SemanticEditor {
    /// Authored `#+ATTR_ORG:` attributes attached to the image line: the closest
    /// keyword above it wins.
    pub(super) fn inline_image_attribute_spec(
        &self,
        snapshot: &DocumentSnapshot,
        line_start: ByteOffset,
    ) -> ImageAttributeSpec {
        match snapshot.line_index_at(line_start) {
            Ok(line) => image_attributes_at(snapshot, line.0),
            Err(_) => ImageAttributeSpec::default(),
        }
    }

    /// Width an in-flight drag currently previews for `line_start`.
    pub(super) fn inline_image_resize_width(&self, line_start: ByteOffset) -> Option<f32> {
        self.image_resize
            .as_ref()
            .filter(|session| session.line_start == line_start)
            .map(|session| session.current_width)
    }

    /// Attributes that position the image this frame: authored values with an
    /// in-flight drag layered on top.
    pub(super) fn effective_inline_image_spec(
        &self,
        line_start: ByteOffset,
        authored: &ImageAttributeSpec,
    ) -> ImageAttributeSpec {
        match self.inline_image_resize_width(line_start) {
            Some(width) => ImageAttributeSpec::from_width_px(width),
            None => authored.clone(),
        }
    }

    /// Re-measures one inline image row for its current (possibly dragged) width.
    ///
    /// The drag handlers must use this instead of invalidating the line:
    /// `invalidate_line_layout` drops the row to an estimated height, so the
    /// document height would oscillate between estimated and measured on every
    /// pointer move and the viewport would thrash.
    pub(super) fn apply_inline_image_row_height(
        &mut self,
        line: u64,
        column_width: f32,
        viewport_height: f32,
    ) -> bool {
        let Some(metrics) = self
            .inline_image_line_dimensions
            .borrow()
            .get(&line)
            .cloned()
        else {
            return false;
        };
        let sizing = image_sizing(column_width, viewport_height, self.font_size_px());
        let (_, height) = resolved_image_size(self, &metrics, &sizing);
        self.display_map.update_line_layout(
            line,
            1,
            height,
            INLINE_IMAGE_VERTICAL_PADDING,
            INLINE_IMAGE_VERTICAL_PADDING,
        )
    }

    /// Consumes a press on an image grip; `false` lets the event fall through.
    pub(super) fn image_resize_down(
        &mut self,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(handle) = self
            .inline_image_handles
            .iter()
            .find(|handle| handle.bounds.contains(&event.position))
            .copied()
        else {
            return false;
        };
        if event.click_count >= 2 {
            // Double-clicking the grip returns the image to its auto size.
            self.reset_inline_image_size(handle.line_start, cx);
        } else {
            self.image_resize = Some(ImageResizeSession {
                line: handle.line,
                line_start: handle.line_start,
                start_pointer_x: f32::from(event.position.x),
                start_width: handle.width,
                current_width: handle.width,
                moved: false,
            });
        }
        cx.stop_propagation();
        true
    }

    /// Applies an in-flight drag; `false` when no drag is active.
    pub(super) fn image_resize_move(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.image_resize.is_none() {
            return false;
        }
        let column_width = self.display_map.wrap_width();
        let viewport_height = self.image_drag_viewport_height();
        let line = {
            let Some(session) = self.image_resize.as_mut() else {
                return false;
            };
            let dragged = session.start_width + (f32::from(position.x) - session.start_pointer_x);
            let width = crate::preview::clamp_image_width(dragged, column_width);
            if (width - session.current_width).abs() < 0.25 {
                return true;
            }
            session.current_width = width;
            session.moved = true;
            session.line
        };
        // Keep the row measured at its new height. Invalidating it instead would
        // drop the line to an estimated height, so the document height would
        // oscillate between estimated and measured on every pointer move and the
        // viewport would thrash. The shape cache stays untouched: text shaping
        // does not depend on an image's size.
        self.apply_inline_image_row_height(line, column_width, viewport_height);
        cx.notify();
        true
    }

    /// Height of the editor viewport, used for the drag's pathological cap.
    fn image_drag_viewport_height(&self) -> f32 {
        self.viewport
            .map_or(0.0, |bounds| f32::from(bounds.size.height))
    }

    /// Ends an in-flight drag, committing it when the grip actually moved.
    pub(super) fn image_resize_up(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.image_resize.take() else {
            return;
        };
        let column_width = self.display_map.wrap_width();
        let viewport_height = self.image_drag_viewport_height();
        if !session.moved {
            // A press without movement is not an edit: restore the authored size.
            self.apply_inline_image_row_height(session.line, column_width, viewport_height);
            cx.notify();
            return;
        }
        self.write_inline_image_width(session.line_start, session.current_width, cx);
    }

    /// Writes the dragged width to the image's `#+ATTR_ORG:` line.
    fn write_inline_image_width(
        &mut self,
        line_start: ByteOffset,
        width: f32,
        cx: &mut Context<Self>,
    ) {
        let snapshot = self.snapshot(cx);
        let width = crate::preview::clamp_image_width(width, self.display_map.wrap_width());
        match self.inline_image_attribute_line(&snapshot, line_start) {
            Some(attribute) => {
                let Some(updated) = attr_org_line_with_width(&attribute.text, width) else {
                    return;
                };
                if updated == attribute.text.trim_end() {
                    return;
                }
                let revision = snapshot.revision();
                self.apply_image_edit(revision, attribute.content, updated, line_start, cx);
            }
            None => {
                let revision = snapshot.revision();
                let inserted = format!("#+ATTR_ORG: :width {}\n", width.round() as i64);
                self.apply_image_edit(
                    revision,
                    ByteRange::new(line_start.0, line_start.0),
                    inserted,
                    line_start,
                    cx,
                );
            }
        }
    }

    /// Drops the image's `:width` attribute so it returns to its auto size.
    fn reset_inline_image_size(&mut self, line_start: ByteOffset, cx: &mut Context<Self>) {
        let snapshot = self.snapshot(cx);
        let Some(attribute) = self.inline_image_attribute_line(&snapshot, line_start) else {
            return;
        };
        let revision = snapshot.revision();
        match attr_org_line_without_width(&attribute.text) {
            Some(updated) => {
                self.apply_image_edit(revision, attribute.content, updated, line_start, cx)
            }
            // The width was the only attribute: drop the whole line.
            None => self.apply_image_edit(revision, attribute.full, String::new(), line_start, cx),
        }
    }

    /// The closest `#+ATTR_ORG:` line above the image, if any.
    fn inline_image_attribute_line(
        &self,
        snapshot: &DocumentSnapshot,
        line_start: ByteOffset,
    ) -> Option<ImageAttributeLine> {
        let line = snapshot.line_index_at(line_start).ok()?;
        let index = LineIndex(attr_org_line_above(snapshot, line.0)?);
        let content = snapshot.line_content_range(index).ok()?;
        let text = snapshot.copy_range(content);
        let full = snapshot.line_range(index).ok()?;
        Some(ImageAttributeLine {
            content,
            full,
            text,
        })
    }

    /// Applies one programmatic edit as a single undo step, keeping the
    /// selection anchored to the same text and the image pinned on screen.
    fn apply_image_edit(
        &mut self,
        revision: Revision,
        range: ByteRange,
        replacement: String,
        image_line_start: ByteOffset,
        cx: &mut Context<Self>,
    ) {
        let before = self.selection;
        let inserted = replacement.len() as u64;
        let removed = range.end.0.saturating_sub(range.start.0);
        let shift = |offset: ByteOffset| {
            let value = offset.0;
            if value <= range.start.0 {
                ByteOffset(value)
            } else if value >= range.end.0 {
                ByteOffset(value.saturating_add(inserted).saturating_sub(removed))
            } else {
                ByteOffset(range.start.0 + (value - range.start.0).min(inserted))
            }
        };
        let after = Selection::new(shift(before.anchor()), shift(before.head()));
        // What the document event needs to repair the rows afterwards: the
        // image's own metrics and how many lines this edit adds or removes above
        // it, both captured before the edit.
        let snapshot = self.snapshot(cx);
        let image_line = snapshot
            .line_index_at(image_line_start)
            .ok()
            .map(|line| line.0);
        let image_metrics = image_line.and_then(|line| {
            self.inline_image_line_dimensions
                .borrow()
                .get(&line)
                .cloned()
        });
        let removed_lines = snapshot.copy_range(range).matches('\n').count() as i64;
        let inserted_lines = replacement.matches('\n').count() as i64;
        let line_delta = inserted_lines - removed_lines;
        // The document event for this edit arrives after the line splice, so the
        // geometry is repaired there in post-splice line numbers; recording it
        // here also stops that handler from reverting the viewport.
        self.pending_image_edit = Some(PendingImageEdit {
            previous_line: image_line.unwrap_or(0),
            line_delta,
            metrics: image_metrics,
            scroll_correction: line_delta as f32 * self.display_map.base_line_height(),
        });
        let result = self.session.update(cx, |session, cx| {
            session.edit(
                DocumentCommand::new(
                    EditTransaction::new(revision, vec![TextEdit::new(range, replacement)]),
                    before,
                    after,
                    EditOrigin::Other,
                ),
                cx,
            )
        });
        if result.is_ok() {
            self.selection = after;
            let snapshot = self.snapshot(cx);
            self.selection_revision = snapshot.revision();
            self.sync_selection_utf16(&snapshot);
            self.hit_rows = Arc::from([]);
            cx.notify();
        }
    }

    /// Repairs the image row for our own attribute edit, after the document event
    /// handler has spliced the layout map (so line numbers match the document),
    /// and applies the viewport correction recorded with it.
    pub(super) fn apply_pending_image_edit(
        &mut self,
        pending: PendingImageEdit,
        snapshot: &DocumentSnapshot,
        viewport_height: f32,
    ) {
        let column_width = self.display_map.wrap_width();
        let new_line = pending
            .previous_line
            .saturating_add_signed(pending.line_delta);
        if let Some(metrics) = pending.metrics
            && let Ok(new_start) = snapshot
                .line_content_range(LineIndex(new_line))
                .map(|range| range.start)
        {
            let spec = self.inline_image_attribute_spec(snapshot, new_start);
            {
                let mut cache = self.inline_image_line_dimensions.borrow_mut();
                cache.remove(&pending.previous_line);
                cache.insert(
                    new_line,
                    InlineImageMetrics {
                        line_start: new_start.0,
                        source: metrics.source,
                        spec,
                    },
                );
            }
            // Lines the edit inserted above the image are plain text.
            for line in pending.previous_line..new_line {
                self.display_map.update_line_layout(
                    line,
                    1,
                    self.display_map.base_line_height(),
                    0.0,
                    0.0,
                );
            }
            self.apply_inline_image_row_height(new_line, column_width, viewport_height);
        }
        // At the document end the layout's own end pin moves the viewport, so an
        // extra correction would fight it.
        if !self.scroll_at_end && pending.scroll_correction != 0.0 {
            let max_scroll = (self.display_map.total_height() - viewport_height).max(0.0);
            self.scroll_y = (self.scroll_y + pending.scroll_correction).clamp(0.0, max_scroll);
        }
    }
}
