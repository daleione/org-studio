use std::{collections::HashSet, sync::Arc, time::Instant};

use gpui::{Context, EventEmitter, ListOffset, ListState, Window, px};

use super::{
    BlockId, DerivedEvent, DocumentFormat, FoldMeasurement, FoldTransition, FoldTransitionInput,
    FoldTransitionPlan, GlobalVisibility, LOCAL_FOLD_ANIMATION_DURATION, ListAlignment,
    LocalCycleProjection, LocalVisibility, PreviewSnapshot, accept_generation, changed_range,
    cycle_markdown_subtree_visibility, cycle_org_subtree_visibility, global_markdown_visibility,
    global_org_visibility, minimap, should_eagerly_measure_rows,
};

/// Owns all state whose lifetime and invalidation are local to one rendered document.
///
/// The workspace routes commands and owns window-level settings; this entity owns the immutable
/// derived snapshot plus the virtual-list, fold and minimap presentation state for that snapshot.
pub(crate) struct PreviewPanel {
    document: Arc<PreviewSnapshot>,
    list_state: ListState,
    fold_markers: Arc<HashSet<BlockId>>,
    visible_rows: Arc<Vec<usize>>,
    minimap_state: Arc<minimap::MinimapState>,
    global_visibility: GlobalVisibility,
    global_cycle_contiguous: bool,
    local_cycle_continuation: Option<(BlockId, LocalVisibility)>,
    fold_animation_revision: u64,
    fold_animation: Option<FoldTransition>,
    presentation_revision: u64,
    viewport_revision_key: Option<(u32, u32)>,
    minimap_pending_seek: Option<(u64, ListOffset)>,
    minimap_seek_scheduled: bool,
}

#[derive(Clone)]
pub(in crate::preview) struct PreviewRenderState {
    pub(in crate::preview) document: Arc<PreviewSnapshot>,
    pub(in crate::preview) minimap_state: Arc<minimap::MinimapState>,
    pub(in crate::preview) list_state: ListState,
    pub(in crate::preview) visible_rows: Arc<Vec<usize>>,
    pub(in crate::preview) fold_markers: Arc<HashSet<BlockId>>,
    pub(in crate::preview) fold_animation: Option<FoldTransition>,
    pub(in crate::preview) presentation_revision: u64,
}

impl EventEmitter<DerivedEvent> for PreviewPanel {}

struct GlobalVisibilityProjection {
    visibility: GlobalVisibility,
    visible_rows: Vec<usize>,
    fold_markers: HashSet<BlockId>,
}

impl PreviewPanel {
    pub(in crate::preview) fn new(document: Arc<PreviewSnapshot>, list_overdraw: f32) -> Self {
        let visible_rows = Arc::new((0..document.projection.rows.len()).collect::<Vec<_>>());
        let list_state = ListState::new(visible_rows.len(), ListAlignment::Top, px(list_overdraw));
        if should_eagerly_measure_rows(visible_rows.len()) {
            list_state.clone().measure_all();
        }
        Self {
            document,
            list_state,
            fold_markers: Arc::new(HashSet::new()),
            visible_rows,
            minimap_state: Arc::new(minimap::MinimapState::new()),
            global_visibility: GlobalVisibility::All,
            global_cycle_contiguous: false,
            local_cycle_continuation: None,
            fold_animation_revision: 0,
            fold_animation: None,
            presentation_revision: 0,
            viewport_revision_key: None,
            minimap_pending_seek: None,
            minimap_seek_scheduled: false,
        }
    }

    pub(in crate::preview) fn render_state(&self) -> PreviewRenderState {
        PreviewRenderState {
            document: self.document.clone(),
            minimap_state: self.minimap_state.clone(),
            list_state: self.list_state.clone(),
            visible_rows: self.visible_rows.clone(),
            fold_markers: self.fold_markers.clone(),
            fold_animation: self.fold_animation.clone(),
            presentation_revision: self.presentation_revision,
        }
    }

    pub(in crate::preview) fn document(&self) -> &Arc<PreviewSnapshot> {
        &self.document
    }

    pub(in crate::preview) fn list_state(&self) -> &ListState {
        &self.list_state
    }

