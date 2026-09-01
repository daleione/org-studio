use super::*;
use super::{raster::*, viewport::*};
use crate::preview::display_map::*;
use crate::preview::projection::VisualRowId;
use crate::preview::table::test_table_projection;
use crate::{document::Revision, preview::layout::LayoutKey};
use unicode_segmentation::UnicodeSegmentation;

fn test_line_index(
    width: u16,
    rows_signature: u64,
    density: MinimapDensity,
    display_prefix: &[usize],
    pixel_prefix: &[f32],
) -> MinimapLineIndex {
    assert_eq!(display_prefix.len(), pixel_prefix.len());
    let measures = display_prefix
        .windows(2)
        .zip(pixel_prefix.windows(2))
        .map(|(display, pixels)| {
            ProjectionMeasure::new(display[1] - display[0], pixels[1] - pixels[0], true)
        })
        .collect();
    let projection = Arc::new(ProjectionSnapshot::new(measures));
    MinimapLineIndex {
        layout: LayoutKey {
            document_revision: Revision::INITIAL,
            content_width_px: width,
            text_metrics_revision: 0,
            fold_revision: 0,
        },
        width,
        rows_signature,
        density,
        total: projection.total_display_lines(),
        projection,
    }
}

#[test]
fn minimap_width_matches_render_constraints() {
    assert_eq!(width_for_viewport(100.0, None), 24.0);
    assert!((width_for_viewport(400.0, None) - 60.0).abs() < 0.001);
    assert!((width_for_viewport(2_000.0, None) - 175.2).abs() < 0.001);
    assert_eq!(width_for_viewport(4_000.0, None), 220.0);
    assert_eq!(width_for_viewport(800.0, Some(220)), 160.0);
    assert_eq!(width_for_viewport(2_000.0, Some(220)), 220.0);
    assert_eq!(manual_width_for_viewport(800.0, 300.0), 160.0);
    assert_eq!(manual_width_for_viewport(1_200.0, 480.0), 240.0);
    assert!((manual_width_for_viewport(1_500.0, 480.0) - 351.5625).abs() < 0.001);
    assert_eq!(manual_width_for_viewport(2_000.0, 600.0), 480.0);
    assert_eq!(manual_width_for_viewport(2_000.0, 20.0), 48.0);
    let resize = MinimapResizeSession {
        start_pointer_x: 1_800.0,
        start_width: 160.0,
    };
    assert_eq!(width_from_resize_drag(2_000.0, resize, 1_760.0), 200.0);
    assert_eq!(width_from_resize_drag(2_000.0, resize, 1_900.0), 60.0);
    assert_eq!(width_from_resize_drag(800.0, resize, 1_000.0), 160.0);
    assert_eq!(MinimapDensity::for_width(96.0), MinimapDensity::Compact);
    assert_eq!(
        MinimapDensity::for_width(160.0),
        MinimapDensity::Comfortable
    );
    assert_eq!(MinimapDensity::for_width(220.0), MinimapDensity::Large);
    assert_eq!(MinimapDensity::for_width(320.0), MinimapDensity::ExtraLarge);
    assert_eq!(MinimapDensity::for_width(440.0), MinimapDensity::Maximum);
    assert_eq!(MinimapDensity::Compact.font_px(), 2.0);
    assert_eq!(MinimapDensity::Comfortable.font_px(), 3.0);
    assert_eq!(MinimapDensity::Large.font_px(), 3.8);
    assert_eq!(MinimapDensity::ExtraLarge.font_px(), 4.1);
    assert_eq!(MinimapDensity::Maximum.font_px(), 4.25);
    assert!(
        MinimapDensity::Large.font_px() / MinimapDensity::Compact.font_px() > 1.8,
        "automatic large-screen density must provide a perceptible clarity gain"
    );
}

#[test]
fn table_minimap_geometry_is_a_projection_of_preview_columns() {
    let projection = test_table_projection();
    let resolved = projection.resolve();
    let table = crate::preview::table::project_table(projection.table(), 400.0, 100.0);
    assert_eq!(table.columns.len(), resolved.columns.len());
    for (minimap, preview) in table.columns.iter().zip(resolved.columns.iter()) {
        assert!((minimap.start_x - (4.0 + preview.start_x * 92.0 / 400.0)).abs() < 0.001);
        assert!((minimap.end_x - (4.0 + preview.end_x * 92.0 / 400.0)).abs() < 0.001);
        assert!(
            (minimap.content_start_x - (4.0 + preview.content_start_x * 92.0 / 400.0)).abs()
                < 0.001
        );
    }
    assert_eq!(table.separators.len(), resolved.separators.len());
    let source = "|apple|42|";
    assert!(
        projection
            .cells()
            .iter()
            .all(|cell| !cell.text(source).contains('|'))
    );
}

#[test]
fn taking_resize_session_releases_lock_before_commit_callback() {
    let state = Mutex::new(Some(MinimapResizeSession {
        start_pointer_x: 100.0,
        start_width: 160.0,
    }));
    let session = take_resize_session(&state).expect("active resize session");
    assert_eq!(session.start_width, 160.0);
    let mut guard = state
        .try_lock()
        .expect("commit callback must be able to re-enter interaction cleanup");
    *guard = None;
}

#[test]
fn projection_snapshot_publishes_exact_chunks_without_rebuilding_unchanged_leaves() {
    let estimates = (0..600)
        .map(|_| ProjectionMeasure::new(1, 24.0, false))
        .collect();
    let initial = ProjectionSnapshot::new(estimates);
    assert_eq!(initial.chunks.len(), 3);
    assert_eq!(initial.exact_rows, 0);
    assert_eq!(initial.total_display_lines(), 600);

    let first_chunk = initial.chunks[0].clone();
    let last_chunk = initial.chunks[2].clone();
    let updated = initial.replacing(&[
        (255, ProjectionMeasure::new(3, 72.0, true)),
        (256, ProjectionMeasure::new(2, 48.0, true)),
    ]);

    assert!(!Arc::ptr_eq(&first_chunk, &updated.chunks[0]));
    assert!(!Arc::ptr_eq(&initial.chunks[1], &updated.chunks[1]));
    assert!(Arc::ptr_eq(&last_chunk, &updated.chunks[2]));
    assert_eq!(updated.exact_rows, 2);
    assert_eq!(updated.total_display_lines(), 603);
    assert_eq!(updated.total_pixels(), 600.0 * 24.0 + 72.0);
    assert_eq!(updated.locate_display(255), (255, 0));
    assert_eq!(updated.locate_display(257), (255, 2));
    assert_eq!(updated.locate_display(258), (256, 0));
    assert_eq!(updated.prefix_for_row(257), (260, 6240.0));
}

