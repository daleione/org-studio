use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use gpui::{
    App, Context, FontStyle, FontWeight, HighlightStyle, IntoElement, ListAlignment, ListState,
    PathPromptOptions, Render, StyledText, Task, Window, actions, div, list, prelude::*, px, rgb,
};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

mod rows;

use rows::build_preview_rows;

use crate::{
    document::{ByteRange, RopeSnapshot, SharedTextSnapshot},
    org_syntax::{
        BlockArena, BlockId, BlockKind, BlockNode,
        inline::{InlineKind, InlineText, parse as parse_inline},
        parse,
    },
    theme::current_theme,
};

const INLINE_CACHE_CAPACITY: usize = 2048;
const INLINE_CACHE_MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_SYNC_INLINE_BYTES: usize = 64 * 1024;
const HIGHLIGHT_CACHE_CAPACITY: usize = 512;
const HIGHLIGHT_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;

const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "boolean",
    "comment",
    "conditional",
    "constant",
    "constructor",
    "delimiter",
    "embedded",
    "escape",
    "field",
    "function",
    "function.call",
    "keyword",
    "keyword.operator",
    "label",
    "number",
    "operator",
    "parameter",
    "property",
    "punctuation.bracket",
    "punctuation.delimiter",
    "string",
    "storageclass",
    "type",
    "type.builtin",
    "type.qualifier",
    "variable",
];

actions!(org_preview, [OpenDocument, ReloadDocument]);

pub struct PreviewDocument {
    pub path: PathBuf,
    pub text: SharedTextSnapshot,
    pub blocks: Arc<BlockArena>,
    rows: Arc<Vec<PreviewRow>>,
    inline_cache: Mutex<InlineCache>,
    highlight_cache: Mutex<HighlightCache>,
    pub metrics: LoadMetrics,
}

#[derive(Clone, Copy)]
pub(super) struct PreviewRow {
    pub(super) block_id: BlockId,
    pub(super) content: ByteRange,
    pub(super) continuation: bool,
    pub(super) source_line: u64,
    pub(super) show_line_number: bool,
    pub(super) blank: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LoadMetrics {
    pub bytes: u64,
    pub read: Duration,
    pub rope: Duration,
    pub parse: Duration,
    pub total: Duration,
}

struct InlineCache {
    capacity: usize,
    max_bytes: usize,
    bytes: usize,
    entries: HashMap<BlockId, InlineText>,
    order: VecDeque<BlockId>,
    pending: HashSet<BlockId>,
}

#[derive(Clone, Copy)]
struct CodeHighlightSpan {
    start: usize,
    end: usize,
    kind: CodeHighlightKind,
}

#[derive(Clone, Copy)]
enum CodeHighlightKind {
    Attribute,
    Boolean,
    Comment,
    Constant,
    Function,
    Keyword,
    Number,
    Operator,
    Property,
    Punctuation,
    String,
    Type,
    Variable,
}

struct HighlightCache {
    capacity: usize,
    max_bytes: usize,
    bytes: usize,
    entries: HashMap<BlockId, Arc<Vec<CodeHighlightSpan>>>,
    order: VecDeque<BlockId>,
    pending: HashSet<BlockId>,
}

impl HighlightCache {
    fn new(capacity: usize, max_bytes: usize) -> Self {
        Self {
            capacity,
            max_bytes,
            bytes: 0,
            entries: HashMap::new(),
            order: VecDeque::new(),
            pending: HashSet::new(),
        }
    }

    fn insert(&mut self, id: BlockId, spans: Vec<CodeHighlightSpan>) {
        self.pending.remove(&id);
        let entry_bytes = spans.len() * std::mem::size_of::<CodeHighlightSpan>();
        while !self.entries.is_empty()
            && (self.entries.len() >= self.capacity || self.bytes + entry_bytes > self.max_bytes)
        {
            if let Some(oldest) = self.order.pop_front()
                && let Some(removed) = self.entries.remove(&oldest)
            {
                self.bytes = self.bytes.saturating_sub(
                    removed.len() * std::mem::size_of::<CodeHighlightSpan>(),
                );
            }
        }
        if entry_bytes <= self.max_bytes {
            self.bytes += entry_bytes;
            self.order.push_back(id);
            self.entries.insert(id, Arc::new(spans));
        }
    }
}

impl InlineCache {
    fn new(capacity: usize, max_bytes: usize) -> Self {
        Self {
            capacity,
            max_bytes,
            bytes: 0,
            entries: HashMap::new(),
            order: VecDeque::new(),
            pending: HashSet::new(),
        }
    }

