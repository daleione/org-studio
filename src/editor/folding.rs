use crate::document::{
    ByteOffset, DocumentFormat, DocumentId, DocumentSnapshot, HeadingIndex, LineCursor, LineIndex,
    LocalVisibility, OutlineHeading, Revision, RevisionDelta, RevisionRange, TextSnapshot,
    global_outline_visibility, next_local_visibility,
};
use std::{cell::RefCell, collections::HashSet, path::Path, sync::Arc};

pub(super) use crate::document::GlobalVisibility;

#[derive(Default)]
pub(super) struct EditorFoldState {
    headings: Vec<FoldedHeading>,
    blocks: Vec<FoldedBlock>,
    pub(super) global: GlobalVisibility,
    heading_cache: RefCell<Option<CachedHeadingIndex>>,
}

struct FoldedHeading {
    source: RevisionRange,
    visibility: LocalVisibility,
}

struct FoldedBlock {
    source: RevisionRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BlockRegion {
    source: crate::document::ByteRange,
    start_line: u64,
    end_line: u64,
}

struct CachedHeadingIndex {
    document_id: DocumentId,
    revision: Revision,
    format: DocumentFormat,
    index: Arc<HeadingIndex>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct EditorFoldProjection {
    pub(super) hidden_ranges: Vec<std::ops::Range<u64>>,
    pub(super) marker_lines: HashSet<u64>,
}

impl EditorFoldState {
    pub(super) fn toggle_heading(
        &mut self,
        path: &Path,
        snapshot: &DocumentSnapshot,
        line: u64,
    ) -> bool {
        let Ok(range) = snapshot.line_content_range(LineIndex(line)) else {
            return false;
        };
        let index = self.heading_index(path, snapshot);
        let Some((heading_index, heading)) = index
            .as_slice()
            .iter()
            .copied()
            .enumerate()
            .find(|(_, heading)| heading.start == range.start)
        else {
            return false;
        };
        let subtree_end = heading_subtree_end(
            semantic_line_count(snapshot),
            index.as_slice(),
            heading_index,
        );
        if subtree_end == heading.line + 1 {
            return false;
        }
        let existing = self
            .headings
            .iter()
            .position(|heading| heading.source.range.start == range.start);
        let has_children =
            !direct_child_indices(index.as_slice(), heading_index, subtree_end).is_empty();
        let current = if self.global == GlobalVisibility::All {
            existing.map(|index| self.headings[index].visibility)
        } else {
            let projection = self.projection(path, snapshot);
            if projection
                .hidden_ranges
                .iter()
                .any(|hidden| hidden.start <= heading.line + 1 && hidden.end >= subtree_end)
            {
                Some(LocalVisibility::Folded)
            } else if projection
                .hidden_ranges
                .iter()
                .any(|hidden| hidden.start < subtree_end && hidden.end > heading.line + 1)
            {
                Some(LocalVisibility::Children)
            } else {
                None
            }
        };
        self.set_heading_visibility(
            snapshot,
            range,
            next_local_visibility(current, has_children),
        );
        true
    }

    fn set_heading_visibility(
        &mut self,
        snapshot: &DocumentSnapshot,
        range: crate::document::ByteRange,
        visibility: LocalVisibility,
    ) {
        self.headings
            .retain(|heading| heading.source.range.start != range.start);
        // An expanded subtree is an explicit exception to a global folded view.
        if visibility != LocalVisibility::Subtree || self.global != GlobalVisibility::All {
            self.headings.push(FoldedHeading {
                source: snapshot.revision_range(range),
                visibility,
            });
        }
    }

    /// Toggle an Org begin/end block or Markdown fence only from its opening boundary. Tabs inside
    /// the body remain normal editing input, especially for indentation-sensitive source blocks.
    pub(super) fn toggle_block(
        &mut self,
        path: &Path,
        snapshot: &DocumentSnapshot,
        line: u64,
    ) -> bool {
        if !is_block_boundary_candidate(path, snapshot, line) {
            return false;
        }
        let Some(region) = block_regions(path, snapshot)
            .into_iter()
            .find(|region| region.start_line == line && region.end_line > line + 1)
        else {
            return false;
        };
        if let Some(index) = self
            .blocks
            .iter()
            .position(|block| block.source.range.start == region.source.start)
        {
            self.blocks.remove(index);
        } else {
            self.blocks.push(FoldedBlock {
                source: snapshot.revision_range(region.source),
            });
        }
        true
    }