#[test]
fn projection_snapshot_prefix_and_reverse_lookup_match_a_naive_model() {
    let measures = (0..10_000)
        .map(|row| ProjectionMeasure::new(row % 5 + 1, (row % 7 + 1) as f32 * 3.0, false))
        .collect::<Vec<_>>();
    let projection = ProjectionSnapshot::new(measures.clone());
    let mut display = 0usize;
    let mut pixels = 0.0f32;
    for (row, measure) in measures.iter().enumerate() {
        assert_eq!(projection.prefix_for_row(row), (display, pixels));
        assert_eq!(projection.locate_display(display), (row, 0));
        assert_eq!(projection.locate_pixel(pixels), (row, 0.0));
        display += measure.display_lines as usize;
        pixels += measure.pixels;
    }
    assert_eq!(projection.prefix_for_row(measures.len()), (display, pixels));
    assert_eq!(projection.total_display_lines(), display);
    assert_eq!(projection.total_pixels(), pixels);

    let fifty_mib_fixture_rows = ProjectionSnapshot::new(
        (0..341_392)
            .map(|_| ProjectionMeasure::new(1, 24.0, false))
            .collect(),
    );
    assert!(fifty_mib_fixture_rows.estimated_heap_bytes() <= 8 * 1024 * 1024);
}

#[test]
fn projection_range_replacement_preserves_prefixes_and_unaffected_chunks() {
    let measures = (0..1_024)
        .map(|row| ProjectionMeasure::new(row % 3 + 1, (row % 5 + 1) as f32, row % 2 == 0))
        .collect::<Vec<_>>();
    let initial = ProjectionSnapshot::new(measures.clone());
    let first = initial.chunks[0].clone();
    let last = initial.chunks[3].clone();
    let replacements = vec![
        ProjectionMeasure::new(7, 17.0, true),
        ProjectionMeasure::new(2, 9.0, false),
    ];
    let updated = initial.replacing_range(400..510, replacements.clone());

    let mut expected = measures;
    expected.splice(400..510, replacements);
    assert!(Arc::ptr_eq(&first, &updated.chunks[0]));
    assert!(Arc::ptr_eq(&last, updated.chunks.last().unwrap()));
    assert_eq!(updated.rows, expected.len());

    let mut display = 0usize;
    let mut pixels = 0.0f32;
    for (row, measure) in expected.iter().enumerate() {
        assert_eq!(updated.prefix_for_row(row), (display, pixels));
        assert_eq!(updated.locate_display(display), (row, 0));
        assert_eq!(updated.locate_pixel(pixels), (row, 0.0));
        display += measure.display_lines as usize;
        pixels += measure.pixels;
    }
    assert_eq!(updated.prefix_for_row(expected.len()), (display, pixels));
}

#[test]
fn projection_range_insertion_at_a_chunk_boundary_keeps_following_chunk_shared() {
    let initial = ProjectionSnapshot::new(
        (0..768)
            .map(|_| ProjectionMeasure::new(1, 24.0, true))
            .collect(),
    );
    let following = initial.chunks[2].clone();
    let updated = initial.replacing_range(256..256, vec![ProjectionMeasure::new(2, 48.0, true)]);
    assert!(Arc::ptr_eq(&following, &updated.chunks[3]));
    assert_eq!(updated.rows, 769);
    assert_eq!(updated.prefix_for_row(257), (258, 6_192.0));
}

#[test]
fn projection_scheduler_prioritizes_the_current_view_before_sequential_work() {
    let row_count = 2_000;
    let projection = Arc::new(ProjectionSnapshot::new(
        (0..row_count)
            .map(|_| ProjectionMeasure::new(1, 24.0, false))
            .collect(),
    ));
    let mut builder = MinimapLineIndexBuilder {
        key: MinimapLineIndexKey {
            presentation_rows: 1,
            width: 800,
            density: MinimapDensity::Compact,
            layout: LayoutKey {
                document_revision: Revision::INITIAL,
                content_width_px: 800,
                text_metrics_revision: 0,
                fold_revision: 0,
            },
        },
        presentation_rows: Arc::new((0..row_count).collect()),
        rows_signature: 1,
        sequential_cursor: 0,
        priority_range: 0..0,
        priority_cursor: 0,
        exact_bits: vec![0; row_count.div_ceil(64)],
        exact_rows: 0,
        started_at: Instant::now(),
        projection,
        pending_updates: Vec::new(),
        slices: 0,
        work: Duration::ZERO,
        max_slice: Duration::ZERO,
    };

    builder.prioritize(1_000);
    assert_eq!(builder.next_candidate(), Some(872));
    builder.mark_exact(872);
    assert_eq!(builder.next_candidate(), Some(873));

    builder.prioritize(1_600);
    assert_eq!(builder.next_candidate(), Some(1_472));
    for row in 1_472..1_856 {
        builder.mark_exact(row);
    }
    builder.priority_cursor = builder.priority_range.end;
    assert_eq!(builder.next_candidate(), Some(0));
}

#[test]
fn fold_projection_reuses_exact_minimap_measurements() {
    use crate::document::DocumentBuffer;
    use crate::preview::loading::derive_preview;

    let buffer =
        DocumentBuffer::from_utf8(b"* First\nfirst body\n* Second\nsecond body\n".to_vec())
            .unwrap();
    let document = derive_preview(
        std::path::PathBuf::from("fold-rebase.org"),
        buffer.snapshot(),
    );
    assert!(document.projection.rows.len() >= 3);
    let model = document.display_map.as_deref().unwrap();
    let previous_rows = Arc::new(vec![0, 2]);
    let previous_projection = ProjectionSnapshot::new(vec![
        ProjectionMeasure::new(3, 33.0, true),
        ProjectionMeasure::new(5, 55.0, true),
    ]);
    let expanded_rows = Arc::new(vec![0, 1, 2]);
    let key = MinimapLineIndexKey::new(
        &expanded_rows,
        800.0,
        MinimapDensity::Comfortable,
        document.revision,
        1,
    );

    let mut builder = MinimapLineIndexBuilder::rebased(
        model,
        key,
        expanded_rows,
        800.0,
        &previous_rows,
        &previous_projection,
    );

    assert_eq!(builder.exact_rows, 2);
    assert_eq!(
        builder.projection.measure(0),
        previous_projection.measure(0)
    );
    assert!(!builder.projection.measure(1).exact);
    assert_eq!(
        builder.projection.measure(2),
        previous_projection.measure(1)
    );
    assert_eq!(builder.next_candidate(), Some(1));
}

