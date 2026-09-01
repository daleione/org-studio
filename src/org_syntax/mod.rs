use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::Arc,
};

use crate::document::{
    ByteOffset, ByteRange, LineCursor, RevisionDelta, RevisionRange, TextSnapshot,
};

pub(crate) mod command;
pub mod inline;
pub(crate) mod list;

pub type BlockId = u32;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SyntaxId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyntaxCheckpoint {
    pub source: ByteOffset,
    pub block_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxPatch {
    pub old_blocks: std::ops::Range<usize>,
    pub new_blocks: std::ops::Range<usize>,
    pub old_source: ByteRange,
    pub new_source: ByteRange,
    pub reparsed_bytes: u64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum BlockKind {
    BlankLine,
    Heading { level: u16 },
    Paragraph,
    Image { path: Arc<str> },
    Planning,
    ListItem,
    TableRow,
    FixedWidth,
    FootnoteDefinition,
    Drawer { name: Arc<str> },
    SourceBlock { language: Option<Arc<str>> },
    ExampleBlock,
    QuoteBlock,
    VerseBlock,
    CenterBlock,
    CommentBlock,
    ExportBlock { backend: Option<Arc<str>> },
    SpecialBlock { name: Arc<str> },
    Keyword,
    Comment,
    HorizontalRule,
    Raw,
}

#[derive(Clone, Debug)]
pub struct BlockNode {
    pub syntax_id: SyntaxId,
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
    checkpoints: Arc<[SyntaxCheckpoint]>,
    syntax_index: Arc<HashMap<SyntaxId, BlockId>>,
    source_len: u64,
}

impl BlockArena {
    pub fn nodes(&self) -> &[BlockNode] {
        &self.nodes
    }

    pub fn checkpoints(&self) -> &[SyntaxCheckpoint] {
        &self.checkpoints
    }

    pub fn block_for_syntax_id(&self, syntax_id: SyntaxId) -> Option<BlockId> {
        self.syntax_index.get(&syntax_id).copied()
    }

    fn finish(&mut self, snapshot: &dyn TextSnapshot) {
        let mut occurrences = HashMap::<u64, u32>::new();
        for node in &mut self.nodes {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            node.kind.hash(&mut hasher);
            snapshot.copy_range(node.content).hash(&mut hasher);
            let fingerprint = hasher.finish();
            let occurrence = occurrences.entry(fingerprint).or_default();
            let mut identity = std::collections::hash_map::DefaultHasher::new();
            fingerprint.hash(&mut identity);
            occurrence.hash(&mut identity);
            node.syntax_id = SyntaxId(identity.finish().max(1));
            *occurrence = occurrence.wrapping_add(1);
        }
        ensure_unique_syntax_ids(&mut self.nodes);
        self.finish_checkpoints();
        self.finish_syntax_index();
        self.source_len = snapshot.len_bytes();
    }

    fn finish_syntax_index(&mut self) {
        self.syntax_index = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                BlockId::try_from(index)
                    .ok()
                    .map(|block_id| (node.syntax_id, block_id))
            })
            .collect::<HashMap<_, _>>()
            .into();
    }

    fn finish_checkpoints(&mut self) {
        let mut checkpoints = vec![SyntaxCheckpoint {
            source: ByteOffset(0),
            block_index: 0,
        }];
        checkpoints.extend(self.nodes.iter().enumerate().filter_map(|(index, node)| {
            matches!(node.kind, BlockKind::Heading { level: 1 }).then_some(SyntaxCheckpoint {
                source: node.source.start,
                block_index: index,
            })
        }));
        checkpoints.dedup_by_key(|checkpoint| checkpoint.source);
        self.checkpoints = checkpoints.into();
    }

    fn rebuild_links(&mut self) {
        self.last_child = vec![None; self.nodes.len()];
        for node in &mut self.nodes {
            node.first_child = None;
            node.next_sibling = None;
        }
        for id in 0..self.nodes.len() {
            let Some(parent) = self.nodes[id].parent else {
                continue;
            };
            let parent = parent as usize;
            if let Some(previous) = self.last_child[parent] {
                self.nodes[previous as usize].next_sibling = Some(id as BlockId);
            } else {
                self.nodes[parent].first_child = Some(id as BlockId);
            }
            self.last_child[parent] = Some(id as BlockId);
        }
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
    parse_range(snapshot, ByteRange::new(0, snapshot.len_bytes()))
        .expect("the full document is a valid parse range")
}

pub fn parse_range(snapshot: &dyn TextSnapshot, range: ByteRange) -> Option<BlockArena> {
    let mut arena = BlockArena::default();
    let mut cursor = LineCursor::within(snapshot, range)?;
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
            let parent = heading_stack.last().map(|(_, id)| *id);
            arena.push(BlockNode {
                syntax_id: SyntaxId::default(),
                kind: BlockKind::BlankLine,
                source: line.range,
                content: ByteRange::new(line.range.start.0, content_end(&line)),
                parent,
                first_child: None,
                next_sibling: None,
            });
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
                syntax_id: SyntaxId::default(),
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
                syntax_id: SyntaxId::default(),
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
                syntax_id: SyntaxId::default(),
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
                    syntax_id: SyntaxId::default(),
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
            syntax_id: SyntaxId::default(),
            kind,
            source: line.range,
            content: ByteRange::new(line.range.start.0, content_end(&line)),
            parent,
            first_child: None,
            next_sibling: None,
        });
    }

    arena.finish(snapshot);
    Some(arena)
}

