use std::collections::HashSet;

use crate::org_syntax::{BlockArena, BlockId, BlockKind};

use super::{
    markdown::{MarkdownBlock, MarkdownKind},
    projection::VisualRowTree,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum GlobalVisibility {
    #[default]
    All,
    Overview,
    Contents,
}

impl GlobalVisibility {
    pub(super) fn next(self) -> Self {
        match self {
            Self::All => Self::Overview,
            Self::Overview => Self::Contents,
            Self::Contents => Self::All,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LocalVisibility {
    Empty,
    Folded,
    Children,
    Subtree,
}

pub(super) struct LocalCycleProjection {
    pub(super) visibility: LocalVisibility,
    pub(super) visible_rows: Vec<usize>,
    pub(super) fold_markers: HashSet<BlockId>,
}

#[derive(Clone, Copy)]
struct HeadingRow {
    row: usize,
    block_id: BlockId,
    level: u16,
}

pub(super) fn global_org_visibility(
    rows: &VisualRowTree,
    blocks: &BlockArena,
    visibility: GlobalVisibility,
) -> (Vec<usize>, HashSet<BlockId>) {
    let headings = org_heading_rows(rows, blocks);
    global_heading_visibility(rows.len(), &headings, visibility)
}

pub(super) fn global_markdown_visibility(
    rows: &VisualRowTree,
    blocks: &[MarkdownBlock],
    visibility: GlobalVisibility,
) -> (Vec<usize>, HashSet<BlockId>) {
    let headings = markdown_heading_rows(rows, blocks);
    global_heading_visibility(rows.len(), &headings, visibility)
}

pub(super) fn cycle_org_subtree_visibility(
    rows: &VisualRowTree,
    blocks: &BlockArena,
    current_visible: &[usize],
    current_markers: &HashSet<BlockId>,
    block_id: BlockId,
    continue_from_children: bool,
) -> Option<LocalCycleProjection> {
    cycle_subtree_visibility(
        rows.len(),
        &org_heading_rows(rows, blocks),
        current_visible,
        current_markers,
        block_id,
        continue_from_children,
    )
}

pub(super) fn cycle_markdown_subtree_visibility(
    rows: &VisualRowTree,
    blocks: &[MarkdownBlock],
    current_visible: &[usize],
    current_markers: &HashSet<BlockId>,
    block_id: BlockId,
    continue_from_children: bool,
) -> Option<LocalCycleProjection> {
    cycle_subtree_visibility(
        rows.len(),
        &markdown_heading_rows(rows, blocks),
        current_visible,
        current_markers,
        block_id,
        continue_from_children,
    )
}

fn org_heading_rows(rows: &VisualRowTree, blocks: &BlockArena) -> Vec<HeadingRow> {
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let block = blocks.nodes().get(row.block_id as usize)?;
            let BlockKind::Heading { level } = block.kind else {
                return None;
            };
            Some(HeadingRow {
                row: index,
                block_id: row.block_id,
                level,
            })
        })
        .collect()
}

fn markdown_heading_rows(rows: &VisualRowTree, blocks: &[MarkdownBlock]) -> Vec<HeadingRow> {
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let block = blocks.get(row.block_id as usize)?;
            let MarkdownKind::Heading { level } = block.kind else {
                return None;
            };
            Some(HeadingRow {
                row: index,
                block_id: row.block_id,
                level,
            })
        })
        .collect()
}

fn global_heading_visibility(
    row_count: usize,
    headings: &[HeadingRow],
    visibility: GlobalVisibility,
) -> (Vec<usize>, HashSet<BlockId>) {
    if visibility == GlobalVisibility::All {
        return ((0..row_count).collect(), HashSet::new());
    }

    let top_level = headings.iter().map(|heading| heading.level).min();
    let visible = headings
        .iter()
        .filter(|heading| {
            visibility == GlobalVisibility::Contents || Some(heading.level) == top_level
        })
        .map(|heading| heading.row)
        .collect();
    let markers = headings
        .iter()
        .enumerate()
        .filter(|(_, heading)| {
            visibility == GlobalVisibility::Contents || Some(heading.level) == top_level
        })
        .filter_map(|(index, heading)| {
            let subtree_end = heading_subtree_end(row_count, headings, index);
            let has_hidden_rows = subtree_end > heading.row + 1;
            let shows_child = visibility == GlobalVisibility::Contents
                && headings
                    .get(index + 1)
                    .is_some_and(|next| next.level > heading.level);
            (has_hidden_rows && !shows_child).then_some(heading.block_id)
        })
        .collect();
    (visible, markers)
}

