//! Org affiliated-keyword parsing for inline image display attributes.
//!
//! Only `#+ATTR_ORG:` participates in image display; the other affiliated
//! keywords are recognized so that walking up from an image line stops at the
//! end of its keyword block.

use crate::document::{DocumentSnapshot, LineIndex, TextSnapshot};

/// A length written in an `#+ATTR_ORG:` line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ImageLength {
    /// Absolute logical pixels (`320`, `320px`, `320pt` normalized to px).
    Px(f32),
    /// Percentage of the available width (`50%`).
    Percent(f32),
    /// Multiple of the content font size (`10em`).
    Em(f32),
}

/// Display attributes for one inline image.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ImageAttributeSpec {
    pub(crate) width: Option<ImageLength>,
    pub(crate) height: Option<ImageLength>,
    pub(crate) scale: Option<f32>,
}

impl ImageAttributeSpec {
    pub(crate) fn is_empty(&self) -> bool {
        self.width.is_none() && self.height.is_none() && self.scale.is_none()
    }

    /// An explicit width in logical pixels, as produced by a drag.
    pub(crate) fn from_width_px(width: f32) -> Self {
        Self {
            width: Some(ImageLength::Px(width)),
            ..Self::default()
        }
    }
}

/// Whether the line is an `#+ATTR_ORG:` keyword, regardless of its values.
pub(crate) fn is_attr_org_line(line: &str) -> bool {
    keyword_name(line).is_some_and(|name| name.eq_ignore_ascii_case("ATTR_ORG"))
}

/// Whether the line opens an Org affiliated keyword block.
pub(crate) fn is_affiliated_keyword(line: &str) -> bool {
    let Some(keyword) = keyword_name(line) else {
        return false;
    };
    matches!(
        keyword.to_ascii_uppercase().as_str(),
        "ATTR_ORG"
            | "ATTR_HTML"
            | "ATTR_LATEX"
            | "ATTR_MD"
            | "ATTR_TYPST"
            | "CAPTION"
            | "NAME"
            | "RESULTS"
            | "LABEL"
            | "HEADER"
            | "PLOT"
    )
}

/// Line index of the closest `#+ATTR_ORG:` line above `line`, walking only the
/// affiliated keyword block directly above it.
pub(crate) fn attr_org_line_above(snapshot: &DocumentSnapshot, line: u64) -> Option<u64> {
    let mut index = line;
    while index > 0 {
        index -= 1;
        let range = snapshot.line_content_range(LineIndex(index)).ok()?;
        let text = snapshot.copy_range(range);
        if !is_affiliated_keyword(&text) {
            return None;
        }
        if is_attr_org_line(&text) {
            return Some(index);
        }
    }
    None
}

/// `#+ATTR_ORG:` attributes attached to the element on `line`.
pub(crate) fn image_attributes_at(snapshot: &DocumentSnapshot, line: u64) -> ImageAttributeSpec {
    attr_org_line_above(snapshot, line)
        .and_then(|index| snapshot.line_content_range(LineIndex(index)).ok())
        .and_then(|range| parse_attr_org_line(&snapshot.copy_range(range)))
        .unwrap_or_default()
}

/// Parses one `#+ATTR_ORG:` line. Other keywords and unusable input return `None`.
pub(crate) fn parse_attr_org_line(line: &str) -> Option<ImageAttributeSpec> {
    if !keyword_name(line)?.eq_ignore_ascii_case("ATTR_ORG") {
        return None;
    }
    let mut spec = ImageAttributeSpec::default();
    let tokens = key_value_tokens(line);
    for (key, value) in tokens {
        match key.to_ascii_lowercase().as_str() {
            ":width" => spec.width = parse_length(value),
            ":height" => spec.height = parse_length(value),
            ":scale" => {
                spec.scale = value
                    .parse::<f32>()
                    .ok()
                    .filter(|scale| scale.is_finite() && *scale > 0.0)
            }
            _ => {}
        }
    }
    (!spec.is_empty()).then_some(spec)
}

