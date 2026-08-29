use std::path::PathBuf;

use crate::{
    document::{RopeSnapshot, TextSnapshot},
    org_syntax::{self, BlockKind},
};

use super::{
    ExportDiagnostic,
    model::{ExportBlock, ExportDocument, ExportInline},
};

pub(super) fn parse(
    source: &str,
    include_task_metadata: bool,
) -> (Result<ExportDocument, String>, Vec<ExportDiagnostic>) {
    let snapshot = match RopeSnapshot::from_utf8(source.as_bytes().to_vec()) {
        Ok(snapshot) => snapshot,
        Err(error) => return (Err(error.to_string()), Vec::new()),
    };
    let arena = org_syntax::parse(&snapshot);
    let nodes = arena.nodes();
    let mut document = ExportDocument::default();
    let mut diagnostics = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        let node = &nodes[index];
        let range = node.content.as_usize();
        let text = snapshot.copy_range(node.content);
        match &node.kind {
            BlockKind::Heading { level } => {
                let mut heading = text.trim().to_owned();
                if !include_task_metadata {
                    heading = clean_heading(&heading);
                }
                document.blocks.push(ExportBlock::Heading {
                    level: (*level).min(6) as u8,
                    content: org_inlines(&heading),
                    source: range,
                });
                if *level > 6 {
                    diagnostics.push(ExportDiagnostic::warning(
                        "org-heading-level",
                        format!("Org heading level {level} was clamped to 6"),
                    ));
                }
            }
            BlockKind::Paragraph | BlockKind::FixedWidth | BlockKind::FootnoteDefinition => {
                document.blocks.push(ExportBlock::Paragraph {
                    content: org_inlines(text.trim_end()),
                    source: range,
                });
            }
            BlockKind::Image { path } => document.blocks.push(ExportBlock::Image {
                path: PathBuf::from(path),
                alt: None,
                caption: None,
                source: range,
            }),
            BlockKind::SourceBlock { language } => {
                document.blocks.push(ExportBlock::Code {
                    language: language.clone(),
                    code: text,
                    source: range,
                });
            }
            BlockKind::ExampleBlock => {
                document.blocks.push(ExportBlock::Code {
                    language: None,
                    code: text,
                    source: range,
                });
            }
            BlockKind::QuoteBlock | BlockKind::VerseBlock | BlockKind::CenterBlock => {
                document.blocks.push(ExportBlock::Quote {
                    blocks: vec![ExportBlock::Paragraph {
                        content: org_inlines(text.trim()),
                        source: range.clone(),
                    }],
                    source: range,
                });
            }
            BlockKind::ListItem => {
                let start = index;
                let mut items = Vec::new();
                while index < nodes.len() && matches!(nodes[index].kind, BlockKind::ListItem) {
                    let item_text = snapshot.copy_range(nodes[index].content);
                    let body = clean_list_item(item_text.trim_end());
                    items.push(vec![ExportBlock::Paragraph {
                        content: org_inlines(&body),
                        source: nodes[index].content.as_usize(),
                    }]);
                    index += 1;
                }
                let ordered = source[nodes[start].content.as_usize()]
                    .trim_start()
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_digit());
                document.blocks.push(ExportBlock::List {
                    ordered,
                    items,
                    source: nodes[start].content.start.0 as usize
                        ..nodes[index - 1].content.end.0 as usize,
                });
                continue;
            }
            BlockKind::TableRow => {
                let start = index;
                let mut rows = Vec::new();
                while index < nodes.len() && matches!(nodes[index].kind, BlockKind::TableRow) {
                    let row = snapshot.copy_range(nodes[index].content);
                    let cells = row
                        .trim()
                        .trim_matches('|')
                        .split('|')
                        .map(|cell| org_inlines(cell.trim()))
                        .collect::<Vec<_>>();
                    let separator = cells.iter().all(|cell| {
                        cell.iter().all(|inline| match inline {
                            ExportInline::Text(text) => text
                                .chars()
                                .all(|character| matches!(character, '-' | '+' | ' ')),
                            _ => false,
                        })
                    });
                    if !separator {
                        rows.push(cells);
                    }
                    index += 1;
                }
                document.blocks.push(ExportBlock::Table {
                    rows,
                    source: nodes[start].content.start.0 as usize
                        ..nodes[index - 1].content.end.0 as usize,
                });
                continue;
            }
            BlockKind::Planning if include_task_metadata => {
                document.blocks.push(ExportBlock::Paragraph {
                    content: org_inlines(text.trim()),
                    source: range,
                });
            }
            BlockKind::HorizontalRule => {
                document.blocks.push(ExportBlock::Rule { source: range });
            }
            BlockKind::Keyword => apply_keyword(&mut document, text.trim()),
            BlockKind::ExportBlock { .. } => diagnostics.push(ExportDiagnostic::warning(
                "org-export-block-ignored",
                "Org export block was ignored",
            )),
            BlockKind::SpecialBlock { name } => {
                document.blocks.push(ExportBlock::Paragraph {
                    content: vec![ExportInline::Text(text.trim().to_owned())],
                    source: range,
                });
                diagnostics.push(ExportDiagnostic::warning(
                    "org-special-block-degraded",
                    format!("Org special block `{name}` was exported as text"),
                ));
            }
            BlockKind::BlankLine
            | BlockKind::Planning
            | BlockKind::Drawer { .. }
            | BlockKind::CommentBlock
            | BlockKind::Comment
            | BlockKind::Raw => {}
        }
        index += 1;
    }
    (Ok(document), diagnostics)
}

