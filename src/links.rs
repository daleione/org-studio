//! Org / Markdown link classification and plain-link scanning.
//!
//! The classification rules mirror the official Org Mode manual
//! (<https://orgmode.org/manual/External-Links.html>, `Internal-Links.html`)
//! and the CommonMark / GFM specs (`https://spec.commonmark.org/0.31.2/`,
//! `https://github.github.com/gfm/`). No behavior here is guessed.

use std::{ops::Range, sync::Arc};

mod destination;
pub(crate) use destination::LiteralDestination;
mod file;
pub(crate) use file::{resolve_file_link, split_link_target};

/// Document format context for [`classify`]: a bare word is a fuzzy internal
/// link in Org but a relative file path in Markdown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LinkFormat {
    Org,
    Markdown,
}

/// Style/behavior family of a resolved link target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LinkKind {
    /// `http://`, `https://`, `ftp://`, bare `www.` / URL text.
    External,
    /// `file:`, `attachment:`, `./`, `../`, `/`, `~/` paths.
    File,
    /// `#custom-id`, `*headline`, `id:`, `<<target>>`, `#+NAME:`, MD fragment.
    Internal,
    /// `mailto:`.
    Mail,
    /// `shell:`, `elisp:`, `javascript:` — executed on activation.
    Dangerous,
    /// Any other recognized `scheme:path` (irc, doi, info, …).
    Other,
}

/// A classified link target plus the raw text it came from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LinkInfo {
    pub(crate) kind: LinkKind,
    pub(crate) raw: Arc<str>,
}

impl LinkInfo {
    fn new(kind: LinkKind, raw: &str) -> Self {
        Self {
            kind,
            raw: Arc::from(raw),
        }
    }
}

/// Schemes recognized as bare/plain links in running text (GFM autolinks
/// extension covers `http://`, `https://`, `www.`, `mailto:`, `xmpp:`; the Org
/// manual's plain-link regex covers every type registered in
/// `org-link-parameters` — we keep the safe subset plus `ftp:`).
const PLAIN_SCHEMES: [&str; 6] = ["https://", "http://", "ftp://", "www.", "mailto:", "xmpp:"];

/// Classifies a raw link target (the `[[…]]` inner for Org, the `dest_url` for
/// Markdown) into a [`LinkKind`].
pub(crate) fn classify(raw: &str, format: LinkFormat) -> LinkInfo {
    let target = raw.trim();
    if target.is_empty() {
        return LinkInfo::new(LinkKind::Other, raw);
    }
    if target.starts_with('#')
        || target.starts_with('*')
        || target.starts_with("id:")
        || target.starts_with("<<")
    {
        return LinkInfo::new(LinkKind::Internal, target);
    }
    if target.starts_with("mailto:") {
        return LinkInfo::new(LinkKind::Mail, target);
    }
    if target.starts_with("shell:")
        || target.starts_with("elisp:")
        || target.starts_with("javascript:")
    {
        return LinkInfo::new(LinkKind::Dangerous, target);
    }
    if target.starts_with("file:")
        || target.starts_with("attachment:")
        || target.starts_with("./")
        || target.starts_with("../")
        || target.starts_with('/')
        || target.starts_with("~/")
    {
        return LinkInfo::new(LinkKind::File, target);
    }
    for scheme in ["https://", "http://", "ftp://", "www."] {
        if target.starts_with(scheme) {
            return LinkInfo::new(LinkKind::External, target);
        }
    }
    if let Some((prefix, _)) = target.split_once(':')
        && is_valid_scheme(prefix)
    {
        return LinkInfo::new(LinkKind::Other, target);
    }
    match format {
        // A bare word is a fuzzy link into the current document (Org Manual 4.2).
        LinkFormat::Org => LinkInfo::new(LinkKind::Internal, target),
        // A bare path is a relative file link in Markdown.
        LinkFormat::Markdown => LinkInfo::new(LinkKind::File, target),
    }
}

/// A scheme is 2–32 characters, ASCII letter first, then letters/digits/`+`/`.`/`-`
/// (CommonMark 6.5 scheme rule).
fn is_valid_scheme(prefix: &str) -> bool {
    let mut chars = prefix.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    let mut len = 1;
    for character in chars {
        if !(character.is_ascii_alphanumeric() || matches!(character, '+' | '.' | '-')) {
            return false;
        }
        len += 1;
    }
    (2..=32).contains(&len)
}