    pub(in crate::preview) fn visible_rows(&self) -> &[usize] {
        &self.visible_rows
    }

    #[cfg(test)]
    pub(in crate::preview) fn fold_markers(&self) -> &HashSet<BlockId> {
        &self.fold_markers
    }

    #[cfg(test)]
    pub(in crate::preview) fn global_visibility(&self) -> GlobalVisibility {
        self.global_visibility
    }

    #[cfg(test)]
    pub(in crate::preview) fn fold_animation(&self) -> Option<&FoldTransition> {
        self.fold_animation.as_ref()
    }

    #[cfg(test)]
    pub(in crate::preview) fn local_cycle_continuation(
        &self,
    ) -> Option<(BlockId, LocalVisibility)> {
        self.local_cycle_continuation
    }

    pub(in crate::preview) fn bump_presentation_revision(&mut self) {
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
    }

    pub(in crate::preview) fn scroll_to(&mut self, offset: ListOffset) {
        self.list_state.scroll_to(offset);
    }

    pub(in crate::preview) fn scroll_to_end(&mut self) {
        self.list_state.scroll_to(ListOffset {
            item_ix: self.list_state.item_count(),
            offset_in_item: px(0.0),
        });
    }

    pub(in crate::preview) fn scroll_by(&mut self, amount: gpui::Pixels) {
        self.list_state.scroll_by(amount);
    }

    pub(in crate::preview) fn replace_document(
        &mut self,
        document: Arc<PreviewSnapshot>,
        list_overdraw: f32,
        cx: &mut Context<Self>,
    ) {
        let source_anchor = self.top_source_anchor();
        let global_visibility = self.global_visibility;
        let global_cycle_contiguous = self.global_cycle_contiguous;
        let folded_sources = self
            .fold_markers
            .iter()
            .filter_map(|block_id| self.block_source_start(*block_id))
            .collect::<Vec<_>>();
        let event = document.derived_event();
        *self = Self::new(document, list_overdraw);
        if global_visibility != GlobalVisibility::All {
            if let Some(projection) = self.global_visibility_projection(global_visibility) {
                self.global_visibility = global_visibility;
                self.global_cycle_contiguous = global_cycle_contiguous;
                self.apply_fold_projection(projection.visible_rows, projection.fold_markers);
            }
        } else {
            for source in folded_sources {
                let Some(block_id) = self.closest_block_at(source) else {
                    continue;
                };
                let Some(projection) = self.local_fold_projection(block_id) else {
                    continue;
                };
                if projection.visibility == LocalVisibility::Folded {
                    self.apply_fold_projection(projection.visible_rows, projection.fold_markers);
                }
            }
        }
        if let Some((anchor, offset_in_item)) = source_anchor {
            self.scroll_to_source_offset_with_offset(anchor, offset_in_item);
        }
        cx.emit(event);
        cx.notify();
    }

    fn block_source_start(&self, block_id: BlockId) -> Option<crate::document::ByteOffset> {
        match self.document.format {
            DocumentFormat::Org => self
                .document
                .blocks
                .nodes()
                .get(block_id as usize)
                .map(|block| block.source.start),
            DocumentFormat::Markdown => self
                .document
                .markdown_blocks
                .get(block_id as usize)
                .map(|block| block.source.start),
        }
    }

    fn closest_block_at(&self, source: crate::document::ByteOffset) -> Option<BlockId> {
        let starts: Box<dyn Iterator<Item = (BlockId, crate::document::ByteOffset)> + '_> =
            match self.document.format {
                DocumentFormat::Org => Box::new(
                    self.document
                        .blocks
                        .nodes()
                        .iter()
                        .enumerate()
                        .map(|(id, block)| (id as BlockId, block.source.start)),
                ),
                DocumentFormat::Markdown => Box::new(
                    self.document
                        .markdown_blocks
                        .iter()
                        .enumerate()
                        .map(|(id, block)| (id as BlockId, block.source.start)),
                ),
            };
        starts
            .min_by_key(|(_, start)| start.0.abs_diff(source.0))
            .map(|(id, _)| id)
    }

