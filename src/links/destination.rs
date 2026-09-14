//! Literal destination editing, shared by availability checks and edit validation.
use std::ops::Range;

/// The syntax surrounding the destination determines which replacements are safe.
/// Reference-style Markdown links are intentionally excluded: editing a reference
/// definition is a different operation from replacing bytes in the hovered link.
pub(crate) struct LiteralDestination {
    pub(crate) range: Range<usize>,
    org_brackets: bool,
}
impl LiteralDestination {
    pub(crate) fn parse(source: &str, target: &str) -> Option<Self> {
        let org_brackets = source.starts_with("[[");
        let start = if source == target {
            0
        } else if org_brackets {
            2
        } else if source.starts_with('<') {
            1
        } else {
            let index = source.find("](")?;
            index + 2 + usize::from(source[index + 2..].starts_with('<'))
        };
        let range = start..start + target.len();
        (source.get(range.clone()) == Some(target)).then_some(Self {
            range,
            org_brackets,
        })
    }

    /// Preserve the editor's conservative delimiter rules. Validation lives with
    /// range extraction so callers cannot apply Org rules to a Markdown link.
    pub(crate) fn accepts(&self, target: &str) -> bool {
        !target.is_empty()
            && !target.contains(['[', ']', '\n', '\r'])
            && (self.org_brackets || !target.contains(['(', ')', '<', '>']))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_literals_preserves_labels_titles_and_markup() {
        for (source, old, expected) in [
            (
                "[[https://a][中文说明]]",
                "https://a",
                "[[https://b][中文说明]]",
            ),
            (
                "[说明](https://a \"title\")",
                "https://a",
                "[说明](https://b \"title\")",
            ),
            ("[说明](<https://a>)", "https://a", "[说明](<https://b>)"),
            ("<https://a>", "https://a", "<https://b>"),
            ("https://a", "https://a", "https://b"),
        ] {
            let destination = LiteralDestination::parse(source, old).unwrap();
            assert!(destination.accepts("https://b"));
            let mut edited = source.to_owned();
            edited.replace_range(destination.range, "https://b");
            assert_eq!(edited, expected);
        }
        assert!(LiteralDestination::parse("[caption][ref]", "https://a").is_none());
        assert!(LiteralDestination::parse("[[https://a]]", "https://b").is_none());
    }

    #[test]
    fn delimiter_validation_depends_on_surrounding_syntax() {
        let org = LiteralDestination::parse("[[https://a]]", "https://a").unwrap();
        let markdown = LiteralDestination::parse("[a](https://a)", "https://a").unwrap();
        for invalid in ["", "https://b]\n* Heading", "https://b\rtext", "[[other]]"] {
            assert!(!org.accepts(invalid));
            assert!(!markdown.accepts(invalid));
        }
        assert!(org.accepts("https://b/(part)"));
        assert!(!markdown.accepts("https://b) extra"));
    }
}
