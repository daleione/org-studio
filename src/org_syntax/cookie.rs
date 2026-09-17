//! Org statistics cookies: the progress token that ends a heading, either a count
//! (`[4/7]`) or a percentage (`[57%]`).

use std::ops::Range;

/// The trailing cookie of one line with its progress ratio, as byte offsets into
/// `line`: `** DONE Phase 3 [4/7]` → `(16..21, 4.0 / 7.0)`.
pub(crate) fn trailing_progress(line: &str) -> Option<(Range<usize>, f32)> {
    let text = line.trim_end_matches(['\r', '\n']).trim_end();
    let start = text.rfind('[')?;
    let range = start..text.len();
    let ratio = progress(&text[range.clone()])?;
    Some((range, ratio))
}

/// Progress of one cookie token: `[4/7]` → `4.0 / 7.0`. `None` covers both non-cookies
/// and the placeholders that carry no ratio (`[/]`, `[0/0]`).
pub(crate) fn progress(token: &str) -> Option<f32> {
    let inner = token.strip_prefix('[')?.strip_suffix(']')?;
    if let Some(percent) = inner.strip_suffix('%') {
        if percent.is_empty() || !percent.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        return Some((percent.parse::<f32>().ok()? / 100.0).clamp(0.0, 1.0));
    }
    let (done, total) = inner.split_once('/')?;
    let digits = |value: &str| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(done) || !digits(total) {
        return None;
    }
    let total: f32 = total.parse().ok()?;
    if total <= 0.0 {
        return None;
    }
    Some((done.parse::<f32>().ok()? / total).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_cookie_reports_its_range_and_ratio() {
        let line = "** DONE Phase 3：颜色收敛与深色验收 [4/7]";
        let (range, ratio) = trailing_progress(line).expect("trailing cookie");
        assert_eq!(&line[range], "[4/7]");
        assert!((ratio - 4.0f32 / 7.0).abs() < f32::EPSILON);
    }

    #[test]
    fn trailing_cookie_ignores_trailing_whitespace() {
        let line = "* Heading [57%]  \r\n";
        let (range, ratio) = trailing_progress(line).expect("trailing cookie");
        assert_eq!(&line[range], "[57%]");
        assert!((ratio - 0.57).abs() < f32::EPSILON);
    }

    #[test]
    fn trailing_cookie_requires_the_last_token_to_be_a_cookie() {
        assert_eq!(trailing_progress("* Heading [4/7] body"), None);
        assert_eq!(trailing_progress("* Heading [#A]"), None);
        assert_eq!(trailing_progress("* Heading"), None);
        assert_eq!(trailing_progress(""), None);
    }

    #[test]
    fn progress_handles_counts_percentages_and_placeholders() {
        assert_eq!(progress("[0/7]"), Some(0.0));
        assert_eq!(progress("[7/7]"), Some(1.0));
        assert_eq!(progress("[9/7]"), Some(1.0));
        assert_eq!(progress("[0%]"), Some(0.0));
        assert_eq!(progress("[100%]"), Some(1.0));
        assert_eq!(progress("[150%]"), Some(1.0));
        assert_eq!(progress("[0/0]"), None);
        assert_eq!(progress("[/]"), None);
        assert_eq!(progress("[/7]"), None);
        assert_eq!(progress("[4/]"), None);
        assert_eq!(progress("[a/b]"), None);
        assert_eq!(progress("[%]"), None);
        assert_eq!(progress("4/7"), None);
    }
}