#[test]
fn larger_density_expands_the_projection_and_visible_thumb_span() {
    let mut index = test_line_index(100, 1, MinimapDensity::Compact, &[0, 100], &[0.0, 100.0]);
    assert_eq!(
        minimap_projection_height(100, 1_000.0, index.density),
        268.0
    );
    assert_eq!(
        minimap_thumb_height_for_scroll(&index, 0.0, 50.0, 1_000.0),
        130.0
    );
    index.density = MinimapDensity::Large;
    assert_eq!(
        minimap_projection_height(100, 1_000.0, index.density),
        472.0
    );
    assert_eq!(
        minimap_thumb_height_for_scroll(&index, 0.0, 50.0, 1_000.0),
        230.0
    );
}

#[test]
fn display_line_index_locates_wrapped_rows_without_changing_units() {
    let index = test_line_index(
        800,
        1,
        MinimapDensity::Compact,
        &[0, 1, 4, 6],
        &[0.0, 24.0, 96.0, 144.0],
    );
    assert_eq!(index.locate(0), (0, 0));
    assert_eq!(index.locate(1), (1, 0));
    assert_eq!(index.locate(3), (1, 2));
    assert_eq!(index.locate(4), (2, 0));
    assert_eq!(index.locate(5), (2, 1));
    assert_eq!(
        index.pixel_for_list_offset(index.list_offset_for_pixel(0.0)),
        0.0
    );
    assert_eq!(
        index.pixel_for_list_offset(index.list_offset_for_pixel(72.0)),
        72.0
    );
    assert_eq!(
        index.pixel_for_list_offset(index.list_offset_for_pixel(144.0)),
        144.0
    );
}

#[test]
fn minimap_content_hit_test_uses_rendered_row_heights() {
    let rows = [
        MinimapHitRow {
            top: 4.0,
            bottom: 6.6,
            presentation_index: 17,
            offset_in_item: 0.0,
        },
        MinimapHitRow {
            top: 6.6,
            bottom: 9.2,
            presentation_index: 18,
            offset_in_item: 0.0,
        },
        MinimapHitRow {
            top: 9.2,
            bottom: 11.8,
            presentation_index: 18,
            offset_in_item: 24.0,
        },
    ];

    let hit = hit_test_minimap_row(&rows, 4.0).expect("first row");
    assert_eq!(hit.presentation_index, 17);
    assert_eq!(hit.offset_in_item, 0.0);
    let wrapped_hit = hit_test_minimap_row(&rows, 10.0).expect("wrapped line");
    assert_eq!(wrapped_hit.presentation_index, 18);
    assert_eq!(wrapped_hit.offset_in_item, 24.0);
    assert_eq!(hit_test_minimap_row(&rows, 3.99), None);
    assert_eq!(hit_test_minimap_row(&rows, 11.8), None);
}

#[test]
fn display_window_expands_wraps_and_bottom_aligns() {
    let counts = [1, 3, 1, 2, 1];
    assert_eq!(
        display_window_range(counts.len(), 0, 0, 4, 0, |i| counts[i]),
        DisplayWindow {
            rows: 0..2,
            skip_display_lines: 0,
        }
    );
    assert_eq!(
        display_window_range(counts.len(), 4, 0, 4, 3, |i| counts[i]),
        DisplayWindow {
            rows: 2..5,
            skip_display_lines: 0,
        }
    );
    assert_eq!(
        display_window_range(counts.len(), 2, 0, 3, 1, |i| counts[i]),
        DisplayWindow {
            rows: 1..4,
            skip_display_lines: 2,
        }
    );
    assert_eq!(
        display_window_range(counts.len(), 1, 2, 3, 1, |i| counts[i]),
        DisplayWindow {
            rows: 1..3,
            skip_display_lines: 1,
        }
    );
    for (anchor, inner, offset) in [(0, 0, 0), (2, 0, 1), (1, 2, 1), (4, 0, 3)] {
        let window = display_window_range(counts.len(), anchor, inner, 4, offset, |i| counts[i]);
        let anchor_local_line = (counts[window.rows.start..anchor].iter().sum::<usize>() + inner)
            .saturating_sub(window.skip_display_lines);
        assert_eq!(anchor_local_line, offset);
    }
}

#[test]
fn display_run_slices_rebase_inline_and_syntax_ranges() {
    let runs = DisplayRuns {
        text: "0123456789".into(),
        inline_spans: Arc::from([InlineSpan {
            source: 2..8,
            range: 2..8,
            kind: InlineKind::Bold,
        }]),
        links: Arc::from([super::super::display_map::InlineLink {
            range: 2..8,
            destination: "target".into(),
        }]),
        code_spans: Arc::from([CodeHighlightSpan {
            start: 4,
            end: 9,
            kind: super::super::CodeHighlightKind::String,
        }]),
    };
    let sliced = slice_display_runs(&runs, 5..9);
    assert_eq!(sliced.text.as_ref(), "5678");
    assert_eq!(sliced.inline_spans[0].range, 0..3);
    assert_eq!(sliced.links[0].range, 0..3);
    assert_eq!(sliced.links[0].destination.as_ref(), "target");
    assert_eq!(
        (sliced.code_spans[0].start, sliced.code_spans[0].end),
        (0, 4)
    );
}

fn empty_runs(text: &'static str) -> DisplayRuns {
    DisplayRuns {
        text: text.into(),
        inline_spans: Arc::from([]),
        links: Arc::from([]),
        code_spans: Arc::from([]),
    }
}

#[test]
fn display_run_cache_is_strictly_bounded() {
    let mut cache = DisplayRunCache {
        entries: HashMap::with_capacity(DisplayRunCache::CAPACITY),
        order: VecDeque::with_capacity(DisplayRunCache::CAPACITY),
    };
    for row in 0..DisplayRunCache::CAPACITY + 17 {
        cache.insert((VisualRowId(row as u64), 0), empty_runs("row"));
    }
    assert_eq!(cache.entries.len(), DisplayRunCache::CAPACITY);
    assert_eq!(cache.order.len(), DisplayRunCache::CAPACITY);
    assert!(!cache.entries.contains_key(&(VisualRowId(0), 0)));
    assert!(
        cache
            .entries
            .contains_key(&(VisualRowId((DisplayRunCache::CAPACITY + 16) as u64), 0))
    );
}

