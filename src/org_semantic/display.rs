//! File-local display policy and lossless inline presentation, shared by editor consumers.

use std::ops::Range;

use crate::document::{
    ByteRange, DocumentId, DocumentSnapshot, LineCursor, LineIndex, Revision, RevisionDelta,
    RevisionRange, TextProjection, TextSnapshot,
};
use crate::org_syntax::inline::{BracketLink, InlineKind, parse};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) enum OrgHeadingStyle {
    Source,
    HiddenStars,
    #[default]
    Bullets,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct OrgDisplayOptions {
    pub(crate) descriptive_links: bool,
    pub(crate) heading_style: OrgHeadingStyle,
    pub(crate) hide_emphasis_markers: bool,
    /// None preserves the existing product default: pretty table data, literal prose entities.
    pub(crate) pretty_entities: Option<bool>,
}

impl Default for OrgDisplayOptions {
    fn default() -> Self {
        Self {
            descriptive_links: true,
            heading_style: OrgHeadingStyle::default(),
            hide_emphasis_markers: true,
            pretty_entities: None,
        }
    }
}

impl OrgDisplayOptions {
    pub(crate) fn apply_startup(&mut self, source: &str) {
        for word in source.split_whitespace() {
            match word.to_ascii_lowercase().as_str() {
                "descriptivelinks" => self.descriptive_links = true,
                "literallinks" => self.descriptive_links = false,
                "hidestars" => self.heading_style = OrgHeadingStyle::HiddenStars,
                "showstars" => self.heading_style = OrgHeadingStyle::Source,
                "entitiespretty" => self.pretty_entities = Some(true),
                "entitiesplain" => self.pretty_entities = Some(false),
                _ => {}
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DisplayDirective {
    pub(crate) source: RevisionRange,
    pub(crate) value: String,
}

/// Index only the directive lines. Normal edits update touched lines instead of rescanning
/// the whole file on every keystroke. The syntax consumer determines whether a candidate is
/// an actual keyword or literal text inside a block.
pub(crate) struct OrgDisplayConfig {
    pub(crate) document_id: DocumentId,
    pub(crate) revision: Revision,
    pub(crate) directives: Vec<DisplayDirective>,
}

impl OrgDisplayConfig {
    pub(crate) fn new(snapshot: &DocumentSnapshot) -> Self {
        let mut cursor = LineCursor::new(snapshot);
        let mut directives = Vec::new();
        while let Some(line) = cursor.next_line() {
            if let Some(entry) = directive(snapshot.revision(), line.range, &line.text) {
                directives.push(entry);
            }
        }
        Self {
            document_id: snapshot.document_id(),
            revision: snapshot.revision(),
            directives,
        }
    }

    pub(crate) fn apply_delta(&mut self, snapshot: &DocumentSnapshot, delta: &RevisionDelta) {
        if self.document_id != snapshot.document_id()
            || self.revision != delta.before
            || snapshot.revision() != delta.after
        {
            return;
        }
        let mut touched = Vec::new();
        let mut shift = 0_i128;
        for edit in delta.edits.iter() {
            let start = (i128::from(edit.old.start.0) + shift) as u64;
            let first = snapshot.line_of_byte(crate::document::ByteOffset(start));
            let last = snapshot.line_of_byte(crate::document::ByteOffset(start + edit.new_len));
            touched.extend(first..=last);
            shift += i128::from(edit.new_len) - i128::from(edit.old.len());
        }
        touched.sort_unstable();
        touched.dedup();
        self.directives = self
            .directives
            .drain(..)
            .filter_map(|mut entry| {
                entry.source = delta.map_range(entry.source).ok()?;
                let line = snapshot.line_of_byte(entry.source.range.start);
                (!touched.contains(&line)).then_some(entry)
            })
            .collect();
        for line in touched {
            let Ok(range) = snapshot.line_range(LineIndex(line)) else {
                continue;
            };
            if let Some(entry) = directive(delta.after, range, &snapshot.copy_range(range)) {
                self.directives.push(entry);
            }
        }
        self.directives
            .sort_unstable_by_key(|entry| entry.source.range.start);
        self.revision = delta.after;
    }
}

fn directive(revision: Revision, range: ByteRange, source: &str) -> Option<DisplayDirective> {
    let (key, value) = source.trim_start().strip_prefix("#+")?.split_once(':')?;
    key.eq_ignore_ascii_case("STARTUP")
        .then(|| DisplayDirective {
            source: RevisionRange::new(revision, range),
            value: value.trim().to_owned(),
        })
}

#[derive(Default)]
pub(crate) struct InlineDisplayContext<'a> {
    pub(crate) start: usize,
    pub(crate) table: bool,
    pub(crate) caret: Option<usize>,
    pub(crate) reveal: &'a [Range<usize>],
}

pub(crate) fn project_inline(
    source: &str,
    options: OrgDisplayOptions,
    context: InlineDisplayContext<'_>,
) -> TextProjection {
    TextProjection::new(source).replace(inline_edits(source, options, context))
}

pub(crate) fn project_table(
    source: &str,
    options: OrgDisplayOptions,
    caret: Option<usize>,
    reveal: &[Range<usize>],
) -> TextProjection {
    let parsed = crate::document::table::parse_line(source, crate::document::DocumentFormat::Org);
    let mut edits = Vec::new();
    if !parsed.separator {
        for cell in parsed.cells {
            let range = cell.raw_range;
            let local_reveal = reveal
                .iter()
                .filter_map(|reveal| {
                    let start = reveal.start.max(range.start);
                    let end = reveal.end.min(range.end);
                    (start < end).then(|| start - range.start..end - range.start)
                })
                .collect::<Vec<_>>();
            edits.extend(
                inline_edits(
                    &source[range.clone()],
                    options,
                    InlineDisplayContext {
                        table: true,
                        caret: caret.and_then(|caret| caret.checked_sub(range.start)),
                        reveal: &local_reveal,
                        ..Default::default()
                    },
                )
                .into_iter()
                .map(|(edit, text)| (edit.start + range.start..edit.end + range.start, text)),
            );
        }
    }
    TextProjection::new(source).replace(edits)
}

pub(crate) fn inline_edits(
    source: &str,
    options: OrgDisplayOptions,
    context: InlineDisplayContext<'_>,
) -> Vec<(Range<usize>, String)> {
    let mut edits = Vec::new();
    let parsed = parse(&source[context.start..]);
    for span in &parsed.spans {
        let range = span.source.start + context.start..span.source.end + context.start;
        match span.kind {
            InlineKind::Link if options.descriptive_links => {
                if let Some(link) = BracketLink::at(source, range.start) {
                    let prefix = link.source.start..link.visible.start;
                    let suffix = link.visible.end..link.source.end;
                    if !intersects_reveal(&prefix, context.reveal)
                        && !intersects_reveal(&suffix, context.reveal)
                    {
                        edits.push((prefix, String::new()));
                        edits.push((suffix, String::new()));
                    }
                }
            }
            InlineKind::Bold
            | InlineKind::Italic
            | InlineKind::Underline
            | InlineKind::Strike
            | InlineKind::Code
            | InlineKind::Verbatim
                if options.hide_emphasis_markers =>
            {
                let prefix = range.start..range.start + 1;
                let suffix = range.end - 1..range.end;
                if !intersects_reveal(&prefix, context.reveal)
                    && !intersects_reveal(&suffix, context.reveal)
                {
                    edits.push((prefix, String::new()));
                    edits.push((suffix, String::new()));
                }
            }
            InlineKind::Entity if options.pretty_entities.unwrap_or(context.table) => {
                if intersects_reveal(&range, context.reveal) {
                    continue;
                }
                if let Some(value) = span.entity_replacement(
                    &source[context.start..],
                    &parsed.text,
                    context
                        .caret
                        .and_then(|caret| caret.checked_sub(context.start)),
                ) {
                    edits.push((range, value));
                }
            }
            _ => {}
        }
    }
    edits.sort_unstable_by_key(|(range, _)| range.start);
    edits
}

fn intersects_reveal(range: &Range<usize>, reveal: &[Range<usize>]) -> bool {
    reveal
        .iter()
        .any(|other| other.start < range.end && other.end > range.start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{DocumentBuffer, EditTransaction, TextEdit};

    #[test]
    fn directive_index_tracks_insertions_deletions_and_multiple_edits() {
        let source = "intro\n#+STARTUP: literallinks\nbody\n#+STARTUP: hidestars\n";
        let mut buffer = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
        let mut config = OrgDisplayConfig::new(&buffer.snapshot());
        let edits = [
            vec![TextEdit::new(ByteRange::new(0, 0), "prefix\n")],
            vec![TextEdit::new(ByteRange::new(7, 13), "")],
            vec![
                TextEdit::new(ByteRange::new(0, 0), "#+STARTUP: entitiesplain\n"),
                TextEdit::new(ByteRange::new(18, 30), "descriptivelinks"),
            ],
        ];
        for edits in edits {
            let delta = buffer
                .commit(EditTransaction::new(buffer.revision(), edits))
                .unwrap();
            let snapshot = buffer.snapshot();
            config.apply_delta(&snapshot, &delta);
            let full = OrgDisplayConfig::new(&snapshot);
            assert_eq!(config.revision, full.revision);
            assert_eq!(
                config
                    .directives
                    .iter()
                    .map(|d| (&d.source, &d.value))
                    .collect::<Vec<_>>(),
                full.directives
                    .iter()
                    .map(|d| (&d.source, &d.value))
                    .collect::<Vec<_>>(),
            );
        }
    }

    #[test]
    fn delayed_event_does_not_apply_offsets_to_a_newer_snapshot() {
        let mut buffer = DocumentBuffer::from_utf8(Vec::new()).unwrap();
        let mut config = OrgDisplayConfig::new(&buffer.snapshot());
        let delta = buffer
            .commit(EditTransaction::new(
                buffer.revision(),
                vec![TextEdit::new(
                    ByteRange::new(0, 0),
                    "#+STARTUP: literallinks\n",
                )],
            ))
            .unwrap();
        let snapshot = buffer.snapshot();
        buffer
            .commit(EditTransaction::new(
                buffer.revision(),
                vec![TextEdit::new(ByteRange::new(0, snapshot.len_bytes()), "")],
            ))
            .unwrap();
        config.apply_delta(&buffer.snapshot(), &delta);
        assert!(config.directives.is_empty());
        assert_eq!(config.revision, delta.before);
    }

    fn table_text(source: &str, caret: Option<usize>) -> TextProjection {
        project_inline(
            source,
            OrgDisplayOptions {
                descriptive_links: false,
                ..Default::default()
            },
            InlineDisplayContext {
                table: true,
                caret,
                ..Default::default()
            },
        )
    }

    #[test]
    fn table_entities_preserve_literal_markup_and_source_boundaries() {
        let source = r"中文 a\vert{}b \nbsp{}padded\nbsp{} \ast{}粗体\ast{} \under{}下划线\under{} \equal{}原样\equal{} \tilde{}代码\tilde{} C:\\zwnj{}Users\\zwnj{}demo [\zwnj{}[file:demo][链接形态]] {\zwnj{}{{macro}}}";
        let visible = table_text(source, None);
        assert_eq!(
            visible.text.replace('\u{200c}', "").replace('\u{a0}', " "),
            r"中文 a|b  padded  *粗体* _下划线_ =原样= ~代码~ C:\Users\demo [[file:demo][链接形态]] {{{macro}}}"
        );
        for offset in source
            .char_indices()
            .map(|(offset, _)| offset)
            .chain([source.len()])
        {
            if parse(source)
                .spans
                .iter()
                .filter(|span| span.kind == InlineKind::Entity)
                .any(|span| span.source.start < offset && offset < span.source.end)
            {
                continue;
            }
            let display = visible.source_to_display(offset);
            assert!(visible.text.is_char_boundary(display));
            assert_eq!(visible.display_to_source(display), offset);
        }
    }

    #[test]
    fn table_entities_respect_verbatim_unknown_names_and_editing_at_point() {
        let source = r"=\alpha{}= \unknown{} a\vert{}b \nbsp{}end";
        let caret = source.find("vert").unwrap() + 2;
        let visible = table_text(source, Some(caret));
        assert_eq!(visible.text, "\\alpha{} \\unknown{} a\\vert{}b \u{a0}end");
        assert_eq!(
            visible.display_to_source(visible.source_to_display(caret)),
            caret
        );
        let visible = table_text(source, None);
        assert_eq!(visible.text, "\\alpha{} \\unknown{} a|b \u{a0}end");
    }

    #[test]
    fn complete_links_hide_and_incomplete_links_remain_editable() {
        for (source, expected) in [
            ("[[https://example.org/guide][Guide]]", "Guide"),
            ("[[file:notes.org]]", "file:notes.org"),
            (
                "[[https://example.org/guide][Guide]",
                "[[https://example.org/guide][Guide]",
            ),
            ("[[][Empty]]", "[[][Empty]]"),
            ("<https://example.org>", "<https://example.org>"),
        ] {
            let text = project_inline(source, OrgDisplayOptions::default(), Default::default());
            assert_eq!(text.text, expected, "{source}");
        }
    }

    #[test]
    fn literal_text_and_product_table_entities_are_preserved() {
        let source = "=literal [[file:notes.org]]= \\alpha{}";
        let text = project_inline(source, OrgDisplayOptions::default(), Default::default());
        assert_eq!(text.text, "literal [[file:notes.org]] \\alpha{}");
        let text = project_inline(
            source,
            OrgDisplayOptions::default(),
            InlineDisplayContext {
                table: true,
                ..Default::default()
            },
        );
        assert_eq!(text.text, "literal [[file:notes.org]] α");
    }

    #[test]
    fn source_reveal_and_literal_links_keep_complete_source() {
        let source = "[[https://example.org][Guide]]";
        let text = project_inline(
            source,
            OrgDisplayOptions::default(),
            InlineDisplayContext {
                reveal: std::slice::from_ref(&(2..10)),
                ..Default::default()
            },
        );
        assert_eq!(text.text, source);
        let mut options = OrgDisplayOptions::default();
        options.apply_startup("LITERALLINKS entitiesplain hidestars");
        assert!(!options.descriptive_links);
        assert_eq!(options.heading_style, OrgHeadingStyle::HiddenStars);
        assert_eq!(options.pretty_entities, Some(false));
        assert_eq!(
            project_inline(source, options, Default::default()).text,
            source
        );
    }

    #[test]
    fn composing_or_searching_visible_labels_keeps_hidden_targets_hidden() {
        let source = "[[https://example.org][Guide]]";
        let label = source.find("Guide").unwrap();
        let text = project_inline(
            source,
            OrgDisplayOptions::default(),
            InlineDisplayContext {
                reveal: std::slice::from_ref(&(label..label + "Guide".len())),
                ..Default::default()
            },
        );
        assert_eq!(text.text, "Guide");
    }
}