    pub(super) fn apply_delta(&mut self, delta: &RevisionDelta) {
        self.heading_cache.take();
        self.headings = self
            .headings
            .drain(..)
            .filter_map(|heading| {
                delta
                    .map_range(heading.source)
                    .ok()
                    .map(|source| FoldedHeading {
                        source,
                        visibility: heading.visibility,
                    })
            })
            .collect();
        self.blocks = self
            .blocks
            .drain(..)
            .filter_map(|block| {
                delta
                    .map_range(block.source)
                    .ok()
                    .map(|source| FoldedBlock { source })
            })
            .collect();
    }

    pub(super) fn cycle_global(&mut self, path: &Path, snapshot: &DocumentSnapshot) -> bool {
        if self.heading_index(path, snapshot).is_empty() {
            self.headings.clear();
            self.global = GlobalVisibility::All;
            return false;
        }
        self.headings.clear();
        self.global = self.global.next();
        true
    }

    pub(super) fn expand_at(&mut self, path: &Path, snapshot: &DocumentSnapshot, line: u64) {
        if let Ok(range) = snapshot.line_content_range(LineIndex(line)) {
            if self
                .heading_index(path, snapshot)
                .as_slice()
                .iter()
                .any(|heading| heading.start == range.start)
            {
                self.set_heading_visibility(snapshot, range, LocalVisibility::Subtree);
            }
            self.blocks
                .retain(|block| block.source.range.start != range.start);
        }
    }

    pub(super) fn projection(
        &self,
        path: &Path,
        snapshot: &DocumentSnapshot,
    ) -> EditorFoldProjection {
        self.projection_revealing(path, snapshot, None)
    }

    pub(super) fn projection_revealing(
        &self,
        path: &Path,
        snapshot: &DocumentSnapshot,
        reveal: Option<std::ops::Range<u64>>,
    ) -> EditorFoldProjection {
        if self.global == GlobalVisibility::All
            && self.headings.is_empty()
            && self.blocks.is_empty()
        {
            return EditorFoldProjection::default();
        }
        let index = self.heading_index(path, snapshot);
        let headings = index.as_slice();
        if headings.is_empty() && self.blocks.is_empty() {
            return EditorFoldProjection::default();
        }
        let mut hidden = Vec::new();
        let mut markers = HashSet::new();
        let semantic_lines = semantic_line_count(snapshot);
        if self.global != GlobalVisibility::All {
            let outline = outline_headings(headings);
            let (visible, global_markers) =
                global_outline_visibility(semantic_lines as usize, &outline, self.global);
            hidden = complement(snapshot.len_lines(), &visible);
            markers.extend(global_markers.into_iter().map(|line| line as u64));
        }
        for folded in &self.headings {
            if let Some((index, heading)) = headings
                .iter()
                .enumerate()
                .find(|(_, heading)| heading.start == folded.source.range.start)
            {
                let end = heading_subtree_end(semantic_lines, headings, index);
                if self.global != GlobalVisibility::All {
                    // Apply local exceptions in action order, so a child can be opened
                    // after revealing its parent without expanding neighboring subtrees.
                    let body = heading.line + 1..end;
                    hidden = subtract_ranges(&hidden, std::slice::from_ref(&body));
                    markers.retain(|line| *line < heading.line || *line >= end);
                }
                if heading.line + 1 < end {
                    match folded.visibility {
                        LocalVisibility::Folded => {
                            hidden.push(heading.line + 1..end);
                            markers.insert(heading.line);
                        }
                        LocalVisibility::Children => {
                            let entry_end = headings.get(index + 1).map_or(end, |next| next.line);
                            let mut cursor = entry_end;
                            for child_index in direct_child_indices(headings, index, end) {
                                let child = headings[child_index];
                                if cursor < child.line {
                                    hidden.push(cursor..child.line);
                                }
                                let child_end =
                                    heading_subtree_end(semantic_lines, headings, child_index);
                                if child_end > child.line + 1 {
                                    markers.insert(child.line);
                                }
                                cursor = child.line + 1;
                            }
                            if cursor < end {
                                hidden.push(cursor..end);
                            }
                        }
                        LocalVisibility::Subtree => {}
                        LocalVisibility::Empty => unreachable!(),
                    }
                }
            }
        }
        if !self.blocks.is_empty() {
            let regions = block_regions(path, snapshot);
            for folded in &self.blocks {
                if let Some(region) = regions
                    .iter()
                    .find(|region| region.source.start == folded.source.range.start)
                    && region.start_line + 1 < region.end_line
                {
                    hidden.push(region.start_line + 1..region.end_line);
                    markers.insert(region.start_line);
                }
            }
        }
        if let Some(reveal) = reveal {
            // Remove containing folds before merging, preserving unrelated nested folds.
            hidden.retain(|range| range.end <= reveal.start || range.start >= reveal.end);
        }
        hidden.sort_by_key(|range| range.start);
        let hidden_ranges = merge(hidden);
        markers.retain(|line| !hidden_ranges.iter().any(|range| range.contains(line)));
        EditorFoldProjection {
            hidden_ranges,
            marker_lines: markers,
        }
    }

