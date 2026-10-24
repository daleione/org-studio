use std::{ops::Range, sync::Arc};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InlineKind {
    Bold,
    Italic,
    Underline,
    Strike,
    Code,
    Verbatim,
    Link,
    Target,
    RadioTarget,
    FootnoteReference,
    Timestamp,
    Entity,
    Latex,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InlineSpan {
    pub kind: InlineKind,
    pub source: Range<usize>,
    pub range: Range<usize>,
    /// Raw link target: the `[[…]]` inner for Org links, the `dest_url` for
    /// Markdown links, the footnote name for footnote references, and the
    /// target text for dedicated/radio targets. `None` for plain emphasis.
    pub target: Option<Arc<str>>,
}

impl InlineSpan {
    pub fn is_code(&self) -> bool {
        matches!(self.kind, InlineKind::Code | InlineKind::Verbatim)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InlineText {
    pub text: String,
    pub spans: Vec<InlineSpan>,
}

/// Entity presentation keeps the source table structure and markup intact.
/// Only entities recognized by the inline parser are substituted; rendered
/// characters are never parsed again as links, emphasis, or table delimiters.
#[derive(Clone, Debug)]
pub(crate) struct EntityText {
    pub(crate) text: String,
    replacements: Vec<(Range<usize>, Range<usize>)>,
}

impl EntityText {
    pub(crate) fn new(source: &str, caret: Option<usize>) -> Self {
        let parsed = parse(source);
        let mut text = String::with_capacity(source.len());
        let mut replacements = Vec::new();
        let mut cursor = 0;
        for span in parsed
            .spans
            .iter()
            .filter(|span| span.kind == InlineKind::Entity)
        {
            let value = &parsed.text[span.range.clone()];
            if value == &source[span.source.clone()]
                || caret.is_some_and(|caret| span.source.start < caret && caret < span.source.end)
            {
                continue;
            }
            text.push_str(&source[cursor..span.source.start]);
            let start = text.len();
            text.push_str(value);
            replacements.push((span.source.clone(), start..text.len()));
            cursor = span.source.end;
        }
        text.push_str(&source[cursor..]);
        Self { text, replacements }
    }

    pub(crate) fn has_replacements(&self) -> bool {
        !self.replacements.is_empty()
    }

    pub(crate) fn source_to_display(&self, source: usize) -> usize {
        let (mut source_end, mut display_end) = (0, 0);
        for (raw, display) in &self.replacements {
            if source < raw.start {
                break;
            }
            if source < raw.end {
                return if source - raw.start <= raw.len() / 2 {
                    display.start
                } else {
                    display.end
                };
            }
            source_end = raw.end;
            display_end = display.end;
        }
        display_end + source.saturating_sub(source_end)
    }

    pub(crate) fn display_to_source(&self, display: usize) -> usize {
        let (mut source_end, mut display_end) = (0, 0);
        for (raw, visible) in &self.replacements {
            if display < visible.start {
                break;
            }
            if display < visible.end {
                return if display - visible.start <= visible.len() / 2 {
                    raw.start
                } else {
                    raw.end
                };
            }
            source_end = raw.end;
            display_end = visible.end;
        }
        source_end + display.saturating_sub(display_end)
    }
}

pub fn parse(source: &str) -> InlineText {
    let mut output = InlineText::default();
    let mut cursor = 0;
    while cursor < source.len() {
        if let Some(end) = footnote_end(source, cursor) {
            push_span(
                &mut output,
                InlineKind::FootnoteReference,
                cursor..end,
                &source[cursor..end],
                Some(Arc::from(&source[cursor..end])),
            );
            cursor = end;
            continue;
        }
        if let Some((end, display, kind)) = target_at(source, cursor) {
            push_span(
                &mut output,
                kind,
                cursor..end,
                display,
                Some(Arc::from(display)),
            );
            cursor = end;
            continue;
        }
        if source[cursor..].starts_with("[[")
            && let Some(close) = source[cursor + 2..].find("]]")
        {
            let inside_end = cursor + 2 + close;
            let inside = &source[cursor + 2..inside_end];
            let display = inside
                .split_once("][")
                .map(|(_, text)| text)
                .unwrap_or(inside);
            let target = inside
                .split_once("][")
                .map(|(target, _)| target)
                .unwrap_or(inside);
            push_span(
                &mut output,
                InlineKind::Link,
                cursor..inside_end + 2,
                display,
                Some(Arc::from(target)),
            );
            cursor = inside_end + 2;
            continue;
        }
        if matches!(source.as_bytes()[cursor], b'<' | b'[')
            && let Some((end, display)) = timestamp_at(source, cursor)
        {
            push_span(
                &mut output,
                InlineKind::Timestamp,
                cursor..end,
                display,
                None,
            );
            cursor = end;
            continue;
        }
        if let Some(range) = super::math::fragment_at(source, cursor) {
            let end = range.end;
            push_span(
                &mut output,
                InlineKind::Latex,
                range.clone(),
                &source[range],
                None,
            );
            cursor = end;
            continue;
        }
        if let Some(end) = entity_end(source, cursor) {
            let raw = &source[cursor..end];
            let display = entity_display(raw).unwrap_or(raw);
            push_span(&mut output, InlineKind::Entity, cursor..end, display, None);
            cursor = end;
            continue;
        }
        let marker = source.as_bytes()[cursor];
        if let Some(kind) = marker_kind(marker)
            && delimiter_can_open(source, cursor)
            && let Some(close) = find_closing_marker(source, cursor + 1, marker)
        {
            let content = &source[cursor + 1..close];
            if !content.is_empty() {
                if matches!(kind, InlineKind::Code | InlineKind::Verbatim) {
                    push_span(&mut output, kind, cursor..close + 1, content, None);
                } else {
                    push_nested_span(&mut output, kind, cursor..close + 1, content, cursor + 1);
                }
                cursor = close + 1;
                continue;
            }
        }
        let character = source[cursor..]
            .chars()
            .next()
            .expect("valid UTF-8 boundary");
        output.text.push(character);
        cursor += character.len_utf8();
    }
    output
}

fn push_nested_span(
    output: &mut InlineText,
    kind: InlineKind,
    source: Range<usize>,
    content: &str,
    source_offset: usize,
) {
    let parsed = parse(content);
    let start = output.text.len();
    output.text.push_str(&parsed.text);
    let end = output.text.len();
    output.spans.push(InlineSpan {
        kind,
        source,
        range: start..end,
        target: None,
    });
    output
        .spans
        .extend(parsed.spans.into_iter().map(|span| InlineSpan {
            kind: span.kind,
            source: span.source.start + source_offset..span.source.end + source_offset,
            range: span.range.start + start..span.range.end + start,
            target: span.target,
        }));
}

fn footnote_end(source: &str, start: usize) -> Option<usize> {
    if !source[start..].starts_with("[fn:") {
        return None;
    }
    let bytes = source.as_bytes();
    let mut index = start + 4;
    let mut link_depth = 0_u32;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"[[") {
            link_depth += 1;
            index += 2;
        } else if bytes[index..].starts_with(b"]]") && link_depth > 0 {
            link_depth -= 1;
            index += 2;
        } else if bytes[index] == b']' && link_depth == 0 {
            return Some(index + 1);
        } else {
            index += 1;
        }
    }
    None
}

fn target_at(source: &str, start: usize) -> Option<(usize, &str, InlineKind)> {
    let (open, close, kind) = if source[start..].starts_with("<<<") {
        (3, ">>>", InlineKind::RadioTarget)
    } else if source[start..].starts_with("<<") {
        (2, ">>", InlineKind::Target)
    } else {
        return None;
    };
    let relative = source[start + open..].find(close)?;
    let content_end = start + open + relative;
    Some((
        content_end + close.len(),
        &source[start + open..content_end],
        kind,
    ))
}

fn push_span(
    output: &mut InlineText,
    kind: InlineKind,
    source: Range<usize>,
    display: &str,
    target: Option<Arc<str>>,
) {
    let start = output.text.len();
    output.text.push_str(display);
    output.spans.push(InlineSpan {
        kind,
        source,
        range: start..output.text.len(),
        target,
    });
}

fn entity_end(source: &str, start: usize) -> Option<usize> {
    if source.as_bytes().get(start) != Some(&b'\\')
        || !source
            .as_bytes()
            .get(start + 1)
            .is_some_and(u8::is_ascii_alphabetic)
    {
        return None;
    }
    let mut end = start + 2;
    while source
        .as_bytes()
        .get(end)
        .is_some_and(u8::is_ascii_alphanumeric)
    {
        end += 1;
    }
    if source.as_bytes().get(end) == Some(&b'{') && source.as_bytes().get(end + 1) == Some(&b'}') {
        end += 2;
    }
    Some(end)
}

fn entity_display(entity: &str) -> Option<&'static str> {
    match entity.trim_end_matches("{}") {
        "\\alpha" => Some("α"),
        "\\beta" => Some("β"),
        "\\gamma" => Some("γ"),
        "\\delta" => Some("δ"),
        "\\lambda" => Some("λ"),
        "\\pi" => Some("π"),
        "\\sigma" => Some("σ"),
        "\\rightarrow" | "\\to" => Some("→"),
        "\\leftarrow" => Some("←"),
        "\\le" => Some("≤"),
        "\\ge" => Some("≥"),
        "\\neq" => Some("≠"),
        "\\nbsp" => Some(" "),
        "\\zwnj" => Some("\u{200c}"),
        "\\vert" => Some("|"),
        "\\ast" => Some("*"),
        "\\under" => Some("_"),
        "\\slash" => Some("/"),
        "\\equal" => Some("="),
        "\\tilde" => Some("~"),
        "\\plus" => Some("+"),
        "\\lt" => Some("<"),
        "\\gt" => Some(">"),
        "\\asciicirc" => Some("^"),
        "\\dollar" => Some("$"),
        _ => None,
    }
}