pub fn parse_incremental(
    snapshot: &dyn TextSnapshot,
    previous: &BlockArena,
    deltas: &[RevisionDelta],
) -> Option<(BlockArena, SyntaxPatch)> {
    let first = deltas.first()?;
    if deltas
        .windows(2)
        .any(|pair| pair[0].after != pair[1].before)
    {
        return None;
    }
    let edit_start = first.edits.iter().map(|edit| edit.old.start).min()?;
    let checkpoint = previous
        .checkpoints
        .partition_point(|checkpoint| checkpoint.source <= edit_start)
        .saturating_sub(1);
    // Include the preceding top-level section so deleting or inserting a level-one heading
    // cannot leak parser state across the restart boundary. Keep one following section as the
    // convergence witness.
    let restart = checkpoint.saturating_sub(1);
    let convergence = (checkpoint + 2).min(previous.checkpoints.len());
    let old_start = previous.checkpoints.get(restart)?.source;
    let old_end = previous
        .checkpoints
        .get(convergence)
        .map_or(ByteOffset(previous.source_len), |checkpoint| {
            checkpoint.source
        });
    let old_block_start = previous.checkpoints.get(restart)?.block_index;
    let old_block_end = previous
        .checkpoints
        .get(convergence)
        .map_or(previous.nodes.len(), |checkpoint| checkpoint.block_index);
    let new_start = map_point(old_start, first.before, deltas, false)?;
    let new_end = map_point(old_end, first.before, deltas, true)?;
    edits_stay_within_region(old_start, old_end, first.before, deltas)?;
    let new_source = ByteRange::new(new_start.0, new_end.0);
    let mut replacement = parse_range(snapshot, new_source)?;
    let old_replaced = &previous.nodes[old_block_start..old_block_end];
    let same_shape = replacement.nodes.len() == old_replaced.len();
    let preserves_all_ids = same_shape
        && replacement
            .nodes
            .iter()
            .zip(old_replaced)
            .all(|(next, old)| next.kind == old.kind);
    if same_shape {
        for (next, old) in replacement.nodes.iter_mut().zip(old_replaced) {
            if next.kind == old.kind {
                next.syntax_id = old.syntax_id;
            }
        }
    }

    let mut nodes = previous.nodes[..old_block_start].to_vec();
    let replacement_start = nodes.len();
    nodes.extend(replacement.nodes.into_iter().map(|mut node| {
        let offset = replacement_start as BlockId;
        node.parent = node.parent.map(|id| id.saturating_add(offset));
        node.first_child = node.first_child.map(|id| id.saturating_add(offset));
        node.next_sibling = node.next_sibling.map(|id| id.saturating_add(offset));
        node
    }));
    let replacement_end = nodes.len();
    let suffix_start = replacement_end;
    let suffix_shift = i128::from(new_end.0) - i128::from(old_end.0);
    for mut node in previous.nodes[old_block_end..].iter().cloned() {
        node.source = shift_byte_range(node.source, suffix_shift)?;
        node.content = shift_byte_range(node.content, suffix_shift)?;
        node.parent = match node.parent {
            Some(parent) if parent as usize >= old_block_end => Some(
                (suffix_start + parent as usize - old_block_end)
                    .try_into()
                    .ok()?,
            ),
            Some(parent) if (parent as usize) < old_block_start => Some(parent),
            Some(_) => return None,
            None => None,
        };
        nodes.push(node);
    }
    let mut arena = BlockArena {
        nodes,
        last_child: Vec::new(),
        checkpoints: Arc::from([]),
        syntax_index: Arc::new(HashMap::new()),
        source_len: snapshot.len_bytes(),
    };
    if !preserves_all_ids {
        ensure_unique_syntax_ids(&mut arena.nodes);
    }
    if !same_shape {
        arena.rebuild_links();
    }
    arena.finish_checkpoints();
    arena.finish_syntax_index();
    Some((
        arena,
        SyntaxPatch {
            old_blocks: old_block_start..old_block_end,
            new_blocks: replacement_start..replacement_end,
            old_source: ByteRange::new(old_start.0, old_end.0),
            new_source,
            reparsed_bytes: new_end.0.saturating_sub(new_start.0),
        },
    ))
}

