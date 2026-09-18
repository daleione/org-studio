//! Frame state produced by prepaint and consumed by paint.
use std::path::Path;
use std::sync::Arc;

use gpui::{Bounds, Hitbox, PaintQuad, Pixels, RenderImage, ShapedLine};

use crate::document::ByteOffset;
use crate::editor::{HitRow, ShapeKey, syntax};

/// A resolved inline image preview: the render image, its painted size and the
/// source path it was loaded from.
pub(super) type InlineImage = (Arc<RenderImage>, f32, f32, Arc<Path>);

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
