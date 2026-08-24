use crate::document::{ByteOffset, ByteRange, LineCursor, TextSnapshot};

pub mod inline;

pub type BlockId = u32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockKind {
    Heading { level: u16 },
    Paragraph,
    ListItem,
    TableRow,
    Drawer { name: String },
    SourceBlock { language: Option<String> },
    ExampleBlock,
    QuoteBlock,
    Keyword,
    Comment,
    HorizontalRule,
    Raw,
}

#[derive(Clone, Debug)]
pub struct BlockNode {
    pub kind: BlockKind,
    pub source: ByteRange,
    pub content: ByteRange,
    pub parent: Option<BlockId>,
    pub first_child: Option<BlockId>,
    pub next_sibling: Option<BlockId>,
}

#[derive(Default)]
pub struct BlockArena {
    nodes: Vec<BlockNode>,
    last_child: Vec<Option<BlockId>>,
}

impl BlockArena {
    pub fn nodes(&self) -> &[BlockNode] {
        &self.nodes
    }

    fn push(&mut self, mut node: BlockNode) -> BlockId {
        let id = self.nodes.len() as BlockId;
        node.first_child = None;
        node.next_sibling = None;
        self.nodes.push(node);
        self.last_child.push(None);

        if let Some(parent) = self.nodes[id as usize].parent {
            let parent_index = parent as usize;
            if let Some(previous) = self.last_child[parent_index] {
                self.nodes[previous as usize].next_sibling = Some(id);
            } else {
                self.nodes[parent_index].first_child = Some(id);
            }
            self.last_child[parent_index] = Some(id);
        }

        id
    }
}

pub fn parse(snapshot: &dyn TextSnapshot) -> BlockArena {
    let mut arena = BlockArena::default();
    let mut cursor = LineCursor::new(snapshot);
    let mut heading_stack: Vec<(u16, BlockId)> = Vec::new();
    let mut open_block: Option<(BlockId, String)> = None;
    let mut paragraph: Option<BlockId> = None;

    while let Some(line) = cursor.next_line() {
        let logical = line.text.trim_end_matches(['\r', '\n']);

        if let Some((block_id, end_marker)) = open_block.as_ref() {
            let node = &mut arena.nodes[*block_id as usize];
            node.source.end = line.range.end;
            if logical.trim().eq_ignore_ascii_case(end_marker) {
                node.content.end = line.range.start;
                open_block = None;
            } else {
                node.content.end = line.range.end;
            }
            continue;
        }

        if logical.trim().is_empty() {
            paragraph = None;
            continue;
        }

        if let Some((level, content_start)) = heading(logical, line.range.start.0) {
            paragraph = None;
            while heading_stack
                .last()
                .is_some_and(|(open_level, _)| *open_level >= level)
            {
                heading_stack.pop();
            }

            for (_, id) in &heading_stack {
                arena.nodes[*id as usize].source.end = line.range.end;
            }

            let parent = heading_stack.last().map(|(_, id)| *id);
            let id = arena.push(BlockNode {
                kind: BlockKind::Heading { level },
                source: line.range,
                content: ByteRange::new(content_start, content_end(&line)),
                parent,
                first_child: None,
                next_sibling: None,
            });
            heading_stack.push((level, id));
            continue;
        }

        for (_, id) in &heading_stack {
            arena.nodes[*id as usize].source.end = line.range.end;
        }

        let parent = heading_stack.last().map(|(_, id)| *id);
        let trimmed = logical.trim_start();

        if let Some((name, content_start)) = drawer_start(trimmed, line.range.end.0) {
            paragraph = None;
            let id = arena.push(BlockNode {
                kind: BlockKind::Drawer { name },
                source: line.range,
                content: ByteRange::new(content_start, content_start),
                parent,
                first_child: None,
                next_sibling: None,
            });
            open_block = Some((id, ":END:".to_owned()));
            continue;
        }

        if let Some((kind, end_marker, content_start)) = block_start(trimmed, line.range.end.0) {
            paragraph = None;
            let id = arena.push(BlockNode {
                kind,
                source: line.range,
                content: ByteRange::new(content_start, content_start),
                parent,
                first_child: None,
                next_sibling: None,
            });
            open_block = Some((id, end_marker));
            continue;
        }

        let kind = classify_line(trimmed);
        if kind == BlockKind::Paragraph {
            if let Some(id) = paragraph {
                let node = &mut arena.nodes[id as usize];
                node.source.end = line.range.end;
                node.content.end = ByteOffset(content_end(&line));
            } else {
                let id = arena.push(BlockNode {
                    kind,
                    source: line.range,
                    content: ByteRange::new(line.range.start.0, content_end(&line)),
                    parent,
                    first_child: None,
                    next_sibling: None,
                });
                paragraph = Some(id);
            }
            continue;
        }

        paragraph = None;
        arena.push(BlockNode {
            kind,
            source: line.range,
            content: ByteRange::new(line.range.start.0, content_end(&line)),
            parent,
            first_child: None,
            next_sibling: None,
        });
    }

    arena
}

