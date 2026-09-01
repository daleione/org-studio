use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CheckboxState {
    Empty,
    Partial,
    Checked,
}

impl CheckboxState {
    pub(crate) fn from_token(token: &str) -> Option<Self> {
        match token {
            "[ ]" => Some(Self::Empty),
            "[-]" => Some(Self::Partial),
            "[X]" | "[x]" => Some(Self::Checked),
            _ => None,
        }
    }

    pub(crate) const fn toggled(self) -> Self {
        match self {
            Self::Empty => Self::Checked,
            Self::Partial | Self::Checked => Self::Empty,
        }
    }

    pub(crate) const fn token(self) -> &'static str {
        match self {
            Self::Empty => "[ ]",
            Self::Partial => "[-]",
            Self::Checked => "[X]",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CheckboxToken {
    pub(crate) range: Range<usize>,
    pub(crate) state: CheckboxState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ListLine<'a> {
    pub(crate) indent: &'a str,
    pub(crate) indent_columns: usize,
    pub(crate) marker: &'a str,
    pub(crate) counter: Option<&'a str>,
    pub(crate) checkbox: Option<CheckboxToken>,
    pub(crate) term: Option<&'a str>,
    pub(crate) body: &'a str,
}

/// Parses the structural prefix of one Org plain-list item.
///
/// A checkbox is recognized only directly after the list marker and optional counter. Literal
/// bracket text later in the item body is never editable checkbox syntax.
pub(crate) fn parse_list_line(text: &str) -> Option<ListLine<'_>> {
    let indent_bytes = text.len() - text.trim_start_matches([' ', '\t']).len();
    let indent = &text[..indent_bytes];
    let indent_columns = indent.bytes().fold(0, |column, byte| {
        if byte == b'\t' {
            (column / 4 + 1) * 4
        } else {
            column + 1
        }
    });
    let rest = &text[indent_bytes..];
    let marker_end = marker_end(rest, indent_columns)?;
    let marker = rest[..marker_end].trim_end();
    let mut cursor = indent_bytes + marker_end;
    cursor += text[cursor..].len() - text[cursor..].trim_start().len();

    let counter = bracket_token(&text[cursor..], "[@");
    if let Some(counter) = counter {
        cursor += counter.len();
        cursor += text[cursor..].len() - text[cursor..].trim_start().len();
    }

    let checkbox = text[cursor..].get(..3).and_then(|token| {
        CheckboxState::from_token(token).map(|state| CheckboxToken {
            range: cursor..cursor + token.len(),
            state,
        })
    });
    if checkbox.is_some() {
        cursor += 3;
        cursor += text[cursor..].len() - text[cursor..].trim_start().len();
    }

    let body = &text[cursor..];
    let (term, body) = body
        .split_once(" :: ")
        .map_or((None, body), |(term, body)| (Some(term), body));
    Some(ListLine {
        indent,
        indent_columns,
        marker,
        counter,
        checkbox,
        term,
        body,
    })
}

pub(crate) fn checkbox_token(text: &str) -> Option<(Range<usize>, &str)> {
    let checkbox = parse_list_line(text)?.checkbox?;
    Some((checkbox.range.clone(), &text[checkbox.range]))
}

fn marker_end(text: &str, indent: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let first = *bytes.first()?;
    if matches!(first, b'-' | b'+') || (first == b'*' && indent > 0) {
        return bytes
            .get(1)
            .is_some_and(u8::is_ascii_whitespace)
            .then_some(2);
    }
    let marker = text
        .char_indices()
        .take_while(|(_, character)| character.is_ascii_alphanumeric())
        .last()
        .map(|(index, character)| index + character.len_utf8())?;
    matches!(bytes.get(marker), Some(b'.' | b')'))
        .then(|| marker + 1)
        .filter(|end| bytes.get(*end).is_some_and(u8::is_ascii_whitespace))
        .map(|end| end + 1)
}

fn bracket_token<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.strip_prefix(prefix)?;
    let end = text.find(']')?;
    Some(&text[..=end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkbox_must_follow_the_marker_and_optional_counter() {
        let parsed = parse_list_line("  3. [@8] [-] parser :: body").unwrap();
        assert_eq!(parsed.marker, "3.");
        assert_eq!(parsed.counter, Some("[@8]"));
        assert_eq!(parsed.checkbox.unwrap().range, 10..13);
        assert_eq!(parsed.term, Some("parser"));
        assert_eq!(parsed.body, "body");
        assert!(checkbox_token("- literal [ ] text").is_none());
        assert!(checkbox_token("- term :: [ ] description").is_none());
    }
}
