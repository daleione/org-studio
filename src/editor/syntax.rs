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
        let (&start, checkpoint_context) = cache
            .contexts
            .range(..=first_line)
            .next_back()
            .expect("line zero syntax checkpoint exists");
        let mut context = checkpoint_context.clone();
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
                    cache.contexts.insert(line, context.clone());
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
            cache.contexts.insert(first_line, context.clone());
        }
        context
    }
}

fn language(path: &Path) -> Language {
    match crate::document::DocumentFormat::from_path(path) {
        crate::document::DocumentFormat::Markdown => Language::Markdown,
        crate::document::DocumentFormat::Org => Language::Org,
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
    pub(super) code_language: Option<Arc<str>>,
    todo: Option<TodoSpan>,
    pub(super) metrics: BlockMetrics,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(super) struct EditorStyleSnapshot {
    pub(super) revision: Revision,
    pub(super) lines: Arc<[EditorLineStyle]>,
}

#[derive(Clone, Debug)]
pub(super) struct SparseEditorStyleSnapshot {
    pub(super) revision: Revision,
    lines: BTreeMap<u64, EditorLineStyle>,
}

#[cfg(test)]
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
                let todo = heading_todo_span(language, &text, &code);
                let id = classify_line(language, &text, &mut code);
                Some(EditorLineStyle {
                    source_range,
                    id,
                    code_language: (id == EditorStyleId::Code)
                        .then(|| code.code_language.clone())
                        .flatten(),
                    todo,
                    metrics: metrics_for(id),
                })
            })
            .collect::<Vec<_>>();
        Self {
            revision: snapshot.revision(),
            lines: styles.into(),
        }
    }
}

impl SparseEditorStyleSnapshot {
    pub(super) fn for_lines(
        path: &Path,
        snapshot: &DocumentSnapshot,
        lines: &[u64],
        cache: &EditorSyntaxCache,
    ) -> Self {
        let language = language(path);
        let lines = lines
            .iter()
            .filter_map(|&line| {
                let source_range = snapshot.line_content_range(LineIndex(line)).ok()?;
                let text = classification_text(snapshot, source_range);
                let mut code = cache.context_at(snapshot, language, line);
                let todo = heading_todo_span(language, &text, &code);
                let id = classify_line(language, &text, &mut code);
                Some((
                    line,
                    EditorLineStyle {
                        source_range,
                        id,
                        code_language: (id == EditorStyleId::Code)
                            .then(|| code.code_language.clone())
                            .flatten(),
                        todo,
                        metrics: metrics_for(id),
                    },
                ))
            })
            .collect();
        Self {
            revision: snapshot.revision(),
            lines,
        }
    }