    fn get_or_insert(&mut self, id: BlockId, source: &str) -> InlineText {
        if let Some(parsed) = self.entries.get(&id) {
            return parsed.clone();
        }

        let parsed = parse_inline(source);
        self.insert(id, parsed.clone());
        parsed
    }

    fn insert(&mut self, id: BlockId, parsed: InlineText) {
        self.pending.remove(&id);
        let entry_bytes = parsed.text.len()
            + parsed.spans.len() * std::mem::size_of::<crate::org_syntax::inline::InlineSpan>();
        while !self.entries.is_empty()
            && (self.entries.len() >= self.capacity || self.bytes + entry_bytes > self.max_bytes)
        {
            if let Some(oldest) = self.order.pop_front()
                && let Some(removed) = self.entries.remove(&oldest)
            {
                self.bytes = self.bytes.saturating_sub(
                    removed.text.len()
                        + removed.spans.len()
                            * std::mem::size_of::<crate::org_syntax::inline::InlineSpan>(),
                );
            }
        }
        if entry_bytes <= self.max_bytes {
            self.bytes += entry_bytes;
            self.order.push_back(id);
            self.entries.insert(id, parsed);
        }
    }
}

impl PreviewDocument {
    fn inline(self: &Arc<Self>, id: BlockId, source: &str, cx: &mut App) -> InlineText {
        if source.len() > MAX_SYNC_INLINE_BYTES {
            let mut cache = self.inline_cache.lock().expect("inline cache poisoned");
            if let Some(parsed) = cache.entries.get(&id) {
                return parsed.clone();
            }
            if cache.pending.insert(id) {
                let document = self.clone();
                let source = source.to_owned();
                cx.spawn(async move |cx| {
                    let parsed = cx
                        .background_spawn(async move { parse_inline(&source) })
                        .await;
                    document
                        .inline_cache
                        .lock()
                        .expect("inline cache poisoned")
                        .insert(id, parsed);
                    let _ = cx.refresh();
                })
                .detach();
            }
            return InlineText {
                text: source.to_owned(),
                spans: Vec::new(),
            };
        }
        self.inline_cache
            .lock()
            .expect("inline cache poisoned")
            .get_or_insert(id, source)
    }

