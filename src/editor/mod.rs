mod commands;
mod element;
mod folding;
mod input;
mod layout_map;
mod minimap;
mod minimap_media;
mod org_commands;
mod read_only;
mod syntax;

pub(crate) use read_only::{
    CommandDisposition, GeneratedCommand, GeneratedTextView, LineHighlights,
};
pub(crate) use syntax::EditorSyntaxService;

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, RenderImage,
    SharedString, Subscription, Task, Window, WrappedLine, actions, div, prelude::*, px, rgb,
};
use layout_map::EditorLayoutMap;
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    document::{
        ByteOffset, ByteRange, DocumentCommand, DocumentEvent, DocumentSession, DocumentSnapshot,
        EditOrigin, EditTransaction, HistoryOutcome, LineIndex, Revision, RevisionRange, Selection,
        TextEdit, TextSnapshot,
    },
    theme::current_theme,
};

pub use element::EditorElement;

actions!(
    semantic_editor,
    [
        Backspace,
        DeleteForward,
        MoveLeft,
        MoveRight,
        MoveWordLeft,
        MoveWordRight,
        MoveUp,
        MoveDown,
        SelectLeft,
        SelectRight,
        SelectWordLeft,
        SelectWordRight,
        SelectUp,
        SelectDown,
        MoveLineStart,
        MoveLineEnd,
        MoveDocumentStart,
        MoveDocumentEnd,
        SelectAll,
        Newline,
        InsertTab,
        ShiftTab,
        AlignTable,
        ToggleTodo,
        ToggleCheckbox,
        Undo,
        Redo,
        Copy,
        Cut,
        Paste,
        RunSourceBlock,
        ToggleInlineImagePreviews
    ]
);

#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = semantic_editor, no_json)]
pub struct RunSourceBlockAt {
    pub(crate) source_offset: ByteOffset,
}

#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = semantic_editor, no_json)]
pub(crate) struct ActivateReadOnlyLine {
    pub(crate) line: u64,
}

const LINE_HEIGHT: f32 = 22.0;
const EDITOR_FONT_FAMILY: &str = "JetBrains Mono";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some("SemanticEditor")),
        KeyBinding::new("delete", DeleteForward, Some("SemanticEditor")),
        KeyBinding::new("left", MoveLeft, Some("SemanticEditor")),
        KeyBinding::new("right", MoveRight, Some("SemanticEditor")),
        KeyBinding::new("alt-left", MoveWordLeft, Some("SemanticEditor")),
        KeyBinding::new("alt-right", MoveWordRight, Some("SemanticEditor")),
        KeyBinding::new("up", MoveUp, Some("SemanticEditor")),
        KeyBinding::new("down", MoveDown, Some("SemanticEditor")),
        KeyBinding::new("shift-left", SelectLeft, Some("SemanticEditor")),
        KeyBinding::new("shift-right", SelectRight, Some("SemanticEditor")),
        KeyBinding::new("alt-shift-left", SelectWordLeft, Some("SemanticEditor")),
        KeyBinding::new("alt-shift-right", SelectWordRight, Some("SemanticEditor")),
        KeyBinding::new("shift-up", SelectUp, Some("SemanticEditor")),
        KeyBinding::new("shift-down", SelectDown, Some("SemanticEditor")),
        KeyBinding::new("cmd-left", MoveLineStart, Some("SemanticEditor")),
        KeyBinding::new("cmd-right", MoveLineEnd, Some("SemanticEditor")),
        KeyBinding::new("cmd-up", MoveDocumentStart, Some("SemanticEditor")),
        KeyBinding::new("cmd-down", MoveDocumentEnd, Some("SemanticEditor")),
        KeyBinding::new("cmd-a", SelectAll, Some("SemanticEditor")),
        KeyBinding::new("enter", Newline, Some("SemanticEditor")),
        KeyBinding::new("tab", InsertTab, Some("SemanticEditor")),
        KeyBinding::new("shift-tab", ShiftTab, Some("SemanticEditor")),
        KeyBinding::new("ctrl-shift-a", AlignTable, Some("SemanticEditor")),
        KeyBinding::new("ctrl-shift-t", ToggleTodo, Some("SemanticEditor")),
        KeyBinding::new("ctrl-shift-x", ToggleCheckbox, Some("SemanticEditor")),
        KeyBinding::new("cmd-z", Undo, Some("SemanticEditor")),
        KeyBinding::new("cmd-shift-z", Redo, Some("SemanticEditor")),
        KeyBinding::new("cmd-c", Copy, Some("SemanticEditor")),
        KeyBinding::new("cmd-x", Cut, Some("SemanticEditor")),
        KeyBinding::new("cmd-v", Paste, Some("SemanticEditor")),
        KeyBinding::new(
            "ctrl-c ctrl-x ctrl-v",
            ToggleInlineImagePreviews,
            Some("SemanticEditor"),
        ),
    ]);
}

#[derive(Clone)]
pub(super) struct HitRow {
    pub(super) range: ByteRange,
    pub(super) line: LineIndex,
    pub(super) origin_y: Pixels,
    pub(super) visible_top: Pixels,
    pub(super) visible_bottom: Pixels,
    pub(super) text_origin_x: Pixels,
    pub(super) line_height: Pixels,
    pub(super) display: layout_map::DisplayLineText,
    pub(super) layout: Arc<WrappedLine>,
    pub(super) inline_image_preview: bool,
}

#[derive(Clone, Copy)]
pub(super) struct SourceRunButtonHit {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) source_offset: ByteOffset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceRunPhase {
    Running,
    Success,
    Failure,
}

impl SourceRunPhase {
    fn dismiss_after(self) -> Option<Duration> {
        match self {
            Self::Running => None,
            Self::Success => Some(Duration::from_millis(1_500)),
            Self::Failure => Some(Duration::from_millis(2_500)),
        }
    }
}

const SOURCE_RUN_MIN_RUNNING_DURATION: Duration = Duration::from_millis(450);

#[derive(Clone, Copy)]
struct SourceRunFeedback {
    source_offset: ByteOffset,
    phase: SourceRunPhase,
    started_at: Instant,
}

#[derive(Clone)]
struct InlineImageCacheEntry {
    image: Arc<RenderImage>,
    dimensions: (u32, u32),
    last_used: u64,
}

#[derive(Default)]
struct InlineImageCache {
    entries: HashMap<PathBuf, InlineImageCacheEntry>,
    refreshing: HashSet<PathBuf>,
    clock: u64,
    resource_generation: u64,
}

impl InlineImageCache {
    const MAX_ENTRIES: usize = 64;

    fn get(&mut self, path: &Path) -> Option<(Arc<RenderImage>, (u32, u32))> {
        self.clock = self.clock.wrapping_add(1);
        let entry = self.entries.get_mut(path)?;
        entry.last_used = self.clock;
        Some((entry.image.clone(), entry.dimensions))
    }

    fn refresh(&mut self, path: &Path) {
        self.resource_generation = self.resource_generation.wrapping_add(1);
        if self.entries.contains_key(path) {
            self.refreshing.insert(path.to_path_buf());
        }
    }

    fn accept(&mut self, path: &Path, image: Arc<RenderImage>) -> (Arc<RenderImage>, (u32, u32)) {
        self.clock = self.clock.wrapping_add(1);
        self.refreshing.remove(path);
        let size = image.size(0);
        let mut dimensions = (u32::from(size.width).max(1), u32::from(size.height).max(1));
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
        {
            dimensions.0 = (dimensions.0 as f32 / gpui::SMOOTH_SVG_SCALE_FACTOR).round() as u32;
            dimensions.1 = (dimensions.1 as f32 / gpui::SMOOTH_SVG_SCALE_FACTOR).round() as u32;
        }
        self.entries.insert(
            path.to_path_buf(),
            InlineImageCacheEntry {
                image: image.clone(),
                dimensions,
                last_used: self.clock,
            },
        );
        while self.entries.len() > Self::MAX_ENTRIES {
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(cached_path, _)| cached_path.as_path() != path)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(path, _)| path.clone())
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
        (image, dimensions)
    }

    fn fail(&mut self, path: &Path) -> bool {
        let was_refreshing = self.refreshing.remove(path);
        let had_cached_image = self.entries.remove(path).is_some();
        was_refreshing || had_cached_image
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.refreshing.clear();
    }
}

#[derive(Clone)]
pub(super) struct Composition {
    pub(super) original_range: ByteRange,
    pub(super) original_text: String,
    pub(super) before: Selection,
    pub(super) revision: crate::document::Revision,
}

#[derive(Clone)]
pub(super) struct PlatformRange {
    pub(super) bytes: ByteRange,
    pub(super) utf16: Range<usize>,
}

struct EditorFoldAnimation {
    revision: u64,
    changed_ranges: Arc<[Range<u64>]>,
    target_hidden_ranges: Arc<[Range<u64>]>,
    target_marker_lines: Arc<HashSet<u64>>,
    collapsing: bool,
    anchor_line: u64,
    anchor_viewport_y: f32,
    started_at: Option<Instant>,
    progress: f32,
}

impl EditorFoldAnimation {
    fn scale(&self) -> f32 {
        let eased = crate::fold_animation::ease_out_cubic(self.progress);
        if self.collapsing { 1.0 - eased } else { eased }
    }