#[test]
fn raster_tile_key_rejects_fold_and_resize_reuse() {
    let density = MinimapDensity::Compact;
    let original = tile_key(&[0, 1, 2, 3], 0, 96, 0, 10, 2.0, density);
    let folded = tile_key(&[0, 3], 0, 96, 1, 10, 2.0, density);
    let resized = tile_key(&[0, 1, 2, 3], 0, 72, 0, 11, 2.0, density);
    let rewrapped = tile_key(&[0, 1, 2, 3], 0, 96, 0, 12, 2.0, density);
    let next_tile = tile_key(&[128, 129], 128, 96, 0, 10, 2.0, density);
    let other_scale = tile_key(&[0, 1, 2, 3], 0, 96, 0, 10, 1.0, density);
    let comfortable = tile_key(
        &[0, 1, 2, 3],
        0,
        96,
        0,
        10,
        2.0,
        MinimapDensity::Comfortable,
    );
    assert_ne!(original, folded);
    assert_ne!(original, resized);
    assert_ne!(original, rewrapped);
    assert_ne!(original, next_tile);
    assert_ne!(original, other_scale);
    assert_ne!(original, comfortable);
}

#[test]
fn fold_refresh_keeps_the_previous_tile_until_its_replacement_is_ready() {
    let density = MinimapDensity::Compact;
    let previous = tile_key(&[10, 11, 12], 128, 96, 0, 1, 2.0, density);
    let folded = tile_key(&[10, 42], 128, 96, 1, 2, 2.0, density);
    let another_slot = tile_key(&[200, 201], 256, 96, 1, 2, 2.0, density);
    let image = Arc::new(RenderImage::new(SmallVec::from_elem(
        Frame::new(RgbaImage::new(1, 1)),
        1,
    )));
    let mut cache = RasterTileCache {
        entries: HashMap::new(),
        order: VecDeque::new(),
        in_flight: HashSet::new(),
    };
    cache.insert_batch(vec![(previous, image.clone())], &[previous]);

    let (fallback, is_missing) = cache.image_or_fallback(folded);
    assert!(is_missing, "the replacement tile still needs rasterizing");
    assert!(
        fallback.is_some_and(|fallback| Arc::ptr_eq(&fallback, &image)),
        "the last image in the same tile slot should remain paintable"
    );
    assert!(cache.image_or_fallback(another_slot).0.is_none());
}

#[test]
fn raster_tile_cache_and_in_flight_sets_are_bounded_by_design() {
    assert_eq!(RASTER_TILE_ROWS, 128);
    assert_eq!(RasterTileCache::CAPACITY, 6);
    let tile_bytes = 96.0 * (RASTER_TILE_ROWS as f32 * MINIMAP_LINE_HEIGHT_PX).ceil() * 4.0;
    assert!(tile_bytes * (RasterTileCache::CAPACITY as f32) < 1_000_000.0);
    let retina_maximum_bytes = MINIMAP_MANUAL_MAX_PX
        * (RASTER_TILE_ROWS as f32 * MinimapDensity::Maximum.line_height()).ceil()
        * 4.0
        * 4.0
        * RasterTileCache::CAPACITY as f32;
    assert!(retina_maximum_bytes < 30_000_000.0);

    let first = tile_key(&[0], 0, 96, 0, 1, 2.0, MinimapDensity::Compact);
    let second = tile_key(&[128], 128, 96, 0, 1, 2.0, MinimapDensity::Compact);
    let mut cache = RasterTileCache {
        entries: HashMap::new(),
        order: VecDeque::new(),
        in_flight: HashSet::new(),
    };
    assert!(cache.image_or_fallback(first).1);
    assert!(cache.image_or_fallback(second).1);
    assert!(cache.reserve(&[first, second]));
    assert_eq!(cache.in_flight.len(), 2);
    assert!(!cache.reserve(&[second]));
}

#[test]
fn complete_visible_batch_is_published_without_internal_eviction() {
    let density = MinimapDensity::Compact;
    let keys = (0..RasterTileCache::CAPACITY + 1)
        .map(|index| {
            let row = index * RASTER_TILE_ROWS;
            tile_key(&[row], row, 96, 0, 1, 2.0, density)
        })
        .collect::<Vec<_>>();
    let mut cache = RasterTileCache {
        entries: HashMap::new(),
        order: VecDeque::new(),
        in_flight: HashSet::new(),
    };
    assert!(cache.reserve(&keys));
    let completed = keys
        .iter()
        .copied()
        .map(|key| {
            let pixels = RgbaImage::new(1, 1);
            let image = Arc::new(RenderImage::new(SmallVec::from_elem(Frame::new(pixels), 1)));
            (key, image)
        })
        .collect();
    cache.insert_batch(completed, &keys);

    assert_eq!(cache.entries.len(), keys.len());
    assert!(cache.in_flight.is_empty());
    assert!(keys.iter().all(|key| cache.entries.contains_key(key)));
}