    fn code_highlights(
        self: &Arc<Self>,
        id: BlockId,
        language: Option<&str>,
        cx: &mut App,
    ) -> Option<Arc<Vec<CodeHighlightSpan>>> {
        let Some(language) = language.filter(|language| supports_code_language(language)) else {
            return None;
        };

        let mut cache = self
            .highlight_cache
            .lock()
            .expect("highlight cache poisoned");
        if let Some(spans) = cache.entries.get(&id) {
            return Some(spans.clone());
        }
        if cache.pending.insert(id) {
            let document = self.clone();
            let source = self
                .text
                .copy_range(self.blocks.nodes()[id as usize].source);
            let language = language.to_owned();
            cx.spawn(async move |cx| {
                let spans = cx
                    .background_spawn(async move {
                        highlight_code(&language, &source).unwrap_or_default()
                    })
                    .await;
                document
                    .highlight_cache
                    .lock()
                    .expect("highlight cache poisoned")
                    .insert(id, spans);
                let _ = cx.refresh();
            })
            .detach();
        }
        None
    }
}

fn supports_code_language(language: &str) -> bool {
    matches!(
        language.trim().to_ascii_lowercase().as_str(),
        "sql"
            | "postgres"
            | "postgresql"
            | "rust"
            | "rs"
            | "python"
            | "py"
            | "sh"
            | "shell"
            | "bash"
            | "zsh"
            | "javascript"
            | "js"
            | "jsx"
            | "typescript"
            | "ts"
            | "tsx"
            | "json"
            | "go"
            | "golang"
            | "c"
            | "h"
            | "cpp"
            | "c++"
            | "cc"
            | "cxx"
            | "hpp"
    )
}

fn highlight_code(language: &str, source: &str) -> Result<Vec<CodeHighlightSpan>, String> {
    let normalized = language.trim().to_ascii_lowercase();
    let combined_query: String;
    let (language, name, highlights, injections, locals) = match normalized.as_str() {
        "sql" | "postgres" | "postgresql" => (
            tree_sitter_sequel::LANGUAGE.into(),
            "sql",
            tree_sitter_sequel::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "rust" | "rs" => (
            tree_sitter_rust::LANGUAGE.into(),
            "rust",
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        ),
        "python" | "py" => (
            tree_sitter_python::LANGUAGE.into(),
            "python",
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "sh" | "shell" | "bash" | "zsh" => (
            tree_sitter_bash::LANGUAGE.into(),
            "bash",
            tree_sitter_bash::HIGHLIGHT_QUERY,
            "",
            "",
        ),
        "javascript" | "js" => (
            tree_sitter_javascript::LANGUAGE.into(),
            "javascript",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        ),
        "jsx" => {
            combined_query = format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
            );
            (
                tree_sitter_javascript::LANGUAGE.into(),
                "jsx",
                combined_query.as_str(),
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_javascript::LOCALS_QUERY,
            )
        }
        "typescript" | "ts" => {
            combined_query = format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            );
            (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                "typescript",
                combined_query.as_str(),
                "",
                tree_sitter_typescript::LOCALS_QUERY,
            )
        }
        "tsx" => {
            combined_query = format!(
                "{}\n{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            );
            (
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                "tsx",
                combined_query.as_str(),
                "",
                tree_sitter_typescript::LOCALS_QUERY,
            )
        }
        "json" => (
            tree_sitter_json::LANGUAGE.into(),
            "json",
            tree_sitter_json::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "go" | "golang" => (
            tree_sitter_go::LANGUAGE.into(),
            "go",
            tree_sitter_go::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "c" | "h" => (
            tree_sitter_c::LANGUAGE.into(),
            "c",
            tree_sitter_c::HIGHLIGHT_QUERY,
            "",
            "",
        ),
        "cpp" | "c++" | "cc" | "cxx" | "hpp" => {
            combined_query = format!(
                "{}\n{}",
                tree_sitter_c::HIGHLIGHT_QUERY,
                tree_sitter_cpp::HIGHLIGHT_QUERY
            );
            (
                tree_sitter_cpp::LANGUAGE.into(),
                "cpp",
                combined_query.as_str(),
                "",
                "",
            )
        }
        _ => return Ok(Vec::new()),
    };
    let mut configuration =
        HighlightConfiguration::new(language, name, highlights, injections, locals)
            .map_err(|error| error.to_string())?;
    configuration.configure(HIGHLIGHT_NAMES);

    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(&configuration, source.as_bytes(), None, |_| None)
        .map_err(|error| error.to_string())?;
    let mut active = Vec::new();
    let mut spans = Vec::new();
    for event in events {
        match event.map_err(|error| error.to_string())? {
            HighlightEvent::HighlightStart(highlight) => active.push(highlight.0),
            HighlightEvent::HighlightEnd => {
                active.pop();
            }
            HighlightEvent::Source { start, end } => {
                if start < end
                    && let Some(index) = active.last()
                    && let Some(kind) = code_highlight_kind(HIGHLIGHT_NAMES[*index])
                {
                    spans.push(CodeHighlightSpan { start, end, kind });
                }
            }
        }
    }
    Ok(spans)
}

fn code_highlight_kind(name: &str) -> Option<CodeHighlightKind> {
    let root = name.split('.').next().unwrap_or(name);
    Some(match root {
        "attribute" => CodeHighlightKind::Attribute,
        "boolean" => CodeHighlightKind::Boolean,
        "comment" => CodeHighlightKind::Comment,
        "constant" => CodeHighlightKind::Constant,
        "constructor" => CodeHighlightKind::Type,
        "embedded" | "escape" => CodeHighlightKind::String,
        "delimiter" | "punctuation" => CodeHighlightKind::Punctuation,
        "field" | "property" => CodeHighlightKind::Property,
        "function" => CodeHighlightKind::Function,
        "keyword" | "conditional" | "storageclass" => CodeHighlightKind::Keyword,
        "number" => CodeHighlightKind::Number,
        "operator" => CodeHighlightKind::Operator,
        "label" => CodeHighlightKind::Attribute,
        "string" => CodeHighlightKind::String,
        "type" => CodeHighlightKind::Type,
        "parameter" | "variable" => CodeHighlightKind::Variable,
        _ => return None,
    })
}

enum PreviewLoadState {
    Empty,
    Loading {
        path: PathBuf,
    },
    Ready {
        generation: u64,
        document: Arc<PreviewDocument>,
    },
    Failed {
        path: PathBuf,
        message: String,
    },
}

pub struct PreviewApp {
    state: PreviewLoadState,
    generation: u64,
    load_task: Option<Task<()>>,
    picker_task: Option<Task<()>>,
    list_state: ListState,
    last_ready: Option<(u64, Arc<PreviewDocument>)>,
    opened_at: Option<Instant>,
    first_frame_scheduled: Option<u64>,
    scroll_benchmark: Option<ScrollBenchmark>,
}

struct ScrollBenchmark {
    target_frames: usize,
    warmup_remaining: usize,
    sampling_started: bool,
    scroll_pixels: f32,
    samples: Vec<Duration>,
    last_frame: Instant,
}

impl PreviewApp {
    pub fn new() -> Self {
        let list_overdraw = std::env::var("ORG_STUDIO_LIST_OVERDRAW")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(80.0);
        Self {
            state: PreviewLoadState::Empty,
            generation: 0,
            load_task: None,
            picker_task: None,
            list_state: ListState::new(0, ListAlignment::Top, px(list_overdraw)),
            last_ready: None,
            opened_at: None,
            first_frame_scheduled: None,
            scroll_benchmark: std::env::var("ORG_STUDIO_SCROLL_BENCH_FRAMES")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|frames| *frames > 0)
                .map(|target_frames| ScrollBenchmark {
                    target_frames,
                    warmup_remaining: std::env::var("ORG_STUDIO_SCROLL_BENCH_WARMUP_FRAMES")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0),
                    sampling_started: false,
                    scroll_pixels: std::env::var("ORG_STUDIO_SCROLL_BENCH_PIXELS")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(640.0),
                    samples: Vec::with_capacity(target_frames),
                    last_frame: Instant::now(),
                }),
        }
    }

    pub fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.generation += 1;
        self.opened_at = Some(Instant::now());
        self.first_frame_scheduled = None;
        let generation = self.generation;
        self.state = PreviewLoadState::Loading { path: path.clone() };

        let background = cx.background_spawn(async move { load_document(path) });
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = background.await;
            let _ = this.update(cx, |this, cx| {
                if !accept_generation(this.generation, generation) {
                    return;
                }

                this.state = match result {
                    Ok(document) => {
                        this.list_state.reset(document.rows.len());
                        let document = Arc::new(document);
                        this.last_ready = Some((generation, document.clone()));
                        PreviewLoadState::Ready {
                            generation,
                            document,
                        }
                    }
                    Err((path, message)) => PreviewLoadState::Failed { path, message },
                };
                cx.notify();
            });
        }));

        cx.notify();
    }

    fn choose_file(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open Org document".into()),
        });

        self.picker_task = Some(cx.spawn(async move |this, cx| {
            let selected = receiver.await;
            if let Ok(Ok(Some(paths))) = selected
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| this.open(path, cx));
            }
        }));
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let path = match &self.state {
            PreviewLoadState::Loading { path } | PreviewLoadState::Failed { path, .. } => {
                Some(path.clone())
            }
            PreviewLoadState::Ready { document, .. } => Some(document.path.clone()),
            PreviewLoadState::Empty => None,
        };
        if let Some(path) = path {
            self.open(path, cx);
        }
    }

    fn body(&self) -> gpui::Div {
        let theme = current_theme();
        match &self.state {
            PreviewLoadState::Empty => centered_message(
                "ORG STUDIO",
                "Open an Org document from the File menu or press Command-O.",
            ),
            PreviewLoadState::Loading { path } => {
                centered_message("OPENING DOCUMENT", &path.display().to_string())
            }
            PreviewLoadState::Failed {
                path,
                message,
            } => {
                let error = format!("{}: {message}", path.display());
                if let Some((_, document)) = &self.last_ready {
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .bg(rgb(theme.background))
                        .child(
                            div()
                                .flex_none()
                                .px_6()
                                .py_3()
                                .bg(rgb(0xfff2f0))
                                .border_b_1()
                                .border_color(rgb(0xf2c8c2))
                                .text_size(px(13.0))
                                .text_color(rgb(0xa12b1f))
                                .child(format!("Could not open document. Showing the previous file. {error}")),
                        )
                        .child(render_document(document.clone(), self.list_state.clone()))
                } else {
                    centered_message("COULD NOT OPEN DOCUMENT", &error)
                }
            }
            PreviewLoadState::Ready { document, .. } => {
                render_document(document.clone(), self.list_state.clone())
            }
        }
    }

    fn window_title(&self) -> String {
        let path = match &self.state {
            PreviewLoadState::Loading { path } | PreviewLoadState::Failed { path, .. } => {
                Some(path)
            }
            PreviewLoadState::Ready { document, .. } => Some(&document.path),
            PreviewLoadState::Empty => None,
        };
        path.and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("Org Studio")
            .to_owned()
    }

    fn schedule_scroll_sample(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.on_next_frame(window, |this, window, cx| {
            let now = Instant::now();
            let Some(benchmark) = this.scroll_benchmark.as_mut() else {
                return;
            };
            if benchmark.warmup_remaining > 0 {
                benchmark.warmup_remaining -= 1;
                if benchmark.warmup_remaining % 60 == 0 {
                    eprintln!(
                        "org_preview_scroll_warmup remaining={} display={:?}",
                        benchmark.warmup_remaining,
                        window.display(cx).map(|display| display.id())
                    );
                }
                benchmark.last_frame = now;
                this.schedule_scroll_sample(window, cx);
                return;
            }
            if !benchmark.sampling_started {
                benchmark.sampling_started = true;
                benchmark.last_frame = now;
                this.list_state.scroll_by(px(benchmark.scroll_pixels));
                this.schedule_scroll_sample(window, cx);
                return;
            }
            benchmark
                .samples
                .push(now.duration_since(benchmark.last_frame));
            benchmark.last_frame = now;
            if benchmark.samples.len() >= benchmark.target_frames {
                let mut samples = benchmark.samples.clone();
                samples.sort_unstable();
                let percentile = |p: f64| {
                    let index = ((samples.len() - 1) as f64 * p).ceil() as usize;
                    samples[index].as_secs_f64() * 1000.0
                };
                let cadence_ms = percentile(0.50);
                let late_frame_threshold_ms = cadence_ms * 1.5;
                let late_frames = samples
                    .iter()
                    .filter(|sample| {
                        sample.as_secs_f64() * 1000.0 > late_frame_threshold_ms
                    })
                    .count();
                let estimated_missed_vsyncs: u64 = samples
                    .iter()
                    .map(|sample| {
                        let elapsed_ms = sample.as_secs_f64() * 1000.0;
                        (elapsed_ms / cadence_ms).round().max(1.0) as u64 - 1
                    })
                    .sum();
                let over_12_5 = samples
                    .iter()
                    .filter(|sample| sample.as_secs_f64() * 1000.0 > 12.5)
                    .count();
                let over_16_67 = samples
                    .iter()
                    .filter(|sample| sample.as_secs_f64() * 1000.0 > 16.67)
                    .count();
                eprintln!(
                    "org_preview_scroll frames={} pixels_per_frame={:.1} cadence_ms={:.3} cadence_hz={:.2} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3} max_ms={:.3} late_frames={} late_rate_pct={:.3} estimated_missed_vsyncs={} over_12_5={} over_16_67={}",
                    samples.len(),
                    benchmark.scroll_pixels,
                    cadence_ms,
                    1000.0 / cadence_ms,
                    percentile(0.50),
                    percentile(0.95),
                    percentile(0.99),
                    samples.last().unwrap().as_secs_f64() * 1000.0,
                    late_frames,
                    late_frames as f64 * 100.0 / samples.len() as f64,
                    estimated_missed_vsyncs,
                    over_12_5,
                    over_16_67,
                );
                crate::perf_tracing::report();
                this.scroll_benchmark = None;
                cx.quit();
            } else {
                this.list_state.scroll_by(px(benchmark.scroll_pixels));
                this.schedule_scroll_sample(window, cx);
            }
        });
    }
}

