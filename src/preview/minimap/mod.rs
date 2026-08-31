#[cfg(test)]
#[allow(unused_imports)]
use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::{Hash, Hasher},
    ops::Range,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use std::{sync::OnceLock, time::Duration};

#[cfg(test)]
#[allow(unused_imports)]
use gpui::FontFallbacks;
#[cfg(test)]
#[allow(unused_imports)]
use gpui::{
    BorderStyle, Bounds, Corners, CursorStyle, DispatchPhase, FontWeight, ListOffset, ListState,
    MouseButton, MouseMoveEvent, MouseUpEvent, RenderImage, SharedString, TextRun, canvas, div,
    fill, font, outline, point, prelude::*, px, rgba,
};
#[cfg(test)]
#[allow(unused_imports)]
use image::{Frame, RgbaImage};
#[cfg(test)]
#[allow(unused_imports)]
use smallvec::SmallVec;

#[cfg(test)]
#[allow(unused_imports)]
use crate::{
    document::SharedTextSnapshot,
    org_syntax::{
        BlockArena, BlockKind,
        inline::{InlineKind, InlineSpan},
    },
    theme::current_theme,
};

#[cfg(test)]
#[allow(unused_imports)]
use super::code_highlight_style;
#[cfg(test)]
#[allow(unused_imports)]
use super::{
    CodeHighlightSpan, DocumentFormat, PreviewRow, PreviewSnapshot, highlight_code,
    markdown::{MarkdownBlock, MarkdownKind},
    parse_document_inline,
};

pub(in crate::preview) mod projection;
mod raster;
mod render;
mod state;
#[cfg(test)]
mod tests;
mod viewport;
mod width;
pub(super) use super::display_map::PreviewDisplayMap;
use super::display_map::{
    DisplayLines, DisplayRuns, PreviewLineKind, kind_color, minimap_runs, slice_display_runs,
};
#[cfg(test)]
use super::layout::{LayoutSnapshot as ProjectionSnapshot, ResolvedRow as ProjectionMeasure};
pub(super) use crate::minimap::DragSession as MinimapDragSession;
#[cfg(test)]
use projection::MinimapLineIndexKey;
pub(super) use projection::{
    CachedMinimapLineIndex, MinimapLineIndex, MinimapLineIndexBuilder, MinimapProjectionReadiness,
};
pub(super) use raster::RasterTileCache;
pub(super) use raster::prewarm_text_rasterizer;
use raster::{
    RasterRow, RasterTileKey, RasterTilePaint, RasterTileRequest, display_window_range,
    folded_signature, rasterize_tile, tile_key,
};
pub(super) use render::render;
pub(super) use state::{
    MinimapState, ResizeSession as MinimapResizeSession, current_resize_session,
    take_resize_session,
};
pub(super) use viewport::MinimapInteractionAnchor;
use viewport::{
    minimap_anchor_for_thumb_top, minimap_click_target_for_viewport, minimap_drag_target,
    minimap_thumb_for_drag, minimap_viewport_for_list_with_anchor, scroll_list_to_ratio,
    scroll_ratio_after_wheel, source_target_for_list_offset, thumb_alphas,
};
pub(super) use width::Density as MinimapDensity;
use width::from_resize_drag as width_from_resize_drag;
#[cfg(test)]
use width::{
    EDGE_PADDING_PX as MINIMAP_EDGE_PADDING_PX, FONT_PX as MINIMAP_FONT_PX,
    LINE_HEIGHT_PX as MINIMAP_LINE_HEIGHT_PX, manual_for_viewport as manual_width_for_viewport,
};
pub(super) use width::{
    MANUAL_MAX_PX as MINIMAP_MANUAL_MAX_PX, WidthChange as MinimapWidthChange,
    for_viewport as width_for_viewport,
};

const MIN_THUMB_PX: f32 = crate::minimap::MIN_THUMB_PX;
const SCROLL_WHEEL_LINE_PX: f32 = 20.0;
const MINIMAP_RESIZE_HANDLE_PX: f32 = 6.0;
// Leave the overwhelming majority of a 120Hz frame (8.333ms) to GPUI layout,
// paint and presentation. Projection refinement is cooperative and can take as
// many frames as necessary because an estimated projection is available first.
pub(super) const MINIMAP_INDEX_FRAME_BUDGET: Duration = Duration::from_micros(350);
#[cfg(test)]
const PREVIEW_BASE_ROW_PX: f32 = 24.0;
const RASTER_TILE_ROWS: usize = 128;
const RASTER_TILE_CACHE_CAPACITY: usize = 6;

pub(super) fn minimap_trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("ORG_STUDIO_MINIMAP_TRACE").is_some())
}

pub(super) fn minimap_perf_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("ORG_STUDIO_MINIMAP_PERF").is_some() || minimap_trace_enabled()
    })
}
