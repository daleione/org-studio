use std::{
    collections::BTreeMap,
    ops::Range,
    path::Path,
    sync::{Arc, Mutex},
};

use crate::document::{
    ByteOffset, ByteRange, DocumentId, DocumentSnapshot, LineCursor, LineIndex, Revision,
    TextSnapshot,
};

mod display;
mod spans;
mod style;

use style::metrics_for;
pub(super) use style::{
    BlockMetrics, EditorBlockDecoration, EditorBlockEdge, EditorBlockKind, EditorLineStyle,
    EditorStyleId,
};

pub(super) use spans::{
    EditorColorToken, EditorSemanticSpan, apply_swatch_text, emphasize_inline_delimiters,
    link_accent, org_tag_ranges, runs_from_spans, semantic_spans,
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
    display_config: Mutex<Option<crate::org_semantic::display::OrgDisplayConfig>>,
}

struct SyntaxCacheInner {
    document_id: Option<DocumentId>,
    revision: Revision,
    language: Language,
    contexts: BTreeMap<u64, CodeContext>,
    focus_line: u64,
    builder: Option<SyntaxBuilderToken>,
}

impl SyntaxCacheInner {
    fn contains_newer_revision(&self, document_id: DocumentId, revision: Revision) -> bool {
        self.document_id == Some(document_id) && self.revision > revision
    }
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
            display_config: Mutex::new(None),
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
        if cache.contains_newer_revision(document_id, revision) {
            return;
        }
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
            if cache.contains_newer_revision(document_id, revision) {
                // The caller can have passed ready_context_at before an edit arrived.
                // Parse its old snapshot privately; never roll the shared cache back.
                (0, CodeContext::default())
            } else {
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
            }
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
            if cache.contains_newer_revision(snapshot.document_id(), snapshot.revision()) {
                return None;
            }
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
    /// background executor after `query_lines` returns Pending (prefetch).
    pub(super) fn build_focused(&self, path: &Path, snapshot: &DocumentSnapshot) {
        self.build_until(path, snapshot, None)
    }

