//! LaTeX fragments shared by syntax, presentation and export. No rendering dependencies.
use std::{borrow::Cow, ops::Range, sync::Arc};
use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MathMode {
    Inline,
    Display,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Formula {
    pub body: Arc<str>,
    pub mode: MathMode,
}

impl Formula {
    pub fn new(body: impl Into<Arc<str>>, mode: MathMode) -> Self {
        Self {
            body: body.into(),
            mode,
        }
    }

    /// Decode a fragment that has already been recognized by a document parser.
    pub fn from_delimited(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        for (open, close, mode) in [
            ("\\(", "\\)", MathMode::Inline),
            ("\\[", "\\]", MathMode::Display),
            ("$$", "$$", MathMode::Display),
            ("$", "$", MathMode::Inline),
        ] {
            if let Some(body) = raw.strip_prefix(open).and_then(|s| s.strip_suffix(close)) {
                return Some(Self::new(body, mode));
            }
        }
        let (_, close) = environment_start(raw)?;
        raw.ends_with(&close)
            .then(|| Self::new(raw, MathMode::Display))
    }
}

fn escaped(source: &str, index: usize) -> bool {
    source.as_bytes()[..index]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
        % 2
        == 1
}

/// Shared by inline recognition and the block parser, including escaped delimiters.
pub(crate) fn closing_end(source: &str, from: usize, close: &str) -> Option<usize> {
    let mut cursor = from;
    while let Some(offset) = source[cursor..].find(close) {
        let at = cursor + offset;
        if !escaped(source, at) {
            return Some(at + close.len());
        }
        // End markers are ASCII. Advance one byte so an escaped dollar followed
        // immediately by a double-dollar ending is not skipped as an overlap.
        cursor = at + 1;
    }
    None
}

/// A standalone display opening also gives the block parser its closing marker.
pub(crate) fn display_start(line: &str) -> Option<String> {
    let line = line.trim_start();
    if line.starts_with("\\[") {
        Some("\\]".into())
    } else if line.starts_with("$$") {
        Some("$$".into())
    } else {
        environment_start(line).map(|(_, end)| end)
    }
}

fn environment_start(source: &str) -> Option<(&str, String)> {
    let rest = source.strip_prefix("\\begin{")?;
    let name = rest.split_once('}')?.0;
    let base = name.trim_end_matches('*');
    matches!(
        base,
        "equation" | "align" | "alignat" | "gather" | "multline" | "displaymath" | "eqnarray"
    )
    .then(|| (name, format!("\\end{{{name}}}")))
}

/// Org's single-dollar rules deliberately exclude currency and surrounding whitespace.
/// Code and verbatim precedence belongs to the caller's inline parser.
pub(crate) fn fragment_at(source: &str, start: usize) -> Option<Range<usize>> {
    let rest = &source[start..];
    let (open, close): (_, Cow<'_, str>) = if rest.starts_with("\\(") {
        (2, "\\)".into())
    } else if rest.starts_with("\\[") {
        (2, "\\]".into())
    } else if rest.starts_with("$$") {
        (2, "$$".into())
    } else if let Some(body) = rest.strip_prefix('$') {
        let next = body.chars().next()?;
        if source.as_bytes().get(start.wrapping_sub(1)) == Some(&b'$')
            || next.is_whitespace()
            || matches!(next, '$' | '.' | ',' | ';')
        {
            return None;
        }
        (1, "$".into())
    } else if let Some((name, end)) = environment_start(rest) {
        // Org environments must begin on a line containing only whitespace before them.
        let line = source[..start].rsplit('\n').next().unwrap_or_default();
        if !line.trim().is_empty() {
            return None;
        }
        ("\\begin{}".len() + name.len(), end.into())
    } else {
        return None;
    };
    if escaped(source, start) {
        return None;
    }
    let end = closing_end(source, start + open, &close)?;
    if open == 1 {
        let body = &source[start + 1..end - 1];
        let last = body.chars().next_back()?;
        // https://orgmode.org/worg/org-syntax.html#LaTeX_Fragments
        if last.is_whitespace()
            || matches!(last, '.' | ',')
            || (body.chars().count() == 1 && matches!(last, '?' | '"' | ';'))
            || body.matches('\n').count() > 2
            || source[end..].chars().next().is_some_and(|ch| {
                !ch.is_whitespace()
                    && ch.general_category_group() != GeneralCategoryGroup::Punctuation
            })
        {
            return None;
        }
    }
    Some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_math_dollar_boundaries_include_unicode_punctuation() {
        for suffix in ["。", "，", "、", "—", "-tail", "\"", " "] {
            let source = format!("$x^2${suffix}");
            assert_eq!(fragment_at(&source, 0), Some(0..5), "{source}");
        }
        for source in ["$?$", "$\"$", "$x,$", "$x.$", "$x$tail and $y$", "$x$$"] {
            assert_eq!(fragment_at(source, 0), None, "{source}");
        }
    }

    #[test]
    fn recognizes_org_fragments_without_currency_or_escaped_delimiters() {
        for source in [
            r"$x^2$",
            r"\(x+1\)",
            "\\[\n\\frac{a}{b}\n\\]",
            r"$$x+y$$",
            r"$$a\$$$",
            "\\begin{align*}\nx &= y \\\\\n z &= 1\n\\end{align*}",
        ] {
            assert_eq!(fragment_at(source, 0), Some(0..source.len()), "{source}");
            assert!(Formula::from_delimited(source).is_some());
        }
        for source in ["$ 5$", "$5 $", "$5 and $10", r"\$x$", "$a\nb\nc\nd$"] {
            assert_eq!(fragment_at(source, 0), None, "{source}");
        }
        assert_eq!(fragment_at(r"$a\$b$", 0), Some(0..6));
    }
}