fn edits_stay_within_region(
    start: ByteOffset,
    end: ByteOffset,
    revision: crate::document::Revision,
    deltas: &[RevisionDelta],
) -> Option<()> {
    let mut region = RevisionRange::new(revision, ByteRange { start, end });
    for delta in deltas {
        if delta.before != region.revision
            || delta
                .edits
                .iter()
                .any(|edit| edit.old.start < region.range.start || edit.old.end > region.range.end)
        {
            return None;
        }
        let start = map_point(
            region.range.start,
            region.revision,
            std::slice::from_ref(delta),
            false,
        )?;
        let end = map_point(
            region.range.end,
            region.revision,
            std::slice::from_ref(delta),
            true,
        )?;
        region = RevisionRange::new(delta.after, ByteRange { start, end });
    }
    Some(())
}

fn shift_byte_range(range: ByteRange, shift: i128) -> Option<ByteRange> {
    Some(ByteRange::new(
        u64::try_from(i128::from(range.start.0) + shift).ok()?,
        u64::try_from(i128::from(range.end.0) + shift).ok()?,
    ))
}

fn ensure_unique_syntax_ids(nodes: &mut [BlockNode]) {
    let mut syntax_ids = HashSet::with_capacity(nodes.len());
    for (index, node) in nodes.iter_mut().enumerate() {
        if syntax_ids.insert(node.syntax_id) {
            continue;
        }
        let mut salt = 0_u64;
        loop {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            node.syntax_id.hash(&mut hasher);
            index.hash(&mut hasher);
            salt.hash(&mut hasher);
            let candidate = SyntaxId(hasher.finish().max(1));
            if syntax_ids.insert(candidate) {
                node.syntax_id = candidate;
                break;
            }
            salt = salt.wrapping_add(1);
        }
    }
}

fn map_point(
    point: ByteOffset,
    revision: crate::document::Revision,
    deltas: &[RevisionDelta],
    after_insertions: bool,
) -> Option<ByteOffset> {
    let mut mapped = RevisionRange::new(revision, ByteRange::new(point.0, point.0));
    for delta in deltas {
        let insertion_at_boundary = if after_insertions {
            delta
                .edits
                .iter()
                .filter(|edit| {
                    edit.old.start == mapped.range.start && edit.old.start == edit.old.end
                })
                .map(|edit| edit.new_len)
                .sum::<u64>()
        } else {
            0
        };
        mapped = delta.map_range(mapped).ok()?;
        mapped.range.start.0 = mapped.range.start.0.checked_add(insertion_at_boundary)?;
        mapped.range.end = mapped.range.start;
    }
    Some(mapped.range.start)
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
            let language = line.split_whitespace().nth(1).map(Arc::from);
            BlockKind::SourceBlock { language }
        }
        "example" => BlockKind::ExampleBlock,
        "quote" => BlockKind::QuoteBlock,
        "verse" => BlockKind::VerseBlock,
        "center" => BlockKind::CenterBlock,
        "comment" => BlockKind::CommentBlock,
        "export" => BlockKind::ExportBlock {
            backend: line.split_whitespace().nth(1).map(Arc::from),
        },
        _ => BlockKind::SpecialBlock {
            name: Arc::from(name),
        },
    };

    Some((kind, format!("#+end_{name}"), content_start))
}

fn drawer_start(line: &str, content_start: u64) -> Option<(Arc<str>, u64)> {
    if line.len() <= 2 || !line.starts_with(':') || !line.ends_with(':') {
        return None;
    }
    let name = line.trim_matches(':');
    if !matches!(name.to_ascii_uppercase().as_str(), "PROPERTIES" | "LOGBOOK") {
        return None;
    }
    Some((Arc::from(name), content_start))
}

