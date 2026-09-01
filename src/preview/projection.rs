use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{
    document::{EditLog, Revision, RevisionDelta, RevisionRange, TextSnapshot},
    org_syntax::{BlockArena, BlockId, BlockKind},
};

use super::{
    DocumentFormat, PreviewRow,
    markdown::{MarkdownBlock, MarkdownKind},
    org_line::{CheckboxState, parse_list_item},
    style::RowStyleKind,
    table::TableRowProjection,
};

const ROW_CHUNK_CAPACITY: usize = 128;
static NEXT_VISUAL_ROW_ID: AtomicU64 = AtomicU64::new(1);

fn next_visual_row_id() -> VisualRowId {
    VisualRowId(NEXT_VISUAL_ROW_ID.fetch_add(1, Ordering::Relaxed))
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::preview) struct VisualRowId(pub(in crate::preview) u64);

#[derive(Clone, Debug)]
pub(in crate::preview) enum VisualRowKind {
    Text,
    Heading(u8),
    List(ReadingListMarker),
    Caption,
    Quote,
    Code(ReadingCodeRow),
    Blank,
    Table(TableRowProjection),
    Image { dimensions: Option<(u32, u32)> },
    Rule,
    Hidden,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::preview) enum ReadingCodeRow {
    Start,
    Body,
    End,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::preview) struct ReadingListMarker {
    pub(in crate::preview) marker: Arc<str>,
    pub(in crate::preview) checkbox: Option<CheckboxState>,
    pub(in crate::preview) indent: u16,
    /// Hash of the rendered term/body, deliberately excluding checkbox state.
    /// Checkbox state changes paint the marker but do not change row geometry.
    pub(in crate::preview) text_signature: u64,
}

#[derive(Clone, Copy)]
pub(in crate::preview) struct ReadingProjectionResources<'a> {
    pub(in crate::preview) tables: &'a HashMap<BlockId, TableRowProjection>,
    pub(in crate::preview) images: &'a HashMap<BlockId, (u32, u32)>,
}

#[derive(Clone, Debug)]
pub(in crate::preview) struct VisualRow {
    pub(in crate::preview) id: VisualRowId,
    pub(in crate::preview) source: RevisionRange,
    pub(in crate::preview) block_id: BlockId,
    pub(in crate::preview) semantic_revision: u64,
    pub(in crate::preview) kind: VisualRowKind,
    pub(in crate::preview) code_language: Option<Arc<str>>,
    pub(in crate::preview) code_action_range: Option<crate::document::ByteRange>,
    pub(in crate::preview) style_kind: RowStyleKind,
    pub(in crate::preview) render: PreviewRow,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::preview) struct VisualRowSummary {
    pub(in crate::preview) rows: usize,
    pub(in crate::preview) source_bytes: u64,
}

#[derive(Clone, Debug)]
pub(in crate::preview) struct VisualRowChunk {
    rows: Arc<[VisualRow]>,
    summary: VisualRowSummary,
    first_source: Option<RevisionRange>,
    last_source: Option<RevisionRange>,
}

