use std::sync::{Mutex, OnceLock};

pub const MIN_THUMB_PX: f32 = 24.0;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Density {
    Compact,
    Comfortable,
    Large,
    ExtraLarge,
    Maximum,
}

impl Density {
    pub(crate) fn for_width(width: f32) -> Self {
        if width < 128.0 {
            Self::Compact
        } else if width < 184.0 {
            Self::Comfortable
        } else if width < 280.0 {
            Self::Large
        } else if width < 400.0 {
            Self::ExtraLarge
        } else {
            Self::Maximum
        }
    }

    pub(crate) fn font_px(self) -> f32 {
        match self {
            Self::Compact => 2.0,
            Self::Comfortable => 3.0,
            Self::Large => 3.8,
            Self::ExtraLarge => 4.1,
            Self::Maximum => 4.25,
        }
    }

    pub(crate) fn line_height(self) -> f32 {
        match self {
            Self::Compact => 2.6,
            Self::Comfortable => 3.8,
            Self::Large => 4.6,
            Self::ExtraLarge => 4.9,
            Self::Maximum => 5.0,
        }
    }

    pub(crate) fn edge_padding(self) -> f32 {
        match self {
            Self::Compact => 4.0,
            Self::Comfortable => 5.0,
            Self::Large => 6.0,
            Self::ExtraLarge => 7.0,
            Self::Maximum => 8.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ProjectionViewport {
    pub(crate) content_top: f32,
    pub(crate) thumb_top: f32,
    pub(crate) thumb_height: f32,
    pub(crate) interaction_height: f32,
    pub(crate) scroll_ratio: f32,
}

pub(crate) fn projection_viewport(
    total_units: f32,
    visible_top: f32,
    visible_bottom: f32,
    scroll_ratio: f32,
    track_height: f32,
    density: Density,
) -> ProjectionViewport {
    projection_viewport_with_line_height(
        total_units,
        visible_top,
        visible_bottom,
        scroll_ratio,
        track_height,
        density,
        density.line_height(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn projection_viewport_with_line_height(
    total_units: f32,
    visible_top: f32,
    visible_bottom: f32,
    scroll_ratio: f32,
    track_height: f32,
    density: Density,
    line_height: f32,
) -> ProjectionViewport {
    if total_units <= 0.0 || track_height <= 0.0 {
        return ProjectionViewport::default();
    }
    let line_height = line_height.max(f32::EPSILON);
    let interaction_height =
        (total_units * line_height + density.edge_padding() * 2.0).min(track_height);
    let visible_minimap_units =
        ((interaction_height - density.edge_padding() * 2.0) / line_height).max(1.0);
    let max_content_top = (total_units - visible_minimap_units).max(0.0);
    let visible_top = visible_top.clamp(0.0, total_units);
    let visible_bottom = visible_bottom.clamp(visible_top, total_units);
    let minimum_content_top = (visible_bottom - visible_minimap_units)
        .max(0.0)
        .min(max_content_top);
    let maximum_content_top = visible_top.max(minimum_content_top).min(max_content_top);
    let scroll_ratio = scroll_ratio.clamp(0.0, 1.0);
    let content_top = if max_content_top > 0.0 {
        (scroll_ratio * max_content_top).clamp(minimum_content_top, maximum_content_top)
    } else {
        0.0
    };
    let thumb_height = ((visible_bottom - visible_top) * line_height
        + density.edge_padding() * 2.0)
        .max(MIN_THUMB_PX)
        .min(interaction_height);
    let thumb_top = ((visible_top - content_top).max(0.0) * line_height)
        .clamp(0.0, (interaction_height - thumb_height).max(0.0));
    ProjectionViewport {
        content_top,
        thumb_top,
        thumb_height,
        interaction_height,
        scroll_ratio,
    }
}

pub(crate) fn stabilize_projection_camera(
    viewport: ProjectionViewport,
    total_units: f32,
    visible_top: f32,
    visible_bottom: f32,
    density: Density,
    content_anchor: f32,
) -> ProjectionViewport {
    stabilize_projection_camera_with_line_height(
        viewport,
        total_units,
        visible_top,
        visible_bottom,
        density,
        density.line_height(),
        content_anchor,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn stabilize_projection_camera_with_line_height(
    mut viewport: ProjectionViewport,
    total_units: f32,
    visible_top: f32,
    visible_bottom: f32,
    density: Density,
    line_height: f32,
    content_anchor: f32,
) -> ProjectionViewport {
    let line_height = line_height.max(f32::EPSILON);
    let visible_minimap_units =
        ((viewport.interaction_height - density.edge_padding() * 2.0) / line_height).max(1.0);
    let max_content_top = (total_units - visible_minimap_units).max(0.0);
    let visible_top = visible_top.clamp(0.0, total_units);
    let visible_bottom = visible_bottom.clamp(visible_top, total_units);
    let minimum_content_top = (visible_bottom - visible_minimap_units)
        .max(0.0)
        .min(max_content_top);
    let maximum_content_top = visible_top.max(minimum_content_top).min(max_content_top);
    viewport.content_top = content_anchor.clamp(minimum_content_top, maximum_content_top);

    viewport.thumb_top = ((visible_top - viewport.content_top).max(0.0) * line_height).clamp(
        0.0,
        (viewport.interaction_height - viewport.thumb_height).max(0.0),
    );
    viewport
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DragSession {
    pub start_pointer_y: f32,
    pub start_thumb_top: f32,
    pub start_ratio: f32,
    pub current_thumb_top: f32,
}

pub(crate) fn drag_target(
    local_y: f32,
    session: DragSession,
    thumb_height: f32,
    track_height: f32,
) -> (f32, f32) {
    let travel = (track_height - thumb_height).max(0.0);
    if travel <= f32::EPSILON {
        return (0.0, 0.0);
    }
    let start_top = session.start_thumb_top.clamp(0.0, travel);
    let thumb_top = (start_top + local_y - session.start_pointer_y).clamp(0.0, travel);
    let start_ratio = session.start_ratio.clamp(0.0, 1.0);
    let ratio = if thumb_top >= start_top {
        let remaining_travel = travel - start_top;
        if remaining_travel <= f32::EPSILON {
            1.0
        } else {
            start_ratio + (thumb_top - start_top) / remaining_travel * (1.0 - start_ratio)
        }
    } else if start_top <= f32::EPSILON {
        0.0
    } else {
        start_ratio - (start_top - thumb_top) / start_top * start_ratio
    };
    (ratio.clamp(0.0, 1.0), thumb_top)
}

pub(crate) fn thumb_alphas(active: bool, hovered: bool, hover_only: bool) -> (u32, u32) {
    if active {
        (0x48, 0xe8)
    } else if hovered {
        (0x34, 0xd0)
    } else if hover_only {
        (0, 0)
    } else {
        (0x24, 0xb0)
    }
}

type TextRasterizer = Mutex<(cosmic_text::FontSystem, cosmic_text::SwashCache)>;
static TEXT_RASTERIZER: OnceLock<TextRasterizer> = OnceLock::new();

pub(crate) fn text_rasterizer_initialized() -> bool {
    TEXT_RASTERIZER.get().is_some()
}

pub(crate) fn text_rasterizer() -> &'static TextRasterizer {
    TEXT_RASTERIZER.get_or_init(|| {
        Mutex::new((
            cosmic_text::FontSystem::new(),
            cosmic_text::SwashCache::new(),
        ))
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_text_pixels(
    pixels: &mut [u8],
    image_width: usize,
    image_height: usize,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    color: cosmic_text::Color,
) {
    for py in 0..height as i32 {
        let target_y = y + py;
        if target_y < 0 || target_y >= image_height as i32 {
            continue;
        }
        for px_offset in 0..width as i32 {
            let target_x = x + px_offset;
            if target_x < 0 || target_x >= image_width as i32 {
                continue;
            }
            let offset = (target_y as usize * image_width + target_x as usize) * 4;
            pixels[offset] = color.b();
            pixels[offset + 1] = color.g();
            pixels[offset + 2] = color.r();
            pixels[offset + 3] = color.a();
        }
    }
}

pub fn ratio_to_offset(ratio: f32, total: f32, viewport: f32) -> f32 {
    ratio.clamp(0.0, 1.0) * (total - viewport).max(0.0)
}
pub fn offset_to_ratio(offset: f32, total: f32, viewport: f32) -> f32 {
    if total <= viewport {
        0.0
    } else {
        (offset / (total - viewport)).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scroll_ratio_reaches_the_document_end() {
        assert_eq!(ratio_to_offset(1.0, 1000.0, 200.0), 800.0);
    }

    #[test]
    fn shared_drag_geometry_reaches_both_ends_without_jumping() {
        let session = DragSession {
            start_pointer_y: 10.0,
            start_thumb_top: 0.0,
            start_ratio: 0.0,
            current_thumb_top: 0.0,
        };
        assert_eq!(drag_target(10.0, session, 24.0, 300.0), (0.0, 0.0));
        assert_eq!(drag_target(400.0, session, 24.0, 300.0), (1.0, 276.0));
    }

    #[test]
    fn projection_viewport_keeps_document_and_thumb_bottom_aligned() {
        let viewport = projection_viewport(1_000.0, 900.0, 1_000.0, 1.0, 300.0, Density::Compact);
        assert_eq!(
            viewport.content_top
                + (viewport.interaction_height - Density::Compact.edge_padding() * 2.0)
                    / Density::Compact.line_height(),
            1_000.0
        );
        assert_eq!(
            viewport.thumb_top + viewport.thumb_height,
            viewport.interaction_height
        );
    }
}