fn marker_kind(marker: u8) -> Option<InlineKind> {
    match marker {
        b'*' => Some(InlineKind::Bold),
        b'/' => Some(InlineKind::Italic),
        b'_' => Some(InlineKind::Underline),
        b'+' => Some(InlineKind::Strike),
        b'~' => Some(InlineKind::Code),
        b'=' => Some(InlineKind::Verbatim),
        _ => None,
    }
}

fn delimiter_can_open(source: &str, index: usize) -> bool {
    let previous = source[..index].chars().next_back();
    let next = source[index + 1..].chars().next();
    previous.is_none_or(|character| {
        character != '\\' && (character.is_whitespace() || !is_word_character(character))
    }) && next.is_some_and(|character| !character.is_whitespace())
}

fn find_closing_marker(source: &str, start: usize, marker: u8) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = start;
    while index < bytes.len() {
        if bytes[index] == marker
            && bytes.get(index.wrapping_sub(1)) != Some(&b'\\')
            && source[..index]
                .chars()
                .next_back()
                .is_some_and(|character| !character.is_whitespace())
            && (index + 1 == bytes.len()
                || bytes[index + 1].is_ascii_whitespace()
                || matches!(
                    bytes[index + 1],
                    b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b']' | b'}'
                )
                || source[index + 1..].chars().next().is_some_and(|character| {
                    matches!(
                        character,
                        '、' | '，' | '。' | '；' | '：' | '！' | '？' | '）' | '】' | '》'
                    )
                }))
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn timestamp_at(source: &str, start: usize) -> Option<(usize, &str)> {
    let closer = if source.as_bytes()[start] == b'<' {
        '>'
    } else {
        ']'
    };
    let relative_end = source[start + 1..].find(closer)?;
    let end = start + 1 + relative_end;
    let inside = &source[start + 1..end];
    let bytes = inside.as_bytes();
    let date = bytes.len() >= 10
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit);
    date.then_some((end + 1, &source[start..=end]))
}