fn classify_line(line: &str) -> BlockKind {
    if let Some(path) = standalone_image_path(line) {
        BlockKind::Image {
            path: Arc::from(path),
        }
    } else if line.starts_with("SCHEDULED:")
        || line.starts_with("DEADLINE:")
        || line.starts_with("CLOSED:")
    {
        BlockKind::Planning
    } else if line.starts_with('|') {
        BlockKind::TableRow
    } else if line.starts_with(": ") || line == ":" {
        BlockKind::FixedWidth
    } else if line.starts_with("[fn:") && line.contains(']') {
        BlockKind::FootnoteDefinition
    } else if is_list_item(line) {
        BlockKind::ListItem
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

fn standalone_image_path(line: &str) -> Option<&str> {
    let target = line.strip_prefix("[[file:")?.strip_suffix("]]")?;
    if target.contains("][") || target.is_empty() {
        return None;
    }
    let extension = target.rsplit_once('.')?.1;
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "avif"
            | "jpg"
            | "jpeg"
            | "png"
            | "gif"
            | "webp"
            | "tif"
            | "tiff"
            | "tga"
            | "dds"
            | "bmp"
            | "ico"
            | "hdr"
            | "exr"
            | "pbm"
            | "pam"
            | "ppm"
            | "pgm"
            | "ff"
            | "farbfeld"
            | "qoi"
            | "svg"
    )
    .then_some(target)
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
        || bytes.first().is_some_and(u8::is_ascii_alphabetic)
            && matches!(bytes.get(1), Some(b'.' | b')'))
            && bytes.get(2).is_some_and(u8::is_ascii_whitespace)
}

fn content_end(line: &crate::document::TextLine<'_>) -> u64 {
    let trimmed = line.text.trim_end_matches(['\r', '\n']);
    line.range.start.0 + trimmed.len() as u64
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::document::{
        ByteOffset, ByteRange, DocumentBuffer, DocumentSnapshot, EditTransaction,
        SharedTextSnapshot, TextEdit, TextSnapshot,
    };

    use super::{BlockArena, BlockKind, parse, parse_incremental};

    fn snapshot(text: &str) -> SharedTextSnapshot {
        Arc::new(DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap())
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
    fn recognizes_only_standalone_file_images() {
        let text = snapshot(
            "[[file:images/diagram.png]]\n[[file:images/diagram.png][diagram]]\n[[https://example.com/photo.png]]\n",
        );
        let arena = parse(text.as_ref());
        assert!(matches!(
            &arena.nodes()[0].kind,
            BlockKind::Image { path } if path.as_ref() == "images/diagram.png"
        ));
        assert!(matches!(arena.nodes()[1].kind, BlockKind::Paragraph));
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
        let text = snapshot("* SQL\n  #+BEGIN_SRC sql\n  select 1;\n  #+END_SRC  \nAfter block.\n");
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
    fn syntax_ids_survive_unrelated_prefix_edits_and_checkpoints_are_safe() {
        let before = snapshot("preamble\n* One\nbody\n* Two\nbody\n");
        let after = snapshot("new preamble\npreamble\n* One\nbody\n* Two\nbody\n");
        let before = parse(before.as_ref());
        let after = parse(after.as_ref());
        let heading_ids = |arena: &BlockArena| {
            arena
                .nodes()
                .iter()
                .filter(|node| matches!(node.kind, BlockKind::Heading { level: 1 }))
                .map(|node| node.syntax_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(heading_ids(&before), heading_ids(&after));
        assert_eq!(after.checkpoints().len(), 3);
        assert_eq!(after.checkpoints()[0].source, ByteOffset(0));
        assert!(
            after
                .checkpoints()
                .windows(2)
                .all(|pair| pair[0].source < pair[1].source)
        );
    }

    #[test]
    fn incremental_parse_matches_full_parse_and_restarts_at_safe_checkpoints() {
        let source = (0..100)
            .map(|index| format!("* Heading {index}\nbody {index}\n"))
            .collect::<String>();
        let mut buffer = DocumentBuffer::from_utf8(source.into_bytes()).unwrap();
        let before = buffer.snapshot();
        let previous = parse(&before);
        let text = before.copy_range(ByteRange::new(0, before.len_bytes()));
        let edit_start = text.find("body 50").unwrap() as u64 + 5;
        let delta = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(
                    ByteRange::new(edit_start, edit_start + 2),
                    "fifty",
                )],
            ))
            .unwrap();
        let after = buffer.snapshot();
        let full = parse(&after);
        let (incremental, patch) = parse_incremental(&after, &previous, &[delta]).unwrap();
        assert!(patch.reparsed_bytes < after.len_bytes() / 10);
        assert_eq!(incremental.nodes().len(), full.nodes().len());
        for (actual, expected) in incremental.nodes().iter().zip(full.nodes()) {
            assert_eq!(actual.kind, expected.kind);
            assert_eq!(actual.source, expected.source);
            assert_eq!(actual.content, expected.content);
            assert_eq!(actual.parent, expected.parent);
            assert_eq!(actual.first_child, expected.first_child);
            assert_eq!(actual.next_sibling, expected.next_sibling);
            assert_eq!(
                after.copy_range(actual.content),
                after.copy_range(expected.content)
            );
        }
        assert!(
            incremental
                .nodes()
                .iter()
                .zip(previous.nodes())
                .all(|(next, old)| next.syntax_id == old.syntax_id)
        );
        assert_eq!(incremental.checkpoints(), full.checkpoints());
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