#[gpui::test]
fn real_list_state_uses_exact_variable_row_heights_and_resize(cx: &mut gpui::TestAppContext) {
    use gpui::{AppContext, Context, IntoElement, Render, Styled, Window, list, size};

    let cx = cx.add_empty_window();
    let heights = Arc::new([24.0_f32, 240.0, 48.0, 300.0, 24.0]);
    let state = ListState::new(heights.len(), gpui::ListAlignment::Top, px(0.0)).measure_all();

    struct VariableRows {
        state: ListState,
        heights: Arc<[f32; 5]>,
    }
    impl Render for VariableRows {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let heights = self.heights.clone();
            list(self.state.clone(), move |index, _, _| {
                div().h(px(heights[index])).w_full().into_any()
            })
            .size_full()
        }
    }

    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(100.0), px(100.0)),
        |_, cx| {
            cx.new(|_| VariableRows {
                state: state.clone(),
                heights: heights.clone(),
            })
            .into_any_element()
        },
    );

    let index = test_line_index(
        100,
        1,
        MinimapDensity::Compact,
        &[0, 10, 120, 140, 280, 300],
        &[0.0, 24.0, 264.0, 312.0, 612.0, 636.0],
    );
    state.scroll_to(index.list_offset_for_pixel(200.0));
    let wheel_down = scroll_ratio_after_wheel(&index, &state, -40.0);
    let wheel_up = scroll_ratio_after_wheel(&index, &state, 40.0);
    assert!((wheel_down - 240.0 / 536.0).abs() < 0.001);
    assert!((wheel_up - 160.0 / 536.0).abs() < 0.001);
    state.scroll_to(index.list_offset_for_pixel(0.0));
    assert_eq!(scroll_ratio_after_wheel(&index, &state, 200.0), 0.0);
    state.scroll_to(index.list_offset_for_pixel(536.0));
    assert_eq!(scroll_ratio_after_wheel(&index, &state, -200.0), 1.0);

    scroll_list_to_ratio(&index, &state, 1.0);
    let bottom = minimap_viewport_for_list(&index, &state, 500.0);
    assert_eq!(bottom.scroll_ratio, 1.0);
    assert!((bottom.thumb.top + bottom.thumb.height - 500.0).abs() < 0.001);

    state.scroll_to(ListOffset::default());
    let before_content_click = minimap_viewport_for_list(&index, &state, 500.0);
    let content_target =
        minimap_click_target_for_viewport(&index, &state, before_content_click, 250.0);
    let clicked_offset = index.list_offset_for_display_position(content_target.clicked_display);
    let clicked_pixel = index.pixel_for_list_offset(clicked_offset);
    state.scroll_to(content_target.offset);
    let anchor = MinimapInteractionAnchor {
        layout: index.layout,
        width: index.width,
        rows_signature: index.rows_signature,
        interaction_height: before_content_click.interaction_height,
        content_top: before_content_click.content_top,
    };
    let after_content_click =
        minimap_viewport_for_list_with_anchor(&index, &state, 500.0, Some(anchor));
    assert_eq!(
        after_content_click.content_top, before_content_click.content_top,
        "the content under a small-window click must not be replaced"
    );
    assert!(
        (after_content_click.thumb.top + after_content_click.thumb.height * 0.5 - 250.0).abs()
            < 0.001
    );
    assert!(
        (index.pixel_for_list_offset(content_target.offset) + 50.0 - clicked_pixel).abs() < 0.001,
        "the clicked minimap content must land at the left viewport center"
    );

    let clicked_thumb_center =
        after_content_click.thumb.top + after_content_click.thumb.height * 0.5;
    let wheel_target = scroll_ratio_after_wheel(&index, &state, -8.0);
    scroll_list_to_ratio(&index, &state, wheel_target);
    let after_first_wheel =
        minimap_viewport_for_list_with_anchor(&index, &state, 500.0, Some(anchor));
    let wheel_thumb_center = after_first_wheel.thumb.top + after_first_wheel.thumb.height * 0.5;
    assert!(
        (wheel_thumb_center - clicked_thumb_center).abs() < 20.0,
        "the first wheel event after a click must move continuously, not drop the minimap camera"
    );

    scroll_list_to_ratio(&index, &state, 0.5);
    let variable_height_viewport = minimap_viewport_for_list(&index, &state, 500.0);
    let desired_thumb_top = 180.0;
    assert!(
        (variable_height_viewport.thumb.top - desired_thumb_top).abs() > 1.0,
        "the fixture must exercise the non-linear variable-height mapping"
    );
    let drag = MinimapDragSession {
        start_pointer_y: desired_thumb_top,
        start_thumb_top: desired_thumb_top,
        start_ratio: variable_height_viewport.scroll_ratio,
        current_thumb_top: desired_thumb_top,
    };
    assert_eq!(
        minimap_thumb_for_drag(variable_height_viewport, Some(drag)).top,
        desired_thumb_top,
        "the active transparent viewport must paint at the pointer-derived position"
    );
    let aligned_anchor = minimap_anchor_for_thumb_top(&index, &state, 500.0, desired_thumb_top);
    let released_viewport =
        minimap_viewport_for_list_with_anchor(&index, &state, 500.0, Some(aligned_anchor));
    assert!(
        (released_viewport.thumb.top - desired_thumb_top).abs() < 0.001,
        "releasing after a variable-height drag must not jump the viewport"
    );

    let short_projection = test_line_index(
        100,
        2,
        MinimapDensity::Compact,
        &[0, 1, 3, 4, 8, 9],
        &[0.0, 24.0, 264.0, 312.0, 612.0, 636.0],
    );
    scroll_list_to_ratio(&short_projection, &state, 1.0);
    let short_bottom = minimap_viewport_for_list(&short_projection, &state, 500.0);
    let projected_height = 9.0 * MINIMAP_LINE_HEIGHT_PX + MINIMAP_EDGE_PADDING_PX * 2.0;
    assert_eq!(short_bottom.scroll_ratio, 1.0);
    assert!(
        (short_bottom.thumb.top + short_bottom.thumb.height - projected_height).abs() < 0.001,
        "a fullscreen thumb must stop at the document projection, not in track whitespace"
    );

    let max = f32::from(state.max_offset_for_scrollbar().y);
    assert_eq!(max, 536.0);
    seek_to_ratio(&state, 1.0, false);
    assert_eq!(-f32::from(state.scroll_px_offset_for_scrollbar().y), max);
    let compact = thumb_geometry(&state, heights.len(), 500.0);

    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(100.0), px(200.0)),
        |_, cx| {
            cx.new(|_| VariableRows {
                state: state.clone(),
                heights: heights.clone(),
            })
            .into_any_element()
        },
    );
    let tall = thumb_geometry(&state, heights.len(), 500.0);
    assert_eq!(tall.height, compact.height);
    assert_eq!(tall.height, MIN_THUMB_PX);

    // Model folding a heading subtree by replacing three presentation
    // rows (including the 300px image-like row) with one placeholder.
    state.splice(1..4, 1);
    state.clone().measure_all();
    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(100.0), px(100.0)),
        |_, cx| {
            cx.new(|_| VariableRows {
                state: state.clone(),
                heights: heights.clone(),
            })
            .into_any_element()
        },
    );
    assert_eq!(state.item_count(), 3);
    seek_to_ratio(&state, 1.0, false);
    let folded_max = f32::from(state.max_offset_for_scrollbar().y);
    assert_eq!(
        -f32::from(state.scroll_px_offset_for_scrollbar().y),
        folded_max
    );
}

#[test]
fn sampled_text_is_bounded_and_uses_zed_scale() {
    assert_eq!(MINIMAP_FONT_PX, 2.0);
}

