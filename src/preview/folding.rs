use std::collections::HashSet;

use crate::org_syntax::{BlockArena, BlockId};

use super::{
    markdown::{MarkdownBlock, MarkdownKind},
    projection::VisualRowTree,
};

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
        org_syntax::{BlockKind, parse},
        preview::{markdown, rows::build_preview_rows},
    };

    use super::{changed_range, visible_markdown_row_indices, visible_row_indices};

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
