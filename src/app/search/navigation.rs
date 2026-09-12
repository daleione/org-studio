use super::notice::Notice;
use super::session::Step;
use crate::{
    app::{ContentRoute, PaneSurface, WorkspaceWindow},
    document::{ByteRange, RevisionRange, TextSnapshot},
    search::Completion,
};
use gpui::{Context, Window};
use std::{sync::Arc, time::Instant};

impl WorkspaceWindow {
    pub(super) fn search_navigate(&mut self, backwards: bool, cx: &mut Context<Self>) {
        let Some(s) = self.search.session.as_mut() else {
            return;
        };
        if s.planning {
            return;
        }
        if s.query.pattern.is_empty() {
            if let Some(last) = self.search.history.last().cloned() {
                s.query.pattern = last;
                s.backwards = backwards;
                s.input.update(cx, |i, cx| i.sync(&s.query.pattern, cx));
                self.start_search(cx);
            }
            return;
        }
        if s.results
            .as_ref()
            .is_none_or(|r| r.completion == Completion::Partial)
        {
            if s.pending_navigation.len() < 256 {
                s.pending_navigation.push(backwards);
            }
            return;
        }
        let Some(results) = &s.results else { return };
        if s.mode.is_incremental() {
            s.steps.push(Step {
                successful: s.query.pattern.is_empty() || s.current_index().is_some(),
                query: s.query.pattern.clone(),
                current: s.current,
                backwards: s.backwards,
                boundary: s.boundary,
            });
            if backwards != s.backwards {
                s.backwards = backwards;
                s.boundary = false;
                cx.notify();
                return;
            }
        }
        s.backwards = backwards;
        let anchor = s
            .current
            .map(|r| if backwards { r.start } else { r.end })
            .unwrap_or(s.origin.range.start);
        if s.mode.is_query_replace() {
            s.progress = anchor;
        }
        let index = if backwards {
            results
                .matches
                .partition_point(|r| r.end <= anchor)
                .checked_sub(1)
        } else {
            Some(results.matches.partition_point(|r| r.start < anchor))
        };
        let next = index.and_then(|i| results.matches.get(i)).copied();
        if let Some(next) = next {
            s.current = Some(next);
            s.boundary = false;
            s.notice = Notice::None;
        } else if results.completion == Completion::Partial {
            s.notice = Notice::Scanning;
        } else if s.mode.is_query_replace() {
            s.current = None;
            s.notice = Notice::ReplaceDone;
            s.mode.stop_confirming();
        } else if results.completion == Completion::Truncated {
            s.notice = Notice::Truncated;
        } else if !s.mode.is_incremental() || s.boundary {
            s.current = if backwards {
                results.matches.last()
            } else {
                results.matches.first()
            }
            .copied();
            s.boundary = false;
            s.notice = Notice::None;
        } else {
            s.boundary = true;
            s.notice = Notice::WrapBoundary;
        }
        self.update_search_preview(cx);
        cx.notify();
    }

    pub(super) fn update_search_preview(&mut self, cx: &mut Context<Self>) {
        let Some(s) = &self.search.session else {
            return;
        };
        let pane = s.pane;
        let current = s.current;
        let ranges: Arc<[ByteRange]> = s
            .results
            .as_ref()
            .map(|r| r.matches.clone())
            .unwrap_or_default();
        if self.document_workspace.surface(pane) == PaneSurface::Editor {
            if let Some(editor) = self.editor(pane) {
                editor.update(cx, |e, cx| e.search_preview(ranges, current, cx));
            }
        } else if let Some(panel) = self.reading_panel_for(pane) {
            let ready = panel.read(cx).document().revision == s.revision
                && panel.read(cx).document().document_id == s.document;
            panel.update(cx, |p, cx| {
                if ready {
                    p.search_preview(ranges, current)
                } else {
                    p.search_preview(Arc::from([]), None)
                };
                cx.notify();
            });
            self.search.session.as_mut().unwrap().preview_pending = !ready;
        }
    }

