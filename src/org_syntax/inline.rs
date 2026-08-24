use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InlineKind {
    Bold,
    Italic,
    Underline,
    Strike,
    Code,
    Verbatim,
    Link,
    Timestamp,
    Entity,
    Latex,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InlineSpan {
    pub kind: InlineKind,
    pub source: Range<usize>,
    pub range: Range<usize>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InlineText {
    pub text: String,
    pub spans: Vec<InlineSpan>,
}

pub fn parse(source: &str) -> InlineText {
    let mut output = InlineText::default();
    let mut cursor = 0;
    while cursor < source.len() {
        if source[cursor..].starts_with("[[")
            && let Some(close) = source[cursor + 2..].find("]]")
        {
            let inside_end = cursor + 2 + close;
            let inside = &source[cursor + 2..inside_end];
            let display = inside
                .split_once("][")
                .map(|(_, text)| text)
                .unwrap_or(inside);
            push_span(
                &mut output,
                InlineKind::Link,
                cursor..inside_end + 2,
                display,
            );
            cursor = inside_end + 2;
            continue;
        }
        if matches!(source.as_bytes()[cursor], b'<' | b'[')
            && let Some((end, display)) = timestamp_at(source, cursor)
        {
            push_span(&mut output, InlineKind::Timestamp, cursor..end, display);
            cursor = end;
            continue;
        }
        if let Some((end, display)) = latex_at(source, cursor) {
            push_span(&mut output, InlineKind::Latex, cursor..end, display);
            cursor = end;
            continue;
        }
        if let Some(end) = entity_end(source, cursor) {
            let display = &source[cursor..end];
            push_span(&mut output, InlineKind::Entity, cursor..end, display);
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
                push_span(&mut output, kind, cursor..close + 1, content);
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

fn push_span(output: &mut InlineText, kind: InlineKind, source: Range<usize>, display: &str) {
    let start = output.text.len();
    output.text.push_str(display);
    output.spans.push(InlineSpan {
        kind,
        source,
        range: start..output.text.len(),
    });
}

fn latex_at(source: &str, start: usize) -> Option<(usize, &str)> {
    if source[start..].starts_with("\\(") {
        let relative = source[start + 2..].find("\\)")?;
        let end = start + 2 + relative + 2;
        return Some((end, &source[start..end]));
    }
    if source.as_bytes()[start] == b'$'
        && source.as_bytes().get(start + 1) != Some(&b'$')
        && let Some(relative) = source[start + 1..].find('$')
    {
        let end = start + 1 + relative + 1;
        return Some((end, &source[start..end]));
    }
    None
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
    (index == 0 || source.as_bytes()[index - 1] != b'\\')
        && source
            .as_bytes()
            .get(index + 1)
            .is_some_and(|next| !next.is_ascii_whitespace())
}

fn find_closing_marker(source: &str, start: usize, marker: u8) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = start;
    while index < bytes.len() {
        if bytes[index] == marker
            && bytes.get(index.wrapping_sub(1)) != Some(&b'\\')
            && (index + 1 == bytes.len()
                || bytes[index + 1].is_ascii_whitespace()
                || matches!(
                    bytes[index + 1],
                    b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b']' | b'}'
                ))
        {
            return Some(index);
        }
        index += 1;
    }
    None
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
    use super::{InlineKind, parse};

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
}
