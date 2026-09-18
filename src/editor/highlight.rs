//! Shared rounded background painted behind a run of source text.
//!
//! Org tag pills, hex-color swatches and the transient hover highlights all
//! paint the same shape: a wrap-aware rounded rectangle with horizontal
//! breathing room. Keeping one implementation means radius and padding stay in
//! sync across every consumer.

use std::ops::Range;

use gpui::{BorderStyle, Bounds, Corners, Edges, Hsla, PaintQuad, Pixels, point, px, quad, rgba};

use super::HitRow;

/// Geometry and fill of one rounded background behind a source range.
///
/// The glyph color is deliberately not part of this type: an Org tag keeps its
/// semantic face while a hex swatch switches to a contrasting ink. Callers own
/// the text color, and share the shape and fill through this component.
#[derive(Clone, Copy, Debug)]
pub(super) struct RangeHighlight {
    /// Horizontal breathing room added on both sides of the text.
    pub(super) pad_x: f32,
    /// Vertical inset from the line box, so adjacent rows never touch.
    pub(super) inset_y: f32,
    /// Corner radius of the range's outermost corners.
    pub(super) radius: f32,
    pub(super) fill: Hsla,
}

impl RangeHighlight {
    /// Side padding shared by tag pills and color swatches.
    pub(super) const PAD_X: f32 = 3.0;
    /// Vertical inset shared by tag pills and color swatches.
    pub(super) const INSET_Y: f32 = 2.0;
    /// Corner radius shared by tag pills and color swatches.
    pub(super) const RADIUS: f32 = 5.0;

    /// The standard rounded background used by tag pills and color swatches.
    pub(super) fn rounded(fill: Hsla) -> Self {
        Self {
            pad_x: Self::PAD_X,
            inset_y: Self::INSET_Y,
            radius: Self::RADIUS,
            fill,
        }
    }

    /// Paints the background behind `range`, sliced per visual row so a wrapped
    /// range keeps one continuous fill. Only the range's outer corners round.
    pub(super) fn paint(
        self,
        quads: &mut Vec<PaintQuad>,
        hit: &HitRow,
        range: Range<usize>,
        wrap_width: Pixels,
    ) {
        let start_position = hit
            .position_for_display_index(range.start)
            .unwrap_or_default();
        let end_position = hit
            .position_for_display_index(range.end.max(range.start.saturating_add(1)))
            .unwrap_or(start_position);
        let line_height_px = f32::from(hit.line_height).max(1.0);
        let first_row = (f32::from(start_position.y) / line_height_px).round() as usize;
        let last_row = (f32::from(end_position.y) / line_height_px).round() as usize;
        let pad = px(self.pad_x);
        let inset = px(self.inset_y);
        let radius = px(self.radius);
        let zero = px(0.0);
        let single_row = first_row == last_row;
        for row in first_row..=last_row {
            let left = if row == first_row {
                start_position.x
            } else {
                Pixels::ZERO
            } - pad;
            let right = if row == last_row {
                end_position.x
            } else {
                wrap_width
            } + pad;
            if right <= left {
                continue;
            }
            let top = hit.origin_y + px(row as f32 * line_height_px) + inset;
            let bottom = hit.origin_y + px((row + 1) as f32 * line_height_px) - inset;
            if bottom <= top {
                continue;
            }
            let leading = if row == first_row { radius } else { zero };
            let trailing = if row == last_row { radius } else { zero };
            quads.push(quad(
                Bounds::from_corners(
                    point(hit.text_origin_x + left, top),
                    point(hit.text_origin_x + right, bottom),
                ),
                Corners {
                    top_left: leading,
                    top_right: if single_row { leading } else { zero },
                    bottom_right: trailing,
                    bottom_left: if single_row { trailing } else { zero },
                },
                self.fill,
                Edges::default(),
                rgba(0),
                BorderStyle::default(),
            ));
        }
    }
}
