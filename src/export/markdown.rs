use std::{iter::Peekable, ops::Range, path::PathBuf};

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use super::{
    ExportDiagnostic,
    model::{ExportBlock, ExportDocument, ExportInline},
};

type OwnedEvent<'a> = (Event<'a>, Range<usize>);
type Events<'a> = Peekable<std::vec::IntoIter<OwnedEvent<'a>>>;

pub(super) fn parse(source: &str) -> (Result<ExportDocument, String>, Vec<ExportDiagnostic>) {
    let (body, meta) = frontmatter(source);
    let options = Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let events = Parser::new_ext(body, options)
        .into_offset_iter()
        .collect::<Vec<_>>();
    let mut events = events.into_iter().peekable();
    let mut diagnostics = Vec::new();
    let blocks = parse_blocks(&mut events, None, &mut diagnostics);
    (Ok(ExportDocument { meta, blocks }), diagnostics)
}

fn frontmatter(source: &str) -> (&str, super::model::ExportMeta) {
    let mut meta = super::model::ExportMeta::default();
    let Some(rest) = source
        .strip_prefix("---\n")
        .or_else(|| source.strip_prefix("---\r\n"))
    else {
        return (source, meta);
    };
    let mut consumed = source.len() - rest.len();
    for line in rest.split_inclusive('\n') {
        let logical = line.trim_end_matches(['\r', '\n']);
        consumed += line.len();
        if logical == "---" {
            return (&source[consumed..], meta);
        }
        let Some((key, value)) = logical.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_owned();
        match key.trim().to_ascii_lowercase().as_str() {
            "title" => meta.title = Some(value),
            "author" => meta.author = Some(value),
            "date" => meta.date = Some(value),
            "lang" | "language" => meta.language = Some(value),
            "toc" => meta.toc = Some(matches!(value.as_str(), "true" | "yes" | "on")),
            _ => {}
        }
    }
    (source, super::model::ExportMeta::default())
}

fn parse_blocks<'a>(
    events: &mut Events<'a>,
    until: Option<TagEnd>,
    diagnostics: &mut Vec<ExportDiagnostic>,
) -> Vec<ExportBlock> {
    let mut blocks = Vec::new();
    while let Some((event, range)) = events.next() {
        if matches!(&event, Event::End(end) if Some(end) == until.as_ref()) {
            break;
        }
        match event {
            Event::Start(Tag::Paragraph) => {
                if let Some((Event::Start(Tag::Image { .. }), _)) = events.peek() {
                    let Some((
                        Event::Start(Tag::Image {
                            dest_url, title, ..
                        }),
                        image_range,
                    )) = events.next()
                    else {
                        unreachable!()
                    };
                    let alt = plain_inline_text(parse_inlines(events, TagEnd::Image, diagnostics));
                    if matches!(events.peek(), Some((Event::End(TagEnd::Paragraph), _))) {
                        events.next();
                    }
                    blocks.push(ExportBlock::Image {
                        path: PathBuf::from(dest_url.as_ref()),
                        alt: (!alt.is_empty()).then_some(alt),
                        caption: (!title.is_empty()).then(|| title.to_string()),
                        source: image_range,
                    });
                } else {
                    let content = parse_inlines(events, TagEnd::Paragraph, diagnostics);
                    blocks.push(ExportBlock::Paragraph {
                        content,
                        source: range,
                    });
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let end = TagEnd::Heading(level);
                let content = parse_inlines(events, end, diagnostics);
                blocks.push(ExportBlock::Heading {
                    level: heading_level(level),
                    content,
                    source: range,
                });
            }
            Event::Start(Tag::BlockQuote(kind)) => {
                let nested = parse_blocks(events, Some(TagEnd::BlockQuote(kind)), diagnostics);
                blocks.push(ExportBlock::Quote {
                    blocks: nested,
                    source: range,
                });
            }
            Event::Start(Tag::List(start)) => {
                let mut items = Vec::new();
                while let Some((next, _)) = events.peek() {
                    if matches!(next, Event::End(TagEnd::List(_))) {
                        events.next();
                        break;
                    }
                    if matches!(next, Event::Start(Tag::Item)) {
                        events.next();
                        items.push(parse_blocks(events, Some(TagEnd::Item), diagnostics));
                    } else {
                        events.next();
                    }
                }
                blocks.push(ExportBlock::List {
                    ordered: start.is_some(),
                    items,
                    source: range,
                });
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned),
                    CodeBlockKind::Indented => None,
                };
                let mut code = String::new();
                while let Some((event, _)) = events.next() {
                    match event {
                        Event::End(TagEnd::CodeBlock) => break,
                        Event::Text(text) | Event::Code(text) => code.push_str(&text),
                        Event::SoftBreak | Event::HardBreak => code.push('\n'),
                        _ => {}
                    }
                }
                blocks.push(ExportBlock::Code {
                    language,
                    code,
                    source: range,
                });
            }
            Event::Start(Tag::Table(_)) => {
                blocks.push(parse_table(events, range, diagnostics));
            }
            Event::Rule => blocks.push(ExportBlock::Rule { source: range }),
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                let label = plain_inline_text(parse_inlines(events, TagEnd::Image, diagnostics));
                blocks.push(ExportBlock::Image {
                    path: PathBuf::from(dest_url.as_ref()),
                    alt: (!label.is_empty()).then_some(label),
                    caption: (!title.is_empty()).then(|| title.to_string()),
                    source: range,
                });
            }
            Event::Html(_) | Event::InlineHtml(_) => diagnostics.push(ExportDiagnostic::warning(
                "markdown-raw-html",
                "raw HTML was omitted from export",
            )),
            Event::Text(text) if !text.trim().is_empty() => {
                blocks.push(ExportBlock::Paragraph {
                    content: vec![ExportInline::Text(text.to_string())],
                    source: range,
                });
            }
            Event::Code(text) => {
                blocks.push(ExportBlock::Paragraph {
                    content: vec![ExportInline::Code(text.to_string())],
                    source: range,
                });
            }
            _ => {}
        }
    }
    blocks
}