/// Scans running text for bare links (`https://…`, `www.…`, `mailto:…`),
/// following the GFM autolink-extension boundary rules: the candidate must
/// start at a word boundary, `<` / whitespace / quotes terminate it, trailing
/// punctuation (`?!.,:*_~`) is excluded, unbalanced trailing `)` is excluded,
/// and an entity-like trailing `&alnum;` tail is excluded.
///
/// Returns byte ranges in `text` plus their classified metadata. Callers must
/// still skip ranges already covered by bracketed links / code spans.
pub(crate) fn scan_plain_links(text: &str, format: LinkFormat) -> Vec<(Range<usize>, LinkInfo)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let preceded_by_word = cursor > 0
            && (bytes[cursor - 1].is_ascii_alphanumeric()
                || matches!(bytes[cursor - 1], b'_' | b'-' | b'.'));
        if preceded_by_word {
            cursor = advance_char(text, cursor);
            continue;
        }
        let Some(scheme) = PLAIN_SCHEMES
            .iter()
            .find(|scheme| text[cursor..].starts_with(**scheme))
            .copied()
        else {
            cursor = advance_char(text, cursor);
            continue;
        };
        let start = cursor;
        let mut end = start + scheme.len();
        while end < bytes.len()
            && !bytes[end].is_ascii_whitespace()
            && !matches!(
                bytes[end],
                b'<' | b'>' | b'(' | b')' | b'[' | b']' | b'"' | b'\'' | b'`'
            )
        {
            end += 1;
        }
        let slice_end = trim_plain_link_tail(text, start, end);
        if slice_end <= start + scheme.len() {
            cursor = advance_char(text, end.max(start + 1));
            continue;
        }
        let raw = &text[start..slice_end];
        out.push((start..slice_end, classify(raw, format)));
        cursor = slice_end;
    }
    out
}

/// Advances one full UTF-8 character from a char boundary.
fn advance_char(text: &str, byte: usize) -> usize {
    if byte >= text.len() {
        return text.len();
    }
    text[byte..]
        .chars()
        .next()
        .map_or(byte + 1, |character| byte + character.len_utf8())
}

