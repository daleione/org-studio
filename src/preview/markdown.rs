use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::{
    document::{
        ByteRange, LineCursor, RevisionDelta, RevisionRange, TextSnapshot,
        markdown::{atx_heading, fence_start, is_closing_fence},
    },
    org_syntax::inline::{InlineKind, InlineSpan, InlineText},
};

use super::{CodeRowRole, PreviewRow};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MarkdownKind {
    Blank,
    Heading {
        level: u16,
    },
    Paragraph,
    ListItem,
    Quote,
    Code {
        language: Option<String>,
        role: CodeRowRole,
    },
    TableRow,
    HorizontalRule,
    Image {
        path: String,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct MarkdownBlock {
    pub kind: MarkdownKind,
    pub source: ByteRange,
}

#[derive(Clone, Debug)]
pub(crate) struct MarkdownPatch {
    pub(crate) old_blocks: std::ops::Range<usize>,
    pub(crate) new_blocks: std::ops::Range<usize>,
    pub(crate) reparsed_bytes: u64,
}

pub(crate) fn parse_markdown(text: &dyn TextSnapshot) -> (Vec<MarkdownBlock>, Vec<PreviewRow>) {
    parse_markdown_range(text, ByteRange::new(0, text.len_bytes()), 0)
        .expect("the complete document is a valid Markdown parse range")
}

/// Reparse a bounded line region when its surrounding parser state is known to be neutral.
/// Fenced code, tables, images and line-count changes deliberately fall back to the full parser;
/// those constructs have dependencies outside a single physical line.
pub(crate) fn parse_markdown_incremental(
    text: &dyn TextSnapshot,
    previous_blocks: &[MarkdownBlock],
    previous_rows: &[PreviewRow],
    deltas: &[RevisionDelta],
) -> Option<(Vec<MarkdownBlock>, Vec<PreviewRow>, MarkdownPatch)> {
    let [delta] = deltas else {
        return None;
    };
    if previous_blocks.len() != previous_rows.len()
        || delta.after != text.revision()
        || previous_blocks.is_empty()
    {
        return None;
    }
    let edit_start = delta.edits.iter().map(|edit| edit.old.start).min()?;
    let edit_end = delta.edits.iter().map(|edit| edit.old.end).max()?;
    let first = previous_blocks
        .partition_point(|block| block.source.end < edit_start)
        .saturating_sub(1);
    let last = previous_blocks
        .partition_point(|block| block.source.start <= edit_end)
        .saturating_add(1)
        .min(previous_blocks.len());
    let old_blocks = first..last.max(first + 1).min(previous_blocks.len());
    if previous_blocks[old_blocks.clone()]
        .iter()
        .any(|block| has_external_dependency(&block.kind))
    {
        return None;
    }
    let old_source = ByteRange {
        start: previous_blocks[old_blocks.start].source.start,
        end: previous_blocks[old_blocks.end - 1].source.end,
    };
    let new_source = ByteRange {
        start: map_delta_boundary(old_source.start, delta, false)?,
        end: map_delta_boundary(old_source.end, delta, true)?,
    };
    let (replacement_blocks, replacement_rows) =
        parse_markdown_range(text, new_source, old_blocks.start as u32)?;
    if replacement_blocks.len() != old_blocks.len()
        || replacement_blocks
            .iter()
            .any(|block| has_external_dependency(&block.kind))
    {
        return None;
    }

    let map_range = |range| {
        delta
            .map_range(RevisionRange::new(delta.before, range))
            .ok()
            .map(|mapped| mapped.range)
    };
    let mut blocks = Vec::with_capacity(previous_blocks.len());
    let mut rows = Vec::with_capacity(previous_rows.len());
    for index in 0..previous_blocks.len() {
        if old_blocks.contains(&index) {
            let local = index - old_blocks.start;
            blocks.push(replacement_blocks[local].clone());
            rows.push(replacement_rows[local]);
        } else {
            blocks.push(MarkdownBlock {
                kind: previous_blocks[index].kind.clone(),
                source: map_range(previous_blocks[index].source)?,
            });
            let mut row = previous_rows[index];
            row.content = delta.map_range(row.content).ok()?;
            rows.push(row);
        }
    }
    Some((
        blocks,
        rows,
        MarkdownPatch {
            old_blocks: old_blocks.clone(),
            new_blocks: old_blocks,
            reparsed_bytes: new_source.len(),
        },
    ))
}

fn parse_markdown_range(
    text: &dyn TextSnapshot,
    range: ByteRange,
    block_offset: u32,
) -> Option<(Vec<MarkdownBlock>, Vec<PreviewRow>)> {
    let mut blocks = Vec::new();
    let mut rows = Vec::new();
    let mut cursor = LineCursor::within(text, range)?;
    let mut fence: Option<(char, usize, Option<String>)> = None;

    while let Some(line) = cursor.next_line() {
        let logical = line.text.trim_end_matches(['\r', '\n']);
        let trimmed = logical.trim_start();
        let leading = logical.len() - trimmed.len();
        let line_end = line.range.start.0 + logical.len() as u64;
        let (kind, content_start, content_end) = if let Some((marker, count, language)) = &fence {
            let closes = leading <= 3 && is_closing_fence(trimmed, *marker, *count);
            let kind = MarkdownKind::Code {
                language: language.clone(),
                role: if closes {
                    CodeRowRole::Close
                } else {
                    CodeRowRole::Body
                },
            };
            if closes {
                fence = None;
            }
            (kind, line.range.start.0, line_end)
        } else if leading <= 3
            && let Some((marker, count, language)) = fence_start(trimmed)
        {
            fence = Some((marker, count, language.clone()));
            (
                MarkdownKind::Code {
                    language,
                    role: CodeRowRole::Open,
                },
                line.range.start.0,
                line_end,
            )
        } else if trimmed.is_empty() {
            (MarkdownKind::Blank, line.range.start.0, line_end)
        } else if let Some((level, offset)) = atx_heading(trimmed) {
            (
                MarkdownKind::Heading { level },
                line.range.start.0 + leading as u64 + offset as u64,
                line_end,
            )
        } else if let Some(path) = standalone_image(trimmed) {
            (MarkdownKind::Image { path }, line.range.start.0, line_end)
        } else if is_horizontal_rule(trimmed) {
            (MarkdownKind::HorizontalRule, line.range.start.0, line_end)
        } else if let Some(offset) = quote_offset(trimmed) {
            (
                MarkdownKind::Quote,
                line.range.start.0 + leading as u64 + offset as u64,
                line_end,
            )
        } else if is_list_item(trimmed) {
            (
                MarkdownKind::ListItem,
                line.range.start.0 + leading as u64,
                line_end,
            )
        } else if trimmed.starts_with('|') && trimmed.ends_with('|') {
            (
                MarkdownKind::TableRow,
                line.range.start.0 + leading as u64,
                line_end,
            )
        } else {
            (
                MarkdownKind::Paragraph,
                line.range.start.0 + leading as u64,
                line_end,
            )
        };
        let id = block_offset.checked_add(blocks.len().try_into().ok()?)?;
        let blank = matches!(kind, MarkdownKind::Blank);
        blocks.push(MarkdownBlock {
            kind,
            source: line.range,
        });
        rows.push(PreviewRow {
            block_id: id,
            content: text.revision_range(ByteRange::new(content_start, content_end)),
            continuation: false,
            blank,
        });
    }
    Some((blocks, rows))
}

fn has_external_dependency(kind: &MarkdownKind) -> bool {
    matches!(
        kind,
        MarkdownKind::Code { .. } | MarkdownKind::TableRow | MarkdownKind::Image { .. }
    )
}

fn map_delta_boundary(
    point: crate::document::ByteOffset,
    delta: &RevisionDelta,
    after_insertions: bool,
) -> Option<crate::document::ByteOffset> {
    let mut shift = 0_i128;
    for edit in delta.edits.iter().copied() {
        let insertion = edit.old.start == edit.old.end;
        if edit.old.end < point || (edit.old.end == point && (!insertion || after_insertions)) {
            shift += i128::from(edit.new_len) - i128::from(edit.old.len());
        }
    }
    u64::try_from(i128::from(point.0) + shift)
        .ok()
        .map(crate::document::ByteOffset)
}

pub(crate) fn parse_markdown_inline(source: &str) -> InlineText {
    // pulldown-cmark always starts in block context. A heading title such as `2. Design`
    // would therefore be reinterpreted as an ordered-list item and lose its marker. Prefixing
    // one ordinary paragraph fragment forces the parser into inline context; both rendered and
    // source ranges are rebased before the result leaves this function.
    const INLINE_CONTEXT_PREFIX: &str = "p: ";
    let wrapped = format!("{INLINE_CONTEXT_PREFIX}{source}");
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut result = InlineText::default();
    let mut stack: Vec<(TagEnd, InlineKind, usize, usize)> = Vec::new();
    for (event, source_range) in Parser::new_ext(&wrapped, options).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                let kind = match &tag {
                    Tag::Strong => Some(InlineKind::Bold),
                    Tag::Emphasis => Some(InlineKind::Italic),
                    Tag::Strikethrough => Some(InlineKind::Strike),
                    Tag::Link { .. } => Some(InlineKind::Link),
                    _ => None,
                };
                if let Some(kind) = kind {
                    stack.push((tag.to_end(), kind, result.text.len(), source_range.start));
                }
            }
            Event::End(end) => {
                if let Some(index) = stack.iter().rposition(|(expected, ..)| *expected == end) {
                    let (_, kind, start, source_start) = stack.remove(index);
                    if start < result.text.len() {
                        result.spans.push(InlineSpan {
                            kind,
                            source: source_start..source_range.end,
                            range: start..result.text.len(),
                        });
                    }
                }
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                result.text.push_str(&text)
            }
            Event::Code(text) => {
                let start = result.text.len();
                result.text.push_str(&text);
                result.spans.push(InlineSpan {
                    kind: InlineKind::Code,
                    source: source_range,
                    range: start..result.text.len(),
                });
            }
            Event::SoftBreak => result.text.push(' '),
            Event::HardBreak => result.text.push('\n'),
            Event::TaskListMarker(checked) => {
                result.text.push_str(if checked { "[x] " } else { "[ ] " })
            }
            _ => {}
        }
    }
    if !result.text.starts_with(INLINE_CONTEXT_PREFIX) {
        return InlineText {
            text: source.to_owned(),
            spans: Vec::new(),
        };
    }
    result.text.drain(..INLINE_CONTEXT_PREFIX.len());
    result.spans.retain_mut(|span| {
        if span.range.start < INLINE_CONTEXT_PREFIX.len()
            || span.source.start < INLINE_CONTEXT_PREFIX.len()
        {
            return false;
        }
        span.range.start -= INLINE_CONTEXT_PREFIX.len();
        span.range.end -= INLINE_CONTEXT_PREFIX.len();
        span.source.start -= INLINE_CONTEXT_PREFIX.len();
        span.source.end -= INLINE_CONTEXT_PREFIX.len();
        true
    });
    result.spans.sort_by_key(|span| span.range.start);
    result
}

