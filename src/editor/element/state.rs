//! Frame state produced by prepaint and consumed by paint.
use std::sync::Arc;

use gpui::{Bounds, Hitbox, PaintQuad, Pixels, RenderImage, ShapedLine};

use crate::document::ByteOffset;
use crate::editor::{HitRow, ShapeKey, syntax};

/// A resolved inline image preview: the render image plus the geometry the
/// paint pass and the resize handle need.
#[derive(Clone)]
pub(super) struct InlineImage {
    pub(super) image: Arc<RenderImage>,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) line_start: ByteOffset,
}

/// The bottom-right drag grip of one painted inline image.
pub(super) struct InlineImageResizeHandlePaint {
    /// Painted grip quad.
    pub(super) bounds: Bounds<Pixels>,
    /// Grip plus hit slop; the editor hit-tests against this.
    pub(super) interaction_bounds: Bounds<Pixels>,
    pub(super) hitbox: Hitbox,
    /// Painted image rect the drag starts from, and the hover box that reveals
    /// this grip.
    pub(super) image_bounds: Bounds<Pixels>,
    pub(super) image_hitbox: Hitbox,
    pub(super) line: u64,
    pub(super) line_start: ByteOffset,
}

pub(super) struct PaintRow {
    pub(super) hit: HitRow,
    pub(super) gutter_layout: ShapedLine,
    pub(super) source_line_number_layout: Option<ShapedLine>,
    pub(super) shape_key: ShapeKey,
    pub(super) visual_rows: usize,
    pub(super) metrics: syntax::BlockMetrics,
    pub(super) block: Option<syntax::EditorBlockDecoration>,
    pub(super) active: bool,
    pub(super) folded: bool,
    pub(super) background: Option<PaintQuad>,
    pub(super) animation_clip_y: Option<(Pixels, Pixels)>,
    pub(super) inline_image: Option<InlineImagePaint>,
}

pub(super) struct InlineImagePaint {
    pub(super) image: Arc<RenderImage>,
    pub(super) bounds: Bounds<Pixels>,
}

pub(super) struct SourceRunButtonPaint {
    pub(super) source_offset: ByteOffset,
    pub(super) bounds: Bounds<Pixels>,
    pub(super) interaction_bounds: Bounds<Pixels>,
    pub(super) hitbox: Hitbox,
    pub(super) accent: u32,
    pub(super) icon: ShapedLine,
}
