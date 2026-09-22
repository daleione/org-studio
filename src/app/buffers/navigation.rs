//! Confirmed document/heading jumps retain source anchors, not copies of document text.
use super::*;
use crate::{
    app::{ContentRoute, PaneSurface},
    document::{ByteOffset, ByteRange, LineIndex, RevisionRange, Selection, TextSnapshot},
};

const HISTORY_LIMIT: usize = 100;

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;

#[derive(PartialEq)]
pub(crate) struct NavigationLocation {
    document: DocumentId,
    workspace: DocumentWorkspaceState,
    reversed: bool,
    source: RevisionRange,
    scroll: RevisionRange,
    line: u64,
    scroll_line: u64,
    fraction: f32,
    scroll_x: f32,
}

#[derive(Default)]
pub(super) struct NavigationHistory {
    back: Vec<NavigationLocation>,
    forward: Vec<NavigationLocation>,
    restoring: bool,
    pending: Option<NavigationLocation>,
}

impl WorkspaceWindow {
    pub(crate) fn capture_navigation_location(&self, cx: &App) -> Option<NavigationLocation> {
        if self.content_route != ContentRoute::Document {
            return None;
        }
        let session = self.document_session()?.read(cx);
        let snapshot = session.snapshot();
        let pane = self.document_workspace.active_pane;
        let (selection, scroll, fraction, scroll_x) = match self.document_workspace.active_surface()
        {
            PaneSurface::Editor => {
                let editor = self.editor(pane)?;
                let editor = editor.read(cx);
                let (scroll, fraction) = editor.top_source_anchor(&snapshot);
                (editor.selection(), scroll, fraction, editor.scroll_x())
            }
            PaneSurface::Reading => {
                let panel = self.reading_panel_for(pane)?.read(cx);
                let source = panel.top_source_revision_range()?;
                let source = session.map_range_to_current(source).ok()?.range.start;
                let fraction = panel
                    .top_source_anchor()
                    .map_or(0., |(_, offset)| f32::from(offset));
                (Selection::caret(source), source, fraction, 0.)
            }
        };
        Some(NavigationLocation {
            document: session.id(),
            workspace: self.document_workspace,
            reversed: selection.is_reversed(),
            source: snapshot.revision_range(selection.range()),
            scroll: snapshot.revision_range(ByteRange::new(scroll.0, scroll.0)),
            line: snapshot.line_index_at(selection.head()).ok()?.0,
            scroll_line: snapshot.line_index_at(scroll).ok()?.0,
            fraction,
            scroll_x,
        })
    }

    pub(crate) fn remember_navigation_location(&mut self, origin: Option<NavigationLocation>) {
        let history = &mut self.buffers.navigation;
        if history.restoring {
            return;
        }
        history.pending = None;
        let Some(origin) = origin else { return };
        history.forward.clear();
        if history.back.last() == Some(&origin) {
            return;
        }
        history.back.push(origin);
        if history.back.len() > HISTORY_LIMIT {
            history.back.remove(0);
        }
    }

    pub(crate) fn navigate_to_source(&mut self, target: ByteOffset, cx: &mut Context<Self>) {
        self.close_search(false, cx);
        let origin = self.capture_navigation_location(cx);
        self.remember_navigation_location(origin);
        let pane = self.document_workspace.active_pane;
        *self.pending_surface_anchors.get_mut(pane) = None;
        match self.document_workspace.active_surface() {
            PaneSurface::Editor => {
                if let Some(editor) = self.editor(pane) {
                    editor.update(cx, |editor, cx| editor.jump_to_source_offset(target, cx));
                }
            }
            PaneSurface::Reading => {
                if let Some(panel) = self.reading_panel_for(pane) {
                    panel.update(cx, |panel, cx| {
                        panel.reveal_source_offset(target);
                        cx.notify();
                    });
                }
            }
        }
        self.request_document_focus(cx);
        cx.notify();
    }

