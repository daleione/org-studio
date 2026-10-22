//! Preview a column drag, then commit its padding and rules as one document edit.
#[cfg(test)]
mod tests;

use super::*;
use crate::document::{DocumentFormat, table};

const HIT_SLOP: f32 = 4.;
const DRAG_THRESHOLD: f32 = 3.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ResizeTarget {
    anchor: u64,
    column: usize,
}

struct ResizeHit {
    target: ResizeTarget,
    widths: Vec<f32>,
    metrics: Arc<TableColumnMetrics>,
    source_widths: Vec<usize>,
    min_width: f32,
    max_width: f32,
}

struct ResizeDrag {
    start: ResizeHit,
    start_x: f32,
    widths_em: Vec<f32>,
    moved: bool,
}

impl ResizeDrag {
    fn update(&mut self, pointer_x: f32) -> bool {
        let delta = pointer_x - self.start_x;
        if !self.moved && delta.abs() < DRAG_THRESHOLD {
            return false;
        }
        let column = self.start.target.column;
        let width =
            (self.start.widths[column] + delta).clamp(self.start.min_width, self.start.max_width);
        let width_em = width / f32::from(self.start.metrics.font_size);
        if (width_em - self.widths_em[column]).abs() < 0.01 {
            return false;
        }
        self.widths_em[column] = width_em;
        self.moved = true;
        true
    }
}

#[derive(Default)]
pub(super) struct TableResize {
    hover: Option<ResizeTarget>,
    drag: Option<ResizeDrag>,
    pub(super) revision: u64,
}

impl SemanticEditor {
    pub(super) fn preferred_table_widths(&self, anchor: u64) -> Option<&[f32]> {
        self.table_resize
            .drag
            .as_ref()
            .filter(|drag| drag.moved && drag.start.target.anchor == anchor)
            .map(|drag| drag.widths_em.as_slice())
    }

    pub(super) fn table_resize_is_dragging(&self) -> bool {
        self.table_resize.drag.is_some()
    }

    pub(super) fn table_resize_is_hovered(&self) -> bool {
        self.table_resize.hover.is_some() || self.table_resize_is_dragging()
    }

