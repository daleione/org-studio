use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use gpui::{
    App, Context, FontStyle, FontWeight, HighlightStyle, IntoElement, ListAlignment, ListState,
    PathPromptOptions, Render, StyledText, Task, Window, div, list, prelude::*, px, rgb,
};

use crate::{
    document::{ByteRange, RopeSnapshot, SharedTextSnapshot},
    org_syntax::{
        BlockArena, BlockId, BlockKind, BlockNode,
        inline::{InlineKind, InlineText, parse as parse_inline},
        parse,
    },
};

const INLINE_CACHE_CAPACITY: usize = 2048;
const INLINE_CACHE_MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_SYNC_INLINE_BYTES: usize = 64 * 1024;
const MAX_PARAGRAPH_ROW_BYTES: usize = 256;

pub struct PreviewDocument {
    pub path: PathBuf,
    pub text: SharedTextSnapshot,
    pub blocks: Arc<BlockArena>,
    rows: Arc<Vec<PreviewRow>>,
    inline_cache: Mutex<InlineCache>,
    pub metrics: LoadMetrics,
}

#[derive(Clone, Copy)]
struct PreviewRow {
    block_id: BlockId,
    content: ByteRange,
    continuation: bool,
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
}

enum PreviewLoadState {
    Empty,
    Loading {
        path: PathBuf,
        generation: u64,
    },
    Ready {
        generation: u64,
        document: Arc<PreviewDocument>,
    },
    Failed {
        path: PathBuf,
        generation: u64,
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
        self.state = PreviewLoadState::Loading {
            path: path.clone(),
            generation,
        };

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
                    Err((path, message)) => PreviewLoadState::Failed {
                        path,
                        generation,
                        message,
                    },
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
            PreviewLoadState::Loading { path, .. } | PreviewLoadState::Failed { path, .. } => {
                Some(path.clone())
            }
            PreviewLoadState::Ready { document, .. } => Some(document.path.clone()),
            PreviewLoadState::Empty => None,
        };
        if let Some(path) = path {
            self.open(path, cx);
        }
    }

    fn body(&self, cx: &mut Context<Self>) -> gpui::Div {
        match &self.state {
            PreviewLoadState::Empty => centered_message(
                "ORG STUDIO",
                "Open a local .org document or run:\ncargo run -- notes.org",
            )
            .child(toolbar_button(
                "OPEN",
                cx.listener(|this, _, _, cx| this.choose_file(cx)),
            )),
            PreviewLoadState::Loading { path, generation } => centered_message(
                "LOADING",
                &format!("{}\nrequest #{generation}", path.display()),
            ),
            PreviewLoadState::Failed {
                path,
                generation,
                message,
            } => {
                let error = format!("{}: {message} · request #{generation}", path.display());
                if let Some((ready_generation, document)) = &self.last_ready {
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex_none()
                                .px_8()
                                .py_2()
                                .bg(rgb(0xf1d8cf))
                                .text_color(rgb(0x8f321f))
                                .child(format!(
                                    "OPEN FAILED · showing previous document · {error}"
                                )),
                        )
                        .child(render_document(
                            document.clone(),
                            *ready_generation,
                            self.list_state.clone(),
                            toolbar_button(
                                "OPEN",
                                cx.listener(|this, _, _, cx| this.choose_file(cx)),
                            ),
                            toolbar_button("RELOAD", cx.listener(|this, _, _, cx| this.reload(cx))),
                        ))
                } else {
                    centered_message("FAILED TO OPEN", &error)
                }
            }
            PreviewLoadState::Ready {
                generation,
                document,
            } => render_document(
                document.clone(),
                *generation,
                self.list_state.clone(),
                toolbar_button("OPEN", cx.listener(|this, _, _, cx| this.choose_file(cx))),
                toolbar_button("RELOAD", cx.listener(|this, _, _, cx| this.reload(cx))),
            ),
        }
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
            .bg(rgb(0xf3efe4))
            .text_color(rgb(0x24231f))
            .font_family("Iowan Old Style")
            .child(self.body(cx))
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
        metrics: LoadMetrics {
            bytes: byte_count,
            read,
            rope,
            parse,
            total: total_started.elapsed(),
        },
    })
}

fn build_preview_rows(
    text: &dyn crate::document::TextSnapshot,
    blocks: &BlockArena,
) -> Vec<PreviewRow> {
    let mut rows = Vec::with_capacity(blocks.nodes().len());
    for (block_id, block) in blocks.nodes().iter().enumerate() {
        if !matches!(block.kind, BlockKind::Paragraph)
            || block.content.end.0 - block.content.start.0 <= MAX_PARAGRAPH_ROW_BYTES as u64
        {
            rows.push(PreviewRow {
                block_id: block_id as BlockId,
                content: block.content,
                continuation: false,
            });
            continue;
        }

        let source = text.copy_range(block.content);
        let mut start = 0;
        let mut continuation = false;
        while source.len() - start > MAX_PARAGRAPH_ROW_BYTES {
            let target = start + MAX_PARAGRAPH_ROW_BYTES;
            let mut end = target;
            while !source.is_char_boundary(end) {
                end -= 1;
            }
            let search_start = start + MAX_PARAGRAPH_ROW_BYTES / 2;
            if let Some(boundary) = source[search_start..end]
                .rfind(|character: char| character == '\n' || character == ' ' || character == '\t')
            {
                end = search_start
                    + boundary
                    + source[search_start + boundary..]
                        .chars()
                        .next()
                        .unwrap()
                        .len_utf8();
            }
            rows.push(PreviewRow {
                block_id: block_id as BlockId,
                content: ByteRange::new(
                    block.content.start.0 + start as u64,
                    block.content.start.0 + end as u64,
                ),
                continuation,
            });
            continuation = true;
            start = end;
        }
        if start < source.len() {
            rows.push(PreviewRow {
                block_id: block_id as BlockId,
                content: ByteRange::new(block.content.start.0 + start as u64, block.content.end.0),
                continuation,
            });
        }
    }
    rows
}

