//! Window-local open-file overview. The document pool remains the sole owner
//! of open documents. A new draft stays provisional until its zoom commits.
mod background;
#[cfg(feature = "visual-check")]
pub mod benchmark;
mod close;
mod frame;
mod gesture;
mod layout;
mod motion;
mod view;
pub(super) use view::toolbar_button;
#[cfg(test)]
mod close_tests;
#[cfg(test)]
mod tests;

use super::{
    ContentRoute, WorkspaceWindow,
    native_input::{InputConfig, InputEvent, NativeInput},
};
use crate::document::{DocumentId, DocumentSession, Revision};
use frame::{DocumentFrame, FrameCache};
use gesture::Gesture;
use gpui::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, KeyDownEvent, Pixels, ScrollHandle,
    Size, Subscription, TouchPhase, Window, point, px, size,
};
use layout::GridLayout;
use motion::{Follow, Motion};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

gpui::actions!(file_overview, [FileOverview]);

#[derive(Default)]
pub(crate) struct OverviewHost {
    #[cfg(feature = "visual-check")]
    trace: benchmark::GestureTrace,
    pub(super) drawn_frame: crate::document_preview::FrameSlot,
    session: Option<Session>,
    cache: FrameCache,
    presented: Option<Presented>,
    probe: Option<Gesture>,
    serial: u64,
}

#[derive(Clone, Copy)]
struct Presented {
    id: DocumentId,
    revision: Revision,
    bounds: Bounds<Pixels>,
    scale: f32,
    palette: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Entering,
    Visible,
    Returning,
    Opening,
    Rebounding,
    Handoff,
}

struct Session {
    phase: Phase,
    progress: f32,
    motion: Option<Motion>,
    handoff_frames: u8,
    handoff_backdrop: f32,
    gesture: Option<Gesture>,
    follow: Option<Follow>,
    outward_probe: Option<(DocumentId, Gesture)>,
    origin: Option<DocumentId>,
    origin_route: ContentRoute,
    origin_focus: Option<FocusHandle>,
    origin_bounds: Bounds<Pixels>,
    origin_card: Bounds<Pixels>,
    origin_frame: Option<Arc<DocumentFrame>>,
    target: Option<DocumentId>,
    target_from: Bounds<Pixels>,
    target_to: Bounds<Pixels>,
    target_frame: Option<Arc<DocumentFrame>>,
    new_document: Option<super::ReadyDocument>,
    selected: Option<DocumentId>,
    query: Entity<NativeInput>,
    _subscription: Subscription,
    grid_focus: FocusHandle,
    new_focus: FocusHandle,
    done_focus: FocusHandle,
    query_bounds: Bounds<Pixels>,
    focus_query_pending: bool,
    pending_command: Option<(String, gpui::Modifiers)>,
    scroll: ScrollHandle,
    viewport: Size<Pixels>,
    scale: f32,
    palette: u32,
    reduced_motion: bool,
    preparation: Option<Preparation>,
    notice: Option<String>,
}

struct Preparation {
    started: Instant,
    step: PreparationStep,
}

enum PreparationStep {
    Reading { _task: gpui::Task<()> },
    Painting(PreparingDocument),
}

pub(crate) struct PreparingDocument {
    pub(crate) document: super::ReadyDocument,
    pub(crate) workspace: super::DocumentWorkspaceState,
    pub(crate) bounds: Bounds<Pixels>,
    revision: Revision,
    path: std::path::PathBuf,
    pub(crate) has_plots: bool,
    pub(crate) frame: crate::document_preview::FrameSlot,
}

impl Session {
    fn is_preparing(&self) -> bool {
        self.phase == Phase::Opening && self.preparation.is_some()
    }

    fn gesture_scale(&self) -> f32 {
        if matches!(self.phase, Phase::Opening | Phase::Rebounding) {
            self.target_to.size.width / self.target_from.size.width
        } else {
            self.origin_card.size.width / self.origin_bounds.size.width
        }
    }

    fn begin_gesture(&mut self, mut gesture: Gesture, now: Instant) {
        gesture.set_scale(self.gesture_scale());
        self.motion = None;
        self.follow = (!self.reduced_motion).then(|| Follow::new(now, 0.));
        self.progress = 0.;
        self.gesture = Some(gesture);
        self.update_gesture_target(now);
    }

    fn resume_gesture(&mut self, now: Instant) {
        let Some(motion) = self.motion.take() else {
            return;
        };
        let sample = motion.sample(now);
        let opening = matches!(self.phase, Phase::Opening | Phase::Rebounding);
        let mut gesture = Gesture::new(opening, now);
        gesture.set_scale(self.gesture_scale());
        gesture.seek(sample.value);
        self.phase = if opening {
            Phase::Opening
        } else {
            Phase::Entering
        };
        self.progress = sample.value;
        self.follow = (!self.reduced_motion).then(|| Follow::resume(now, sample));
        self.gesture = Some(gesture);
    }

    fn update_gesture_target(&mut self, now: Instant) {
        let Some(gesture) = &self.gesture else {
            return;
        };
        let target = gesture.progress();
        if let Some(follow) = &mut self.follow {
            follow.target(now, target);
        } else {
            self.progress = target;
        }
    }

    fn settle(&mut self, phase: Phase, target: f32, now: Instant) {
        let velocity = if self.is_preparing() {
            // No opening frame has been shown yet. Cancelling must start at
            // that visible position, not at the unseen preparation clock.
            self.follow = None;
            self.progress = 0.;
            0.
        } else if let Some(follow) = self.follow.take() {
            let sample = follow.sample(now);
            self.progress = sample.value;
            sample.velocity
        } else if let Some(motion) = self.motion {
            let sample = motion.sample(now);
            self.progress = sample.value;
            sample.velocity
        } else {
            0.
        };
        self.phase = phase;
        if phase != Phase::Opening {
            self.preparation = None;
        }
        self.gesture = None;
        self.motion = Some(Motion::settle(
            now,
            self.progress,
            target,
            velocity,
            self.reduced_motion,
        ));
    }
}

