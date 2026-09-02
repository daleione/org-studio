mod commands;
mod element;
mod folding;
mod input;
mod layout_map;
mod minimap;
mod org_commands;
mod syntax;

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, SharedString,
    Subscription, Task, Window, WrappedLine, actions, div, prelude::*, px, rgb,
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
        Paste
    ]
);

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

    fn changed_line_count(&self) -> u64 {
        self.changed_ranges
            .iter()
            .map(|range| range.end - range.start)
            .sum()
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ShapeKey {
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
        Some(Self {
            warmup_remaining,
            target_samples,
            samples: Vec::with_capacity(target_samples),
        })
    }

    fn record(&mut self, elapsed: Duration) -> bool {
        if self.warmup_remaining > 0 {
            self.warmup_remaining -= 1;
            return false;
        }
        self.samples.push(elapsed);
        self.samples.len() >= self.target_samples
    }

    fn report(&self) {
        let mut samples = self.samples.clone();
        samples.sort_unstable();
        let percentile = |p: f64| {
            let index = ((samples.len() - 1) as f64 * p).ceil() as usize;
            samples[index].as_secs_f64() * 1_000.0
        };
        eprintln!(
            "org_editor_frame_cpu samples={} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3}",
            samples.len(),
            percentile(0.50),
            percentile(0.95),
            percentile(0.99),
        );
    }
}

pub struct SemanticEditor {
    session: Entity<DocumentSession>,
    focus_handle: FocusHandle,
    selection: Selection,
    selection_revision: Revision,
    selection_utf16: Range<usize>,
    selection_utf16_reversed: bool,
    marked: Option<PlatformRange>,
    composition: Option<Composition>,
    display_map: EditorLayoutMap,
    folds: folding::EditorFoldState,
    fold_markers: Arc<HashSet<u64>>,
    fold_animation: Option<EditorFoldAnimation>,
    fold_animation_revision: u64,
    command_feedback: Option<SharedString>,
    minimap: minimap::EditorMinimapHost,
    syntax_cache: syntax::EditorSyntaxCache,
    layout_anchor: Option<RevisionRange>,
    shape_cache: HashMap<ShapeKey, Arc<WrappedLine>>,
    scroll_y: f32,
    scroll_x: f32,
    vertical_goal_x: Option<f32>,
    viewport: Option<Bounds<Pixels>>,
    hit_rows: Arc<[HitRow]>,
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

    pub(super) fn animated_visible_position_at_y(&self, y: f32) -> f32 {
        if self.animated_visible_line_count() <= 0.0 {
            return 0.0;
        }
        let line = self.animated_line_at_y(y);
        let line_top = self.animated_line_start_y(line);
        let line_height = self.animated_line_height_px(line).max(1.0);
        let fraction = ((y - line_top) / line_height).clamp(0.0, 1.0);
        let base = self.display_map.visible_ordinal_for_line(line) as f32;
        let compressed_lines = self.fold_animation.as_ref().map_or(0.0, |animation| {
            animation
                .changed_ranges
                .iter()
                .map(|range| range.end.min(line).saturating_sub(range.start) as f32)
                .sum::<f32>()
                * (1.0 - animation.scale())
        });
        let line_scale = self.fold_animation.as_ref().map_or(1.0, |animation| {
            if animation
                .changed_ranges
                .iter()
                .any(|range| range.contains(&line))
            {
                animation.scale()
            } else {
                1.0
            }
        });
        (base - compressed_lines + fraction * line_scale).max(0.0)
    }

    pub fn new(session: Entity<DocumentSession>, cx: &mut Context<Self>) -> Self {
        Self::new_with_autofocus(session, true, cx)
    }

    pub(crate) fn new_with_autofocus(
        session: Entity<DocumentSession>,
        autofocus: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.subscribe(&session, |this, _, event: &DocumentEvent, cx| {
            if let DocumentEvent::Edited { delta, .. } = event {
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
                this.syntax_cache.invalidate_from(
                    snapshot.document_id(),
                    snapshot.revision(),
                    first_line,
                );
                let wrap_width = this.display_map.wrap_width();
                if previous_line_count != snapshot.len_lines() && delta.edits.len() == 1 {
                    let edit = delta.edits[0];
                    let new_end = edit.old.start.0.saturating_add(edit.new_len);
                    let last_line = snapshot
                        .line_index_at(ByteOffset(new_end.min(snapshot.len_bytes())))
                        .map_or(first_line, |line| line.0);
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
                this.vertical_goal_x = None;
            } else if matches!(event, DocumentEvent::Reloaded { .. }) {
                this.display_map.invalidate_layout();
            } else if matches!(event, DocumentEvent::PathChanged { .. }) {
                this.syntax_cache.reset();
                this.folds = folding::EditorFoldState::default();
                this.fold_markers = Arc::new(HashSet::new());
                this.fold_animation = None;
                this.fold_animation_revision = this.fold_animation_revision.wrapping_add(1);
                this.display_map.set_hidden_ranges(Vec::new());
                this.shape_cache.clear();
                this.minimap.invalidate_raster();
            }
            if matches!(event, DocumentEvent::Reloaded { .. }) {
                this.syntax_cache.reset();
                let snapshot = this.snapshot(cx);
                this.selection = this.selection.clamp(&snapshot);
                this.selection_revision = snapshot.revision();
                this.sync_selection_utf16(&snapshot);
                this.marked = None;
                this.composition = None;
                this.shape_cache.clear();
                this.hit_rows = Arc::from([]);
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
            focus_handle: cx.focus_handle(),
            selection: Selection::default(),
            selection_revision: initial_snapshot.revision(),
            selection_utf16: 0..0,
            selection_utf16_reversed: false,
            marked: None,
            composition: None,
            display_map,
            folds: folding::EditorFoldState::default(),
            fold_markers: Arc::new(HashSet::new()),
            fold_animation: None,
            fold_animation_revision: 0,
            command_feedback: None,
            minimap: minimap::EditorMinimapHost::default(),
            syntax_cache: syntax::EditorSyntaxCache::default(),
            layout_anchor: None,
            shape_cache: HashMap::with_capacity(128),
            scroll_y: 0.0,
            scroll_x: 0.0,
            vertical_goal_x: None,
            viewport: None,
            hit_rows: Arc::from([]),
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
            self.minimap.invalidate_raster();
            cx.notify();
        }
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

    fn sync_selection_revision(&mut self, cx: &App) {
        self.selection_revision = self.session.read(cx).revision();
    }

    pub fn snapshot(&self, cx: &App) -> DocumentSnapshot {
        self.session.read(cx).snapshot()
    }

    #[cfg(test)]
    pub(crate) fn has_active_composition(&self) -> bool {
        self.composition.is_some()
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
        let Some(benchmark) = self.frame_benchmark.as_mut() else {
            return FrameBenchmarkAction::Inactive;
        };
        if benchmark.record(elapsed) {
            benchmark.report();
            self.frame_benchmark = None;
            return FrameBenchmarkAction::Complete;
        }
        self.replace_selection("a", EditOrigin::Typing, cx);
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
    use std::path::PathBuf;

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
    fn minimap_geometry_uses_the_settled_visible_source_span(cx: &mut gpui::TestAppContext) {
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
        let initial_height = cx.read(|cx| editor.read(cx).display_map.total_height());
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
            });
        }
        let final_height = cx.read(|cx| editor.read(cx).display_map.total_height());
        assert!(final_height > initial_height + 2_000.0);
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
}
