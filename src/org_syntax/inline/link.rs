//! Complete Org bracket links, escaping and editable field values.

use std::ops::Range;

/// A complete bracket link, with all ranges expressed in the original source.
/// Shared by parsing, display projection and link editing; incomplete input stays literal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BracketLink {
    pub(crate) source: Range<usize>,
    pub(crate) target: Range<usize>,
    pub(crate) visible: Range<usize>,
    pub(crate) description: Option<Range<usize>>,
}

impl BracketLink {
    pub(crate) fn at(source: &str, start: usize) -> Option<Self> {
        let bytes = source.as_bytes();
        if !bytes.get(start..)?.starts_with(b"[[") {
            return None;
        }
        let target_start = start + 2;
        let mut cursor = target_start;
        while let Some(&byte) = bytes.get(cursor) {
            if matches!(byte, b'[' | b']') && !escaped_at(source, cursor) {
                if byte == b'[' || cursor == target_start {
                    return None;
                }
                let target = target_start..cursor;
                return match bytes.get(cursor + 1) {
                    Some(b']') => {
                        let link = Self {
                            source: start..cursor + 2,
                            visible: target.clone(),
                            target,
                            description: None,
                        };
                        (!has_paragraph_break(&source[link.source.clone()])).then_some(link)
                    }
                    Some(b'[') => {
                        let description_start = cursor + 2;
                        let end = source[description_start..].find("]]")? + description_start;
                        if end == description_start {
                            return None;
                        }
                        let description = description_start..end;
                        let link = Self {
                            source: start..end + 2,
                            target,
                            visible: description.clone(),
                            description: Some(description),
                        };
                        (!has_paragraph_break(&source[link.source.clone()])).then_some(link)
                    }
                    _ => None,
                };
            }
            cursor += 1;
        }
        None
    }
}

fn escaped_at(source: &str, offset: usize) -> bool {
    source.as_bytes()[..offset]
        .iter()
        .rev()
        .take_while(|&&byte| byte == b'\\')
        .count()
        % 2
        == 1
}

fn has_paragraph_break(source: &str) -> bool {
    source.lines().skip(1).any(|line| line.trim().is_empty())
}

pub(crate) fn escape_link_target(target: &str) -> String {
    link_target_backslashes(target, true)
}

pub(crate) fn unescape_link_target(target: &str) -> String {
    link_target_backslashes(target, false)
}

fn link_target_backslashes(target: &str, escape: bool) -> String {
    let mut output = String::with_capacity(target.len());
    let mut slashes = 0;
    for ch in target.chars() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }
        let special = matches!(ch, '[' | ']');
        let count = if special && escape {
            slashes * 2 + 1
        } else if special {
            slashes / 2
        } else {
            slashes
        };
        output.extend(std::iter::repeat_n('\\', count));
        slashes = 0;
        output.push(ch);
    }
    output.extend(std::iter::repeat_n(
        '\\',
        if escape { slashes * 2 } else { slashes / 2 },
    ));
    output
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LinkValue {
    pub(crate) target: String,
    pub(crate) description: String,
}

impl LinkValue {
    pub(crate) fn source(&self) -> Option<String> {
        let target = self.target.trim();
        if target.is_empty()
            || target.contains(['\n', '\r', '\0'])
            || self.description.contains(['\n', '\r', '\0'])
        {
            return None;
        }
        let target = escape_link_target(target);
        let source = if self.description.is_empty() {
            format!("[[{target}]]")
        } else {
            format!("[[{target}][{}]]", self.description)
        };
        let link = BracketLink::at(&source, 0)?;
        (link.source.end == source.len()).then_some(source)
    }
}

#[cfg(test)]
mod tests {
    use super::LinkValue;
    use crate::org_syntax::inline::{BracketLink, unescape_link_target};

    #[test]
    fn link_fields_round_trip_target_escapes_and_validate_complete_source() {
        for target in [
            "https://example.org/a[b]",
            r"file:C:\notes\",
            r"file:a\[b]",
            "mailto:person@example.org",
        ] {
            for description in ["", "指南 😀", "A [label] B"] {
                let source = LinkValue {
                    target: target.into(),
                    description: description.into(),
                }
                .source()
                .unwrap();
                let link = BracketLink::at(&source, 0).unwrap();
                assert_eq!(unescape_link_target(&source[link.target]), target);
                assert_eq!(link.source.end, source.len());
                assert_eq!(
                    link.description
                        .map(|range| &source[range])
                        .unwrap_or_default(),
                    description
                );
            }
        }
        for (target, description) in [
            ("", "Label"),
            ("a\nb", "Label"),
            ("a", "broken]]tail"),
            ("a", "two\nlines"),
        ] {
            assert!(
                LinkValue {
                    target: target.into(),
                    description: description.into()
                }
                .source()
                .is_none()
            );
        }
    }
}