    pub(super) fn line(&self, line: u64) -> Option<&EditorLineStyle> {
        self.lines.get(&line)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TodoFace {
    Open,
    Active,
    Project,
    Waiting,
    Done,
}

impl TodoFace {
    fn color(self, theme: &Theme) -> u32 {
        match self {
            Self::Open => theme.todo,
            Self::Active => theme.todo_active,
            Self::Project => theme.todo_project,
            Self::Waiting => theme.waiting,
            Self::Done => theme.done,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TodoSpan {
    range: Range<usize>,
    face: TodoFace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CodeContext {
    in_block: bool,
    org_end_marker: Option<Arc<str>>,
    markdown_fence: Option<u8>,
    code_language: Option<Arc<str>>,
    todo_faces: BTreeMap<Arc<str>, TodoFace>,
    has_custom_todo_faces: bool,
}

impl Default for CodeContext {
    fn default() -> Self {
        Self {
            in_block: false,
            org_end_marker: None,
            markdown_fence: None,
            code_language: None,
            todo_faces: BTreeMap::from([
                (Arc::from("TODO"), TodoFace::Open),
                (Arc::from("PROJ"), TodoFace::Project),
                (Arc::from("STRT"), TodoFace::Active),
                (Arc::from("WAIT"), TodoFace::Waiting),
                (Arc::from("HOLD"), TodoFace::Waiting),
                (Arc::from("DONE"), TodoFace::Done),
                (Arc::from("KILL"), TodoFace::Done),
            ]),
            has_custom_todo_faces: false,
        }
    }
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
    if language == Language::Org {
        update_org_todo_faces(trimmed, code);
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
            } else if is_org_property_line(trimmed) {
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
            if let Some(end_marker) = code.org_end_marker.as_deref() {
                if text.trim().eq_ignore_ascii_case(end_marker) {
                    code.in_block = false;
                    code.org_end_marker = None;
                    code.code_language = None;
                    return true;
                }
                return false;
            }
            if let Some((name, language)) = org_block_start(text) {
                code.in_block = true;
                code.org_end_marker = Some(Arc::from(format!("#+end_{name}")));
                code.code_language = language;
                true
            } else {
                starts_with_ascii_case_insensitive(text, "#+end_")
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
            if code.in_block {
                if code.markdown_fence != Some(marker) {
                    return false;
                }
                code.in_block = false;
                code.markdown_fence = None;
                code.code_language = None;
            } else {
                code.in_block = true;
                code.markdown_fence = Some(marker);
                code.code_language = markdown_fence_language(text, marker);
            }
            true
        }
    }
}

fn org_block_start(text: &str) -> Option<(String, Option<Arc<str>>)> {
    let rest = text
        .get("#+begin_".len()..)
        .filter(|_| starts_with_ascii_case_insensitive(text, "#+begin_"))?;
    let name = rest
        .split_once(char::is_whitespace)
        .map_or(rest, |(name, _)| name);
    if name.is_empty() {
        return None;
    }
    let name = name.to_ascii_lowercase();
    let language = (name == "src")
        .then(|| text.split_whitespace().nth(1).map(Arc::from))
        .flatten();
    Some((name, language))
}

fn markdown_fence_language(text: &str, marker: u8) -> Option<Arc<str>> {
    let fence_end = text.bytes().take_while(|byte| *byte == marker).count();
    text.get(fence_end..)?
        .split_whitespace()
        .next()
        .filter(|language| !language.is_empty())
        .map(Arc::from)
}

fn starts_with_ascii_case_insensitive(text: &str, prefix: &str) -> bool {
    text.as_bytes()
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix.as_bytes()))
}

fn update_code_context(language: Language, text: &str, code: &mut CodeContext) {
    let trimmed = text.trim_start();
    let was_in_block = code.in_block;
    let boundary = code_boundary(language, trimmed, code);
    if language == Language::Org && !was_in_block && !boundary && !code.in_block {
        update_org_todo_faces(trimmed, code);
    }
}

fn update_org_todo_faces(text: &str, context: &mut CodeContext) {
    let Some((key, value)) = text.split_once(':') else {
        return;
    };
    if !["#+TODO", "#+SEQ_TODO", "#+TYP_TODO"]
        .iter()
        .any(|candidate| key.eq_ignore_ascii_case(candidate))
    {
        return;
    }

    if !context.has_custom_todo_faces {
        context.todo_faces.clear();
        context.has_custom_todo_faces = true;
    }

    let tokens = value.split_whitespace().collect::<Vec<_>>();
    let separator = tokens.iter().position(|token| *token == "|");
    let last_keyword = tokens.iter().rposition(|token| *token != "|");
    for (index, token) in tokens.into_iter().enumerate() {
        if token == "|" {
            continue;
        }
        let keyword = token.split_once('(').map_or(token, |(keyword, _)| keyword);
        if keyword.is_empty() {
            continue;
        }
        let default_face = if separator.map_or(Some(index) == last_keyword, |pipe| index > pipe) {
            TodoFace::Done
        } else {
            TodoFace::Open
        };
        let face = match keyword {
            "PROJ" => TodoFace::Project,
            "STRT" => TodoFace::Active,
            "WAIT" | "HOLD" => TodoFace::Waiting,
            _ => default_face,
        };
        context.todo_faces.insert(Arc::from(keyword), face);
    }
}

fn heading_todo_span(language: Language, text: &str, context: &CodeContext) -> Option<TodoSpan> {
    if language != Language::Org || context.in_block {
        return None;
    }
    let indent = text.len() - text.trim_start().len();
    let trimmed = &text[indent..];
    let stars = trimmed.bytes().take_while(|byte| *byte == b'*').count();
    if stars == 0 || trimmed.as_bytes().get(stars) != Some(&b' ') {
        return None;
    }
    let keyword_start = indent
        + stars
        + trimmed[stars..]
            .bytes()
            .take_while(u8::is_ascii_whitespace)
            .count();
    let keyword_end = text[keyword_start..]
        .find(char::is_whitespace)
        .map_or(text.len(), |end| keyword_start + end);
    let keyword = text.get(keyword_start..keyword_end)?;
    let face = *context.todo_faces.get(keyword)?;
    Some(TodoSpan {
        range: keyword_start..keyword_end,
        face,
    })
}

fn metrics_for(_style: EditorStyleId) -> BlockMetrics {
    BlockMetrics::default()
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

fn is_org_property_line(text: &str) -> bool {
    let Some(rest) = text.strip_prefix(':') else {
        return false;
    };
    rest.find(':').is_some_and(|end| {
        end > 0
            && rest[..end].chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
            })
    })
}

#[derive(Clone, Copy, Debug, Default)]
struct SpanStyle {
    color: Option<u32>,
    weight: Option<FontWeight>,
    font_style: Option<FontStyle>,
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
            base.font.weight = FontWeight::BOLD;
            base.color = rgb(theme.heading[(level.saturating_sub(1) as usize).min(3)]).into();
        }
        EditorStyleId::CodeBoundary => {
            base.color = rgb(theme.code_boundary).into();
            base.font.style = FontStyle::Italic;
        }
        EditorStyleId::Code => {
            base.color = rgb(theme.code_foreground).into();
        }
        EditorStyleId::Quote => base.color = rgb(theme.quote).into(),
        EditorStyleId::Property | EditorStyleId::Meta => {}
        EditorStyleId::Comment => {
            base.color = rgb(theme.comment).into();
            base.font.style = FontStyle::Italic;
        }
        EditorStyleId::Table => base.color = rgb(theme.link).into(),
        EditorStyleId::List | EditorStyleId::Plain => {}
    }

    let mut spans = Vec::<(Range<usize>, SpanStyle)>::new();
    let verbatim = matches!(
        line_style.id,
        EditorStyleId::Code | EditorStyleId::CodeBoundary
    );
    if !verbatim {
        collect_common_semantics(text, theme, &mut spans);
        if let Some(todo) = &line_style.todo {
            spans.push((
                todo.range.clone(),
                SpanStyle {
                    color: Some(todo.face.color(theme)),
                    weight: Some(FontWeight::BOLD),
                    ..SpanStyle::default()
                },
            ));
        }
        match line_style.id {
            EditorStyleId::Property => collect_org_property_key(text, theme.attribute, &mut spans),
            EditorStyleId::Meta => collect_org_meta_key(text, theme.meta, &mut spans),
            EditorStyleId::List => collect_list_marker(text, theme.inline_code, &mut spans),
            _ => {}
        }
    }
    if line_style.id == EditorStyleId::Code
        && let Some(language) = line_style.code_language.as_deref()
        && let Ok(code_spans) = crate::preview::highlight_code(language, text)
    {
        spans.extend(
            code_spans
                .into_iter()
                .filter(|span| {
                    span.start < span.end
                        && span.end <= text.len()
                        && text.is_char_boundary(span.start)
                        && text.is_char_boundary(span.end)
                })
                .map(|span| {
                    (
                        span.start..span.end,
                        code_span_style(span.kind, &text[span.start..span.end], theme),
                    )
                }),
        );
        if matches!(
            language.trim().to_ascii_lowercase().as_str(),
            "cpp" | "c++" | "cc" | "cxx" | "hpp"
        ) {
            collect_cpp_namespace_qualifiers(text, theme.variable, theme.foreground, &mut spans);
        }
    }
    match language(path) {
        Language::Org if !verbatim => {
            collect_org_links(text, theme.link, &mut spans);
            collect_org_timestamps(text, theme.date, &mut spans);
            for keyword in ["SCHEDULED:", "DEADLINE:"] {
                collect_token(text, keyword, theme.meta, &mut spans);
            }
            collect_token(text, "CLOSED:", theme.done, &mut spans);
            for priority in ["[#A]", "[#B]", "[#C]"] {
                collect_token(text, priority, theme.keyword, &mut spans);
            }
            collect_org_tags(text, theme.attribute, &mut spans);
        }
        Language::Markdown if !verbatim => {
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
        Language::Org | Language::Markdown => {}
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

fn code_span_style(
    kind: crate::preview::CodeHighlightKind,
    source: &str,
    theme: &Theme,
) -> SpanStyle {
    use crate::preview::CodeHighlightKind;

    let color = match kind {
        CodeHighlightKind::Keyword if source.starts_with('#') => theme.comment,
        CodeHighlightKind::Attribute => theme.attribute,
        CodeHighlightKind::Boolean | CodeHighlightKind::Constant => theme.constant,
        CodeHighlightKind::Comment => theme.comment,
        CodeHighlightKind::Function => theme.function,
        CodeHighlightKind::Keyword => theme.keyword,
        CodeHighlightKind::Number => theme.number,
        CodeHighlightKind::Operator | CodeHighlightKind::Punctuation => theme.operator,
        CodeHighlightKind::Property => theme.foreground,
        CodeHighlightKind::Variable => theme.variable,
        CodeHighlightKind::String => theme.string,
        CodeHighlightKind::Type => theme.type_name,
    };
    SpanStyle {
        color: Some(color),
        font_style: matches!(kind, CodeHighlightKind::Comment).then_some(FontStyle::Italic),
        ..SpanStyle::default()
    }
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
                ..SpanStyle::default()
            },
        ),
        (
            "=",
            SpanStyle {
                color: Some(theme.verbatim),
                ..SpanStyle::default()
            },
        ),
        (
            "`",
            SpanStyle {
                color: Some(theme.inline_code),
                ..SpanStyle::default()
            },
        ),
    ] {
        collect_delimited(text, marker, marker, style, spans);
    }
    collect_token(text, "[ ]", theme.todo, spans);
    collect_token(text, "[-]", theme.todo_active, spans);
    collect_token(text, "[?]", theme.waiting, spans);
    for checkbox in ["[X]", "[x]"] {
        collect_token(text, checkbox, theme.done, spans);
    }
}

fn collect_cpp_namespace_qualifiers(
    text: &str,
    qualifier_color: u32,
    member_color: u32,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
    let mut cursor = 0;
    while let Some(separator) = text[cursor..].find("::").map(|index| cursor + index) {
        let start = text[..separator]
            .rfind(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .map_or(0, |index| index + 1);
        if start < separator {
            spans.push((
                start..separator,
                SpanStyle {
                    color: Some(qualifier_color),
                    ..SpanStyle::default()
                },
            ));
        }
        let member_start = separator + 2;
        let member_end = text[member_start..]
            .find(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .map_or(text.len(), |index| member_start + index);
        if member_start < member_end {
            spans.push((
                member_start..member_end,
                SpanStyle {
                    color: Some(member_color),
                    ..SpanStyle::default()
                },
            ));
        }
        cursor = member_end.max(member_start);
    }
}

fn collect_org_links(text: &str, color: u32, spans: &mut Vec<(Range<usize>, SpanStyle)>) {
    let mut cursor = 0;
    while let Some(start) = text[cursor..].find("[[").map(|index| cursor + index) {
        let body = start + 2;
        let Some(end) = text[body..].find("]]").map(|index| body + index) else {
            break;
        };
        let close = end + 2;
        spans.push((
            start..close,
            SpanStyle {
                color: Some(color),
                ..SpanStyle::default()
            },
        ));

        // The official illustration keeps the brackets regular and bolds the
        // target/description text inside them.
        if let Some(separator) = text[body..end].find("][").map(|index| body + index) {
            if body < separator {
                spans.push((
                    body..separator,
                    SpanStyle {
                        weight: Some(FontWeight::BOLD),
                        ..SpanStyle::default()
                    },
                ));
            }
            let description = separator + 2;
            if description < end {
                spans.push((
                    description..end,
                    SpanStyle {
                        weight: Some(FontWeight::BOLD),
                        ..SpanStyle::default()
                    },
                ));
            }
        } else if body < end {
            spans.push((
                body..end,
                SpanStyle {
                    weight: Some(FontWeight::BOLD),
                    ..SpanStyle::default()
                },
            ));
        }
        cursor = close;
    }
}

fn collect_org_timestamps(text: &str, color: u32, spans: &mut Vec<(Range<usize>, SpanStyle)>) {
    for (open, close) in [(b'<', b'>'), (b'[', b']')] {
        let mut cursor = 0;
        while let Some(start) = text[cursor..]
            .bytes()
            .position(|byte| byte == open)
            .map(|index| cursor + index)
        {
            let body = start + 1;
            let Some(end) = text[body..]
                .bytes()
                .position(|byte| byte == close)
                .map(|index| body + index)
            else {
                break;
            };
            let candidate = &text[body..end];
            if candidate.len() >= 10
                && candidate.as_bytes()[..4].iter().all(u8::is_ascii_digit)
                && candidate.as_bytes().get(4) == Some(&b'-')
            {
                spans.push((
                    start..end + 1,
                    SpanStyle {
                        color: Some(color),
                        ..SpanStyle::default()
                    },
                ));
            }
            cursor = end + 1;
        }
    }
}

fn collect_org_property_key(text: &str, color: u32, spans: &mut Vec<(Range<usize>, SpanStyle)>) {
    let indent = text.len() - text.trim_start().len();
    let trimmed = &text[indent..];
    let Some(rest) = trimmed.strip_prefix(':') else {
        return;
    };
    let Some(end) = rest.find(':') else {
        return;
    };
    spans.push((
        indent..indent + end + 2,
        SpanStyle {
            color: Some(color),
            weight: Some(FontWeight::SEMIBOLD),
            ..SpanStyle::default()
        },
    ));
}

fn collect_org_meta_key(text: &str, color: u32, spans: &mut Vec<(Range<usize>, SpanStyle)>) {
    let indent = text.len() - text.trim_start().len();
    let trimmed = &text[indent..];
    let key_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    if trimmed.starts_with("#+") && key_end > 2 {
        spans.push((
            indent..indent + key_end,
            SpanStyle {
                color: Some(color),
                weight: Some(FontWeight::SEMIBOLD),
                ..SpanStyle::default()
            },
        ));
    }
}

fn collect_list_marker(text: &str, color: u32, spans: &mut Vec<(Range<usize>, SpanStyle)>) {
    let indent = text.len() - text.trim_start().len();
    let trimmed = &text[indent..];
    let marker_len = if trimmed.starts_with("- ") || trimmed.starts_with("+ ") {
        1
    } else {
        trimmed
            .find(['.', ')'])
            .filter(|end| trimmed[..*end].bytes().all(|byte| byte.is_ascii_digit()))
            .map_or(0, |end| end + 1)
    };
    if marker_len > 0 {
        spans.push((
            indent..indent + marker_len,
            SpanStyle {
                color: Some(color),
                weight: Some(FontWeight::BOLD),
                ..SpanStyle::default()
            },
        ));
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
        && candidate[1..candidate.len() - 1].chars().all(|character| {
            character.is_alphanumeric() || matches!(character, ':' | '_' | '@' | '#')
        })
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

    fn run_at(runs: &[TextRun], offset: usize) -> &TextRun {
        let mut cursor = 0;
        runs.iter()
            .find(|run| {
                let contains = cursor <= offset && offset < cursor + run.len;
                cursor += run.len;
                contains
            })
            .expect("offset belongs to a paint run")
    }

    fn line_style(text: &str, context: &mut CodeContext) -> EditorLineStyle {
        let todo = heading_todo_span(Language::Org, text, context);
        let id = classify_line(Language::Org, text, context);
        EditorLineStyle {
            source_range: ByteRange::new(0, text.len() as u64),
            id,
            code_language: None,
            todo,
            metrics: metrics_for(id),
        }
    }

    #[test]
    fn decoration_runs_preserve_every_source_byte() {
        let text = "** TODO Heading *bold* [[target]] :tag:";
        let style = line_style(text, &mut CodeContext::default());
        let markup_runs = runs(
            Path::new("a.org"),
            text,
            base_run(text.len()),
            &style,
            None,
            current_theme(),
        );
        assert_eq!(
            markup_runs.iter().map(|run| run.len).sum::<usize>(),
            text.len()
        );
        assert_eq!(style.metrics, BlockMetrics::default());
    }

    #[test]
    fn official_org_palette_is_applied_to_source_markup() {
        let theme = current_theme();
        assert_eq!(theme.foreground, 0x373942);
        assert_eq!(theme.heading[0], 0xe45549);
        assert_eq!(theme.heading[1], 0xd98547);
        assert_eq!(theme.inline_code, 0xd98547);
        assert_eq!(theme.verbatim, 0x4fa14e);
        assert_eq!(theme.link, 0x3f78f2);
        assert_eq!(theme.todo, 0x50a14f);
        assert_eq!(theme.todo_active, 0xb751b6);
        assert_eq!(theme.todo_project, 0x84888b);
        assert_eq!(theme.waiting, 0x986801);
        assert_eq!(theme.done, 0x383a42);
        assert_eq!(theme.code_background, 0xe6e6e6);
        assert_eq!(theme.code_active_background, 0xf0f0f0);
        assert_eq!(theme.code_boundary_background, 0xc8c8c8);

        let text = "~code~ =verbatim= [[https://orgmode.org][Org]] <2026-09-02 Wed> :ui:mac:";
        let style = line_style(text, &mut CodeContext::default());
        let markup_runs = runs(
            Path::new("a.org"),
            text,
            base_run(text.len()),
            &style,
            None,
            theme,
        );

        let color = |offset| run_at(&markup_runs, offset).color;
        assert_eq!(
            color(text.find("code").unwrap()),
            rgb(theme.inline_code).into()
        );
        assert_eq!(
            color(text.find("verbatim").unwrap()),
            rgb(theme.verbatim).into()
        );
        let link = text.find("[[").unwrap();
        assert_eq!(color(link), rgb(theme.link).into());
        assert_eq!(run_at(&markup_runs, link).font.weight, FontWeight::NORMAL);
        assert_eq!(
            run_at(&markup_runs, text.find("https").unwrap())
                .font
                .weight,
            FontWeight::BOLD
        );
        assert_eq!(color(text.find("<2026").unwrap()), rgb(theme.date).into());
        assert_eq!(
            color(text.find(":ui:mac:").unwrap()),
            rgb(theme.attribute).into()
        );
        assert!(markup_runs.iter().all(|run| run.background_color.is_none()));

        for (keyword, expected) in [
            ("TODO", theme.todo),
            ("PROJ", theme.todo_project),
            ("STRT", theme.todo_active),
            ("WAIT", theme.waiting),
            ("HOLD", theme.waiting),
            ("DONE", theme.done),
            ("KILL", theme.done),
        ] {
            let heading = format!("** {keyword} Task");
            let style = line_style(&heading, &mut CodeContext::default());
            let runs = runs(
                Path::new("a.org"),
                &heading,
                base_run(heading.len()),
                &style,
                None,
                theme,
            );
            assert_eq!(
                run_at(&runs, heading.find(keyword).unwrap()).color,
                rgb(expected).into()
            );
        }
    }

    #[test]
    fn official_headings_keep_source_markers_at_body_size() {
        let theme = current_theme();
        for (text, level) in [("* Headline", 1), ("** Sub-headline :tag:", 2)] {
            let style = line_style(text, &mut CodeContext::default());
            let runs = runs(
                Path::new("a.org"),
                text,
                base_run(text.len()),
                &style,
                None,
                theme,
            );
            let expected: gpui::Hsla = rgb(theme.heading[level - 1]).into();
            assert_eq!(run_at(&runs, 0).color, expected);
            let title = text
                .find(|character: char| character.is_ascii_alphabetic())
                .unwrap();
            assert_eq!(run_at(&runs, title).color, expected);
            assert_eq!(run_at(&runs, 0).font.weight, FontWeight::BOLD);
            assert_eq!(style.metrics, BlockMetrics::default());
        }
    }

    #[test]
    fn custom_todo_sequences_follow_the_org_separator_and_official_overrides() {
        let source = "#+TODO: PLAN(p) BUILD(b) WAIT(w@/!) | SHIPPED(s!)\n\
                      #+SEQ_TODO: OPEN(o) CLOSED(c)\n\
                      * PLAN Plan\n\
                      * BUILD Build\n\
                      * WAIT Wait\n\
                      * SHIPPED Ship\n\
                      * OPEN Open\n\
                      * CLOSED Close\n\
                      * TODO Plain title\n";
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let styles = EditorStyleSnapshot::for_lines(
            Path::new("a.org"),
            &snapshot,
            2..9,
            &EditorSyntaxCache::default(),
        );
        let theme = current_theme();
        for (index, keyword, expected) in [
            (0, "PLAN", theme.todo),
            (1, "BUILD", theme.todo),
            (2, "WAIT", theme.waiting),
            (3, "SHIPPED", theme.done),
            (4, "OPEN", theme.todo),
            (5, "CLOSED", theme.done),
        ] {
            let text = snapshot.copy_range(styles.lines[index].source_range);
            let runs = runs(
                Path::new("a.org"),
                &text,
                base_run(text.len()),
                &styles.lines[index],
                None,
                theme,
            );
            assert_eq!(
                run_at(&runs, text.find(keyword).unwrap()).color,
                rgb(expected).into(),
                "wrong face for {keyword}"
            );
        }

        let plain = snapshot.copy_range(styles.lines[6].source_range);
        let runs = runs(
            Path::new("a.org"),
            &plain,
            base_run(plain.len()),
            &styles.lines[6],
            None,
            theme,
        );
        assert_eq!(
            run_at(&runs, plain.find("TODO").unwrap()).color,
            rgb(theme.heading[0]).into()
        );
    }

    #[test]
    fn official_checkbox_faces_cover_open_active_waiting_and_done_states() {
        let text = "- [ ] [-] [?] [X] [#A]";
        let style = line_style(text, &mut CodeContext::default());
        let theme = current_theme();
        let runs = runs(
            Path::new("a.org"),
            text,
            base_run(text.len()),
            &style,
            None,
            theme,
        );
        for (token, expected) in [
            ("[ ]", theme.todo),
            ("[-]", theme.todo_active),
            ("[?]", theme.waiting),
            ("[X]", theme.done),
            ("[#A]", theme.keyword),
        ] {
            assert_eq!(
                run_at(&runs, text.find(token).unwrap()).color,
                rgb(expected).into(),
                "wrong face for {token}"
            );
        }
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
        assert_eq!(styles.lines[1].code_language.as_deref(), Some("rust"));
        assert_eq!(styles.lines[2].id, EditorStyleId::CodeBoundary);
        assert_eq!(styles.lines[3].id, EditorStyleId::Plain);
        let body_only = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 1..2, &cache);
        assert_eq!(body_only.lines[0].id, EditorStyleId::Code);
    }

    #[test]
    fn example_block_keeps_nested_org_blocks_verbatim() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"#+begin_example\n#+name: hello-rust\n#+begin_src rust :results output\nfn main() {\n    println!(\"hello\");\n}\n#+end_src\n#+end_example\nafter\n".to_vec(),
        )
        .unwrap();
        let cache = EditorSyntaxCache::default();
        let styles = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 0..9, &cache);

        assert_eq!(styles.lines[0].id, EditorStyleId::CodeBoundary);
        assert!(
            styles.lines[1..7]
                .iter()
                .all(|line| line.id == EditorStyleId::Code)
        );
        assert_eq!(styles.lines[7].id, EditorStyleId::CodeBoundary);
        assert_eq!(styles.lines[8].id, EditorStyleId::Plain);

        let tail = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 6..9, &cache);
        assert_eq!(tail.lines[0].id, EditorStyleId::Code);
        assert_eq!(tail.lines[1].id, EditorStyleId::CodeBoundary);
        assert_eq!(tail.lines[2].id, EditorStyleId::Plain);
    }

    #[test]
    fn org_code_content_does_not_receive_org_inline_semantics() {
        let text = "*bold* [[target]] TODO :tag:";
        let style = EditorLineStyle {
            source_range: ByteRange::new(0, text.len() as u64),
            id: EditorStyleId::Code,
            code_language: None,
            todo: None,
            metrics: metrics_for(EditorStyleId::Code),
        };
        let runs = runs(
            Path::new("a.org"),
            text,
            base_run(text.len()),
            &style,
            None,
            current_theme(),
        );

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len, text.len());
        assert!(runs[0].underline.is_none());
        assert!(runs[0].strikethrough.is_none());
    }

    #[test]
    fn editor_code_runs_use_the_declared_source_language() {
        let snapshot =
            DocumentSnapshot::from_utf8(b"#+begin_src rust\nfn main() {}\n#+end_src\n".to_vec())
                .unwrap();
        let styles = EditorStyleSnapshot::for_lines(
            Path::new("a.org"),
            &snapshot,
            0..3,
            &EditorSyntaxCache::default(),
        );
        let text = snapshot.copy_range(styles.lines[1].source_range);
        let theme = current_theme();
        let runs = runs(
            Path::new("a.org"),
            &text,
            base_run(text.len()),
            &styles.lines[1],
            None,
            theme,
        );
        let keyword_color: gpui::Hsla = rgb(theme.keyword).into();
        let function_color: gpui::Hsla = rgb(theme.function).into();

        assert_eq!(styles.lines[1].code_language.as_deref(), Some("rust"));
        assert!(runs.iter().any(|run| run.color == keyword_color));
        assert!(runs.iter().any(|run| run.color == function_color));
    }

    #[test]
    fn cpp_source_uses_the_official_feature_illustration_faces() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"#+begin_src cpp\n#include <iostream>\nint main() { std::cout << \"You\"; return 0; }\n#+end_src\n".to_vec(),
        )
        .unwrap();
        let styles = EditorStyleSnapshot::for_lines(
            Path::new("a.org"),
            &snapshot,
            0..4,
            &EditorSyntaxCache::default(),
        );
        let theme = current_theme();

        let boundary_text = snapshot.copy_range(styles.lines[0].source_range);
        let boundary_runs = runs(
            Path::new("a.org"),
            &boundary_text,
            base_run(boundary_text.len()),
            &styles.lines[0],
            None,
            theme,
        );
        assert_eq!(boundary_runs[0].font.style, FontStyle::Italic);
        assert_eq!(boundary_runs[0].color, rgb(theme.code_boundary).into());

        let include_text = snapshot.copy_range(styles.lines[1].source_range);
        let include_runs = runs(
            Path::new("a.org"),
            &include_text,
            base_run(include_text.len()),
            &styles.lines[1],
            None,
            theme,
        );
        assert_eq!(
            run_at(&include_runs, include_text.find("#include").unwrap()).color,
            rgb(theme.comment).into()
        );
        assert_eq!(
            run_at(&include_runs, include_text.find("<iostream>").unwrap()).color,
            rgb(theme.string).into()
        );

        let body_text = snapshot.copy_range(styles.lines[2].source_range);
        let body_runs = runs(
            Path::new("a.org"),
            &body_text,
            base_run(body_text.len()),
            &styles.lines[2],
            None,
            theme,
        );
        for (token, color) in [
            ("int", theme.type_name),
            ("main", theme.function),
            ("std", theme.variable),
            ("cout", theme.foreground),
            ("\"You\"", theme.string),
            ("return", theme.keyword),
            ("0", theme.number),
        ] {
            assert_eq!(
                run_at(&body_runs, body_text.find(token).unwrap()).color,
                rgb(color).into(),
                "unexpected face for {token}"
            );
        }
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