fn accept_generation(current: u64, completed: u64) -> bool {
    current == completed
}

impl Render for PreviewApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        profiling::scope!("PreviewApp::render");
        window.set_window_title(&self.window_title());
        if self.scroll_benchmark.is_some() {
            window.request_animation_frame();
        }
        if let PreviewLoadState::Ready { generation, .. } = &self.state
            && self.first_frame_scheduled != Some(*generation)
        {
            let generation = *generation;
            let opened_at = self.opened_at.unwrap_or_else(Instant::now);
            self.first_frame_scheduled = Some(generation);
            cx.on_next_frame(window, move |this, window, cx| {
                let elapsed = opened_at.elapsed();
                eprintln!(
                    "org_preview_first_readable_frame generation={} elapsed_ms={:.3}",
                    generation,
                    elapsed.as_secs_f64() * 1000.0
                );
                if generation == 1
                    && std::env::var_os("ORG_STUDIO_RELOAD_BENCH").is_some()
                    && let PreviewLoadState::Ready { document, .. } = &this.state
                {
                    this.open(document.path.clone(), cx);
                    return;
                }
                if this.scroll_benchmark.is_some() {
                    if let Some(benchmark) = this.scroll_benchmark.as_mut() {
                        benchmark.last_frame = Instant::now();
                    }
                    this.schedule_scroll_sample(window, cx);
                } else if std::env::var_os("ORG_STUDIO_EXIT_AFTER_FIRST_FRAME").is_some() {
                    cx.quit();
                }
            });
        }
        div()
            .size_full()
            .bg(rgb(current_theme().background))
            .text_color(rgb(current_theme().foreground))
            .font_family("Menlo")
            .text_size(px(14.0))
            .on_action(cx.listener(|this, _: &OpenDocument, _, cx| this.choose_file(cx)))
            .on_action(cx.listener(|this, _: &ReloadDocument, _, cx| this.reload(cx)))
            .child(self.body())
    }
}