fn cycle_subtree_visibility(
    row_count: usize,
    headings: &[HeadingRow],
    current_visible: &[usize],
    current_markers: &HashSet<BlockId>,
    block_id: BlockId,
    continue_from_children: bool,
) -> Option<LocalCycleProjection> {
    let heading_index = headings
        .iter()
        .position(|heading| heading.block_id == block_id)?;
    let heading = headings[heading_index];
    let subtree_end = heading_subtree_end(row_count, headings, heading_index);
    if subtree_end == heading.row + 1 {
        return Some(LocalCycleProjection {
            visibility: LocalVisibility::Empty,
            visible_rows: current_visible.to_vec(),
            fold_markers: current_markers.clone(),
        });
    }

    let direct_children = direct_child_headings(headings, heading_index, subtree_end);
    let subtree_is_folded = current_visible
        .iter()
        .filter(|row| **row >= heading.row && **row < subtree_end)
        .copied()
        .eq(std::iter::once(heading.row));
    let visibility = if subtree_is_folded {
        if direct_children.is_empty() {
            LocalVisibility::Subtree
        } else {
            LocalVisibility::Children
        }
    } else if continue_from_children {
        LocalVisibility::Subtree
    } else {
        LocalVisibility::Folded
    };

    let replacement = match visibility {
        LocalVisibility::Folded => vec![heading.row],
        LocalVisibility::Children => {
            let entry_end = headings
                .get(heading_index + 1)
                .map_or(subtree_end, |next| next.row);
            (heading.row..entry_end)
                .chain(direct_children.iter().map(|(_, child)| child.row))
                .collect()
        }
        LocalVisibility::Subtree => (heading.row..subtree_end).collect(),
        LocalVisibility::Empty => unreachable!(),
    };
    let mut visible_rows = Vec::with_capacity(
        current_visible.len() - current_visible.partition_point(|row| *row < subtree_end)
            + current_visible.partition_point(|row| *row < heading.row)
            + replacement.len(),
    );
    visible_rows.extend(
        current_visible
            .iter()
            .copied()
            .take_while(|row| *row < heading.row),
    );
    visible_rows.extend(replacement);
    visible_rows.extend(
        current_visible
            .iter()
            .copied()
            .skip_while(|row| *row < subtree_end),
    );

    let mut fold_markers = current_markers.clone();
    for nested in headings
        .iter()
        .skip(heading_index)
        .take_while(|nested| nested.row < subtree_end)
    {
        fold_markers.remove(&nested.block_id);
    }
    match visibility {
        LocalVisibility::Folded => {
            fold_markers.insert(block_id);
        }
        LocalVisibility::Children => {
            for (child_index, child) in direct_children {
                if heading_subtree_end(row_count, headings, child_index) > child.row + 1 {
                    fold_markers.insert(child.block_id);
                }
            }
        }
        LocalVisibility::Subtree | LocalVisibility::Empty => {}
    }

    Some(LocalCycleProjection {
        visibility,
        visible_rows,
        fold_markers,
    })
}

fn heading_subtree_end(row_count: usize, headings: &[HeadingRow], index: usize) -> usize {
    let heading = headings[index];
    headings
        .iter()
        .skip(index + 1)
        .find(|next| next.level <= heading.level)
        .map_or(row_count, |next| next.row)
}

fn direct_child_headings(
    headings: &[HeadingRow],
    index: usize,
    subtree_end: usize,
) -> Vec<(usize, HeadingRow)> {
    let parent = headings[index];
    let mut stack = vec![parent];
    let mut children = Vec::new();
    for (heading_index, heading) in headings
        .iter()
        .copied()
        .enumerate()
        .skip(index + 1)
        .take_while(|(_, heading)| heading.row < subtree_end)
    {
        while stack
            .last()
            .is_some_and(|ancestor| ancestor.level >= heading.level)
        {
            stack.pop();
        }
        if stack
            .last()
            .is_some_and(|ancestor| ancestor.block_id == parent.block_id)
        {
            children.push((heading_index, heading));
        }
        stack.push(heading);
    }
    children
}

#[cfg(test)]
pub(super) fn visible_row_indices(
    rows: &VisualRowTree,
    blocks: &BlockArena,
    folded: &HashSet<BlockId>,
) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| {
            (!has_folded_ancestor(row.block_id, blocks, folded)).then_some(index)
        })
        .collect()
}

#[cfg(test)]
pub(super) fn visible_markdown_row_indices(
    rows: &VisualRowTree,
    blocks: &[MarkdownBlock],
    folded: &HashSet<BlockId>,
) -> Vec<usize> {
    let mut hidden_below_level = None::<u16>;
    let mut visible = Vec::with_capacity(rows.len());

    for (index, row) in rows.iter().enumerate() {
        let kind = blocks.get(row.block_id as usize).map(|block| &block.kind);
        if let Some(MarkdownKind::Heading { level }) = kind {
            if hidden_below_level.is_some_and(|parent_level| *level > parent_level) {
                continue;
            }
            hidden_below_level = None;
            visible.push(index);
            if folded.contains(&row.block_id) {
                hidden_below_level = Some(*level);
            }
        } else if hidden_below_level.is_none() {
            visible.push(index);
        }
    }

    visible
}