fn apply_keyword(document: &mut ExportDocument, line: &str) {
    let Some((key, value)) = line
        .strip_prefix("#+")
        .and_then(|rest| rest.split_once(':'))
    else {
        return;
    };
    let value = value.trim().to_owned();
    match key.to_ascii_uppercase().as_str() {
        "TITLE" => document.meta.title = Some(value),
        "AUTHOR" => document.meta.author = Some(value),
        "DATE" => document.meta.date = Some(value),
        "LANGUAGE" => document.meta.language = Some(value),
        _ => {}
    }
}

fn org_inlines(source: &str) -> Vec<ExportInline> {
    let parsed = org_syntax::inline::parse(source);
    if parsed.spans.is_empty() {
        return vec![ExportInline::Text(parsed.text)];
    }
    build_inlines(
        source,
        &parsed.text,
        &parsed.spans,
        0,
        parsed.text.len(),
        &[],
    )
}

fn build_inlines(
    source: &str,
    text: &str,
    spans: &[org_syntax::inline::InlineSpan],
    start: usize,
    end: usize,
    excluded: &[usize],
) -> Vec<ExportInline> {
    use org_syntax::inline::InlineKind;

    let mut output = Vec::new();
    let mut cursor = start;
    while cursor < end {
        let selected = spans
            .iter()
            .enumerate()
            .filter(|(index, _)| !excluded.contains(index))
            .filter(|span| {
                span.1.range.start == cursor && span.1.range.end <= end && span.1.range.end > cursor
            })
            .max_by_key(|(_, span)| span.range.end);
        if let Some((selected_index, span)) = selected {
            let mut child_excluded = excluded.to_vec();
            child_excluded.push(selected_index);
            let children = build_inlines(
                source,
                text,
                spans,
                span.range.start,
                span.range.end,
                &child_excluded,
            );
            let inline = match span.kind {
                InlineKind::Bold => ExportInline::Strong(children),
                InlineKind::Italic => ExportInline::Emphasis(children),
                InlineKind::Underline => ExportInline::Underline(children),
                InlineKind::Strike => ExportInline::Strike(children),
                InlineKind::Code | InlineKind::Verbatim => {
                    ExportInline::Code(text[span.range.clone()].to_owned())
                }
                InlineKind::Link => ExportInline::Link {
                    target: org_link_target(&source[span.source.clone()]),
                    label: children,
                },
                _ => ExportInline::Text(text[span.range.clone()].to_owned()),
            };
            output.push(inline);
            cursor = span.range.end;
        } else {
            let next = spans
                .iter()
                .enumerate()
                .filter(|(index, _)| !excluded.contains(index))
                .filter(|(_, span)| span.range.start > cursor && span.range.start < end)
                .map(|(_, span)| span.range.start)
                .min()
                .unwrap_or(end);
            output.push(ExportInline::Text(text[cursor..next].to_owned()));
            cursor = next;
        }
    }
    output
}

fn org_link_target(raw: &str) -> String {
    raw.strip_prefix("[[")
        .and_then(|raw| raw.strip_suffix("]]"))
        .and_then(|raw| raw.split_once("][").map(|(target, _)| target).or(Some(raw)))
        .unwrap_or(raw)
        .strip_prefix("file:")
        .unwrap_or_else(|| {
            raw.strip_prefix("[[file:")
                .and_then(|raw| raw.strip_suffix("]]"))
                .unwrap_or(raw)
        })
        .to_owned()
}

fn clean_heading(source: &str) -> String {
    let mut words = source.split_whitespace().peekable();
    if matches!(words.peek(), Some(&"TODO" | &"DONE")) {
        words.next();
    }
    if words
        .peek()
        .is_some_and(|word| word.len() == 4 && word.starts_with("[#") && word.ends_with(']'))
    {
        words.next();
    }
    let mut words = words.collect::<Vec<_>>();
    if words
        .last()
        .is_some_and(|word| word.starts_with(':') && word.ends_with(':'))
    {
        words.pop();
    }
    words.join(" ")
}

fn clean_list_item(source: &str) -> String {
    let trimmed = source.trim_start();
    let marker_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let mut body = trimmed[marker_end..].trim_start();
    let marker = match body.get(..3) {
        Some("[ ]") => Some("☐ "),
        Some("[-]") => Some("▣ "),
        Some("[X]") | Some("[x]") => Some("☑ "),
        _ => None,
    };
    if marker.is_some() {
        body = body[3..].trim_start();
    }
    format!("{}{}", marker.unwrap_or(""), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_org_without_repeating_heading_children() {
        let (document, diagnostics) =
            parse("#+TITLE: Notes\n* One\nBody\n** Two\n- [X] Done\n", true);
        let document = document.unwrap();
        assert!(diagnostics.is_empty());
        assert_eq!(document.meta.title.as_deref(), Some("Notes"));
        assert_eq!(
            document
                .blocks
                .iter()
                .filter(|block| matches!(block, ExportBlock::Heading { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn preserves_org_inline_styles_and_link_targets() {
        let inlines = org_inlines("*bold /nested/* and [[file:notes.org][notes]]");
        assert!(matches!(inlines.first(), Some(ExportInline::Strong(_))));
        assert!(inlines.iter().any(|inline| matches!(
            inline,
            ExportInline::Link { target, .. } if target == "notes.org"
        )));
    }
}
