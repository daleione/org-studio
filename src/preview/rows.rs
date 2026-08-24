use crate::{
    document::{ByteOffset, ByteRange, TextSnapshot},
    org_syntax::{BlockArena, BlockId, BlockKind},
};

use super::PreviewRow;

const MAX_PARAGRAPH_ROW_BYTES: usize = 256;

pub(super) fn build_preview_rows(
    text: &dyn TextSnapshot,
    blocks: &BlockArena,
) -> Vec<PreviewRow> {
    let mut rows = Vec::with_capacity(blocks.nodes().len());
    for (block_id, block) in blocks.nodes().iter().enumerate() {
        let block_id = block_id as BlockId;

        if matches!(block.kind, BlockKind::SourceBlock { .. }) {
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

        push_paragraph_rows(text, block_id, block.content, &mut rows);
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

fn push_paragraph_rows(
    text: &dyn TextSnapshot,
    block_id: BlockId,
    range: ByteRange,
    rows: &mut Vec<PreviewRow>,
) {
    let source = text.copy_range(range);
    let mut physical_start = 0;
    let mut block_continuation = false;
    while physical_start < source.len() {
        let physical_next = source[physical_start..]
            .find('\n')
            .map(|newline| physical_start + newline + 1)
            .unwrap_or(source.len());
        let mut physical_end = physical_next;
        while physical_end > physical_start
            && matches!(source.as_bytes()[physical_end - 1], b'\n' | b'\r')
        {
            physical_end -= 1;
        }

        let mut visual_start = physical_start;
        let mut first_visual_row = true;
        while visual_start < physical_end {
            let mut visual_end = (visual_start + MAX_PARAGRAPH_ROW_BYTES).min(physical_end);
            while !source.is_char_boundary(visual_end) {
                visual_end -= 1;
            }
            if visual_end < physical_end {
                let search_start = visual_start + MAX_PARAGRAPH_ROW_BYTES / 2;
                if let Some(boundary) = source[search_start..visual_end]
                    .rfind(|character: char| character == ' ' || character == '\t')
                {
                    visual_end = search_start
                        + boundary
                        + source[search_start + boundary..]
                            .chars()
                            .next()
                            .unwrap()
                            .len_utf8();
                }
            }
            let global_start = range.start.0 + visual_start as u64;
            rows.push(PreviewRow {
                block_id,
                content: ByteRange::new(global_start, range.start.0 + visual_end as u64),
                continuation: block_continuation,
                source_line: text.line_of_byte(ByteOffset(global_start)) + 1,
                show_line_number: first_visual_row,
                blank: false,
            });
            block_continuation = true;
            first_visual_row = false;
            visual_start = visual_end;
        }
        physical_start = physical_next;
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        document::RopeSnapshot,
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
}