    pub(in crate::preview) fn top_source_offset(&self) -> Option<crate::document::ByteOffset> {
        self.top_source_anchor().map(|(offset, _)| offset)
    }

    pub(in crate::preview) fn split_anchor(&self) -> Option<(crate::document::ByteOffset, f32)> {
        let scroll_top = self.list_state.logical_scroll_top();
        let source = self.top_source_offset()?;
        let height = self
            .list_state
            .bounds_for_item(scroll_top.item_ix)
            .map(|bounds| f32::from(bounds.size.height))
            .filter(|height| *height > 0.0)
            .unwrap_or(1.0);
        Some((
            source,
            (f32::from(scroll_top.offset_in_item) / height).clamp(0.0, 1.0),
        ))
    }

    fn top_source_anchor(&self) -> Option<(crate::document::ByteOffset, gpui::Pixels)> {
        let scroll_top = self.list_state.logical_scroll_top();
        let item = self
            .list_state
            .logical_scroll_top()
            .item_ix
            .min(self.visible_rows.len().saturating_sub(1));
        let visual = *self.visible_rows.get(item)?;
        self.document
            .projection
            .source_row(visual)
            .map(|row| (row.content.range.start, scroll_top.offset_in_item))
    }

    pub(in crate::preview) fn scroll_to_split_anchor(
        &mut self,
        offset: crate::document::ByteOffset,
        fraction: f32,
    ) {
        let Some(visual) = self
            .document
            .projection
            .visual_row_for_source_offset(offset)
        else {
            return;
        };
        let item = self
            .visible_rows
            .binary_search(&visual)
            .unwrap_or_else(|index| index)
            .min(self.visible_rows.len().saturating_sub(1));
        let height = self
            .list_state
            .bounds_for_item(item)
            .map(|bounds| f32::from(bounds.size.height))
            .filter(|height| *height > 0.0)
            .unwrap_or_else(|| {
                self.document
                    .display_map
                    .as_ref()
                    .map(|map| {
                        let layout = map.layout(visual);
                        layout.fixed_height.unwrap_or(layout.min_height).max(24.0)
                    })
                    .unwrap_or(24.0)
            });
        self.list_state.scroll_to(ListOffset {
            item_ix: item,
            offset_in_item: px(height * fraction.clamp(0.0, 1.0)),
        });
    }

    fn scroll_to_source_offset_with_offset(
        &mut self,
        offset: crate::document::ByteOffset,
        offset_in_item: gpui::Pixels,
    ) {
        let Some(visual) = self
            .document
            .projection
            .visual_row_for_source_offset(offset)
        else {
            return;
        };
        let item = self
            .visible_rows
            .binary_search(&visual)
            .unwrap_or_else(|index| index)
            .min(self.visible_rows.len().saturating_sub(1));
        self.list_state.scroll_to(ListOffset {
            item_ix: item,
            offset_in_item,
        });
    }

    pub(in crate::preview) fn note_viewport(&mut self, viewport: (u32, u32)) -> bool {
        if self.viewport_revision_key == Some(viewport) {
            return false;
        }
        self.viewport_revision_key = Some(viewport);
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        self.cancel_minimap_interaction();
        true
    }

    #[cfg(test)]
    pub(in crate::preview) fn toggle_fold(&mut self, block_id: BlockId) {
        self.discard_fold_animation();
        let Some(projection) = self.local_fold_projection(block_id) else {
            return;
        };
        self.remember_local_fold(block_id, projection.visibility);
        if projection.visibility != LocalVisibility::Empty {
            self.apply_fold_projection(projection.visible_rows, projection.fold_markers);
        }
    }

    pub(in crate::preview) fn toggle_fold_animated(
        &mut self,
        block_id: BlockId,
        viewport_height: f32,
        available_width: f32,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        self.discard_fold_animation();
        let Some(projection) = self.local_fold_projection(block_id) else {
            return;
        };
        self.remember_local_fold(block_id, projection.visibility);
        if projection.visibility != LocalVisibility::Empty {
            self.apply_fold_projection_animated(
                projection.visible_rows,
                projection.fold_markers,
                viewport_height,
                available_width,
                window,
                cx,
            );
        }
    }

