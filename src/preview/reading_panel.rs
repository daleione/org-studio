use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{Context, EventEmitter, ListOffset, ListState, ScrollHandle, Window, px};

use super::{
    BlockId, CopyFeedbackState, DerivedEvent, DocumentFormat, FoldMeasurement, FoldTransition,
    FoldTransitionInput, FoldTransitionPlan, GlobalVisibility, LOCAL_FOLD_ANIMATION_DURATION,
    ListAlignment, LocalCycleProjection, LocalVisibility, PendingPreviewAction,
    PreviewActionVisualState, PreviewSnapshot, accept_generation, changed_range,
    cycle_markdown_subtree_visibility, cycle_org_subtree_visibility, global_markdown_visibility,
    global_org_visibility, minimap, should_eagerly_measure_rows, visible_markdown_row_indices,
    visible_row_indices,
};

/// Owns all state whose lifetime and invalidation are local to one rendered document.
///
/// The workspace routes commands and owns window-level settings; this entity owns the immutable
/// derived snapshot plus the virtual-list, fold and minimap presentation state for that snapshot.
pub(crate) struct ReadingPreviewPanel {
    document: Arc<PreviewSnapshot>,
    list_state: ListState,
    fold_markers: Arc<HashSet<BlockId>>,
    visible_rows: Arc<Vec<usize>>,
    minimap_state: Arc<minimap::MinimapState>,
    table_scroll_handles: Arc<HashMap<BlockId, ScrollHandle>>,
    global_visibility: GlobalVisibility,
    global_cycle_contiguous: bool,
    local_cycle_continuation: Option<(BlockId, LocalVisibility)>,
    fold_animation_revision: u64,
    fold_animation: Option<FoldTransition>,
    geometry_revision: u64,
    zoom: f32,
    viewport_revision_key: Option<(u32, u32)>,
    minimap_pending_seek: Option<(u64, ListOffset)>,
    minimap_seek_scheduled: bool,
    next_action_id: u64,
    pending_actions: Vec<PendingPreviewAction>,
    failed_action: Option<super::PreviewActionIdentity>,
    copy_feedback: Option<(crate::document::ByteRange, CopyFeedbackState)>,
    copy_feedback_request: u64,
    copy_feedback_task: Option<gpui::Task<()>>,
}

#[derive(Clone)]
pub(in crate::preview) struct ReadingRenderState {
    pub(in crate::preview) document: Arc<PreviewSnapshot>,
    pub(in crate::preview) minimap_state: Arc<minimap::MinimapState>,
    pub(in crate::preview) table_scroll_handles: Arc<HashMap<BlockId, ScrollHandle>>,
    pub(in crate::preview) list_state: ListState,
    pub(in crate::preview) visible_rows: Arc<Vec<usize>>,
    pub(in crate::preview) fold_markers: Arc<HashSet<BlockId>>,
    pub(in crate::preview) fold_animation: Option<FoldTransition>,
    pub(in crate::preview) geometry_revision: u64,
    pub(in crate::preview) zoom: f32,
    pub(in crate::preview) action_states:
        Arc<Vec<(super::PreviewActionIdentity, PreviewActionVisualState)>>,
    pub(in crate::preview) copy_feedback: Option<(crate::document::ByteRange, CopyFeedbackState)>,
}

impl EventEmitter<DerivedEvent> for ReadingPreviewPanel {}

struct GlobalVisibilityProjection {
    visibility: GlobalVisibility,
    visible_rows: Vec<usize>,
    fold_markers: HashSet<BlockId>,
}

fn table_scroll_handles(
    document: &PreviewSnapshot,
    previous: Option<&HashMap<BlockId, ScrollHandle>>,
) -> Arc<HashMap<BlockId, ScrollHandle>> {
    let mut handles = HashMap::new();
    for row in document.projection.rows.iter() {
        let super::projection::VisualRowKind::Table(table) = &row.kind else {
            continue;
        };
        handles.entry(table.group_id()).or_insert_with(|| {
            previous
                .and_then(|previous| previous.get(&table.group_id()))
                .cloned()
                .unwrap_or_default()
        });
    }
    handles.into()
}