/// Rewrites an `#+ATTR_ORG:` line to carry `:width <width_px>`.
///
/// `:scale` and `:height` are dropped because they would either override the
/// explicit width or contradict it; every other token keeps its position.
/// Returns `None` when nothing would remain.
pub(crate) fn attr_org_line_with_width(line: &str, width_px: f32) -> Option<String> {
    let keyword = attr_org_literal(line)?;
    let width = format_width(width_px);
    let mut out = Vec::new();
    let mut replaced = false;
    let tokens = key_value_tokens(line);
    for (key, value) in tokens {
        match key.to_ascii_lowercase().as_str() {
            ":width" => {
                if !replaced {
                    out.push(format!("{key} {width}"));
                    replaced = true;
                }
            }
            ":scale" | ":height" => {}
            _ => out.push(format!("{key} {value}")),
        }
    }
    if !replaced {
        out.push(format!(":width {width}"));
    }
    (!out.is_empty()).then(|| format!("{keyword} {}", out.join(" ")))
}

/// Rewrites an `#+ATTR_ORG:` line without its `:width` token.
///
/// Returns `None` when no other attribute remains, which means the whole line
/// can be removed.
pub(crate) fn attr_org_line_without_width(line: &str) -> Option<String> {
    let keyword = attr_org_literal(line)?;
    let mut out = Vec::new();
    let tokens = key_value_tokens(line);
    for (key, value) in tokens {
        if !key.eq_ignore_ascii_case(":width") {
            out.push(format!("{key} {value}"));
        }
    }
    (!out.is_empty()).then(|| format!("{keyword} {}", out.join(" ")))
}

fn keyword_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("#+")?;
    let end = rest
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// The `#+ATTR_ORG:` literal, preserving the original spelling.
///
/// Returns `None` for every other keyword so a rewrite can never turn an
/// unrelated affiliated keyword into an image attribute.
fn attr_org_literal(line: &str) -> Option<String> {
    if !keyword_name(line)?.eq_ignore_ascii_case("ATTR_ORG") {
        return None;
    }
    let trimmed = line.trim_start();
    let name = keyword_name(line)?;
    let rest = trimmed.strip_prefix("#+")?;
    let after = &rest[name.len()..];
    let after = if let Some(closing) = after.strip_prefix('[') {
        closing.split_once(']').map_or(after, |(_, rest)| rest)
    } else {
        after
    };
    after.strip_prefix(':')?;
    Some(format!("#+{name}:"))
}

/// Iterates `:key value` pairs of an attribute line, keeping the leading colon
/// on each key so callers can rebuild the line verbatim.
fn key_value_tokens(line: &str) -> impl Iterator<Item = (&str, &str)> {
    let tokens = line
        .split_whitespace()
        .skip_while(|token| !token.starts_with(':'))
        .collect::<Vec<_>>();
    let mut index = 0;
    std::iter::from_fn(move || {
        while index < tokens.len() {
            let key = tokens[index];
            index += 1;
            if !key.starts_with(':') {
                continue;
            }
            let value = tokens.get(index).copied().unwrap_or("");
            if !value.starts_with(':') {
                index += 1;
                return Some((key, value));
            }
            return Some((key, ""));
        }
        None
    })
}

fn parse_length(value: &str) -> Option<ImageLength> {
    // Strip a known suffix before parsing the number: `10em` must not be read
    // as the exponent form `10e`, and unknown suffixes are rejected.
    let lower = value.trim().to_ascii_lowercase();
    let (number, unit) = if let Some(number) = lower.strip_suffix("px") {
        (number, Unit::Px)
    } else if let Some(number) = lower.strip_suffix("pt") {
        (number, Unit::Pt)
    } else if let Some(number) = lower.strip_suffix("em") {
        (number, Unit::Em)
    } else if let Some(number) = lower.strip_suffix('%') {
        (number, Unit::Percent)
    } else {
        (lower.as_str(), Unit::Px)
    };
    let number = number
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())?;
    Some(match unit {
        Unit::Px => ImageLength::Px(number),
        Unit::Pt => ImageLength::Px(number * 96.0 / 72.0),
        Unit::Em => ImageLength::Em(number),
        Unit::Percent => ImageLength::Percent(number),
    })
}

enum Unit {
    Px,
    Pt,
    Em,
    Percent,
}

