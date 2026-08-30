mod commands;
mod display_map;
mod element;
mod input;

use std::{collections::HashMap, ops::Range, sync::Arc, time::Duration};

use display_map::SourceDisplayMap;
use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, FocusHandle, Focusable, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ShapedLine, SharedString,
    Subscription, Task, Window, actions, div, prelude::*, px, rgb,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    document::{
        ByteOffset, ByteRange, DocumentEvent, DocumentSession, DocumentSnapshot, EditOrigin,
        EditTransaction, HistoryOutcome, LineIndex, Selection, SessionEdit, TextEdit, TextSnapshot,
    },
    theme::current_theme,
};

pub use element::EditorElement;

actions!(
    source_editor,
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
        Undo,
        Redo,
        Copy,
        Cut,
        Paste
    ]
);

const LINE_HEIGHT: f32 = 22.0;

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some("SourceEditor")),
        KeyBinding::new("delete", DeleteForward, Some("SourceEditor")),
        KeyBinding::new("left", MoveLeft, Some("SourceEditor")),
        KeyBinding::new("right", MoveRight, Some("SourceEditor")),
        KeyBinding::new("alt-left", MoveWordLeft, Some("SourceEditor")),
        KeyBinding::new("alt-right", MoveWordRight, Some("SourceEditor")),
        KeyBinding::new("up", MoveUp, Some("SourceEditor")),
        KeyBinding::new("down", MoveDown, Some("SourceEditor")),
        KeyBinding::new("shift-left", SelectLeft, Some("SourceEditor")),
        KeyBinding::new("shift-right", SelectRight, Some("SourceEditor")),
        KeyBinding::new("alt-shift-left", SelectWordLeft, Some("SourceEditor")),
        KeyBinding::new("alt-shift-right", SelectWordRight, Some("SourceEditor")),
        KeyBinding::new("shift-up", SelectUp, Some("SourceEditor")),
        KeyBinding::new("shift-down", SelectDown, Some("SourceEditor")),
        KeyBinding::new("cmd-left", MoveLineStart, Some("SourceEditor")),
        KeyBinding::new("cmd-right", MoveLineEnd, Some("SourceEditor")),
        KeyBinding::new("cmd-up", MoveDocumentStart, Some("SourceEditor")),
        KeyBinding::new("cmd-down", MoveDocumentEnd, Some("SourceEditor")),
        KeyBinding::new("cmd-a", SelectAll, Some("SourceEditor")),
        KeyBinding::new("enter", Newline, Some("SourceEditor")),
        KeyBinding::new("tab", InsertTab, Some("SourceEditor")),
        KeyBinding::new("cmd-z", Undo, Some("SourceEditor")),
        KeyBinding::new("cmd-shift-z", Redo, Some("SourceEditor")),
        KeyBinding::new("cmd-c", Copy, Some("SourceEditor")),
        KeyBinding::new("cmd-x", Cut, Some("SourceEditor")),
        KeyBinding::new("cmd-v", Paste, Some("SourceEditor")),
    ]);
}