pub fn load_document(path: PathBuf) -> Result<PreviewDocument, (PathBuf, String)> {
    load_document_profiled(path)
}

pub fn load_document_profiled(path: PathBuf) -> Result<PreviewDocument, (PathBuf, String)> {
    let total_started = Instant::now();
    let read_started = Instant::now();
    let bytes = std::fs::read(&path).map_err(|error| (path.clone(), error.to_string()))?;
    let read = read_started.elapsed();
    let byte_count = bytes.len() as u64;
    let rope_started = Instant::now();
    let snapshot =
        RopeSnapshot::from_utf8(bytes).map_err(|error| (path.clone(), error.to_string()))?;
    let rope = rope_started.elapsed();
    let text: SharedTextSnapshot = Arc::new(snapshot);
    let parse_started = Instant::now();
    let blocks = Arc::new(parse(text.as_ref()));
    let parse = parse_started.elapsed();
    let rows = Arc::new(build_preview_rows(text.as_ref(), &blocks));

    Ok(PreviewDocument {
        path,
        text,
        blocks,
        rows,
        inline_cache: Mutex::new(InlineCache::new(
            INLINE_CACHE_CAPACITY,
            INLINE_CACHE_MAX_BYTES,
        )),
        highlight_cache: Mutex::new(HighlightCache::new(
            HIGHLIGHT_CACHE_CAPACITY,
            HIGHLIGHT_CACHE_MAX_BYTES,
        )),
        metrics: LoadMetrics {
            bytes: byte_count,
            read,
            rope,
            parse,
            total: total_started.elapsed(),
        },
    })
}

