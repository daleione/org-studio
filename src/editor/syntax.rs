use std::{
    collections::BTreeMap,
    ops::Range,
    path::Path,
    sync::{Arc, Mutex},
};

use gpui::{FontStyle, FontWeight, TextRun, UnderlineStyle, px, rgb};

use crate::{
    document::{
        ByteOffset, ByteRange, DocumentId, DocumentSnapshot, LineCursor, LineIndex, Revision,
        TextSnapshot,
    },
    theme::Theme,
};

const MAX_CLASSIFICATION_BYTES: u64 = 4 * 1024;
const CONTEXT_CHECKPOINT_LINES: u64 = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Language {
    Org,
    Markdown,
}

pub(super) struct EditorSyntaxCache {
    inner: Mutex<SyntaxCacheInner>,
}

struct SyntaxCacheInner {
    document_id: Option<DocumentId>,
    revision: Revision,
    language: Language,
    contexts: BTreeMap<u64, CodeContext>,
}

impl Default for EditorSyntaxCache {
    fn default() -> Self {
        Self {
            inner: Mutex::new(SyntaxCacheInner {
                document_id: None,
                revision: Revision::INITIAL,
                language: Language::Org,
                contexts: BTreeMap::from([(0, CodeContext::default())]),
            }),
        }
    }
}

impl EditorSyntaxCache {
    pub(super) fn invalidate_from(
        &self,
        document_id: DocumentId,
        revision: Revision,
        first_line: u64,
    ) {
        let mut cache = self.inner.lock().expect("editor syntax cache poisoned");
        if cache.document_id != Some(document_id) {
            cache.contexts.clear();
            cache.contexts.insert(0, CodeContext::default());
        } else {
            let keep_through = first_line / CONTEXT_CHECKPOINT_LINES * CONTEXT_CHECKPOINT_LINES;
            cache.contexts.retain(|line, _| *line <= keep_through);
            cache.contexts.entry(0).or_default();
        }
        cache.document_id = Some(document_id);
        cache.revision = revision;
    }

    pub(super) fn reset(&self) {
        let mut cache = self.inner.lock().expect("editor syntax cache poisoned");
        cache.document_id = None;
        cache.contexts.clear();
        cache.contexts.insert(0, CodeContext::default());
    }

    fn context_at(
        &self,
        snapshot: &DocumentSnapshot,
        language: Language,
        first_line: u64,
    ) -> CodeContext {
        let mut cache = self.inner.lock().expect("editor syntax cache poisoned");
        if cache.document_id != Some(snapshot.document_id())
            || cache.revision != snapshot.revision()
            || cache.language != language
        {
            cache.document_id = Some(snapshot.document_id());
            cache.revision = snapshot.revision();
            cache.language = language;
            cache.contexts.clear();
            cache.contexts.insert(0, CodeContext::default());
        }
        let (&start, &mut_context) = cache
            .contexts
            .range(..=first_line)
            .next_back()
            .expect("line zero syntax checkpoint exists");
        let mut context = mut_context;
        if start < first_line
            && let (Ok(start_range), Ok(end_range)) = (
                snapshot.line_content_range(LineIndex(start)),
                snapshot.line_content_range(LineIndex(first_line)),
            )
            && let Some(mut cursor) = LineCursor::within(
                snapshot,
                ByteRange::new(start_range.start.0, end_range.start.0),
            )
        {
            let mut line = start;
            while let Some(source) = cursor.next_line() {
                if line > start && line.is_multiple_of(CONTEXT_CHECKPOINT_LINES) {
                    cache.contexts.insert(line, context);
                }
                update_code_context(
                    language,
                    classification_prefix(source.text.as_ref()),
                    &mut context,
                );
                line += 1;
            }
        }
        if first_line.is_multiple_of(CONTEXT_CHECKPOINT_LINES) {
            cache.contexts.insert(first_line, context);
        }
        context
    }
}

