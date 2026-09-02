use crate::document::{
    DocumentFormat, DocumentId, DocumentSnapshot, HeadingIndex, LineIndex, LocalVisibility,
    OutlineHeading, Revision, RevisionDelta, RevisionRange, TextSnapshot,
    global_outline_visibility, next_local_visibility,
};
use std::{cell::RefCell, collections::HashSet, path::Path, sync::Arc};

pub(super) use crate::document::GlobalVisibility;

#[derive(Default)]
pub(super) struct EditorFoldState {
    headings: Vec<FoldedHeading>,
    pub(super) global: GlobalVisibility,
    heading_cache: RefCell<Option<CachedHeadingIndex>>,
}

struct FoldedHeading {
    source: RevisionRange,
    visibility: LocalVisibility,
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
        self.global = GlobalVisibility::All;
        let existing = self
            .headings
            .iter()
            .position(|heading| heading.source.range.start == range.start);
        let has_children =
            !direct_child_indices(index.as_slice(), heading_index, subtree_end).is_empty();
        match next_local_visibility(
            existing.map(|index| self.headings[index].visibility),
            has_children,
        ) {
            LocalVisibility::Folded => self.headings.push(FoldedHeading {
                source: snapshot.revision_range(range),
                visibility: LocalVisibility::Folded,
            }),
            LocalVisibility::Children => {
                let index = existing.expect("fold index exists");
                self.headings[index].visibility = LocalVisibility::Children;
            }
            LocalVisibility::Subtree => {
                self.headings.remove(existing.expect("fold index exists"));
            }
            LocalVisibility::Empty => unreachable!(),
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

    pub(super) fn expand_heading(&mut self, snapshot: &DocumentSnapshot, line: u64) {
        if self.global != GlobalVisibility::All {
            self.global = GlobalVisibility::All;
            self.headings.clear();
        } else if let Ok(range) = snapshot.line_content_range(LineIndex(line)) {
            self.headings
                .retain(|heading| heading.source.range.start != range.start);
        }
    }

    pub(super) fn projection(
        &self,
        path: &Path,
        snapshot: &DocumentSnapshot,
    ) -> EditorFoldProjection {
        if self.global == GlobalVisibility::All && self.headings.is_empty() {
            return EditorFoldProjection::default();
        }
        let index = self.heading_index(path, snapshot);
        let headings = index.as_slice();
        if headings.is_empty() {
            return EditorFoldProjection::default();
        }
        let mut hidden = Vec::new();
        let mut markers = HashSet::new();
        let semantic_lines = semantic_line_count(snapshot);
        match self.global {
            GlobalVisibility::All => {
                for folded in &self.headings {
                    if let Some((index, heading)) = headings
                        .iter()
                        .enumerate()
                        .find(|(_, heading)| heading.start == folded.source.range.start)
                    {
                        let end = heading_subtree_end(semantic_lines, headings, index);
                        if heading.line + 1 < end {
                            match folded.visibility {
                                LocalVisibility::Folded => {
                                    hidden.push(heading.line + 1..end);
                                    markers.insert(heading.line);
                                }
                                LocalVisibility::Children => {
                                    let entry_end =
                                        headings.get(index + 1).map_or(end, |next| next.line);
                                    let mut cursor = entry_end;
                                    for child_index in direct_child_indices(headings, index, end) {
                                        let child = headings[child_index];
                                        if cursor < child.line {
                                            hidden.push(cursor..child.line);
                                        }
                                        let child_end = heading_subtree_end(
                                            semantic_lines,
                                            headings,
                                            child_index,
                                        );
                                        if child_end > child.line + 1 {
                                            markers.insert(child.line);
                                        }
                                        cursor = child.line + 1;
                                    }
                                    if cursor < end {
                                        hidden.push(cursor..end);
                                    }
                                }
                                LocalVisibility::Empty | LocalVisibility::Subtree => unreachable!(),
                            }
                        }
                    }
                }
            }
            GlobalVisibility::Overview | GlobalVisibility::Contents => {
                let outline = outline_headings(headings);
                let (visible, global_markers) =
                    global_outline_visibility(semantic_lines as usize, &outline, self.global);
                hidden = complement(snapshot.len_lines(), &visible);
                markers.extend(global_markers.into_iter().map(|line| line as u64));
            }
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
        assert!(folds.cycle_global(org(), &snapshot));
        assert_eq!(folds.global, GlobalVisibility::Contents);
        assert_eq!(
            folds.hidden_ranges(org(), &snapshot),
            vec![0..1, 2..3, 4..5, 6..8]
        );
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
