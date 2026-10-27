//! Custom GPUI element for the semantic editor.
//!
//! The element turns editor state into one painted frame:
//!
//! - [`prepaint`](gpui::Element::prepaint) shapes every visible row, resolves
//!   inline images and block chrome, and publishes render requests to the
//!   minimap and syntax services.
//! - [`paint`](gpui::Element::paint) paints the gutter, the text layer and the
//!   prepared minimap frame inside the element's content mask.
//!
//! Support code lives in sibling modules grouped by responsibility:
//!
//! - `prepaint` / `paint`: the two frame passes this element delegates to.
//! - `rows` / `text`: visible-row planning and text shaping primitives.
//! - `quads` / `cookie` / `blocks` / `buttons`: row-local highlights, block
//!   chrome and the source block action buttons.
//! - `minimap` / `minimap_layout` / `minimap_raster`: the editor-side minimap
//!   adapters (frame building, background layout and raster scheduling).
//! - `inline_images`: inline image preview resolution.
//! - `scroll`: scroll anchoring helpers shared by prepaint and the commit.
//! - `state`: the frame's data types.

mod blocks;
mod buttons;
mod cookie;
mod inline_images;
mod minimap;
mod minimap_layout;
mod minimap_raster;
mod paint;
mod plots;
mod prepaint;
mod quads;
mod rows;
mod scroll;
mod state;
mod text;

use std::sync::Arc;
#[cfg(feature = "benchmarks")]
use std::time::Duration;
#[cfg(feature = "benchmarks")]
use std::time::Instant;

use gpui::{
    App, BorderStyle, Bounds, ContentMask, Corners, CursorStyle, Edges, Element, ElementId,
    ElementInputHandler, FontWeight, GlobalElementId, HitboxBehavior, LayoutId, PaintQuad, Pixels,
    Style, TextAlign, TextRun, Window, WrappedLine, fill, point, px, quad, relative, rgba, size,
};

use crate::{
    document::{ByteOffset, ByteRange, LineIndex, TextSnapshot},
    theme::current_theme,
};

#[cfg(feature = "benchmarks")]
use super::FrameBenchmarkAction;
use super::{HitRow, SemanticEditor, highlight::RangeHighlight, syntax};

use blocks::{
    editor_block_accent, editor_block_content_width, editor_block_left_inset,
    editor_block_text_inset, editor_block_wrap_width, editor_row_background, is_source_block_kind,
};
use cookie::push_cookie_progress_quads;
use minimap::{MinimapPaint, build_minimap};
use minimap_layout::{prepare_minimap_layout_request, schedule_minimap_layout_preparation};
use minimap_raster::schedule_minimap_raster;
use quads::{
    push_link_hover_quad, push_search_quads, push_selection_quads, push_swatch_quads,
    push_tag_pill_quads,
};
use rows::animated_paint_lines;
pub(super) use scroll::{scroll_is_at_end, stabilized_scroll_y};
use state::{
    InlineImagePaint, InlineImageResizeHandlePaint, PaintRow, PaintedCaret, SourceRunButtonPaint,
};
pub(super) use text::ShapeKey;
use text::{
    folded_display_text, local_marked, lower_fence_backticks, markdown_fence_backticks, shape_key,
};

const GUTTER_PADDING: f32 = 16.0;
const BLOCK_LEFT_INSET: f32 = 8.0;
const BLOCK_RIGHT_INSET: f32 = 16.0;
const BLOCK_TEXT_INSET: f32 = 16.0;
const BLOCK_TEXT_RIGHT_PADDING: f32 = 8.0;
const SOURCE_GUTTER_INSET: f32 = 1.0;
const SOURCE_GUTTER_WIDTH: f32 = 24.0;
const SOURCE_TEXT_PADDING: f32 = 4.0;
const SOURCE_LINE_NUMBER_FONT_SCALE: f32 = 0.70;
const SOURCE_RUN_BUTTON_SIZE: f32 = 20.0;
const SOURCE_RUN_BUTTON_HIT_SLOP: f32 = 4.0;
const SOURCE_RUN_ICON_FONT_SCALE: f32 = 0.82;
const SOURCE_COPY_BUTTON_SIZE: f32 = 26.0;
const SOURCE_COPY_BUTTON_RIGHT_INSET: f32 = 10.0;
const SOURCE_COPY_BUTTON_SPACE: f32 = SOURCE_COPY_BUTTON_SIZE + SOURCE_COPY_BUTTON_RIGHT_INSET;
const BLOCK_VERTICAL_INSET: f32 = 2.0;
const BLOCK_RADIUS: f32 = 7.0;
/// Bottom-right grip of an inline image, plus the slack around its hit area.
const INLINE_IMAGE_RESIZE_HANDLE_SIZE: f32 = 24.0;
const INLINE_IMAGE_RESIZE_HANDLE_SLOP: f32 = 6.0;
/// gpui exposes the diagonal resize cursor as an enum variant only. If a
/// platform turns out not to back it, this constant is the single knob.
const INLINE_IMAGE_RESIZE_CURSOR: CursorStyle = CursorStyle::ResizeUpLeftDownRight;

pub struct EditorElement {
    editor: gpui::Entity<SemanticEditor>,
}

impl EditorElement {
    pub fn new(editor: gpui::Entity<SemanticEditor>) -> Self {
        Self { editor }
    }
}

impl SemanticEditor {
    pub(crate) fn gutter_width(&self, cx: &App) -> f32 {
        let digits = self.snapshot(cx).len_lines().max(1).ilog10() + 1;
        let font_id = cx
            .text_system()
            .resolve_font(&crate::typography::editor_font());
        let digit_width = ('0'..='9')
            .filter_map(|digit| {
                cx.text_system()
                    .advance(font_id, px(self.font_size_px()), digit)
                    .ok()
            })
            .map(|advance| f32::from(advance.width))
            .fold(0.0_f32, f32::max)
            .max(1.0);
        digits as f32 * digit_width + GUTTER_PADDING * 2.0
    }
}

impl gpui::IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct PrepaintState {
    #[cfg(feature = "benchmarks")]
    started_at: Instant,
    rows: Vec<PaintRow>,
    plots: Vec<gpui::AnyElement>,
    plot_output_errors: Vec<gpui::AnyElement>,
    block_backgrounds: Vec<PaintQuad>,
    tag_pills: Vec<PaintQuad>,
    swatches: Vec<PaintQuad>,
    hover_quads: Vec<PaintQuad>,
    link_hits: Vec<super::LinkHit>,
    source_run_buttons: Vec<SourceRunButtonPaint>,
    source_copy_buttons: Vec<super::source_copy::CopyButtonPaint>,
    image_resize_handles: Vec<InlineImageResizeHandlePaint>,
    selection: Vec<PaintQuad>,
    caret: Option<PaintedCaret>,
    gutter: PaintQuad,
    content_left: Pixels,
    minimap: MinimapPaint,
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        prepaint::build_frame(&self.editor, bounds, window, cx)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        paint::paint_frame(&self.editor, bounds, state, window, cx)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod resize_pin_tests;