    pub(crate) fn search_render_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = &self.search.session else {
            return;
        };
        let valid = self.content_route == ContentRoute::Document
            && self.document_workspace.active_pane == s.pane
            && self.document_workspace.surface(s.pane) == s.surface
            && self
                .document_session()
                .is_some_and(|d| d.read(cx).id() == s.document);
        if !valid {
            self.close_search(false, cx);
            return;
        }
        let revision = self.document_session().unwrap().read(cx).revision();
        if revision != s.revision {
            if s.scope != super::session::SearchScope::WholeDocument {
                let s = self.search.session.as_mut().unwrap();
                s.current = None;
                s.results = None;
                s.range_blocked = true;
                s.mode.stop_confirming();
                s.notice = Notice::DocumentChanged;
                s.revision = revision;
                self.update_search_preview(cx);
                return;
            }
            let doc = self.document_session().unwrap().clone();
            let s = self.search.session.as_mut().unwrap();
            s.current = None;
            s.query.scope = ByteRange::new(0, doc.read(cx).snapshot().len_bytes());
            if let Ok(anchor) = doc.read(cx).map_range_to_current(s.origin) {
                s.origin = anchor;
            } else {
                s.origin = RevisionRange::new(revision, ByteRange::new(0, 0));
            }
            self.start_search(cx);
        }
        if self.search.session.as_ref().is_some_and(|s| {
            s.preview_pending
                && self
                    .reading_panel_for(s.pane)
                    .is_some_and(|p| p.read(cx).document().revision == s.revision)
        }) {
            self.update_search_preview(cx);
        }
        let s = self.search.session.as_mut().unwrap();
        let animating = self.search.presentation.as_ref().is_some_and(|p| {
            let shape = self
                .status
                .shell
                .motion
                .sample(p.available_width, Instant::now())
                .0;
            let target = if s.mode.replacing() {
                super::geometry::BarLayout::new(p.available_width, self.language).replacement_height
            } else {
                0.
            };
            (shape.expansion_height - target).abs() > 0.5
        });
        let failed = s.failed();
        s.input.update(cx, |input, cx| {
            let placeholder = self.language.text("search.query_placeholder");
            if input.config.placeholder.as_ref() != placeholder {
                input.config.placeholder = placeholder.into();
                cx.notify();
            }
            if input.invalid != failed {
                input.invalid = failed;
                cx.notify();
            }
        });
        let enabled = s.mode.replacing() && !animating;
        s.replacement_input.update(cx, |input, cx| {
            let placeholder = self.language.text("search.replacement_placeholder");
            if input.config.placeholder.as_ref() != placeholder {
                input.config.placeholder = placeholder.into();
                cx.notify();
            }
            if input.enabled != enabled {
                input.enabled = enabled;
                cx.notify();
            }
        });
        let owns_shell = self
            .status
            .shell
            .owns(crate::app::status_line::shell::ShellKind::Search, s.pane);
        if owns_shell && std::mem::take(&mut s.focus_pending) {
            let focus = s.input.read(cx).focus.clone();
            window.focus(&focus, cx);
        }
        if owns_shell
            && !animating
            && std::mem::take(&mut s.replacement_focus_pending)
            && s.mode.replacing()
        {
            let focus = s.replacement_input.read(cx).focus.clone();
            window.focus(&focus, cx);
        }
        let pane = s.pane;
        let clearance = self.search.presentation.as_ref().map_or(
            crate::app::status_line::FLOATING_STATUS_CLEARANCE,
            |p| {
                self.status
                    .shell
                    .motion
                    .sample(p.available_width, Instant::now())
                    .0
                    .height
                    + crate::app::status_line::FLOATING_STATUS_BOTTOM
                    + 12.
            },
        );
        if let Some(editor) = self.editor(pane) {
            editor.update(cx, |editor, cx| {
                editor.search_overlay_clearance(clearance, cx)
            });
        }
    }
}