#[derive(Clone)]
struct Item {
    id: DocumentId,
    session: Entity<DocumentSession>,
    name: String,
    path: String,
    revision: Revision,
    current: bool,
    dirty: bool,
    read_only: bool,
    conflict: bool,
    missing: bool,
    saving: bool,
}

impl WorkspaceWindow {
    pub(crate) fn file_overview_is_open(&self) -> bool {
        self.file_overview.session.is_some()
    }

    pub(crate) fn file_overview_records_frame(&self) -> bool {
        self.file_overview
            .session
            .as_ref()
            .is_none_or(|s| s.phase == Phase::Handoff)
    }

    pub(crate) fn file_overview_covers_document(&self) -> bool {
        self.file_overview.session.as_ref().is_some_and(|state| {
            !state.reduced_motion
                && state.phase != Phase::Handoff
                && (!matches!(state.phase, Phase::Entering | Phase::Returning)
                    || state.origin_frame.is_some())
        })
    }

    pub(crate) fn overview_preparing_document(&self) -> Option<&PreparingDocument> {
        let preparation = self.file_overview.session.as_ref()?.preparation.as_ref()?;
        match &preparation.step {
            PreparationStep::Painting(drawing) => Some(drawing),
            PreparationStep::Reading { .. } => None,
        }
    }

    fn prepare_overview_document_frame(
        &mut self,
        id: DocumentId,
        bounds: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let pending = self
            .file_overview
            .session
            .as_ref()
            .and_then(|state| state.new_document.as_ref())
            .filter(|document| document.session.read(cx).id() == id)
            .map(|document| (document.clone(), super::DocumentWorkspaceState::default()));
        let drawing = pending
            .or_else(|| self.overview_document_for_paint(id, cx))
            .map(|(document, workspace)| {
                let doc = document.session.read(cx);
                PreparingDocument {
                    revision: doc.revision(),
                    path: doc.syntax_path().to_path_buf(),
                    has_plots: crate::document::DocumentFormat::from_path(doc.syntax_path())
                        == crate::document::DocumentFormat::Org
                        && crate::plot::may_have_plots(&doc.snapshot()),
                    document,
                    workspace,
                    bounds,
                    frame: Default::default(),
                }
            });
        if let Some(state) = self.file_overview.session.as_mut() {
            if let Some(drawing) = drawing {
                state.preparation = Some(Preparation {
                    started: state
                        .preparation
                        .as_ref()
                        .map_or_else(Instant::now, |p| p.started),
                    step: PreparationStep::Painting(drawing),
                });
                if state.phase == Phase::Opening {
                    state.progress = 0.;
                }
            } else if state.phase == Phase::Visible {
                self.file_overview.cache.remove(id);
            } else if state.phase == Phase::Opening {
                state.notice =
                    Some("无法准备文档，请重新选择 / Could not prepare document; retry".into());
                state.settle(Phase::Rebounding, 0., Instant::now());
            }
        }
        cx.notify();
    }

    fn finish_overview_document_frame(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(drawing) = self.overview_preparing_document() else {
            return;
        };
        let doc = drawing.document.session.read(cx);
        let id = doc.id();
        if doc.revision() != drawing.revision || doc.syntax_path() != drawing.path {
            if let Some(state) = self.file_overview.session.as_mut() {
                if state.phase == Phase::Visible {
                    state.preparation = None;
                    self.file_overview.cache.remove(id);
                } else {
                    state.notice =
                        Some("文件已变化，请重新选择 / File changed; select it again".into());
                    state.settle(Phase::Rebounding, 0., Instant::now());
                }
            }
            cx.notify();
            return;
        }
        if drawing.frame.paint_count() < 2 {
            return;
        }
        let Some(frame) =
            DocumentFrame::from_painted(&drawing.frame, drawing.bounds, drawing.revision)
        else {
            return;
        };
        let new_document = self
            .file_overview
            .session
            .as_ref()
            .is_some_and(|state| state.new_document.is_some());
        if !new_document {
            self.file_overview.cache.insert(id, frame.clone());
        }
        if let Some(state) = self.file_overview.session.as_mut() {
            if state.phase == Phase::Visible {
                if state.origin == Some(id) {
                    state.origin_frame = Some(frame);
                }
                state.preparation = None;
                cx.notify();
                return;
            }
        }
        let items = self.overview_items(cx);
        let index = items.iter().position(|item| item.id == id);
        if index.is_none() && !new_document {
            if let Some(state) = self.file_overview.session.as_mut() {
                state.notice =
                    Some("目标已不可选，请重新选择 / Target unavailable; select it again".into());
                state.settle(Phase::Rebounding, 0., Instant::now());
            }
            return;
        }
        let grid = self.overview_grid(window, &items);
        let stage = self.overview_stage_bounds(window);
        let Some(state) = self.file_overview.session.as_mut() else {
            return;
        };
        if let Some(index) = index {
            state.target_from =
                grid.document_preview(index, state.scroll.offset().y, frame.source.bounds.size);
        }
        state.target_to = layout::contain(frame.source.bounds.size, stage);
        let scale = state.gesture_scale();
        if let Some(gesture) = &mut state.gesture {
            gesture.set_scale(scale);
        }
        state.target_frame = Some(frame);
        state.preparation = None;
        state.progress = 0.;
        state.follow = (state.gesture.is_some() && !state.reduced_motion)
            .then(|| Follow::new(Instant::now(), 0.));
        if state.gesture.is_some() {
            state.update_gesture_target(Instant::now());
        } else {
            state.motion = Some(Motion::settle(
                Instant::now(),
                0.,
                1.,
                0.,
                state.reduced_motion,
            ));
        }
    }

    pub(crate) fn invalidate_file_overview_frames(&mut self) {
        self.file_overview.cache.clear();
        self.file_overview.presented = None;
        if let Some(state) = &mut self.file_overview.session {
            if state.phase == Phase::Opening {
                // A resource update invalidates the transition as well as the
                // thumbnail. Return to the grid without activating stale content.
                state.settle(Phase::Rebounding, 0., Instant::now());
            }
            state.origin_frame = None;
            state.target_frame = None;
        }
    }