fn language(path: &Path) -> Language {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        })
    {
        Language::Markdown
    } else {
        Language::Org
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(super) enum EditorStyleId {
    #[default]
    Plain,
    Heading(u8),
    List,
    Table,
    Quote,
    CodeBoundary,
    Code,
    Property,
    Meta,
    Comment,
}

impl EditorStyleId {
    pub(super) const fn cache_key(self) -> u8 {
        match self {
            Self::Plain => 0,
            Self::Heading(level) => 10 + if level < 9 { level } else { 9 },
            Self::List => 20,
            Self::Table => 21,
            Self::Quote => 22,
            Self::CodeBoundary => 23,
            Self::Code => 24,
            Self::Property => 25,
            Self::Meta => 26,
            Self::Comment => 27,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BlockMetrics {
    pub(super) font_scale: f32,
    pub(super) line_height: f32,
    pub(super) before: f32,
    pub(super) after: f32,
}

impl Default for BlockMetrics {
    fn default() -> Self {
        Self {
            font_scale: 1.0,
            line_height: super::LINE_HEIGHT,
            before: 0.0,
            after: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct EditorLineStyle {
    pub(super) source_range: ByteRange,
    pub(super) id: EditorStyleId,
    pub(super) metrics: BlockMetrics,
}

#[derive(Clone, Debug)]
pub(super) struct EditorStyleSnapshot {
    pub(super) revision: Revision,
    pub(super) lines: Arc<[EditorLineStyle]>,
}

impl EditorStyleSnapshot {
    pub(super) fn for_lines(
        path: &Path,
        snapshot: &DocumentSnapshot,
        lines: Range<u64>,
        cache: &EditorSyntaxCache,
    ) -> Self {
        let language = language(path);
        let mut code = cache.context_at(snapshot, language, lines.start);
        let styles = lines
            .filter_map(|line| {
                let source_range = snapshot.line_content_range(LineIndex(line)).ok()?;
                let text = classification_text(snapshot, source_range);
                let id = classify_line(language, &text, &mut code);
                Some(EditorLineStyle {
                    source_range,
                    id,
                    metrics: metrics_for(id),
                })
            })
            .collect::<Vec<_>>();
        Self {
            revision: snapshot.revision(),
            lines: styles.into(),
        }
    }

    pub(super) fn line(&self, line: u64, first_line: u64) -> Option<&EditorLineStyle> {
        self.lines.get(line.saturating_sub(first_line) as usize)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CodeContext {
    in_block: bool,
    markdown_fence: Option<u8>,
}

fn classification_text(snapshot: &DocumentSnapshot, range: ByteRange) -> String {
    let mut end = (range.start.0 + MAX_CLASSIFICATION_BYTES).min(range.end.0);
    while end > range.start.0 && !snapshot.is_char_boundary(ByteOffset(end)) {
        end -= 1;
    }
    snapshot.copy_range(ByteRange::new(range.start.0, end))
}

fn classification_prefix(text: &str) -> &str {
    let mut end = text.len().min(MAX_CLASSIFICATION_BYTES as usize);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn classify_line(language: Language, text: &str, code: &mut CodeContext) -> EditorStyleId {
    let trimmed = text.trim_start();
    let boundary = code_boundary(language, trimmed, code);
    if boundary {
        return EditorStyleId::CodeBoundary;
    }
    if code.in_block {
        return EditorStyleId::Code;
    }
    match language {
        Language::Org => {
            let stars = trimmed.bytes().take_while(|byte| *byte == b'*').count();
            if stars > 0 && trimmed.as_bytes().get(stars) == Some(&b' ') {
                return EditorStyleId::Heading(stars.min(u8::MAX as usize) as u8);
            }
            if trimmed.starts_with("#+") {
                EditorStyleId::Meta
            } else if trimmed.starts_with('#') {
                EditorStyleId::Comment
            } else if trimmed.starts_with('|') {
                EditorStyleId::Table
            } else if trimmed.starts_with(':') && trimmed.ends_with(':') {
                EditorStyleId::Property
            } else if trimmed.starts_with('>') {
                EditorStyleId::Quote
            } else if is_list_line(trimmed) {
                EditorStyleId::List
            } else {
                EditorStyleId::Plain
            }
        }
        Language::Markdown => {
            let hashes = trimmed.bytes().take_while(|byte| *byte == b'#').count();
            if hashes > 0 && trimmed.as_bytes().get(hashes) == Some(&b' ') {
                EditorStyleId::Heading(hashes.min(u8::MAX as usize) as u8)
            } else if trimmed.starts_with('>') {
                EditorStyleId::Quote
            } else if trimmed.starts_with("<!--") {
                EditorStyleId::Comment
            } else if is_list_line(trimmed) {
                EditorStyleId::List
            } else {
                EditorStyleId::Plain
            }
        }
    }
}

fn code_boundary(language: Language, text: &str, code: &mut CodeContext) -> bool {
    match language {
        Language::Org => {
            if starts_with_ascii_case_insensitive(text, "#+begin_") {
                code.in_block = true;
                true
            } else if starts_with_ascii_case_insensitive(text, "#+end_") {
                code.in_block = false;
                true
            } else {
                false
            }
        }
        Language::Markdown => {
            let marker = if text.starts_with("```") {
                Some(b'`')
            } else if text.starts_with("~~~") {
                Some(b'~')
            } else {
                None
            };
            let Some(marker) = marker else {
                return false;
            };
            if code.in_block && code.markdown_fence == Some(marker) {
                code.in_block = false;
                code.markdown_fence = None;
            } else if !code.in_block {
                code.in_block = true;
                code.markdown_fence = Some(marker);
            }
            true
        }
    }
}

fn starts_with_ascii_case_insensitive(text: &str, prefix: &str) -> bool {
    text.as_bytes()
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix.as_bytes()))
}

fn update_code_context(language: Language, text: &str, code: &mut CodeContext) {
    code_boundary(language, text.trim_start(), code);
}

fn metrics_for(style: EditorStyleId) -> BlockMetrics {
    match style {
        EditorStyleId::Heading(1) => BlockMetrics {
            font_scale: 1.50,
            line_height: 34.0,
            before: 12.0,
            after: 5.0,
        },
        EditorStyleId::Heading(2) => BlockMetrics {
            font_scale: 1.34,
            line_height: 30.0,
            before: 10.0,
            after: 4.0,
        },
        EditorStyleId::Heading(3) => BlockMetrics {
            font_scale: 1.20,
            line_height: 27.0,
            before: 8.0,
            after: 3.0,
        },
        EditorStyleId::Heading(_) => BlockMetrics {
            font_scale: 1.08,
            line_height: 24.0,
            before: 5.0,
            after: 2.0,
        },
        EditorStyleId::CodeBoundary | EditorStyleId::Quote | EditorStyleId::Property => {
            BlockMetrics {
                before: 2.0,
                after: 2.0,
                ..BlockMetrics::default()
            }
        }
        _ => BlockMetrics::default(),
    }
}

fn is_list_line(text: &str) -> bool {
    text.starts_with("- ")
        || text.starts_with("+ ")
        || text.split_once(['.', ')']).is_some_and(|(prefix, tail)| {
            !prefix.is_empty()
                && prefix.bytes().all(|byte| byte.is_ascii_digit())
                && tail.starts_with(' ')
        })
}

#[derive(Clone, Copy, Debug, Default)]
struct SpanStyle {
    color: Option<u32>,
    weight: Option<FontWeight>,
    font_style: Option<FontStyle>,
    background: Option<u32>,
    underline: bool,
    strikethrough: bool,
}

/// Produces semantic paint runs without hiding or replacing any source byte.
pub(super) fn runs(
    path: &Path,
    text: &str,
    mut base: TextRun,
    line_style: &EditorLineStyle,
    marked: Option<Range<usize>>,
    theme: &Theme,
) -> Vec<TextRun> {
    match line_style.id {
        EditorStyleId::Heading(level) => {
            base.font.weight = if level <= 2 {
                FontWeight::BOLD
            } else {
                FontWeight::SEMIBOLD
            };
            base.color = rgb(theme.heading[(level.saturating_sub(1) as usize).min(3)]).into();
        }
        EditorStyleId::CodeBoundary => {
            base.color = rgb(theme.code_boundary).into();
            base.background_color = Some(rgb(theme.code_boundary_background).into());
        }
        EditorStyleId::Code => {
            base.color = rgb(theme.code_foreground).into();
            base.background_color = Some(rgb(theme.code_background).into());
        }
        EditorStyleId::Quote => base.color = rgb(theme.quote).into(),
        EditorStyleId::Property => base.color = rgb(theme.attribute).into(),
        EditorStyleId::Meta => base.color = rgb(theme.meta).into(),
        EditorStyleId::Comment => base.color = rgb(theme.comment).into(),
        EditorStyleId::List | EditorStyleId::Table | EditorStyleId::Plain => {}
    }

    let mut spans = Vec::<(Range<usize>, SpanStyle)>::new();
    let trimmed = text.trim_start();
    let indent = text.len() - trimmed.len();
    if let EditorStyleId::Heading(level) = line_style.id {
        let markers = match language(path) {
            Language::Org => trimmed.bytes().take_while(|byte| *byte == b'*').count(),
            Language::Markdown => trimmed.bytes().take_while(|byte| *byte == b'#').count(),
        };
        spans.push((
            indent..(indent + markers).min(text.len()),
            SpanStyle {
                color: Some(theme.foreground_dim),
                weight: Some(if level <= 2 {
                    FontWeight::BOLD
                } else {
                    FontWeight::SEMIBOLD
                }),
                ..SpanStyle::default()
            },
        ));
    }

    collect_common_semantics(text, theme, &mut spans);
    match language(path) {
        Language::Org => {
            collect_delimited(
                text,
                "[[",
                "]]",
                SpanStyle {
                    color: Some(theme.link),
                    underline: true,
                    ..SpanStyle::default()
                },
                &mut spans,
            );
            collect_delimited(
                text,
                "<",
                ">",
                SpanStyle {
                    color: Some(theme.date),
                    ..SpanStyle::default()
                },
                &mut spans,
            );
            for keyword in ["TODO", "DONE", "SCHEDULED:", "DEADLINE:", "CLOSED:"] {
                collect_token(text, keyword, theme.keyword, &mut spans);
            }
            collect_org_tags(text, theme.attribute, &mut spans);
        }
        Language::Markdown => {
            collect_delimited(
                text,
                "[",
                "]",
                SpanStyle {
                    color: Some(theme.link),
                    underline: true,
                    ..SpanStyle::default()
                },
                &mut spans,
            );
        }
    }

    let mut boundaries = vec![0, text.len()];
    for (range, _) in &spans {
        boundaries.extend([range.start, range.end]);
    }
    if let Some(marked) = &marked {
        boundaries.extend([marked.start, marked.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
        .windows(2)
        .filter_map(|boundary| {
            let range = boundary[0]..boundary[1];
            if range.is_empty() {
                return None;
            }
            let mut run = TextRun {
                len: range.len(),
                ..base.clone()
            };
            for (_, style) in spans
                .iter()
                .filter(|(span, _)| span.start <= range.start && range.start < span.end)
            {
                if let Some(color) = style.color {
                    run.color = rgb(color).into();
                }
                if let Some(weight) = style.weight {
                    run.font.weight = weight;
                }
                if let Some(font_style) = style.font_style {
                    run.font.style = font_style;
                }
                if let Some(background) = style.background {
                    run.background_color = Some(rgb(background).into());
                }
                if style.underline {
                    run.underline = Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    });
                }
                if style.strikethrough {
                    run.strikethrough = Some(gpui::StrikethroughStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                    });
                }
            }
            if marked
                .as_ref()
                .is_some_and(|marked| marked.start < range.end && range.start < marked.end)
            {
                run.underline = Some(UnderlineStyle {
                    color: Some(run.color),
                    thickness: px(1.0),
                    wavy: false,
                });
            }
            Some(run)
        })
        .collect()
}

fn collect_common_semantics(text: &str, theme: &Theme, spans: &mut Vec<(Range<usize>, SpanStyle)>) {
    for (marker, style) in [
        (
            "*",
            SpanStyle {
                weight: Some(FontWeight::BOLD),
                ..SpanStyle::default()
            },
        ),
        (
            "/",
            SpanStyle {
                font_style: Some(FontStyle::Italic),
                ..SpanStyle::default()
            },
        ),
        (
            "_",
            SpanStyle {
                underline: true,
                ..SpanStyle::default()
            },
        ),
        (
            "+",
            SpanStyle {
                strikethrough: true,
                ..SpanStyle::default()
            },
        ),
        (
            "~",
            SpanStyle {
                color: Some(theme.inline_code),
                background: Some(theme.inline_code_background),
                ..SpanStyle::default()
            },
        ),
        (
            "=",
            SpanStyle {
                color: Some(theme.inline_code),
                background: Some(theme.inline_code_background),
                ..SpanStyle::default()
            },
        ),
        (
            "`",
            SpanStyle {
                color: Some(theme.inline_code),
                background: Some(theme.inline_code_background),
                ..SpanStyle::default()
            },
        ),
    ] {
        collect_delimited(text, marker, marker, style, spans);
    }
    for checkbox in ["[ ]", "[X]", "[x]", "[-]"] {
        collect_token(text, checkbox, theme.keyword, spans);
    }
}

fn collect_delimited(
    text: &str,
    open: &str,
    close: &str,
    style: SpanStyle,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
    let mut cursor = 0;
    while let Some(start) = text[cursor..].find(open).map(|index| cursor + index) {
        let body = start + open.len();
        let Some(end) = text[body..]
            .find(close)
            .map(|index| body + index + close.len())
        else {
            break;
        };
        if end > body + close.len() {
            spans.push((start..end, style));
        }
        cursor = end.max(cursor + open.len());
    }
}

fn collect_token(text: &str, token: &str, color: u32, spans: &mut Vec<(Range<usize>, SpanStyle)>) {
    let mut cursor = 0;
    while let Some(start) = text[cursor..].find(token).map(|index| cursor + index) {
        let end = start + token.len();
        spans.push((
            start..end,
            SpanStyle {
                color: Some(color),
                weight: Some(FontWeight::SEMIBOLD),
                ..SpanStyle::default()
            },
        ));
        cursor = end;
    }
}

fn collect_org_tags(text: &str, color: u32, spans: &mut Vec<(Range<usize>, SpanStyle)>) {
    let trimmed_end = text.trim_end();
    let Some(start) = trimmed_end.rfind(' ') else {
        return;
    };
    let candidate = &trimmed_end[start + 1..];
    if candidate.len() >= 3
        && candidate.starts_with(':')
        && candidate.ends_with(':')
        && candidate[1..candidate.len() - 1]
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '_' | '@' | '#'))
    {
        spans.push((
            start + 1..trimmed_end.len(),
            SpanStyle {
                color: Some(color),
                ..SpanStyle::default()
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::DocumentSnapshot, theme::current_theme};

    fn base_run(len: usize) -> TextRun {
        TextRun {
            len,
            font: Default::default(),
            color: rgb(0).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    }

    #[test]
    fn decoration_runs_preserve_every_source_byte() {
        let text = "** TODO Heading *bold* [[target]] :tag:";
        let id = classify_line(Language::Org, text, &mut CodeContext::default());
        let style = EditorLineStyle {
            source_range: ByteRange::new(0, text.len() as u64),
            id,
            metrics: metrics_for(id),
        };
        let runs = runs(
            Path::new("a.org"),
            text,
            base_run(text.len()),
            &style,
            None,
            current_theme(),
        );
        assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), text.len());
        assert!(style.metrics.line_height > super::super::LINE_HEIGHT);
    }

    #[test]
    fn style_snapshot_is_viewport_bounded_and_revision_coherent() {
        let snapshot = DocumentSnapshot::from_utf8(b"* One\nbody\n** Two\n".to_vec()).unwrap();
        let styles = EditorStyleSnapshot::for_lines(
            Path::new("a.org"),
            &snapshot,
            1..3,
            &EditorSyntaxCache::default(),
        );
        assert_eq!(styles.revision, snapshot.revision());
        assert_eq!(styles.lines.len(), 2);
        assert_eq!(styles.lines[0].id, EditorStyleId::Plain);
        assert_eq!(styles.lines[1].id, EditorStyleId::Heading(2));
    }

    #[test]
    fn style_snapshot_marks_code_body_without_hiding_boundaries() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"#+begin_src rust\nlet value = 1;\n#+end_src\nafter\n".to_vec(),
        )
        .unwrap();
        let cache = EditorSyntaxCache::default();
        let styles = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 0..4, &cache);
        assert_eq!(styles.lines[0].id, EditorStyleId::CodeBoundary);
        assert_eq!(styles.lines[1].id, EditorStyleId::Code);
        assert_eq!(styles.lines[2].id, EditorStyleId::CodeBoundary);
        assert_eq!(styles.lines[3].id, EditorStyleId::Plain);
        let body_only = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 1..2, &cache);
        assert_eq!(body_only.lines[0].id, EditorStyleId::Code);
    }

    #[test]
    fn long_code_block_style_is_independent_of_viewport_start() {
        let mut source = String::from("#+begin_src rust\n");
        source.push_str(&"let value = 1;\n".repeat(700));
        source.push_str("#+end_src\nafter\n");
        let snapshot = DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
        let cache = EditorSyntaxCache::default();

        let middle =
            EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 400..402, &cache);
        let later = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 650..652, &cache);
        let middle_again =
            EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 400..402, &cache);

        assert!(
            middle
                .lines
                .iter()
                .all(|line| line.id == EditorStyleId::Code)
        );
        assert!(
            later
                .lines
                .iter()
                .all(|line| line.id == EditorStyleId::Code)
        );
        assert_eq!(middle.lines[0].id, middle_again.lines[0].id);
        assert!(
            cache
                .inner
                .lock()
                .expect("editor syntax cache poisoned")
                .contexts
                .len()
                >= 3
        );
    }
}