    #[cfg(test)]
    pub(super) fn hidden_ranges(
        &self,
        path: &Path,
        snapshot: &DocumentSnapshot,
    ) -> Vec<std::ops::Range<u64>> {
        self.projection(path, snapshot).hidden_ranges
    }

    pub(super) fn heading_index(
        &self,
        path: &Path,
        snapshot: &DocumentSnapshot,
    ) -> Arc<HeadingIndex> {
        if crate::syntax_highlighting::language_for_path(path).is_some() {
            return Arc::new(HeadingIndex::default());
        }
        let format = DocumentFormat::from_path(path);
        if let Some(cached) = self.heading_cache.borrow().as_ref()
            && cached.document_id == snapshot.document_id()
            && cached.revision == snapshot.revision()
            && cached.format == format
        {
            return cached.index.clone();
        }
        let index = Arc::new(HeadingIndex::parse(format, snapshot));
        *self.heading_cache.borrow_mut() = Some(CachedHeadingIndex {
            document_id: snapshot.document_id(),
            revision: snapshot.revision(),
            format,
            index: index.clone(),
        });
        index
    }
}

pub(super) fn is_block_boundary_candidate(
    path: &Path,
    snapshot: &DocumentSnapshot,
    line: u64,
) -> bool {
    let Ok(range) = snapshot.line_content_range(LineIndex(line)) else {
        return false;
    };
    let text = snapshot.copy_range(range);
    let trimmed = text.trim_start();
    match DocumentFormat::detect(path) {
        Some(DocumentFormat::Org) => trimmed
            .get(.."#+begin_".len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("#+begin_")),
        Some(DocumentFormat::Markdown) => crate::document::markdown::fence_open(&text).is_some(),
        None => false,
    }
}

fn block_regions(path: &Path, snapshot: &DocumentSnapshot) -> Vec<BlockRegion> {
    match DocumentFormat::detect(path) {
        Some(DocumentFormat::Org) => org_block_regions(snapshot),
        Some(DocumentFormat::Markdown) => markdown_fenced_block_regions(snapshot),
        None => Vec::new(),
    }
}

fn org_block_regions(snapshot: &DocumentSnapshot) -> Vec<BlockRegion> {
    crate::org_syntax::parse(snapshot)
        .nodes()
        .iter()
        .filter(|node| is_foldable_org_block(&node.kind))
        .filter_map(|node| block_region(snapshot, node.source))
        .collect()
}

fn is_foldable_org_block(kind: &crate::org_syntax::BlockKind) -> bool {
    use crate::org_syntax::BlockKind;
    matches!(
        kind,
        BlockKind::SourceBlock { .. }
            | BlockKind::ExampleBlock
            | BlockKind::QuoteBlock
            | BlockKind::VerseBlock
            | BlockKind::CenterBlock
            | BlockKind::CommentBlock
            | BlockKind::ExportBlock { .. }
            | BlockKind::SpecialBlock { .. }
    )
}

fn markdown_fenced_block_regions(snapshot: &DocumentSnapshot) -> Vec<BlockRegion> {
    let mut cursor = LineCursor::new(snapshot);
    let mut open: Option<(char, usize, crate::document::ByteRange, u64)> = None;
    let mut regions = Vec::new();
    let mut line_number = 0_u64;
    while let Some(line) = cursor.next_line() {
        let logical = line.text.trim_end_matches(['\r', '\n']);
        if let Some((marker, count, source, start_line)) = open {
            if crate::document::markdown::fence_close(logical, marker, count) {
                regions.push(BlockRegion {
                    source,
                    start_line,
                    end_line: line_number + 1,
                });
                open = None;
            }
        } else if let Some((marker, count, _)) = crate::document::markdown::fence_open(logical) {
            let source = snapshot
                .line_content_range(LineIndex(line_number))
                .unwrap_or(line.range);
            open = Some((marker, count, source, line_number));
        }
        line_number += 1;
    }
    if let Some((_, _, source, start_line)) = open {
        regions.push(BlockRegion {
            source,
            start_line,
            end_line: snapshot.len_lines(),
        });
    }
    regions
}

fn block_region(
    snapshot: &DocumentSnapshot,
    source: crate::document::ByteRange,
) -> Option<BlockRegion> {
    let start_line = snapshot.line_of_byte(source.start);
    let last_byte = source.end.0.saturating_sub(1).max(source.start.0);
    let end_line = snapshot
        .line_of_byte(ByteOffset(last_byte))
        .saturating_add(1);
    let source = snapshot.line_content_range(LineIndex(start_line)).ok()?;
    Some(BlockRegion {
        source,
        start_line,
        end_line,
    })
}

fn outline_headings(headings: &[crate::document::DocumentHeading]) -> Vec<OutlineHeading<usize>> {
    headings
        .iter()
        .map(|heading| OutlineHeading {
            position: heading.line as usize,
            id: heading.line as usize,
            level: heading.level,
        })
        .collect()
}

fn heading_subtree_end(
    line_count: u64,
    headings: &[crate::document::DocumentHeading],
    index: usize,
) -> u64 {
    let heading = headings[index];
    headings[index + 1..]
        .iter()
        .find(|candidate| candidate.level <= heading.level)
        .map_or(line_count, |candidate| candidate.line)
}

fn direct_child_indices(
    headings: &[crate::document::DocumentHeading],
    index: usize,
    subtree_end: u64,
) -> Vec<usize> {
    let parent = headings[index];
    let mut stack = vec![parent.level];
    let mut children = Vec::new();
    for (heading_index, heading) in headings
        .iter()
        .enumerate()
        .skip(index + 1)
        .take_while(|(_, heading)| heading.line < subtree_end)
    {
        while stack.last().is_some_and(|level| *level >= heading.level) {
            stack.pop();
        }
        if stack.len() == 1 {
            children.push(heading_index);
        }
        stack.push(heading.level);
    }
    children
}

fn complement(line_count: u64, visible: &[usize]) -> Vec<std::ops::Range<u64>> {
    let mut hidden = Vec::new();
    let mut cursor = 0;
    for line in visible.iter().map(|line| *line as u64) {
        if cursor < line {
            hidden.push(cursor..line);
        }
        cursor = line.saturating_add(1);
    }
    if cursor < line_count {
        hidden.push(cursor..line_count);
    }
    hidden
}

fn semantic_line_count(snapshot: &DocumentSnapshot) -> u64 {
    let lines = snapshot.len_lines();
    if lines == 0 {
        return 0;
    }
    if snapshot
        .line_content_range(LineIndex(lines - 1))
        .is_ok_and(|range| range.is_empty())
    {
        lines - 1
    } else {
        lines
    }
}

pub(super) fn subtract_ranges(
    minuend: &[std::ops::Range<u64>],
    subtrahend: &[std::ops::Range<u64>],
) -> Vec<std::ops::Range<u64>> {
    let mut output = Vec::new();
    for range in minuend {
        let mut cursor = range.start;
        for excluded in subtrahend {
            if excluded.end <= cursor || excluded.start >= range.end {
                continue;
            }
            if cursor < excluded.start {
                output.push(cursor..excluded.start.min(range.end));
            }
            cursor = cursor.max(excluded.end);
            if cursor >= range.end {
                break;
            }
        }
        if cursor < range.end {
            output.push(cursor..range.end);
        }
    }
    output
}

fn merge(ranges: Vec<std::ops::Range<u64>>) -> Vec<std::ops::Range<u64>> {
    let mut merged: Vec<std::ops::Range<u64>> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    #[test]
    fn search_reveal_preserves_nested_sibling_folds() {
        let snapshot = crate::document::DocumentSnapshot::from_utf8(
            b"* Root\n** Left\nneedle\n** Right\nsibling\n".to_vec(),
        )
        .unwrap();
        let path = std::path::Path::new("search.org");
        let mut folds = super::EditorFoldState::default();
        folds.toggle_heading(path, &snapshot, 3);
        folds.toggle_heading(path, &snapshot, 0);
        let original = folds.projection(path, &snapshot);
        let revealed = folds.projection_revealing(path, &snapshot, Some(2..3));
        assert!(
            !revealed
                .hidden_ranges
                .iter()
                .any(|range| range.contains(&2))
        );
        assert!(
            revealed
                .hidden_ranges
                .iter()
                .any(|range| range.contains(&4))
        );
        assert_eq!(folds.projection(path, &snapshot), original);
    }
    use super::*;
    use crate::document::{ByteRange, DocumentBuffer, EditTransaction, TextEdit};

    fn org() -> &'static Path {
        Path::new("test.org")
    }