#[test]
fn pixel_scroll_metrics_drive_content_and_thumb_together() {
    let metrics = ScrollMetrics {
        offset: 4_500.0,
        max_offset: 9_000.0,
        viewport: 1_000.0,
    };
    let layout = minimap_layout_from_metrics(1_000, 200, 500.0, metrics);
    assert_eq!(layout.first_line, 400);
    assert!((layout.thumb.height - 108.333_336).abs() < 0.001);
    assert!((layout.thumb.top - 195.833_33).abs() < 0.001);
}

#[test]
fn thumb_height_does_not_change_when_virtual_height_converges() {
    // 400 rows deliberately stays below the old 4096-row branch. The
    // viewport extent must not depend on ListState's progressively refined
    // max offset for ordinary documents either.
    let before_measurement = thumb_geometry_for_document(
        ScrollMetrics {
            offset: 1_000.0,
            max_offset: 4_000.0,
            viewport: 1_000.0,
        },
        400,
        500.0,
    );
    let after_measurement = thumb_geometry_for_document(
        ScrollMetrics {
            offset: 2_000.0,
            max_offset: 8_000.0,
            viewport: 1_000.0,
        },
        400,
        500.0,
    );
    assert_eq!(before_measurement, after_measurement);
}

#[test]
fn fullscreen_uses_one_coordinate_space_without_a_blank_prefix() {
    let before_list_layout = minimap_viewport(
        ScrollMetrics {
            offset: 4_000.0,
            max_offset: 8_000.0,
            viewport: 800.0,
        },
        400,
        1_400.0,
    );
    let after_list_layout = minimap_viewport(
        ScrollMetrics {
            offset: 3_800.0,
            max_offset: 7_600.0,
            viewport: 1_200.0,
        },
        400,
        1_400.0,
    );

    // The whole 400-line projection fits in the 538-line minimap. Resize
    // cannot scroll its content into blank space, before or after ListState
    // commits the new viewport measurement.
    assert_eq!(before_list_layout.content_top, 0.0);
    assert_eq!(after_list_layout.content_top, 0.0);
    assert!(before_list_layout.thumb.top >= 0.0);
    assert!(after_list_layout.thumb.top >= 0.0);
}

#[test]
fn upward_scroll_keeps_content_window_inside_document() {
    let viewport = 800.0;
    let track = 520.0;
    let total = 1_000;
    let mut previous_top = f32::MAX;
    for step in (0..=20).rev() {
        let progress = step as f32 / 20.0;
        let layout = minimap_viewport(
            ScrollMetrics {
                offset: progress * 9_000.0,
                max_offset: 9_000.0,
                viewport,
            },
            total,
            track,
        );
        let capacity = track / MINIMAP_LINE_HEIGHT_PX;
        assert!(layout.content_top >= 0.0);
        assert!(layout.content_top <= total as f32 - capacity + f32::EPSILON);
        assert!(layout.content_top <= previous_top);
        assert!(layout.thumb.top >= 0.0);
        assert!(layout.thumb.top + layout.thumb.height <= track + f32::EPSILON);
        previous_top = layout.content_top;
    }
    assert_eq!(previous_top, 0.0);
}

#[test]
fn bottom_alignment_uses_total_display_lines_not_source_rows() {
    // 400 source rows expand to 900 wrapped display lines. At the bottom,
    // the minimap window must end at display line 900 exactly.
    let track = 520.0;
    let capacity = track / MINIMAP_LINE_HEIGHT_PX;
    let layout = minimap_viewport(
        ScrollMetrics {
            offset: 9_000.0,
            max_offset: 9_000.0,
            viewport: 800.0,
        },
        900,
        track,
    );
    assert!((layout.content_top + capacity - 900.0).abs() < 0.001);
    assert!((layout.thumb.top + layout.thumb.height - track).abs() < 0.001);
}

#[test]
fn thumb_visual_state_has_clear_hover_and_active_steps() {
    let idle = thumb_alphas(false, false, false);
    let hovered = thumb_alphas(false, true, false);
    let active = thumb_alphas(true, true, false);
    assert!(idle.0 < hovered.0 && hovered.0 < active.0);
    assert!(idle.1 < hovered.1 && hovered.1 < active.1);
    assert_eq!(thumb_alphas(false, false, true), (0, 0));
}

#[test]
fn thumb_mapping_reaches_both_ends_for_variable_height_content() {
    let start = thumb_geometry_from_metrics(0.0, 9000.0, 1000.0, 500.0);
    let middle = thumb_geometry_from_metrics(4500.0, 9000.0, 1000.0, 500.0);
    let end = thumb_geometry_from_metrics(9000.0, 9000.0, 1000.0, 500.0);
    assert_eq!(
        start,
        ThumbGeometry {
            top: 0.0,
            height: 50.0
        }
    );
    assert_eq!(
        middle,
        ThumbGeometry {
            top: 225.0,
            height: 50.0
        }
    );
    assert_eq!(
        end,
        ThumbGeometry {
            top: 450.0,
            height: 50.0
        }
    );
}

#[test]
fn minimum_thumb_height_keeps_the_end_reachable() {
    let end = thumb_geometry_from_metrics(99_900.0, 99_900.0, 100.0, 500.0);
    assert_eq!(end.height, MIN_THUMB_PX);
    assert_eq!(end.top + end.height, 500.0);
}

#[test]
fn zed_layout_uses_one_coordinate_for_content_and_thumb() {
    let top = minimap_layout_from_range(1000, 0, 100, 200, 520.0);
    let middle = minimap_layout_from_range(1000, 450, 550, 200, 520.0);
    let end = minimap_layout_from_range(1000, 900, 1000, 200, 520.0);

    assert_eq!(top.first_line, 0);
    assert_eq!(
        top.thumb,
        ThumbGeometry {
            top: 4.0,
            height: 260.0
        }
    );
    assert_eq!(middle.first_line, 400);
    assert_eq!(
        middle.thumb,
        ThumbGeometry {
            top: 134.0,
            height: 260.0
        }
    );
    assert_eq!(end.first_line, 800);
    assert_eq!(end.thumb.top + end.thumb.height, 520.0);
}

#[test]
fn short_document_does_not_fake_scroll_the_minimap_content() {
    let layout = minimap_layout_from_range(100, 70, 100, 200, 520.0);
    assert_eq!(layout.first_line, 0);
    assert_eq!(
        layout.thumb,
        ThumbGeometry {
            top: 186.0,
            height: 78.0
        }
    );
}

