use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::{
    document::{ByteRange, LineCursor, TextSnapshot},
    org_syntax::inline::{InlineKind, InlineSpan, InlineText},
};

use super::{CodeRowRole, PreviewRow};

#[derive(Clone, Debug)]
pub(super) enum MarkdownKind {
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
pub(super) struct MarkdownBlock {
    pub kind: MarkdownKind,
    pub source: ByteRange,
}

pub(super) fn parse_markdown(text: &dyn TextSnapshot) -> (Vec<MarkdownBlock>, Vec<PreviewRow>) {
    let mut blocks = Vec::new();
    let mut rows = Vec::new();
    let mut cursor = LineCursor::new(text);
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
        let id = blocks.len() as u32;
        let blank = matches!(kind, MarkdownKind::Blank);
        blocks.push(MarkdownBlock {
            kind,
            source: line.range,
        });
        rows.push(PreviewRow {
            block_id: id,
            content: text.revision_range(ByteRange::new(content_start, content_end)),
            continuation: false,
            show_line_number: true,
            blank,
        });
    }
    (blocks, rows)
}

pub(super) fn parse_markdown_inline(source: &str) -> InlineText {
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut result = InlineText::default();
    let mut stack: Vec<(TagEnd, InlineKind, usize, usize)> = Vec::new();
    for (event, source_range) in Parser::new_ext(source, options).into_offset_iter() {
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
    result.spans.sort_by_key(|span| span.range.start);
    result
}

fn fence_start(line: &str) -> Option<(char, usize, Option<String>)> {
    let marker = line.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let count = line.chars().take_while(|ch| *ch == marker).count();
    if count < 3 {
        return None;
    }
    let marker_bytes = marker.len_utf8() * count;
    let info = line[marker_bytes..].trim();
    if marker == '`' && info.contains('`') {
        return None;
    }
    let language = info
        .split_whitespace()
        .next()
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Some((marker, count, language))
}

fn is_closing_fence(line: &str, marker: char, opening_count: usize) -> bool {
    let count = line.chars().take_while(|ch| *ch == marker).count();
    count >= opening_count && line[marker.len_utf8() * count..].trim().is_empty()
}

fn atx_heading(line: &str) -> Option<(u16, usize)> {
    let count = line.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&count)
        || line
            .as_bytes()
            .get(count)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
    {
        return None;
    }
    let rest = line[count..].trim_start();
    Some((count as u16, line.len() - rest.len()))
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
    use crate::document::{RopeSnapshot, TextSnapshot};

    #[test]
    fn parses_markdown_blocks_and_inline_markup() {
        let text = RopeSnapshot::from_utf8(
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
    fn fenced_code_preserves_fences_indentation_and_source_line_numbers() {
        let text =
            RopeSnapshot::from_utf8(b"before\n```rust\n    let value = 1;\n```\nafter\n".to_vec())
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
        let text =
            RopeSnapshot::from_utf8(b"```rust\n```not-a-close\nvalue\n```   \nafter\n".to_vec())
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
}