    pub(crate) fn navigate_history(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.buffer_busy() {
            return;
        }
        let target = loop {
            let history = &mut self.buffers.navigation;
            let target = if forward {
                history.forward.pop()
            } else {
                history.back.pop()
            };
            let Some(target) = target else { return };
            if self.buffer_session(target.document, cx).is_some() {
                break target;
            }
        };
        if let Some(current) = self.capture_navigation_location(cx) {
            let history = &mut self.buffers.navigation;
            let stack = if forward {
                &mut history.back
            } else {
                &mut history.forward
            };
            stack.push(current);
            if stack.len() > HISTORY_LIMIT {
                stack.remove(0);
            }
        }
        self.buffers.navigation.restoring = true;
        self.activate_buffer(target.document, cx);
        self.buffers.navigation.restoring = false;
        self.document_workspace = target.workspace;
        self.reconcile_visible_editor_panes(cx);
        self.reconcile_derived_preview(cx);
        self.buffers.navigation.pending = Some(target);
        self.restore_navigation_location(cx);
        cx.notify();
    }

    pub(super) fn restore_navigation_location(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.buffers.navigation.pending.as_ref() else {
            return;
        };
        let Some(session) = self.document_session().cloned() else {
            return;
        };
        if session.read(cx).id() != target.document || self.document_workspace != target.workspace {
            self.buffers.navigation.pending = None;
            return;
        }
        if self.document_workspace.active_surface() == PaneSurface::Reading
            && !self.latest_preview_is_current(cx)
        {
            return;
        }
        let target = self.buffers.navigation.pending.take().unwrap();
        let session = session.read(cx);
        let snapshot = session.snapshot();
        // A reload can outlive the edit map. Fall back to the nearest surviving source line.
        let map = |range, line: u64| {
            session
                .map_range_to_current(range)
                .map(|r| r.range)
                .unwrap_or_else(|_| {
                    let start = snapshot
                        .line_content_range(LineIndex(
                            line.min(snapshot.len_lines().saturating_sub(1)),
                        ))
                        .map_or(ByteOffset(0), |r| r.start);
                    ByteRange::new(start.0, start.0)
                })
        };
        let source = map(target.source, target.line);
        let scroll = map(target.scroll, target.scroll_line);
        let pane = target.workspace.active_pane;
        *self.pending_surface_anchors.get_mut(pane) = None;
        match target.workspace.active_surface() {
            PaneSurface::Editor => {
                if let Some(editor) = self.editor(pane) {
                    editor.update(cx, |editor, cx| {
                        editor.set_selection(
                            if target.reversed {
                                Selection::new(source.end, source.start)
                            } else {
                                Selection::new(source.start, source.end)
                            },
                            cx,
                        );
                        editor.restore_scroll(scroll.start, target.fraction, target.scroll_x, cx);
                    });
                }
            }
            PaneSurface::Reading => {
                if let Some(panel) = self.reading_panel_for(pane) {
                    panel.update(cx, |panel, cx| {
                        panel.scroll_to_source_offset_with_offset(
                            scroll.start,
                            gpui::px(target.fraction),
                        );
                        cx.notify();
                    });
                }
            }
        }
        self.request_document_focus(cx);
    }

    pub(crate) fn navigation_can_go(&self, forward: bool, cx: &App) -> bool {
        let history = &self.buffers.navigation;
        let stack = if forward {
            &history.forward
        } else {
            &history.back
        };
        stack
            .iter()
            .any(|target| self.buffer_session(target.document, cx).is_some())
    }

    pub(crate) fn navigation_back_label(&self, cx: &App) -> Option<String> {
        let target = self
            .buffers
            .navigation
            .back
            .iter()
            .rev()
            .find(|target| self.buffer_session(target.document, cx).is_some())?;
        let session = self.buffer_session(target.document, cx)?;
        let session = session.read(cx);
        let line = session
            .map_range_to_current(target.source)
            .ok()
            .and_then(|source| {
                let head = if target.reversed {
                    source.range.start
                } else {
                    source.range.end
                };
                session.snapshot().line_index_at(head).ok()
            })
            .map_or(target.line, |line| line.0);
        Some(format!("{} · {}", session.display_name(), line + 1))
    }
}