impl VisualRowChunk {
    fn new(rows: impl Into<Arc<[VisualRow]>>) -> Self {
        let rows = rows.into();
        let first_source = rows.first().map(|row| row.source);
        let last_source = rows.last().map(|row| row.source);
        let source_bytes = rows
            .iter()
            .map(|row| {
                row.source
                    .range
                    .end
                    .0
                    .saturating_sub(row.source.range.start.0)
            })
            .sum();
        Self {
            summary: VisualRowSummary {
                rows: rows.len(),
                source_bytes,
            },
            rows,
            first_source,
            last_source,
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::preview) struct VisualRowTree {
    chunks: Arc<[Arc<VisualRowChunk>]>,
    row_prefix: Arc<[usize]>,
    summary: VisualRowSummary,
}

#[derive(Clone, Debug)]
pub(in crate::preview) struct PresentationTree {
    chunks: Arc<[Arc<[VisualRowId]>]>,
    row_prefix: Arc<[usize]>,
    len: usize,
}

impl PresentationTree {
    fn from_visual_rows(rows: &VisualRowTree) -> Self {
        let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
        Self::from_chunks(
            ids.chunks(ROW_CHUNK_CAPACITY)
                .map(|chunk| Arc::<[VisualRowId]>::from(chunk.to_vec()))
                .collect(),
        )
    }

    pub(in crate::preview) fn len(&self) -> usize {
        self.len
    }

    fn replace(&self, range: Range<usize>, replacements: &[VisualRow]) -> Self {
        assert!(range.start <= range.end && range.end <= self.len);
        let (start_chunk, start_local) = self.locate_boundary(range.start);
        let (end_chunk, end_local) = self.locate_boundary(range.end);
        let mut chunks = self.chunks[..start_chunk].to_vec();
        let mut middle = Vec::new();
        if let Some(chunk) = self.chunks.get(start_chunk) {
            middle.extend_from_slice(&chunk[..start_local]);
        }
        middle.extend(replacements.iter().map(|row| row.id));
        let after_start = if let Some(chunk) = self.chunks.get(end_chunk) {
            if end_chunk == start_chunk || end_local > 0 {
                middle.extend_from_slice(&chunk[end_local..]);
                end_chunk + 1
            } else {
                end_chunk
            }
        } else {
            self.chunks.len()
        };
        chunks.extend(
            middle
                .chunks(ROW_CHUNK_CAPACITY)
                .filter(|chunk| !chunk.is_empty())
                .map(|chunk| Arc::<[VisualRowId]>::from(chunk.to_vec())),
        );
        chunks.extend_from_slice(&self.chunks[after_start..]);
        Self::from_chunks(chunks.into())
    }

    fn locate_boundary(&self, position: usize) -> (usize, usize) {
        if position >= self.len {
            return (self.chunks.len(), 0);
        }
        let chunk = self
            .row_prefix
            .partition_point(|&start| start <= position)
            .saturating_sub(1);
        (chunk, position - self.row_prefix[chunk])
    }

    fn from_chunks(chunks: Arc<[Arc<[VisualRowId]>]>) -> Self {
        let mut row_prefix = Vec::with_capacity(chunks.len() + 1);
        row_prefix.push(0usize);
        for chunk in chunks.iter() {
            row_prefix.push(row_prefix.last().copied().unwrap_or(0) + chunk.len());
        }
        let len = row_prefix.last().copied().unwrap_or(0);
        Self {
            chunks,
            row_prefix: row_prefix.into(),
            len,
        }
    }
}

impl VisualRowTree {
    pub(in crate::preview) fn from_rows(rows: Vec<VisualRow>) -> Self {
        let chunks = rows
            .chunks(ROW_CHUNK_CAPACITY)
            .map(|rows| Arc::new(VisualRowChunk::new(rows.to_vec())))
            .collect::<Arc<[_]>>();
        Self::from_chunks(chunks)
    }

    fn from_chunks(chunks: Arc<[Arc<VisualRowChunk>]>) -> Self {
        let mut row_prefix = Vec::with_capacity(chunks.len() + 1);
        row_prefix.push(0usize);
        let summary = chunks
            .iter()
            .fold(VisualRowSummary::default(), |mut sum, chunk| {
                sum.rows += chunk.summary.rows;
                sum.source_bytes += chunk.summary.source_bytes;
                row_prefix.push(sum.rows);
                sum
            });
        Self {
            chunks,
            row_prefix: row_prefix.into(),
            summary,
        }
    }

    pub(in crate::preview) fn len(&self) -> usize {
        self.summary.rows
    }

    pub(in crate::preview) fn get(&self, index: usize) -> Option<&VisualRow> {
        if index >= self.summary.rows {
            return None;
        }
        let chunk = self
            .row_prefix
            .partition_point(|&start| start <= index)
            .saturating_sub(1);
        self.chunks[chunk].rows.get(index - self.row_prefix[chunk])
    }

    pub(in crate::preview) fn iter(&self) -> impl Iterator<Item = &VisualRow> {
        self.chunks.iter().flat_map(|chunk| chunk.rows.iter())
    }

    #[allow(dead_code)] // Called by the editing pipeline once a syntax adapter supplies replacements.
    fn replace(&self, range: Range<usize>, replacements: Vec<VisualRow>) -> Self {
        assert!(range.start <= range.end && range.end <= self.len());
        let (start_chunk, start_local) = self.locate_boundary(range.start);
        let (end_chunk, end_local) = self.locate_boundary(range.end);
        let mut chunks = self.chunks[..start_chunk].to_vec();
        let mut middle = Vec::new();
        if let Some(chunk) = self.chunks.get(start_chunk) {
            middle.extend_from_slice(&chunk.rows[..start_local]);
        }
        middle.extend(replacements);
        let after_start = if let Some(chunk) = self.chunks.get(end_chunk) {
            if end_chunk == start_chunk || end_local > 0 {
                middle.extend_from_slice(&chunk.rows[end_local..]);
                end_chunk + 1
            } else {
                end_chunk
            }
        } else {
            self.chunks.len()
        };
        let rebuilt = middle
            .chunks(ROW_CHUNK_CAPACITY)
            .filter(|rows| !rows.is_empty())
            .map(|rows| Arc::new(VisualRowChunk::new(rows.to_vec())));
        chunks.extend(rebuilt);
        chunks.extend_from_slice(&self.chunks[after_start..]);
        Self::from_chunks(chunks.into())
    }

    #[allow(dead_code)]
    fn locate_boundary(&self, position: usize) -> (usize, usize) {
        if position >= self.summary.rows {
            return (self.chunks.len(), 0);
        }
        let chunk = self
            .row_prefix
            .partition_point(|&start| start <= position)
            .saturating_sub(1);
        (chunk, position - self.row_prefix[chunk])
    }

    #[cfg(test)]
    fn chunks(&self) -> &[Arc<VisualRowChunk>] {
        &self.chunks
    }
}

#[derive(Clone, Debug)]
pub(in crate::preview) struct ReadingProjection {
    pub(in crate::preview) revision: Revision,
    pub(in crate::preview) revisions: VisualRevisions,
    pub(in crate::preview) rows: VisualRowTree,
    pub(in crate::preview) presentation: PresentationTree,
    edit_log: Arc<EditLog>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::preview) struct VisualRevisions {
    pub(in crate::preview) geometry: u64,
    pub(in crate::preview) paint: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(in crate::preview) struct InvalidationFlags(u8);

impl InvalidationFlags {
    pub(in crate::preview) const CONTENT: Self = Self(1 << 0);
    pub(in crate::preview) const GEOMETRY: Self = Self(1 << 1);
    pub(in crate::preview) const PAINT: Self = Self(1 << 2);

    pub(in crate::preview) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(in crate::preview) const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::preview) struct VisualPatch {
    pub(in crate::preview) before_revision: Revision,
    pub(in crate::preview) after_revision: Revision,
    pub(in crate::preview) old_visual: Range<usize>,
    pub(in crate::preview) new_visual: Range<usize>,
    pub(in crate::preview) invalidation: InvalidationFlags,
}

/// The changed-row envelope bound to the exact revision chain that produced it.
///
/// Keeping this opaque prevents callers from accidentally pairing a range calculated for one
/// delta chain with a different patch operation.
#[derive(Clone, Debug)]
pub(in crate::preview) struct VisualPatchPlan {
    before_revision: Revision,
    after_revision: Revision,
    affected: Range<usize>,
}

impl VisualPatchPlan {
    pub(in crate::preview) fn affected(&self) -> Range<usize> {
        self.affected.clone()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(in crate::preview) enum ProjectionPatchError {
    StaleSnapshot,
    InvalidVisualRange,
    ReplacementRevision,
    UncoveredChangedRow,
}

impl ReadingProjection {
    fn rebased(&self) -> Result<Self, ProjectionPatchError> {
        let rows = self
            .rows
            .iter()
            .map(|row| {
                let source = self
                    .edit_log
                    .map_range(row.source, self.revision)
                    .map_err(|_| ProjectionPatchError::StaleSnapshot)?;
                let mut row = row.clone();
                row.source = source;
                row.render.content = source;
                Ok(row)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            revision: self.revision,
            revisions: self.revisions,
            rows: VisualRowTree::from_rows(rows),
            presentation: self.presentation.clone(),
            edit_log: Arc::new(
                EditLog::new(256).expect("projection edit log capacity is non-zero"),
            ),
        })
    }

    pub(in crate::preview) fn visual_row_for_source_offset(
        &self,
        offset: crate::document::ByteOffset,
    ) -> Option<usize> {
        if self.rows.len() == 0 {
            return None;
        }
        let (mut low, mut high) = (0usize, self.rows.len());
        while low < high {
            let middle = low + (high - low) / 2;
            let row = self.source_row(middle)?;
            if row.content.range.end <= offset {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        Some(low.min(self.rows.len() - 1))
    }

    pub(in crate::preview) fn source_row(&self, index: usize) -> Option<PreviewRow> {
        let visual = self.rows.get(index)?;
        let mut row = visual.render;
        row.content = self.edit_log.map_range(visual.source, self.revision).ok()?;
        Some(row)
    }

    fn affected_visual_range_with_log(
        &self,
        delta: &RevisionDelta,
        edit_log: &EditLog,
    ) -> Result<Range<usize>, ProjectionPatchError> {
        if self.revision != delta.before && edit_log.latest_revision() != Some(delta.before) {
            return Err(ProjectionPatchError::StaleSnapshot);
        }
        let mut affected_start = self.rows.len();
        let mut affected_end = 0usize;
        for edit in delta.edits.iter().copied() {
            let mut offset = 0usize;
            let mut edit_found = false;
            for chunk in self.rows.chunks.iter() {
                let chunk_len = chunk.rows.len();
                let envelope =
                    chunk
                        .first_source
                        .zip(chunk.last_source)
                        .and_then(|(first, last)| {
                            let first = edit_log.map_range(first, delta.before).ok()?;
                            let last = edit_log.map_range(last, delta.before).ok()?;
                            Some(first.range.start.0..last.range.end.0)
                        });
                let Some(envelope) = envelope else {
                    affected_start = affected_start.min(offset);
                    affected_end = affected_end.max(offset + chunk_len);
                    edit_found = true;
                    offset += chunk_len;
                    continue;
                };
                if edit.old.end.0 < envelope.start {
                    affected_start = affected_start.min(offset);
                    edit_found = true;
                    break;
                }
                if edit.old.start.0 > envelope.end {
                    offset += chunk_len;
                    continue;
                }
                for (local, row) in chunk.rows.iter().enumerate() {
                    let Ok(mapped) = edit_log.map_range(row.source, delta.before) else {
                        affected_start = affected_start.min(offset + local);
                        affected_end = affected_end.max(offset + local + 1);
                        edit_found = true;
                        continue;
                    };
                    let range = mapped.range;
                    if edit_intersects_range(edit, range) {
                        affected_start = affected_start.min(offset + local);
                        affected_end = affected_end.max(offset + local + 1);
                        edit_found = true;
                    } else if !edit_found && edit.old.start <= range.start {
                        affected_start = affected_start.min(offset + local);
                        edit_found = true;
                        break;
                    }
                }
                offset += chunk_len;
                if edit_found {
                    break;
                }
            }
            if !edit_found {
                affected_start = affected_start.min(self.rows.len());
            }
        }
        if affected_start == self.rows.len() && affected_end == 0 {
            return Ok(self.rows.len()..self.rows.len());
        }
        Ok(affected_start..affected_end.max(affected_start))
    }

    pub(in crate::preview) fn patch_plan(
        &self,
        deltas: &[RevisionDelta],
    ) -> Result<VisualPatchPlan, ProjectionPatchError> {
        let Some(first) = deltas.first() else {
            return Err(ProjectionPatchError::StaleSnapshot);
        };
        if self.revision != first.before
            || deltas
                .windows(2)
                .any(|pair| pair[0].after != pair[1].before)
        {
            return Err(ProjectionPatchError::StaleSnapshot);
        }
        let mut edit_log = (*self.edit_log).clone();
        let mut affected_start = self.rows.len();
        let mut affected_end = 0;
        for delta in deltas {
            let next = self.affected_visual_range_with_log(delta, &edit_log)?;
            affected_start = affected_start.min(next.start);
            affected_end = affected_end.max(next.end);
            edit_log
                .push(delta.clone())
                .map_err(|_| ProjectionPatchError::StaleSnapshot)?;
        }
        let affected = if affected_start == self.rows.len() && affected_end == 0 {
            self.rows.len()..self.rows.len()
        } else {
            affected_start..affected_end.max(affected_start)
        };
        Ok(VisualPatchPlan {
            before_revision: first.before,
            after_revision: deltas
                .last()
                .expect("a patch plan has at least one delta")
                .after,
            affected,
        })
    }

    #[cfg(test)]
    pub(in crate::preview) fn apply_patch(
        &self,
        delta: &RevisionDelta,
        old_visual: Range<usize>,
        replacements: Vec<VisualRow>,
    ) -> Result<(Self, VisualPatch), ProjectionPatchError> {
        self.apply_patch_chain(std::slice::from_ref(delta), old_visual, replacements)
    }

    #[cfg(test)]
    pub(in crate::preview) fn apply_patch_chain(
        &self,
        deltas: &[RevisionDelta],
        old_visual: Range<usize>,
        replacements: Vec<VisualRow>,
    ) -> Result<(Self, VisualPatch), ProjectionPatchError> {
        let plan = self.patch_plan(deltas)?;
        self.apply_patch_chain_with_plan(deltas, old_visual, replacements, plan)
    }

    pub(in crate::preview) fn apply_patch_chain_with_plan(
        &self,
        deltas: &[RevisionDelta],
        old_visual: Range<usize>,
        replacements: Vec<VisualRow>,
        plan: VisualPatchPlan,
    ) -> Result<(Self, VisualPatch), ProjectionPatchError> {
        let Some(first) = deltas.first() else {
            return Err(ProjectionPatchError::StaleSnapshot);
        };
        let Some(last) = deltas.last() else {
            return Err(ProjectionPatchError::StaleSnapshot);
        };
        if self.revision != first.before
            || plan.before_revision != first.before
            || plan.after_revision != last.after
            || deltas
                .windows(2)
                .any(|pair| pair[0].after != pair[1].before)
        {
            return Err(ProjectionPatchError::StaleSnapshot);
        }
        if old_visual.start > old_visual.end || old_visual.end > self.rows.len() {
            return Err(ProjectionPatchError::InvalidVisualRange);
        }
        if replacements
            .iter()
            .any(|row| row.source.revision != last.after)
        {
            return Err(ProjectionPatchError::ReplacementRevision);
        }
        if old_visual.start > plan.affected.start || old_visual.end < plan.affected.end {
            return Err(ProjectionPatchError::UncoveredChangedRow);
        }
        let rebased;
        let base = if self.edit_log.can_append_without_expiring(deltas.len()) {
            self
        } else {
            rebased = self.rebased()?;
            &rebased
        };
        let mut edit_log = (*base.edit_log).clone();
        for delta in deltas {
            edit_log
                .push(delta.clone())
                .map_err(|_| ProjectionPatchError::StaleSnapshot)?;
        }
        let old_rows = old_visual
            .clone()
            .filter_map(|index| base.rows.get(index))
            .collect::<Vec<_>>();
        let invalidates_geometry = old_rows.len() != replacements.len()
            || old_rows
                .iter()
                .zip(&replacements)
                .any(|(old, new)| !geometry_compatible(old, new));
        let invalidation = if invalidates_geometry {
            InvalidationFlags::CONTENT.union(InvalidationFlags::GEOMETRY)
        } else {
            InvalidationFlags::CONTENT.union(InvalidationFlags::PAINT)
        };
        let new_start = old_visual.start;
        let new_end = new_start + replacements.len();
        let presentation = base.presentation.replace(old_visual.clone(), &replacements);
        Ok((
            Self {
                revision: last.after,
                revisions: VisualRevisions {
                    geometry: self
                        .revisions
                        .geometry
                        .wrapping_add(u64::from(invalidates_geometry)),
                    paint: self.revisions.paint.wrapping_add(1),
                },
                rows: base.rows.replace(old_visual.clone(), replacements),
                presentation,
                edit_log: Arc::new(edit_log),
            },
            VisualPatch {
                before_revision: first.before,
                after_revision: last.after,
                old_visual,
                new_visual: new_start..new_end,
                invalidation,
            },
        ))
    }

    pub(in crate::preview) fn replacement_rows(
        &self,
        range: Range<usize>,
    ) -> Option<Vec<VisualRow>> {
        (range.start <= range.end && range.end <= self.rows.len()).then(|| {
            range
                .filter_map(|index| self.rows.get(index).cloned())
                .collect()
        })
    }

    pub(in crate::preview) fn shared_chunk_count(&self, other: &Self) -> usize {
        let other_chunks = other
            .rows
            .chunks
            .iter()
            .map(Arc::as_ptr)
            .collect::<HashSet<_>>();
        self.rows
            .chunks
            .iter()
            .filter(|chunk| other_chunks.contains(&Arc::as_ptr(chunk)))
            .count()
    }

    pub(in crate::preview) fn chunk_count(&self) -> usize {
        self.rows.chunks.len()
    }
}

fn edit_intersects_range(
    edit: crate::document::TextEditSummary,
    range: crate::document::ByteRange,
) -> bool {
    if edit.old.start == edit.old.end {
        edit.old.start > range.start && edit.old.start < range.end
    } else {
        edit.old.start < range.end && edit.old.end > range.start
    }
}

pub(in crate::preview) fn build_projection_snapshot(
    text: &dyn TextSnapshot,
    revision: Revision,
    format: DocumentFormat,
    source_rows: Arc<Vec<PreviewRow>>,
    blocks: &BlockArena,
    markdown_blocks: &[MarkdownBlock],
    resources: ReadingProjectionResources<'_>,
) -> Arc<ReadingProjection> {
    let rows = build_visual_rows(
        text,
        revision,
        format,
        &source_rows,
        blocks,
        markdown_blocks,
        resources,
    );
    let rows = VisualRowTree::from_rows(rows);
    Arc::new(ReadingProjection {
        revision,
        revisions: VisualRevisions::default(),
        presentation: PresentationTree::from_visual_rows(&rows),
        edit_log: Arc::new(EditLog::new(256).expect("projection edit log capacity is non-zero")),
        rows,
    })
}

pub(in crate::preview) fn build_visual_rows(
    text: &dyn TextSnapshot,
    revision: Revision,
    format: DocumentFormat,
    source_rows: &[PreviewRow],
    blocks: &BlockArena,
    markdown_blocks: &[MarkdownBlock],
    resources: ReadingProjectionResources<'_>,
) -> Vec<VisualRow> {
    source_rows
        .iter()
        .map(|row| {
            let kind = visual_kind(text, format, row, blocks, markdown_blocks, resources);
            let style_kind =
                visual_row_style_kind(format, &kind, row.block_id, blocks, markdown_blocks);
            VisualRow {
                id: next_visual_row_id(),
                source: row.content,
                block_id: row.block_id,
                semantic_revision: revision.0,
                kind,
                code_language: visual_code_language(format, row.block_id, blocks, markdown_blocks),
                code_action_range: code_action_range(format, row.block_id, blocks, markdown_blocks),
                style_kind,
                render: *row,
            }
        })
        .collect()
}

fn visual_row_style_kind(
    format: DocumentFormat,
    visual: &VisualRowKind,
    block_id: BlockId,
    blocks: &BlockArena,
    markdown_blocks: &[MarkdownBlock],
) -> RowStyleKind {
    match visual {
        VisualRowKind::Caption => return RowStyleKind::Caption,
        VisualRowKind::Code(ReadingCodeRow::End) | VisualRowKind::Hidden => {
            return RowStyleKind::Hidden;
        }
        _ => {}
    }
    match format {
        DocumentFormat::Org => match &blocks.nodes()[block_id as usize].kind {
            BlockKind::Heading { level } => {
                RowStyleKind::Heading((*level).min(u8::MAX as u16) as u8)
            }
            BlockKind::BlankLine => RowStyleKind::Blank,
            BlockKind::Paragraph => RowStyleKind::Paragraph,
            BlockKind::ListItem => RowStyleKind::List,
            BlockKind::Keyword => RowStyleKind::Keyword,
            BlockKind::Image { .. } => RowStyleKind::Image,
            BlockKind::Planning | BlockKind::FixedWidth | BlockKind::FootnoteDefinition => {
                RowStyleKind::Metadata
            }
            BlockKind::SourceBlock { .. } => RowStyleKind::Code,
            BlockKind::ExampleBlock | BlockKind::Raw | BlockKind::ExportBlock { .. } => {
                RowStyleKind::RawCode
            }
            BlockKind::QuoteBlock => RowStyleKind::Quote,
            BlockKind::VerseBlock => RowStyleKind::Verse,
            BlockKind::CenterBlock => RowStyleKind::Center,
            BlockKind::SpecialBlock { .. } => RowStyleKind::Special,
            BlockKind::Drawer { .. } => RowStyleKind::Drawer,
            BlockKind::HorizontalRule => RowStyleKind::Rule,
            BlockKind::Comment | BlockKind::CommentBlock => RowStyleKind::Hidden,
            BlockKind::TableRow => RowStyleKind::Table,
        },
        DocumentFormat::Markdown => match &markdown_blocks[block_id as usize].kind {
            MarkdownKind::Heading { level } => {
                RowStyleKind::Heading((*level).min(u8::MAX as u16) as u8)
            }
            MarkdownKind::Blank => RowStyleKind::Blank,
            MarkdownKind::Paragraph => RowStyleKind::Paragraph,
            MarkdownKind::ListItem => RowStyleKind::List,
            MarkdownKind::Quote => RowStyleKind::CompactQuote,
            MarkdownKind::Code { role, .. } => match role {
                crate::preview::CodeRowRole::Close => RowStyleKind::Hidden,
                _ => RowStyleKind::Code,
            },
            MarkdownKind::TableRow => RowStyleKind::Table,
            MarkdownKind::HorizontalRule => RowStyleKind::Rule,
            MarkdownKind::Image { .. } => RowStyleKind::Image,
        },
    }
}

fn code_action_range(
    format: DocumentFormat,
    block_id: BlockId,
    blocks: &BlockArena,
    markdown_blocks: &[MarkdownBlock],
) -> Option<crate::document::ByteRange> {
    match format {
        DocumentFormat::Org => match blocks.nodes().get(block_id as usize)?.kind {
            BlockKind::SourceBlock { .. } | BlockKind::ExampleBlock => {
                Some(blocks.nodes()[block_id as usize].content)
            }
            _ => None,
        },
        DocumentFormat::Markdown => {
            let block_index = block_id as usize;
            if !matches!(
                markdown_blocks.get(block_index)?.kind,
                MarkdownKind::Code {
                    role: crate::preview::CodeRowRole::Open,
                    ..
                }
            ) {
                return None;
            }
            let mut body = markdown_blocks
                .iter()
                .skip(block_index + 1)
                .take_while(|block| {
                    matches!(
                        block.kind,
                        MarkdownKind::Code {
                            role: crate::preview::CodeRowRole::Body,
                            ..
                        }
                    )
                })
                .map(|block| block.source);
            let first = body.next().unwrap_or_else(|| {
                let end = markdown_blocks[block_index].source.end.0;
                crate::document::ByteRange::new(end, end)
            });
            Some(body.fold(first, |range, next| {
                crate::document::ByteRange::new(range.start.0, next.end.0)
            }))
        }
    }
}

fn geometry_compatible(old: &VisualRow, new: &VisualRow) -> bool {
    if old.style_kind != new.style_kind {
        return false;
    }
    match (&old.kind, &new.kind) {
        (VisualRowKind::List(old), VisualRowKind::List(new)) => {
            old.marker == new.marker
                && old.indent == new.indent
                && old.checkbox.is_some() == new.checkbox.is_some()
                && old.text_signature == new.text_signature
        }
        (VisualRowKind::Table(old), VisualRowKind::Table(new)) => {
            old.table().same_geometry(new.table())
        }
        (
            VisualRowKind::Image {
                dimensions: old_dimensions,
            },
            VisualRowKind::Image {
                dimensions: new_dimensions,
            },
        ) => old_dimensions == new_dimensions,
        (VisualRowKind::Rule, VisualRowKind::Rule)
        | (VisualRowKind::Blank, VisualRowKind::Blank)
        | (VisualRowKind::Hidden, VisualRowKind::Hidden) => true,
        _ => false,
    }
}

fn visual_kind(
    text: &dyn TextSnapshot,
    format: DocumentFormat,
    row: &PreviewRow,
    blocks: &BlockArena,
    markdown_blocks: &[MarkdownBlock],
    resources: ReadingProjectionResources<'_>,
) -> VisualRowKind {
    let block_id = row.block_id;
    if let Some(table) = resources.tables.get(&block_id) {
        return VisualRowKind::Table(table.clone());
    }
    if let Some(&(width, height)) = resources.images.get(&block_id) {
        return VisualRowKind::Image {
            dimensions: Some((width, height)),
        };
    }
    match format {
        DocumentFormat::Org => match blocks.nodes()[block_id as usize].kind {
            BlockKind::Heading { level } => VisualRowKind::Heading(level.min(4) as u8),
            BlockKind::ListItem => list_visual_kind(&text.copy_range(row.content.range)),
            BlockKind::Keyword
                if text
                    .copy_range(row.content.range)
                    .trim_start()
                    .get(.."#+caption:".len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("#+caption:")) =>
            {
                VisualRowKind::Caption
            }
            BlockKind::QuoteBlock => VisualRowKind::Quote,
            BlockKind::Image { .. } => VisualRowKind::Image { dimensions: None },
            BlockKind::SourceBlock { .. } | BlockKind::ExampleBlock => {
                VisualRowKind::Code(ReadingCodeRow::Body)
            }
            BlockKind::BlankLine => VisualRowKind::Blank,
            BlockKind::HorizontalRule => VisualRowKind::Rule,
            BlockKind::Comment | BlockKind::CommentBlock => VisualRowKind::Hidden,
            _ => VisualRowKind::Text,
        },
        DocumentFormat::Markdown => match markdown_blocks[block_id as usize].kind {
            MarkdownKind::Heading { level } => VisualRowKind::Heading(level.min(4) as u8),
            MarkdownKind::ListItem => list_visual_kind(&text.copy_range(row.content.range)),
            MarkdownKind::Quote => VisualRowKind::Quote,
            MarkdownKind::Image { .. } => VisualRowKind::Image { dimensions: None },
            MarkdownKind::Code { role, .. } => VisualRowKind::Code(match role {
                crate::preview::CodeRowRole::Open => ReadingCodeRow::Start,
                crate::preview::CodeRowRole::Body => ReadingCodeRow::Body,
                crate::preview::CodeRowRole::Close => ReadingCodeRow::End,
            }),
            MarkdownKind::Blank => VisualRowKind::Blank,
            MarkdownKind::HorizontalRule => VisualRowKind::Rule,
            _ => VisualRowKind::Text,
        },
    }
}

fn list_visual_kind(source: &str) -> VisualRowKind {
    let parts = parse_list_item(source);
    let mut text_hasher = std::collections::hash_map::DefaultHasher::new();
    parts.term.hash(&mut text_hasher);
    parts.body.hash(&mut text_hasher);
    VisualRowKind::List(ReadingListMarker {
        marker: Arc::from(parts.marker),
        checkbox: parts.checkbox,
        indent: reading_indent(&parts.indent),
        text_signature: text_hasher.finish(),
    })
}

fn reading_indent(indent: &str) -> u16 {
    indent
        .bytes()
        .fold(0usize, |column, byte| {
            if byte == b'\t' {
                (column / 4 + 1) * 4
            } else {
                column + 1
            }
        })
        .min(u16::MAX as usize) as u16
}

fn visual_code_language(
    format: DocumentFormat,
    block_id: BlockId,
    blocks: &BlockArena,
    markdown_blocks: &[MarkdownBlock],
) -> Option<Arc<str>> {
    match format {
        DocumentFormat::Org => match &blocks.nodes()[block_id as usize].kind {
            BlockKind::SourceBlock { language } => language.as_deref().map(Arc::from),
            _ => None,
        },
        DocumentFormat::Markdown => match &markdown_blocks[block_id as usize].kind {
            MarkdownKind::Code { language, .. } => language.as_deref().map(Arc::from),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ByteRange, TextEditSummary};
    use crate::preview::table::test_table_projection;

    fn row(index: usize, revision: Revision) -> VisualRow {
        VisualRow {
            id: VisualRowId(index as u64),
            source: RevisionRange::new(
                revision,
                ByteRange::new(index as u64 * 10, index as u64 * 10 + 5),
            ),
            block_id: index as u32,
            semantic_revision: revision.0,
            kind: VisualRowKind::Text,
            code_language: None,
            code_action_range: None,
            style_kind: RowStyleKind::Paragraph,
            render: PreviewRow {
                block_id: index as u32,
                content: RevisionRange::new(
                    revision,
                    ByteRange::new(index as u64 * 10, index as u64 * 10 + 5),
                ),
                continuation: false,
                blank: false,
            },
        }
    }

    #[test]
    fn local_replacement_reuses_untouched_chunks_and_row_ids() {
        let tree =
            VisualRowTree::from_rows((0..300).map(|index| row(index, Revision(0))).collect());
        let first_chunk = tree.chunks()[0].clone();
        let last_chunk = tree.chunks()[2].clone();
        let replacement = VisualRow {
            id: VisualRowId(10_000),
            ..row(140, Revision(1))
        };
        let replaced = tree.replace(140..141, vec![replacement]);

        assert!(Arc::ptr_eq(&first_chunk, &replaced.chunks()[0]));
        assert!(Arc::ptr_eq(
            &last_chunk,
            replaced.chunks().last().expect("last chunk")
        ));
        assert_eq!(replaced.get(139).unwrap().id, VisualRowId(139));
        assert_eq!(replaced.get(140).unwrap().id, VisualRowId(10_000));
        assert_eq!(replaced.get(141).unwrap().id, VisualRowId(141));
    }

    #[test]
    fn patch_rejects_stale_and_incompletely_covered_changes() {
        let snapshot = ReadingProjection {
            revision: Revision(0),
            revisions: VisualRevisions::default(),
            rows: VisualRowTree::from_rows((0..3).map(|index| row(index, Revision(0))).collect()),
            presentation: PresentationTree::from_visual_rows(&VisualRowTree::from_rows(
                (0..3).map(|index| row(index, Revision(0))).collect(),
            )),
            edit_log: Arc::new(EditLog::new(256).unwrap()),
        };
        let delta = RevisionDelta::new(
            Revision(0),
            Revision(1),
            vec![TextEditSummary::new(ByteRange::new(11, 12), 1)],
        )
        .unwrap();
        assert!(matches!(
            snapshot.apply_patch(&delta, 0..1, vec![row(0, Revision(1))]),
            Err(ProjectionPatchError::UncoveredChangedRow)
        ));
        let stale = RevisionDelta::new(Revision(1), Revision(2), Vec::new()).unwrap();
        assert!(matches!(
            snapshot.apply_patch(&stale, 0..0, Vec::new()),
            Err(ProjectionPatchError::StaleSnapshot)
        ));
    }

    #[test]
    fn patch_advances_snapshot_and_reports_only_the_replaced_visual_range() {
        let snapshot = ReadingProjection {
            revision: Revision(0),
            revisions: VisualRevisions::default(),
            rows: VisualRowTree::from_rows((0..3).map(|index| row(index, Revision(0))).collect()),
            presentation: PresentationTree::from_visual_rows(&VisualRowTree::from_rows(
                (0..3).map(|index| row(index, Revision(0))).collect(),
            )),
            edit_log: Arc::new(EditLog::new(256).unwrap()),
        };
        let delta = RevisionDelta::new(
            Revision(0),
            Revision(1),
            vec![TextEditSummary::new(ByteRange::new(11, 12), 2)],
        )
        .unwrap();
        let replacement = VisualRow {
            id: VisualRowId(20),
            source: RevisionRange::new(Revision(1), ByteRange::new(10, 16)),
            block_id: 1,
            semantic_revision: 1,
            kind: VisualRowKind::Text,
            code_language: None,
            code_action_range: None,
            style_kind: RowStyleKind::Paragraph,
            render: row(1, Revision(1)).render,
        };
        let (next, patch) = snapshot
            .apply_patch(&delta, 1..2, vec![replacement])
            .unwrap();
        assert_eq!(next.revision, Revision(1));
        assert_eq!(next.presentation.len(), 3);
        assert_eq!(patch.old_visual, 1..2);
        assert_eq!(patch.new_visual, 1..2);
        assert_eq!(next.rows.get(0).unwrap().id, VisualRowId(0));
        assert_eq!(next.rows.get(2).unwrap().id, VisualRowId(2));
        assert_eq!(
            next.source_row(2).unwrap().content.range,
            ByteRange::new(21, 26)
        );

        let second_delta = RevisionDelta::new(
            Revision(1),
            Revision(2),
            vec![TextEditSummary::new(ByteRange::new(0, 0), 3)],
        )
        .unwrap();
        let (latest, _) = next.apply_patch(&second_delta, 0..0, Vec::new()).unwrap();
        assert_eq!(
            latest.source_row(2).unwrap().content.range,
            ByteRange::new(24, 29)
        );
    }

    #[test]
    fn table_content_patch_preserves_geometry_when_columns_do_not_change() {
        let mut old = row(0, Revision(0));
        old.kind = VisualRowKind::Table(test_table_projection());
        let rows = VisualRowTree::from_rows(vec![old]);
        let snapshot = ReadingProjection {
            revision: Revision(0),
            revisions: VisualRevisions::default(),
            presentation: PresentationTree::from_visual_rows(&rows),
            edit_log: Arc::new(EditLog::new(256).unwrap()),
            rows,
        };
        let delta = RevisionDelta::new(
            Revision(0),
            Revision(1),
            vec![TextEditSummary::new(ByteRange::new(2, 3), 1)],
        )
        .unwrap();
        let mut replacement = row(0, Revision(1));
        replacement.kind = VisualRowKind::Table(test_table_projection());
        let (next, patch) = snapshot
            .apply_patch(&delta, 0..1, vec![replacement])
            .unwrap();
        assert_eq!(next.revisions.geometry, snapshot.revisions.geometry);
        assert_eq!(patch.invalidation.0 & InvalidationFlags::GEOMETRY.0, 0);
    }

    #[test]
    fn projection_rebases_before_bounded_history_expires() {
        let rows = VisualRowTree::from_rows((0..3).map(|index| row(index, Revision(0))).collect());
        let mut snapshot = ReadingProjection {
            revision: Revision(0),
            revisions: VisualRevisions::default(),
            presentation: PresentationTree::from_visual_rows(&rows),
            edit_log: Arc::new(EditLog::new(256).unwrap()),
            rows,
        };
        for revision in 0..300 {
            let delta = RevisionDelta::new(
                Revision(revision),
                Revision(revision + 1),
                vec![TextEditSummary::new(ByteRange::new(1, 1), 1)],
            )
            .unwrap();
            let replacement = VisualRow {
                source: RevisionRange::new(Revision(revision + 1), ByteRange::new(0, 5)),
                semantic_revision: revision + 1,
                ..row(0, Revision(revision + 1))
            };
            snapshot = snapshot
                .apply_patch(&delta, 0..1, vec![replacement])
                .unwrap()
                .0;
        }
        assert_eq!(snapshot.revision, Revision(300));
        assert_eq!(
            snapshot.source_row(2).unwrap().content.range,
            ByteRange::new(320, 325)
        );
    }
}
