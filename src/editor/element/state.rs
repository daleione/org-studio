//! Frame state produced by prepaint and consumed by paint.
use std::sync::Arc;

use gpui::{
    Bounds, Hitbox, PaintQuad, Pixels, Point, RenderImage, ShapedLine, fill, point, px, size,
};

use crate::document::{ByteOffset, LineIndex};
use crate::editor::{HitRow, ShapeKey, SourceRunButtonHit, syntax};
use crate::theme::Theme;

/// Complete caret paint prepared with the row; application code only receives its bounds.
pub(super) struct PaintedCaret {
    pub(super) quad: PaintQuad,
    pub(super) text: Option<CaretText>,
}

pub(super) struct CaretText {
    pub(super) line: Box<ShapedLine>,
    pub(super) origin: Point<Pixels>,
    pub(super) line_height: Pixels,
}

impl PaintedCaret {
    const VERTICAL_INSET: f32 = 1.0;
    const CORNER_RADIUS: f32 = 1.0;

    pub(super) fn new(bounds: Bounds<Pixels>, theme: &Theme, text: Option<CaretText>) -> Self {
        Self {
            quad: fill(
                Bounds::new(
                    bounds.origin + point(px(0.), px(Self::VERTICAL_INSET)),
                    size(
                        bounds.size.width,
                        (bounds.size.height - px(Self::VERTICAL_INSET * 2.)).max(px(1.)),
                    ),
                ),
                gpui::rgb(theme.caret_background),
            )
            .corner_radii(px(Self::CORNER_RADIUS)),
            text,
        }
    }

    pub(super) fn image_edge(image: Bounds<Pixels>, at_start: bool, theme: &Theme) -> Self {
        let width = px(4.);
        let x = if at_start {
            image.left() - width / 2.
        } else {
            image.right() + px(1.)
        };
        Self::new(
            Bounds::new(point(x, image.top()), size(width, image.size.height)),
            theme,
            None,
        )
    }

    pub(super) fn bounds(&self) -> Bounds<Pixels> {
        self.quad.bounds
    }

    pub(super) fn visible_bounds(&self, viewport: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
        let bounds = self.bounds().intersect(&viewport);
        (bounds.size.width > px(0.) && bounds.size.height > px(0.)).then_some(bounds)
    }
}

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
    pub(super) plot: Option<crate::plot::PlotId>,
    pub(super) edges: Vec<(crate::plot::PlotResizeAxis, Bounds<Pixels>, Hitbox)>,
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
    pub(super) preview_runs: Arc<[gpui::TextRun]>,
    pub(super) hit: HitRow,
    pub(super) gutter_layout: ShapedLine,
    pub(super) source_line_number_layout: Option<ShapedLine>,
    pub(super) shape_key: ShapeKey,
    pub(super) style_id: syntax::EditorStyleId,
    pub(super) visual_rows: usize,
    pub(super) metrics: syntax::BlockMetrics,
    pub(super) block: Option<syntax::EditorBlockDecoration>,
    pub(super) active: bool,
    pub(super) selected: bool,
    pub(super) folded: bool,
    pub(super) background: Option<PaintQuad>,
    pub(super) animation_clip_y: Option<(Pixels, Pixels)>,
    pub(super) inline_image: Option<InlineImagePaint>,
}

impl PaintRow {
    pub(super) fn gutter_number_span(&self) -> (Pixels, Pixels) {
        let center = self.hit.origin_y + self.hit.line_height / 2.0;
        let half_height = (self.gutter_layout.ascent + self.gutter_layout.descent) / 2.0;
        (center - half_height, center + half_height)
    }
}

pub(super) struct InlineImagePaint {
    pub(super) image: Arc<RenderImage>,
    pub(super) bounds: Bounds<Pixels>,
}

pub(super) struct SourceRunButtonPaint {
    pub(super) gutter: bool,
    pub(super) hit: SourceRunButtonHit,
    pub(super) line: LineIndex,
    pub(super) bounds: Bounds<Pixels>,
    pub(super) hitbox: Hitbox,
    pub(super) accent: u32,
    pub(super) icon: ShapedLine,
}

impl InlineImageResizeHandlePaint {
    pub(super) fn new(
        image_bounds: Bounds<Pixels>,
        line: u64,
        line_start: ByteOffset,
        plot: Option<crate::plot::PlotId>,
        window: &mut gpui::Window,
    ) -> Self {
        use gpui::{HitboxBehavior, point, px, size};
        let edge = super::INLINE_IMAGE_RESIZE_HANDLE_SIZE;
        let slop = super::INLINE_IMAGE_RESIZE_HANDLE_SLOP;
        let bounds = Bounds::new(
            point(
                image_bounds.right() - px(edge),
                image_bounds.bottom() - px(edge),
            ),
            size(px(edge), px(edge)),
        );
        let interaction_bounds = Bounds::new(
            point(bounds.left() - px(slop), bounds.top() - px(slop)),
            size(px(edge + slop * 2.0), px(edge + slop * 2.0)),
        );
        let edges = if plot.is_some() {
            [
                (
                    crate::plot::PlotResizeAxis::Width,
                    Bounds::new(
                        point(
                            image_bounds.right() - px(5.0),
                            image_bounds.top() + px(12.0),
                        ),
                        size(
                            px(10.0),
                            (image_bounds.size.height - px(edge + 12.0)).max(px(0.0)),
                        ),
                    ),
                ),
                (
                    crate::plot::PlotResizeAxis::Height,
                    Bounds::new(
                        point(
                            image_bounds.left() + px(12.0),
                            image_bounds.bottom() - px(5.0),
                        ),
                        size(
                            (image_bounds.size.width - px(edge + 12.0)).max(px(0.0)),
                            px(10.0),
                        ),
                    ),
                ),
            ]
            .into_iter()
            .map(|(axis, bounds)| {
                (
                    axis,
                    bounds,
                    window.insert_hitbox(bounds, HitboxBehavior::Normal),
                )
            })
            .collect()
        } else {
            Vec::new()
        };
        Self {
            plot,
            edges,
            bounds,
            interaction_bounds,
            line,
            line_start,
            image_bounds,
            hitbox: window.insert_hitbox(interaction_bounds, HitboxBehavior::Normal),
            image_hitbox: window.insert_hitbox(image_bounds, HitboxBehavior::Normal),
        }
    }
}