    fn overview_stage_bounds(&self, window: &Window) -> Bounds<Pixels> {
        let viewport = window.viewport_size();
        let left = if self.sidebar_visible() {
            self.rendered_sidebar_width(f32::from(viewport.width)) + 6.0
        } else {
            0.0
        };
        Bounds::new(
            point(px(left), px(super::TITLEBAR_HEIGHT)),
            size(
                (viewport.width - px(left)).max(px(1.0)),
                (viewport.height - px(super::TITLEBAR_HEIGHT)).max(px(1.0)),
            ),
        )
    }

    fn overview_items(&self, cx: &App) -> Vec<Item> {
        let active = self.document_session().map(|s| s.read(cx).id());
        let mut sessions: HashMap<_, _> = self
            .buffer_sessions()
            .map(|session| (session.read(cx).id(), session))
            .collect();
        let query = self
            .file_overview
            .session
            .as_ref()
            .map_or("", |state| state.query.read(cx).text.as_str());
        self.buffers
            .opened_ids()
            .filter_map(|id| {
                let session = sessions.remove(&id)?;
                let doc = session.read(cx);
                if doc.is_generated() {
                    return None;
                }
                let name = doc.display_name();
                let path = doc
                    .file_path()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                super::buffers::match_score(&name, &path, query)?;
                Some(Item {
                    id,
                    name,
                    path,
                    revision: doc.revision(),
                    current: active == Some(doc.id()),
                    dirty: doc.is_dirty(),
                    read_only: doc.is_read_only(),
                    conflict: matches!(
                        doc.sync_state(),
                        crate::document::SyncState::Conflict { .. }
                    ),
                    missing: matches!(doc.sync_state(), crate::document::SyncState::Missing { .. }),
                    saving: matches!(doc.save_state(), crate::document::SaveState::Saving { .. }),
                    session,
                })
            })
            .collect()
    }

    fn overview_source_size(&self, window: &Window) -> Size<Pixels> {
        self.file_overview.session.as_ref().map_or_else(
            || self.overview_stage_bounds(window).size,
            |state| state.origin_bounds.size,
        )
    }

    fn overview_grid(&self, window: &Window, items: &[Item]) -> GridLayout {
        let sources = vec![self.overview_source_size(window); items.len() + 1];
        GridLayout::with_sources(window.viewport_size(), &sources)
    }