#[derive(Clone)]
pub(super) struct HitRow {
    pub(super) range: ByteRange,
    pub(super) origin_y: Pixels,
    pub(super) text_origin_x: Pixels,
    pub(super) layout: ShapedLine,
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ShapeKey {
    pub(super) text: SharedString,
    pub(super) font_size_bits: u32,
    pub(super) marked: Option<(usize, usize)>,
}

#[cfg(feature = "benchmarks")]
pub(super) enum FrameBenchmarkAction {
    Inactive,
    Continue,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SourceEditorStatus {
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

pub struct SourceEditor {
    session: Entity<DocumentSession>,
    focus_handle: FocusHandle,
    selection: Selection,
    selection_utf16: Range<usize>,
    selection_utf16_reversed: bool,
    marked: Option<PlatformRange>,
    composition: Option<Composition>,
    display_map: SourceDisplayMap,
    shape_cache: HashMap<ShapeKey, ShapedLine>,
    scroll_y: f32,
    viewport: Option<Bounds<Pixels>>,
    hit_rows: Arc<[HitRow]>,
    is_selecting: bool,
    drag_position: Option<Point<Pixels>>,
    autoscroll_task: Option<Task<()>>,
    autofocus: bool,
    focus_lost_subscription: Option<Subscription>,
    #[cfg(feature = "benchmarks")]
    frame_benchmark: Option<EditorFrameBenchmark>,
    _session_subscription: Subscription,
}

impl SourceEditor {
    pub fn new(session: Entity<DocumentSession>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&session, |this, _, event: &DocumentEvent, cx| {
            if matches!(event, DocumentEvent::Reloaded { .. }) {
                let snapshot = this.snapshot(cx);
                this.selection = this.selection.clamp(&snapshot);
                this.sync_selection_utf16(&snapshot);
                this.marked = None;
                this.composition = None;
                this.shape_cache.clear();
                this.scroll_y = 0.0;
            }
            cx.notify();
        });
        Self {
            session,
            focus_handle: cx.focus_handle(),
            selection: Selection::default(),
            selection_utf16: 0..0,
            selection_utf16_reversed: false,
            marked: None,
            composition: None,
            display_map: SourceDisplayMap::default(),
            shape_cache: HashMap::with_capacity(128),
            scroll_y: 0.0,
            viewport: None,
            hit_rows: Arc::from([]),
            is_selecting: false,
            drag_position: None,
            autoscroll_task: None,
            autofocus: true,
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

    pub fn set_selection(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.finish_composition(cx);
        let snapshot = self.snapshot(cx);
        self.selection = selection.clamp(&snapshot);
        self.sync_selection_utf16(&snapshot);
        self.reveal_caret(&snapshot);
        cx.notify();
    }

    pub fn snapshot(&self, cx: &App) -> DocumentSnapshot {
        self.session.read(cx).snapshot()
    }

    pub(crate) fn status(&self, cx: &App) -> SourceEditorStatus {
        let snapshot = self.snapshot(cx);
        let (line, column) = snapshot
            .line_and_column_at(self.selection.head())
            .unwrap_or_default();
        let total_lines = snapshot.len_lines();
        let viewport_height = self
            .viewport
            .map_or(0.0, |bounds| f32::from(bounds.size.height));
        let visible_bottom_line = if viewport_height > 0.0 {
            ((self.scroll_y + viewport_height) / LINE_HEIGHT)
                .ceil()
                .max(0.0) as u64
        } else {
            0
        }
        .min(total_lines);
        let document_height = total_lines as f32 * LINE_HEIGHT;
        let reached_end =
            viewport_height > 0.0 && self.scroll_y + viewport_height + 0.5 >= document_height;
        SourceEditorStatus {
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

    #[gpui::test]
    fn source_editor_edits_unicode_and_restores_selection_on_undo(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("test.org"), "a中".as_bytes().to_vec())
                .unwrap()
        });
        let editor = cx.new(|cx| SourceEditor::new(session.clone(), cx));
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
    fn source_status_uses_live_caret_and_viewport_coordinates(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                PathBuf::from("test.org"),
                "first\n你😀z\n".as_bytes().to_vec(),
            )
            .unwrap()
        });
        let editor = cx.new(|cx| SourceEditor::new(session, cx));
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
    fn platform_input_and_ime_are_single_undo_steps(cx: &mut gpui::TestAppContext) {
        cx.update(init);
        let session =
            cx.new(|_| DocumentSession::from_utf8(PathBuf::from("test.org"), Vec::new()).unwrap());
        let session_for_view = session.clone();
        let (editor, cx) =
            cx.add_window_view(move |_, cx| SourceEditor::new(session_for_view.clone(), cx));

        cx.simulate_input("e\u{301}🙂");
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                <SourceEditor as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                    editor, None, "n", None, window, cx,
                );
                <SourceEditor as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
                    editor,
                    None,
                    "ni",
                    Some(2..2),
                    window,
                    cx,
                );
                <SourceEditor as gpui::EntityInputHandler>::replace_text_in_range(
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
            cx.add_window_view(move |_, cx| SourceEditor::new(session_for_view.clone(), cx));

        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                <SourceEditor as gpui::EntityInputHandler>::replace_and_mark_text_in_range(
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
}
