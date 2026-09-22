//! Scroll anchoring helpers shared by prepaint and the frame commit.

use super::*;

pub(super) fn scroll_is_at_end(scroll_y: f32, viewport_height: f32, document_height: f32) -> bool {
    scroll_y + viewport_height + 0.5 >= document_height
}

pub(super) fn stabilized_scroll_y(
    was_at_end: bool,
    anchored: f32,
    viewport_height: f32,
    document_height: f32,
) -> f32 {
    let max_scroll = (document_height - viewport_height).max(0.0);
    if was_at_end {
        max_scroll
    } else {
        anchored.clamp(0.0, max_scroll)
    }
}

/// Publishes this frame's measurements: hit rows, shaped lines, scroll
/// anchoring and caret/search reveal.
#[cfg(feature = "benchmarks")]
type FrameAction = crate::editor::FrameBenchmarkAction;
#[cfg(not(feature = "benchmarks"))]
type FrameAction = ();

pub(super) fn publish_frame(
    host: &gpui::Entity<SemanticEditor>,
    bounds: Bounds<Pixels>,
    state: &mut PrepaintState,
    cx: &mut App,
) -> Option<FrameAction> {
    let hits = state
        .rows
        .iter()
        .map(|row| row.hit.clone())
        .collect::<std::sync::Arc<[HitRow]>>();
    let source_run_button_hits = state
        .source_run_buttons
        .iter()
        .map(|button| crate::editor::SourceRunButtonHit {
            bounds: button.interaction_bounds,
            source_offset: button.source_offset,
        })
        .collect::<std::sync::Arc<[_]>>();
    let image_resize_handles = state
        .image_resize_handles
        .iter()
        .map(|handle| crate::editor::InlineImageResizeHandle {
            bounds: handle.interaction_bounds,
            line: handle.line,
            line_start: handle.line_start,
            width: f32::from(handle.image_bounds.size.width),
        })
        .collect::<std::sync::Arc<[_]>>();
    let shaped = state
        .rows
        .iter()
        .map(|row| (row.shape_key.clone(), row.hit.layout.clone()))
        .collect::<Vec<_>>();
    #[cfg(feature = "benchmarks")]
    let elapsed = state.started_at.elapsed();
    let measured_rows = state
        .rows
        .iter()
        .map(|row| {
            // Parallel cell wraps are not contiguous slices of a source line.
            let wrap_starts = (1..if row.hit.table_layout.is_some() {
                1
            } else {
                row.visual_rows
            })
                .map(|visual_row| {
                    let display = row
                        .hit
                        .layout
                        .index_for_position(
                            point(px(0.0), px(visual_row as f32 * row.metrics.line_height)),
                            px(row.metrics.line_height),
                        )
                        .unwrap_or_else(|index| index);
                    row.hit.display.display_to_source(display)
                })
                .collect::<Vec<_>>();
            (row.hit.line.0, row.visual_rows, row.metrics, wrap_starts)
        })
        .collect::<Vec<_>>();

    host.update(cx, |editor, cx| {
        let viewport_changed = editor.viewport != Some(bounds);
        if viewport_changed {
            editor.dismiss_table_actions(cx);
        }
        editor.viewport = Some(bounds);
        editor.minimap.bounds =
            (editor.minimap.visible && editor.minimap.reveal >= 1.0).then_some(Bounds::new(
                point(bounds.right() - px(editor.minimap.width), bounds.top()),
                size(px(editor.minimap.width), bounds.size.height),
            ));
        editor.hit_rows = hits;
        editor.link_hits = std::sync::Arc::from(std::mem::take(&mut state.link_hits));
        editor.source_run_buttons = source_run_button_hits;
        editor.inline_image_handles = image_resize_handles;
        editor.source_copy.buttons = state
            .source_copy_buttons
            .iter()
            .map(|button| button.hit)
            .collect();
        if editor.display_map.soft_wrap() {
            editor.scroll_x = 0.0;
        } else {
            editor.scroll_x = editor.scroll_x.min(editor.max_horizontal_scroll());
        }
        if editor.shape_cache.len() + shaped.len() > 512 {
            editor.shape_cache.clear();
        }
        editor.shape_cache.extend(shaped);
        let viewport_height = f32::from(bounds.size.height);
        let was_at_end = editor.scroll_at_end
            || scroll_is_at_end(
                editor.scroll_y,
                viewport_height,
                editor.animated_document_height(),
            );
        let anchor_line = editor.animated_line_at_y(editor.scroll_y);
        let anchor_start = editor.animated_line_start_y(anchor_line);
        let anchor_fraction = ((editor.scroll_y - anchor_start)
            / editor.animated_line_height_px(anchor_line).max(1.0))
        .clamp(0.0, 1.0);
        let layout_changed =
            measured_rows
                .into_iter()
                .fold(false, |changed, (line, rows, metrics, wrap_starts)| {
                    let height_changed = editor.display_map.update_line_layout(
                        line,
                        rows,
                        metrics.line_height,
                        metrics.before,
                        metrics.after,
                    );
                    let wraps_changed = editor
                        .display_map
                        .update_line_wrap_starts(line, &wrap_starts);
                    height_changed || wraps_changed || changed
                });
        if layout_changed {
            editor.minimap.note_layout_changed();
        }
        let anchored = if layout_changed {
            editor.animated_line_start_y(anchor_line)
                + anchor_fraction * editor.animated_line_height_px(anchor_line)
        } else {
            editor.scroll_y
        };
        let settled_scroll_y = stabilized_scroll_y(
            was_at_end,
            anchored,
            viewport_height,
            editor.animated_document_height(),
        );
        let scroll_settled = (settled_scroll_y - editor.scroll_y).abs() > 0.5;
        editor.scroll_y = settled_scroll_y;
        editor.scroll_at_end = was_at_end;
        let snapshot = editor.snapshot(cx);
        let pending_reveal = editor.pending_reveal_caret;
        if pending_reveal {
            let caret_line = snapshot.line_index_at(editor.selection.head()).ok();
            let has_exact_row = caret_line.is_some_and(|caret_line| {
                editor.hit_rows.iter().any(|row| {
                    row.line == caret_line
                        && editor.selection.head() >= row.range.start
                        && editor.selection.head() <= row.range.end
                })
            });
            editor.pending_reveal_caret = !has_exact_row;
            editor.reveal_caret(&snapshot);
        }
        let search_revealed = editor.reveal_pending_search(&snapshot);
        let final_anchor_line = editor.animated_line_at_y(editor.scroll_y);
        editor.layout_anchor = snapshot
            .line_content_range(LineIndex(final_anchor_line))
            .ok()
            .map(|range| {
                crate::document::RevisionRange::new(
                    snapshot.revision(),
                    ByteRange::new(range.start.0, range.start.0),
                )
            });
        if viewport_changed || layout_changed || scroll_settled || pending_reveal || search_revealed
        {
            cx.notify();
        }
        #[cfg(feature = "benchmarks")]
        return Some(editor.record_frame_benchmark(elapsed, cx));
        #[cfg(not(feature = "benchmarks"))]
        None::<()>
    })
}