fn trim_plain_link_tail(text: &str, start: usize, mut end: usize) -> usize {
    let bytes = text.as_bytes();
    while end > start
        && matches!(
            bytes[end - 1],
            b'?' | b'!' | b'.' | b',' | b':' | b'*' | b'_' | b'~'
        )
    {
        end -= 1;
    }
    // Entity-reference tail exclusion (GFM 6.9): a trailing `;` that closes an
    // `&alnum…` run belongs to the surrounding text, not the link.
    while end > start && bytes[end - 1] == b';' {
        let slice = &text[start..end - 1];
        let Some(amp) = slice.rfind('&') else {
            break;
        };
        let between = &text[start + amp + 1..end - 1];
        if !between.is_empty()
            && between
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
        {
            end = start + amp;
        } else {
            break;
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(raw: &str, format: LinkFormat) -> LinkKind {
        classify(raw, format).kind
    }

    #[test]
    fn org_builtin_types_classify_correctly() {
        assert_eq!(
            kind("https://orgmode.org/", LinkFormat::Org),
            LinkKind::External
        );
        assert_eq!(
            kind("http://example.com", LinkFormat::Org),
            LinkKind::External
        );
        assert_eq!(
            kind("file:papers/last.pdf", LinkFormat::Org),
            LinkKind::File
        );
        assert_eq!(
            kind("file:papers/last.pdf::123", LinkFormat::Org),
            LinkKind::File
        );
        assert_eq!(
            kind("file:projects.org::*task title", LinkFormat::Org),
            LinkKind::File
        );
        assert_eq!(kind("./papers/last.pdf", LinkFormat::Org), LinkKind::File);
        assert_eq!(
            kind("/home/dominik/jupiter.jpg", LinkFormat::Org),
            LinkKind::File
        );
        assert_eq!(kind("~/notes.org", LinkFormat::Org), LinkKind::File);
        assert_eq!(
            kind("attachment:projects.org", LinkFormat::Org),
            LinkKind::File
        );
        assert_eq!(
            kind("id:B7423F4D-2E8A-471B-8810-C40F074717E9", LinkFormat::Org),
            LinkKind::Internal
        );
        assert_eq!(kind("#my-custom-id", LinkFormat::Org), LinkKind::Internal);
        assert_eq!(kind("*Some section", LinkFormat::Org), LinkKind::Internal);
        assert_eq!(
            kind("mailto:adent@galaxy.net", LinkFormat::Org),
            LinkKind::Mail
        );
        assert_eq!(kind("shell:ls *.org", LinkFormat::Org), LinkKind::Dangerous);
        assert_eq!(
            kind("elisp:(find-file \"x\")", LinkFormat::Org),
            LinkKind::Dangerous
        );
        assert_eq!(
            kind("irc:/irc.com/#emacs/bob", LinkFormat::Org),
            LinkKind::Other
        );
        assert_eq!(kind("doi:10.1000/182", LinkFormat::Org), LinkKind::Other);
        assert_eq!(
            kind("info:org#External links", LinkFormat::Org),
            LinkKind::Other
        );
        assert_eq!(kind("news:comp.emacs", LinkFormat::Org), LinkKind::Other);
        assert_eq!(
            kind("docview:papers/last.pdf::9", LinkFormat::Org),
            LinkKind::Other
        );
        assert_eq!(kind("bbdb:R.*Stallman", LinkFormat::Org), LinkKind::Other);
        // Fuzzy links are internal (Org Manual 4.2).
        assert_eq!(kind("My Target", LinkFormat::Org), LinkKind::Internal);
        assert_eq!(kind("some words", LinkFormat::Org), LinkKind::Internal);
    }

    #[test]
    fn markdown_targets_classify_correctly() {
        assert_eq!(
            kind("https://example.com", LinkFormat::Markdown),
            LinkKind::External
        );
        assert_eq!(kind("a.md", LinkFormat::Markdown), LinkKind::File);
        assert_eq!(kind("docs/plan.org", LinkFormat::Markdown), LinkKind::File);
        assert_eq!(kind("/abs/path.png", LinkFormat::Markdown), LinkKind::File);
        assert_eq!(kind("#section", LinkFormat::Markdown), LinkKind::Internal);
        assert_eq!(
            kind("mailto:foo@bar.baz", LinkFormat::Markdown),
            LinkKind::Mail
        );
        assert_eq!(
            kind("javascript:alert(1)", LinkFormat::Markdown),
            LinkKind::Dangerous
        );
        assert_eq!(
            kind("tel:+8612345678", LinkFormat::Markdown),
            LinkKind::Other
        );
    }

    #[test]
    fn plain_links_respect_gfm_boundary_rules() {
        let text = "Visit https://example.com/path?q=1 and www.commonmark.org. Trailing! (https://a.com/b))";
        let hits = scan_plain_links(text, LinkFormat::Org);
        let found: Vec<&str> = hits.iter().map(|(range, _)| &text[range.clone()]).collect();
        assert_eq!(
            found,
            vec![
                "https://example.com/path?q=1",
                "www.commonmark.org",
                "https://a.com/b"
            ]
        );
        assert!(hits.iter().all(|(range, _)| range.start < range.end));
        for (range, meta) in hits {
            assert_eq!(meta.kind, LinkKind::External);
            assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
        }
    }

    #[test]
    fn plain_links_do_not_trigger_inside_words() {
        let text = "abc.www.x.com [[https://kept.org]] xmailto:a@b.c email@example.com";
        let hits = scan_plain_links(text, LinkFormat::Org);
        let found: Vec<&str> = hits.iter().map(|(range, _)| &text[range.clone()]).collect();
        // `abc.www.x.com` is blocked by the word boundary; `mailto:` inside
        // `xmailto:` is blocked; the bare email is not a plain scheme. The
        // bracketed link IS scanned (terminated at `]`) but the editor layer
        // filters ranges covered by the `[[…]]` link parser.
        assert_eq!(found, vec!["https://kept.org"]);
    }

    #[test]
    fn plain_links_trim_entity_tails() {
        let text =
            "see www.example.com/search?q=commonmark&hl=en and www.google.com/search?q=x&hl;";
        let hits = scan_plain_links(text, LinkFormat::Org);
        let found: Vec<&str> = hits.iter().map(|(range, _)| &text[range.clone()]).collect();
        assert_eq!(
            found,
            vec![
                "www.example.com/search?q=commonmark&hl=en",
                "www.google.com/search?q=x"
            ]
        );
    }
}