fn centered_message(title: &str, detail: &str) -> gpui::Div {
    let theme = current_theme();
    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(rgb(theme.background))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            div()
                .text_size(px(20.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme.heading[0]))
                .child(title.to_owned()),
        )
        .child(
            div()
                .max_w(px(520.0))
                .text_size(px(14.0))
                .line_height(px(21.0))
                .text_color(rgb(theme.foreground_dim))
                .child(detail.to_owned()),
        )
}

fn render_document(
    document: Arc<PreviewDocument>,
    list_state: ListState,
) -> gpui::Div {
    let theme = current_theme();
    div()
        .size_full()
        .flex()
        .flex_col()
        .relative()
        .bg(rgb(theme.background))
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(px(50.0))
                .bg(rgb(theme.background_alt))
                .border_r_1()
                .border_color(rgb(theme.border)),
        )
        .child({
            let document = document.clone();
            list(list_state, move |index, _, cx| {
                let row = document.rows[index];
                let block = &document.blocks.nodes()[row.block_id as usize];
                div()
                    .w_full()
                    .min_h(px(24.0))
                    .when(index == 0, |element| element.pt_1())
                    .when(index + 1 == document.rows.len(), |element| element.pb_2())
                    .flex()
                    .items_start()
                    .child(
                        div()
                            .flex_none()
                            .w(px(50.0))
                            .pr_3()
                            .h(px(24.0))
                            .flex()
                            .items_center()
                            .justify_end()
                            .text_right()
                            .font_family("Menlo")
                            .text_size(px(10.0))
                            .text_color(rgb(theme.foreground_dim))
                            .child(if row.show_line_number {
                                row.source_line.to_string()
                            } else {
                                String::new()
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h(px(24.0))
                            .flex()
                            .items_center()
                            .pl_3()
                            .pr_8()
                            .child(
                                div()
                                    .w_full()
                                    .child(render_block(
                                        &document,
                                        row.block_id,
                                        row,
                                        block,
                                        cx,
                                    )),
                            ),
                    )
                    .into_any()
            })
            .flex_1()
            .w_full()
        })
}

fn render_block(
    document: &Arc<PreviewDocument>,
    block_id: BlockId,
    row: PreviewRow,
    block: &BlockNode,
    cx: &mut App,
) -> gpui::Div {
    let theme = current_theme();
    if row.blank {
        return div().h(px(24.0));
    }
    let text = document
        .text
        .copy_range(row.content)
        .trim_end_matches(['\r', '\n'])
        .to_owned();

    let element = match &block.kind {
        BlockKind::BlankLine => div().h(px(24.0)),
        BlockKind::Heading { level } => {
            let heading_index = (*level as usize).saturating_sub(1);
            let marker = format!(
                "{} ",
                theme.heading_bullets[heading_index % theme.heading_bullets.len()]
            );
            let size = match level {
                1 => 22.0,
                2 => 18.0,
                3 => 15.0,
                _ => 14.0,
            };
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(size))
                .line_height(px(24.0))
                .font_weight(if *level <= 2 {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(rgb(theme.heading[heading_index.min(3)]))
                .child(
                    div()
                        .flex_none()
                        .font_family("Menlo")
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme.heading[heading_index.min(3)]))
                        .child(marker),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(styled_inline(document.inline(block_id, &text, cx))),
                )
        }
        BlockKind::Paragraph => {
            let paragraph = div()
                .text_size(px(14.0))
                .line_height(px(22.0))
                .text_color(rgb(theme.foreground))
                .child(styled_inline(document.inline(block_id, &text, cx)));
            if row.continuation {
                paragraph
            } else {
                paragraph
            }
        }
        BlockKind::ListItem => div()
            .pl_1()
            .text_size(px(14.0))
            .line_height(px(22.0))
            .text_color(rgb(theme.foreground))
            .child(styled_inline(document.inline(block_id, &text, cx))),
        BlockKind::TableRow => div()
            .px_3()
            .py(px(5.0))
            .bg(rgb(theme.background_alt))
            .border_b_1()
            .border_color(rgb(theme.border))
            .font_family("Menlo")
            .text_size(px(13.0))
            .line_height(px(20.0))
            .text_color(rgb(theme.foreground))
            .child(text),
        BlockKind::SourceBlock { language } => {
            let marker = text.trim_start().to_ascii_lowercase();
            let is_boundary = marker.starts_with("#+begin_") || marker.starts_with("#+end_");
            let content = if is_boundary {
                StyledText::new(text.clone())
            } else if let Some(spans) = document.code_highlights(
                block_id,
                language.as_deref(),
                cx,
            ) {
                styled_code_row(
                    text.clone(),
                    row.content.start.0.saturating_sub(block.source.start.0) as usize,
                    &spans,
                )
            } else {
                StyledText::new(text.clone())
            };
            div()
                .min_h(px(24.0))
                .px_4()
                .py(px(2.0))
                .bg(rgb(if is_boundary {
                    theme.code_boundary_background
                } else {
                    theme.code_background
                }))
                .text_color(if is_boundary {
                    rgb(theme.code_boundary)
                } else {
                    rgb(theme.code_foreground)
                })
                .font_family("Menlo")
                .text_size(px(13.0))
                .line_height(px(19.0))
                .child(content)
        }
        BlockKind::ExampleBlock | BlockKind::Raw => div()
            .my_4()
            .p_5()
            .rounded_lg()
            .bg(rgb(theme.code_background))
            .font_family("Menlo")
            .text_size(px(13.0))
            .line_height(px(21.0))
            .text_color(rgb(theme.code_foreground))
            .child(text),
        BlockKind::QuoteBlock => div()
            .my_4()
            .pl_4()
            .pr_2()
            .py_2()
            .border_l_2()
            .border_color(rgb(theme.heading[1]))
            .text_color(rgb(theme.quote))
            .text_size(px(16.0))
            .line_height(px(25.0))
            .child(text),
        BlockKind::Drawer { name } => div()
            .my_2()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(theme.background_alt))
            .font_family("Menlo")
            .text_size(px(12.0))
            .line_height(px(19.0))
            .text_color(rgb(theme.meta))
            .child(format!("{name}: {text}")),
        BlockKind::Keyword | BlockKind::Comment => div()
            .py(px(3.0))
            .font_family("Menlo")
            .text_size(px(12.0))
            .line_height(px(18.0))
            .text_color(rgb(theme.meta))
            .child(text),
        BlockKind::HorizontalRule => div().my_5().h(px(1.0)).w_full().bg(rgb(theme.border)),
    };
    element
}