    pub(crate) fn open_file_overview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_document_name(cx);
        let handoff_frame = match self.file_overview.session.as_mut() {
            Some(state) if state.phase == Phase::Returning => {
                // The button reverses the current motion, including its tail.
                state.settle(Phase::Entering, 1., Instant::now());
                cx.notify();
                return;
            }
            Some(state) if state.phase == Phase::Handoff => {
                // The document is already full-size. This click starts a new
                // overview rather than only dismissing the invisible handoff.
                let frame = state.target.zip(state.target_frame.clone()).filter(|_| {
                    state.scale == window.scale_factor() && state.viewport == window.viewport_size()
                });
                self.close_file_overview_now(window, cx);
                frame
            }
            Some(_) => {
                self.cancel_file_overview(window, cx);
                return;
            }
            None => None,
        };
        if self.buffers.review().is_some() && self.buffers.inline_close_target().is_none()
            || self.export.is_open()
            || self.split_resize.is_some()
            || self.file_manager.is_resizing_sidebar()
        {
            return;
        }
        let transient_focus = self.command_line_holds_focus(window, cx)
            || self.buffers.panel.is_some()
            || self.focus_workspace_on_render;
        self.close_command_line(cx);
        self.dismiss_buffer_panel(cx);
        self.end_prefix(cx);
        self.background_pending_open(cx);
        let items = self.overview_items(cx);
        let origin = self.document_session().map(|session| session.read(cx).id());
        let presented = self.file_overview.presented.filter(|last| {
            Some(last.id) == origin
                && last.scale == window.scale_factor()
                && last.palette == crate::theme::current_theme().background
                && last.bounds.right() == window.viewport_size().width
                && last.bounds.bottom() == window.viewport_size().height
        });
        let origin_bounds =
            presented.map_or_else(|| self.overview_stage_bounds(window), |last| last.bounds);
        let origin_frame = if self.content_route == ContentRoute::Document {
            // The session may have changed since the last paint. Tag the
            // actual painted revision, never newer text that has not painted.
            presented
                .and_then(|last| {
                    DocumentFrame::from_painted(
                        &self.file_overview.drawn_frame,
                        origin_bounds,
                        last.revision,
                    )
                })
                .or_else(|| {
                    // Handoff may precede the first live paint. Reuse the frame
                    // actually on screen, without tagging old paint as new content.
                    handoff_frame
                        .filter(|(id, frame)| {
                            Some(*id) == origin
                                && frame.source.bounds == origin_bounds
                                && frame.palette == crate::theme::current_theme().background
                                && self.document_session().is_some_and(|session| {
                                    session.read(cx).revision() == frame.revision
                                })
                        })
                        .map(|(_, frame)| frame)
                })
        } else {
            None
        };
        if let (Some(id), Some(frame)) = (origin, &origin_frame) {
            self.file_overview.cache.insert(id, frame.clone());
        }
        let placeholder = self.buffer_text("筛选文件名或路径…", "Filter filenames or paths…");
        let input = cx.new(|cx| {
            NativeInput::new(
                InputConfig {
                    id: "file-overview-query",
                    placeholder: placeholder.into(),
                    outlined: true,
                    font_size: 13.,
                    command: |key, modifiers, _| {
                        matches!(key, "escape" | "enter" | "up" | "down")
                            || modifiers.control && matches!(key, "g" | "n" | "p")
                    },
                    ..InputConfig::default()
                },
                cx,
            )
        });
        let subscription = cx.subscribe(&input, |this, _, event, cx| {
            if !this.file_overview_is_open() {
                return;
            }
            match event {
                InputEvent::Changed(_) => {
                    this.cancel_overview_close(cx);
                    let items = this.overview_items(cx);
                    if let Some(state) = &mut this.file_overview.session {
                        if !items.iter().any(|item| Some(item.id) == state.selected) {
                            state.selected = items.first().map(|item| item.id);
                        }
                        state.scroll.set_offset(point(px(0.), px(0.)));
                        state.focus_query_pending = true;
                    }
                }
                // Commands are intercepted before NativeInput dispatch. They
                // can also arrive through IME/native input, so defer to a frame
                // where a mutable Window is available.
                InputEvent::Command { key, modifiers } => {
                    if let Some(state) = &mut this.file_overview.session {
                        state.pending_command = Some((key.clone(), *modifiers));
                    }
                }
                InputEvent::MetricsChanged => {}
            }
            cx.notify();
        });
        let grid_focus = cx.focus_handle();
        let done_focus = cx.focus_handle();
        let origin_focus = if transient_focus
            || self.content_route == ContentRoute::Document
                && self
                    .file_overview
                    .presented
                    .is_none_or(|last| Some(last.id) != origin)
        {
            None
        } else {
            window.focused(cx)
        };
        self.cancel_pending_document_focus(cx);
        self.focus_workspace_on_render = false;
        self.key_focus_restore = None;
        self.status.dismiss_popover();
        let viewport = window.viewport_size();
        let grid = self.overview_grid(window, &items);
        let origin_index = items
            .iter()
            .position(|item| Some(item.id) == origin)
            .unwrap_or(0);
        let scroll = ScrollHandle::default();
        let offset = grid.clamp_scroll(grid.reveal(origin_index, px(0.)));
        scroll.set_offset(point(px(0.), offset));
        let reduced_motion = cx.reduce_motion();
        let selected = items.get(origin_index).map(|item| item.id);
        window.focus(&grid_focus, cx);
        self.file_overview.serial = self.file_overview.serial.wrapping_add(1);
        self.file_overview.session = Some(Session {
            phase: Phase::Entering,
            progress: 0.,
            motion: Some(Motion::settle(Instant::now(), 0., 1., 0., reduced_motion)),
            handoff_frames: 0,
            handoff_backdrop: 1.,
            gesture: None,
            follow: None,
            outward_probe: None,
            origin,
            origin_route: self.content_route,
            origin_focus,
            origin_bounds,
            origin_card: grid.document_preview(origin_index, offset, origin_bounds.size),
            origin_frame,
            target: None,
            target_from: origin_bounds,
            target_to: origin_bounds,
            target_frame: None,
            new_document: None,
            selected,
            query: input,
            _subscription: subscription,
            grid_focus,
            new_focus: cx.focus_handle(),
            done_focus,
            query_bounds: Bounds::default(),
            focus_query_pending: false,
            pending_command: None,
            scroll,
            viewport,
            scale: window.scale_factor(),
            palette: crate::theme::current_theme().background,
            reduced_motion,
            preparation: None,
            notice: None,
        });
        cx.notify();
    }

    pub(crate) fn close_file_overview_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_overview_close(cx);
        self.file_overview.serial = self.file_overview.serial.wrapping_add(1);
        self.file_overview.probe = None;
        let Some(state) = self.file_overview.session.take() else {
            return;
        };
        if state.phase != Phase::Handoff {
            // External lifecycle actions may have replaced the origin. Restore
            // its focus only when that exact session is still displayed.
            let current = self.document_session().map(|session| session.read(cx).id());
            if current == state.origin && self.content_route == state.origin_route {
                if let Some(focus) = state.origin_focus {
                    window.focus(&focus, cx);
                } else if self.content_route == ContentRoute::Document {
                    self.request_document_focus(cx);
                } else if let Some(focus) = self.focus_handle.clone() {
                    window.focus(&focus, cx);
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn cancel_file_overview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_overview_close(cx);
        let Some(state) = self.file_overview.session.as_mut() else {
            return;
        };
        state.outward_probe = None;
        state.focus_query_pending = false;
        match state.phase {
            Phase::Opening | Phase::Rebounding => {
                state.settle(Phase::Rebounding, 0., Instant::now());
            }
            Phase::Handoff => {
                self.close_file_overview_now(window, cx);
                return;
            }
            _ => {
                state.settle(Phase::Returning, 0., Instant::now());
            }
        }
        cx.notify();
    }

    pub(super) fn new_overview_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_overview_close(cx);
        if self.buffers.panel.is_some() && self.buffers.inline_close_target().is_none()
            || !self.file_overview.session.as_ref().is_some_and(|state| {
                state.phase == Phase::Visible && !state.query.read(cx).is_composing()
            })
        {
            return;
        }
        let items = self.overview_items(cx);
        let grid = self.overview_grid(window, &items);
        let stage = self.overview_stage_bounds(window);
        let state = self.file_overview.session.as_mut().unwrap();
        let offset = state.scroll.offset().y;
        let revealed = grid.reveal(items.len(), offset);
        if revealed != offset {
            state.scroll.set_offset(point(px(0.), revealed));
            let serial = self.file_overview.serial;
            cx.on_next_frame(window, move |this, window, cx| {
                if this.file_overview.serial == serial {
                    this.new_overview_document(window, cx);
                }
            });
            cx.notify();
            return;
        }
        let source = self.overview_source_size(window);
        let from = grid.document_preview(items.len(), offset, source);
        let document = self.prepare_draft(false, cx);
        let id = document.session.read(cx).id();
        let state = self.file_overview.session.as_mut().unwrap();
        state.target = Some(id);
        state.target_from = layout::contain(stage.size, from);
        state.target_to = stage;
        state.target_frame = None;
        state.new_document = Some(document);
        state.phase = Phase::Opening;
        state.progress = 0.;
        state.motion = None;
        state.preparation = None;
        state.outward_probe = None;
        state.focus_query_pending = false;
        state.notice = None;
        // Even without motion, prepare the blank surface before switching away
        // from the overview. The old document must never show through.
        self.prepare_overview_document_frame(id, stage, cx);
    }

    pub(crate) fn file_overview_focus_lost(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.file_overview.probe = None;
        if let Some(state) = self.file_overview.session.as_mut() {
            state.outward_probe = None;
        }
        if self
            .file_overview
            .session
            .as_ref()
            .is_some_and(|state| state.gesture.is_some())
        {
            self.cancel_file_overview(window, cx);
        }
    }

    fn select_overview_item(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.buffer_busy() && self.buffers.inline_close_target() == Some(id) {
            return;
        }
        self.cancel_overview_close(cx);
        if self.buffers.panel.is_some() && self.buffers.inline_close_target().is_none() {
            return;
        }
        let items = self.overview_items(cx);
        let Some(index) = items.iter().position(|item| item.id == id) else {
            return;
        };
        let stage = self.overview_stage_bounds(window);
        let grid = self.overview_grid(window, &items);
        let source_size = self.overview_source_size(window);
        let preparation_request = self.overview_preview_request(id, cx);
        let Some(state) = self.file_overview.session.as_mut() else {
            return;
        };
        if state.phase != Phase::Visible {
            return;
        }
        let offset = state.scroll.offset().y;
        let revealed = grid.reveal(index, offset);
        if revealed != offset {
            state.selected = Some(id);
            state.scroll.set_offset(point(px(0.), revealed));
            let serial = self.file_overview.serial;
            cx.on_next_frame(window, move |this, window, cx| {
                if this.file_overview.serial == serial {
                    this.select_overview_item(id, window, cx);
                }
            });
            cx.notify();
            return;
        }
        let preview = grid.document_preview(index, offset, source_size);
        state.target_from = layout::contain(source_size, preview);
        state.target_to = layout::contain(source_size, stage);
        // Cached frames describe the last visible viewport. Prepare the selected
        // view at today's size/style before using it for a full-size transition.
        state.target_frame = None;
        state.target = Some(id);
        state.selected = Some(id);
        state.phase = Phase::Opening;
        state.focus_query_pending = false;
        state.progress = 0.;
        state.notice = None;
        state.motion = Some(Motion::settle(
            Instant::now(),
            0.,
            1.,
            0.,
            state.reduced_motion,
        ));
        if let Some((path, snapshot)) = preparation_request {
            self.file_overview.serial = self.file_overview.serial.wrapping_add(1);
            let serial = self.file_overview.serial;
            let work = cx.background_executor().spawn(async move {
                Arc::new(crate::preview::derive_preview_incremental(
                    path,
                    snapshot,
                    None,
                    &[],
                ))
            });
            let task = cx.spawn(async move |this, cx| {
                let preview = work.await;
                let _ = this.update(cx, |this, cx| {
                    if this.file_overview.serial != serial
                        || !this.file_overview.session.as_ref().is_some_and(|state| {
                            state.phase == Phase::Opening && state.target == Some(id)
                        })
                    {
                        return;
                    }
                    if !this.install_overview_preview(id, preview, cx) {
                        let state = this.file_overview.session.as_mut().unwrap();
                        state.notice =
                            Some("文件已变化，请重新选择 / File changed; select it again".into());
                        state.settle(Phase::Rebounding, 0., Instant::now());
                    } else if !this.file_overview.session.as_ref().unwrap().reduced_motion {
                        this.prepare_overview_document_frame(id, stage, cx);
                    } else {
                        let state = this.file_overview.session.as_mut().unwrap();
                        state.preparation = None;
                        if state.gesture.is_some() {
                            state.update_gesture_target(Instant::now());
                        } else {
                            state.motion = Some(Motion::settle(Instant::now(), 0., 1., 0., true));
                        }
                    }
                    cx.notify();
                });
            });
            if let Some(state) = self.file_overview.session.as_mut() {
                state.preparation = Some(Preparation {
                    started: Instant::now(),
                    step: PreparationStep::Reading { _task: task },
                });
            }
        } else if !state.reduced_motion {
            self.prepare_overview_document_frame(id, stage, cx);
        }
        cx.notify();
    }

    fn overview_step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.overview_items(cx);
        let grid = self.overview_grid(window, &items);
        let Some(state) = self.file_overview.session.as_mut() else {
            return;
        };
        if state.phase != Phase::Visible || items.is_empty() {
            return;
        }
        let index = items
            .iter()
            .position(|item| Some(item.id) == state.selected)
            .unwrap_or(0);
        let next = index.saturating_add_signed(delta).min(items.len() - 1);
        state.selected = Some(items[next].id);
        let y = grid.reveal(next, state.scroll.offset().y);
        state.scroll.set_offset(point(px(0.), y));
        cx.notify();
    }

    fn overview_key_command(
        &mut self,
        key: &str,
        modifiers: gpui::Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.file_overview.session.as_ref() else {
            return;
        };
        if state.query.read(cx).is_composing() {
            return;
        }
        if key == "escape" || key == "g" && modifiers.control {
            self.cancel_file_overview(window, cx);
            return;
        }
        if state.phase != Phase::Visible {
            return;
        }
        let columns = self.overview_grid(window, &self.overview_items(cx)).columns as isize;
        let query_focused = state.query.read(cx).focus.is_focused(window);
        match key {
            "enter" => {
                if state.new_focus.is_focused(window) {
                    self.new_overview_document(window, cx);
                } else if state.done_focus.is_focused(window) {
                    self.cancel_file_overview(window, cx);
                } else if let Some(id) = state.selected {
                    self.select_overview_item(id, window, cx);
                }
            }
            "up" => self.overview_step(-columns, window, cx),
            "down" => self.overview_step(columns, window, cx),
            "left" if !query_focused => self.overview_step(-1, window, cx),
            "right" if !query_focused => self.overview_step(1, window, cx),
            "n" if modifiers.control => self.overview_step(1, window, cx),
            "p" if modifiers.control => self.overview_step(-1, window, cx),
            "tab" => {
                let focus = if modifiers.shift {
                    if query_focused {
                        &state.new_focus
                    } else if state.done_focus.is_focused(window) {
                        &state.query.read(cx).focus
                    } else if state.new_focus.is_focused(window) {
                        &state.grid_focus
                    } else {
                        &state.done_focus
                    }
                } else if state.grid_focus.is_focused(window) {
                    &state.new_focus
                } else if state.new_focus.is_focused(window) {
                    &state.query.read(cx).focus
                } else if query_focused {
                    &state.done_focus
                } else {
                    &state.grid_focus
                };
                let focus = focus.clone();
                if focus == state.new_focus {
                    let items = self.overview_items(cx);
                    let grid = self.overview_grid(window, &items);
                    state.scroll.set_offset(point(
                        px(0.),
                        grid.reveal(items.len(), state.scroll.offset().y),
                    ));
                }
                window.focus(&focus, cx);
            }
            _ => {}
        }
    }

    pub(crate) fn file_overview_capture_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.file_overview.session.as_ref() else {
            return;
        };
        if state.query.read(cx).is_composing() {
            return;
        }
        let key = event.keystroke.key.as_str();
        let m = event.keystroke.modifiers;
        if m.platform && !m.control && !m.alt && !m.shift && key.eq_ignore_ascii_case("n") {
            self.new_draft(window, cx);
            cx.stop_propagation();
            return;
        }
        if m.control && key == "x" {
            self.close_file_overview_now(window, cx);
            self.key_down(event, window, cx);
            cx.stop_propagation();
            return;
        }
        let editing = state.query.read(cx).focus.is_focused(window);
        if matches!(key, "escape" | "enter" | "up" | "down" | "tab")
            || m.control && matches!(key, "g" | "n" | "p")
            || !editing && matches!(key, "left" | "right")
        {
            self.overview_key_command(key, m, window, cx);
            cx.stop_propagation();
        } else if state.phase != Phase::Visible
            || m.platform
                && (!editing
                    || !matches!(
                        key,
                        "a" | "c"
                            | "v"
                            | "x"
                            | "z"
                            | "left"
                            | "right"
                            | "backspace"
                            | "delete"
                            | "home"
                            | "end"
                    ))
        {
            cx.stop_propagation();
        }
    }

    pub(crate) fn file_overview_pinch(
        &mut self,
        event: &gpui::PinchEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_overview_close(cx);
        if self.buffers.review().is_some() && self.buffers.inline_close_target().is_none() {
            cx.stop_propagation();
            return;
        }
        let now = Instant::now();
        #[cfg(feature = "visual-check")]
        self.file_overview.trace.input(event);
        if self.file_overview_is_open() {
            if let Some(state) = self.file_overview.session.as_mut() {
                if state.motion.is_some()
                    && !state.is_preparing()
                    && ((matches!(state.phase, Phase::Entering | Phase::Returning)
                        && state.origin_frame.is_some())
                        || (matches!(state.phase, Phase::Opening | Phase::Rebounding)
                            && state.target_frame.is_some()))
                    && !matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled)
                    && event.delta.is_finite()
                    && (event.phase == TouchPhase::Started || event.delta != 0.)
                    && event.position.y >= px(super::TITLEBAR_HEIGHT)
                    && !state.query.read(cx).is_composing()
                {
                    state.resume_gesture(now);
                }
            }
            let mut already_integrated = false;
            if self
                .file_overview
                .session
                .as_ref()
                .is_some_and(|state| state.phase == Phase::Visible && state.gesture.is_none())
            {
                let items = self.overview_items(cx);
                let grid = self.overview_grid(window, &items);
                let source = self.overview_source_size(window);
                let state = self.file_overview.session.as_mut().unwrap();
                if state.query.read(cx).is_composing() {
                    return;
                }
                let grid_bounds = Bounds::new(
                    point(px(0.), px(super::TITLEBAR_HEIGHT + grid.header)),
                    size(window.viewport_size().width, px(grid.height)),
                );
                if matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled)
                    || !layout::point_in(grid_bounds, event.position)
                {
                    state.outward_probe = None;
                    cx.stop_propagation();
                    return;
                }
                let offset = state.scroll.offset().y;
                let pointed = items
                    .iter()
                    .enumerate()
                    .find(|(i, _)| {
                        layout::point_in(grid.document_preview(*i, offset, source), event.position)
                    })
                    .map(|(_, item)| item.id);
                let selected_visible = state.selected.filter(|id| {
                    items.iter().enumerate().any(|(index, item)| {
                        item.id == *id
                            && !grid
                                .document_preview(index, offset, source)
                                .intersect(&grid_bounds)
                                .is_empty()
                    })
                });
                if state.outward_probe.is_none() && event.delta > 0.0 {
                    state.outward_probe = pointed
                        .or(selected_visible)
                        .map(|id| (id, Gesture::new(true, now)));
                }
                if let Some((_, probe)) = state.outward_probe.as_mut() {
                    probe.update(event.delta, now);
                }
                let claimed = state
                    .outward_probe
                    .as_ref()
                    .is_some_and(|(_, probe)| probe.claimed());
                if claimed {
                    let (id, gesture) = state.outward_probe.take().unwrap();
                    self.select_overview_item(id, window, cx);
                    if let Some(state) = self.file_overview.session.as_mut() {
                        if state.phase == Phase::Opening {
                            state.begin_gesture(gesture, Instant::now());
                            already_integrated = true;
                        }
                    }
                }
            }
            let now = Instant::now();
            let Some(state) = self.file_overview.session.as_mut() else {
                return;
            };
            if let Some(gesture) = state.gesture.as_mut() {
                if !already_integrated {
                    gesture.update(event.delta, now);
                }
                let finished = matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled);
                let commit = finished && gesture.commits(event.phase, now);
                state.update_gesture_target(now);
                if finished {
                    let phase = match (state.phase == Phase::Opening, commit) {
                        (true, true) => Phase::Opening,
                        (true, false) => Phase::Rebounding,
                        (false, true) => Phase::Entering,
                        (false, false) => Phase::Returning,
                    };
                    state.settle(phase, if commit { 1. } else { 0. }, now);
                }
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        if self.content_route != ContentRoute::Document
            || !matches!(self.state, super::WorkspaceLoadState::Ready { .. })
            || self.buffers.panel.is_some()
            || self.search_is_open()
            || self.command_line_is_open()
            || self.split_resize.is_some()
            || self.file_manager.is_resizing_sidebar()
            || self.export.is_open()
            || !layout::point_in(self.overview_stage_bounds(window), event.position)
            || self
                .editor(self.document_workspace.active_pane)
                .is_some_and(|editor| {
                    let editor = editor.read(cx);
                    editor.has_active_composition() || editor.has_active_pointer_interaction()
                })
            || self
                .reading_panel()
                .is_some_and(|panel| panel.read(cx).has_active_pointer_interaction())
        {
            self.file_overview.probe = None;
            return;
        }
        if matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.file_overview.probe = None;
            return;
        }
        let probe = self
            .file_overview
            .probe
            .get_or_insert_with(|| Gesture::new(false, now));
        probe.update(event.delta, now);
        if probe.claimed() {
            let gesture = self.file_overview.probe.take().unwrap();
            self.open_file_overview(window, cx);
            if let Some(state) = self.file_overview.session.as_mut() {
                state.begin_gesture(gesture, Instant::now());
            }
            cx.stop_propagation();
        }
    }

    pub(crate) fn file_overview_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let palette = crate::theme::current_theme().background;
        if self
            .file_overview
            .session
            .as_ref()
            .is_some_and(|state| state.palette != palette)
        {
            if self.file_overview.session.as_ref().unwrap().phase == Phase::Handoff {
                self.close_file_overview_now(window, cx);
                return;
            }
            // Keep the grid's query, selection and scroll position. Old paint
            // commands contain baked colors, so discard any active transition
            // and repaint visible cached documents with the existing stage.
            self.file_overview.serial = self.file_overview.serial.wrapping_add(1);
            self.file_overview.presented = None;
            let state = self.file_overview.session.as_mut().unwrap();
            state.palette = palette;
            state.phase = Phase::Visible;
            state.progress = 1.;
            state.motion = None;
            state.gesture = None;
            state.follow = None;
            state.outward_probe = None;
            state.origin_frame = None;
            state.target = None;
            state.target_frame = None;
            state.new_document = None;
            state.preparation = None;
        }
        self.finish_overview_document_frame(window, cx);
        if let Some((key, modifiers)) = self
            .file_overview
            .session
            .as_mut()
            .and_then(|state| state.pending_command.take())
        {
            self.overview_key_command(&key, modifiers, window, cx);
        }
        if !self.file_overview_is_open() {
            let current = if self.content_route == ContentRoute::Document
                && matches!(self.state, super::WorkspaceLoadState::Ready { .. })
            {
                self.document_session().map(|session| session.read(cx).id())
            } else {
                None
            };
            if let Some(previous) = self
                .file_overview
                .presented
                .take()
                .filter(|p| Some(p.id) != current)
            {
                if self
                    .buffer_sessions()
                    .any(|doc| doc.read(cx).id() == previous.id)
                    && previous.scale == window.scale_factor()
                    && previous.palette == crate::theme::current_theme().background
                {
                    if let Some(frame) = DocumentFrame::from_painted(
                        &self.file_overview.drawn_frame,
                        previous.bounds,
                        previous.revision,
                    ) {
                        self.file_overview.cache.insert(previous.id, frame);
                    }
                }
            }
            return;
        }
        let items = self.overview_items(cx);
        let all_ids = self
            .buffer_sessions()
            .map(|session| session.read(cx).id())
            .collect::<Vec<_>>();
        let current = self.document_session().map(|session| session.read(cx).id());
        self.file_overview.cache.retain(&all_ids);
        let grid = self.overview_grid(window, &items);
        if let Some(id) = self.buffers.inline_close_target()
            && !self.buffer_busy()
        {
            let state = self.file_overview.session.as_ref().unwrap();
            let viewport = Bounds::new(
                point(px(0.), px(super::TITLEBAR_HEIGHT + grid.header)),
                size(window.viewport_size().width, px(grid.height)),
            );
            let visible = items
                .iter()
                .position(|item| item.id == id)
                .is_some_and(|index| {
                    let bounds = grid
                        .document_preview(
                            index,
                            state.scroll.offset().y,
                            self.overview_source_size(window),
                        )
                        .intersect(&viewport);
                    bounds.size.width > px(0.) && bounds.size.height > px(0.)
                });
            if state.phase != Phase::Visible || !visible {
                self.cancel_overview_close(cx);
            }
        }
        if let Some(state) = self
            .file_overview
            .session
            .as_ref()
            .filter(|state| state.phase == Phase::Visible && state.preparation.is_none())
        {
            let bounds = self.overview_stage_bounds(window);
            let stale = grid
                .visible_range(items.len(), state.scroll.offset().y)
                .filter_map(|index| {
                    let item = &items[index];
                    self.file_overview
                        .cache
                        .needs_refresh(item.id, item.revision, bounds)
                        .then_some(item.id)
                })
                .find(|id| self.overview_preview_request(*id, cx).is_none());
            if let Some(id) = stale {
                self.prepare_overview_document_frame(id, bounds, cx);
            }
        }
        if let Some(state) = self.file_overview.session.as_mut() {
            let reduced = cx.reduce_motion();
            if reduced && !state.reduced_motion {
                state.motion = state
                    .motion
                    .map(|motion| motion.finish_with_reduced_motion(now));
                state.follow = None;
            }
            state.reduced_motion = reduced;
            if state.gesture.is_some() && reduced {
                state.update_gesture_target(now);
            }
            if state.phase == Phase::Visible
                && state.origin.is_some_and(|id| !all_ids.contains(&id))
            {
                // Closing the current card keeps the grid open. Its old paint
                // must never be used to return to the next active document.
                state.origin = current;
                state.origin_route = self.content_route;
                state.origin_focus = None;
                state.origin_frame = None;
                state.focus_query_pending = true;
            }
            if (state.phase == Phase::Handoff
                && (current != state.target || self.content_route != ContentRoute::Document))
                || (state.phase != Phase::Handoff
                    && (current != state.origin
                        || self.content_route != state.origin_route
                        || self.buffers.panel.as_ref().is_some_and(|panel| {
                            !matches!(panel, super::buffers::Panel::Review(review)
                                if matches!(review.kind, super::buffers::ReviewKind::Close(_)))
                        })))
            {
                self.close_file_overview_now(window, cx);
                return;
            }
            if !items.iter().any(|item| Some(item.id) == state.selected) {
                state.selected = items.first().map(|item| item.id);
            }
            // Native wheel deltas can temporarily exceed the content extent.
            // Clamp before choosing virtual rows; GPUI also clamps during
            // prepaint, which is too late to choose this frame's children.
            let offset = state.scroll.offset();
            state
                .scroll
                .set_offset(point(px(0.), grid.clamp_scroll(offset.y)));
            if state.viewport != window.viewport_size() || state.scale != window.scale_factor() {
                self.close_file_overview_now(window, cx);
                self.file_overview.cache.clear();
                return;
            }
            if state.new_document.is_none() && state.target.is_some_and(|id| !all_ids.contains(&id))
            {
                state.phase = Phase::Visible;
                state.progress = 1.;
                state.motion = None;
                state.target = None;
                state.target_frame = None;
                state.gesture = None;
                state.follow = None;
                state.preparation = None;
            }
            if state.focus_query_pending {
                state.focus_query_pending = false;
                let focus = state.query.read(cx).focus.clone();
                window.focus(&focus, cx);
            }
            if state
                .preparation
                .as_ref()
                .is_some_and(|p| p.started.elapsed() > Duration::from_secs(1))
            {
                if state.phase == Phase::Visible {
                    if let Some(Preparation {
                        step: PreparationStep::Painting(drawing),
                        ..
                    }) = state.preparation.take()
                    {
                        self.file_overview
                            .cache
                            .remove(drawing.document.session.read(cx).id());
                    }
                } else {
                    state.notice = Some("准备超时，请重试 / Preparation timed out; retry".into());
                    state.settle(Phase::Rebounding, 0., now);
                }
            }
            if state.phase == Phase::Handoff && state.motion.is_none() {
                if state.handoff_frames >= 2 || state.reduced_motion {
                    state.motion = Some(Motion::fade(now, state.reduced_motion));
                } else {
                    cx.on_next_frame(window, |_, _, cx| cx.notify());
                    return;
                }
            }
            if state.preparation.is_some() {
                if state.is_preparing() {
                    state.progress = 0.;
                }
                cx.on_next_frame(window, |_, _, cx| cx.notify());
                return;
            }
            if let Some(follow) = state.follow {
                let sample = follow.sample(now);
                state.progress = sample.value;
                if sample.active {
                    cx.on_next_frame(window, |_, _, cx| cx.notify());
                }
                return;
            }
            let Some(motion) = state.motion else {
                return;
            };
            let sample = motion.sample(now);
            state.progress = sample.value;
            if sample.active {
                cx.on_next_frame(window, |_, _, cx| cx.notify());
                return;
            }
            state.motion = None;
            match state.phase {
                Phase::Entering | Phase::Rebounding => {
                    state.phase = Phase::Visible;
                    state.progress = 1.;
                    state.target_frame = None;
                    state.new_document = None;
                    cx.on_next_frame(window, |_, _, cx| cx.notify());
                }
                Phase::Returning => {
                    if !state.reduced_motion
                        && state.origin_frame.is_some()
                        && state.origin_route == ContentRoute::Document
                    {
                        // The real body was paused during the zoom. Restore its
                        // focus and paint it behind the opaque, full-size frame
                        // before removing that frame, just as when opening a card.
                        state.target = state.origin;
                        state.target_frame = state.origin_frame.clone();
                        state.target_to = state.origin_bounds;
                        state.phase = Phase::Handoff;
                        state.progress = 0.;
                        state.handoff_frames = 0;
                        // Returning has already revealed the sidebar. Covering
                        // it again at the phase boundary would itself flash.
                        state.handoff_backdrop = 0.;
                        let focus = state.origin_focus.clone();
                        if let Some(focus) = focus {
                            window.focus(&focus, cx);
                        } else {
                            self.request_document_focus(cx);
                        }
                        cx.on_next_frame(window, |_, _, cx| cx.notify());
                    } else {
                        self.close_file_overview_now(window, cx);
                    }
                }
                Phase::Opening => {
                    let target = state.target;
                    let origin = state.origin;
                    let new_document = state.new_document.take();
                    if let Some(target) =
                        target.filter(|id| new_document.is_some() || all_ids.contains(id))
                    {
                        if let Some(document) = new_document {
                            self.activate_prepared_draft(document, cx);
                        } else if origin != Some(target) {
                            self.activate_buffer(target, cx);
                        } else {
                            self.content_route = ContentRoute::Document;
                            let original_focus = self
                                .file_overview
                                .session
                                .as_ref()
                                .filter(|state| state.origin_route == ContentRoute::Document)
                                .and_then(|state| state.origin_focus.clone());
                            if let Some(focus) = original_focus {
                                window.focus(&focus, cx);
                            } else {
                                self.request_document_focus(cx);
                            }
                        }
                        if let Some(state) = &mut self.file_overview.session {
                            state.phase = Phase::Handoff;
                            state.progress = 0.;
                            // Keep the recorded frame fully opaque while the
                            // real target paints with its final focus/layout.
                            state.handoff_frames = 0;
                            state.motion = None;
                        }
                        cx.on_next_frame(window, |_, _, cx| cx.notify());
                    } else {
                        self.close_file_overview_now(window, cx);
                    }
                }
                Phase::Handoff => self.close_file_overview_now(window, cx),
                Phase::Visible => {}
            }
        }
    }
}