    fn table_resize_hit(&self, position: Point<Pixels>, cx: &App) -> Option<ResizeHit> {
        if self.is_read_only(cx)
            || self.is_selecting
            || self.table_menu_is_open()
            || self.image_resize.is_some()
            || self.minimap.resizing.is_some()
            || self.minimap.drag.is_some()
            || !self.viewport?.contains(&position)
            || self
                .minimap
                .bounds
                .is_some_and(|bounds| bounds.contains(&position))
        {
            return None;
        }
        let row = self
            .hit_rows
            .iter()
            .find(|row| position.y >= row.visible_top && position.y < row.visible_bottom)?;
        let snapshot = self.snapshot(cx);
        let path = self.session.read(cx).syntax_path();
        let format = DocumentFormat::from_path(path);
        let text = snapshot.copy_range(snapshot.line_content_range(row.line).ok()?);
        if !table::is_table_row(&text, format) {
            return None;
        }
        let query = syntax::SparseEditorStyleSnapshot::query_lines(
            path,
            &snapshot,
            &[row.line.0],
            &self.syntax_service,
        );
        if query.snapshot.line(row.line.0)?.id != syntax::EditorStyleId::Table {
            return None;
        }
        let positions = delimiter_positions(row, &snapshot, format);
        let column = (1..positions.len()).find(|&index| {
            let (x, width) = positions[index];
            let distance = (f32::from(position.x - row.text_origin_x - x - width / 2.)).abs();
            // Keep the resize band clear of the menu button in the cell padding.
            distance <= HIT_SLOP.min(f32::from(width) / 2. + 0.5)
        })? - 1;
        let columns = self.table_columns(&snapshot, row.line, format)?;
        let anchor = columns.source_range.start.0;
        let text_system = gpui::WindowTextSystem::new(cx.text_system().clone());
        let metrics = row.table_layout.as_ref().map_or_else(
            || {
                self.table_column_metrics(
                    &snapshot,
                    &columns,
                    crate::typography::table_font(),
                    px(self.font_size_px()),
                    &text_system,
                )
            },
            |layout| layout.metrics.clone(),
        );
        let widths = row.table_layout.as_ref().map_or_else(
            || {
                positions
                    .windows(2)
                    .map(|pair| f32::from(pair[1].0 - pair[0].0 - pair[0].1))
                    .collect::<Vec<_>>()
            },
            |layout| {
                layout
                    .column_widths
                    .iter()
                    .copied()
                    .map(f32::from)
                    .collect()
            },
        );
        if widths.len() != columns.widths.len() {
            return None;
        }
        let min_width = if self.display_map.soft_wrap() {
            let requested = columns.minimum_widths[column]
                .max(columns.widths[column].min(org_commands::MIN_RESIZED_COLUMN_WIDTH));
            f32::from(metrics.column_width(column, requested, columns.widths[column]))
        } else {
            f32::from(metrics.natural_widths[column])
        };
        let min_width = min_width.min(widths[column]);
        // Later columns retain their widths and move with this boundary.
        let max_width = if self.display_map.soft_wrap() {
            let &(x, pipe_width) = positions.last()?;
            let limit =
                (widths[column] + self.display_map.wrap_width() - f32::from(x + pipe_width) - 2.)
                    .max(widths[column]);
            let natural = columns.widths[column];
            let mut requested = metrics.source_width(column, px(limit), natural);
            // Source padding uses whole spaces; rounding up would squeeze other columns.
            if metrics.column_width(column, requested, natural) > px(limit) {
                requested = requested.saturating_sub(1);
            }
            f32::from(metrics.column_width(column, requested, natural)).max(widths[column])
        } else {
            f32::MAX
        };
        Some(ResizeHit {
            target: ResizeTarget { anchor, column },
            widths,
            metrics,
            source_widths: columns.widths.clone(),
            min_width,
            max_width,
        })
    }

    pub(super) fn table_resize_down(
        &mut self,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(hit) = self.table_resize_hit(event.position, cx) else {
            return false;
        };
        self.dismiss_table_actions(cx);
        self.dismiss_inline(cx);
        self.dismiss_todo(cx);
        self.dismiss_timestamp(cx);
        self.hovered_link = None;
        self.table_resize.hover = Some(hit.target);
        if event.click_count >= 2 {
            self.commit_table_widths(hit.target.anchor, None, cx);
        } else {
            self.table_resize.drag = Some(ResizeDrag {
                start_x: f32::from(event.position.x),
                widths_em: hit
                    .widths
                    .iter()
                    .map(|width| width / f32::from(hit.metrics.font_size))
                    .collect(),
                moved: false,
                start: hit,
            });
        }
        cx.notify();
        cx.stop_propagation();
        true
    }

    pub(super) fn table_resize_move(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(drag) = &mut self.table_resize.drag {
            if drag.update(f32::from(position.x)) {
                self.table_column_layout_changed(cx);
            }
            return true;
        }
        let hover = self.table_resize_hit(position, cx).map(|hit| hit.target);
        if self.table_resize.hover != hover {
            self.table_resize.hover = hover;
            cx.notify();
        }
        if hover.is_some() {
            self.dismiss_table_actions(cx);
            self.hovered_link = None;
            return true;
        }
        false
    }

    pub(super) fn table_resize_up(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.table_resize_is_dragging() {
            return false;
        }
        self.table_resize_move(position, cx);
        let drag = self.table_resize.drag.take().expect("active table drag");
        if drag.moved {
            let widths = drag
                .widths_em
                .iter()
                .enumerate()
                .map(|(index, width)| {
                    drag.start.metrics.source_width(
                        index,
                        drag.start.metrics.font_size * *width,
                        drag.start.source_widths[index],
                    )
                })
                .collect::<Vec<_>>();
            self.commit_table_widths(drag.start.target.anchor, Some(&widths), cx);
            self.table_column_layout_changed(cx);
        }
        self.table_resize_move(position, cx);
        cx.notify();
        true
    }