fn styled_code_row(
    text: String,
    row_start: usize,
    spans: &[CodeHighlightSpan],
) -> StyledText {
    let row_end = row_start + text.len();
    let highlights = spans.iter().filter_map(|span| {
        let start = span.start.max(row_start);
        let end = span.end.min(row_end);
        (start < end).then(|| {
            (
                start - row_start..end - row_start,
                code_highlight_style(span.kind),
            )
        })
    });
    StyledText::new(text).with_highlights(highlights)
}

fn code_highlight_style(kind: CodeHighlightKind) -> HighlightStyle {
    let theme = current_theme();
    let color = match kind {
        CodeHighlightKind::Attribute => theme.attribute,
        CodeHighlightKind::Boolean | CodeHighlightKind::Constant => theme.constant,
        CodeHighlightKind::Comment => theme.comment,
        CodeHighlightKind::Function => theme.function,
        CodeHighlightKind::Keyword => theme.keyword,
        CodeHighlightKind::Number => theme.number,
        CodeHighlightKind::Operator | CodeHighlightKind::Punctuation => theme.operator,
        CodeHighlightKind::Property | CodeHighlightKind::Variable => theme.variable,
        CodeHighlightKind::String => theme.string,
        CodeHighlightKind::Type => theme.type_name,
    };
    HighlightStyle {
        color: Some(rgb(color).into()),
        font_style: matches!(kind, CodeHighlightKind::Comment).then_some(FontStyle::Italic),
        ..Default::default()
    }
}