impl ReadingPreviewPanel {
    pub(in crate::preview) fn new(document: Arc<PreviewSnapshot>, list_overdraw: f32) -> Self {
        let visible_rows = Arc::new((0..document.projection.rows.len()).collect::<Vec<_>>());
        let list_state = ListState::new(visible_rows.len(), ListAlignment::Top, px(list_overdraw));
        if should_eagerly_measure_rows(visible_rows.len()) {
            list_state.clone().measure_all();
        }
        let minimap_state = Arc::new(minimap::MinimapState::new());
        let table_scroll_handles = table_scroll_handles(&document, None);
        Self {
            document,
            list_state,
            fold_markers: Arc::new(HashSet::new()),
            visible_rows,
            minimap_state,
            table_scroll_handles,
            global_visibility: GlobalVisibility::All,
            global_cycle_contiguous: false,
            local_cycle_continuation: None,
            fold_animation_revision: 0,
            fold_animation: None,
            geometry_revision: 0,
            zoom: 1.0,
            viewport_revision_key: None,
            minimap_pending_seek: None,
            minimap_seek_scheduled: false,
            next_action_id: 1,
            pending_actions: Vec::new(),
            failed_action: None,
            copy_feedback: None,
            copy_feedback_request: 0,
            copy_feedback_task: None,
        }
    }

    pub(in crate::preview) fn render_state(&self) -> ReadingRenderState {
        ReadingRenderState {
            document: self.document.clone(),
            minimap_state: self.minimap_state.clone(),
            table_scroll_handles: self.table_scroll_handles.clone(),
            list_state: self.list_state.clone(),
            visible_rows: self.visible_rows.clone(),
            fold_markers: self.fold_markers.clone(),
            fold_animation: self.fold_animation.clone(),
            geometry_revision: self.geometry_revision,
            zoom: self.zoom(),
            action_states: self
                .pending_actions
                .iter()
                .map(|action| (action.identity, PreviewActionVisualState::Pending))
                .chain(
                    self.failed_action
                        .as_ref()
                        .map(|identity| (*identity, PreviewActionVisualState::Failed)),
                )
                .collect::<Vec<_>>()
                .into(),
            copy_feedback: self.copy_feedback,
        }
    }

    pub(in crate::preview) fn change_style(
        &mut self,
        previous: super::PreviewStyle,
        next: super::PreviewStyle,
        cx: &mut Context<Self>,
    ) {
        let layout_changed = previous.layout_key() != next.layout_key();
        let paint_changed = previous.paint_key() != next.paint_key();
        if !layout_changed && !paint_changed {
            return;
        }
        self.discard_fold_animation();
        let was_at_bottom = layout_changed && self.viewport_reaches_document_end();
        let source_anchor = layout_changed.then(|| self.top_source_anchor()).flatten();
        if layout_changed {
            self.geometry_revision = self.geometry_revision.wrapping_add(1);
            self.list_state
                .remeasure_items(0..self.list_state.item_count());
            if was_at_bottom && self.list_state.item_count() > 0 {
                self.list_state.scroll_to(ListOffset {
                    item_ix: self.list_state.item_count() - 1,
                    offset_in_item: px(0.0),
                });
            } else if let Some((offset, offset_in_item)) = source_anchor {
                self.scroll_to_source_offset_with_offset(offset, offset_in_item);
            }
        }
        self.minimap_pending_seek = None;
        self.minimap_seek_scheduled = false;
        self.minimap_state.invalidate_style(layout_changed);
        cx.notify();
    }