    fn markdown() -> &'static Path {
        Path::new("test.md")
    }

    #[test]
    fn local_and_global_visibility_hide_only_expected_source_lines() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"intro\n* One\nbody\n** Child\nchild body\n* Two\ntail\n".to_vec(),
        )
        .unwrap();
        let mut folds = EditorFoldState::default();
        folds.toggle_heading(org(), &snapshot, 1);
        assert_eq!(folds.hidden_ranges(org(), &snapshot), vec![2..5]);
        folds.toggle_heading(org(), &snapshot, 1);
        assert_eq!(folds.hidden_ranges(org(), &snapshot), vec![4..5]);
        folds.toggle_heading(org(), &snapshot, 1);
        assert!(folds.hidden_ranges(org(), &snapshot).is_empty());
        folds.toggle_heading(org(), &snapshot, 1);
        assert!(folds.cycle_global(org(), &snapshot));
        assert_eq!(folds.global, GlobalVisibility::Overview);
        assert_eq!(
            folds.hidden_ranges(org(), &snapshot),
            vec![0..1, 2..5, 6..8]
        );
        // After global collapse, Tab opens only the current (second) heading.
        assert!(folds.toggle_heading(org(), &snapshot, 5));
        assert_eq!(
            folds.hidden_ranges(org(), &snapshot),
            vec![0..1, 2..5, 7..8]
        );
        assert!(folds.projection(org(), &snapshot).marker_lines.contains(&1));
        assert!(!folds.projection(org(), &snapshot).marker_lines.contains(&5));
        folds.toggle_heading(org(), &snapshot, 5);
        assert_eq!(
            folds.hidden_ranges(org(), &snapshot),
            vec![0..1, 2..5, 6..8]
        );
        folds.toggle_heading(org(), &snapshot, 1);
        assert_eq!(
            folds.hidden_ranges(org(), &snapshot),
            vec![0..1, 4..5, 6..8]
        );
        folds.toggle_heading(org(), &snapshot, 3);
        assert_eq!(folds.hidden_ranges(org(), &snapshot), vec![0..1, 6..8]);
        folds.toggle_heading(org(), &snapshot, 3);
        assert_eq!(
            folds.hidden_ranges(org(), &snapshot),
            vec![0..1, 4..5, 6..8]
        );
        folds.toggle_heading(org(), &snapshot, 1);
        assert_eq!(folds.hidden_ranges(org(), &snapshot), vec![0..1, 6..8]);
        // The child's old local fold must not override the parent's later expansion.
        folds.toggle_heading(org(), &snapshot, 3);
        assert_eq!(
            folds.hidden_ranges(org(), &snapshot),
            vec![0..1, 4..5, 6..8]
        );
        assert!(folds.cycle_global(org(), &snapshot));
        assert_eq!(folds.global, GlobalVisibility::Contents);
        assert_eq!(
            folds.hidden_ranges(org(), &snapshot),
            vec![0..1, 2..3, 4..5, 6..8]
        );
        folds.toggle_heading(org(), &snapshot, 3);
        assert_eq!(
            folds.hidden_ranges(org(), &snapshot),
            vec![0..1, 2..3, 6..8]
        );
        // Clicking a gutter fold marker must also leave the other heading alone.
        folds.expand_at(org(), &snapshot, 1);
        assert_eq!(folds.hidden_ranges(org(), &snapshot), vec![0..1, 6..8]);
        assert!(folds.cycle_global(org(), &snapshot));
        assert!(folds.hidden_ranges(org(), &snapshot).is_empty());
    }

    #[test]
    fn local_fold_follows_its_heading_across_edits_above_it() {
        let source = b"intro\n* One\nbody\n** Child\nchild body\n* Two\ntail\n".to_vec();
        let mut buffer = DocumentBuffer::from_utf8(source).unwrap();
        let snapshot = buffer.snapshot();
        let mut folds = EditorFoldState::default();
        folds.toggle_heading(org(), &snapshot, 1);

        let delta = buffer
            .commit(EditTransaction::new(
                snapshot.revision(),
                vec![TextEdit::new(ByteRange::new(0, 0), "new\n")],
            ))
            .unwrap();
        folds.apply_delta(&delta);

        assert_eq!(folds.hidden_ranges(org(), &buffer.snapshot()), vec![3..6]);
    }

    #[test]
    fn leaf_heading_cycles_directly_between_folded_and_expanded() {
        let snapshot = DocumentSnapshot::from_utf8(b"* One\nbody\n* Two\n".to_vec()).unwrap();
        let mut folds = EditorFoldState::default();

        folds.toggle_heading(org(), &snapshot, 0);
        assert_eq!(folds.hidden_ranges(org(), &snapshot), vec![1..2]);
        folds.toggle_heading(org(), &snapshot, 0);
        assert!(folds.hidden_ranges(org(), &snapshot).is_empty());
        folds.toggle_heading(org(), &snapshot, 0);
        assert_eq!(folds.hidden_ranges(org(), &snapshot), vec![1..2]);
    }

    #[test]
    fn headings_inside_example_blocks_do_not_truncate_parent_folds() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"* Parent\n#+begin_example\n* literal heading\n#+end_example\n* Next\n".to_vec(),
        )
        .unwrap();
        let mut folds = EditorFoldState::default();

        folds.toggle_heading(org(), &snapshot, 0);
        assert_eq!(folds.hidden_ranges(org(), &snapshot), vec![1..4]);
    }

    #[test]
    fn markdown_headings_fold_with_the_same_local_cycle() {
        let snapshot =
            DocumentSnapshot::from_utf8(b"# Parent\nbody\n## Child\nchild body\n# Next\n".to_vec())
                .unwrap();
        let mut folds = EditorFoldState::default();

        folds.toggle_heading(markdown(), &snapshot, 0);
        assert_eq!(folds.hidden_ranges(markdown(), &snapshot), vec![1..4]);
        folds.toggle_heading(markdown(), &snapshot, 0);
        assert_eq!(folds.hidden_ranges(markdown(), &snapshot), vec![3..4]);
        folds.toggle_heading(markdown(), &snapshot, 0);
        assert!(folds.hidden_ranges(markdown(), &snapshot).is_empty());
        folds.cycle_global(markdown(), &snapshot);
        folds.toggle_heading(markdown(), &snapshot, 0);
        assert_eq!(folds.hidden_ranges(markdown(), &snapshot), vec![3..4, 5..6]);
    }

    #[test]
    fn markdown_headings_inside_fences_do_not_end_a_fold() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"# Parent\n```md\n# literal heading\n```\n# Next\n".to_vec(),
        )
        .unwrap();
        let mut folds = EditorFoldState::default();

        folds.toggle_heading(markdown(), &snapshot, 0);
        assert_eq!(folds.hidden_ranges(markdown(), &snapshot), vec![1..4]);
    }

    #[test]
    fn org_begin_end_blocks_toggle_from_the_opening_boundary() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"before\n#+begin_src rust\nfn main() {\n    println!(\"hi\");\n}\n#+end_src\nafter\n"
                .to_vec(),
        )
        .unwrap();
        let mut folds = EditorFoldState::default();

        assert!(folds.toggle_block(org(), &snapshot, 1));
        assert_eq!(folds.hidden_ranges(org(), &snapshot), vec![2..6]);
        assert_eq!(
            folds.projection(org(), &snapshot).marker_lines,
            HashSet::from([1])
        );
        assert!(!folds.toggle_block(org(), &snapshot, 2));
        assert!(folds.toggle_block(org(), &snapshot, 1));
        assert!(folds.hidden_ranges(org(), &snapshot).is_empty());
    }

    #[test]
    fn org_quote_and_other_named_blocks_use_the_same_fold_projection() {
        for name in [
            "quote", "example", "verse", "center", "comment", "export", "details",
        ] {
            let source = format!("before\n#+begin_{name}\nbody\n#+end_{name}\nafter\n");
            let snapshot = DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
            let mut folds = EditorFoldState::default();

            assert!(folds.toggle_block(org(), &snapshot, 1), "{name}");
            assert_eq!(folds.hidden_ranges(org(), &snapshot), vec![2..4], "{name}");
            assert_eq!(
                folds.projection(org(), &snapshot).marker_lines,
                HashSet::from([1]),
                "{name}"
            );
        }
    }

    #[test]
    fn markdown_fenced_code_blocks_fold_and_unclosed_fences_reach_eof() {
        let closed =
            DocumentSnapshot::from_utf8(b"before\n```rust\nfn main() {}\n```\nafter\n".to_vec())
                .unwrap();
        let mut folds = EditorFoldState::default();
        assert!(folds.toggle_block(markdown(), &closed, 1));
        assert_eq!(folds.hidden_ranges(markdown(), &closed), vec![2..4]);

        let unclosed =
            DocumentSnapshot::from_utf8(b"```python\nprint('hi')\nmore\n".to_vec()).unwrap();
        let mut folds = EditorFoldState::default();
        assert!(folds.toggle_block(markdown(), &unclosed, 0));
        assert_eq!(folds.hidden_ranges(markdown(), &unclosed), vec![1..4]);
    }

    #[test]
    fn structural_block_fold_follows_edits_above_it() {
        let mut buffer = DocumentBuffer::from_utf8(
            b"before\n#+begin_src rust\nfn main() {}\n#+end_src\n".to_vec(),
        )
        .unwrap();
        let snapshot = buffer.snapshot();
        let mut folds = EditorFoldState::default();
        assert!(folds.toggle_block(org(), &snapshot, 1));

        let delta = buffer
            .commit(EditTransaction::new(
                snapshot.revision(),
                vec![TextEdit::new(ByteRange::new(0, 0), "new\n")],
            ))
            .unwrap();
        folds.apply_delta(&delta);

        assert_eq!(folds.hidden_ranges(org(), &buffer.snapshot()), vec![3..5]);
    }

    #[test]
    fn range_subtraction_preserves_only_uncovered_segments() {
        assert_eq!(
            subtract_ranges(&[2..12, 20..25], &[0..4, 7..9, 10..22]),
            vec![4..7, 9..10, 22..25]
        );
    }

    #[test]
    fn global_cycle_is_a_no_op_when_the_document_has_no_headings() {
        let snapshot = DocumentSnapshot::from_utf8(b"plain\ncontent\n".to_vec()).unwrap();
        let mut folds = EditorFoldState::default();

        assert!(!folds.cycle_global(org(), &snapshot));
        assert_eq!(folds.global, GlobalVisibility::All);
        assert_eq!(
            folds.projection(org(), &snapshot),
            EditorFoldProjection::default()
        );
    }

    #[test]
    fn source_file_does_not_treat_org_like_text_as_headings() {
        let snapshot = DocumentSnapshot::from_utf8(b"* pointer\nbody\n".to_vec()).unwrap();
        let path = std::path::Path::new("main.rs");
        let mut folds = EditorFoldState::default();
        assert!(folds.heading_index(path, &snapshot).is_empty());
        assert!(!folds.cycle_global(path, &snapshot));
        assert_eq!(
            folds.projection(path, &snapshot),
            EditorFoldProjection::default()
        );
    }

    #[test]
    fn markers_follow_shared_preview_semantics_in_children_and_contents_states() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"* Parent\nbody\n** Child\nchild body\n*** Grand\ndeep\n* Next\n".to_vec(),
        )
        .unwrap();
        let mut folds = EditorFoldState::default();

        folds.toggle_heading(org(), &snapshot, 0);
        assert_eq!(
            folds.projection(org(), &snapshot).marker_lines,
            HashSet::from([0])
        );
        folds.toggle_heading(org(), &snapshot, 0);
        assert_eq!(
            folds.projection(org(), &snapshot).marker_lines,
            HashSet::from([2])
        );

        assert!(folds.cycle_global(org(), &snapshot));
        assert_eq!(folds.global, GlobalVisibility::Overview);
        assert!(folds.cycle_global(org(), &snapshot));
        assert_eq!(folds.global, GlobalVisibility::Contents);
        assert_eq!(
            folds.projection(org(), &snapshot).marker_lines,
            HashSet::from([4])
        );
    }
}
