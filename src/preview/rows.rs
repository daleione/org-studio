use crate::{
    document::{ByteOffset, ByteRange, TextSnapshot},
    org_syntax::{BlockArena, BlockId, BlockKind},
};

use super::PreviewRow;

pub(super) fn build_preview_rows(
    text: &dyn TextSnapshot,
    blocks: &BlockArena,
) -> Vec<PreviewRow> {
    let mut rows = Vec::with_capacity(blocks.nodes().len());
    for (block_id, block) in blocks.nodes().iter().enumerate() {
        let block_id = block_id as BlockId;

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
                | BlockKind::Raw
        ) {
            push_physical_rows(text, block_id, block.source, false, &mut rows);
            continue;
        }

        if matches!(block.kind, BlockKind::BlankLine) {
            rows.push(PreviewRow {
                block_id,
                content: block.content,
                continuation: false,
                source_line: text.line_of_byte(block.source.start) + 1,
                show_line_number: true,
                blank: true,
            });
            continue;
        }

        if !matches!(block.kind, BlockKind::Paragraph) {
            rows.push(PreviewRow {
                block_id,
                content: block.content,
                continuation: false,
                source_line: text.line_of_byte(block.content.start) + 1,
                show_line_number: true,
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
            content: ByteRange::new(global_start, range.start.0 + end as u64),
            continuation,
            source_line: text.line_of_byte(ByteOffset(global_start)) + 1,
            show_line_number: true,
            blank,
        });
        continuation = true;
        start = next;
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        document::{RopeSnapshot, TextSnapshot},
        org_syntax::{BlockKind, parse},
    };

    use super::build_preview_rows;

    #[test]
    fn preserves_consecutive_blank_lines_and_source_numbers() {
        let text = RopeSnapshot::from_utf8(b"* Heading\nbody\n\n\nnext\n".to_vec()).unwrap();
        let blocks = parse(&text);
        let rows = build_preview_rows(&text, &blocks);

        assert_eq!(
            rows.iter().map(|row| row.source_line).collect::<Vec<_>>(),
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
        let text = RopeSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let blocks = parse(&text);
        let rows = build_preview_rows(&text, &blocks);

        assert_eq!(rows.len(), 1);
        assert_eq!(text.copy_range(rows[0].content), source.trim_end());
    }
}