fn heading(line: &str, line_start: u64) -> Option<(u16, u64)> {
    let stars = line.bytes().take_while(|byte| *byte == b'*').count();
    if stars == 0
        || !line
            .as_bytes()
            .get(stars)
            .is_some_and(u8::is_ascii_whitespace)
    {
        return None;
    }

    let content = line[stars..].trim_start_matches([' ', '\t']);
    let content_start = line_start + (line.len() - content.len()) as u64;
    Some((stars.min(u16::MAX as usize) as u16, content_start))
}

fn block_start(line: &str, content_start: u64) -> Option<(BlockKind, String, u64)> {
    let lower = line.to_ascii_lowercase();
    let rest = lower.strip_prefix("#+begin_")?;
    let name_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let name = &rest[..name_end];
    let kind = match name {
        "src" => {
            let language = line.split_whitespace().nth(1).map(ToOwned::to_owned);
            BlockKind::SourceBlock { language }
        }
        "example" => BlockKind::ExampleBlock,
        "quote" => BlockKind::QuoteBlock,
        _ => BlockKind::Raw,
    };

    Some((kind, format!("#+end_{name}"), content_start))
}

fn drawer_start(line: &str, content_start: u64) -> Option<(String, u64)> {
    if line.len() <= 2 || !line.starts_with(':') || !line.ends_with(':') {
        return None;
    }
    let name = line.trim_matches(':');
    if name.eq_ignore_ascii_case("END") || name.contains(':') {
        return None;
    }
    Some((name.to_owned(), content_start))
}

fn classify_line(line: &str) -> BlockKind {
    if line.starts_with('|') {
        BlockKind::TableRow
    } else if is_list_item(line) {
        BlockKind::ListItem
    } else if line.starts_with(':') && line.ends_with(':') && line.len() > 2 {
        BlockKind::Drawer {
            name: line.trim_matches(':').to_owned(),
        }
    } else if line.starts_with("#+") {
        BlockKind::Keyword
    } else if line.starts_with('#') {
        BlockKind::Comment
    } else if line.bytes().filter(|byte| *byte == b'-').count() >= 5
        && line
            .bytes()
            .all(|byte| byte == b'-' || byte.is_ascii_whitespace())
    {
        BlockKind::HorizontalRule
    } else {
        BlockKind::Paragraph
    }
}

fn is_list_item(line: &str) -> bool {
    let bytes = line.as_bytes();
    if matches!(bytes.first(), Some(b'-' | b'+' | b'*')) {
        return bytes.get(1).is_some_and(u8::is_ascii_whitespace);
    }

    let digits = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    digits > 0
        && matches!(bytes.get(digits), Some(b'.' | b')'))
        && bytes.get(digits + 1).is_some_and(u8::is_ascii_whitespace)
}

