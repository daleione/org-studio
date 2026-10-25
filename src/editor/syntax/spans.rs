//! Theme-independent spans and their text paint runs.

use super::{EditorLineStyle, EditorStyleId, Language, language};
use crate::{
    org_syntax::inline::{BracketLink, InlineKind, InlineText},
    theme::Theme,
};
use gpui::{FontStyle, FontWeight, TextRun, UnderlineStyle, px, rgb, rgba};
use std::{ops::Range, path::Path, sync::Arc};

#[derive(Clone, Debug, Default)]
struct SpanStyle {
    color: Option<EditorColorToken>,
    weight: Option<FontWeight>,
    font_style: Option<FontStyle>,
    underline: bool,
    strikethrough: bool,
    /// Painted as a rounded pill behind the source text (Org trailing tags).
    pill: bool,
    /// `0xRRGGBBAA` document-authored hex literal, painted as an opaque swatch.
    swatch: Option<u32>,
    /// Classified link metadata (type + raw target) for hover/activation.
    link: Option<crate::links::LinkInfo>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::editor) enum EditorColorToken {
    Hidden,
    Foreground,
    InlineCode,
    Verbatim,
    InlineDelimiter,
    Link,
    LinkExternal,
    LinkFile,
    LinkInternal,
    LinkMail,
    LinkWarn,
    LinkOther,
    Date,
    Todo,
    TodoActive,
    TodoProject,
    Waiting,
    Done,
    Keyword,
    String,
    Comment,
    Type,
    Function,
    Constant,
    Number,
    Variable,
    Operator,
    Attribute,
    Meta,
}