fn format_width(width_px: f32) -> String {
    let rounded = width_px.round();
    if (width_px - rounded).abs() < 0.001 {
        format!("{}", rounded as i64)
    } else {
        format!("{width_px}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_width_and_scale() {
        let spec = parse_attr_org_line("#+ATTR_ORG: :width 320 :scale 0.5").unwrap();
        assert_eq!(spec.width, Some(ImageLength::Px(320.0)));
        assert_eq!(spec.scale, Some(0.5));
        // Attributes this version does not act on still count as "present".
        assert!(parse_attr_org_line("#+ATTR_ORG: :align center :scale 1").is_some());
    }

    #[test]
    fn parses_units_and_ignores_malformed_values() {
        for (line, width) in [
            ("#+attr_org: :width 320", ImageLength::Px(320.0)),
            ("#+ATTR_ORG: :width 320px", ImageLength::Px(320.0)),
            ("#+ATTR_ORG: :width 72pt", ImageLength::Px(96.0)),
            ("#+ATTR_ORG: :width 50%", ImageLength::Percent(50.0)),
            ("#+ATTR_ORG: :width 10em", ImageLength::Em(10.0)),
        ] {
            assert_eq!(
                parse_attr_org_line(line).unwrap().width,
                Some(width),
                "{line}"
            );
        }
        // Nothing usable left means "no spec", so the image keeps its auto size.
        assert!(parse_attr_org_line("#+ATTR_ORG: :width abc :scale 0").is_none());
        assert!(parse_attr_org_line("#+ATTR_ORG: :width abc").is_none());
        // A usable key survives alongside unusable ones.
        let spec = parse_attr_org_line("#+ATTR_ORG: :width abc :scale 0.5").unwrap();
        assert_eq!(spec.width, None);
        assert_eq!(spec.scale, Some(0.5));
    }

    #[test]
    fn recognizes_affiliated_and_attr_org_lines() {
        for line in [
            "#+ATTR_ORG: :width 1",
            "#+ATTR_HTML: :width 1",
            "#+CAPTION: hi",
            "#+RESULTS:",
            "#+name: x",
        ] {
            assert!(is_affiliated_keyword(line), "{line}");
        }
        for line in [
            "",
            "[[file:a.png]]",
            "#+begin_src typst",
            "- item",
            "* Head",
        ] {
            assert!(!is_affiliated_keyword(line), "{line}");
        }
        assert!(is_attr_org_line("#+ATTR_ORG: :width abc"));
        assert!(is_attr_org_line("#+attr_org:"));
        assert!(!is_attr_org_line("#+ATTR_HTML: :width 320"));
        assert!(!is_attr_org_line("#+RESULTS:"));
    }

    #[test]
    fn finds_attributes_in_the_keyword_block_above() {
        let text = crate::document::DocumentSnapshot::from_utf8(
            b"#+RESULTS:\n#+ATTR_ORG: :width 320\n[[file:a.png]]\n\n#+ATTR_ORG: :width 9\n\n[[file:b.png]]\n".to_vec(),
        )
        .expect("fixture document");
        // The closest keyword wins; other affiliated keywords are walked through.
        assert_eq!(
            image_attributes_at(&text, 2).width,
            Some(ImageLength::Px(320.0))
        );
        // A blank line ends the affiliated keyword block.
        assert_eq!(image_attributes_at(&text, 6).width, None);
        assert_eq!(attr_org_line_above(&text, 6), None);
    }

    #[test]
    fn rewrites_only_the_width_token() {
        // Replaced in place, so the author's token order survives.
        assert_eq!(
            attr_org_line_with_width("#+ATTR_ORG: :width 320 :align center", 512.0).unwrap(),
            "#+ATTR_ORG: :width 512 :align center"
        );
        // Conflicting size keys are dropped; the width is always written.
        assert_eq!(
            attr_org_line_with_width("#+ATTR_ORG: :height 100 :width 320", 256.0).unwrap(),
            "#+ATTR_ORG: :width 256"
        );
        assert_eq!(
            attr_org_line_with_width("#+ATTR_ORG: :scale 0.5", 300.0).unwrap(),
            "#+ATTR_ORG: :width 300"
        );
        // Only an actual `#+ATTR_ORG:` line may be rewritten.
        assert!(attr_org_line_with_width("#+RESULTS:", 300.0).is_none());
        assert_eq!(
            attr_org_line_without_width("#+ATTR_ORG: :width 320 :align center").unwrap(),
            "#+ATTR_ORG: :align center"
        );
        // Nothing left to keep means the whole line can go.
        assert!(attr_org_line_without_width("#+ATTR_ORG: :width 320").is_none());
    }
}
