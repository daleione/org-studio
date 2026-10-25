use super::*;
use image::{Frame, RgbaImage};
use smallvec::SmallVec;

fn prepared_layout_key(revision: u64, wrap_width: f32) -> PreparedLayoutKey {
    PreparedLayoutKey {
        source_reveals: Arc::from([]),
        display_options: Default::default(),
        revision: Revision(revision),
        path: std::path::PathBuf::from("fixture.org"),
        line_count: 100,
        wrap_width_bits: wrap_width.to_bits(),
        base_line_height_bits: 24.0_f32.to_bits(),
        soft_wrap: true,
        font: Font::default(),
        font_size_bits: 15.0_f32.to_bits(),
        content_scale_bits: 1.0_f32.to_bits(),
        fold_revision: 0,
        inline_images: true,
        inline_image_overrides: Arc::from([]),
        inline_image_resource_generation: 0,
        table_layout_revision: 0,
    }
}

#[test]
fn prepared_layout_build_is_deduplicated_and_published_for_the_exact_key() {
    let host = EditorMinimapHost::default();
    let key = prepared_layout_key(1, 400.0);
    let epoch = host
        .reserve_layout_preparation(key.clone())
        .expect("first request should reserve a build");
    assert!(host.reserve_layout_preparation(key.clone()).is_none());

    let mut layout = super::super::layout_map::EditorLayoutMap::default();
    layout.configure(100, 400.0);
    let layout = Arc::new(layout);
    assert!(host.publish_prepared_layout(&key, epoch, layout.clone()));
    assert!(Arc::ptr_eq(
        &host
            .prepared_layout()
            .expect("prepared layout should be ready"),
        &layout
    ));
    assert!(host.reserve_layout_preparation(key).is_none());
}

#[test]
fn changed_geometry_key_cancels_stale_background_publication() {
    let host = EditorMinimapHost::default();
    let old = prepared_layout_key(1, 400.0);
    let old_epoch = host
        .reserve_layout_preparation(old.clone())
        .expect("old build should reserve");
    let new = prepared_layout_key(1, 500.0);
    let new_epoch = host
        .reserve_layout_preparation(new.clone())
        .expect("new geometry should replace the build");
    assert_ne!(old_epoch, new_epoch);

    let layout = Arc::new(super::super::layout_map::EditorLayoutMap::default());
    assert!(!host.publish_prepared_layout(&old, old_epoch, layout));
    assert!(host.prepared_layout().is_none());
}

#[test]
fn complete_layout_ignores_later_sparse_visible_measurements() {
    let mut host = EditorMinimapHost::default();
    let key = prepared_layout_key(1, 400.0);
    let epoch = host
        .reserve_layout_preparation(key.clone())
        .expect("build should reserve");
    let layout = Arc::new(super::super::layout_map::EditorLayoutMap::default());
    assert!(host.publish_prepared_layout(&key, epoch, layout));
    let generation = host.generation;
    let raster_epoch = host.raster_epoch.load(Ordering::Acquire);

    host.note_layout_changed();
    host.last_layout_raster_activity = Instant::now() - LAYOUT_RASTER_SETTLE;
    assert!(!host.settle_layout_raster_invalidation());
    assert_eq!(host.generation, generation);
    assert_eq!(host.raster_epoch.load(Ordering::Acquire), raster_epoch);
    assert!(!host.pending_layout_raster_invalidation);
}

#[test]
fn wrapped_rows_fill_text_segments_and_background_spacing_without_repetition() {
    let mut raster_lines = vec![None; 8];
    fill_visual_rows(&mut raster_lines, 12, 1.25, 3.5, &[5, 9]);
    fill_visual_rows(&mut raster_lines, 13, 4.75, 1.0, &[]);

    assert_eq!(
        raster_lines,
        vec![
            None,
            Some(RasterSourceRow {
                line: 12,
                text_range: Some((0, Some(5))),
            }),
            Some(RasterSourceRow {
                line: 12,
                text_range: Some((5, Some(9))),
            }),
            Some(RasterSourceRow {
                line: 12,
                text_range: Some((9, None)),
            }),
            Some(RasterSourceRow {
                line: 13,
                text_range: Some((0, None)),
            }),
            Some(RasterSourceRow {
                line: 13,
                text_range: None,
            }),
            None,
            None
        ]
    );
}

#[test]
fn hosts_keep_independent_generation_and_interaction_state() {
    let snapshot = crate::document::DocumentSnapshot::from_utf8(b"* one\nbody\n".to_vec()).unwrap();
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
    assert_ne!(left.telemetry.host_id(), right.telemetry.host_id());
}

#[test]
fn pending_age_is_owned_and_resolved_by_each_host() {
    let host = EditorMinimapHost::default();
    host.note_semantics_pending(true);
    assert!(host.telemetry.pending_since.lock().unwrap().is_some());
    host.note_semantics_pending(true);
    assert!(host.telemetry.pending_since.lock().unwrap().is_some());
    host.note_semantics_pending(false);
    assert!(host.telemetry.pending_since.lock().unwrap().is_none());
}