    pub(in crate::preview) fn show_copy_feedback(
        &mut self,
        target_range: crate::document::ByteRange,
        state: CopyFeedbackState,
        cx: &mut Context<Self>,
    ) {
        const FEEDBACK_DURATION: Duration = Duration::from_millis(1_600);
        self.copy_feedback_request = self.copy_feedback_request.wrapping_add(1);
        let request = self.copy_feedback_request;
        self.copy_feedback = Some((target_range, state));
        let delay = cx.background_executor().timer(FEEDBACK_DURATION);
        self.copy_feedback_task = Some(cx.spawn(async move |this, cx| {
            delay.await;
            let _ = this.update(cx, |this, cx| {
                if this.copy_feedback_request != request {
                    return;
                }
                this.copy_feedback_task = None;
                this.copy_feedback = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(in crate::preview) fn begin_action(
        &mut self,
        identity: super::PreviewActionIdentity,
    ) -> Option<u64> {
        if self
            .pending_actions
            .iter()
            .any(|action| action.identity == identity)
        {
            return None;
        }
        let id = self.next_action_id;
        self.next_action_id = self.next_action_id.wrapping_add(1).max(1);
        self.pending_actions.push(PendingPreviewAction {
            id,
            identity,
            committed_revision: None,
        });
        self.failed_action = None;
        Some(id)
    }

    pub(in crate::preview) fn commit_action(
        &mut self,
        id: u64,
        revision: crate::document::Revision,
    ) {
        if let Some(action) = self
            .pending_actions
            .iter_mut()
            .find(|action| action.id == id)
        {
            action.committed_revision = Some(revision);
        }
    }

    pub(in crate::preview) fn fail_action(&mut self, id: u64) {
        let Some(index) = self
            .pending_actions
            .iter()
            .position(|action| action.id == id)
        else {
            return;
        };
        let action = self.pending_actions.remove(index);
        self.failed_action = Some(action.identity);
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

    pub(in crate::preview) fn zoom(&self) -> f32 {
        self.zoom
    }

    #[cfg(test)]
    pub(in crate::preview) fn set_zoom(&mut self, zoom: f32) -> bool {
        let zoom = zoom.clamp(0.75, 2.0);
        if (self.zoom - zoom).abs() < f32::EPSILON {
            return false;
        }
        self.zoom = zoom;
        self.bump_geometry_revision();
        true
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

    pub(in crate::preview) fn bump_geometry_revision(&mut self) {
        self.geometry_revision = self.geometry_revision.wrapping_add(1);
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

    pub(in crate::preview) fn replace_document_with_style(
        &mut self,
        document: Arc<PreviewSnapshot>,
        style: super::PreviewStyle,
        cx: &mut Context<Self>,
    ) {
        let source_anchor = self.top_source_anchor();
        let previous_visible_rows = self.visible_rows.clone();
        let previous_minimap = self.minimap_state.clone();
        let global_visibility = self.global_visibility;
        let global_cycle_contiguous = self.global_cycle_contiguous;
        let local_cycle = self
            .local_cycle_continuation
            .and_then(|(block_id, visibility)| {
                self.block_source_start(block_id).map(|source| {
                    let syntax_id = (self.document.format == DocumentFormat::Org)
                        .then(|| {
                            self.document
                                .blocks
                                .nodes()
                                .get(block_id as usize)
                                .map(|block| block.syntax_id)
                        })
                        .flatten();
                    (syntax_id, source, visibility)
                })
            });
        let folded_sources = self
            .fold_markers
            .iter()
            .filter_map(|block_id| {
                self.block_source_start(*block_id).map(|source| {
                    let syntax_id = match self.document.format {
                        DocumentFormat::Org => self
                            .document
                            .blocks
                            .nodes()
                            .get(*block_id as usize)
                            .map(|block| block.syntax_id),
                        DocumentFormat::Markdown => None,
                    };
                    (syntax_id, source)
                })
            })
            .collect::<Vec<_>>();
        let event = document.derived_event();
        let mut next_global_visibility = GlobalVisibility::All;
        let mut next_global_cycle_contiguous = false;
        let (next_visible_rows, next_fold_markers) = if global_visibility != GlobalVisibility::All {
            let (visible_rows, fold_markers) = match document.format {
                DocumentFormat::Org => global_org_visibility(
                    &document.projection.rows,
                    &document.blocks,
                    global_visibility,
                ),
                DocumentFormat::Markdown => global_markdown_visibility(
                    &document.projection.rows,
                    &document.markdown_blocks,
                    global_visibility,
                ),
            };
            if visible_rows.is_empty() {
                (
                    (0..document.projection.rows.len()).collect(),
                    HashSet::new(),
                )
            } else {
                next_global_visibility = global_visibility;
                next_global_cycle_contiguous = global_cycle_contiguous;
                (visible_rows, fold_markers)
            }
        } else {
            let mut markers = HashSet::with_capacity(folded_sources.len());
            for (syntax_id, source) in folded_sources {
                let Some(block_id) = syntax_id
                    .and_then(|syntax_id| document.blocks.block_for_syntax_id(syntax_id))
                    .or_else(|| Self::closest_block_in(&document, source))
                else {
                    continue;
                };
                if Self::is_heading_in(&document, block_id) {
                    markers.insert(block_id);
                }
            }
            let visible_rows = match document.format {
                DocumentFormat::Org => {
                    visible_row_indices(&document.projection.rows, &document.blocks, &markers)
                }
                DocumentFormat::Markdown => visible_markdown_row_indices(
                    &document.projection.rows,
                    &document.markdown_blocks,
                    &markers,
                ),
            };
            (visible_rows, markers)
        };
        let next_local_cycle = local_cycle.and_then(|(syntax_id, source, visibility)| {
            syntax_id
                .and_then(|syntax_id| document.blocks.block_for_syntax_id(syntax_id))
                .or_else(|| Self::closest_block_in(&document, source))
                .filter(|block_id| Self::is_heading_in(&document, *block_id))
                .map(|block_id| (block_id, visibility))
        });
        let incremental_update = match &document.update {
            super::DerivedUpdate::Incremental {
                patch,
                reused_chunks,
                total_chunks,
            } => Some((patch.clone(), *reused_chunks, *total_chunks)),
            super::DerivedUpdate::Full => None,
        };
        let invalidates_geometry = incremental_update.as_ref().is_none_or(|(patch, _, _)| {
            patch
                .invalidation
                .contains(super::projection::InvalidationFlags::GEOMETRY)
        });
        let preserves_visible_rows = incremental_update.is_some()
            && next_visible_rows.as_slice() == previous_visible_rows.as_slice();
        self.discard_fold_animation();
        self.table_scroll_handles =
            table_scroll_handles(&document, Some(&self.table_scroll_handles));
        self.document = document;
        let previous_action_count = self.pending_actions.len();
        self.pending_actions.retain(|action| {
            action
                .committed_revision
                .is_none_or(|revision| self.document.revision < revision)
        });
        if self.pending_actions.len() != previous_action_count {
            self.failed_action = None;
        }
        self.fold_markers = Arc::new(next_fold_markers);
        self.global_visibility = next_global_visibility;
        self.global_cycle_contiguous = next_global_cycle_contiguous;
        self.local_cycle_continuation = next_local_cycle;
        if invalidates_geometry {
            self.geometry_revision = self.geometry_revision.wrapping_add(1);
        }
        self.minimap_pending_seek = None;
        self.minimap_seek_scheduled = false;
        if preserves_visible_rows {
            self.visible_rows = previous_visible_rows;
            self.minimap_state = previous_minimap;
        } else {
            self.minimap_state = Arc::new(minimap::MinimapState::new());
            self.apply_visible_rows(Arc::new(next_visible_rows));
            if let Some((anchor, offset_in_item)) = source_anchor {
                self.scroll_to_source_offset_with_offset(anchor, offset_in_item);
            }
        }
        if let Some((patch, reused_chunks, total_chunks)) = incremental_update {
            if preserves_visible_rows {
                if invalidates_geometry {
                    let start = self
                        .visible_rows
                        .partition_point(|row| *row < patch.new_visual.start);
                    let end = self
                        .visible_rows
                        .partition_point(|row| *row < patch.new_visual.end);
                    self.list_state.remeasure_items(start..end);
                }
                self.minimap_state.apply_document_patch(
                    &self.document,
                    &patch,
                    &self.visible_rows,
                    self.zoom(),
                    style,
                );
            }
            if minimap::minimap_perf_enabled() {
                eprintln!(
                    "org_preview_incremental revision={} old_rows={} new_rows={} reused_chunks={} total_chunks={} reparsed_bytes={} document_bytes={}",
                    self.document.revision.0,
                    patch.old_visual.len(),
                    patch.new_visual.len(),
                    reused_chunks,
                    total_chunks,
                    self.document.metrics.syntax_reparsed_bytes,
                    self.document.metrics.bytes,
                );
            }
        } else {
            let count = self.list_state.item_count();
            // A full projection rebuild invalidates every row's measurement, but the item set was
            // already reconciled above. Splicing the whole list a second time resets its logical
            // scroll position after the source anchor has been restored.
            self.list_state.remeasure_items(0..count);
            if minimap::minimap_perf_enabled() {
                eprintln!(
                    "org_preview_full_update revision={} fallback={} parsed_bytes={} document_bytes={}",
                    self.document.revision.0,
                    self.document.metrics.full_syntax_fallback,
                    self.document.metrics.syntax_reparsed_bytes,
                    self.document.metrics.bytes,
                );
            }
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

    fn closest_block_in(
        document: &PreviewSnapshot,
        source: crate::document::ByteOffset,
    ) -> Option<BlockId> {
        match document.format {
            DocumentFormat::Org => {
                closest_block_by_source(document.blocks.nodes().len(), source, |index| {
                    document.blocks.nodes()[index].source.start
                })
            }
            DocumentFormat::Markdown => {
                closest_block_by_source(document.markdown_blocks.len(), source, |index| {
                    document.markdown_blocks[index].source.start
                })
            }
        }
    }

    fn is_heading_in(document: &PreviewSnapshot, block_id: BlockId) -> bool {
        match document.format {
            DocumentFormat::Org => {
                document
                    .blocks
                    .nodes()
                    .get(block_id as usize)
                    .is_some_and(|block| {
                        matches!(block.kind, crate::org_syntax::BlockKind::Heading { .. })
                    })
            }
            DocumentFormat::Markdown => document
                .markdown_blocks
                .get(block_id as usize)
                .is_some_and(|block| {
                    matches!(block.kind, super::markdown::MarkdownKind::Heading { .. })
                }),
        }
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

    fn viewport_reaches_document_end(&self) -> bool {
        let count = self.list_state.item_count();
        if count == 0 {
            return true;
        }
        let viewport = self.list_state.viewport_bounds();
        if f32::from(viewport.size.height) <= 0.0 {
            return false;
        }
        self.list_state
            .bounds_for_item(count - 1)
            .is_some_and(|bounds| bounds.bottom() <= viewport.bottom() + px(0.5))
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

    pub(in crate::preview) fn jump_to_destination(&mut self, destination: &str) -> bool {
        let target = destination
            .strip_prefix('#')
            .or_else(|| destination.strip_prefix('*'))
            .unwrap_or(destination)
            .trim();
        if target.is_empty() {
            return false;
        }
        let target_slug = heading_slug(target);
        let offset = self
            .document
            .projection
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row.kind, super::projection::VisualRowKind::Heading(_)))
            .find_map(|(index, _)| {
                let source_row = self.document.projection.source_row(index)?;
                let source = self.document.text.copy_range(source_row.content.range);
                let source_title = match self.document.format {
                    DocumentFormat::Org => {
                        super::org_line::parse_heading(source.trim_end_matches(['\r', '\n'])).title
                    }
                    DocumentFormat::Markdown => source
                        .trim_end_matches(['\r', '\n'])
                        .trim_start_matches('#')
                        .trim()
                        .trim_end_matches('#')
                        .trim()
                        .to_owned(),
                };
                let title = super::parse_document_inline(self.document.format, &source_title).text;
                (title.eq_ignore_ascii_case(target) || heading_slug(&title) == target_slug)
                    .then_some(source_row.content.range.start)
            });
        let Some(offset) = offset else {
            return false;
        };
        self.scroll_to_source_offset_with_offset(offset, px(0.0));
        true
    }

    pub(in crate::preview) fn note_viewport(&mut self, viewport: (u32, u32)) -> bool {
        if self.viewport_revision_key == Some(viewport) {
            return false;
        }
        self.viewport_revision_key = Some(viewport);
        self.geometry_revision = self.geometry_revision.wrapping_add(1);
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
        style: super::PreviewStyle,
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
                style,
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
        style: super::PreviewStyle,
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
            zoom: self.zoom(),
            style,
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
        self.geometry_revision = self.geometry_revision.wrapping_add(1);
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
        self.geometry_revision = self.geometry_revision.wrapping_add(1);
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
        style: super::PreviewStyle,
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
            style,
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
        geometry_revision: u64,
        offset: ListOffset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.minimap_pending_seek = Some((geometry_revision, offset));
        if self.minimap_seek_scheduled {
            return;
        }
        self.minimap_seek_scheduled = true;
        cx.on_next_frame(window, |this, _, cx| {
            this.minimap_seek_scheduled = false;
            if let Some((revision, offset)) = this.minimap_pending_seek.take()
                && accept_generation(this.geometry_revision, revision)
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

fn heading_slug(title: &str) -> String {
    let mut slug = String::with_capacity(title.len());
    let mut separator = false;
    for character in title.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() || character == '_' {
            if separator && !slug.is_empty() {
                slug.push('-');
            }
            separator = false;
            slug.push(character);
        } else {
            separator = true;
        }
    }
    slug
}

fn closest_block_by_source(
    len: usize,
    source: crate::document::ByteOffset,
    source_at: impl Fn(usize) -> crate::document::ByteOffset,
) -> Option<BlockId> {
    if len == 0 {
        return None;
    }
    let mut low = 0;
    let mut high = len;
    while low < high {
        let middle = low + (high - low) / 2;
        if source_at(middle) <= source {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    let after = low;
    let index = match (after.checked_sub(1), (after < len).then_some(after)) {
        (Some(before), Some(after)) => {
            if source_at(before).0.abs_diff(source.0) <= source_at(after).0.abs_diff(source.0) {
                before
            } else {
                after
            }
        }
        (Some(before), None) => before,
        (None, Some(after)) => after,
        (None, None) => return None,
    };
    BlockId::try_from(index).ok()
}
