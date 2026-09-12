use super::{
    geometry::{BarLayout, ShellMotion, ShellShape},
    session::Session,
};
use crate::{
    app::{PaneSide, PaneSurface},
    document::DocumentId,
    search::Completion,
};
use gpui::{Context, Window};
use std::time::Instant;

/// Plain display data. A returning shell holds no inputs, subscriptions, tasks or results.
#[derive(Clone)]
pub(super) struct BarSnapshot {
    pub query: String,
    pub replacement: String,
    pub query_empty: bool,
    pub replacement_empty: bool,
    pub replacement_expanded: bool,
    pub query_replace: bool,
    pub count: String,
    pub detail: Option<String>,
    pub failed: bool,
    pub range_blocked: bool,
    pub planning: bool,
    pub more_open: bool,
    pub can_navigate: bool,
    pub can_replace: bool,
    pub can_replace_all: bool,
}

pub(super) enum PresentationPhase {
    Search,
    Returning(BarSnapshot),
}
pub(crate) struct Presentation {
    pub pane: PaneSide,
    pub surface: PaneSurface,
    pub document: DocumentId,
    pub available_width: f32,
    pub motion: ShellMotion,
    pub(super) phase: PresentationPhase,
}
impl Presentation {
    pub fn new(pane: PaneSide, surface: PaneSurface, document: DocumentId) -> Self {
        Self {
            pane,
            surface,
            document,
            available_width: 0.,
            motion: ShellMotion::default(),
            phase: PresentationPhase::Search,
        }
    }
    pub fn returning(&self) -> bool {
        matches!(self.phase, PresentationPhase::Returning(_))
    }
}
impl BarSnapshot {
    pub fn shape(
        &self,
        query_width: f32,
        replacement_width: f32,
        available: f32,
        layout: BarLayout,
    ) -> ShellShape {
        let replacement = if self.replacement_expanded {
            layout.replacement_height
        } else {
            0.
        };
        ShellShape {
            width: super::geometry::content_width(
                query_width,
                self.replacement_expanded.then_some(replacement_width),
                available,
                layout,
            ),
            height: crate::app::status_line::FLOATING_STATUS_HEIGHT
                + replacement
                + if self.detail.is_some() { 26. } else { 0. },
            expansion_height: replacement,
            content_opacity: 1.,
        }
    }
}
impl Session {
    pub(super) fn failed(&self) -> bool {
        !self.query.pattern.is_empty()
            && self
                .results
                .as_ref()
                .is_some_and(|r| r.completion == Completion::Complete && r.matches.is_empty())
    }
    pub(super) fn bar_snapshot(
        &self,
        language: crate::i18n::Language,
        cx: &gpui::App,
    ) -> BarSnapshot {
        let failed = self.failed();
        let count = if self.query.pattern.is_empty() {
            self.scope.label(language).into()
        } else if failed {
            language.text("search.not_found").into()
        } else if let Some(r) = &self.results {
            match r.completion {
                Completion::Partial => format!("{}+", r.matches.len()),
                Completion::Truncated => "100,000+".into(),
                _ => format!(
                    "{} / {}",
                    self.current_index().map_or(0, |i| i + 1),
                    r.matches.len()
                ),
            }
        } else {
            language.text("search.scanning").into()
        };
        let detail = if failed {
            None
        } else if self.surface == PaneSurface::Reading && self.mode.replacing() {
            Some(language.text("search.needs_editor").to_owned())
        } else if self.mode.confirming() {
            Some(language.text("search.confirm_hint").to_owned())
        } else if self.mode.is_query_replace() {
            Some(language.text("search.query_replace_hint").to_owned())
        } else {
            self.notice.detail(language)
        };
        let can_replace = self.mode.replacing()
            && !self.planning
            && !self.range_blocked
            && self.current.is_some()
            && self.surface == PaneSurface::Editor;
        BarSnapshot {
            query: self.input.read(cx).display_text().to_string(),
            replacement: self.replacement_input.read(cx).display_text().to_string(),
            query_empty: self.input.read(cx).text.is_empty(),
            replacement_empty: self.replacement_input.read(cx).text.is_empty(),
            replacement_expanded: self.mode.replacing(),
            query_replace: self.mode.is_query_replace(),
            count,
            detail,
            failed,
            range_blocked: self.range_blocked,
            planning: self.planning,
            more_open: self.more_open,
            can_navigate: !self.query.pattern.is_empty() && !self.planning && !failed,
            can_replace,
            can_replace_all: can_replace
                && self
                    .results
                    .as_ref()
                    .is_some_and(|r| r.completion == Completion::Complete),
        }
    }
}

impl crate::app::WorkspaceWindow {
    pub(super) fn search_presentation_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = &self.search.presentation else {
            return;
        };
        let valid = self.content_route == crate::app::ContentRoute::Document
            && p.pane == self.document_workspace.active_pane
            && p.surface == self.document_workspace.surface(p.pane)
            && self
                .document_session()
                .is_some_and(|d| d.read(cx).id() == p.document);
        if !valid {
            self.search.presentation = None;
            return;
        }
        let viewport = f32::from(window.viewport_size().width);
        let document_width = if self.file_manager.sidebar_visible() {
            viewport
                - self.rendered_sidebar_width(viewport)
                - crate::app::file_manager::sidebar::RESIZE_HANDLE_PX
        } else {
            viewport
        };
        let pane_width = if self.document_workspace.is_split() {
            let left = self.rendered_left_pane_width(document_width);
            if p.pane == PaneSide::Left {
                left
            } else {
                document_width - left - crate::app::split_layout::RESIZE_HANDLE_PX
            }
        } else {
            document_width
        };
        let available = (pane_width - 2. * crate::app::status_line::FLOATING_STATUS_INSET).max(1.);
        let target = self.search.session.as_ref().map_or_else(
            || ShellShape::status(available),
            |s| {
                let snapshot = s.bar_snapshot(self.language, cx);
                snapshot.shape(
                    s.input.read(cx).content_width,
                    s.replacement_input.read(cx).content_width,
                    available,
                    BarLayout::new(available, self.language),
                )
            },
        );
        let p = self.search.presentation.as_mut().unwrap();
        let now = Instant::now();
        p.available_width = available;
        p.motion.update(target, available, now, !cx.reduce_motion());
        let (_, active) = p.motion.sample(available, now);
        if active {
            cx.on_next_frame(window, |_, _, cx| cx.notify());
        } else if p.returning() {
            self.search.presentation = None;
        }
    }
}