impl EditorColorToken {
    pub(in crate::editor) fn resolve(self, theme: &Theme) -> u32 {
        match self {
            Self::Foreground => theme.foreground,
            Self::InlineCode => theme.inline_code,
            Self::Verbatim => theme.verbatim,
            Self::InlineDelimiter => {
                u32::from(rgb(theme.foreground_dim).blend(rgb(theme.background).opacity(0.30))) >> 8
            }
            Self::Hidden => theme.background,
            Self::Link => theme.link,
            Self::LinkExternal => theme.link_external,
            Self::LinkFile => theme.link_file,
            Self::LinkInternal => theme.link_internal,
            Self::LinkMail => theme.link_mail,
            Self::LinkWarn => theme.link_warn,
            Self::LinkOther => theme.link_other,
            Self::Date => theme.date,
            Self::Todo => theme.todo,
            Self::TodoActive => theme.todo_active,
            Self::TodoProject => theme.todo_project,
            Self::Waiting => theme.waiting,
            Self::Done => theme.done,
            Self::Keyword => theme.keyword,
            Self::String => theme.string,
            Self::Comment => theme.comment,
            Self::Type => theme.type_name,
            Self::Function => theme.function,
            Self::Constant => theme.constant,
            Self::Number => theme.number,
            Self::Variable => theme.variable,
            Self::Operator => theme.operator,
            Self::Attribute => theme.attribute,
            Self::Meta => theme.meta,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::editor) enum EditorSemanticWeight {
    #[default]
    Inherit,
    Normal,
    Semibold,
    Bold,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::editor) struct EditorSemanticSpan {
    pub(in crate::editor) bytes: Range<usize>,
    pub(in crate::editor) color: Option<EditorColorToken>,
    pub(in crate::editor) weight: EditorSemanticWeight,
    pub(in crate::editor) italic: bool,
    pub(in crate::editor) underline: bool,
    pub(in crate::editor) strikethrough: bool,
    /// Paint a rounded pill behind the span while keeping every source byte visible.
    pub(in crate::editor) pill: bool,
    /// `0xRRGGBBAA` hex literal painted as an opaque swatch behind the span.
    pub(in crate::editor) swatch: Option<u32>,
    /// Classified link metadata when this span is a link (hover/activation).
    pub(in crate::editor) link: Option<crate::links::LinkInfo>,
}

/// Restore the code accent on the delimiters of the span under the caret.
/// Returning the span (rather than the caret offset) keeps shaping cacheable while typing within it.
pub(in crate::editor) fn emphasize_inline_delimiters(
    spans: &mut [EditorSemanticSpan],
    caret: Option<usize>,
) -> Option<Range<usize>> {
    let caret = caret?;
    let range = spans
        .iter()
        .find(|span| {
            matches!(
                span.color,
                Some(EditorColorToken::InlineCode | EditorColorToken::Verbatim)
            ) && span.bytes.contains(&caret)
        })?
        .bytes
        .clone();
    let mut emphasized = false;
    for span in spans {
        if span.color == Some(EditorColorToken::InlineDelimiter)
            && span.bytes.start >= range.start
            && span.bytes.end <= range.end
        {
            // Removing the override reveals the enclosing code span's original color.
            span.color = None;
            emphasized = true;
        }
    }
    emphasized.then_some(range)
}

/// Produces paint runs from precomputed semantic spans without changing source text.
pub(in crate::editor) fn runs_from_spans(
    spans: &[EditorSemanticSpan],
    mut base: TextRun,
    line_style: &EditorLineStyle,
    marked: Option<Range<usize>>,
    theme: &Theme,
) -> Vec<TextRun> {
    // Apply the block face before inline overrides.
    match line_style.id {
        EditorStyleId::Heading(level) => {
            base.font.weight = line_style.metrics.font_weight;
            // JetBrains Mono substitutes `***` with a staggered asterism ligature.
            // Source headings must keep their level markers individually legible.
            base.font.features = gpui::FontFeatures::disable_ligatures();
            base.color =
                rgb(theme.heading[level.saturating_sub(1) as usize % theme.heading.len()]).into();
        }
        EditorStyleId::CodeBoundary => {
            base.color = rgb(theme.meta).into();
        }
        EditorStyleId::Code => {
            base.color = rgb(theme.code_foreground).into();
        }
        EditorStyleId::Quote => base.color = rgb(theme.quote).into(),
        EditorStyleId::Property | EditorStyleId::Meta => {}
        EditorStyleId::Comment => {
            base.color = rgb(theme.comment).into();
            base.font.style = FontStyle::Italic;
        }
        EditorStyleId::Table => base.color = rgb(theme.link).into(),
        EditorStyleId::List | EditorStyleId::Plain => {}
    }

    let mut boundaries = vec![0, base.len];
    for span in spans {
        boundaries.extend([span.bytes.start, span.bytes.end]);
    }
    if let Some(marked) = &marked {
        boundaries.extend([marked.start, marked.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
        .windows(2)
        .filter_map(|boundary| {
            let range = boundary[0]..boundary[1];
            if range.is_empty() {
                return None;
            }
            let mut run = TextRun {
                len: range.len(),
                ..base.clone()
            };
            for span in spans
                .iter()
                .filter(|span| span.bytes.start <= range.start && range.start < span.bytes.end)
            {
                if let Some(color) = span.color {
                    run.color = if color == EditorColorToken::Hidden {
                        gpui::transparent_black()
                    } else {
                        rgb(color.resolve(theme)).into()
                    };
                }
                run.font.weight = match span.weight {
                    EditorSemanticWeight::Inherit => run.font.weight,
                    EditorSemanticWeight::Normal => FontWeight::NORMAL,
                    EditorSemanticWeight::Semibold => FontWeight::SEMIBOLD,
                    EditorSemanticWeight::Bold => FontWeight::BOLD,
                };
                if span.italic {
                    run.font.style = FontStyle::Italic;
                }
                if span.underline {
                    run.underline = Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    });
                }
                if span.strikethrough {
                    run.strikethrough = Some(gpui::StrikethroughStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                    });
                }
            }
            if marked
                .as_ref()
                .is_some_and(|marked| marked.start < range.end && range.start < marked.end)
            {
                run.underline = Some(UnderlineStyle {
                    color: Some(run.color),
                    thickness: px(1.0),
                    wavy: false,
                });
            }
            Some(run)
        })
        .collect()
}

/// Recolors the glyphs that sit on a swatch so the literal stays readable on
/// its own fill. Kept separate from [`runs_from_spans`] because consumers that
/// never paint the swatch (the minimap raster, for example) must keep their
/// original text colors.
pub(in crate::editor) fn apply_swatch_text(
    runs: &mut [TextRun],
    spans: &[EditorSemanticSpan],
    theme: &Theme,
) {
    if !spans.iter().any(|span| span.swatch.is_some()) {
        return;
    }
    let mut offset = 0usize;
    for run in runs.iter_mut() {
        let range = offset..offset + run.len;
        offset = range.end;
        let literal = spans.iter().find_map(|span| {
            span.swatch
                .filter(|_| span.bytes.start < range.end && range.start < span.bytes.end)
        });
        if let Some(literal) = literal {
            run.color = rgba(crate::org_syntax::color::swatch_text_color(
                literal,
                theme.background,
            ))
            .into();
        }
    }
}

/// Returns theme-independent inline and code semantics for both the editor and
/// its minimap. Ranges always refer to UTF-8 byte offsets in `text`.
pub(in crate::editor) fn semantic_spans(
    path: &Path,
    text: &str,
    line_style: &EditorLineStyle,
) -> Vec<EditorSemanticSpan> {
    let mut spans = Vec::<(Range<usize>, SpanStyle)>::new();
    let verbatim = matches!(
        line_style.id,
        EditorStyleId::Code | EditorStyleId::CodeBoundary
    ) || line_style.code_language.is_some();
    let document_language = language(path);
    let mut opaque_inline_ranges = Vec::new();
    if !verbatim {
        let inline_start = line_style.inline_start(path, text);
        let table_cells = if line_style.id == EditorStyleId::Table {
            let format = match document_language {
                Language::Org => crate::document::DocumentFormat::Org,
                Language::Markdown => crate::document::DocumentFormat::Markdown,
            };
            Some(crate::document::table::parse_line(text, format).cells)
        } else {
            None
        };
        let inline_ranges = table_cells
            .iter()
            .flatten()
            .map(|cell| cell.text_range.clone())
            .chain(table_cells.is_none().then_some(inline_start..text.len()));
        for range in inline_ranges {
            let start = spans.len();
            let opaque =
                collect_inline_semantics(document_language, &text[range.clone()], &mut spans);
            for (span, _) in &mut spans[start..] {
                *span = span.start + range.start..span.end + range.start;
            }
            opaque_inline_ranges.extend(
                opaque
                    .into_iter()
                    .map(|span| span.start + range.start..span.end + range.start),
            );
        }
        if let Some(todo) = &line_style.todo {
            spans.push((
                todo.range.clone(),
                SpanStyle {
                    color: Some(todo.face.token()),
                    weight: Some(FontWeight::BOLD),
                    ..SpanStyle::default()
                },
            ));
        }
        match line_style.id {
            EditorStyleId::Property => {
                collect_org_property_key(text, EditorColorToken::Attribute, &mut spans)
            }
            EditorStyleId::Meta => collect_org_meta_key(text, EditorColorToken::Meta, &mut spans),
            EditorStyleId::List => {
                collect_list_marker(text, EditorColorToken::InlineCode, &mut spans)
            }
            _ => {}
        }
    }
    if let Some(language) = line_style.code_language.as_deref()
        && let Ok(code_spans) = crate::syntax_highlighting::highlight_code(language, text)
    {
        spans.extend(
            code_spans
                .into_iter()
                .filter(|span| {
                    span.start < span.end
                        && span.end <= text.len()
                        && text.is_char_boundary(span.start)
                        && text.is_char_boundary(span.end)
                })
                .map(|span| {
                    (
                        span.start..span.end,
                        code_span_style(span.kind, &text[span.start..span.end]),
                    )
                }),
        );
        if matches!(
            language.trim().to_ascii_lowercase().as_str(),
            "cpp" | "c++" | "cc" | "cxx" | "hpp"
        ) {
            collect_cpp_namespace_qualifiers(
                text,
                EditorColorToken::Variable,
                EditorColorToken::Foreground,
                &mut spans,
            );
        }
    }
    match document_language {
        Language::Org if !verbatim => {
            for keyword in ["SCHEDULED:", "DEADLINE:"] {
                collect_token_outside(
                    text,
                    keyword,
                    EditorColorToken::Meta,
                    &opaque_inline_ranges,
                    &mut spans,
                );
            }
            collect_token_outside(
                text,
                "CLOSED:",
                EditorColorToken::Done,
                &opaque_inline_ranges,
                &mut spans,
            );
            for priority in ["[#A]", "[#B]", "[#C]"] {
                collect_token_outside(
                    text,
                    priority,
                    EditorColorToken::Keyword,
                    &opaque_inline_ranges,
                    &mut spans,
                );
            }
            collect_org_tags_outside(
                text,
                EditorColorToken::Attribute,
                &opaque_inline_ranges,
                &mut spans,
            );
            // The cookie shares the meta face its progress bar uses; heading hue and
            // meta only agree in the light palette.
            if matches!(line_style.id, EditorStyleId::Heading(_))
                && let Some((range, _)) = crate::org_syntax::cookie::trailing_progress(text)
            {
                spans.push((
                    range,
                    SpanStyle {
                        color: Some(EditorColorToken::Meta),
                        ..SpanStyle::default()
                    },
                ));
            }
        }
        Language::Markdown | Language::Org => {}
    }

    // Bare links (GFM autolink extension): recognized schemes in running text.
    // Ranges covered by bracketed links or code/verbatim spans stay untouched.
    if !verbatim {
        let link_ranges: Vec<Range<usize>> = spans
            .iter()
            .filter(|(_, style)| style.link.is_some())
            .map(|(range, _)| range.clone())
            .collect();
        let plain_link_format = match document_language {
            Language::Org => crate::links::LinkFormat::Org,
            Language::Markdown => crate::links::LinkFormat::Markdown,
        };
        for (range, meta) in crate::links::scan_plain_links(text, plain_link_format) {
            if range_intersects_any(&range, &opaque_inline_ranges)
                || range_intersects_any(&range, &link_ranges)
            {
                continue;
            }
            spans.push((
                range,
                SpanStyle {
                    color: Some(link_color_token(&meta.kind)),
                    link: Some(meta),
                    ..SpanStyle::default()
                },
            ));
        }
    }

    // Document-authored hex colors get an opaque swatch behind the literal. The
    // scan is deliberately unfiltered by opaque ranges: colors written inside
    // inline code, verbatim text or a source block are exactly the ones worth
    // previewing.
    let literals = crate::org_syntax::color::scan_line(text);
    if !literals.is_empty() {
        // Org tags may legally contain `#`, so skip ranges already pilled.
        let pill_ranges: Vec<Range<usize>> = spans
            .iter()
            .filter(|(_, style)| style.pill)
            .map(|(range, _)| range.clone())
            .collect();
        for literal in literals {
            if range_intersects_any(&literal.range, &pill_ranges) {
                continue;
            }
            spans.push((
                literal.range,
                SpanStyle {
                    swatch: Some(literal.rgba),
                    ..SpanStyle::default()
                },
            ));
        }
    }

    spans
        .into_iter()
        .filter(|(range, _)| {
            range.start < range.end
                && range.end <= text.len()
                && text.is_char_boundary(range.start)
                && text.is_char_boundary(range.end)
        })
        .map(|(bytes, style)| EditorSemanticSpan {
            bytes,
            color: style.color,
            weight: match style.weight {
                Some(FontWeight::BOLD) => EditorSemanticWeight::Bold,
                Some(FontWeight::SEMIBOLD) => EditorSemanticWeight::Semibold,
                Some(FontWeight::NORMAL) => EditorSemanticWeight::Normal,
                _ => EditorSemanticWeight::Inherit,
            },
            italic: style.font_style == Some(FontStyle::Italic),
            underline: style.underline,
            strikethrough: style.strikethrough,
            pill: style.pill,
            swatch: style.swatch,
            link: style.link,
        })
        .collect()
}

fn code_span_style(kind: crate::syntax_highlighting::CodeHighlightKind, source: &str) -> SpanStyle {
    use crate::syntax_highlighting::CodeHighlightKind;

    let color = match kind {
        CodeHighlightKind::Keyword if source.starts_with('#') => EditorColorToken::Comment,
        CodeHighlightKind::Attribute => EditorColorToken::Attribute,
        CodeHighlightKind::Boolean | CodeHighlightKind::Constant => EditorColorToken::Constant,
        CodeHighlightKind::Comment => EditorColorToken::Comment,
        CodeHighlightKind::Function => EditorColorToken::Function,
        CodeHighlightKind::Keyword => EditorColorToken::Keyword,
        CodeHighlightKind::Number => EditorColorToken::Number,
        CodeHighlightKind::Operator | CodeHighlightKind::Punctuation => EditorColorToken::Operator,
        CodeHighlightKind::Property => EditorColorToken::Foreground,
        CodeHighlightKind::Variable => EditorColorToken::Variable,
        CodeHighlightKind::String => EditorColorToken::String,
        CodeHighlightKind::Type => EditorColorToken::Type,
    };
    SpanStyle {
        color: Some(color),
        font_style: matches!(kind, CodeHighlightKind::Comment).then_some(FontStyle::Italic),
        ..SpanStyle::default()
    }
}

fn collect_inline_semantics(
    language: Language,
    text: &str,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) -> Vec<Range<usize>> {
    let inline = match language {
        Language::Org => crate::org_syntax::inline::parse(text),
        Language::Markdown => crate::preview::markdown::parse_markdown_inline(text),
    };
    let opaque_ranges = collect_inline_spans(language, text, &inline, spans);

    collect_token_outside(text, "[ ]", EditorColorToken::Todo, &opaque_ranges, spans);
    collect_token_outside(
        text,
        "[-]",
        EditorColorToken::TodoActive,
        &opaque_ranges,
        spans,
    );
    collect_token_outside(
        text,
        "[?]",
        EditorColorToken::Waiting,
        &opaque_ranges,
        spans,
    );
    for checkbox in ["[X]", "[x]"] {
        collect_token_outside(
            text,
            checkbox,
            EditorColorToken::Done,
            &opaque_ranges,
            spans,
        );
    }
    opaque_ranges
}

fn link_color_token(kind: &crate::links::LinkKind) -> EditorColorToken {
    use crate::links::LinkKind;
    match kind {
        LinkKind::External => EditorColorToken::LinkExternal,
        LinkKind::File => EditorColorToken::LinkFile,
        LinkKind::Internal => EditorColorToken::LinkInternal,
        LinkKind::Mail => EditorColorToken::LinkMail,
        LinkKind::Dangerous => EditorColorToken::LinkWarn,
        LinkKind::Other => EditorColorToken::LinkOther,
    }
}

/// Resolved accent color for a link kind. Kept as the single source of truth
/// so hover highlights and tooltips always match the text color.
pub(in crate::editor) fn link_accent(kind: &crate::links::LinkKind, theme: &Theme) -> u32 {
    link_color_token(kind).resolve(theme)
}

fn collect_inline_spans(
    language: Language,
    text: &str,
    inline: &InlineText,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) -> Vec<Range<usize>> {
    let mut opaque_ranges = Vec::new();
    for inline_span in &inline.spans {
        let source = inline_span.source.clone();
        if source.start >= source.end
            || source.end > text.len()
            || !text.is_char_boundary(source.start)
            || !text.is_char_boundary(source.end)
        {
            continue;
        }

        let style = match inline_span.kind {
            InlineKind::Bold => SpanStyle {
                weight: Some(FontWeight::BOLD),
                ..SpanStyle::default()
            },
            InlineKind::Italic => SpanStyle {
                font_style: Some(FontStyle::Italic),
                ..SpanStyle::default()
            },
            InlineKind::Underline => SpanStyle {
                underline: true,
                ..SpanStyle::default()
            },
            InlineKind::Strike => SpanStyle {
                strikethrough: true,
                ..SpanStyle::default()
            },
            InlineKind::Code => SpanStyle {
                color: Some(EditorColorToken::InlineCode),
                ..SpanStyle::default()
            },
            InlineKind::Verbatim => SpanStyle {
                color: Some(EditorColorToken::Verbatim),
                ..SpanStyle::default()
            },
            InlineKind::Link => {
                let format = match language {
                    Language::Org => crate::links::LinkFormat::Org,
                    Language::Markdown => crate::links::LinkFormat::Markdown,
                };
                let meta = inline_span
                    .target
                    .as_deref()
                    .map(|target| crate::links::classify(target, format));
                SpanStyle {
                    color: Some(
                        meta.as_ref()
                            .map(|meta| link_color_token(&meta.kind))
                            .unwrap_or(EditorColorToken::Link),
                    ),
                    underline: language == Language::Markdown,
                    link: meta,
                    ..SpanStyle::default()
                }
            }
            InlineKind::Target | InlineKind::RadioTarget | InlineKind::FootnoteReference => {
                let meta = inline_span
                    .target
                    .as_deref()
                    .map(|target| crate::links::LinkInfo {
                        kind: crate::links::LinkKind::Internal,
                        raw: Arc::from(target),
                    });
                SpanStyle {
                    color: Some(EditorColorToken::LinkInternal),
                    link: meta,
                    ..SpanStyle::default()
                }
            }
            InlineKind::Timestamp => SpanStyle {
                color: Some(EditorColorToken::Date),
                ..SpanStyle::default()
            },
            InlineKind::Entity | InlineKind::Latex => SpanStyle {
                color: Some(EditorColorToken::Constant),
                ..SpanStyle::default()
            },
        };
        spans.push((source.clone(), style));

        if matches!(inline_span.kind, InlineKind::Code | InlineKind::Verbatim) {
            opaque_ranges.push(source.clone());
            let delimiter_len = match language {
                Language::Org => 1,
                Language::Markdown => text[source.clone()]
                    .bytes()
                    .take_while(|byte| *byte == b'`')
                    .count(),
            };
            for marker in [
                source.start..source.start + delimiter_len,
                source.end - delimiter_len..source.end,
            ] {
                spans.push((
                    marker,
                    SpanStyle {
                        color: Some(EditorColorToken::InlineDelimiter),
                        ..SpanStyle::default()
                    },
                ));
            }
        }
        if language == Language::Org && inline_span.kind == InlineKind::Link {
            collect_org_link_inner_weights(text, source, spans);
        }
    }
    opaque_ranges.sort_unstable_by_key(|range| range.start);
    opaque_ranges
}

fn collect_cpp_namespace_qualifiers(
    text: &str,
    qualifier_color: EditorColorToken,
    member_color: EditorColorToken,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
    let mut cursor = 0;
    while let Some(separator) = text[cursor..].find("::").map(|index| cursor + index) {
        let start = text[..separator]
            .rfind(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .map_or(0, |index| index + 1);
        if start < separator {
            spans.push((
                start..separator,
                SpanStyle {
                    color: Some(qualifier_color),
                    ..SpanStyle::default()
                },
            ));
        }
        let member_start = separator + 2;
        let member_end = text[member_start..]
            .find(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .map_or(text.len(), |index| member_start + index);
        if member_start < member_end {
            spans.push((
                member_start..member_end,
                SpanStyle {
                    color: Some(member_color),
                    ..SpanStyle::default()
                },
            ));
        }
        cursor = member_end.max(member_start);
    }
}

fn collect_org_link_inner_weights(
    text: &str,
    source: Range<usize>,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
    let Some(link) = BracketLink::at(text, source.start) else {
        return;
    };

    // Keep the brackets regular and emphasize the target/description source,
    // matching the existing editor treatment while the parser owns recognition.
    push_bold_span(link.target, spans);
    if let Some(description) = link.description {
        push_bold_span(description, spans);
    }
}

fn push_bold_span(range: Range<usize>, spans: &mut Vec<(Range<usize>, SpanStyle)>) {
    if range.start < range.end {
        spans.push((
            range,
            SpanStyle {
                weight: Some(FontWeight::BOLD),
                ..SpanStyle::default()
            },
        ));
    }
}

fn collect_org_property_key(
    text: &str,
    color: EditorColorToken,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
    let indent = text.len() - text.trim_start().len();
    let trimmed = &text[indent..];
    let Some(rest) = trimmed.strip_prefix(':') else {
        return;
    };
    let Some(end) = rest.find(':') else {
        return;
    };
    spans.push((
        indent..indent + end + 2,
        SpanStyle {
            color: Some(color),
            weight: Some(FontWeight::SEMIBOLD),
            ..SpanStyle::default()
        },
    ));
}

fn collect_org_meta_key(
    text: &str,
    color: EditorColorToken,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
    let indent = text.len() - text.trim_start().len();
    let trimmed = &text[indent..];
    let key_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    if trimmed.starts_with("#+") && key_end > 2 {
        spans.push((
            indent..indent + key_end,
            SpanStyle {
                color: Some(color),
                weight: Some(FontWeight::SEMIBOLD),
                ..SpanStyle::default()
            },
        ));
    }
}

fn collect_list_marker(
    text: &str,
    color: EditorColorToken,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
    let indent = text.len() - text.trim_start().len();
    let trimmed = &text[indent..];
    let marker_len = if trimmed.starts_with("- ") || trimmed.starts_with("+ ") {
        1
    } else {
        trimmed
            .find(['.', ')'])
            .filter(|end| trimmed[..*end].bytes().all(|byte| byte.is_ascii_digit()))
            .map_or(0, |end| end + 1)
    };
    if marker_len > 0 {
        spans.push((
            indent..indent + marker_len,
            SpanStyle {
                color: Some(color),
                weight: Some(FontWeight::BOLD),
                ..SpanStyle::default()
            },
        ));
    }
}

fn collect_token_outside(
    text: &str,
    token: &str,
    color: EditorColorToken,
    opaque_ranges: &[Range<usize>],
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
    let mut cursor = 0;
    while let Some(start) = text[cursor..].find(token).map(|index| cursor + index) {
        let end = start + token.len();
        if !range_intersects_any(&(start..end), opaque_ranges) {
            spans.push((
                start..end,
                SpanStyle {
                    color: Some(color),
                    weight: Some(FontWeight::SEMIBOLD),
                    ..SpanStyle::default()
                },
            ));
        }
        cursor = end;
    }
}

fn collect_org_tags_outside(
    text: &str,
    color: EditorColorToken,
    opaque_ranges: &[Range<usize>],
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
    for range in org_tag_ranges(text) {
        if !range_intersects_any(&range, opaque_ranges) {
            spans.push((
                range,
                SpanStyle {
                    color: Some(color),
                    pill: true,
                    ..SpanStyle::default()
                },
            ));
        }
    }
}

/// Byte ranges of each individual tag in a trailing Org tag cluster.
///
/// `* H :alpha:beta:` yields the ranges of `:alpha:` and `:beta:`. Adjacent
/// tags share the separator colon, so the two ranges overlap on that single
/// byte — exactly how Org writes the cluster. Callers paint each tag as a
/// separate pill while keeping every source byte visible. Returns an empty vec
/// when the trailing token is not a valid Org tag cluster.
pub(in crate::editor) fn org_tag_ranges(text: &str) -> Vec<Range<usize>> {
    let trimmed_end = text.trim_end();
    let Some(start) = trimmed_end.rfind(char::is_whitespace) else {
        return Vec::new();
    };
    let candidate = &trimmed_end[start + 1..];
    if candidate.len() < 3
        || !candidate.starts_with(':')
        || !candidate.ends_with(':')
        || !candidate[1..candidate.len() - 1].chars().all(|character| {
            character.is_alphanumeric() || matches!(character, ':' | '_' | '@' | '#' | '%')
        })
    {
        return Vec::new();
    }
    let base = start + 1;
    let inner = &candidate[1..candidate.len() - 1];
    let mut name_start = base + 1;
    let mut ranges = Vec::new();
    for name in inner.split(':') {
        // Leading colon (shared with the previous tag's trailing colon) + name
        // + trailing colon.
        let start = name_start - 1;
        let end = name_start + name.len() + 1;
        ranges.push(start..end);
        name_start += name.len() + 1;
    }
    ranges
}

fn range_intersects_any(range: &Range<usize>, others: &[Range<usize>]) -> bool {
    others
        .iter()
        .any(|other| range.start < other.end && other.start < range.end)
}