fn styled_inline(parsed: InlineText) -> StyledText {
    let theme = current_theme();
    let highlights = parsed.spans.into_iter().map(|span| {
        let style = match span.kind {
            InlineKind::Bold => HighlightStyle {
                font_weight: Some(FontWeight::BOLD),
                ..Default::default()
            },
            InlineKind::Italic => HighlightStyle {
                font_style: Some(FontStyle::Italic),
                ..Default::default()
            },
            InlineKind::Underline => HighlightStyle {
                color: Some(rgb(theme.link).into()),
                ..Default::default()
            },
            InlineKind::Strike => HighlightStyle {
                fade_out: Some(0.55),
                ..Default::default()
            },
            InlineKind::Code | InlineKind::Verbatim => HighlightStyle {
                color: Some(rgb(theme.inline_code).into()),
                background_color: Some(rgb(theme.inline_code_background).into()),
                ..Default::default()
            },
            InlineKind::Link => HighlightStyle {
                color: Some(rgb(theme.link).into()),
                font_weight: Some(FontWeight::MEDIUM),
                ..Default::default()
            },
            InlineKind::Timestamp => HighlightStyle {
                color: Some(rgb(theme.date).into()),
                ..Default::default()
            },
            InlineKind::Entity | InlineKind::Latex => HighlightStyle {
                color: Some(rgb(theme.function).into()),
                ..Default::default()
            },
        };
        (span.range, style)
    });
    StyledText::new(parsed.text).with_highlights(highlights)
}

#[cfg(test)]
mod tests {
    use super::{InlineCache, InlineText, MAX_SYNC_INLINE_BYTES, accept_generation};

    #[test]
    fn stale_generations_are_rejected() {
        assert!(accept_generation(7, 7));
        assert!(!accept_generation(8, 7));
    }

    #[test]
    fn inline_cache_never_exceeds_capacity() {
        let mut cache = InlineCache::new(4, 1024);
        for id in 0..32 {
            cache.get_or_insert(id, "*text*");
            assert!(cache.entries.len() <= 4);
        }
    }

    #[test]
    fn inline_cache_enforces_byte_capacity() {
        let mut cache = InlineCache::new(4, 1024);
        cache.insert(
            0,
            InlineText {
                text: "a".repeat(MAX_SYNC_INLINE_BYTES + 1),
                spans: Vec::new(),
            },
        );
        assert!(cache.entries.is_empty());
        assert_eq!(cache.bytes, 0);
    }
}
