//! Document-authored hex color literals (`#rrggbb` / `#rrggbbaa`).
//!
//! Colors are language-agnostic document data, so the scanner lives in the
//! shared syntax kernel and stays free of UI dependencies. Only the
//! unambiguous six- and eight-digit forms are recognized; three- and four-digit
//! hex would collide with ordinary Markdown hashtags such as `#abc` or `#cafe`.

use std::ops::Range;

/// A recognized hex color literal and the byte range it occupies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColorLiteral {
    pub range: Range<usize>,
    /// `0xRRGGBBAA`. The six-digit form reports an opaque alpha.
    pub rgba: u32,
}

/// Byte ranges of every recognizable hex literal in `text`, in source order.
///
/// The scan is boundary-guarded: `a#ff0000`, `##ff0000`, `#ff0000z` and
/// `#ff00000` are not colors, while `color: #ff0000;` and `=#ff0000=` are.
pub fn scan_line(text: &str) -> Vec<ColorLiteral> {
    let bytes = text.as_bytes();
    let mut literals = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'#' {
            index += 1;
            continue;
        }
        let digits_start = index + 1;
        let mut digits_end = digits_start;
        while digits_end < bytes.len() && bytes[digits_end].is_ascii_hexdigit() {
            digits_end += 1;
        }
        let digits = digits_end - digits_start;
        if matches!(digits, 6 | 8)
            && boundary_before_ok(bytes, index)
            && boundary_after_ok(bytes, digits_end)
            && let Some(rgba) = parse_digits(&bytes[digits_start..digits_end])
        {
            literals.push(ColorLiteral {
                range: index..digits_end,
                rgba,
            });
        }
        index = if digits_end > digits_start {
            digits_end
        } else {
            index + 1
        };
    }
    literals
}

/// The `#` must not continue an identifier or another literal.
fn boundary_before_ok(bytes: &[u8], index: usize) -> bool {
    index == 0 || !is_identifier_byte(bytes[index - 1])
}

/// The literal must not continue into a longer word (`#ff0000z`). Hex digits
/// are already consumed, so only non-hex identifier bytes remain to reject.
fn boundary_after_ok(bytes: &[u8], index: usize) -> bool {
    index == bytes.len() || !is_identifier_byte(bytes[index])
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'#')
}

fn parse_digits(digits: &[u8]) -> Option<u32> {
    let value = u32::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()?;
    Some(if digits.len() == 6 {
        (value << 8) | 0xff
    } else {
        value
    })
}

/// The swatch painted behind a literal: the authored color composited over the
/// canvas. A six-digit literal is opaque, so the fill is exactly the authored
/// color on both themes; an eight-digit literal keeps its own alpha. Returns
/// `0xRRGGBBAA`.
pub fn swatch_fill(rgba: u32, background: u32) -> u32 {
    let alpha = rgba & 0xff;
    let mut rgb = 0u32;
    for shift in [16u32, 8, 0] {
        let value = mix_channel(
            (rgba >> (shift + 8)) & 0xff,
            (background >> shift) & 0xff,
            alpha,
        );
        rgb |= value << shift;
    }
    (rgb << 8) | 0xff
}

/// Glyph color that stays readable on the swatch: black or white, picked by
/// WCAG contrast against the composited fill. Returns `0xRRGGBBAA` with full
/// alpha, so callers decode it with `rgba()` like [`swatch_fill`].
pub fn swatch_text_color(rgba: u32, background: u32) -> u32 {
    let fill = swatch_fill(rgba, background) >> 8;
    let swatch = relative_luminance(fill);
    let ink = if contrast_ratio(swatch, 0.0) >= contrast_ratio(swatch, 1.0) {
        0x000000
    } else {
        0xffffff
    };
    (ink << 8) | 0xff
}

/// Integer alpha blend of two channels at `alpha` in `0..=255`.
fn mix_channel(color: u32, background: u32, alpha: u32) -> u32 {
    (color * alpha + background * (0xff - alpha)) / 0xff
}