    fn local_fold_projection(&self, block_id: BlockId) -> Option<LocalCycleProjection> {
        let continue_from_children =
            self.local_cycle_continuation == Some((block_id, LocalVisibility::Children));
        match self.document.format {
            DocumentFormat::Org => cycle_org_subtree_visibility(
                &self.document.projection.rows,
                &self.document.blocks,
                &self.visible_rows,
                &self.fold_markers,
                block_id,
                continue_from_children,
            ),
            DocumentFormat::Markdown => cycle_markdown_subtree_visibility(
                &self.document.projection.rows,
                &self.document.markdown_blocks,
                &self.visible_rows,
                &self.fold_markers,
                block_id,
                continue_from_children,
            ),
        }
    }

    fn remember_local_fold(&mut self, block_id: BlockId, visibility: LocalVisibility) {
        self.global_cycle_contiguous = false;
        self.local_cycle_continuation = match visibility {
            LocalVisibility::Empty => None,
            visibility => Some((block_id, visibility)),
        };
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_fold_projection_animated(
        &mut self,
        visible_rows: Vec<usize>,
        fold_markers: HashSet<BlockId>,
        viewport_height: f32,
        available_width: f32,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        if cx.reduce_motion() {
            self.apply_fold_projection(visible_rows, fold_markers);
            return;
        }
        let measurement = window
            .as_deref()
            .map_or(FoldMeasurement::Estimated, FoldMeasurement::Rendered);
        let Some(plan) = FoldTransitionPlan::build(FoldTransitionInput {
            current_rows: &self.visible_rows,
            target_rows: &visible_rows,
            document: &self.document,
            list_state: &self.list_state,
            viewport_height,
            available_width,
            measurement,
        }) else {
            self.apply_fold_projection(visible_rows, fold_markers);
            return;
        };

        self.fold_animation_revision = self.fold_animation_revision.wrapping_add(1);
        let revision = self.fold_animation_revision;
        let suppressed_markers = fold_markers
            .difference(&self.fold_markers)
            .copied()
            .collect();
        let transition = plan.into_transition(revision, suppressed_markers);
        self.cancel_minimap_interaction();
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        self.fold_markers = Arc::new(fold_markers);
        self.visible_rows = Arc::new(visible_rows);
        for edit in transition.initial_edits.iter() {
            self.list_state.splice(edit.range.clone(), edit.new_count);
        }
        self.fold_animation = Some(transition);
        if let Some(window) = window {
            self.schedule_fold_animation_frame(revision, window, cx);
        }
    }

    fn apply_fold_projection(&mut self, visible_rows: Vec<usize>, fold_markers: HashSet<BlockId>) {
        self.cancel_minimap_interaction();
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        self.fold_markers = Arc::new(fold_markers);
        self.apply_visible_rows(Arc::new(visible_rows));
    }

    pub(in crate::preview) fn discard_fold_animation(&mut self) {
        self.fold_animation_revision = self.fold_animation_revision.wrapping_add(1);
        self.finish_fold_transition();
    }

    fn finish_fold_transition(&mut self) {
        let Some(animation) = self.fold_animation.take() else {
            return;
        };
        let actual_count = self.list_state.item_count();
        let expected_count = animation.transition_item_count();
        debug_assert_eq!(actual_count, expected_count);
        if actual_count != expected_count {
            self.list_state
                .splice(0..actual_count, self.visible_rows.len());
            return;
        }
        for edit in animation.completion_edits() {
            self.list_state.splice(edit.range, edit.new_count);
        }
    }

    fn schedule_fold_animation_frame(
        &mut self,
        revision: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.on_next_frame(window, move |this, window, cx| {
            let Some(animation) = this.fold_animation.as_mut() else {
                return;
            };
            if animation.revision != revision {
                return;
            }
            let Some(started_at) = animation.started_at else {
                animation.started_at = Some(Instant::now());
                this.schedule_fold_animation_frame(revision, window, cx);
                return;
            };
            let progress = (started_at.elapsed().as_secs_f32()
                / LOCAL_FOLD_ANIMATION_DURATION.as_secs_f32())
            .clamp(0.0, 1.0);
            animation.progress = progress;
            for shell in animation.segments.iter() {
                this.list_state
                    .remeasure_items(shell.transition_index..shell.transition_index + 1);
            }
            cx.notify();
            if progress < 1.0 {
                this.schedule_fold_animation_frame(revision, window, cx);
            } else {
                cx.on_next_frame(window, move |this, _, cx| {
                    if this.fold_animation.as_ref().is_some_and(|animation| {
                        animation.revision == revision && animation.progress >= 1.0
                    }) {
                        this.finish_fold_transition();
                        cx.notify();
                    }
                });
            }
        });
    }

    #[cfg(test)]
    pub(in crate::preview) fn cycle_global_visibility(&mut self) {
        self.discard_fold_animation();
        let Some(projection) = self.next_global_visibility_projection() else {
            return;
        };
        self.remember_global_visibility(projection.visibility);
        self.apply_fold_projection(projection.visible_rows, projection.fold_markers);
    }

    pub(in crate::preview) fn cycle_global_visibility_animated(
        &mut self,
        viewport_height: f32,
        available_width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.discard_fold_animation();
        let Some(projection) = self.next_global_visibility_projection() else {
            return;
        };
        self.remember_global_visibility(projection.visibility);
        self.apply_fold_projection_animated(
            projection.visible_rows,
            projection.fold_markers,
            viewport_height,
            available_width,
            Some(window),
            cx,
        );
    }

    fn next_global_visibility_projection(&self) -> Option<GlobalVisibilityProjection> {
        let next = if self.global_cycle_contiguous {
            self.global_visibility.next()
        } else {
            GlobalVisibility::Overview
        };
        self.global_visibility_projection(next)
    }

    fn global_visibility_projection(
        &self,
        next: GlobalVisibility,
    ) -> Option<GlobalVisibilityProjection> {
        let (visible_rows, fold_markers) = match self.document.format {
            DocumentFormat::Org => {
                global_org_visibility(&self.document.projection.rows, &self.document.blocks, next)
            }
            DocumentFormat::Markdown => global_markdown_visibility(
                &self.document.projection.rows,
                &self.document.markdown_blocks,
                next,
            ),
        };
        if visible_rows.is_empty() && next != GlobalVisibility::All {
            return None;
        }
        Some(GlobalVisibilityProjection {
            visibility: next,
            visible_rows,
            fold_markers,
        })
    }

    fn remember_global_visibility(&mut self, next: GlobalVisibility) {
        self.global_visibility = next;
        self.global_cycle_contiguous = true;
        self.local_cycle_continuation = None;
    }

    fn apply_visible_rows(&mut self, visible_rows: Arc<Vec<usize>>) {
        let (old_range, new_count) = changed_range(&self.visible_rows, &visible_rows);
        self.list_state.splice(old_range, new_count);
        if should_eagerly_measure_rows(visible_rows.len()) {
            self.list_state.clone().measure_all();
        }
        self.visible_rows = visible_rows;
    }

    pub(in crate::preview) fn reset_cycle_continuation(&mut self) {
        self.global_cycle_contiguous = false;
        self.local_cycle_continuation = None;
    }

    pub(in crate::preview) fn seek_minimap(
        &mut self,
        presentation_revision: u64,
        offset: ListOffset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.minimap_pending_seek = Some((presentation_revision, offset));
        if self.minimap_seek_scheduled {
            return;
        }
        self.minimap_seek_scheduled = true;
        cx.on_next_frame(window, |this, _, cx| {
            this.minimap_seek_scheduled = false;
            if let Some((revision, offset)) = this.minimap_pending_seek.take()
                && accept_generation(this.presentation_revision, revision)
            {
                this.list_state.scroll_to(offset);
                cx.notify();
            }
        });
    }

    pub(in crate::preview) fn cancel_minimap_interaction(&mut self) -> bool {
        self.minimap_pending_seek = None;
        self.minimap_seek_scheduled = false;
        let was_dragging = self.minimap_state.cancel_interaction();
        self.list_state.scrollbar_drag_ended();
        was_dragging
    }
}