#[cfg(test)]
fn has_folded_ancestor(block_id: BlockId, blocks: &BlockArena, folded: &HashSet<BlockId>) -> bool {
    let mut parent = blocks.nodes()[block_id as usize].parent;
    while let Some(block_id) = parent {
        if folded.contains(&block_id) {
            return true;
        }
        parent = blocks.nodes()[block_id as usize].parent;
    }
    false
}

pub(super) fn changed_range(old: &[usize], new: &[usize]) -> (std::ops::Range<usize>, usize) {
    let prefix = old
        .iter()
        .zip(new)
        .take_while(|(old, new)| old == new)
        .count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    (
        prefix..old.len().saturating_sub(suffix),
        new.len().saturating_sub(prefix + suffix),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::{
        document::{RopeSnapshot, TextSnapshot},
        org_syntax::{BlockId, BlockKind, parse},
        preview::{markdown, rows::build_preview_rows},
    };

    use super::{
        GlobalVisibility, LocalVisibility, changed_range, cycle_markdown_subtree_visibility,
        cycle_org_subtree_visibility, global_markdown_visibility, global_org_visibility,
        visible_markdown_row_indices, visible_row_indices,
    };

    fn source_lines(
        text: &RopeSnapshot,
        rows: &crate::preview::projection::VisualRowTree,
        visible: &[usize],
    ) -> Vec<u64> {
        visible
            .iter()
            .map(|index| text.line_of_byte(rows.get(*index).unwrap().source.range.start) + 1)
            .collect()
    }

    #[test]
    fn folding_hides_only_heading_descendants() {
        let text = RopeSnapshot::from_utf8(
            b"* One\nbody\n** Child\nchild body\n* Two\nvisible\n".to_vec(),
        )
        .unwrap();
        let blocks = parse(&text);
        let rows = build_preview_rows(&text, &blocks);
        let heading = blocks
            .nodes()
            .iter()
            .position(|block| matches!(block.kind, BlockKind::Heading { level: 1 }))
            .unwrap() as u32;

        let projection = crate::preview::projection::build_projection_snapshot(
            text.revision(),
            crate::preview::DocumentFormat::Org,
            std::sync::Arc::new(rows),
            &blocks,
            &[],
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        );
        let visible = visible_row_indices(&projection.rows, &blocks, &HashSet::from([heading]));
        let lines = visible
            .into_iter()
            .map(|index| {
                text.line_of_byte(projection.rows.get(index).unwrap().source.range.start) + 1
            })
            .collect::<Vec<_>>();
        assert_eq!(lines, vec![1, 5, 6]);
    }

    #[test]
    fn computes_minimal_list_splice() {
        assert_eq!(changed_range(&[0, 1, 2, 3, 4], &[0, 1, 4]), (2..4, 0));
        assert_eq!(changed_range(&[0, 1, 4], &[0, 1, 2, 3, 4]), (2..2, 2));
    }

    #[test]
    fn org_global_cycle_projects_overview_contents_and_all() {
        let text = RopeSnapshot::from_utf8(
            b"preamble\n** One\nbody\n*** Child\nchild body\n**** Grandchild\ndeep\n** Two\nvisible\n"
                .to_vec(),
        )
        .unwrap();
        let blocks = parse(&text);
        let rows = build_preview_rows(&text, &blocks);
        let projection = crate::preview::projection::build_projection_snapshot(
            text.revision(),
            crate::preview::DocumentFormat::Org,
            std::sync::Arc::new(rows),
            &blocks,
            &[],
            &Default::default(),
            &Default::default(),
        );

        let (overview, overview_markers) =
            global_org_visibility(&projection.rows, &blocks, GlobalVisibility::Overview);
        assert_eq!(source_lines(&text, &projection.rows, &overview), vec![2, 8]);
        assert_eq!(overview_markers.len(), 2);

        let (contents, contents_markers) =
            global_org_visibility(&projection.rows, &blocks, GlobalVisibility::Contents);
        assert_eq!(
            source_lines(&text, &projection.rows, &contents),
            vec![2, 4, 6, 8]
        );
        assert_eq!(contents_markers.len(), 2);
        let parent = blocks
            .nodes()
            .iter()
            .position(|block| matches!(block.kind, BlockKind::Heading { level: 2 }))
            .unwrap() as BlockId;
        assert!(!contents_markers.contains(&parent));

        let child = blocks
            .nodes()
            .iter()
            .position(|block| matches!(block.kind, BlockKind::Heading { level: 3 }))
            .unwrap() as BlockId;
        let folded = cycle_org_subtree_visibility(
            &projection.rows,
            &blocks,
            &contents,
            &contents_markers,
            child,
            false,
        )
        .unwrap();
        assert_eq!(folded.visibility, LocalVisibility::Folded);
        assert_eq!(
            source_lines(&text, &projection.rows, &folded.visible_rows),
            vec![2, 4, 8]
        );

        let children = cycle_org_subtree_visibility(
            &projection.rows,
            &blocks,
            &overview,
            &overview_markers,
            parent,
            false,
        )
        .unwrap();
        assert_eq!(children.visibility, LocalVisibility::Children);
        assert_eq!(
            source_lines(&text, &projection.rows, &children.visible_rows),
            vec![2, 3, 4, 8]
        );
        let subtree = cycle_org_subtree_visibility(
            &projection.rows,
            &blocks,
            &children.visible_rows,
            &children.fold_markers,
            parent,
            true,
        )
        .unwrap();
        assert_eq!(subtree.visibility, LocalVisibility::Subtree);
        assert_eq!(
            source_lines(&text, &projection.rows, &subtree.visible_rows),
            vec![2, 3, 4, 5, 6, 7, 8]
        );

        let (all, all_markers) =
            global_org_visibility(&projection.rows, &blocks, GlobalVisibility::All);
        assert_eq!(all.len(), projection.rows.len());
        assert!(all_markers.is_empty());
        assert_eq!(GlobalVisibility::All.next(), GlobalVisibility::Overview);
        assert_eq!(
            GlobalVisibility::Overview.next(),
            GlobalVisibility::Contents
        );
        assert_eq!(GlobalVisibility::Contents.next(), GlobalVisibility::All);
    }

    #[test]
    fn markdown_global_cycle_uses_the_same_heading_projection() {
        let text = RopeSnapshot::from_utf8(
            b"preamble\n## One\nbody\n### Child\nchild body\n## Two\nvisible\n".to_vec(),
        )
        .unwrap();
        let (blocks, rows) = markdown::parse_markdown(&text);
        let projection = crate::preview::projection::build_projection_snapshot(
            text.revision(),
            crate::preview::DocumentFormat::Markdown,
            std::sync::Arc::new(rows),
            &Default::default(),
            &blocks,
            &Default::default(),
            &Default::default(),
        );

        let (overview, _) =
            global_markdown_visibility(&projection.rows, &blocks, GlobalVisibility::Overview);
        assert_eq!(source_lines(&text, &projection.rows, &overview), vec![2, 6]);
        let (contents, contents_markers) =
            global_markdown_visibility(&projection.rows, &blocks, GlobalVisibility::Contents);
        assert_eq!(
            source_lines(&text, &projection.rows, &contents),
            vec![2, 4, 6]
        );
        assert_eq!(contents_markers, HashSet::from([3, 5]));
        let expanded = cycle_markdown_subtree_visibility(
            &projection.rows,
            &blocks,
            &contents,
            &contents_markers,
            3,
            false,
        )
        .unwrap();
        assert_eq!(expanded.visibility, LocalVisibility::Subtree);
        assert_eq!(
            source_lines(&text, &projection.rows, &expanded.visible_rows),
            vec![2, 4, 5, 6]
        );
    }

    #[test]
    fn markdown_folding_uses_heading_sections_and_preserves_nested_folds() {
        let text = RopeSnapshot::from_utf8(
            b"# One\nbody\n## Child\nchild body\n### Grandchild\ndeep\n# Two\nvisible\n".to_vec(),
        )
        .unwrap();
        let (blocks, rows) = markdown::parse_markdown(&text);
        let projection = crate::preview::projection::build_projection_snapshot(
            text.revision(),
            crate::preview::DocumentFormat::Markdown,
            std::sync::Arc::new(rows),
            &Default::default(),
            &blocks,
            &Default::default(),
            &Default::default(),
        );

        let top_folded =
            visible_markdown_row_indices(&projection.rows, &blocks, &HashSet::from([0]));
        assert_eq!(top_folded, vec![0, 6, 7]);

        let child_folded =
            visible_markdown_row_indices(&projection.rows, &blocks, &HashSet::from([2]));
        assert_eq!(child_folded, vec![0, 1, 2, 6, 7]);

        let nested_folds =
            visible_markdown_row_indices(&projection.rows, &blocks, &HashSet::from([0, 2]));
        assert_eq!(nested_folds, vec![0, 6, 7]);
    }
}
