use super::*;
impl SemanticEditor {
    pub(crate) fn search_overlay_clearance(&mut self, height: f32, cx: &mut Context<Self>) {
        if (self.display_map.bottom_overlay_clearance - height).abs() < 0.5 {
            return;
        }
        let grew = height > self.display_map.bottom_overlay_clearance;
        self.set_bottom_overlay_clearance(height);
        if grew && self.search_current.is_some() {
            self.pending_search_reveal = true;
        }
        cx.notify();
    }
    pub(crate) fn search_move_vertical(&mut self, delta: i64, cx: &mut Context<Self>) {
        self.move_vertical(delta, false, cx);
    }
    pub(crate) fn search_preview(
        &mut self,
        ranges: Arc<[ByteRange]>,
        current: Option<ByteRange>,
        cx: &mut Context<Self>,
    ) {
        if self.search_current == current {
            self.search_ranges = ranges;
            cx.notify();
            return;
        }
        let snapshot = self.snapshot(cx);
        let path = self.session.read(cx).path().to_owned();
        let base = self.folds.projection(&path, &snapshot);
        if self.search_fold_origin.is_none() {
            self.search_fold_origin = Some(base.clone());
        }
        let changed = self.search_current != current;
        self.search_ranges = ranges;
        self.search_current = current;
        if changed {
            let reveal = current.map(|range| {
                snapshot.line_of_byte(range.start)..snapshot.line_of_byte(range.end) + 1
            });
            let projection = self.folds.projection_revealing(&path, &snapshot, reveal);
            self.finish_fold_animation();
            self.display_map.set_hidden_ranges(projection.hidden_ranges);
            self.fold_markers = Arc::new(projection.marker_lines);
            if let Some(range) = current {
                self.scroll_to_source_offset(range.start, cx);
                self.pending_search_reveal = true;
            }
        }
        cx.notify();
    }
    pub(crate) fn search_finish(&mut self, cancel: bool, cx: &mut Context<Self>) {
        self.set_bottom_overlay_clearance(crate::app::status_line::FLOATING_STATUS_CLEARANCE);
        self.pending_search_reveal = false;
        self.search_ranges = Arc::from([]);
        self.search_current = None;
        if self.search_fold_origin.take().is_some() && cancel {
            let snapshot = self.snapshot(cx);
            let path = self.session.read(cx).path().to_owned();
            let projection = self.folds.projection(&path, &snapshot);
            self.display_map.set_hidden_ranges(projection.hidden_ranges);
            self.fold_markers = Arc::new(projection.marker_lines);
        }
        cx.notify();
    }

    pub(super) fn reveal_pending_search(&mut self, snapshot: &DocumentSnapshot) -> bool {
        if !self.pending_search_reveal {
            return false;
        }
        let Some(range) = self.search_current else {
            self.pending_search_reveal = false;
            return false;
        };
        if !self
            .hit_rows
            .iter()
            .any(|row| row.range.start <= range.start && row.range.end >= range.start)
        {
            return false;
        }
        let selection = self.selection;
        self.selection = Selection::caret(range.start);
        self.reveal_caret(snapshot);
        self.selection = selection;
        self.pending_search_reveal = false;
        true
    }
}