/// WCAG relative luminance of an `0xRRGGBB` color, in `0..=1`.
fn relative_luminance(rgb: u32) -> f32 {
    0.2126 * linear_channel((rgb >> 16) & 0xff)
        + 0.7152 * linear_channel((rgb >> 8) & 0xff)
        + 0.0722 * linear_channel(rgb & 0xff)
}

fn linear_channel(channel: u32) -> f32 {
    let value = channel as f32 / 255.0;
    if value <= 0.03928 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn contrast_ratio(left: f32, right: f32) -> f32 {
    let (bright, dark) = if left >= right {
        (left, right)
    } else {
        (right, left)
    };
    (bright + 0.05) / (dark + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(text: &str) -> Vec<Range<usize>> {
        scan_line(text)
            .into_iter()
            .map(|literal| literal.range)
            .collect()
    }

    fn rgba_of(text: &str) -> u32 {
        scan_line(text).into_iter().next().expect("literal").rgba
    }

    #[test]
    fn six_and_eight_digit_hex_are_recognized() {
        assert_eq!(ranges("color: #ff0000;"), vec![7..14]);
        assert_eq!(rgba_of("#ff0000"), 0xff0000ff);
        assert_eq!(ranges("#FFAA0080"), vec![0..9]);
        assert_eq!(rgba_of("#FFAA0080"), 0xffaa0080);
        assert_eq!(rgba_of("#abcdef"), 0xabcdefff);
    }

    #[test]
    fn multiple_literals_keep_source_order() {
        let text = "#112233 and #44556677";
        assert_eq!(ranges(text), vec![0..7, 12..21]);
    }

    #[test]
    fn shorthand_and_other_lengths_are_rejected() {
        for text in ["#abc", "#abcd", "#ff00000", "#ff0000ff0"] {
            assert!(scan_line(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn identifier_boundaries_are_enforced() {
        for text in ["a#ff0000", "##ff0000", "#ff0000z", "#ff0000_", "x#ff0000"] {
            assert!(scan_line(text).is_empty(), "{text}");
        }
        assert_eq!(ranges("=#ff0000="), vec![1..8]);
        assert_eq!(ranges(":#ff0000:"), vec![1..8]);
        assert_eq!(ranges("(#ff0000)"), vec![1..8]);
    }

    #[test]
    fn org_keywords_and_headings_are_not_colors() {
        for text in [
            "#+begin_src rust",
            "#+attr_html: :style x",
            "## Heading",
            "# Heading",
        ] {
            assert!(scan_line(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn opaque_literals_fill_with_their_exact_color_on_both_themes() {
        // Six-digit hex is opaque, so the theme must not tint the swatch.
        assert_eq!(swatch_fill(0xff0000ff, 0xffffff), 0xff0000ff);
        assert_eq!(swatch_fill(0xff0000ff, 0x121d28), 0xff0000ff);
        assert_eq!(swatch_fill(0x00ff00ff, 0xffffff), 0x00ff00ff);
    }

    #[test]
    fn eight_digit_literals_composite_their_own_alpha_over_the_canvas() {
        // Half-transparent black over white lands mid grey.
        assert_eq!(swatch_fill(0x00000080, 0xffffff), 0x7f7f7fff);
        // A 27%-opaque literal is mostly canvas.
        assert_eq!(swatch_fill(0x11223344, 0x000000), 0x04090dff);
        assert_eq!(swatch_fill(0x11223344, 0xffffff), 0xbfc4c8ff);
    }

    #[test]
    fn swatch_text_color_picks_the_readable_ink() {
        assert_eq!(swatch_text_color(0xffffffff, 0xffffff), 0x000000ff);
        assert_eq!(swatch_text_color(0x000000ff, 0xffffff), 0xffffffff);
        assert_eq!(swatch_text_color(0x0000ffff, 0xffffff), 0xffffffff);
        assert_eq!(swatch_text_color(0xff0000ff, 0xffffff), 0x000000ff);
        assert_eq!(swatch_text_color(0x808080ff, 0xffffff), 0x000000ff);
    }
}
