use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use gpui::{Bounds, Pixels, RenderImage};
use image::{Frame, RgbaImage};
use smallvec::SmallVec;

use crate::document::{ByteRange, Revision, TextSnapshot};

pub(super) const DEFAULT_WIDTH: f32 = 96.0;
pub(super) const MIN_WIDTH: f32 = 56.0;
pub(super) const MAX_WIDTH: f32 = 220.0;
pub(super) const RESIZE_HANDLE: f32 = 6.0;
const RASTER_PREFETCH_ROWS: usize = 64;
const MAX_RASTER_ROWS: usize = 700;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LineKind {
    Plain,
    Heading(u8),
    Table,
    Code,
    Quote,
    Property,
}

pub(super) fn classify_line(text: &str) -> LineKind {
    let trimmed = text.trim_start();
    let stars = trimmed.bytes().take_while(|byte| *byte == b'*').count();
    if stars > 0 && trimmed.as_bytes().get(stars) == Some(&b' ') {
        LineKind::Heading(stars.min(u8::MAX as usize) as u8)
    } else if trimmed.starts_with('|') {
        LineKind::Table
    } else if starts_with_ascii_case_insensitive(trimmed, "#+begin_")
        || starts_with_ascii_case_insensitive(trimmed, "#+end_")
    {
        LineKind::Code
    } else if trimmed.starts_with('>') {
        LineKind::Quote
    } else if trimmed.starts_with(':') && trimmed.ends_with(':') {
        LineKind::Property
    } else {
        LineKind::Plain
    }
}

fn starts_with_ascii_case_insensitive(text: &str, prefix: &str) -> bool {
    text.as_bytes()
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix.as_bytes()))
}

pub(super) type ViewportGeometry = crate::minimap::ProjectionViewport;

#[cfg(test)]
pub(super) fn viewport_geometry(
    total_units: f32,
    scroll_top: f32,
    viewport_units: f32,
    track_height: f32,
    density: crate::minimap::Density,
) -> ViewportGeometry {
    viewport_geometry_for_range(
        total_units,
        scroll_top,
        (scroll_top + viewport_units).min(total_units),
        track_height,
        density,
    )
}

#[cfg(test)]
pub(super) fn viewport_geometry_for_range(
    total_units: f32,
    visible_top: f32,
    visible_bottom: f32,
    track_height: f32,
    density: crate::minimap::Density,
) -> ViewportGeometry {
    let line_height = raster_line_height(density, 1.0);
    let viewport_units = (visible_bottom - visible_top).max(0.0);
    let max_scroll = (total_units - viewport_units).max(0.0);
    let scroll_top = visible_top.clamp(0.0, max_scroll);
    let scroll_ratio = if max_scroll > 0.0 {
        scroll_top / max_scroll
    } else {
        0.0
    };
    crate::minimap::projection_viewport_with_line_height(
        total_units,
        scroll_top,
        visible_bottom.clamp(scroll_top, total_units),
        scroll_ratio,
        track_height,
        density,
        line_height,
    )
}

pub(super) fn raster_line_height(density: crate::minimap::Density, scale_factor: f32) -> f32 {
    let scale_factor = scale_factor.max(1.0);
    (density.line_height() * scale_factor).round().max(1.0) / scale_factor
}