    /// Advances shared context checkpoints until `target` is covered (bounded
    /// flavor), or toward the recorded focus line when `target` is `None`.
    ///
    /// The bounded flavor is used synchronously by `query_lines` so an actually
    /// requested range never renders the Plain fallback: the block structure,
    /// code language and metrics are available the same frame. Building only up
    /// to `target` — never the shared focus line, which a minimap tail request
    /// may have set to the end of a large document — keeps a deep jump from
    /// rescanning the whole file on the calling thread.
    fn build_until(&self, path: &Path, snapshot: &DocumentSnapshot, target: Option<u64>) {
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
                    let target = target.unwrap_or(cache.focus_line);
                    (frontier.saturating_add(CONTEXT_CHECKPOINT_LINES) < target)
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
        let styles =
            SparseEditorStyleSnapshot::for_lines(path, snapshot, &lines.collect::<Vec<_>>(), cache);
        Self {
            revision: styles.revision,
            lines: styles.lines.into_values().collect(),
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
        if let Some(language) = crate::syntax_highlighting::language_for_path(path) {
            return EditorStyleQuery {
                snapshot: Self {
                    revision: snapshot.revision(),
                    lines: code_file_styles(snapshot, lines, language),
                },
                pending: false,
                start_builder: false,
            };
        }
        let language = language(path);
        let mut requested = lines.to_vec();
        requested.sort_unstable();
        requested.dedup();
        let mut styles = BTreeMap::new();
        let mut unresolved = Vec::new();
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
                unresolved.push((range_start, range_end));
            }
            index += 1;
        }
        let mut pending = false;
        let mut furthest_pending = 0;
        if !unresolved.is_empty() {
            // Synchronously advance the shared checkpoints to cover the requested
            // lines. An actually visible range therefore never renders Plain
            // fallback chrome: block structure, code language and metrics are
            // available on this frame, and the async builder stays as a pure
            // prefetch/fallback path. The bounded target (not the shared focus
            // line, which a minimap tail request may have pushed to the end of a
            // huge document) keeps a deep jump from rescanning the whole file on
            // the calling thread.
            let furthest = unresolved.last().map_or(0, |(start, _)| *start);
            service.request_build(snapshot, language, furthest);
            service.build_until(path, snapshot, Some(furthest));
            for (range_start, range_end) in unresolved {
                if let Some(code) = service.ready_context_at(snapshot, language, range_start) {
                    append_styles(
                        snapshot,
                        language,
                        range_start..range_end,
                        code,
                        &mut styles,
                    );
                } else {
                    // Only reachable for a snapshot older than the shared cache
                    // (background work racing an edit). Never roll the cache back.
                    pending = true;
                    furthest_pending = furthest_pending.max(range_start);
                }
            }
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
        if let Some(language) = crate::syntax_highlighting::language_for_path(path) {
            return Self {
                revision: snapshot.revision(),
                lines: code_file_styles(snapshot, lines, language),
            };
        }
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

fn code_file_styles(
    snapshot: &DocumentSnapshot,
    lines: &[u64],
    language: &'static str,
) -> BTreeMap<u64, EditorLineStyle> {
    let language: Arc<str> = Arc::from(language);
    lines
        .iter()
        .filter_map(|&line| {
            let source_range = snapshot.line_content_range(LineIndex(line)).ok()?;
            Some((
                line,
                EditorLineStyle {
                    source_range,
                    id: EditorStyleId::Plain,
                    code_language: Some(language.clone()),
                    block: None,
                    todo: None,
                    metrics: BlockMetrics::default(),
                },
            ))
        })
        .collect()
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
                metrics: metrics_for(language, id),
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
    markdown_fence: Option<(char, usize)>,
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

pub(super) fn classification_text(snapshot: &DocumentSnapshot, range: ByteRange) -> String {
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
    let boundary = code_boundary(language, text, code);
    if boundary {
        return EditorStyleId::CodeBoundary;
    }
    if code.in_block {
        code.block_body_line = code.block_body_line.saturating_add(1);
        return EditorStyleId::Code;
    }
    let trimmed = text.trim_start();
    if language == Language::Org {
        update_org_todo_faces(trimmed, code);
    }
    match language {
        Language::Org => {
            let stars = trimmed.bytes().take_while(|byte| *byte == b'*').count();
            if stars > 0
                && trimmed.len() == text.len()
                && trimmed.as_bytes().get(stars) == Some(&b' ')
            {
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
            } else if trimmed.starts_with('|') {
                EditorStyleId::Table
            } else if is_list_line(trimmed) {
                EditorStyleId::List
            } else {
                EditorStyleId::Plain
            }
        }
    }
}

fn code_boundary(language: Language, text: &str, code: &mut CodeContext) -> bool {
    let trimmed = text.trim_start();
    match language {
        Language::Org => {
            if let Some(end_marker) = code.org_end_marker.as_deref() {
                // `text` is the complete source line and normally includes its
                // trailing newline.  Compare the marker against a fully
                // trimmed line; using only `trim_start` leaves the newline in
                // place and makes every Org block appear unclosed.
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
            if let Some((name, language)) = org_block_start(trimmed) {
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
            if code.in_block {
                let Some((marker, count)) = code.markdown_fence else {
                    return false;
                };
                if crate::document::markdown::fence_close(text, marker, count) {
                    code.in_block = false;
                    code.block_kind = None;
                    code.block_body_line = 0;
                    code.markdown_fence = None;
                    code.code_language = None;
                    return true;
                }
                return false;
            }
            if let Some((marker, count, language)) = crate::document::markdown::fence_open(text) {
                code.in_block = true;
                code.block_kind = Some(EditorBlockKind::MarkdownFence);
                code.block_body_line = 0;
                code.markdown_fence = Some((marker, count));
                code.code_language = language.map(Arc::from);
                return true;
            }
            false
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
    let was_in_block = code.in_block;
    let boundary = code_boundary(language, text, code);
    if !boundary && code.in_block {
        code.block_body_line = code.block_body_line.saturating_add(1);
    }
    if language == Language::Org && !was_in_block && !boundary && !code.in_block {
        update_org_todo_faces(text.trim_start(), code);
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
            "STRT" | "DOING" | "NEXT" => TodoFace::Active,
            "WAIT" | "WAITING" | "HOLD" => TodoFace::Waiting,
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

fn is_list_line(text: &str) -> bool {
    text.starts_with("- ")
        || text.starts_with("+ ")
        || text.starts_with("* ")
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

#[cfg(test)]
mod tests;