#[test]
fn repeated_raster_rows_are_counted_only_within_the_same_revision() {
    let telemetry = EditorMinimapTelemetry::new();
    assert_eq!(telemetry.note_request(Revision(1), &[10, 11, 12]), 0);
    assert_eq!(telemetry.note_request(Revision(1), &[12, 13, 14]), 1);
    assert_eq!(telemetry.note_request(Revision(2), &[12, 13, 14]), 0);
}

#[test]
fn newer_raster_window_supersedes_an_in_flight_scroll_request() {
    let host = EditorMinimapHost::default();
    let first = RasterKey {
        generation: 1,
        first_unit: 0,
        rows: 200,
        width: 96,
        scale_x100: 200,
        density: crate::minimap::Density::Compact,
        theme_generation: 0,
    };
    let next = RasterKey {
        first_unit: 64,
        ..first
    };
    assert!(host.reserve_raster(first));
    let first_epoch = host.raster_epoch.load(Ordering::Acquire);
    assert!(!host.reserve_raster(first));
    assert_eq!(host.raster_epoch.load(Ordering::Acquire), first_epoch);
    assert!(host.reserve_raster(next));
    assert_ne!(host.raster_epoch.load(Ordering::Acquire), first_epoch);
    assert_eq!(host.telemetry.stale_cancels.load(Ordering::Relaxed), 1);
    assert_eq!(
        *host
            .raster_build
            .lock()
            .expect("editor minimap raster build poisoned"),
        Some(next)
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
    let visible_rows = ((300.0 - density.edge_padding() * 2.0) / line_height).ceil() + 1.0;
    let old_end = first.0 as f32 + first.1 as f32;
    let next_visible_end = 129.0 + visible_rows;
    assert!(old_end - next_visible_end >= (RASTER_PREFETCH_ROWS - 1) as f32);
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
    assert!(host.reserve_raster(RasterKey {
        generation: 0,
        first_unit: 0,
        rows: 1,
        width: 96,
        scale_x100: 100,
        density: crate::minimap::Density::Compact,
        theme_generation: 0,
    }));
    host.invalidate_raster();
    assert_ne!(host.raster_epoch.load(Ordering::Acquire), epoch);
    assert_eq!(host.telemetry.stale_cancels.load(Ordering::Relaxed), 1);
}

#[test]
fn measured_layout_updates_are_published_once_after_scroll_settles() {
    let mut host = EditorMinimapHost::default();
    let generation = host.generation;
    let epoch = host.raster_epoch.load(Ordering::Acquire);

    host.note_layout_changed();
    assert!(host.settle_layout_raster_invalidation());
    assert_eq!(host.generation, generation);
    assert_eq!(host.raster_epoch.load(Ordering::Acquire), epoch);

    host.note_viewport_changed();
    host.last_layout_raster_activity = Instant::now() - LAYOUT_RASTER_SETTLE;
    assert!(!host.settle_layout_raster_invalidation());
    assert_eq!(host.generation, generation.wrapping_add(1));
    assert_ne!(host.raster_epoch.load(Ordering::Acquire), epoch);
    assert!(!host.pending_layout_raster_invalidation);

    assert!(!host.settle_layout_raster_invalidation());
    assert_eq!(host.generation, generation.wrapping_add(1));
}

#[test]
fn explicit_invalidation_supersedes_a_pending_layout_update() {
    let mut host = EditorMinimapHost::default();
    host.note_layout_changed();

    host.invalidate_raster();

    assert!(!host.pending_layout_raster_invalidation);
    assert!(!host.settle_layout_raster_invalidation());
    assert_eq!(host.generation, 1);
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
        theme_generation: 0,
    };
    let image = Arc::new(RenderImage::new(SmallVec::from_elem(
        Frame::new(RgbaImage::new(1, 1)),
        1,
    )));
    let layout = Arc::new(super::super::layout_map::EditorLayoutMap::default());
    host.publish_frame(PreparedEditorMinimapFrame::new(
        layout,
        CachedRaster {
            key,
            image,
            media: Arc::from([]),
            content_top: 0.0,
            viewport_generation: 0,
            line_height: raster_line_height(crate::minimap::Density::Compact, 1.0),
        },
    ));

    host.invalidate_raster();

    assert!(
        host.prepared_frame
            .lock()
            .expect("editor minimap prepared frame poisoned")
            .is_some()
    );
}