    #[cfg(test)]
    fn changed_line_count(&self) -> u64 {
        self.changed_ranges
            .iter()
            .map(|range| range.end - range.start)
            .sum()
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ShapeKey {
    pub(super) generated_line: Option<u64>,
    pub(super) text: SharedString,
    pub(super) font_size_bits: u32,
    pub(super) wrap_width_bits: u32,
    pub(super) syntax_key: u8,
    pub(super) code_language: Option<Arc<str>>,
    pub(super) marked: Option<(usize, usize)>,
}

#[cfg(feature = "benchmarks")]
pub(super) enum FrameBenchmarkAction {
    Inactive,
    Continue,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SemanticEditorStatus {
    pub(crate) caret_offset: ByteOffset,
    pub(crate) caret_line: u64,
    pub(crate) caret_column: u64,
    pub(crate) visible_bottom_line: u64,
    pub(crate) total_lines: u64,
    pub(crate) reached_end: bool,
    pub(crate) characters: u64,
    pub(crate) bytes: u64,
}

#[cfg(feature = "benchmarks")]
struct EditorFrameBenchmark {
    warmup_remaining: usize,
    target_samples: usize,
    samples: Vec<Duration>,
    random_seek: bool,
    scroll_pixels: Option<f32>,
    bounce_scroll: bool,
    returning_to_top: bool,
    turn_pause_remaining: usize,
    step: usize,
}

#[cfg(feature = "benchmarks")]
impl EditorFrameBenchmark {
    fn from_environment() -> Option<Self> {
        let target_samples = std::env::var("ORG_STUDIO_EDITOR_BENCH_FRAMES")
            .ok()?
            .parse::<usize>()
            .ok()
            .filter(|samples| *samples > 0)?;
        let warmup_remaining = std::env::var("ORG_STUDIO_EDITOR_BENCH_WARMUP_FRAMES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(120);
        let scroll_pixels = std::env::var("ORG_STUDIO_EDITOR_BENCH_SCROLL_PIXELS")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|pixels| pixels.is_finite() && *pixels > 0.0);
        Some(Self {
            warmup_remaining,
            target_samples,
            samples: Vec::with_capacity(target_samples),
            random_seek: std::env::var_os("ORG_STUDIO_EDITOR_BENCH_RANDOM_SEEK").is_some(),
            scroll_pixels,
            bounce_scroll: std::env::var_os("ORG_STUDIO_EDITOR_BENCH_BOUNCE").is_some(),
            returning_to_top: false,
            turn_pause_remaining: 0,
            step: 0,
        })
    }

    fn record(&mut self, elapsed: Duration) -> bool {
        if self.warmup_remaining > 0 {
            self.warmup_remaining -= 1;
            if self.warmup_remaining == 0 {
                crate::perf_tracing::reset_samples();
            }
            return false;
        }
        self.samples.push(elapsed);
        self.samples.len() >= self.target_samples
    }

    fn report(&self, host_id: u64) {
        let mut samples = self.samples.clone();
        samples.sort_unstable();
        let percentile = |p: f64| {
            let index = ((samples.len() - 1) as f64 * p).ceil() as usize;
            samples[index].as_secs_f64() * 1_000.0
        };
        eprintln!(
            "org_editor_frame_cpu host_id={} mode={} scroll_pixels={:.3} samples={} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3}",
            host_id,
            if self.bounce_scroll {
                "scroll_bounce"
            } else if self.scroll_pixels.is_some() {
                "scroll"
            } else if self.random_seek {
                "random_seek"
            } else {
                "typing"
            },
            self.scroll_pixels.unwrap_or(0.0),
            samples.len(),
            percentile(0.50),
            percentile(0.95),
            percentile(0.99),
        );
    }
}

pub struct SemanticEditor {
    generated_highlights: Option<Vec<LineHighlights>>,
    activate_read_only_lines: bool,
    session: Entity<DocumentSession>,
    focus_handle: FocusHandle,
    selection: Selection,
    selection_revision: Revision,
    selection_utf16: Range<usize>,
    selection_utf16_reversed: bool,
    marked: Option<PlatformRange>,
    composition: Option<Composition>,
    content_font_size: crate::typography::ContentFontSize,
    display_map: EditorLayoutMap,
    folds: folding::EditorFoldState,
    fold_markers: Arc<HashSet<u64>>,
    fold_animation: Option<EditorFoldAnimation>,
    fold_animation_revision: u64,
    command_feedback: Option<SharedString>,
    minimap: minimap::EditorMinimapHost,
    syntax_service: Arc<syntax::EditorSyntaxService>,
    layout_anchor: Option<RevisionRange>,
    shape_cache: HashMap<ShapeKey, Arc<WrappedLine>>,
    scroll_y: f32,
    scroll_x: f32,
    vertical_goal_x: Option<f32>,
    viewport: Option<Bounds<Pixels>>,
    hit_rows: Arc<[HitRow]>,
    source_run_buttons: Arc<[SourceRunButtonHit]>,
    source_run_button_hovered: bool,
    source_run_feedback: Option<SourceRunFeedback>,
    source_run_feedback_generation: u64,
    source_run_feedback_task: Option<Task<()>>,
    inline_image_previews: bool,
    inline_image_preview_overrides: HashMap<u64, bool>,
    inline_image_cache: RefCell<InlineImageCache>,
    inline_image_line_dimensions: RefCell<HashMap<u64, (u64, u32, u32)>>,
    pending_reveal_caret: bool,
    is_selecting: bool,
    drag_position: Option<Point<Pixels>>,
    autoscroll_task: Option<Task<()>>,
    autofocus: bool,
    focus_lost_subscription: Option<Subscription>,
    #[cfg(feature = "benchmarks")]
    frame_benchmark: Option<EditorFrameBenchmark>,
    _session_subscription: Subscription,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct EditorMinimapWidthEvent(pub(crate) f32);

impl EventEmitter<EditorMinimapWidthEvent> for SemanticEditor {}

impl SemanticEditor {
    pub(super) fn animated_line_start_y(&self, line: u64) -> f32 {
        let base = self.display_map.line_start_y(line);
        let Some(animation) = self.fold_animation.as_ref() else {
            return base;
        };
        let compression = animation
            .changed_ranges
            .iter()
            .filter_map(|range| {
                let end = range.end.min(line);
                (range.start < end).then(|| {
                    self.display_map.line_start_y(end) - self.display_map.line_start_y(range.start)
                })
            })
            .sum::<f32>()
            * (1.0 - animation.scale());
        base - compression
    }

    pub(super) fn animated_line_height_px(&self, line: u64) -> f32 {
        let height = self.display_map.line_height_px(line);
        self.fold_animation.as_ref().map_or(height, |animation| {
            if animation
                .changed_ranges
                .iter()
                .any(|range| range.contains(&line))
            {
                height * animation.scale()
            } else {
                height
            }
        })
    }

    pub(super) fn animated_document_height(&self) -> f32 {
        self.animated_line_start_y(self.display_map.line_count())
    }

    #[cfg(test)]
    pub(super) fn animated_visible_line_count(&self) -> f32 {
        let base = self.display_map.visible_line_count() as f32;
        self.fold_animation.as_ref().map_or(base, |animation| {
            let changed = animation.changed_line_count() as f32;
            (base - changed * (1.0 - animation.scale())).max(0.0)
        })
    }

    pub(super) fn animated_line_at_y(&self, y: f32) -> u64 {
        let line_count = self.display_map.line_count();
        if line_count == 0 {
            return 0;
        }
        let total_height = self.animated_document_height();
        let target = y.max(0.0).min((total_height - 0.01).max(0.0));
        let (mut low, mut high) = (0, line_count);
        while low < high {
            let middle = low + (high - low) / 2;
            if self.animated_line_start_y(middle + 1) <= target {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low.min(line_count - 1)
    }

    pub(super) fn animated_visible_line_range(
        &self,
        snapshot: &DocumentSnapshot,
        scroll_y: f32,
        viewport_height: f32,
    ) -> Range<u64> {
        let first = self.animated_line_at_y(scroll_y).min(snapshot.len_lines());
        let last = self
            .animated_line_at_y(scroll_y.max(0.0) + viewport_height.max(0.0))
            .saturating_add(2)
            .min(snapshot.len_lines());
        first..last.max(first)
    }

    pub fn new(session: Entity<DocumentSession>, cx: &mut Context<Self>) -> Self {
        Self::new_with_autofocus(session, true, cx)
    }

    pub(crate) fn new_with_autofocus(
        session: Entity<DocumentSession>,
        autofocus: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_syntax_service(
            session,
            autofocus,
            Arc::new(syntax::EditorSyntaxService::default()),
            cx,
        )
    }

    pub(crate) fn new_with_syntax_service(
        session: Entity<DocumentSession>,
        autofocus: bool,
        syntax_service: Arc<syntax::EditorSyntaxService>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.subscribe(&session, |this, _, event: &DocumentEvent, cx| {
            if let DocumentEvent::Edited { delta, .. } = event {
                this.inline_image_preview_overrides =
                    std::mem::take(&mut this.inline_image_preview_overrides)
                        .into_iter()
                        .filter_map(|(offset, visible)| {
                            delta
                                .map_range(RevisionRange::new(
                                    delta.before,
                                    ByteRange::new(offset, offset),
                                ))
                                .ok()
                                .map(|range| (range.range.start.0, visible))
                        })
                        .collect();
                if this.selection_revision == delta.before {
                    let range = RevisionRange::new(delta.before, this.selection.range());
                    this.selection = delta.map_range(range).map_or_else(
                        |_| {
                            let start = delta.edits.first().map_or(this.selection.head(), |edit| {
                                ByteOffset(edit.old.start.0 + edit.new_len)
                            });
                            Selection::caret(start)
                        },
                        |mapped| {
                            if this.selection.anchor() <= this.selection.head() {
                                Selection::new(mapped.range.start, mapped.range.end)
                            } else {
                                Selection::new(mapped.range.end, mapped.range.start)
                            }
                        },
                    );
                    let snapshot = this.snapshot(cx);
                    this.selection = this.selection.clamp(&snapshot);
                    this.sync_selection_utf16(&snapshot);
                    this.selection_revision = delta.after;
                }
                let viewport_height = this
                    .viewport
                    .map_or(0.0, |viewport| f32::from(viewport.size.height));
                let was_at_end = this.viewport.is_some()
                    && this.scroll_y + viewport_height + 0.5 >= this.display_map.total_height();
                let anchor_line = this.display_map.line_at_y(this.scroll_y);
                let anchor_start = this.display_map.line_start_y(anchor_line);
                let anchor_fraction = (this.scroll_y - anchor_start)
                    / this.display_map.line_height_px(anchor_line).max(1.0);
                let mapped_anchor = this
                    .layout_anchor
                    .and_then(|anchor| delta.map_range(anchor).ok())
                    .map(|anchor| anchor.range.start);
                let snapshot = this.snapshot(cx);
                let previous_line_count = this.display_map.line_count();
                let first_line = delta
                    .edits
                    .iter()
                    .filter_map(|edit| {
                        snapshot
                            .line_index_at(ByteOffset(edit.old.start.0.min(snapshot.len_bytes())))
                            .ok()
                    })
                    .map(|line| line.0)
                    .min()
                    .unwrap_or(0);
                let last_line = delta
                    .edits
                    .iter()
                    .filter_map(|edit| {
                        let new_end = edit.old.start.0.saturating_add(edit.new_len);
                        snapshot
                            .line_index_at(ByteOffset(new_end.min(snapshot.len_bytes())))
                            .ok()
                    })
                    .map(|line| line.0)
                    .max()
                    .unwrap_or(first_line);
                let edited_inline_image_lines = this
                    .inline_image_line_dimensions
                    .borrow()
                    .keys()
                    .copied()
                    .filter(|line| (first_line..=last_line).contains(line))
                    .collect::<Vec<_>>();
                for line in edited_inline_image_lines {
                    this.display_map.invalidate_line_layout(line);
                }
                this.syntax_service.invalidate_from(
                    snapshot.document_id(),
                    snapshot.revision(),
                    first_line,
                );
                let wrap_width = this.display_map.wrap_width();
                if previous_line_count != snapshot.len_lines() && delta.edits.len() == 1 {
                    let new_count = last_line.saturating_sub(first_line).saturating_add(1);
                    let added_lines = snapshot.len_lines().saturating_sub(previous_line_count);
                    let removed_lines = previous_line_count.saturating_sub(snapshot.len_lines());
                    let old_count = new_count
                        .saturating_add(removed_lines)
                        .saturating_sub(added_lines)
                        .max(1);
                    this.display_map.splice_lines(
                        first_line..first_line.saturating_add(old_count),
                        new_count,
                        snapshot.len_lines(),
                    );
                } else {
                    this.display_map.configure(snapshot.len_lines(), wrap_width);
                }
                if delta.edits.len() != 1 {
                    this.display_map.invalidate_layout_from(first_line);
                }
                this.folds.apply_delta(delta);
                this.fold_animation = None;
                this.fold_animation_revision = this.fold_animation_revision.wrapping_add(1);
                let path = this.session.read(cx).path().to_path_buf();
                let projection = this.folds.projection(&path, &snapshot);
                this.fold_markers = Arc::new(projection.marker_lines);
                this.display_map.set_hidden_ranges(projection.hidden_ranges);
                let anchor_line = mapped_anchor
                    .and_then(|offset| snapshot.line_index_at(offset).ok())
                    .map_or(anchor_line, |line| line.0)
                    .min(snapshot.len_lines().saturating_sub(1));
                let anchored = this.display_map.line_start_y(anchor_line)
                    + anchor_fraction.clamp(0.0, 1.0)
                        * this.display_map.line_height_px(anchor_line);
                let max_scroll = (this.display_map.total_height() - viewport_height).max(0.0);
                this.scroll_y = if was_at_end {
                    max_scroll
                } else {
                    anchored.clamp(0.0, max_scroll)
                };
                this.layout_anchor = snapshot
                    .line_content_range(LineIndex(anchor_line))
                    .ok()
                    .map(|range| {
                        RevisionRange::new(
                            snapshot.revision(),
                            ByteRange::new(range.start.0, range.start.0),
                        )
                    });
                this.shape_cache.clear();
                this.hit_rows = Arc::from([]);
                this.source_run_buttons = Arc::from([]);
                this.source_run_button_hovered = false;
                this.inline_image_line_dimensions.borrow_mut().clear();
                this.map_source_run_feedback(delta);
                this.vertical_goal_x = None;
            } else if let DocumentEvent::ResourceChanged { path, .. } = event {
                this.refresh_inline_image(path, cx);
            } else if matches!(event, DocumentEvent::Reloaded { .. }) {
                this.display_map.invalidate_layout();
            } else if matches!(event, DocumentEvent::PathChanged { .. }) {
                this.syntax_service.reset();
                this.folds = folding::EditorFoldState::default();
                this.fold_markers = Arc::new(HashSet::new());
                this.fold_animation = None;
                this.fold_animation_revision = this.fold_animation_revision.wrapping_add(1);
                this.display_map.set_hidden_ranges(Vec::new());
                this.shape_cache.clear();
                this.source_run_buttons = Arc::from([]);
                this.source_run_button_hovered = false;
                this.inline_image_preview_overrides.clear();
                this.inline_image_line_dimensions.borrow_mut().clear();
                this.inline_image_cache.borrow_mut().clear();
                this.clear_source_run_feedback();
                this.minimap.invalidate_raster();
            }
            if matches!(event, DocumentEvent::Reloaded { .. }) {
                this.syntax_service.reset();
                let snapshot = this.snapshot(cx);
                this.selection = this.selection.clamp(&snapshot);
                this.selection_revision = snapshot.revision();
                this.sync_selection_utf16(&snapshot);
                this.marked = None;
                this.composition = None;
                this.shape_cache.clear();
                this.hit_rows = Arc::from([]);
                this.source_run_buttons = Arc::from([]);
                this.source_run_button_hovered = false;
                this.inline_image_preview_overrides.clear();
                this.inline_image_line_dimensions.borrow_mut().clear();
                this.inline_image_cache.borrow_mut().clear();
                this.clear_source_run_feedback();
                this.pending_reveal_caret = false;
                this.scroll_y = 0.0;
                this.minimap.note_viewport_changed();
                this.scroll_x = 0.0;
                this.layout_anchor = None;
                this.folds = folding::EditorFoldState::default();
                this.fold_markers = Arc::new(HashSet::new());
                this.fold_animation = None;
                this.fold_animation_revision = this.fold_animation_revision.wrapping_add(1);
                this.display_map.set_hidden_ranges(Vec::new());
            }
            cx.notify();
        });
        let initial_snapshot = session.read(cx).snapshot();
        let mut display_map = EditorLayoutMap::default();
        display_map.configure(initial_snapshot.len_lines(), 1.0);
        Self {
            session,
            generated_highlights: None,
            activate_read_only_lines: false,
            focus_handle: cx.focus_handle(),
            selection: Selection::default(),
            selection_revision: initial_snapshot.revision(),
            selection_utf16: 0..0,
            selection_utf16_reversed: false,
            marked: None,
            composition: None,
            content_font_size: crate::typography::ContentFontSize::default(),
            display_map,
            folds: folding::EditorFoldState::default(),
            fold_markers: Arc::new(HashSet::new()),
            fold_animation: None,
            fold_animation_revision: 0,
            command_feedback: None,
            minimap: minimap::EditorMinimapHost::default(),
            syntax_service,
            layout_anchor: None,
            shape_cache: HashMap::with_capacity(128),
            scroll_y: 0.0,
            scroll_x: 0.0,
            vertical_goal_x: None,
            viewport: None,
            hit_rows: Arc::from([]),
            source_run_buttons: Arc::from([]),
            source_run_button_hovered: false,
            source_run_feedback: None,
            source_run_feedback_generation: 0,
            source_run_feedback_task: None,
            inline_image_previews: true,
            inline_image_preview_overrides: HashMap::new(),
            inline_image_cache: RefCell::new(InlineImageCache::default()),
            inline_image_line_dimensions: RefCell::new(HashMap::new()),
            pending_reveal_caret: false,
            is_selecting: false,
            drag_position: None,
            autoscroll_task: None,
            autofocus,
            focus_lost_subscription: None,
            #[cfg(feature = "benchmarks")]
            frame_benchmark: EditorFrameBenchmark::from_environment(),
            _session_subscription: subscription,
        }
    }

    pub fn session(&self) -> &Entity<DocumentSession> {
        &self.session
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    pub(crate) fn request_focus(&mut self, cx: &mut Context<Self>) {
        self.autofocus = true;
        cx.notify();
    }

    pub(crate) fn set_soft_wrap(&mut self, soft_wrap: bool, cx: &mut Context<Self>) {
        if self.display_map.set_soft_wrap(soft_wrap) {
            if soft_wrap {
                self.scroll_x = 0.0;
            }
            self.shape_cache.clear();
            cx.notify();
        }
    }

    pub(crate) fn content_font_size(&self) -> crate::typography::ContentFontSize {
        self.content_font_size
    }

    pub(super) fn font_size_px(&self) -> f32 {
        self.content_font_size.get() as f32
    }

    pub(super) fn base_line_height(&self) -> f32 {
        LINE_HEIGHT * self.content_font_size.scale()
    }

    pub(crate) fn set_content_font_size(
        &mut self,
        font_size: crate::typography::ContentFontSize,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.content_font_size == font_size {
            return false;
        }
        self.finish_fold_animation();
        let viewport_height = self
            .viewport
            .map_or(0.0, |viewport| f32::from(viewport.size.height));
        let was_at_end = self.viewport.is_some()
            && self.scroll_y + viewport_height + 0.5 >= self.display_map.total_height();
        let anchor_line = self.display_map.line_at_y(self.scroll_y);
        let anchor_start = self.display_map.line_start_y(anchor_line);
        let anchor_fraction =
            (self.scroll_y - anchor_start) / self.display_map.line_height_px(anchor_line).max(1.0);

        self.content_font_size = font_size;
        self.display_map
            .set_base_line_height(self.base_line_height());
        let anchored = self.display_map.line_start_y(anchor_line)
            + anchor_fraction.clamp(0.0, 1.0) * self.display_map.line_height_px(anchor_line);
        let max_scroll = (self.display_map.total_height() - viewport_height).max(0.0);
        self.scroll_y = if was_at_end {
            max_scroll
        } else {
            anchored.clamp(0.0, max_scroll)
        };
        self.shape_cache.clear();
        self.hit_rows = Arc::from([]);
        self.source_run_buttons = Arc::from([]);
        self.source_run_button_hovered = false;
        self.vertical_goal_x = None;
        self.minimap.note_viewport_changed();
        cx.notify();
        true
    }

    pub(crate) fn set_minimap(
        &mut self,
        visible: bool,
        width: Option<u16>,
        cx: &mut Context<Self>,
    ) {
        self.minimap.visible = visible;
        self.minimap.width = width.map_or(minimap::DEFAULT_WIDTH, |width| {
            (width as f32).clamp(minimap::MIN_WIDTH, minimap::MAX_WIDTH)
        });
        if !visible {
            self.minimap.drag = None;
            self.minimap.resizing = None;
            self.minimap.bounds = None;
        }
        self.shape_cache.clear();
        cx.notify();
    }

    pub fn set_minimap_search_marks(&mut self, marks: Arc<[ByteRange]>, cx: &mut Context<Self>) {
        self.minimap.search_marks = marks;
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn soft_wrap(&self) -> bool {
        self.display_map.soft_wrap()
    }

    #[cfg(test)]
    pub(crate) fn autofocus_pending(&self) -> bool {
        self.autofocus
    }

    #[cfg(test)]
    pub(crate) fn minimap_width(&self) -> f32 {
        self.minimap.width
    }

    pub fn set_selection(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.finish_composition(cx);
        self.vertical_goal_x = None;
        let snapshot = self.snapshot(cx);
        self.selection = selection.clamp(&snapshot);
        self.selection_revision = snapshot.revision();
        self.sync_selection_utf16(&snapshot);
        self.reveal_caret(&snapshot);
        cx.notify();
    }

    pub(crate) fn clear_source_run_feedback(&mut self) {
        self.source_run_feedback_generation = self.source_run_feedback_generation.wrapping_add(1);
        self.source_run_feedback_task = None;
        self.source_run_feedback = None;
    }

    fn map_source_run_feedback(&mut self, delta: &crate::document::RevisionDelta) {
        let Some(mut feedback) = self.source_run_feedback else {
            return;
        };
        let position = RevisionRange::new(
            delta.before,
            ByteRange::new(feedback.source_offset.0, feedback.source_offset.0),
        );
        match delta.map_range(position) {
            Ok(mapped) => {
                feedback.source_offset = mapped.range.start;
                self.source_run_feedback = Some(feedback);
            }
            Err(_) => self.clear_source_run_feedback(),
        }
    }

    pub(crate) fn show_source_run_feedback(
        &mut self,
        source_offset: ByteOffset,
        phase: SourceRunPhase,
        cx: &mut Context<Self>,
    ) {
        self.clear_source_run_feedback();
        self.source_run_feedback = Some(SourceRunFeedback {
            source_offset,
            phase,
            started_at: Instant::now(),
        });
        let duration = phase.dismiss_after();
        if let Some(duration) = duration {
            let generation = self.source_run_feedback_generation;
            let delay = cx.background_executor().timer(duration);
            self.source_run_feedback_task = Some(cx.spawn(async move |this, cx| {
                delay.await;
                let _ = this.update(cx, |this, cx| {
                    if this.source_run_feedback_generation != generation {
                        return;
                    }
                    this.clear_source_run_feedback();
                    cx.notify();
                });
            }));
        }
        cx.notify();
    }

    pub(crate) fn finish_source_run_feedback(
        &mut self,
        phase: SourceRunPhase,
        cx: &mut Context<Self>,
    ) {
        let Some(feedback) = self.source_run_feedback else {
            return;
        };
        self.finish_source_run_feedback_at(feedback.source_offset, phase, cx);
    }

    pub(crate) fn finish_source_run_feedback_at(
        &mut self,
        source_offset: ByteOffset,
        phase: SourceRunPhase,
        cx: &mut Context<Self>,
    ) {
        let transition_delay = self
            .source_run_feedback
            .filter(|feedback| {
                feedback.source_offset == source_offset && feedback.phase == SourceRunPhase::Running
            })
            .map(|feedback| {
                SOURCE_RUN_MIN_RUNNING_DURATION.saturating_sub(feedback.started_at.elapsed())
            })
            .unwrap_or_default();
        if transition_delay.is_zero() {
            self.show_source_run_feedback(source_offset, phase, cx);
            return;
        }

        self.source_run_feedback_generation = self.source_run_feedback_generation.wrapping_add(1);
        self.source_run_feedback_task = None;
        let generation = self.source_run_feedback_generation;
        let delay = cx.background_executor().timer(transition_delay);
        self.source_run_feedback_task = Some(cx.spawn(async move |this, cx| {
            delay.await;
            let _ = this.update(cx, |this, cx| {
                let still_running = this.source_run_feedback.is_some_and(|feedback| {
                    feedback.source_offset == source_offset
                        && feedback.phase == SourceRunPhase::Running
                });
                if this.source_run_feedback_generation != generation || !still_running {
                    return;
                }
                this.show_source_run_feedback(source_offset, phase, cx);
            });
        }));
    }

    fn sync_selection_revision(&mut self, cx: &App) {
        self.selection_revision = self.session.read(cx).revision();
    }

    pub fn snapshot(&self, cx: &App) -> DocumentSnapshot {
        self.session.read(cx).snapshot()
    }

    fn previews_inline_image_at(&self, line_start: ByteOffset) -> bool {
        self.inline_image_preview_overrides
            .get(&line_start.0)
            .copied()
            .unwrap_or(self.inline_image_previews)
    }

    pub(crate) fn refresh_inline_image(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.inline_image_cache.borrow_mut().refresh(path);
        cx.notify();
    }

    fn cached_inline_image_render(&self, path: &Path) -> Option<(Arc<RenderImage>, (u32, u32))> {
        self.inline_image_cache.borrow_mut().get(path)
    }

    fn accept_inline_image_render(
        &self,
        path: &Path,
        image: Arc<RenderImage>,
    ) -> (Arc<RenderImage>, (u32, u32)) {
        self.inline_image_cache.borrow_mut().accept(path, image)
    }

    fn fail_inline_image_render(&self, path: &Path) -> bool {
        self.inline_image_cache.borrow_mut().fail(path)
    }

    #[cfg(test)]
    pub(crate) fn has_active_composition(&self) -> bool {
        self.composition.is_some()
    }

    #[cfg(test)]
    pub(crate) fn inline_image_resource_generation(&self) -> u64 {
        self.inline_image_cache.borrow().resource_generation
    }

    pub(crate) fn status(&self, cx: &App) -> SemanticEditorStatus {
        let snapshot = self.snapshot(cx);
        let (line, column) = snapshot
            .line_and_column_at(self.selection.head())
            .unwrap_or_default();
        let total_lines = snapshot.len_lines();
        let viewport_height = self
            .viewport
            .map_or(0.0, |bounds| f32::from(bounds.size.height));
        let visible_bottom_line = if viewport_height > 0.0 {
            self.animated_line_at_y((self.scroll_y + viewport_height - 0.5).max(0.0))
                .saturating_add(1)
        } else {
            0
        }
        .min(total_lines);
        let document_height = self.animated_document_height();
        let reached_end =
            viewport_height > 0.0 && self.scroll_y + viewport_height + 0.5 >= document_height;
        SemanticEditorStatus {
            caret_offset: self.selection.head(),
            caret_line: line.0 + 1,
            caret_column: column + 1,
            visible_bottom_line,
            total_lines,
            reached_end,
            characters: snapshot.len_chars(),
            bytes: snapshot.len_bytes(),
        }
    }

    #[cfg(feature = "benchmarks")]
    pub(super) fn record_frame_benchmark(
        &mut self,
        elapsed: Duration,
        cx: &mut Context<Self>,
    ) -> FrameBenchmarkAction {
        let host_id = self.minimap.telemetry.host_id();
        let current_scroll_y = self.scroll_y;
        let viewport_height = self
            .viewport
            .map_or(0.0, |viewport| f32::from(viewport.size.height));
        let current_max_scroll = (self.animated_document_height() - viewport_height).max(0.0);
        let Some(benchmark) = self.frame_benchmark.as_mut() else {
            return FrameBenchmarkAction::Inactive;
        };
        let random_seek = benchmark.random_seek;
        let scroll_pixels = benchmark.scroll_pixels;
        let bounce_scroll = benchmark.bounce_scroll;
        let step = benchmark.step;
        let warmup_completes = benchmark.warmup_remaining == 1;
        benchmark.step = benchmark.step.wrapping_add(1);
        if benchmark.bounce_scroll && benchmark.step.is_multiple_of(120) {
            eprintln!(
                "org_editor_scroll_bounce_progress step={} scroll_y={:.3} max_scroll={:.3} returning_to_top={} pause_remaining={}",
                benchmark.step,
                current_scroll_y,
                current_max_scroll,
                benchmark.returning_to_top,
                benchmark.turn_pause_remaining,
            );
        }
        if benchmark.record(elapsed) {
            benchmark.report(host_id);
            if benchmark.bounce_scroll {
                eprintln!(
                    "org_editor_scroll_bounce_end returning_to_top={} scroll_y={:.3} at_top={}",
                    benchmark.returning_to_top,
                    current_scroll_y,
                    current_scroll_y <= 0.5,
                );
            }
            self.minimap.telemetry.report_summary();
            crate::perf_tracing::report();
            self.frame_benchmark = None;
            return FrameBenchmarkAction::Complete;
        }
        if warmup_completes {
            self.minimap.telemetry.reset_samples();
        }
        if let Some(scroll_pixels) = scroll_pixels {
            let scroll_delta = if bounce_scroll {
                if !benchmark.returning_to_top && current_scroll_y + 0.5 >= current_max_scroll {
                    benchmark.returning_to_top = true;
                    benchmark.turn_pause_remaining = 12;
                }
                if benchmark.turn_pause_remaining > 0 {
                    benchmark.turn_pause_remaining -= 1;
                    None
                } else if benchmark.returning_to_top {
                    Some(scroll_pixels)
                } else {
                    Some(-scroll_pixels)
                }
            } else {
                Some(-scroll_pixels)
            };
            if let Some(scroll_delta) = scroll_delta {
                self.scroll(0.0, scroll_delta, cx);
            }
        } else if random_seek {
            let snapshot = self.snapshot(cx);
            let line_count = snapshot.len_lines().max(1);
            let line = (step as u64).wrapping_mul(104_729) % line_count;
            if let Ok(range) = snapshot.line_content_range(LineIndex(line)) {
                self.set_selection(Selection::caret(range.start), cx);
            }
        } else {
            self.replace_selection("a", EditOrigin::Typing, cx);
        }
        FrameBenchmarkAction::Continue
    }
}
fn word_boundary(snapshot: &DocumentSnapshot, offset: ByteOffset, forward: bool) -> ByteOffset {
    let Ok(line) = snapshot.line_index_at(offset) else {
        return offset;
    };
    let Ok(range) = snapshot.line_content_range(line) else {
        return offset;
    };
    if forward && offset >= range.end {
        return snapshot.next_grapheme_boundary(offset).unwrap_or(offset);
    }
    if !forward && offset <= range.start {
        return snapshot
            .previous_grapheme_boundary(offset)
            .unwrap_or(offset);
    }
    let mut radius = 4 * 1024_u64;
    loop {
        let mut start = offset.0.saturating_sub(radius).max(range.start.0);
        while start > range.start.0 && !snapshot.is_char_boundary(ByteOffset(start)) {
            start -= 1;
        }
        let mut end = offset.0.saturating_add(radius).min(range.end.0);
        while end > offset.0 && !snapshot.is_char_boundary(ByteOffset(end)) {
            end -= 1;
        }
        let text = snapshot.copy_range(ByteRange::new(start, end));
        let local = (offset.0 - start) as usize;
        if forward {
            let candidate = text
                .split_word_bound_indices()
                .map(|(segment_start, segment)| segment_start + segment.len())
                .find(|boundary| *boundary > local)
                .unwrap_or(text.len());
            if candidate < text.len() || end == range.end.0 {
                return ByteOffset(start + candidate as u64);
            }
        } else {
            let candidate = text
                .split_word_bound_indices()
                .map(|(segment_start, _)| segment_start)
                .take_while(|boundary| *boundary < local)
                .last()
                .unwrap_or(0);
            if candidate > 0 || start == range.start.0 {
                return ByteOffset(start + candidate as u64);
            }
        }
        radius = radius.saturating_mul(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Frame, RgbaImage};
    use smallvec::SmallVec;
    use std::path::PathBuf;

    fn render_image(width: u32, height: u32) -> Arc<RenderImage> {
        Arc::new(RenderImage::new(SmallVec::from_elem(
            Frame::new(RgbaImage::new(width, height)),
            1,
        )))
    }

    #[test]
    fn inline_image_cache_is_bounded_and_failed_refresh_drops_stale_content() {
        let mut cache = InlineImageCache::default();
        for index in 0..InlineImageCache::MAX_ENTRIES + 8 {
            cache.accept(
                Path::new(&format!("image-{index}.png")),
                render_image(10, 10),
            );
        }
        assert_eq!(cache.entries.len(), InlineImageCache::MAX_ENTRIES);

        let path = Path::new("image-current.png");
        cache.accept(path, render_image(20, 10));
        cache.refresh(path);
        assert!(cache.fail(path));
        assert!(!cache.fail(path));
        assert!(!cache.refreshing.contains(path));
        assert!(cache.get(path).is_none());
    }

    #[test]
    fn inline_svg_cache_uses_logical_instead_of_supersampled_dimensions() {
        let mut cache = InlineImageCache::default();
        let (_, dimensions) = cache.accept(
            Path::new("diagram.svg"),
            render_image(
                (120.0 * gpui::SMOOTH_SVG_SCALE_FACTOR) as u32,
                (80.0 * gpui::SMOOTH_SVG_SCALE_FACTOR) as u32,
            ),
        );
        assert_eq!(dimensions, (120, 80));
    }

    #[test]
    fn source_run_result_feedback_is_transient() {
        assert_eq!(SOURCE_RUN_MIN_RUNNING_DURATION, Duration::from_millis(450));
        assert_eq!(SourceRunPhase::Running.dismiss_after(), None);
        assert_eq!(
            SourceRunPhase::Success.dismiss_after(),
            Some(Duration::from_millis(1_500))
        );
        assert_eq!(
            SourceRunPhase::Failure.dismiss_after(),
            Some(Duration::from_millis(2_500))
        );
    }

    #[test]
    fn local_fold_animation_eases_between_expanded_and_collapsed_scales() {
        let mut animation = EditorFoldAnimation {
            revision: 1,
            changed_ranges: Arc::from(std::iter::once(1..3).collect::<Vec<_>>()),
            target_hidden_ranges: Arc::from(std::iter::once(1..3).collect::<Vec<_>>()),
            target_marker_lines: Arc::new(HashSet::from([0])),
            collapsing: true,
            anchor_line: 0,
            anchor_viewport_y: 0.0,
            started_at: None,
            progress: 0.0,
        };
        assert_eq!(animation.scale(), 1.0);
        animation.progress = 1.0;
        assert_eq!(animation.scale(), 0.0);
        animation.collapsing = false;
        assert_eq!(animation.scale(), 1.0);
        animation.progress = 0.0;
        assert_eq!(animation.scale(), 0.0);
    }

    #[gpui::test]
    fn content_font_size_updates_editor_typography_and_layout_baseline(
        cx: &mut gpui::TestAppContext,
    ) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("font-size.org"), b"one\ntwo\n".to_vec())
                .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session, cx));
        let minimap_line_height = cx.read(|cx| {
            editor
                .read(cx)
                .minimap
                .line_height(crate::minimap::Density::Compact)
        });

        cx.read(|cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.content_font_size().get(), 15);
            assert_eq!(editor.font_size_px(), 15.0);
            assert_eq!(editor.base_line_height(), 22.0);
            assert_eq!(editor.display_map.base_line_height(), 22.0);
        });

        editor.update(cx, |editor, cx| {
            assert!(editor.set_content_font_size(crate::typography::ContentFontSize::new(30), cx,));
            assert!(
                !editor.set_content_font_size(crate::typography::ContentFontSize::new(30), cx,)
            );
        });
        cx.read(|cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.content_font_size().get(), 30);
            assert_eq!(editor.font_size_px(), 30.0);
            assert_eq!(editor.base_line_height(), 44.0);
            assert_eq!(editor.display_map.base_line_height(), 44.0);
        });

        for size in [5, 96] {
            editor.update(cx, |editor, cx| {
                assert!(
                    editor
                        .set_content_font_size(crate::typography::ContentFontSize::new(size), cx,)
                );
            });
            cx.read(|cx| {
                let editor = editor.read(cx);
                let expected_line_height =
                    22.0 * size as f32 / crate::typography::ContentFontSize::DEFAULT as f32;
                assert_eq!(editor.content_font_size().get(), size);
                assert_eq!(editor.font_size_px(), size as f32);
                assert!((editor.base_line_height() - expected_line_height).abs() < 0.001);
                assert!(
                    (editor.display_map.base_line_height() - expected_line_height).abs() < 0.001
                );
                assert_eq!(
                    editor.minimap.line_height(crate::minimap::Density::Compact),
                    minimap_line_height
                );
            });
        }
    }

    #[gpui::test]
    fn content_font_size_preserves_editor_top_anchor_and_bottom_pin(cx: &mut gpui::TestAppContext) {
        let source = (0..100)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("font-anchor.org"), source.into_bytes())
                .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session, cx));

        editor.update(cx, |editor, cx| {
            editor.viewport = Some(Bounds {
                origin: gpui::point(px(0.0), px(0.0)),
                size: gpui::size(px(800.0), px(220.0)),
            });
            editor.scroll_y = editor.display_map.line_start_y(40) + 5.5;
            assert!(editor.set_content_font_size(crate::typography::ContentFontSize::new(30), cx,));
            assert_eq!(editor.display_map.line_at_y(editor.scroll_y), 40);
            assert!((editor.scroll_y - editor.display_map.line_start_y(40) - 11.0).abs() < 0.001);

            editor.scroll_y = (editor.display_map.total_height() - 220.0).max(0.0);
            assert!(editor.set_content_font_size(crate::typography::ContentFontSize::new(5), cx,));
            let expected_bottom = (editor.display_map.total_height() - 220.0).max(0.0);
            assert!((editor.scroll_y - expected_bottom).abs() < 0.001);
        });
    }

    #[gpui::test]
    fn semantic_editor_edits_unicode_and_restores_selection_on_undo(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("test.org"), "a中".as_bytes().to_vec())
                .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session.clone(), cx));
        editor.update(cx, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(1)), cx);
            editor.replace_selection("🙂", EditOrigin::Typing, cx);
        });
        assert_eq!(
            cx.read(|cx| {
                let snapshot = session.read(cx).snapshot();
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
            }),
            "a🙂中"
        );
        editor.update(cx, |editor, cx| {
            editor.finish_composition(cx);
            if let HistoryOutcome::Applied(selection) =
                session.update(cx, |session, cx| session.undo(cx)).unwrap()
            {
                editor.selection = selection;
            }
        });
        assert_eq!(
            cx.read(|cx| editor.read(cx).selection()),
            Selection::caret(ByteOffset(1))
        );
    }

    #[gpui::test]
    fn semantic_status_uses_live_caret_and_viewport_coordinates(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                PathBuf::from("test.org"),
                "first\n你😀z\n".as_bytes().to_vec(),
            )
            .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session, cx));
        editor.update(cx, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(13)), cx);
            editor.scroll_y = LINE_HEIGHT;
            editor.viewport = Some(Bounds {
                origin: gpui::point(px(0.0), px(0.0)),
                size: gpui::size(px(800.0), px(LINE_HEIGHT)),
            });
        });

        let status = cx.read(|cx| editor.read(cx).status(cx));
        assert_eq!(status.caret_line, 2);
        assert_eq!(status.caret_column, 3);
        assert_eq!(status.visible_bottom_line, 2);
        assert_eq!(status.total_lines, 3);
        assert!(!status.reached_end);
        assert_eq!(status.characters, 10);
        assert_eq!(status.bytes, 15);

        editor.update(cx, |editor, _| editor.scroll_y = LINE_HEIGHT * 2.0);
        let status = cx.read(|cx| editor.read(cx).status(cx));
        assert_eq!(status.visible_bottom_line, 3);
        assert!(status.reached_end);
    }

    #[gpui::test]
    fn edits_above_viewport_preserve_the_same_source_anchor(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                PathBuf::from("test.org"),
                b"first\nsecond\nthird\n".to_vec(),
            )
            .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session.clone(), cx));
        editor.update(cx, |editor, cx| {
            let snapshot = session.read(cx).snapshot();
            let range = snapshot.line_content_range(LineIndex(2)).unwrap();
            editor.scroll_y = editor.display_map.line_start_y(2);
            editor.layout_anchor = Some(RevisionRange::new(
                snapshot.revision(),
                ByteRange::new(range.start.0, range.start.0),
            ));
        });

        session.update(cx, |session, cx| {
            let revision = session.revision();
            session
                .edit(
                    DocumentCommand::new(
                        EditTransaction::new(
                            revision,
                            vec![TextEdit::new(ByteRange::new(0, 0), "\n")],
                        ),
                        Selection::caret(ByteOffset(0)),
                        Selection::caret(ByteOffset(1)),
                        EditOrigin::Typing,
                    ),
                    cx,
                )
                .unwrap();
        });

        cx.read(|cx| {
            let snapshot = session.read(cx).snapshot();
            let (anchor, _) = editor.read(cx).top_source_anchor(&snapshot);
            assert_eq!(
                snapshot.copy_range(ByteRange::new(anchor.0, anchor.0 + 5)),
                "third"
            );
        });
    }

    #[gpui::test]
    fn trailing_space_edit_retains_layout_until_atomic_remeasurement(
        cx: &mut gpui::TestAppContext,
    ) {
        let source = (0..1_000)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("test.org"), source.into_bytes()).unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session.clone(), cx));
        editor.update(cx, |editor, cx| {
            let snapshot = session.read(cx).snapshot();
            editor.display_map.configure(snapshot.len_lines(), 640.0);
            editor
                .display_map
                .update_line_layout(500, 3, 24.0, 2.0, 2.0);
            editor
                .display_map
                .update_line_layout(800, 5, 22.0, 0.0, 0.0);
            let end = snapshot.line_content_range(LineIndex(500)).unwrap().end;
            editor.set_selection(Selection::caret(end), cx);
            editor.replace_selection(" ", EditOrigin::Typing, cx);
        });

        cx.read(|cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.display_map.line_height_px(500), 76.0);
            assert_eq!(editor.display_map.line_height_px(800), 110.0);
        });
    }

    #[gpui::test]
    fn refreshing_an_inline_image_keeps_the_previous_geometry_until_replacement(
        cx: &mut gpui::TestAppContext,
    ) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                PathBuf::from("inline-image.org"),
                b"before\n[[file:result.svg]]\nafter\n".to_vec(),
            )
            .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session.clone(), cx));

        editor.update(cx, |editor, cx| {
            let snapshot = session.read(cx).snapshot();
            let link = snapshot.line_content_range(LineIndex(1)).unwrap();
            editor.display_map.configure(snapshot.len_lines(), 320.0);
            editor.display_map.update_line_layout(1, 1, 180.0, 6.0, 6.0);
            editor
                .inline_image_line_dimensions
                .borrow_mut()
                .insert(1, (link.start.0, 320, 180));

            editor.refresh_inline_image(Path::new("result.svg"), cx);

            assert_eq!(editor.display_map.line_height_px(1), 192.0);
            assert_eq!(
                editor.inline_image_line_dimensions.borrow().get(&1),
                Some(&(link.start.0, 320, 180))
            );
        });
    }

    #[gpui::test]
    fn breaking_an_inline_image_link_immediately_removes_its_cached_height(
        cx: &mut gpui::TestAppContext,
    ) {
        let source = (0..80)
            .map(|line| {
                if line == 66 {
                    "[[file:images/typst-demo.svg]]\n".to_owned()
                } else {
                    format!("line {line}\n")
                }
            })
            .collect::<String>();
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("inline-image.org"), source.into_bytes())
                .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session.clone(), cx));

        editor.update(cx, |editor, cx| {
            editor.viewport = Some(Bounds {
                origin: gpui::point(px(0.0), px(0.0)),
                size: gpui::size(px(800.0), px(500.0)),
            });
            let snapshot = session.read(cx).snapshot();
            let link = snapshot.line_content_range(LineIndex(66)).unwrap();
            editor.display_map.configure(snapshot.len_lines(), 640.0);
            editor
                .display_map
                .update_line_layout(66, 1, 300.0, 8.0, 8.0);
            editor
                .inline_image_line_dimensions
                .borrow_mut()
                .insert(66, (link.start.0, 1200, 800));
            editor.set_selection(Selection::new(ByteOffset(link.end.0 - 1), link.end), cx);
            editor.scroll_y = editor.display_map.line_start_y(65);
            editor.replace_selection("", EditOrigin::DeleteBackward, cx);
        });

        cx.read(|cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.display_map.line_height_px(66), LINE_HEIGHT);
            let viewport_height = editor
                .viewport
                .map_or(0.0, |viewport| f32::from(viewport.size.height));
            let max_scroll = (editor.display_map.total_height() - viewport_height).max(0.0);
            assert!(
                (editor.scroll_y - max_scroll).abs() < 0.01,
                "scroll {} must follow shortened document end {max_scroll} for viewport {viewport_height}",
                editor.scroll_y,
            );
        });
    }

    #[gpui::test]
    fn newline_edit_shifts_downstream_layout_without_clearing_it(cx: &mut gpui::TestAppContext) {
        let source = (0..1_000)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("test.org"), source.into_bytes()).unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session.clone(), cx));
        editor.update(cx, |editor, cx| {
            let snapshot = session.read(cx).snapshot();
            editor.display_map.configure(snapshot.len_lines(), 640.0);
            editor
                .display_map
                .update_line_layout(500, 3, 24.0, 2.0, 2.0);
            editor
                .display_map
                .update_line_layout(800, 5, 22.0, 0.0, 0.0);
            let end = snapshot.line_content_range(LineIndex(500)).unwrap().end;
            editor.set_selection(Selection::caret(end), cx);
            editor.replace_selection("\n", EditOrigin::Typing, cx);
        });

        cx.read(|cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.display_map.line_height_px(500), 76.0);
            assert_eq!(editor.display_map.line_height_px(501), LINE_HEIGHT);
            assert_eq!(editor.display_map.line_height_px(801), 110.0);
        });

        editor.update(cx, |editor, cx| {
            let snapshot = session.read(cx).snapshot();
            let newline = snapshot.line_content_range(LineIndex(500)).unwrap().end;
            editor.set_selection(
                Selection::new(newline, ByteOffset(newline.0.saturating_add(1))),
                cx,
            );
            editor.replace_selection("", EditOrigin::Typing, cx);
        });
        cx.read(|cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.display_map.line_height_px(500), 76.0);
            assert_eq!(editor.display_map.line_height_px(800), 110.0);
        });
    }

    #[gpui::test]
    fn edits_discard_previous_revision_hit_rows_before_revealing_the_caret(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(init);
        let source = (0..100)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("test.org"), source.into_bytes()).unwrap()
        });
        let session_for_view = session.clone();
        let (editor, cx) =
            cx.add_window_view(move |_, cx| SemanticEditor::new(session_for_view, cx));
        cx.run_until_parked();

        editor.update(cx, |editor, cx| {
            assert!(!editor.hit_rows.is_empty());
            let snapshot = session.read(cx).snapshot();
            let caret = snapshot.line_content_range(LineIndex(10)).unwrap().end;
            editor.set_selection(Selection::caret(caret), cx);
            editor.replace_selection("\n", EditOrigin::Newline, cx);
            assert!(
                editor.hit_rows.is_empty(),
                "pixel rows from the previous revision must not drive reveal_caret"
            );
            assert!(
                editor.pending_reveal_caret,
                "caret reveal must wait for rows from the edited revision"
            );
        });
        cx.run_until_parked();
        cx.read(|cx| {
            let editor = editor.read(cx);
            assert!(!editor.pending_reveal_caret);
            let snapshot = session.read(cx).snapshot();
            let caret_line = snapshot.line_index_at(editor.selection.head()).unwrap();
            assert!(editor.hit_rows.iter().any(|row| {
                row.line == caret_line
                    && editor.selection.head() >= row.range.start
                    && editor.selection.head() <= row.range.end
            }));
        });
    }

    #[gpui::test]
    fn reversible_newlines_do_not_move_a_visible_viewport(cx: &mut gpui::TestAppContext) {
        cx.update(init);
        let source = (0..1_000)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("test.org"), source.into_bytes()).unwrap()
        });
        let session_for_view = session.clone();
        let (editor, cx) =
            cx.add_window_view(move |_, cx| SemanticEditor::new(session_for_view, cx));
        cx.run_until_parked();

        editor.update(cx, |editor, cx| {
            let snapshot = session.read(cx).snapshot();
            editor.scroll_y = editor.display_map.line_start_y(500);
            editor.layout_anchor = snapshot
                .line_content_range(LineIndex(500))
                .ok()
                .map(|range| {
                    RevisionRange::new(
                        snapshot.revision(),
                        ByteRange::new(range.start.0, range.start.0),
                    )
                });
            let caret = snapshot.line_content_range(LineIndex(510)).unwrap().end;
            editor.selection = Selection::caret(caret);
            editor.sync_selection_utf16(&snapshot);
            cx.notify();
        });
        cx.run_until_parked();
        let settled_scroll = editor.read_with(cx, |editor, _| editor.scroll_y);

        for _ in 0..8 {
            editor.update(cx, |editor, cx| {
                editor.replace_selection("\n", EditOrigin::Newline, cx);
            });
            cx.run_until_parked();
            assert_eq!(
                editor.read_with(cx, |editor, _| editor.scroll_y),
                settled_scroll,
                "inserting a visible newline must preserve the viewport anchor"
            );

            editor.update(cx, |editor, cx| {
                let caret = editor.selection.head();
                editor.selection = Selection::new(caret, ByteOffset(caret.0 - 1));
                editor.replace_selection("", EditOrigin::DeleteBackward, cx);
            });
            cx.run_until_parked();
            assert_eq!(
                editor.read_with(cx, |editor, _| editor.scroll_y),
                settled_scroll,
                "deleting the newline must restore without moving the viewport"
            );
        }
    }

    #[gpui::test]
    fn deleting_a_visible_bottom_newline_keeps_scroll_within_the_new_document_end(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(init);
        let source = (0..1_000)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("test.org"), source.into_bytes()).unwrap()
        });
        let session_for_view = session.clone();
        let (editor, cx) =
            cx.add_window_view(move |_, cx| SemanticEditor::new(session_for_view, cx));
        cx.run_until_parked();

        editor.update(cx, |editor, cx| {
            let viewport_height = f32::from(editor.viewport.unwrap().size.height);
            editor.scroll_y = (editor.display_map.total_height() - viewport_height).max(0.0);
            let snapshot = session.read(cx).snapshot();
            let line = snapshot.line_content_range(LineIndex(990)).unwrap();
            editor.selection = Selection::new(line.end, ByteOffset(line.end.0 + 1));
            editor.sync_selection_utf16(&snapshot);
            editor.replace_selection("", EditOrigin::DeleteBackward, cx);
        });
        cx.run_until_parked();

        cx.read(|cx| {
            let editor = editor.read(cx);
            let viewport_height = f32::from(editor.viewport.unwrap().size.height);
            let max_scroll = (editor.display_map.total_height() - viewport_height).max(0.0);
            assert!(
                (editor.scroll_y - max_scroll).abs() <= 0.5,
                "bottom scroll {} must follow the shortened document end {max_scroll}",
                editor.scroll_y
            );
        });
    }

    #[gpui::test]
    fn minimap_seek_to_end_places_the_last_line_at_the_viewport_bottom(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(init);
        let source = (0..750)
            .map(|line| {
                if line % 7 == 0 {
                    format!("* Heading {line}\n")
                } else {
                    format!(
                        "- line {line}: https://example.com/a/long/path/that/wraps/in/the/editor/{line}\n"
                    )
                }
            })
            .collect::<String>();
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("test.org"), source.into_bytes()).unwrap()
        });
        let session_for_view = session.clone();
        let (editor, cx) =
            cx.add_window_view(move |_, cx| SemanticEditor::new(session_for_view, cx));
        cx.run_until_parked();

        editor.update(cx, |editor, cx| {
            let bottom = editor.minimap.bounds.unwrap().bottom();
            let geometry = editor.minimap_viewport_geometry(editor.minimap.bounds.unwrap());
            let pointer = geometry.thumb_top + geometry.thumb_height / 2.0;
            editor.minimap.drag = Some(crate::minimap::DragSession {
                start_pointer_y: pointer,
                start_thumb_top: geometry.thumb_top,
                start_ratio: geometry.scroll_ratio,
                current_thumb_top: geometry.thumb_top,
            });
            editor.seek_from_minimap(bottom, cx);
        });
        cx.run_until_parked();

        cx.read(|cx| {
            let editor = editor.read(cx);
            let snapshot = session.read(cx).snapshot();
            let viewport = editor.viewport.unwrap();
            let last = editor.hit_rows.last().unwrap();
            assert_eq!(last.line.0, snapshot.len_lines() - 1);
            let last_bottom =
                last.origin_y + last.line_height * (last.layout.wrap_boundaries().len() + 1) as f32;
            assert!(
                f32::from(viewport.bottom() - last_bottom).abs() <= 1.0,
                "last row bottom {last_bottom:?} must meet viewport bottom {:?}",
                viewport.bottom()
            );
        });
    }

    #[gpui::test]
    fn table_alignment_is_one_transaction_and_one_undo_step(cx: &mut gpui::TestAppContext) {
        let original = "| 名|x|\n|---+---|\n| longer |🙂|\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("test.org"), original.as_bytes().to_vec())
                .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session.clone(), cx));
        editor.update(cx, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(2)), cx);
            let snapshot = editor.snapshot(cx);
            let context = org_commands::EditorCommandContext::at(
                std::path::Path::new("test.org"),
                &snapshot,
                editor.selection.head(),
            )
            .unwrap();
            editor.align_table_from_context(&snapshot, &context, 1, cx);
        });
        assert_eq!(session.read_with(cx, |session, _| session.revision().0), 1);
        session.update(cx, |session, cx| {
            assert!(matches!(
                session.undo(cx).unwrap(),
                HistoryOutcome::Applied(_)
            ));
        });
        assert_eq!(
            cx.read(|cx| {
                let snapshot = session.read(cx).snapshot();
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
            }),
            original
        );
    }

    #[gpui::test]
    fn editor_minimap_seek_controls_only_the_editor_viewport(cx: &mut gpui::TestAppContext) {
        let text = "line\n".repeat(1_000);
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("test.org"), text.into_bytes()).unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session, cx));
        editor.update(cx, |editor, cx| {
            editor.viewport = Some(Bounds::new(
                gpui::point(px(0.0), px(0.0)),
                gpui::size(px(800.0), px(220.0)),
            ));
            editor.minimap.bounds = Some(Bounds::new(
                gpui::point(px(704.0), px(0.0)),
                gpui::size(px(96.0), px(220.0)),
            ));
            editor.seek_from_minimap(px(110.0), cx);
            let density = crate::minimap::Density::for_width(96.0);
            let geometry = crate::editor::minimap::viewport_geometry(
                editor.display_map.total_height() / LINE_HEIGHT,
                0.0,
                220.0 / LINE_HEIGHT,
                220.0,
                density,
            );
            let line_height = editor.minimap.line_height(density);
            let clicked_unit =
                geometry.content_top + ((110.0 - density.edge_padding()) / line_height).max(0.0);
            let max_scroll_units =
                (editor.display_map.total_height() / LINE_HEIGHT - 220.0 / LINE_HEIGHT).max(0.0);
            let expected = crate::minimap::ratio_to_offset(
                (clicked_unit / max_scroll_units).clamp(0.0, 1.0),
                editor.display_map.total_height(),
                220.0,
            );
            assert!((editor.scroll_y - expected).abs() < 0.1);
        });
    }

    #[gpui::test]
    fn editor_minimap_paints_first_frame_with_coherent_revision(cx: &mut gpui::TestAppContext) {
        cx.update(init);
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                PathBuf::from("test.org"),
                b"* Heading\nbody\n| a | b |\n".to_vec(),
            )
            .unwrap()
        });
        let session_for_view = session.clone();
        let (editor, cx) =
            cx.add_window_view(move |_, cx| SemanticEditor::new(session_for_view, cx));
        cx.run_until_parked();
        cx.read(|cx| {
            let editor = editor.read(cx);
            assert!(editor.minimap.bounds.is_some());
            assert_eq!(editor.minimap.revision, Some(session.read(cx).revision()));
            assert!(!editor.hit_rows.is_empty());
        });
    }

    #[gpui::test]
    fn minimap_geometry_uses_one_complete_visual_span_while_scrolling(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(init);
        let wrapping_text = "wrapped source text ".repeat(20);
        let source = (0..1_000)
            .map(|index| format!("* Heading {index} {wrapping_text}\nbody {index}\n"))
            .collect::<String>();
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("diagnostic.org"), source.into_bytes())
                .unwrap()
        });
        let session_for_view = session.clone();
        let (editor, cx) =
            cx.add_window_view(move |_, cx| SemanticEditor::new(session_for_view, cx));
        cx.run_until_parked();
        let (initial_height, baseline_height) = cx.read(|cx| {
            let editor = editor.read(cx);
            (
                editor.display_map.total_height(),
                editor.display_map.line_count() as f32 * editor.display_map.base_line_height(),
            )
        });
        assert!(initial_height > baseline_height + 2_000.0);
        let mut camera_samples = Vec::new();
        for scroll_y in [0.0, 5_000.0, 10_000.0, 15_000.0, 20_000.0] {
            editor.update(cx, |editor, cx| {
                editor.scroll_y = scroll_y;
                cx.notify();
            });
            cx.run_until_parked();
            cx.read(|cx| {
                let editor = editor.read(cx);
                let bounds = editor.minimap.bounds.unwrap();
                let geometry = editor.minimap_viewport_geometry(bounds);
                let (_, visible_top, visible_bottom) =
                    editor.minimap_source_viewport(f32::from(bounds.size.height));
                let density = crate::minimap::Density::for_width(f32::from(bounds.size.width));
                let line_height = editor.minimap.line_height(density);
                let expected = ((visible_bottom - visible_top) * line_height
                    + density.edge_padding() * 2.0)
                    .max(crate::minimap::MIN_THUMB_PX)
                    .min(geometry.interaction_height);
                assert!((geometry.thumb_height - expected).abs() < 0.001);
                camera_samples.push((geometry.content_top, geometry.thumb_top));
            });
        }
        for pair in camera_samples.windows(2) {
            assert!(
                pair[1].0 > pair[0].0,
                "minimap background must move forward"
            );
            assert!(
                pair[1].1 > pair[0].1,
                "transparent viewport must move forward"
            );
        }
        let final_height = cx.read(|cx| editor.read(cx).display_map.total_height());
        assert!((final_height - initial_height).abs() < 0.001);
    }

    #[gpui::test]
    fn minimap_viewport_does_not_resize_while_scrolling_through_a_tall_image(
        cx: &mut gpui::TestAppContext,
    ) {
        let source = "line\n".repeat(200);
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("image-scroll.org"), source.into_bytes())
                .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session, cx));

        editor.update(cx, |editor, _| {
            editor.viewport = Some(Bounds::new(
                gpui::point(px(0.0), px(0.0)),
                gpui::size(px(800.0), px(220.0)),
            ));
            let minimap_bounds = Bounds::new(
                gpui::point(px(704.0), px(0.0)),
                gpui::size(px(96.0), px(220.0)),
            );
            editor.minimap.bounds = Some(minimap_bounds);
            editor
                .display_map
                .update_line_layout(50, 1, 440.0, 6.0, 6.0);

            let image_top = editor.display_map.line_start_y(50);
            let image_bottom = editor.display_map.line_start_y(51);
            let mut thumb_heights = Vec::new();
            for scroll_y in [image_top - 110.0, image_top + 80.0, image_bottom + 40.0] {
                editor.scroll_y = scroll_y;
                let (_, visible_top, visible_bottom) = editor.minimap_source_viewport(220.0);
                assert!(((visible_bottom - visible_top) - 10.0).abs() < 0.001);
                thumb_heights.push(
                    editor
                        .minimap_viewport_geometry(minimap_bounds)
                        .thumb_height,
                );
            }
            assert!(
                thumb_heights
                    .windows(2)
                    .all(|pair| (pair[0] - pair[1]).abs() < 0.001)
            );
        });
    }

    #[gpui::test]
    fn platform_input_and_ime_are_single_undo_steps(cx: &mut gpui::TestAppContext) {
        cx.update(init);
        let session =
            cx.new(|_| DocumentSession::from_utf8(PathBuf::from("test.org"), Vec::new()).unwrap());
        let session_for_view = session.clone();
        let (editor, cx) =
            cx.add_window_view(move |_, cx| SemanticEditor::new(session_for_view.clone(), cx));

        cx.simulate_input("e\u{301}🙂");
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                <SemanticEditor as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                    editor, None, "n", None, window, cx,
                );
                <SemanticEditor as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                    editor,
                    None,
                    "ni",
                    Some(2..2),
                    window,
                    cx,
                );
                <SemanticEditor as gpui::EntityInputHandler>::replace_text_in_range(
                    editor, None, "你", window, cx,
                );
            });
        });
        assert_eq!(
            cx.read_entity(&session, |session, _| {
                let snapshot = session.snapshot();
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
            }),
            "e\u{301}🙂你"
        );

        cx.simulate_keystrokes("cmd-z");
        assert_eq!(
            cx.read_entity(&session, |session, _| {
                let snapshot = session.snapshot();
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
            }),
            "e\u{301}🙂"
        );
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(
            cx.read_entity(&session, |session, _| {
                let snapshot = session.snapshot();
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
            }),
            ""
        );
    }

    #[gpui::test]
    fn changing_selection_commits_active_ime_as_one_undo_step(cx: &mut gpui::TestAppContext) {
        cx.update(init);
        let session =
            cx.new(|_| DocumentSession::from_utf8(PathBuf::from("test.org"), Vec::new()).unwrap());
        let session_for_view = session.clone();
        let (editor, cx) =
            cx.add_window_view(move |_, cx| SemanticEditor::new(session_for_view.clone(), cx));

        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                <SemanticEditor as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                    editor,
                    None,
                    "ni",
                    Some(2..2),
                    window,
                    cx,
                );
                editor.set_selection(Selection::caret(ByteOffset(0)), cx);
            });
        });
        assert_eq!(
            cx.read_entity(&session, |session, _| {
                let snapshot = session.snapshot();
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
            }),
            "ni"
        );
        cx.update(|_, cx| {
            session.update(cx, |session, cx| {
                assert!(matches!(
                    session.undo(cx).unwrap(),
                    HistoryOutcome::Applied(_)
                ));
            });
        });
        assert_eq!(
            cx.read_entity(&session, |session, _| session.snapshot().len_bytes()),
            0
        );
    }

    #[gpui::test]
    fn a_second_editor_maps_its_selection_through_shared_document_edits(
        cx: &mut gpui::TestAppContext,
    ) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("shared.org"), b"abcd".to_vec()).unwrap()
        });
        let left = cx.new(|cx| SemanticEditor::new(session.clone(), cx));
        let right = cx.new(|cx| SemanticEditor::new(session.clone(), cx));
        right.update(cx, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(4)), cx)
        });

        session.update(cx, |session, cx| {
            session
                .edit(
                    DocumentCommand::new(
                        EditTransaction::new(
                            session.revision(),
                            vec![TextEdit::new(ByteRange::new(0, 0), "x")],
                        ),
                        Selection::caret(ByteOffset(0)),
                        Selection::caret(ByteOffset(1)),
                        EditOrigin::Typing,
                    ),
                    cx,
                )
                .unwrap();
        });
        cx.run_until_parked();

        assert_eq!(
            cx.read(|cx| right.read(cx).selection()),
            Selection::caret(ByteOffset(5))
        );
        assert_eq!(
            cx.read(|cx| left.read(cx).selection()),
            Selection::caret(ByteOffset(0))
        );
    }

    #[gpui::test]
    fn editor_panes_share_semantics_but_keep_minimap_state_local(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                PathBuf::from("shared-semantics.org"),
                b"#+begin_quote\nbody\n#+end_quote\n".to_vec(),
            )
            .unwrap()
        });
        let syntax = Arc::new(EditorSyntaxService::default());
        let left = cx.new(|cx| {
            SemanticEditor::new_with_syntax_service(session.clone(), false, syntax.clone(), cx)
        });
        let right = cx
            .new(|cx| SemanticEditor::new_with_syntax_service(session, false, syntax.clone(), cx));

        cx.read(|cx| {
            assert!(Arc::ptr_eq(
                &left.read(cx).syntax_service,
                &right.read(cx).syntax_service
            ));
        });
        left.update(cx, |editor, cx| editor.set_soft_wrap(false, cx));
        cx.read(|cx| {
            assert_eq!(left.read(cx).minimap.generation, 0);
            assert!(right.read(cx).display_map.soft_wrap());
        });
    }

    #[gpui::test]
    fn selection_and_caret_do_not_invalidate_minimap_raster(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                PathBuf::from("selection-overlay.org"),
                b"* Heading\nbody\n".to_vec(),
            )
            .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session, cx));

        editor.update(cx, |editor, cx| {
            let generation = editor.minimap.generation;
            let raster_epoch = editor
                .minimap
                .raster_epoch
                .load(std::sync::atomic::Ordering::Acquire);
            let viewport_generation = editor.minimap.viewport_generation();

            editor.set_selection(Selection::new(ByteOffset(2), ByteOffset(9)), cx);
            assert_eq!(editor.minimap.generation, generation);
            assert_eq!(
                editor
                    .minimap
                    .raster_epoch
                    .load(std::sync::atomic::Ordering::Acquire),
                raster_epoch
            );
            assert_eq!(editor.minimap.viewport_generation(), viewport_generation);

            editor.set_selection(Selection::caret(ByteOffset(10)), cx);
            assert_eq!(editor.minimap.generation, generation);
            assert_eq!(
                editor
                    .minimap
                    .raster_epoch
                    .load(std::sync::atomic::Ordering::Acquire),
                raster_epoch
            );
            assert_eq!(editor.minimap.viewport_generation(), viewport_generation);
        });
    }
}