#[test]
fn empty_single_line_and_tiny_track_degrade_safely() {
    assert_eq!(
        minimap_layout_from_range(0, 0, 0, 1, 2.0),
        MinimapLayout::default()
    );
    let single = minimap_layout_from_range(1, 0, 1, 1, 2.0);
    assert_eq!(single.first_line, 0);
    assert_eq!(
        single.thumb,
        ThumbGeometry {
            top: 0.0,
            height: 2.0
        }
    );
}

#[test]
fn pointer_ratio_is_relative_to_minimap_not_window() {
    let bounds = Bounds::new(point(px(1500.0), px(80.0)), gpui::size(px(92.0), px(600.0)));
    assert_eq!(local_y_ratio(px(80.0), bounds), 0.0);
    assert_eq!(local_y_ratio(px(380.0), bounds), 0.5);
    assert_eq!(local_y_ratio(px(680.0), bounds), 1.0);
    assert_eq!(local_y_ratio(px(40.0), bounds), 0.0);
    assert_eq!(local_y_ratio(px(900.0), bounds), 1.0);
}

#[test]
fn dragging_is_the_exact_inverse_of_thumb_travel() {
    let session = MinimapDragSession {
        start_pointer_y: 360.0,
        start_thumb_top: 320.0,
        start_ratio: 0.5,
        current_thumb_top: 320.0,
    };
    assert_eq!(
        minimap_drag_target(360.0, session, 80.0, 720.0),
        (0.5, 320.0)
    );
    assert_eq!(
        minimap_drag_target(-100.0, session, 80.0, 720.0),
        (0.0, 0.0)
    );
    assert_eq!(
        minimap_drag_target(900.0, session, 80.0, 720.0),
        (1.0, 640.0)
    );
}

#[test]
fn absolute_drag_keeps_the_grab_point_and_clamps_outside_the_track() {
    let thumb_height = 80.0;
    let track_height = 320.0;
    let session = MinimapDragSession {
        start_pointer_y: 140.0,
        start_thumb_top: 120.0,
        start_ratio: 0.5,
        current_thumb_top: 120.0,
    };
    assert_eq!(
        minimap_drag_target(140.0, session, thumb_height, track_height),
        (0.5, 120.0)
    );
    assert_eq!(
        minimap_drag_target(-40.0, session, thumb_height, track_height),
        (0.0, 0.0)
    );
    assert_eq!(
        minimap_drag_target(400.0, session, thumb_height, track_height),
        (1.0, 240.0)
    );
}

#[test]
fn tiny_move_after_content_click_is_continuous_from_the_clicked_ratio() {
    // In a small viewport the anchored thumb position is intentionally not
    // `ratio * travel`. The first drag event must start at the clicked ratio,
    // rather than jumping to the absolute thumb/travel ratio.
    let session = MinimapDragSession {
        start_pointer_y: 120.0,
        start_thumb_top: 80.0,
        start_ratio: 0.72,
        current_thumb_top: 80.0,
    };
    let unchanged = minimap_drag_target(120.0, session, 60.0, 240.0);
    assert_eq!(unchanged, (0.72, 80.0));
    let moved = minimap_drag_target(121.0, session, 60.0, 240.0);
    assert!(moved.0 > 0.72 && moved.0 < 0.73);
    assert_eq!(moved.1, 81.0);
}