fn content_end(line: &crate::document::TextLine<'_>) -> u64 {
    let trimmed = line.text.trim_end_matches(['\r', '\n']);
    line.range.start.0 + trimmed.len() as u64
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::document::{RopeSnapshot, SharedTextSnapshot};

    use super::{BlockKind, parse};

    fn snapshot(text: &str) -> SharedTextSnapshot {
        Arc::new(RopeSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap())
    }

    #[test]
    fn parses_heading_hierarchy_and_blocks() {
        let text = snapshot(
            "* Project\nIntro text.\n** Tasks\n- [ ] First\n#+begin_src rust\nfn main() {}\n#+end_src\n",
        );
        let arena = parse(text.as_ref());
        let nodes = arena.nodes();

        assert_eq!(nodes.len(), 5);
        assert!(matches!(nodes[0].kind, BlockKind::Heading { level: 1 }));
        assert_eq!(nodes[1].parent, Some(0));
        assert!(matches!(nodes[2].kind, BlockKind::Heading { level: 2 }));
        assert_eq!(nodes[2].parent, Some(0));
        assert!(matches!(nodes[3].kind, BlockKind::ListItem));
        assert!(matches!(nodes[4].kind, BlockKind::SourceBlock { .. }));
    }

    #[test]
    fn every_range_is_valid_utf8_and_inside_document() {
        let text = snapshot("* 中文 😀\r\nParagraph with /style/.\r\n| a | b |\r\n");
        let arena = parse(text.as_ref());

        for node in arena.nodes() {
            assert!(node.source.start <= node.source.end);
            assert!(node.source.end.0 <= text.len_bytes());
            assert!(node.content.start <= node.content.end);
            assert!(node.content.end.0 <= text.len_bytes());
            let _ = text.copy_range(node.content);
        }
    }

    #[test]
    fn groups_drawer_and_tolerates_unclosed_blocks() {
        let text = snapshot(":PROPERTIES:\n:ID: example\n:END:\n#+begin_quote\nunclosed\n");
        let arena = parse(text.as_ref());
        let nodes = arena.nodes();

        assert_eq!(nodes.len(), 2);
        assert!(matches!(nodes[0].kind, BlockKind::Drawer { .. }));
        assert!(matches!(nodes[1].kind, BlockKind::QuoteBlock));
        assert_eq!(text.copy_range(nodes[0].content), ":ID: example\n");
        assert_eq!(text.copy_range(nodes[1].content), "unclosed\n");
    }

    #[test]
    fn closes_indented_blocks_with_indented_or_trailing_space_end_markers() {
        let text = snapshot(
            "* SQL\n  #+BEGIN_SRC sql\n  select 1;\n  #+END_SRC  \nAfter block.\n",
        );
        let arena = parse(text.as_ref());
        let nodes = arena.nodes();

        assert_eq!(nodes.len(), 3);
        assert!(matches!(nodes[0].kind, BlockKind::Heading { level: 1 }));
        assert!(matches!(nodes[1].kind, BlockKind::SourceBlock { .. }));
        assert_eq!(text.copy_range(nodes[1].content), "  select 1;\n");
        assert!(matches!(nodes[2].kind, BlockKind::Paragraph));
        assert_eq!(text.copy_range(nodes[2].content), "After block.");
    }

    #[test]
    fn heading_sections_contain_descendants_and_siblings_do_not_overlap() {
        let text = snapshot("* One\nbody\n** Child\nchild body\n* Two\nbody\n");
        let arena = parse(text.as_ref());
        let nodes = arena.nodes();
        assert!(nodes[0].source.start <= nodes[2].source.start);
        assert!(nodes[0].source.end >= nodes[2].source.end);
        assert!(nodes[0].source.end <= nodes[4].source.start);
    }

    #[test]
    fn arbitrary_valid_utf8_does_not_panic_or_escape_document() {
        let alphabet = ["a", "*", "\n", "中", "😀", ":", "#", "[", "\\", "\u{301}"];
        let mut state = 0x9e37_79b9_u32;
        for _ in 0..512 {
            let mut input = String::new();
            for _ in 0..128 {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                input.push_str(alphabet[state as usize % alphabet.len()]);
            }
            let text = snapshot(&input);
            let arena = parse(text.as_ref());
            for node in arena.nodes() {
                assert!(node.source.end.0 <= text.len_bytes());
                assert!(node.content.end.0 <= text.len_bytes());
                let _ = text.copy_range(node.content);
            }
        }
    }
}