pub(super) struct TextRow {
    pub(super) text: String,
    pub(super) color: u32,
    pub(super) indent: f32,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn rasterize_text_rows(
    rows: &[TextRow],
    logical_width: usize,
    scale_factor: f32,
    density: crate::minimap::Density,
    line_height: f32,
    epoch: &AtomicU64,
    expected_epoch: u64,
) -> Option<Arc<RenderImage>> {
    use cosmic_text::{Attrs, Buffer, Color, Family, Metrics, Shaping, Wrap};

    let scale_factor = scale_factor.max(1.0);
    let physical_line_height = line_height * scale_factor;
    let logical_width = logical_width.max(1);
    let width = (logical_width as f32 * scale_factor).ceil().max(1.0) as u32;
    let height = (rows.len().max(1) as f32 * physical_line_height)
        .ceil()
        .max(1.0) as u32;
    let mut pixels = vec![0_u8; width as usize * height as usize * 4];
    let mut rasterizer = crate::minimap::text_rasterizer()
        .lock()
        .expect("minimap rasterizer poisoned");
    if epoch.load(Ordering::Acquire) != expected_epoch {
        return None;
    }
    let (font_system, swash_cache) = &mut *rasterizer;
    let attrs = Attrs::new()
        .family(Family::Name("Menlo"))
        .weight(cosmic_text::Weight::BLACK);
    let mut buffer = Buffer::new(
        font_system,
        Metrics::new(density.font_px() * scale_factor, physical_line_height),
    );
    buffer.set_wrap(Wrap::None);
    for (row, source) in rows.iter().enumerate() {
        if row.is_multiple_of(32) && epoch.load(Ordering::Relaxed) != expected_epoch {
            return None;
        }
        buffer.set_size(
            Some(width as f32 - 6.0 * scale_factor),
            Some(physical_line_height),
        );
        buffer.set_text(&source.text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);
        let base = Color::rgb(
            (source.color >> 16) as u8,
            (source.color >> 8) as u8,
            source.color as u8,
        );
        let origin_x = (source.indent * scale_factor).round() as i32;
        let origin_y = (row as f32 * physical_line_height).round() as i32;
        buffer.draw(font_system, swash_cache, base, |x, y, w, h, color| {
            crate::minimap::paint_text_pixels(
                &mut pixels,
                width as usize,
                height as usize,
                x + origin_x,
                y + origin_y,
                w,
                h,
                color,
            );
        });
    }
    let image = RgbaImage::from_raw(width, height, pixels).expect("valid minimap image dimensions");
    Some(Arc::new(RenderImage::new(SmallVec::from_elem(
        Frame::new(image),
        1,
    ))))
}

pub(super) fn raster_window(
    content_top: f32,
    interaction_height: f32,
    total_units: f32,
    density: crate::minimap::Density,
    line_height: f32,
) -> (u64, usize) {
    let first_visible = content_top.floor().max(0.0) as u64;
    let first = first_visible.saturating_sub(RASTER_PREFETCH_ROWS as u64)
        / RASTER_PREFETCH_ROWS as u64
        * RASTER_PREFETCH_ROWS as u64;
    let visible = ((interaction_height - density.edge_padding() * 2.0) / line_height.max(1.0))
        .ceil()
        .clamp(1.0, (MAX_RASTER_ROWS - RASTER_PREFETCH_ROWS * 2) as f32) as usize
        + 1;
    let requested = (visible + RASTER_PREFETCH_ROWS * 2).min(MAX_RASTER_ROWS);
    let available = (total_units.ceil().max(1.0) as u64)
        .saturating_sub(first)
        .min(usize::MAX as u64) as usize;
    (first, requested.min(available))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RasterKey {
    pub(super) generation: u64,
    pub(super) first_unit: u64,
    pub(super) rows: u16,
    pub(super) width: u16,
    pub(super) scale_x100: u16,
    pub(super) density: crate::minimap::Density,
}

#[derive(Clone)]
pub(super) struct CachedRaster {
    pub(super) key: RasterKey,
    pub(super) image: Arc<RenderImage>,
    pub(super) content_top: f32,
    pub(super) viewport_generation: u64,
    pub(super) line_height: f32,
}

pub(super) fn raster_placement_content_top(
    cached: &CachedRaster,
    viewport_generation: u64,
    current_content_top: f32,
) -> f32 {
    if cached.viewport_generation == viewport_generation {
        cached.content_top
    } else {
        current_content_top
    }
}

#[derive(Clone, Copy, Debug)]
struct ViewportAnchor {
    viewport_generation: u64,
    track_height: f32,
    density: crate::minimap::Density,
    content_top: f32,
}

pub(super) struct EditorMinimapHost {
    pub(super) visible: bool,
    pub(super) width: f32,
    pub(super) bounds: Option<Bounds<Pixels>>,
    pub(super) drag: Option<crate::minimap::DragSession>,
    pub(super) resizing: Option<(f32, f32)>,
    pub(super) generation: u64,
    pub(super) revision: Option<Revision>,
    pub(super) search_marks: Arc<[ByteRange]>,
    pub(super) raster: std::sync::Mutex<Option<CachedRaster>>,
    pub(super) raster_build: std::sync::Mutex<Option<RasterKey>>,
    pub(super) raster_epoch: Arc<AtomicU64>,
    pub(super) scale_factor: f32,
    viewport_generation: u64,
    viewport_anchor: std::sync::Mutex<Option<ViewportAnchor>>,
}

impl Default for EditorMinimapHost {
    fn default() -> Self {
        Self {
            visible: true,
            width: DEFAULT_WIDTH,
            bounds: None,
            drag: None,
            resizing: None,
            generation: 0,
            revision: None,
            search_marks: Arc::from([]),
            raster: std::sync::Mutex::new(None),
            raster_build: std::sync::Mutex::new(None),
            raster_epoch: Arc::new(AtomicU64::new(0)),
            scale_factor: 1.0,
            viewport_generation: 0,
            viewport_anchor: std::sync::Mutex::new(None),
        }
    }
}

impl EditorMinimapHost {
    pub(super) fn stabilize_viewport(
        &self,
        mut viewport: ViewportGeometry,
        total_units: f32,
        visible_top: f32,
        visible_bottom: f32,
        track_height: f32,
        density: crate::minimap::Density,
    ) -> ViewportGeometry {
        let mut anchor = self
            .viewport_anchor
            .lock()
            .expect("editor minimap viewport anchor poisoned");
        let can_reuse = anchor.is_some_and(|anchor| {
            anchor.viewport_generation == self.viewport_generation
                && (anchor.track_height - track_height).abs() < 0.5
                && anchor.density == density
        });
        let preferred_content_top = if can_reuse {
            let previous = (*anchor).expect("reuse requires an anchor");
            viewport = crate::minimap::stabilize_projection_camera_with_line_height(
                viewport,
                total_units,
                visible_top,
                visible_bottom,
                density,
                self.line_height(density),
                previous.content_top,
            );
            previous.content_top
        } else {
            viewport.content_top
        };
        *anchor = Some(ViewportAnchor {
            viewport_generation: self.viewport_generation,
            track_height,
            density,
            content_top: preferred_content_top,
        });
        viewport
    }

    pub(super) fn note_viewport_changed(&mut self) {
        self.viewport_generation = self.viewport_generation.wrapping_add(1);
    }

    pub(super) fn viewport_generation(&self) -> u64 {
        self.viewport_generation
    }

    pub(super) fn line_height(&self, density: crate::minimap::Density) -> f32 {
        raster_line_height(density, self.scale_factor)
    }

    pub(super) fn reserve_raster(&self, key: RasterKey) -> bool {
        let mut in_flight = self
            .raster_build
            .lock()
            .expect("editor minimap raster build poisoned");
        if in_flight.is_some() {
            false
        } else {
            *in_flight = Some(key);
            true
        }
    }

    pub(super) fn invalidate_raster(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.raster_epoch.fetch_add(1, Ordering::AcqRel);
        *self
            .raster_build
            .lock()
            .expect("editor minimap raster build poisoned") = None;
    }

    pub(super) fn update_snapshot(&mut self, snapshot: &crate::document::DocumentSnapshot) {
        if self.revision != Some(snapshot.revision()) {
            self.invalidate_raster();
            self.revision = Some(snapshot.revision());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_keep_independent_generation_and_interaction_state() {
        let snapshot =
            crate::document::DocumentSnapshot::from_utf8(b"* one\nbody\n".to_vec()).unwrap();
        let mut left = EditorMinimapHost::default();
        let mut right = EditorMinimapHost::default();
        left.update_snapshot(&snapshot);
        right.update_snapshot(&snapshot);
        left.drag = Some(crate::minimap::DragSession {
            start_pointer_y: 4.0,
            start_thumb_top: 0.0,
            start_ratio: 0.0,
            current_thumb_top: 0.0,
        });
        left.width = 140.0;
        assert_eq!(left.generation, 1);
        assert_eq!(right.generation, 1);
        assert_eq!(right.drag, None);
        assert_eq!(right.width, DEFAULT_WIDTH);
    }

    #[test]
    fn raster_builds_are_serialized_instead_of_replaced_during_scroll() {
        let host = EditorMinimapHost::default();
        let first = RasterKey {
            generation: 1,
            first_unit: 0,
            rows: 200,
            width: 96,
            scale_x100: 200,
            density: crate::minimap::Density::Compact,
        };
        let next = RasterKey {
            first_unit: 64,
            ..first
        };
        assert!(host.reserve_raster(first));
        assert!(!host.reserve_raster(next));
        assert_eq!(
            *host
                .raster_build
                .lock()
                .expect("editor minimap raster build poisoned"),
            Some(first)
        );
    }

    #[test]
    fn raster_window_is_stable_within_a_prefetched_scroll_chunk() {
        let density = crate::minimap::Density::Compact;
        let line_height = raster_line_height(density, 2.0);
        let first = raster_window(70.0, 300.0, 1_000.0, density, line_height);
        let nearby = raster_window(100.0, 300.0, 1_000.0, density, line_height);
        let next = raster_window(129.0, 300.0, 1_000.0, density, line_height);
        assert_eq!(first, nearby);
        assert_eq!(first.0, 0);
        assert_eq!(next.0, 64);
        assert!(first.0 as f32 + first.1 as f32 > 100.0);
    }

    #[test]
    fn raster_window_does_not_include_rows_after_document_end() {
        let density = crate::minimap::Density::Compact;
        let line_height = raster_line_height(density, 2.0);
        let (first, rows) = raster_window(900.0, 300.0, 1_000.0, density, line_height);
        assert_eq!(first + rows as u64, 1_000);
    }

    #[test]
    fn editor_raster_rows_align_to_physical_pixels() {
        for density in [
            crate::minimap::Density::Compact,
            crate::minimap::Density::Comfortable,
            crate::minimap::Density::Large,
            crate::minimap::Density::ExtraLarge,
            crate::minimap::Density::Maximum,
        ] {
            for scale_factor in [1.0, 1.5, 2.0, 3.0] {
                let physical = raster_line_height(density, scale_factor) * scale_factor;
                assert!((physical - physical.round()).abs() < f32::EPSILON);
            }
        }
    }

    #[test]
    fn invalidation_cancels_the_previous_raster_epoch() {
        let mut host = EditorMinimapHost::default();
        let epoch = host.raster_epoch.load(Ordering::Acquire);
        host.invalidate_raster();
        assert_ne!(host.raster_epoch.load(Ordering::Acquire), epoch);
    }

    #[test]
    fn invalidation_keeps_previous_pixels_until_replacement_is_ready() {
        let mut host = EditorMinimapHost::default();
        let key = RasterKey {
            generation: 0,
            first_unit: 0,
            rows: 1,
            width: 1,
            scale_x100: 100,
            density: crate::minimap::Density::Compact,
        };
        let image = Arc::new(RenderImage::new(SmallVec::from_elem(
            Frame::new(RgbaImage::new(1, 1)),
            1,
        )));
        *host.raster.lock().expect("editor minimap raster poisoned") = Some(CachedRaster {
            key,
            image,
            content_top: 0.0,
            viewport_generation: 0,
            line_height: raster_line_height(crate::minimap::Density::Compact, 1.0),
        });

        host.invalidate_raster();

        assert!(
            host.raster
                .lock()
                .expect("editor minimap raster poisoned")
                .is_some()
        );
    }

    #[test]
    fn retained_raster_stays_on_its_camera_until_the_editor_actually_scrolls() {
        let key = RasterKey {
            generation: 0,
            first_unit: 64,
            rows: 1,
            width: 1,
            scale_x100: 100,
            density: crate::minimap::Density::Compact,
        };
        let image = Arc::new(RenderImage::new(SmallVec::from_elem(
            Frame::new(RgbaImage::new(1, 1)),
            1,
        )));
        let cached = CachedRaster {
            key,
            image,
            content_top: 100.0,
            viewport_generation: 7,
            line_height: raster_line_height(crate::minimap::Density::Compact, 2.0),
        };

        assert_eq!(raster_placement_content_top(&cached, 7, 101.0), 100.0);
        assert_eq!(raster_placement_content_top(&cached, 8, 101.0), 101.0);
    }

    #[test]
    fn editor_viewport_geometry_pins_the_thumb_to_both_ends() {
        let density = crate::minimap::Density::Compact;
        let start = viewport_geometry(1_000.0, 0.0, 100.0, 300.0, density);
        let end = viewport_geometry(1_000.0, 900.0, 100.0, 300.0, density);
        assert_eq!(start.thumb_top, 0.0);
        assert!((end.thumb_top - (end.interaction_height - end.thumb_height)).abs() < 0.001);
    }

    #[test]
    fn document_growth_keeps_the_viewport_top_stable_without_editor_scroll() {
        let host = EditorMinimapHost::default();
        let density = crate::minimap::Density::Compact;
        let before = host.stabilize_viewport(
            viewport_geometry(1_000.0, 400.0, 30.0, 300.0, density),
            1_000.0,
            400.0,
            430.0,
            300.0,
            density,
        );
        let after = host.stabilize_viewport(
            viewport_geometry(1_001.0, 400.0, 30.0, 300.0, density),
            1_001.0,
            400.0,
            430.0,
            300.0,
            density,
        );
        assert_eq!(after.content_top, before.content_top);
        assert_eq!(after.thumb_top, before.thumb_top);
    }

    #[test]
    fn document_growth_does_not_jump_when_the_viewport_stops_being_at_the_end() {
        let host = EditorMinimapHost::default();
        let density = crate::minimap::Density::Compact;
        let before = host.stabilize_viewport(
            viewport_geometry_for_range(749.0, 699.0, 749.0, 700.0, density),
            749.0,
            699.0,
            749.0,
            700.0,
            density,
        );
        let after = host.stabilize_viewport(
            viewport_geometry_for_range(754.0, 700.0, 750.0, 700.0, density),
            754.0,
            700.0,
            750.0,
            700.0,
            density,
        );
        assert!((after.content_top - before.content_top - 1.0).abs() < 0.001);
        assert_eq!(after.thumb_top, before.thumb_top);
        assert_eq!(after.thumb_height, before.thumb_height);
    }

    #[test]
    fn actual_editor_scroll_releases_the_stable_camera() {
        let mut host = EditorMinimapHost::default();
        let density = crate::minimap::Density::Compact;
        let before = host.stabilize_viewport(
            viewport_geometry_for_range(1_000.0, 400.0, 430.0, 300.0, density),
            1_000.0,
            400.0,
            430.0,
            300.0,
            density,
        );
        let without_scroll = host.stabilize_viewport(
            viewport_geometry_for_range(1_000.0, 410.0, 440.0, 300.0, density),
            1_000.0,
            410.0,
            440.0,
            300.0,
            density,
        );
        assert_eq!(without_scroll.content_top, before.content_top);

        host.note_viewport_changed();
        let projected = viewport_geometry_for_range(1_000.0, 600.0, 630.0, 300.0, density);
        let after_scroll =
            host.stabilize_viewport(projected, 1_000.0, 600.0, 630.0, 300.0, density);
        assert_eq!(after_scroll.content_top, projected.content_top);
        assert_ne!(after_scroll.content_top, without_scroll.content_top);
    }

    #[test]
    fn reversible_edits_do_not_ratchet_the_camera() {
        let host = EditorMinimapHost::default();
        let density = crate::minimap::Density::Large;
        let original = host.stabilize_viewport(
            viewport_geometry_for_range(745.0, 721.0, 742.0, 691.0, density),
            745.0,
            721.0,
            742.0,
            691.0,
            density,
        );
        for total in 746..=749 {
            let bottom = 742.0 + (total - 745) as f32 * 0.5;
            host.stabilize_viewport(
                viewport_geometry_for_range(total as f32, 721.0, bottom, 691.0, density),
                total as f32,
                721.0,
                bottom,
                691.0,
                density,
            );
        }
        for total in (745..=748).rev() {
            let bottom = 742.0 + (total - 745) as f32 * 0.5;
            host.stabilize_viewport(
                viewport_geometry_for_range(total as f32, 721.0, bottom, 691.0, density),
                total as f32,
                721.0,
                bottom,
                691.0,
                density,
            );
        }
        let restored = host.stabilize_viewport(
            viewport_geometry_for_range(745.0, 721.0, 742.0, 691.0, density),
            745.0,
            721.0,
            742.0,
            691.0,
            density,
        );
        assert_eq!(restored.content_top, original.content_top);
        assert_eq!(restored.thumb_top, original.thumb_top);
        assert_eq!(restored.thumb_height, original.thumb_height);
    }
}
