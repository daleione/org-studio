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
    org_syntax::inline::{InlineKind, InlineText},
    theme::Theme,
};

const MAX_CLASSIFICATION_BYTES: u64 = 4 * 1024;
const CONTEXT_CHECKPOINT_LINES: u64 = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Language {
    Org,
    Markdown,
}

/// Revision-coherent Editor semantics shared by every Editor pane for a document.
///
/// The service owns syntax checkpoints only. Layout, folds, raster state, and
/// viewport state remain pane-local.
pub(crate) struct EditorSyntaxService {
    inner: Mutex<SyntaxCacheInner>,
}

struct SyntaxCacheInner {
    document_id: Option<DocumentId>,
    revision: Revision,
    language: Language,
    contexts: BTreeMap<u64, CodeContext>,
    focus_line: u64,
    builder: Option<SyntaxBuilderToken>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SyntaxBuilderToken {
    document_id: DocumentId,
    revision: Revision,
    language: Language,
}

impl Default for EditorSyntaxService {
    fn default() -> Self {
        Self {
            inner: Mutex::new(SyntaxCacheInner {
                document_id: None,
                revision: Revision::INITIAL,
                language: Language::Org,
                contexts: BTreeMap::from([(0, CodeContext::default())]),
                focus_line: 0,
                builder: None,
            }),
        }
    }
}

impl EditorSyntaxService {
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
            cache.focus_line = 0;
            cache.builder = None;
        } else {
            let keep_through = first_line / CONTEXT_CHECKPOINT_LINES * CONTEXT_CHECKPOINT_LINES;
            cache.contexts.retain(|line, _| *line <= keep_through);
            cache.contexts.entry(0).or_default();
            cache.focus_line = keep_through;
        }
        if cache.revision != revision {
            cache.builder = None;
        }
        cache.document_id = Some(document_id);
        cache.revision = revision;
    }

    pub(super) fn reset(&self) {
        let mut cache = self.inner.lock().expect("editor syntax cache poisoned");
        cache.document_id = None;
        cache.contexts.clear();
        cache.contexts.insert(0, CodeContext::default());
        cache.focus_line = 0;
        cache.builder = None;
    }

    fn context_at(
        &self,
        snapshot: &DocumentSnapshot,
        language: Language,
        first_line: u64,
    ) -> CodeContext {
        let document_id = snapshot.document_id();
        let revision = snapshot.revision();
        let (start, mut context) = {
            let mut cache = self.inner.lock().expect("editor syntax cache poisoned");
            if cache.document_id != Some(document_id)
                || cache.revision != revision
                || cache.language != language
            {
                cache.document_id = Some(document_id);
                cache.revision = revision;
                cache.language = language;
                cache.contexts.clear();
                cache.contexts.insert(0, CodeContext::default());
                cache.focus_line = 0;
                cache.builder = None;
            }
            let (&start, checkpoint_context) = cache
                .contexts
                .range(..=first_line)
                .next_back()
                .expect("line zero syntax checkpoint exists");
            (start, checkpoint_context.clone())
        };
        let _scan = tracing::info_span!("editor_semantic_scan").entered();
        let mut completed_checkpoints = Vec::new();
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
                    completed_checkpoints.push((line, context.clone()));
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
            completed_checkpoints.push((first_line, context.clone()));
        }
        if !completed_checkpoints.is_empty() {
            let mut cache = self.inner.lock().expect("editor syntax cache poisoned");
            if cache.document_id == Some(document_id)
                && cache.revision == revision
                && cache.language == language
            {
                cache.contexts.extend(completed_checkpoints);
            }
        }
        context
    }

    fn ready_context_at(
        &self,
        snapshot: &DocumentSnapshot,
        language: Language,
        first_line: u64,
    ) -> Option<CodeContext> {
        let ready = {
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
                cache.focus_line = 0;
                cache.builder = None;
            }
            let seed = cache
                .contexts
                .range(..=first_line)
                .next_back()
                .map(|(&line, _)| line)
                .unwrap_or(0);
            first_line.saturating_sub(seed) <= CONTEXT_CHECKPOINT_LINES
        };
        ready.then(|| self.context_at(snapshot, language, first_line))
    }

    fn request_build(&self, snapshot: &DocumentSnapshot, language: Language, line: u64) -> bool {
        let mut cache = self.inner.lock().expect("editor syntax cache poisoned");
        if cache.document_id != Some(snapshot.document_id())
            || cache.revision != snapshot.revision()
            || cache.language != language
        {
            return false;
        }
        cache.focus_line = cache.focus_line.max(line);
        let token = SyntaxBuilderToken {
            document_id: snapshot.document_id(),
            revision: snapshot.revision(),
            language,
        };
        if cache.builder == Some(token) {
            false
        } else {
            cache.builder = Some(token);
            true
        }
    }

    /// Advances only shared context checkpoints. This is intended to run on a
    /// background executor after `query_lines` returns Pending.
    pub(super) fn build_focused(&self, path: &Path, snapshot: &DocumentSnapshot) {
        let language = language(path);
        loop {
            let next = {
                let cache = self.inner.lock().expect("editor syntax cache poisoned");
                if cache.document_id != Some(snapshot.document_id())
                    || cache.revision != snapshot.revision()
                    || cache.language != language
                {
                    None
                } else {
                    let frontier = cache.contexts.keys().next_back().copied().unwrap_or(0);
                    (cache.focus_line > frontier.saturating_add(CONTEXT_CHECKPOINT_LINES))
                        .then_some(frontier.saturating_add(CONTEXT_CHECKPOINT_LINES))
                }
            };
            let Some(next) = next else {
                break;
            };
            self.context_at(snapshot, language, next);
        }
        let mut cache = self.inner.lock().expect("editor syntax cache poisoned");
        let token = SyntaxBuilderToken {
            document_id: snapshot.document_id(),
            revision: snapshot.revision(),
            language,
        };
        if cache.builder == Some(token) {
            cache.builder = None;
        }
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) enum EditorBlockKind {
    Source,
    Example,
    Quote,
    Verse,
    Center,
    Comment,
    Export,
    Special(Arc<str>),
    MarkdownFence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EditorBlockEdge {
    Open,
    Body,
    Close,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EditorBlockDecoration {
    pub(super) kind: EditorBlockKind,
    pub(super) edge: EditorBlockEdge,
    pub(super) body_line: Option<u64>,
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

impl BlockMetrics {
    pub(super) fn scaled(self, scale: f32) -> Self {
        Self {
            font_scale: self.font_scale,
            line_height: self.line_height * scale,
            before: self.before * scale,
            after: self.after * scale,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct EditorLineStyle {
    pub(super) source_range: ByteRange,
    pub(super) id: EditorStyleId,
    pub(super) code_language: Option<Arc<str>>,
    pub(super) block: Option<EditorBlockDecoration>,
    todo: Option<TodoSpan>,
    pub(super) metrics: BlockMetrics,
}

impl EditorLineStyle {
    pub(super) fn pending_fallback(source_range: ByteRange) -> Self {
        Self {
            source_range,
            id: EditorStyleId::Plain,
            code_language: None,
            block: None,
            todo: None,
            metrics: BlockMetrics::default(),
        }
    }
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

pub(super) struct EditorStyleQuery {
    pub(super) snapshot: SparseEditorStyleSnapshot,
    pub(super) pending: bool,
    pub(super) start_builder: bool,
}

#[cfg(test)]
impl EditorStyleSnapshot {
    pub(super) fn for_lines(
        path: &Path,
        snapshot: &DocumentSnapshot,
        lines: Range<u64>,
        cache: &EditorSyntaxService,
    ) -> Self {
        let language = language(path);
        let mut code = cache.context_at(snapshot, language, lines.start);
        let styles = lines
            .filter_map(|line| {
                let source_range = snapshot.line_content_range(LineIndex(line)).ok()?;
                let text = classification_text(snapshot, source_range);
                let todo = heading_todo_span(language, &text, &code);
                let block_before = code.block_kind.clone();
                let block_body_line_before = code.block_body_line;
                let id = classify_line(language, &text, &mut code);
                Some(EditorLineStyle {
                    source_range,
                    id,
                    code_language: (id == EditorStyleId::Code)
                        .then(|| code.code_language.clone())
                        .flatten(),
                    block: block_decoration(
                        id,
                        block_before,
                        code.block_kind.clone(),
                        block_body_line_before,
                        code.block_body_line,
                    ),
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
    pub(super) fn query_lines(
        path: &Path,
        snapshot: &DocumentSnapshot,
        lines: &[u64],
        service: &EditorSyntaxService,
    ) -> EditorStyleQuery {
        let language = language(path);
        let mut requested = lines.to_vec();
        requested.sort_unstable();
        requested.dedup();
        let mut styles = BTreeMap::new();
        let mut pending = false;
        let mut furthest_pending = 0;
        let mut index = 0;
        while index < requested.len() {
            let range_start = requested[index];
            let mut range_end = range_start.saturating_add(1);
            while index + 1 < requested.len() && requested[index + 1] == range_end {
                index += 1;
                range_end = range_end.saturating_add(1);
            }
            if let Some(code) = service.ready_context_at(snapshot, language, range_start) {
                append_styles(
                    snapshot,
                    language,
                    range_start..range_end,
                    code,
                    &mut styles,
                );
            } else {
                pending = true;
                furthest_pending = furthest_pending.max(range_start);
            }
            index += 1;
        }
        let start_builder = pending && service.request_build(snapshot, language, furthest_pending);
        EditorStyleQuery {
            snapshot: Self {
                revision: snapshot.revision(),
                lines: styles,
            },
            pending,
            start_builder,
        }
    }

    #[cfg(test)]
    pub(super) fn for_lines(
        path: &Path,
        snapshot: &DocumentSnapshot,
        lines: &[u64],
        cache: &EditorSyntaxService,
    ) -> Self {
        let language = language(path);
        let mut requested = lines.to_vec();
        requested.sort_unstable();
        requested.dedup();
        let mut styles = BTreeMap::new();
        let mut index = 0;
        while index < requested.len() {
            let range_start = requested[index];
            let mut range_end = range_start.saturating_add(1);
            while index + 1 < requested.len() && requested[index + 1] == range_end {
                index += 1;
                range_end = range_end.saturating_add(1);
            }
            let code = cache.context_at(snapshot, language, range_start);
            append_styles(
                snapshot,
                language,
                range_start..range_end,
                code,
                &mut styles,
            );
            index += 1;
        }
        Self {
            revision: snapshot.revision(),
            lines: styles,
        }
    }

    pub(super) fn line(&self, line: u64) -> Option<&EditorLineStyle> {
        self.lines.get(&line)
    }
}

fn append_styles(
    snapshot: &DocumentSnapshot,
    language: Language,
    lines: Range<u64>,
    mut code: CodeContext,
    styles: &mut BTreeMap<u64, EditorLineStyle>,
) {
    for line in lines {
        let Some(source_range) = snapshot.line_content_range(LineIndex(line)).ok() else {
            continue;
        };
        let text = classification_text(snapshot, source_range);
        let todo = heading_todo_span(language, &text, &code);
        let block_before = code.block_kind.clone();
        let block_body_line_before = code.block_body_line;
        let id = classify_line(language, &text, &mut code);
        styles.insert(
            line,
            EditorLineStyle {
                source_range,
                id,
                code_language: (id == EditorStyleId::Code)
                    .then(|| code.code_language.clone())
                    .flatten(),
                block: block_decoration(
                    id,
                    block_before,
                    code.block_kind.clone(),
                    block_body_line_before,
                    code.block_body_line,
                ),
                todo,
                metrics: metrics_for(id),
            },
        );
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
    fn token(self) -> EditorColorToken {
        match self {
            Self::Open => EditorColorToken::Todo,
            Self::Active => EditorColorToken::TodoActive,
            Self::Project => EditorColorToken::TodoProject,
            Self::Waiting => EditorColorToken::Waiting,
            Self::Done => EditorColorToken::Done,
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
    block_kind: Option<EditorBlockKind>,
    block_body_line: u64,
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
            block_kind: None,
            block_body_line: 0,
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
        code.block_body_line = code.block_body_line.saturating_add(1);
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
                    code.block_kind = None;
                    code.block_body_line = 0;
                    code.org_end_marker = None;
                    code.code_language = None;
                    return true;
                }
                return false;
            }
            if let Some((name, language)) = org_block_start(text) {
                code.in_block = true;
                code.block_kind = Some(match name.as_str() {
                    "src" => EditorBlockKind::Source,
                    "example" => EditorBlockKind::Example,
                    "quote" => EditorBlockKind::Quote,
                    "verse" => EditorBlockKind::Verse,
                    "center" => EditorBlockKind::Center,
                    "comment" => EditorBlockKind::Comment,
                    "export" => EditorBlockKind::Export,
                    _ => EditorBlockKind::Special(Arc::from(name.as_str())),
                });
                code.org_end_marker = Some(Arc::from(format!("#+end_{name}")));
                code.code_language = language;
                code.block_body_line = 0;
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
                code.block_kind = None;
                code.block_body_line = 0;
                code.markdown_fence = None;
                code.code_language = None;
            } else {
                code.in_block = true;
                code.block_kind = Some(EditorBlockKind::MarkdownFence);
                code.block_body_line = 0;
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

fn block_decoration(
    style: EditorStyleId,
    before: Option<EditorBlockKind>,
    after: Option<EditorBlockKind>,
    block_body_line_before: u64,
    block_body_line: u64,
) -> Option<EditorBlockDecoration> {
    match style {
        EditorStyleId::CodeBoundary => match (before, after) {
            (None, Some(kind)) => Some(EditorBlockDecoration {
                kind,
                edge: EditorBlockEdge::Open,
                body_line: None,
            }),
            (Some(kind), None) => Some(EditorBlockDecoration {
                kind,
                edge: EditorBlockEdge::Close,
                body_line: Some(block_body_line_before.saturating_add(1)),
            }),
            _ => None,
        },
        EditorStyleId::Code => after.or(before).map(|kind| EditorBlockDecoration {
            kind,
            edge: EditorBlockEdge::Body,
            body_line: Some(block_body_line),
        }),
        _ => None,
    }
}

fn update_code_context(language: Language, text: &str, code: &mut CodeContext) {
    let trimmed = text.trim_start();
    let was_in_block = code.in_block;
    let boundary = code_boundary(language, trimmed, code);
    if !boundary && code.in_block {
        code.block_body_line = code.block_body_line.saturating_add(1);
    }
    if language == Language::Org && !was_in_block && !boundary && !code.in_block {
        update_org_todo_faces(trimmed, code);
    }
}

fn update_org_todo_faces(text: &str, context: &mut CodeContext) {
    let Some(sequence) = crate::org_semantic::parse_todo_directive(text) else {
        return;
    };

    if !context.has_custom_todo_faces {
        context.todo_faces.clear();
        context.has_custom_todo_faces = true;
    }

    for state in sequence.states.iter() {
        let default_face = match state.kind {
            crate::org_semantic::TodoStateKind::Open => TodoFace::Open,
            crate::org_semantic::TodoStateKind::Done => TodoFace::Done,
        };
        let face = match state.keyword.as_ref() {
            "PROJ" => TodoFace::Project,
            "STRT" => TodoFace::Active,
            "WAIT" | "HOLD" => TodoFace::Waiting,
            _ => default_face,
        };
        context.todo_faces.insert(Arc::clone(&state.keyword), face);
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
    color: Option<EditorColorToken>,
    weight: Option<FontWeight>,
    font_style: Option<FontStyle>,
    underline: bool,
    strikethrough: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EditorColorToken {
    Foreground,
    InlineCode,
    Verbatim,
    Link,
    Date,
    Todo,
    TodoActive,
    TodoProject,
    Waiting,
    Done,
    Keyword,
    String,
    Comment,
    Type,
    Function,
    Constant,
    Number,
    Variable,
    Operator,
    Attribute,
    Meta,
}

impl EditorColorToken {
    pub(super) fn resolve(self, theme: &Theme) -> u32 {
        match self {
            Self::Foreground => theme.foreground,
            Self::InlineCode => theme.inline_code,
            Self::Verbatim => theme.verbatim,
            Self::Link => theme.link,
            Self::Date => theme.date,
            Self::Todo => theme.todo,
            Self::TodoActive => theme.todo_active,
            Self::TodoProject => theme.todo_project,
            Self::Waiting => theme.waiting,
            Self::Done => theme.done,
            Self::Keyword => theme.keyword,
            Self::String => theme.string,
            Self::Comment => theme.comment,
            Self::Type => theme.type_name,
            Self::Function => theme.function,
            Self::Constant => theme.constant,
            Self::Number => theme.number,
            Self::Variable => theme.variable,
            Self::Operator => theme.operator,
            Self::Attribute => theme.attribute,
            Self::Meta => theme.meta,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum EditorSemanticWeight {
    #[default]
    Normal,
    Semibold,
    Bold,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EditorSemanticSpan {
    pub(super) bytes: Range<usize>,
    pub(super) color: Option<EditorColorToken>,
    pub(super) weight: EditorSemanticWeight,
    pub(super) italic: bool,
    pub(super) underline: bool,
    pub(super) strikethrough: bool,
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
            base.color = rgb(theme.meta).into();
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

    let spans = semantic_spans(path, text, line_style);

    let mut boundaries = vec![0, text.len()];
    for span in &spans {
        boundaries.extend([span.bytes.start, span.bytes.end]);
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
            for span in spans
                .iter()
                .filter(|span| span.bytes.start <= range.start && range.start < span.bytes.end)
            {
                if let Some(color) = span.color {
                    run.color = rgb(color.resolve(theme)).into();
                }
                run.font.weight = match span.weight {
                    EditorSemanticWeight::Normal => run.font.weight,
                    EditorSemanticWeight::Semibold => FontWeight::SEMIBOLD,
                    EditorSemanticWeight::Bold => FontWeight::BOLD,
                };
                if span.italic {
                    run.font.style = FontStyle::Italic;
                }
                if span.underline {
                    run.underline = Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    });
                }
                if span.strikethrough {
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

/// Returns theme-independent inline and code semantics for both the editor and
/// its minimap. Ranges always refer to UTF-8 byte offsets in `text`.
pub(super) fn semantic_spans(
    path: &Path,
    text: &str,
    line_style: &EditorLineStyle,
) -> Vec<EditorSemanticSpan> {
    let mut spans = Vec::<(Range<usize>, SpanStyle)>::new();
    let verbatim = matches!(
        line_style.id,
        EditorStyleId::Code | EditorStyleId::CodeBoundary
    );
    let document_language = language(path);
    let mut opaque_inline_ranges = Vec::new();
    if !verbatim {
        opaque_inline_ranges = collect_inline_semantics(document_language, text, &mut spans);
        if let Some(todo) = &line_style.todo {
            spans.push((
                todo.range.clone(),
                SpanStyle {
                    color: Some(todo.face.token()),
                    weight: Some(FontWeight::BOLD),
                    ..SpanStyle::default()
                },
            ));
        }
        match line_style.id {
            EditorStyleId::Property => {
                collect_org_property_key(text, EditorColorToken::Attribute, &mut spans)
            }
            EditorStyleId::Meta => collect_org_meta_key(text, EditorColorToken::Meta, &mut spans),
            EditorStyleId::List => {
                collect_list_marker(text, EditorColorToken::InlineCode, &mut spans)
            }
            _ => {}
        }
    }
    if line_style.id == EditorStyleId::Code
        && let Some(language) = line_style.code_language.as_deref()
        && let Ok(code_spans) = crate::syntax_highlighting::highlight_code(language, text)
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
                        code_span_style(span.kind, &text[span.start..span.end]),
                    )
                }),
        );
        if matches!(
            language.trim().to_ascii_lowercase().as_str(),
            "cpp" | "c++" | "cc" | "cxx" | "hpp"
        ) {
            collect_cpp_namespace_qualifiers(
                text,
                EditorColorToken::Variable,
                EditorColorToken::Foreground,
                &mut spans,
            );
        }
    }
    match document_language {
        Language::Org if !verbatim => {
            for keyword in ["SCHEDULED:", "DEADLINE:"] {
                collect_token_outside(
                    text,
                    keyword,
                    EditorColorToken::Meta,
                    &opaque_inline_ranges,
                    &mut spans,
                );
            }
            collect_token_outside(
                text,
                "CLOSED:",
                EditorColorToken::Done,
                &opaque_inline_ranges,
                &mut spans,
            );
            for priority in ["[#A]", "[#B]", "[#C]"] {
                collect_token_outside(
                    text,
                    priority,
                    EditorColorToken::Keyword,
                    &opaque_inline_ranges,
                    &mut spans,
                );
            }
            collect_org_tags_outside(
                text,
                EditorColorToken::Attribute,
                &opaque_inline_ranges,
                &mut spans,
            );
        }
        Language::Markdown | Language::Org => {}
    }

    spans
        .into_iter()
        .filter(|(range, _)| {
            range.start < range.end
                && range.end <= text.len()
                && text.is_char_boundary(range.start)
                && text.is_char_boundary(range.end)
        })
        .map(|(bytes, style)| EditorSemanticSpan {
            bytes,
            color: style.color,
            weight: match style.weight {
                Some(FontWeight::BOLD) => EditorSemanticWeight::Bold,
                Some(FontWeight::SEMIBOLD) => EditorSemanticWeight::Semibold,
                _ => EditorSemanticWeight::Normal,
            },
            italic: style.font_style == Some(FontStyle::Italic),
            underline: style.underline,
            strikethrough: style.strikethrough,
        })
        .collect()
}

fn code_span_style(kind: crate::syntax_highlighting::CodeHighlightKind, source: &str) -> SpanStyle {
    use crate::syntax_highlighting::CodeHighlightKind;

    let color = match kind {
        CodeHighlightKind::Keyword if source.starts_with('#') => EditorColorToken::Comment,
        CodeHighlightKind::Attribute => EditorColorToken::Attribute,
        CodeHighlightKind::Boolean | CodeHighlightKind::Constant => EditorColorToken::Constant,
        CodeHighlightKind::Comment => EditorColorToken::Comment,
        CodeHighlightKind::Function => EditorColorToken::Function,
        CodeHighlightKind::Keyword => EditorColorToken::Keyword,
        CodeHighlightKind::Number => EditorColorToken::Number,
        CodeHighlightKind::Operator | CodeHighlightKind::Punctuation => EditorColorToken::Operator,
        CodeHighlightKind::Property => EditorColorToken::Foreground,
        CodeHighlightKind::Variable => EditorColorToken::Variable,
        CodeHighlightKind::String => EditorColorToken::String,
        CodeHighlightKind::Type => EditorColorToken::Type,
    };
    SpanStyle {
        color: Some(color),
        font_style: matches!(kind, CodeHighlightKind::Comment).then_some(FontStyle::Italic),
        ..SpanStyle::default()
    }
}

fn collect_inline_semantics(
    language: Language,
    text: &str,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) -> Vec<Range<usize>> {
    let inline = match language {
        Language::Org => crate::org_syntax::inline::parse(text),
        Language::Markdown => crate::preview::markdown::parse_markdown_inline(text),
    };
    let opaque_ranges = collect_inline_spans(language, text, &inline, spans);

    collect_token_outside(text, "[ ]", EditorColorToken::Todo, &opaque_ranges, spans);
    collect_token_outside(
        text,
        "[-]",
        EditorColorToken::TodoActive,
        &opaque_ranges,
        spans,
    );
    collect_token_outside(
        text,
        "[?]",
        EditorColorToken::Waiting,
        &opaque_ranges,
        spans,
    );
    for checkbox in ["[X]", "[x]"] {
        collect_token_outside(
            text,
            checkbox,
            EditorColorToken::Done,
            &opaque_ranges,
            spans,
        );
    }
    opaque_ranges
}

fn collect_inline_spans(
    language: Language,
    text: &str,
    inline: &InlineText,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) -> Vec<Range<usize>> {
    let mut opaque_ranges = Vec::new();
    for inline_span in &inline.spans {
        let source = inline_span.source.clone();
        if source.start >= source.end
            || source.end > text.len()
            || !text.is_char_boundary(source.start)
            || !text.is_char_boundary(source.end)
        {
            continue;
        }

        let style = match inline_span.kind {
            InlineKind::Bold => SpanStyle {
                weight: Some(FontWeight::BOLD),
                ..SpanStyle::default()
            },
            InlineKind::Italic => SpanStyle {
                font_style: Some(FontStyle::Italic),
                ..SpanStyle::default()
            },
            InlineKind::Underline => SpanStyle {
                underline: true,
                ..SpanStyle::default()
            },
            InlineKind::Strike => SpanStyle {
                strikethrough: true,
                ..SpanStyle::default()
            },
            InlineKind::Code => SpanStyle {
                color: Some(EditorColorToken::InlineCode),
                ..SpanStyle::default()
            },
            InlineKind::Verbatim => SpanStyle {
                color: Some(EditorColorToken::Verbatim),
                ..SpanStyle::default()
            },
            InlineKind::Link | InlineKind::FootnoteReference => SpanStyle {
                color: Some(EditorColorToken::Link),
                underline: language == Language::Markdown,
                ..SpanStyle::default()
            },
            InlineKind::Timestamp => SpanStyle {
                color: Some(EditorColorToken::Date),
                ..SpanStyle::default()
            },
            InlineKind::Target | InlineKind::RadioTarget => SpanStyle {
                color: Some(EditorColorToken::Attribute),
                ..SpanStyle::default()
            },
            InlineKind::Entity | InlineKind::Latex => SpanStyle {
                color: Some(EditorColorToken::Constant),
                ..SpanStyle::default()
            },
        };
        spans.push((source.clone(), style));

        if matches!(inline_span.kind, InlineKind::Code | InlineKind::Verbatim) {
            opaque_ranges.push(source.clone());
        }
        if language == Language::Org && inline_span.kind == InlineKind::Link {
            collect_org_link_inner_weights(text, source, spans);
        }
    }
    opaque_ranges.sort_unstable_by_key(|range| range.start);
    opaque_ranges
}

fn collect_cpp_namespace_qualifiers(
    text: &str,
    qualifier_color: EditorColorToken,
    member_color: EditorColorToken,
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

fn collect_org_link_inner_weights(
    text: &str,
    source: Range<usize>,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
    let Some(link) = text.get(source.clone()) else {
        return;
    };
    let Some(inside) = link
        .strip_prefix("[[")
        .and_then(|link| link.strip_suffix("]]"))
    else {
        return;
    };
    let body = source.start + 2;
    let end = source.end - 2;

    // Keep the brackets regular and emphasize the target/description source,
    // matching the existing editor treatment while the parser owns recognition.
    if let Some(separator) = inside.find("][").map(|index| body + index) {
        push_bold_span(body..separator, spans);
        push_bold_span(separator + 2..end, spans);
    } else {
        push_bold_span(body..end, spans);
    }
}

fn push_bold_span(range: Range<usize>, spans: &mut Vec<(Range<usize>, SpanStyle)>) {
    if range.start < range.end {
        spans.push((
            range,
            SpanStyle {
                weight: Some(FontWeight::BOLD),
                ..SpanStyle::default()
            },
        ));
    }
}

fn collect_org_property_key(
    text: &str,
    color: EditorColorToken,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
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

fn collect_org_meta_key(
    text: &str,
    color: EditorColorToken,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
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

fn collect_list_marker(
    text: &str,
    color: EditorColorToken,
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
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

fn collect_token_outside(
    text: &str,
    token: &str,
    color: EditorColorToken,
    opaque_ranges: &[Range<usize>],
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
    let mut cursor = 0;
    while let Some(start) = text[cursor..].find(token).map(|index| cursor + index) {
        let end = start + token.len();
        if !range_intersects_any(&(start..end), opaque_ranges) {
            spans.push((
                start..end,
                SpanStyle {
                    color: Some(color),
                    weight: Some(FontWeight::SEMIBOLD),
                    ..SpanStyle::default()
                },
            ));
        }
        cursor = end;
    }
}

fn collect_org_tags_outside(
    text: &str,
    color: EditorColorToken,
    opaque_ranges: &[Range<usize>],
    spans: &mut Vec<(Range<usize>, SpanStyle)>,
) {
    let trimmed_end = text.trim_end();
    let Some(start) = trimmed_end.rfind(' ') else {
        return;
    };
    let candidate = &trimmed_end[start + 1..];
    let range = start + 1..trimmed_end.len();
    if candidate.len() >= 3
        && candidate.starts_with(':')
        && candidate.ends_with(':')
        && candidate[1..candidate.len() - 1].chars().all(|character| {
            character.is_alphanumeric() || matches!(character, ':' | '_' | '@' | '#')
        })
        && !range_intersects_any(&range, opaque_ranges)
    {
        spans.push((
            range,
            SpanStyle {
                color: Some(color),
                ..SpanStyle::default()
            },
        ));
    }
}

fn range_intersects_any(range: &Range<usize>, others: &[Range<usize>]) -> bool {
    others
        .iter()
        .any(|other| range.start < other.end && other.start < range.end)
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
        let block_before = context.block_kind.clone();
        let block_body_line_before = context.block_body_line;
        let id = classify_line(Language::Org, text, context);
        EditorLineStyle {
            source_range: ByteRange::new(0, text.len() as u64),
            id,
            code_language: None,
            block: block_decoration(
                id,
                block_before,
                context.block_kind.clone(),
                block_body_line_before,
                context.block_body_line,
            ),
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
    fn editor_uses_parser_spans_for_org_emphasis() {
        let text = "*=render_document= returns formatted output.* \
                    Then call =load_config= to read settings.";
        let style = line_style(text, &mut CodeContext::default());
        let theme = current_theme();
        let markup_runs = runs(
            Path::new("a.org"),
            text,
            base_run(text.len()),
            &style,
            None,
            theme,
        );
        for (offset, _) in text.match_indices('_') {
            assert!(
                run_at(&markup_runs, offset).underline.is_none(),
                "identifier underscore at byte {offset} was treated as Org underline"
            );
        }
        assert_eq!(
            run_at(&markup_runs, text.find("render_document").unwrap())
                .font
                .weight,
            FontWeight::BOLD
        );
        assert_eq!(
            run_at(&markup_runs, text.find("load_config").unwrap()).color,
            rgb(theme.verbatim).into()
        );

        let underline_text = "_real underline_";
        let underline_style = line_style(underline_text, &mut CodeContext::default());
        let underline_runs = runs(
            Path::new("a.org"),
            underline_text,
            base_run(underline_text.len()),
            &underline_style,
            None,
            theme,
        );
        assert!(
            run_at(&underline_runs, underline_text.find("real").unwrap())
                .underline
                .is_some()
        );
    }

    #[test]
    fn org_tokens_inside_literal_spans_keep_the_literal_face() {
        let text = "=SCHEDULED: [X] :tag:= outside [X]";
        let style = line_style(text, &mut CodeContext::default());
        let theme = current_theme();
        let markup_runs = runs(
            Path::new("a.org"),
            text,
            base_run(text.len()),
            &style,
            None,
            theme,
        );
        assert_eq!(
            run_at(&markup_runs, text.find("SCHEDULED:").unwrap()).color,
            rgb(theme.verbatim).into()
        );
        assert_eq!(
            run_at(&markup_runs, text.find("[X]").unwrap()).color,
            rgb(theme.verbatim).into()
        );
        assert_eq!(
            run_at(&markup_runs, text.rfind("[X]").unwrap()).color,
            rgb(theme.done).into()
        );
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
            &EditorSyntaxService::default(),
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
            &EditorSyntaxService::default(),
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
        let cache = EditorSyntaxService::default();
        let styles = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 0..4, &cache);
        assert_eq!(styles.lines[0].id, EditorStyleId::CodeBoundary);
        assert_eq!(
            styles.lines[0].block,
            Some(EditorBlockDecoration {
                kind: EditorBlockKind::Source,
                edge: EditorBlockEdge::Open,
                body_line: None,
            })
        );
        assert_eq!(styles.lines[1].id, EditorStyleId::Code);
        assert_eq!(styles.lines[1].code_language.as_deref(), Some("rust"));
        assert_eq!(styles.lines[1].block.as_ref().unwrap().body_line, Some(1));
        assert_eq!(styles.lines[2].id, EditorStyleId::CodeBoundary);
        assert_eq!(
            styles.lines[2].block,
            Some(EditorBlockDecoration {
                kind: EditorBlockKind::Source,
                edge: EditorBlockEdge::Close,
                body_line: Some(2),
            })
        );
        assert_eq!(styles.lines[3].id, EditorStyleId::Plain);
        let body_only = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 1..2, &cache);
        assert_eq!(body_only.lines[0].id, EditorStyleId::Code);
        assert_eq!(
            body_only.lines[0].block.as_ref().unwrap().body_line,
            Some(1)
        );
    }

    #[test]
    fn source_body_line_numbers_are_stable_when_the_viewport_starts_inside_the_block() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"#+begin_src plantuml\n@startuml\nAlice -> Bob\n@enduml\n#+end_src\n".to_vec(),
        )
        .unwrap();
        let cache = EditorSyntaxService::default();
        let styles = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 0..5, &cache);
        assert_eq!(styles.lines[1].block.as_ref().unwrap().body_line, Some(1));
        assert_eq!(styles.lines[2].block.as_ref().unwrap().body_line, Some(2));
        assert_eq!(styles.lines[3].block.as_ref().unwrap().body_line, Some(3));
        assert_eq!(styles.lines[4].block.as_ref().unwrap().body_line, Some(4));

        let middle = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 2..3, &cache);
        assert_eq!(middle.lines[0].block.as_ref().unwrap().body_line, Some(2));
    }

    #[test]
    fn every_standard_and_custom_org_block_keeps_its_decoration_kind() {
        let source = concat!(
            "#+begin_src rust\n#+end_src\n",
            "#+begin_example\n#+end_example\n",
            "#+begin_quote\n#+end_quote\n",
            "#+begin_verse\n#+end_verse\n",
            "#+begin_center\n#+end_center\n",
            "#+begin_comment\n#+end_comment\n",
            "#+begin_export html\n#+end_export\n",
            "#+begin_details\n#+end_details\n",
        );
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let styles = EditorStyleSnapshot::for_lines(
            Path::new("a.org"),
            &snapshot,
            0..16,
            &EditorSyntaxService::default(),
        );
        let expected = [
            EditorBlockKind::Source,
            EditorBlockKind::Example,
            EditorBlockKind::Quote,
            EditorBlockKind::Verse,
            EditorBlockKind::Center,
            EditorBlockKind::Comment,
            EditorBlockKind::Export,
            EditorBlockKind::Special(Arc::from("details")),
        ];

        for (index, kind) in expected.into_iter().enumerate() {
            let open = styles.lines[index * 2].block.as_ref().unwrap();
            let close = styles.lines[index * 2 + 1].block.as_ref().unwrap();
            assert_eq!(open.kind, kind);
            assert_eq!(open.edge, EditorBlockEdge::Open);
            assert_eq!(close.kind, open.kind);
            assert_eq!(close.edge, EditorBlockEdge::Close);
        }
    }

    #[test]
    fn example_block_keeps_nested_org_blocks_verbatim() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"#+begin_example\n#+name: hello-rust\n#+begin_src rust :results output\nfn main() {\n    println!(\"hello\");\n}\n#+end_src\n#+end_example\nafter\n".to_vec(),
        )
        .unwrap();
        let cache = EditorSyntaxService::default();
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
            block: None,
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
            &EditorSyntaxService::default(),
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
    fn semantic_spans_are_shared_tokens_with_valid_complex_utf8_ranges() {
        let text = "前缀 [[https://例子.invalid][链接😀]] cafe\u{301} שלום *粗体*";
        let style = EditorLineStyle {
            source_range: ByteRange::new(0, text.len() as u64),
            id: EditorStyleId::Plain,
            code_language: None,
            block: None,
            todo: None,
            metrics: metrics_for(EditorStyleId::Plain),
        };
        let spans = semantic_spans(Path::new("unicode.org"), text, &style);

        assert!(!spans.is_empty());
        assert!(
            spans
                .iter()
                .any(|span| span.color == Some(EditorColorToken::Link))
        );
        assert!(spans.iter().all(|span| {
            span.bytes.start < span.bytes.end
                && span.bytes.end <= text.len()
                && text.is_char_boundary(span.bytes.start)
                && text.is_char_boundary(span.bytes.end)
        }));
    }

    #[test]
    fn supported_code_language_produces_neutral_highlight_tokens() {
        let text = "fn main() { let message = \"你好😀\"; } // cafe\u{301}";
        let style = EditorLineStyle {
            source_range: ByteRange::new(0, text.len() as u64),
            id: EditorStyleId::Code,
            code_language: Some(Arc::from("rust")),
            block: None,
            todo: None,
            metrics: metrics_for(EditorStyleId::Code),
        };
        let spans = semantic_spans(Path::new("code.org"), text, &style);

        for token in [
            EditorColorToken::Keyword,
            EditorColorToken::Function,
            EditorColorToken::String,
            EditorColorToken::Comment,
        ] {
            assert!(
                spans.iter().any(|span| span.color == Some(token)),
                "missing {token:?}"
            );
        }
        assert!(spans.iter().all(|span| {
            text.is_char_boundary(span.bytes.start) && text.is_char_boundary(span.bytes.end)
        }));
    }

    #[test]
    fn unsupported_code_languages_keep_only_the_code_base_style() {
        for language in ["typst", "plantuml"] {
            let text = "#show: 中文😀\nAlice -> Bob";
            let style = EditorLineStyle {
                source_range: ByteRange::new(0, text.len() as u64),
                id: EditorStyleId::Code,
                code_language: Some(Arc::from(language)),
                block: None,
                todo: None,
                metrics: metrics_for(EditorStyleId::Code),
            };

            assert!(semantic_spans(Path::new("code.org"), text, &style).is_empty());
        }
    }

    #[test]
    fn long_code_line_highlights_without_invalid_or_truncated_semantic_ranges() {
        let text = format!("let payload = \"{}😀\"; // 尾部", "x".repeat(16_384));
        let style = EditorLineStyle {
            source_range: ByteRange::new(0, text.len() as u64),
            id: EditorStyleId::Code,
            code_language: Some(Arc::from("rust")),
            block: None,
            todo: None,
            metrics: metrics_for(EditorStyleId::Code),
        };
        let spans = semantic_spans(Path::new("long.org"), &text, &style);

        assert!(
            spans
                .iter()
                .any(|span| span.color == Some(EditorColorToken::String))
        );
        assert!(
            spans
                .iter()
                .any(|span| span.color == Some(EditorColorToken::Comment))
        );
        assert!(spans.iter().all(|span| {
            span.bytes.end <= text.len()
                && text.is_char_boundary(span.bytes.start)
                && text.is_char_boundary(span.bytes.end)
        }));
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
            &EditorSyntaxService::default(),
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
        assert_eq!(boundary_runs[0].font.style, FontStyle::Normal);
        assert_eq!(boundary_runs[0].color, rgb(theme.meta).into());

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
        let cache = EditorSyntaxService::default();

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

    #[test]
    fn cold_random_queries_return_pending_and_share_one_checkpoint_builder() {
        let mut source = String::from("#+begin_quote\n");
        source.push_str(&"body\n".repeat(1_100));
        source.push_str("#+end_quote\n");
        let snapshot = DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
        let service = EditorSyntaxService::default();

        let first = SparseEditorStyleSnapshot::query_lines(
            Path::new("cold.org"),
            &snapshot,
            &[800],
            &service,
        );
        assert!(first.pending);
        assert!(first.start_builder);
        assert!(first.snapshot.line(800).is_none());
        assert_eq!(
            service
                .inner
                .lock()
                .unwrap()
                .contexts
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![0],
            "a cold query must not scan toward the target on its caller"
        );

        let second = SparseEditorStyleSnapshot::query_lines(
            Path::new("cold.org"),
            &snapshot,
            &[900],
            &service,
        );
        assert!(second.pending);
        assert!(!second.start_builder);

        service.build_focused(Path::new("cold.org"), &snapshot);
        let ready = SparseEditorStyleSnapshot::query_lines(
            Path::new("cold.org"),
            &snapshot,
            &[800, 900],
            &service,
        );
        assert!(!ready.pending);
        assert!(!ready.start_builder);
        for line in [800, 900] {
            let style = ready.snapshot.line(line).unwrap();
            assert_eq!(style.id, EditorStyleId::Code);
            assert_eq!(style.block.as_ref().unwrap().kind, EditorBlockKind::Quote);
        }
    }

    #[test]
    fn reset_releases_builder_token_even_when_revision_is_reused() {
        let snapshot = DocumentSnapshot::from_utf8("body\n".repeat(1_100).into_bytes()).unwrap();
        let service = EditorSyntaxService::default();

        let first = SparseEditorStyleSnapshot::query_lines(
            Path::new("before.org"),
            &snapshot,
            &[900],
            &service,
        );
        assert!(first.pending && first.start_builder);

        service.reset();
        let after_reset = SparseEditorStyleSnapshot::query_lines(
            Path::new("after.org"),
            &snapshot,
            &[900],
            &service,
        );
        assert!(after_reset.pending && after_reset.start_builder);
    }
}