#[cfg(test)]
mod tests {
    use super::{EntityText, InlineKind, parse};

    #[test]
    fn table_entities_preserve_literal_markup_and_source_boundaries() {
        let source = r"中文 a\vert{}b \nbsp{}padded\nbsp{} \ast{}粗体\ast{} \under{}下划线\under{} \equal{}原样\equal{} \tilde{}代码\tilde{} C:\\zwnj{}Users\\zwnj{}demo [\zwnj{}[file:demo][链接形态]] {\zwnj{}{{macro}}}";
        let visible = EntityText::new(source, None);
        assert_eq!(
            visible.text.replace('\u{200c}', "").replace('\u{a0}', " "),
            r"中文 a|b  padded  *粗体* _下划线_ =原样= ~代码~ C:\Users\demo [[file:demo][链接形态]] {{{macro}}}"
        );
        for offset in source
            .char_indices()
            .map(|(offset, _)| offset)
            .chain([source.len()])
        {
            if visible
                .replacements
                .iter()
                .any(|(raw, _)| raw.start < offset && offset < raw.end)
            {
                continue;
            }
            let display = visible.source_to_display(offset);
            assert!(visible.text.is_char_boundary(display));
            assert_eq!(visible.display_to_source(display), offset);
        }
    }

    #[test]
    fn table_entities_respect_verbatim_unknown_names_and_editing_at_point() {
        let source = r"=\alpha{}= \unknown{} a\vert{}b \nbsp{}end";
        let caret = source.find("vert").unwrap() + 2;
        let visible = EntityText::new(source, Some(caret));
        assert_eq!(visible.text, "=\\alpha{}= \\unknown{} a\\vert{}b \u{a0}end");
        assert_eq!(
            visible.display_to_source(visible.source_to_display(caret)),
            caret
        );
        let visible = EntityText::new(source, None);
        assert_eq!(visible.text, "=\\alpha{}= \\unknown{} a|b \u{a0}end");
    }