fn parse_table<'a>(
    events: &mut Events<'a>,
    source: Range<usize>,
    diagnostics: &mut Vec<ExportDiagnostic>,
) -> ExportBlock {
    let mut rows = Vec::new();
    let mut current_row = Vec::new();
    while let Some((event, _)) = events.next() {
        match event {
            Event::End(TagEnd::Table) => break,
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => current_row.clear(),
            Event::Start(Tag::TableCell) => {
                current_row.push(parse_inlines(events, TagEnd::TableCell, diagnostics));
            }
            Event::End(TagEnd::TableHead) | Event::End(TagEnd::TableRow) => {
                if !current_row.is_empty() {
                    rows.push(std::mem::take(&mut current_row));
                }
            }
            _ => {}
        }
    }
    ExportBlock::Table { rows, source }
}

fn parse_inlines<'a>(
    events: &mut Events<'a>,
    until: TagEnd,
    diagnostics: &mut Vec<ExportDiagnostic>,
) -> Vec<ExportInline> {
    let mut inlines = Vec::new();
    while let Some((event, _)) = events.next() {
        if matches!(&event, Event::End(end) if *end == until) {
            break;
        }
        match event {
            Event::Text(text) => inlines.push(ExportInline::Text(text.to_string())),
            Event::Code(text) => inlines.push(ExportInline::Code(text.to_string())),
            Event::SoftBreak => inlines.push(ExportInline::Text(" ".into())),
            Event::HardBreak => inlines.push(ExportInline::LineBreak),
            Event::TaskListMarker(checked) => {
                inlines.push(ExportInline::Text(if checked { "☑ " } else { "☐ " }.into()))
            }
            Event::Start(Tag::Strong) => inlines.push(ExportInline::Strong(parse_inlines(
                events,
                TagEnd::Strong,
                diagnostics,
            ))),
            Event::Start(Tag::Emphasis) => inlines.push(ExportInline::Emphasis(parse_inlines(
                events,
                TagEnd::Emphasis,
                diagnostics,
            ))),
            Event::Start(Tag::Strikethrough) => inlines.push(ExportInline::Strike(parse_inlines(
                events,
                TagEnd::Strikethrough,
                diagnostics,
            ))),
            Event::Start(Tag::Link { dest_url, .. }) => {
                let label = parse_inlines(events, TagEnd::Link, diagnostics);
                inlines.push(ExportInline::Link {
                    target: dest_url.to_string(),
                    label,
                });
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                let label = plain_inline_text(parse_inlines(events, TagEnd::Image, diagnostics));
                inlines.push(ExportInline::Link {
                    target: dest_url.to_string(),
                    label: vec![ExportInline::Text(label)],
                });
                diagnostics.push(ExportDiagnostic::warning(
                    "inline-image-degraded",
                    "inline image was exported as a link",
                ));
            }
            Event::InlineMath(math) | Event::DisplayMath(math) => {
                inlines.push(ExportInline::Code(math.to_string()));
                diagnostics.push(ExportDiagnostic::warning(
                    "math-degraded",
                    "math was exported as code",
                ));
            }
            Event::Html(_) | Event::InlineHtml(_) => diagnostics.push(ExportDiagnostic::warning(
                "markdown-raw-html",
                "raw HTML was omitted from export",
            )),
            _ => {}
        }
    }
    inlines
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn plain_inline_text(inlines: Vec<ExportInline>) -> String {
    fn append(out: &mut String, inline: ExportInline) {
        match inline {
            ExportInline::Text(text) | ExportInline::Code(text) => out.push_str(&text),
            ExportInline::Strong(children)
            | ExportInline::Emphasis(children)
            | ExportInline::Underline(children)
            | ExportInline::Strike(children) => {
                for child in children {
                    append(out, child);
                }
            }
            ExportInline::Link { label, .. } => {
                for child in label {
                    append(out, child);
                }
            }
            ExportInline::LineBreak => out.push('\n'),
        }
    }
    let mut out = String::new();
    for inline in inlines {
        append(&mut out, inline);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_structured_markdown() {
        let (document, diagnostics) =
            parse("# Title\n\n- one\n- **two**\n\n```rust\nfn main() {}\n```\n");
        let document = document.unwrap();
        assert!(diagnostics.is_empty());
        assert!(matches!(
            document.blocks[0],
            ExportBlock::Heading { level: 1, .. }
        ));
        assert!(
            document
                .blocks
                .iter()
                .any(|block| matches!(block, ExportBlock::List { .. }))
        );
        assert!(
            document
                .blocks
                .iter()
                .any(|block| matches!(block, ExportBlock::Code { .. }))
        );
    }

    #[test]
    fn extracts_supported_frontmatter() {
        let (document, _) = parse("---\ntitle: Notes\nauthor: Ada\ntoc: true\n---\n# Body\n");
        let document = document.unwrap();
        assert_eq!(document.meta.title.as_deref(), Some("Notes"));
        assert_eq!(document.meta.author.as_deref(), Some("Ada"));
        assert_eq!(document.meta.toc, Some(true));
    }
}