#[test]
fn prepared_frame_owns_the_layout_used_by_its_raster() {
    let mut layout = super::super::layout_map::EditorLayoutMap::default();
    layout.configure(100, 400.0);
    let layout = Arc::new(layout);
    let key = RasterKey {
        generation: 7,
        first_unit: 32,
        rows: 1,
        width: 1,
        scale_x100: 100,
        density: crate::minimap::Density::Compact,
        theme_generation: 0,
    };
    let image = Arc::new(RenderImage::new(SmallVec::from_elem(
        Frame::new(RgbaImage::new(1, 1)),
        1,
    )));
    let frame = PreparedEditorMinimapFrame::new(
        layout.clone(),
        CachedRaster {
            key,
            image,
            media: Arc::from([]),
            content_top: 32.0,
            viewport_generation: 3,
            line_height: 1.0,
        },
    );

    assert!(frame.is_coherent());
    assert_eq!(frame.geometry_generation.raster_generation, 7);
    assert_eq!(
        frame.geometry_generation.layout_identity,
        Arc::as_ptr(&layout) as usize
    );
}

#[test]
fn publishing_refined_geometry_preserves_the_active_source_anchor() {
    fn raster(generation: u64) -> CachedRaster {
        CachedRaster {
            key: RasterKey {
                generation,
                first_unit: 0,
                rows: 1,
                width: 1,
                scale_x100: 100,
                density: crate::minimap::Density::Compact,
                theme_generation: 0,
            },
            image: Arc::new(RenderImage::new(SmallVec::from_elem(
                Frame::new(RgbaImage::new(1, 1)),
                1,
            ))),
            media: Arc::from([]),
            content_top: 0.0,
            viewport_generation: 0,
            line_height: 1.0,
        }
    }

    let mut original = super::super::layout_map::EditorLayoutMap::default();
    original.configure(1_000, 400.0);
    let original = Arc::new(original);
    let mut refined = original.as_ref().clone();
    refined.update_line_layout(100, 12, 24.0, 0.0, 0.0);
    let refined = Arc::new(refined);
    let host = EditorMinimapHost::default();
    host.publish_frame(PreparedEditorMinimapFrame::new(original.clone(), raster(1)));
    {
        let mut camera = host
            .scroll_camera
            .lock()
            .expect("editor minimap scroll camera poisoned");
        camera.content_y = original.line_start_y(500) + original.line_height_px(500) * 0.25;
        camera.layout_identity = Arc::as_ptr(&original) as usize;
        camera.initialized = true;
    }

    host.publish_frame(PreparedEditorMinimapFrame::new(refined.clone(), raster(2)));

    let camera = host
        .scroll_camera
        .lock()
        .expect("editor minimap scroll camera poisoned");
    let expected = refined.line_start_y(500) + refined.line_height_px(500) * 0.25;
    assert!((camera.content_y - expected).abs() < 0.001);
    assert_eq!(camera.layout_identity, Arc::as_ptr(&refined) as usize);
}

#[test]
fn prepared_layout_projection_pins_both_live_scroll_endpoints() {
    let mut layout = super::super::layout_map::EditorLayoutMap::default();
    layout.configure(1_000, 400.0);
    layout.update_line_layout(900, 8, 24.0, 0.0, 0.0);
    let viewport_height = 240.0;
    let live_document_height = 40_000.0;

    let top =
        source_viewport_for_layout(&layout, &layout, 0.0, live_document_height, viewport_height);
    let bottom = source_viewport_for_layout(
        &layout,
        &layout,
        live_document_height - viewport_height,
        live_document_height,
        viewport_height,
    );

    assert_eq!(top.visible_top, 0.0);
    assert_eq!(top.scroll_ratio, 0.0);
    assert_eq!(bottom.visible_bottom, bottom.total_units);
    assert_eq!(bottom.scroll_ratio, 1.0);
}

#[test]
fn prepared_layout_projection_does_not_reverse_when_live_height_grows() {
    let mut prepared = super::super::layout_map::EditorLayoutMap::default();
    prepared.configure(1_000, 400.0);
    let mut live = prepared.clone();
    let anchor_line = 500;
    let anchor_fraction = 0.25;
    let viewport_height = 240.0;
    let before_scroll =
        live.line_start_y(anchor_line) + anchor_fraction * live.line_height_px(anchor_line);
    let before = source_viewport_for_layout(
        &prepared,
        &live,
        before_scroll,
        live.total_height(),
        viewport_height,
    );

    live.update_line_layout(100, 12, 24.0, 0.0, 0.0);
    live.update_line_layout(300, 8, 24.0, 0.0, 0.0);
    let after_scroll =
        live.line_start_y(anchor_line) + anchor_fraction * live.line_height_px(anchor_line);
    let after = source_viewport_for_layout(
        &prepared,
        &live,
        after_scroll,
        live.total_height(),
        viewport_height,
    );

    assert!((after.visible_top - before.visible_top).abs() < 0.001);
    assert!((after.scroll_ratio - before.scroll_ratio).abs() < 0.001);
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
        theme_generation: 0,
    };
    let image = Arc::new(RenderImage::new(SmallVec::from_elem(
        Frame::new(RgbaImage::new(1, 1)),
        1,
    )));
    let cached = CachedRaster {
        key,
        image,
        media: Arc::from([]),
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
    let after_scroll = host.stabilize_viewport(projected, 1_000.0, 600.0, 630.0, 300.0, density);
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
