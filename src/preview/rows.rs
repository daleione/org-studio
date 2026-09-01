use crate::{
    document::{ByteRange, TextSnapshot},
    org_syntax::{BlockArena, BlockId, BlockKind},
};
use std::ops::Range;

use super::PreviewRow;

pub(super) fn build_preview_rows(text: &dyn TextSnapshot, blocks: &BlockArena) -> Vec<PreviewRow> {
    build_preview_rows_in_range(text, blocks, 0..blocks.nodes().len())
}

pub(super) fn build_preview_rows_in_range(
    text: &dyn TextSnapshot,
    blocks: &BlockArena,
    block_range: Range<usize>,
) -> Vec<PreviewRow> {
    let mut rows = Vec::with_capacity(block_range.len());
    for (block_id, block) in blocks.nodes()[block_range.clone()].iter().enumerate() {
        let block_id = (block_range.start + block_id) as BlockId;

        if matches!(block.kind, BlockKind::Comment | BlockKind::CommentBlock) {
            continue;
        }

        if matches!(
            block.kind,
            BlockKind::SourceBlock { .. }
                | BlockKind::Drawer { .. }
                | BlockKind::ExampleBlock
                | BlockKind::QuoteBlock
                | BlockKind::VerseBlock
                | BlockKind::CenterBlock
                | BlockKind::ExportBlock { .. }
                | BlockKind::SpecialBlock { .. }
        ) {
            push_physical_rows(text, block_id, block.content, false, &mut rows);
            continue;
        }

        if matches!(block.kind, BlockKind::Raw) {
            push_physical_rows(text, block_id, block.source, false, &mut rows);
            continue;
        }

        if matches!(block.kind, BlockKind::BlankLine) {
            rows.push(PreviewRow {
                block_id,
                content: text.revision_range(block.content),
                continuation: false,
                blank: true,
            });
            continue;
        }

        if !matches!(block.kind, BlockKind::Paragraph) {
            rows.push(PreviewRow {
                block_id,
                content: text.revision_range(block.content),
                continuation: false,
                blank: false,
            });
            continue;
        }

        push_physical_rows(text, block_id, block.content, false, &mut rows);
    }
    rows
}

fn push_physical_rows(
    text: &dyn TextSnapshot,
    block_id: BlockId,
    range: ByteRange,
    blank: bool,
    rows: &mut Vec<PreviewRow>,
) {
    if range.is_empty() {
        rows.push(PreviewRow {
            block_id,
            content: text.revision_range(range),
            continuation: false,
            blank,
        });
        return;
    }
    let source = text.copy_range(range);
    let mut start = 0;
    let mut continuation = false;
    while start < source.len() {
        let next = source[start..]
            .find('\n')
            .map(|newline| start + newline + 1)
            .unwrap_or(source.len());
        let mut end = next;
        while end > start && matches!(source.as_bytes()[end - 1], b'\n' | b'\r') {
            end -= 1;
        }
        let global_start = range.start.0 + start as u64;
        rows.push(PreviewRow {
            block_id,
            content: text.revision_range(ByteRange::new(global_start, range.start.0 + end as u64)),
            continuation,
            blank,
        });
        continuation = true;
        start = next;
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        document::{DocumentSnapshot, TextSnapshot},
        org_syntax::{BlockKind, parse},
    };

    use super::build_preview_rows;

    #[test]
    fn preserves_consecutive_blank_lines_and_source_numbers() {
        let text = DocumentSnapshot::from_utf8(b"* Heading\nbody\n\n\nnext\n".to_vec()).unwrap();
        let blocks = parse(&text);
        let rows = build_preview_rows(&text, &blocks);

        assert_eq!(
            rows.iter()
                .map(|row| text.line_of_byte(row.content.range.start) + 1)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );
        assert!(rows[2].blank);
        assert!(rows[3].blank);
        assert!(matches!(
            blocks.nodes()[rows[2].block_id as usize].kind,
            BlockKind::BlankLine
        ));
        assert!(matches!(
            blocks.nodes()[rows[3].block_id as usize].kind,
            BlockKind::BlankLine
        ));
    }

    #[test]
    fn keeps_long_cjk_physical_lines_intact_for_gpui_wrapping() {
        let source = format!("{}\n", "返回当前分区中的窗口函数计算结果".repeat(32));
        let text = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let blocks = parse(&text);
        let rows = build_preview_rows(&text, &blocks);

        assert_eq!(rows.len(), 1);
        assert_eq!(text.copy_range(rows[0].content.range), source.trim_end());
    }

    #[test]
    fn org_source_block_projects_only_reading_content() {
        let text = DocumentSnapshot::from_utf8(
            b"before\n#+begin_src rust\n    let value = 1;\n#+end_src\nafter\n".to_vec(),
        )
        .unwrap();
        let blocks = parse(&text);
        let rows = build_preview_rows(&text, &blocks);

        assert_eq!(text.copy_range(rows[1].content.range), "    let value = 1;");
        assert_eq!(
            rows.iter()
                .map(|row| text.line_of_byte(row.content.range.start) + 1)
                .collect::<Vec<_>>(),
            vec![1, 3, 5]
        );
    }
}
