pub(crate) fn fence_start(line: &str) -> Option<(char, usize, Option<String>)> {
    let marker = line.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let count = line.chars().take_while(|ch| *ch == marker).count();
    if count < 3 {
        return None;
    }
    let marker_bytes = marker.len_utf8() * count;
    let info = line[marker_bytes..].trim();
    if marker == '`' && info.contains('`') {
        return None;
    }
    let language = info
        .split_whitespace()
        .next()
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Some((marker, count, language))
}

pub(crate) fn is_closing_fence(line: &str, marker: char, opening_count: usize) -> bool {
    let count = line.chars().take_while(|ch| *ch == marker).count();
    count >= opening_count && line[marker.len_utf8() * count..].trim().is_empty()
}

/// CommonMark fenced-code-block opening fence, on the raw source line.
///
/// At most three leading spaces, then a run of at least three backticks or
/// tildes. A backtick fence's info string may not itself contain a backtick.
/// This is the single fence-opening rule shared by the editor's incremental
/// scanner, Reading, folding, the outline and the copy button.
pub(crate) fn fence_open(line: &str) -> Option<(char, usize, Option<String>)> {
    let trimmed = line.trim_start();
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    fence_start(trimmed)
}

/// CommonMark fenced-code-block closing fence, on the raw source line.
///
/// At most three leading spaces, the same marker as the opening fence, a run
/// at least as long as the opening run, and nothing but whitespace after the
/// markers.
pub(crate) fn fence_close(line: &str, marker: char, opening_count: usize) -> bool {
    let trimmed = line.trim_start();
    if line.len() - trimmed.len() > 3 {
        return false;
    }
    is_closing_fence(trimmed, marker, opening_count)
}

pub(crate) fn atx_heading(line: &str) -> Option<(u16, usize)> {
    let count = line.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&count)
        || line
            .as_bytes()
            .get(count)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
    {
        return None;
    }
    let rest = line[count..].trim_start();
    Some((count as u16, line.len() - rest.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fence_open_follows_commonmark_opening_rules() {
        assert_eq!(fence_open("```"), Some(('`', 3, None)));
        assert_eq!(fence_open("```json"), Some(('`', 3, Some("json".into()))));
        assert_eq!(
            fence_open("```json {#id}"),
            Some(('`', 3, Some("json".into())))
        );
        assert_eq!(
            fence_open("````markdown"),
            Some(('`', 4, Some("markdown".into())))
        );
        assert_eq!(fence_open("~~~"), Some(('~', 3, None)));
        assert_eq!(fence_open("~~~bash"), Some(('~', 3, Some("bash".into()))));
        assert_eq!(
            fence_open("   ```json"),
            Some(('`', 3, Some("json".into())))
        );
        assert_eq!(fence_open("    ```json"), None);
        assert_eq!(fence_open("``"), None);
        assert_eq!(fence_open("a```"), None);
        assert_eq!(fence_open(""), None);
        // A backtick inside the info string of a backtick fence is invalid.
        assert_eq!(fence_open("```a`b```"), None);
        // Tildes are allowed in tilde info strings.
        assert_eq!(
            fence_open("~~~a~b~~~"),
            Some(('~', 3, Some("a~b~~~".into())))
        );
    }

    #[test]
    fn fence_close_follows_commonmark_closing_rules() {
        let opening = ('`', 3);
        assert!(fence_close("```", opening.0, opening.1));
        assert!(fence_close("````", opening.0, opening.1));
        assert!(fence_close("``` ", opening.0, opening.1));
        assert!(fence_close("  ```", opening.0, opening.1));
        assert!(!fence_close("```json", opening.0, opening.1));
        assert!(!fence_close("~~~", opening.0, opening.1));
        assert!(!fence_close("    ```", opening.0, opening.1));
        assert!(!fence_close("a```", opening.0, opening.1));
        // A closing run shorter than the opening run does not close.
        assert!(!fence_close("```", '`', 4));
        assert!(fence_close("````", '`', 4));
        assert!(fence_close("~~~", '~', 3));
        assert!(!fence_close("````", '~', 3));
    }

    #[test]
    fn fence_open_and_close_agree_with_the_trimmed_primitives() {
        for (line, expected) in [
            ("```", Some(('`', 3, None))),
            ("   ```json", Some(('`', 3, Some("json".into())))),
            ("    ```", None),
            ("", None),
        ] {
            assert_eq!(fence_open(line), expected, "open {line:?}");
        }
        assert!(fence_close("   ```", '`', 3));
        assert!(!fence_close("    ```", '`', 3));
        assert!(is_closing_fence("```", '`', 3));
        assert!(!is_closing_fence("    ```", '`', 3));
    }
}