fn centered_message(title: &str, detail: &str) -> gpui::Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_4()
        .child(
            div()
                .text_size(px(12.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xb34a2f))
                .child(title.to_owned()),
        )
        .child(
            div()
                .max_w(px(640.0))
                .text_size(px(17.0))
                .text_color(rgb(0x666157))
                .child(detail.to_owned()),
        )
}

fn toolbar_button(
    label: &'static str,
    listener: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(label)
        .px_3()
        .py_1()
        .border_1()
        .border_color(rgb(0xb9af9d))
        .rounded_sm()
        .text_size(px(11.0))
        .font_weight(FontWeight::BOLD)
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0xe3dbc9)))
        .on_click(listener)
        .child(label.to_owned())
}

fn render_document(
    document: Arc<PreviewDocument>,
    generation: u64,
    list_state: ListState,
    open_button: impl IntoElement,
    reload_button: impl IntoElement,
) -> gpui::Div {
    let block_count = document.blocks.nodes().len();
    let row_count = document.rows.len();
    let title = document
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled.org")
        .to_owned();

    div()
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_none()
                .px_8()
                .py_4()
                .border_b_1()
                .border_color(rgb(0xd8d0bd))
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(open_button)
                        .child(reload_button),
                )
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::BOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(0x7b7468))
                        .child(format!(
                            "{block_count} blocks / {row_count} rows · {:.1} ms load · request #{generation} · read only",
                            document.metrics.total.as_secs_f64() * 1000.0
                        )),
                ),
        )
        .child({
            let document = document.clone();
            list(list_state, move |index, _, cx| {
                let row = document.rows[index];
                let block = &document.blocks.nodes()[row.block_id as usize];
                div()
                    .w_full()
                    .px_8()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(900.0))
                            .mx_auto()
                            .child(render_block(&document, index as BlockId, row, block, cx)),
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
    let text = document
        .text
        .copy_range(row.content)
        .trim_end_matches(['\r', '\n'])
        .to_owned();

    match &block.kind {
        BlockKind::Heading { level } => {
            let size = match level {
                1 => 34.0,
                2 => 27.0,
                3 => 22.0,
                _ => 18.0,
            };
            div()
                .mt(if *level == 1 { px(28.0) } else { px(20.0) })
                .mb_2()
                .text_size(px(size))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0x1e3d36))
                .child(styled_inline(document.inline(block_id, &text, cx)))
        }
        BlockKind::Paragraph => {
            let paragraph = div()
                .text_size(px(17.0))
                .line_height(px(28.0))
                .child(styled_inline(document.inline(block_id, &text, cx)));
            if row.continuation {
                paragraph
            } else {
                paragraph.py_2()
            }
        }
        BlockKind::ListItem => div()
            .pl_5()
            .py_1()
            .text_size(px(16.0))
            .child(styled_inline(document.inline(block_id, &text, cx))),
        BlockKind::TableRow => div()
            .px_4()
            .py_1()
            .bg(rgb(0xe8e1d2))
            .font_family("SFMono-Regular")
            .text_size(px(14.0))
            .child(text),
        BlockKind::SourceBlock { language } => div()
            .my_3()
            .p_4()
            .bg(rgb(0x252a27))
            .text_color(rgb(0xe9e2d2))
            .font_family("SFMono-Regular")
            .text_size(px(14.0))
            .child(
                language
                    .as_ref()
                    .map(|language| format!("{language}\n{text}"))
                    .unwrap_or(text),
            ),
        BlockKind::ExampleBlock | BlockKind::Raw => div()
            .my_3()
            .p_4()
            .bg(rgb(0xe8e1d2))
            .font_family("SFMono-Regular")
            .text_size(px(14.0))
            .child(text),
        BlockKind::QuoteBlock => div()
            .my_3()
            .pl_5()
            .py_3()
            .border_l_4()
            .border_color(rgb(0xb34a2f))
            .text_color(rgb(0x555046))
            .text_size(px(17.0))
            .child(text),
        BlockKind::Drawer { name } => div()
            .py_1()
            .text_size(px(13.0))
            .text_color(rgb(0x81796b))
            .child(format!("{name}: {text}")),
        BlockKind::Keyword | BlockKind::Comment => div()
            .py_1()
            .font_family("SFMono-Regular")
            .text_size(px(12.0))
            .text_color(rgb(0x918879))
            .child(text),
        BlockKind::HorizontalRule => div().my_5().h(px(1.0)).w_full().bg(rgb(0xcac1ae)),
    }
}

fn styled_inline(parsed: InlineText) -> StyledText {
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
                color: Some(rgb(0x315e53).into()),
                ..Default::default()
            },
            InlineKind::Strike => HighlightStyle {
                fade_out: Some(0.55),
                ..Default::default()
            },
            InlineKind::Code | InlineKind::Verbatim => HighlightStyle {
                color: Some(rgb(0x9a3f29).into()),
                background_color: Some(rgb(0xe8e1d2).into()),
                ..Default::default()
            },
            InlineKind::Link => HighlightStyle {
                color: Some(rgb(0x226b73).into()),
                font_weight: Some(FontWeight::MEDIUM),
                ..Default::default()
            },
            InlineKind::Timestamp => HighlightStyle {
                color: Some(rgb(0x8c5b24).into()),
                ..Default::default()
            },
            InlineKind::Entity | InlineKind::Latex => HighlightStyle {
                color: Some(rgb(0x66558a).into()),
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