    pub(super) fn cancel_table_resize(&mut self, cx: &mut Context<Self>) -> bool {
        self.clear_table_resize_hover(cx);
        let Some(drag) = self.table_resize.drag.take() else {
            return false;
        };
        if drag.moved {
            self.table_column_layout_changed(cx);
        }
        cx.notify();
        true
    }

    pub(super) fn clear_table_resize_hover(&mut self, cx: &mut Context<Self>) {
        if self.table_resize.hover.take().is_some() {
            cx.notify();
        }
    }

    fn table_column_layout_changed(&mut self, cx: &mut Context<Self>) {
        self.table_resize.revision = self.table_resize.revision.wrapping_add(1);
        self.layout_reflow_pending = true;
        // Retain measured row heights while visible cells rewrap, then prepare
        // one complete layout on release. Old jobs must not restore old widths.
        self.minimap.cancel_layout_preparation();
        self.minimap.invalidate_raster();
        cx.notify();
    }

    fn commit_table_widths(
        &mut self,
        anchor: u64,
        widths: Option<&[usize]>,
        cx: &mut Context<Self>,
    ) {
        let snapshot = self.snapshot(cx);
        let session = self.session.read(cx);
        let format = DocumentFormat::from_path(session.syntax_path());
        let Some(plan) = org_commands::resize_table(
            &snapshot,
            ByteOffset(anchor),
            widths,
            format,
            session.newline_sequence(),
        ) else {
            return;
        };
        if plan.edits.is_empty() {
            return;
        }
        let before = self.selection;
        let after = plan.map_selection(before);
        let result = self.session.update(cx, |session, cx| {
            session.edit(
                DocumentCommand::new(
                    EditTransaction::new(snapshot.revision(), plan.edits),
                    before,
                    after,
                    EditOrigin::Other,
                ),
                cx,
            )
        });
        if let Err(error) = result {
            self.show_command_feedback(&error.to_string(), cx);
            return;
        }
        self.selection = after;
        let snapshot = self.snapshot(cx);
        self.selection_revision = snapshot.revision();
        self.sync_selection_utf16(&snapshot);
        self.hit_rows = Arc::from([]);
        cx.notify();
    }

    /// Capture the active gesture even outside the editor or over an overlay.
    pub(super) fn capture_table_resize(host: &Entity<Self>, window: &mut Window, cx: &App) {
        if !host.read(cx).table_resize_is_dragging() {
            return;
        }
        let move_host = host.downgrade();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if phase != gpui::DispatchPhase::Capture {
                return;
            }
            let _ = move_host.update(cx, |editor, cx| {
                if editor.table_resize_is_dragging() {
                    editor.table_resize_move(event.position, cx);
                    cx.stop_propagation();
                }
            });
        });
        let up_host = host.downgrade();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase != gpui::DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }
            let _ = up_host.update(cx, |editor, cx| {
                if editor.table_resize_up(event.position, cx) {
                    cx.stop_propagation();
                }
            });
        });
    }
}

fn delimiter_positions(
    row: &HitRow,
    snapshot: &DocumentSnapshot,
    format: DocumentFormat,
) -> Vec<(Pixels, Pixels)> {
    if let Some(layout) = &row.table_layout {
        return layout
            .fragments
            .iter()
            .filter(|fragment| fragment.delimiter)
            .map(|fragment| (fragment.x, fragment.width))
            .collect();
    }
    let Ok(range) = snapshot.line_content_range(row.line) else {
        return Vec::new();
    };
    let text = snapshot.copy_range(range);
    table::delimiter_offsets(&text, format)
        .into_iter()
        .filter_map(|offset| {
            let start = row.position_for_display_index(row.display.source_to_display(offset))?;
            let end = row.position_for_display_index(row.display.source_to_display(offset + 1))?;
            (start.y == px(0.) && end.y == start.y).then_some((start.x, end.x - start.x))
        })
        .collect()
}