    #[test]
    fn removes_markup_and_preserves_unicode() {
        let parsed = parse("Hello *粗体* and /italic/ 😀.");
        assert_eq!(parsed.text, "Hello 粗体 and italic 😀.");
        assert_eq!(parsed.spans[0].source, 6..14);
        assert_eq!(&parsed.text[parsed.spans[0].range.clone()], "粗体");
    }

    #[test]
    fn renders_link_description_and_timestamp() {
        let parsed = parse("See [[file:readme.org][the notes]] <2026-08-24 Mon>.");
        assert_eq!(parsed.text, "See the notes <2026-08-24 Mon>.");
        assert_eq!(parsed.spans[0].kind, InlineKind::Link);
        assert_eq!(parsed.spans[1].kind, InlineKind::Timestamp);
    }

    #[test]
    fn recognizes_entities_and_latex_without_evaluating_them() {
        let parsed = parse(r"Greek \alpha and $x^2$ or \(y + 1\).");
        assert_eq!(
            parsed
                .spans
                .iter()
                .filter(|s| s.kind == InlineKind::Entity)
                .count(),
            1
        );
        assert_eq!(
            parsed
                .spans
                .iter()
                .filter(|s| s.kind == InlineKind::Latex)
                .count(),
            2
        );
    }

    #[test]
    fn leaves_unclosed_markup_unchanged() {
        let parsed = parse("This is *unfinished [[link");
        assert_eq!(parsed.text, "This is *unfinished [[link");
    }

    #[test]
    fn closes_markup_before_cjk_punctuation() {
        let parsed = parse("使用 =code=、*粗体*，继续。");
        assert_eq!(parsed.text, "使用 code、粗体，继续。");
        assert!(
            parsed
                .spans
                .iter()
                .any(|span| span.kind == InlineKind::Verbatim)
        );
        assert!(
            parsed
                .spans
                .iter()
                .any(|span| span.kind == InlineKind::Bold)
        );
    }

    #[test]
    fn parses_nested_emphasis_and_preserves_source_ranges() {
        let parsed = parse("*bold /and italic/*");
        assert_eq!(parsed.text, "bold and italic");
        assert_eq!(parsed.spans.len(), 2);
        assert_eq!(parsed.spans[0].kind, InlineKind::Bold);
        assert_eq!(parsed.spans[1].kind, InlineKind::Italic);
        assert_eq!(&parsed.text[parsed.spans[1].range.clone()], "and italic");
    }

    #[test]
    fn verbatim_identifiers_do_not_leak_underline_markup() {
        let parsed = parse(
            "*=render_document= returns formatted output.* \
             Then call =load_config= to read settings.",
        );
        assert_eq!(
            parsed
                .spans
                .iter()
                .filter(|span| span.kind == InlineKind::Bold)
                .count(),
            1
        );
        assert_eq!(
            parsed
                .spans
                .iter()
                .filter(|span| span.kind == InlineKind::Verbatim)
                .count(),
            2
        );
        assert!(
            parsed
                .spans
                .iter()
                .all(|span| span.kind != InlineKind::Underline)
        );

        let identifiers = parse("render_document and load_config");
        assert!(identifiers.spans.is_empty());
        let underline = parse("_real underline_");
        assert_eq!(underline.spans.len(), 1);
        assert_eq!(underline.spans[0].kind, InlineKind::Underline);
    }

    #[test]
    fn converts_safe_entities_and_recognizes_native_targets() {
        let parsed = parse(r"\alpha \rightarrow <<chapter>> [fn:note]");
        assert_eq!(parsed.text, "α → chapter [fn:note]");
        assert_eq!(
            parsed
                .spans
                .iter()
                .filter(|span| span.kind == InlineKind::Target)
                .count(),
            1
        );
        assert_eq!(
            parsed
                .spans
                .iter()
                .filter(|span| span.kind == InlineKind::FootnoteReference)
                .count(),
            1
        );
    }

    #[test]
    fn footnote_scanner_does_not_stop_inside_org_link() {
        let parsed = parse("[fn::See [[file:guide.org][guide]] for details]");
        assert_eq!(
            parsed.text,
            "[fn::See [[file:guide.org][guide]] for details]"
        );
        assert_eq!(parsed.spans.len(), 1);
        assert_eq!(parsed.spans[0].kind, InlineKind::FootnoteReference);
    }
}