fn quote_offset(line: &str) -> Option<usize> {
    let rest = line.strip_prefix('>')?;
    Some(line.len() - rest.trim_start().len())
}

fn is_list_item(line: &str) -> bool {
    matches!(line.as_bytes(), [b'-' | b'+' | b'*', b' ' | b'\t', ..])
        || line.find('.').is_some_and(|dot| {
            dot > 0
                && line[..dot].bytes().all(|byte| byte.is_ascii_digit())
                && line
                    .as_bytes()
                    .get(dot + 1)
                    .is_some_and(u8::is_ascii_whitespace)
        })
}

fn is_horizontal_rule(line: &str) -> bool {
    let compact = line
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    compact.len() >= 3 && compact.chars().all(|ch| ch == '-')
}

fn standalone_image(line: &str) -> Option<String> {
    line.strip_prefix("![")?;
    let open = line.find("](")? + 2;
    let close = line.rfind(')')?;
    (close > open && close + 1 == line.len()).then(|| {
        line[open..close]
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{
        DocumentBuffer, DocumentSnapshot, EditTransaction, TextEdit, TextSnapshot,
    };

    #[test]
    fn parses_markdown_blocks_and_inline_markup() {
        let text = DocumentSnapshot::from_utf8(
            b"# Title\n\n- **bold** and [link](a.md)\n```rust\nfn main() {}\n```\n".to_vec(),
        )
        .unwrap();
        let (blocks, rows) = parse_markdown(&text);
        assert_eq!(blocks.len(), rows.len());
        assert!(matches!(blocks[0].kind, MarkdownKind::Heading { level: 1 }));
        assert!(matches!(blocks[2].kind, MarkdownKind::ListItem));
        let inline = parse_markdown_inline("**bold** and [link](a.md)");
        assert_eq!(inline.text, "bold and link");
        assert!(
            inline
                .spans
                .iter()
                .any(|span| span.kind == InlineKind::Bold)
        );
        assert!(
            inline
                .spans
                .iter()
                .any(|span| span.kind == InlineKind::Link)
        );
        assert_eq!(
            standalone_image("![diagram](images/a.png)"),
            Some("images/a.png".into())
        );
    }

    #[test]
    fn inline_context_preserves_ordered_markers_at_the_start_of_heading_text() {
        let inline = parse_markdown_inline("2. **语义化编辑器**");
        assert_eq!(inline.text, "2. 语义化编辑器");
        assert_eq!(inline.spans.len(), 1);
        assert_eq!(inline.spans[0].kind, InlineKind::Bold);
        assert_eq!(&inline.text[inline.spans[0].range.clone()], "语义化编辑器");
    }

    #[test]
    fn fenced_code_preserves_fences_indentation_and_source_line_numbers() {
        let text = DocumentSnapshot::from_utf8(
            b"before\n```rust\n    let value = 1;\n```\nafter\n".to_vec(),
        )
        .unwrap();
        let (blocks, rows) = parse_markdown(&text);

        assert!(matches!(
            blocks[1].kind,
            MarkdownKind::Code {
                role: CodeRowRole::Open,
                ..
            }
        ));
        assert!(matches!(
            blocks[2].kind,
            MarkdownKind::Code {
                role: CodeRowRole::Body,
                ..
            }
        ));
        assert!(matches!(
            blocks[3].kind,
            MarkdownKind::Code {
                role: CodeRowRole::Close,
                ..
            }
        ));
        assert_eq!(text.copy_range(rows[1].content.range), "```rust");
        assert_eq!(text.copy_range(rows[2].content.range), "    let value = 1;");
        assert_eq!(text.copy_range(rows[3].content.range), "```");
        assert_eq!(
            rows.iter()
                .map(|row| text.line_of_byte(row.content.range.start) + 1)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );
    }

    #[test]
    fn closing_fence_requires_only_marker_and_trailing_whitespace() {
        let text = DocumentSnapshot::from_utf8(
            b"```rust\n```not-a-close\nvalue\n```   \nafter\n".to_vec(),
        )
        .unwrap();
        let (blocks, _) = parse_markdown(&text);

        assert!(matches!(
            blocks[1].kind,
            MarkdownKind::Code {
                role: CodeRowRole::Body,
                ..
            }
        ));
        assert!(matches!(
            blocks[3].kind,
            MarkdownKind::Code {
                role: CodeRowRole::Close,
                ..
            }
        ));
        assert!(matches!(blocks[4].kind, MarkdownKind::Paragraph));
    }

    #[test]
    fn plain_text_edit_reparses_only_neighboring_markdown_lines() {
        let mut buffer =
            DocumentBuffer::from_utf8(b"# Heading\nbody one\n\nbody two\n".to_vec()).unwrap();
        let before = buffer.snapshot();
        let (blocks, rows) = parse_markdown(&before);
        let delta = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(ByteRange::new(15, 15), "x")],
            ))
            .unwrap();
        let (_, next_rows, patch) =
            parse_markdown_incremental(&buffer.snapshot(), &blocks, &rows, &[delta])
                .expect("plain line edits have a neutral incremental boundary");
        assert_eq!(next_rows.len(), rows.len());
        assert!(patch.reparsed_bytes < buffer.snapshot().len_bytes());
    }
}