#[gpui::test]
fn window_drag_listener_survives_leaving_the_minimap_hitbox(cx: &mut gpui::TestAppContext) {
    use gpui::{AppContext, Context, Modifiers, Render, Window, size};

    let cx = cx.add_empty_window();
    let active = Arc::new(AtomicBool::new(false));
    let move_positions = Arc::new(Mutex::new(Vec::<f32>::new()));

    struct DragHarness {
        active: Arc<AtomicBool>,
        move_positions: Arc<Mutex<Vec<f32>>>,
    }

    impl Render for DragHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let down_active = self.active.clone();
            let event_active = self.active.clone();
            let event_positions = self.move_positions.clone();
            div()
                .size_full()
                .on_mouse_down(MouseButton::Left, move |_, _, _| {
                    down_active.store(true, Ordering::Release);
                })
                .child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, _| {
                            let move_active = event_active.clone();
                            let move_positions = event_positions.clone();
                            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, _| {
                                if phase == DispatchPhase::Bubble
                                    && event.dragging()
                                    && move_active.load(Ordering::Acquire)
                                {
                                    move_positions
                                        .lock()
                                        .expect("drag test positions poisoned")
                                        .push(f32::from(event.position.x));
                                }
                            });
                        },
                    )
                    .size_full(),
                )
        }
    }

    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(100.0), px(100.0)),
        |_, cx| {
            cx.new(|_| DragHarness {
                active: active.clone(),
                move_positions: move_positions.clone(),
            })
            .into_any_element()
        },
    );
    cx.simulate_mouse_down(
        point(px(50.0), px(50.0)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_move(
        point(px(180.0), px(140.0)),
        MouseButton::Left,
        Modifiers::default(),
    );
    assert_eq!(
        move_positions
            .lock()
            .expect("drag test positions poisoned")
            .as_slice(),
        [180.0]
    );
}

#[test]
fn thumb_height_uses_the_exact_visible_display_span() {
    let index = test_line_index(
        100,
        1,
        MinimapDensity::Compact,
        &[0, 100, 101, 201],
        &[0.0, 100.0, 400.0, 500.0],
    );

    let dense_text = minimap_thumb_height_for_scroll(&index, 0.0, 100.0, 500.0);
    let tall_block = minimap_thumb_height_for_scroll(&index, 150.0, 100.0, 500.0);
    let final_text = minimap_thumb_height_for_scroll(&index, 400.0, 100.0, 500.0);

    assert_eq!(dense_text, 260.0);
    assert_eq!(tall_block, MIN_THUMB_PX);
    assert_eq!(final_text, 260.0);
    let (top, bottom) = minimap_visible_display_range(&index, 0.0, 100.0);
    assert_eq!((top, bottom), (0.0, 100.0));
}

#[test]
fn inline_display_runs_cover_the_same_rendered_text() {
    let parsed = parse_document_inline(
        DocumentFormat::Markdown,
        "plain **bold** *italic* and [link](target)",
    );
    let line = DisplayRuns {
        text: parsed.text.into(),
        inline_spans: parsed.spans.into(),
        links: Arc::from([]),
        code_spans: Arc::from([]),
    };
    let mut base_font = font("Menlo");
    base_font.weight = FontWeight::BLACK;
    let runs = minimap_text_runs(PreviewLineKind::Text, &line, base_font);
    assert_eq!(
        runs.iter().map(|run| run.len).sum::<usize>(),
        line.text.len()
    );
    assert!(runs.len() > 1);
    assert!(
        runs.iter()
            .any(|run| run.font.style == gpui::FontStyle::Italic)
    );
}

#[test]
fn minimap_line_limit_preserves_utf8_boundaries() {
    let source = "中".repeat(1100);
    let truncated = truncate_for_minimap(&source);
    assert_eq!(truncated.graphemes(true).count(), 1024);
    assert!(truncated.is_char_boundary(truncated.len()));
}

#[test]
fn minimap_font_has_black_weight_and_cjk_fallback() {
    let font = minimap_font();
    assert_eq!(font.weight, FontWeight::BLACK);
    let fallbacks = font.fallbacks.expect("fallbacks");
    assert!(
        fallbacks
            .fallback_list()
            .iter()
            .any(|name| name == "PingFang SC")
    );
    assert!(
        fallbacks
            .fallback_list()
            .iter()
            .any(|name| name == "Apple Color Emoji")
    );
}

#[test]
fn viewport_border_exceeds_three_to_one_contrast() {
    let theme = current_theme();
    let border = composite_rgb(theme.foreground, theme.background_alt, 0xb0 as f32 / 255.0);
    assert!(contrast_ratio(border, theme.background_alt) >= 3.0);
}

#[test]
fn truncation_preserves_combining_clusters_and_rtl_text() {
    let combining = format!("{}tail", "e\u{301}".repeat(1024));
    let truncated = truncate_for_minimap(&combining);
    assert!(truncated.ends_with('\u{301}'));
    assert_eq!(truncated.graphemes(true).count(), 1024);

    let rtl = "مرحبا بالعالم";
    assert_eq!(truncate_for_minimap(rtl), rtl);
}

#[test]
fn incremental_document_patch_reuses_unaffected_minimap_layout_chunks() {
    use crate::document::{ByteRange, DocumentBuffer, EditTransaction, TextEdit, TextSnapshot};
    use crate::preview::{
        DerivedUpdate,
        loading::{derive_preview, derive_preview_incremental},
    };

    let source = (0..400)
        .map(|index| format!("* Heading {index}\nbody {index}\n"))
        .collect::<String>();
    let mut buffer = DocumentBuffer::from_utf8(source.into_bytes()).unwrap();
    let before = buffer.snapshot();
    let previous = derive_preview(std::path::PathBuf::from("minimap.org"), before.clone());
    let caret = before
        .copy_range(ByteRange::new(0, before.len_bytes()))
        .find("body 200")
        .unwrap() as u64
        + 8;
    let delta = buffer
        .commit(EditTransaction::new(
            before.revision(),
            vec![TextEdit::new(ByteRange::new(caret, caret), "x")],
        ))
        .unwrap();
    let next = derive_preview_incremental(
        std::path::PathBuf::from("minimap.org"),
        buffer.snapshot(),
        Some(&previous),
        &[delta],
    );
    let DerivedUpdate::Incremental { patch, .. } = &next.update else {
        panic!("test edit should produce a visual patch");
    };
    let presentation = Arc::new((0..previous.projection.rows.len()).collect::<Vec<_>>());
    let model = previous.display_map.as_deref().unwrap();
    let key = MinimapLineIndexKey::new(
        &presentation,
        800.0,
        MinimapDensity::Comfortable,
        previous.revision,
        0,
    );
    let index = model.estimated_minimap_line_index(
        &presentation,
        key.width,
        1,
        800.0,
        MinimapDensity::Comfortable,
        key.layout,
    );
    let old_projection = index.projection.clone();
    let state = MinimapState::new();
    *state.line_index.lock().unwrap() = Some(CachedMinimapLineIndex {
        presentation_rows: presentation.clone(),
        index,
    });
    state.apply_document_patch(&next, patch, &presentation);
    let cached = state.line_index.lock().unwrap();
    let patched = cached.as_ref().unwrap();
    assert_eq!(patched.index.layout.document_revision, next.revision);
    assert!(
        old_projection
            .chunks
            .iter()
            .zip(patched.index.projection.chunks.iter())
            .filter(|(old, new)| Arc::ptr_eq(old, new))
            .count()
            > 0
    );

    drop(cached);
    let folded_presentation = Arc::new(
        (0..previous.projection.rows.len())
            .filter(|row| row % 3 != 0 || patch.old_visual.contains(row))
            .collect::<Vec<_>>(),
    );
    assert!(folded_presentation.len() < presentation.len());
    let folded_key = MinimapLineIndexKey::new(
        &folded_presentation,
        800.0,
        MinimapDensity::Comfortable,
        previous.revision,
        1,
    );
    let folded_index = model.estimated_minimap_line_index(
        &folded_presentation,
        folded_key.width,
        2,
        800.0,
        MinimapDensity::Comfortable,
        folded_key.layout,
    );
    *state.line_index.lock().unwrap() = Some(CachedMinimapLineIndex {
        presentation_rows: folded_presentation.clone(),
        index: folded_index,
    });
    state.apply_document_patch(&next, patch, &folded_presentation);
    let cached = state.line_index.lock().unwrap();
    let patched = cached
        .as_ref()
        .expect("unchanged folds should support a local minimap patch");
    assert_eq!(patched.index.layout.document_revision, next.revision);
    assert_eq!(patched.index.projection.rows, folded_presentation.len());
}
#[cfg(test)]
fn composite_rgb(foreground: u32, background: u32, alpha: f32) -> u32 {
    let channel = |shift: u32| {
        let foreground = ((foreground >> shift) & 0xffu32) as f32;
        let background = ((background >> shift) & 0xffu32) as f32;
        (foreground * alpha + background * (1.0 - alpha)).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

#[cfg(test)]
fn contrast_ratio(first: u32, second: u32) -> f32 {
    let luminance = |color: u32| {
        let linear = |value: u32| {
            let value = value as f32 / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear((color >> 16) & 0xff)
            + 0.7152 * linear((color >> 8) & 0xff)
            + 0.0722 * linear(color & 0xff)
    };
    let first = luminance(first);
    let second = luminance(second);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}
