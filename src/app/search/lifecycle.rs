use super::presentation::{Presentation, PresentationPhase};
use super::session::{SearchScope, Session};
use crate::{
    app::{ContentRoute, PaneSurface, WorkspaceWindow},
    document::{ByteOffset, ByteRange, RevisionRange, Selection, TextSnapshot},
    search::{CaseMode, Query},
};
use gpui::{Context, Modifiers, px};
use std::sync::{Arc, atomic::AtomicBool};

impl WorkspaceWindow {
    pub(crate) fn focus_search_input(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = &mut self.search.session {
            session.focus_pending = true;
            cx.notify();
        }
    }

    pub(crate) fn search_input_composing(&self, cx: &gpui::App) -> bool {
        self.search.session.as_ref().is_some_and(|s| {
            s.input.read(cx).is_composing() || s.replacement_input.read(cx).is_composing()
        })
    }
    pub(crate) fn open_search(
        &mut self,
        isearch: bool,
        backwards: bool,
        replace: bool,
        cx: &mut Context<Self>,
    ) {
        self.close_command_line(cx);
        if self.buffers.panel.is_some() {
            self.cancel_buffer_panel(cx);
        }
        if self.content_route != ContentRoute::Document {
            return;
        }
        self.end_prefix(cx);
        if self.search.session.is_some() {
            self.search_key(
                "f",
                Modifiers {
                    platform: true,
                    ..Modifiers::default()
                },
                cx,
            );
            return;
        }
        let Some(doc) = self.document_session().cloned() else {
            return;
        };
        if doc.read(cx).is_read_only() {
            return;
        }
        let snapshot = doc.read(cx).snapshot();
        let pane = self.document_workspace.active_pane;
        let selection = self
            .editor(pane)
            .filter(|_| self.document_workspace.active_surface() == PaneSurface::Editor)
            .map(|e| e.read(cx).selection())
            .unwrap_or_default();
        let anchor = if self.document_workspace.active_surface() == PaneSurface::Editor {
            selection.head()
        } else {
            self.reading_panel_for(pane)
                .and_then(|p| p.read(cx).top_source_revision_range())
                .map_or(ByteOffset(0), |r| r.range.start)
        };
        let scroll = self
            .editor(pane)
            .filter(|_| self.document_workspace.active_surface() == PaneSurface::Editor)
            .map(|e| e.read(cx).top_source_anchor(&snapshot).0)
            .unwrap_or(anchor);
        let reading_scroll = self
            .reading_panel_for(pane)
            .and_then(|p| p.read(cx).top_source_anchor());
        let scroll = if self.document_workspace.active_surface() == PaneSurface::Reading {
            reading_scroll.map_or(scroll, |r| r.0)
        } else {
            scroll
        };
        let pattern = if !isearch
            && !replace
            && !selection.is_empty()
            && self.document_workspace.active_surface() == PaneSurface::Editor
        {
            let text = snapshot.copy_range(selection.range());
            if !text.contains(['\n', '\r']) {
                text
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        let scope = if replace {
            if selection.is_empty() {
                ByteRange::new(anchor.0, snapshot.len_bytes())
            } else {
                selection.range()
            }
        } else {
            ByteRange::new(0, snapshot.len_bytes())
        };
        let (input, query_subscription) = self.create_search_input(false, cx);
        input.update(cx, |i, cx| {
            i.append_only = isearch;
            i.sync(&pattern, cx);
        });
        let (replacement_input, replacement_subscription) = self.create_search_input(true, cx);
        self.search.next_id += 1;
        let subscription = cx.subscribe(&doc, |this, _, event, cx| {
            if let crate::document::DocumentEvent::Edited { delta, .. }
            | crate::document::DocumentEvent::Reloaded { delta, .. } = event
            {
                this.search_document_changed(delta, cx);
            }
        });
        let surface = self.document_workspace.active_surface();
        let presentation = self
            .search
            .presentation
            .get_or_insert_with(|| Presentation::new(pane, surface, snapshot.document_id()));
        if presentation.pane != pane || presentation.document != snapshot.document_id() {
            *presentation = Presentation::new(pane, surface, snapshot.document_id());
        }
        presentation.surface = surface;
        presentation.phase = PresentationPhase::Search;
        self.search.session = Some(Session {
            id: self.search.next_id,
            generation: 0,
            document: snapshot.document_id(),
            revision: snapshot.revision(),
            pane,
            surface: self.document_workspace.active_surface(),
            preview_pending: false,
            pending_navigation: Vec::new(),
            mode: if replace {
                super::session::SearchMode::QueryReplaceInput
            } else if isearch {
                super::session::SearchMode::Incremental
            } else {
                super::session::SearchMode::Find
            },
            backwards,
            boundary: false,
            query: Query {
                pattern,
                case: CaseMode::Auto,
                scope,
            },
            scope: if replace {
                if selection.is_empty() {
                    SearchScope::FromAnchor
                } else {
                    SearchScope::Selection
                }
            } else {
                SearchScope::WholeDocument
            },
            current: None,
            origin: snapshot.revision_range(ByteRange::new(anchor.0, anchor.0)),
            scroll: snapshot.revision_range(ByteRange::new(scroll.0, scroll.0)),
            scroll_fraction: if self.document_workspace.active_surface() == PaneSurface::Reading {
                reading_scroll.map_or(0., |r| f32::from(r.1))
            } else {
                self.editor(pane)
                    .map_or(0., |e| e.read(cx).top_source_anchor(&snapshot).1)
            },
            scroll_x: self.editor(pane).map_or(0., |e| e.read(cx).scroll_x()),
            selection,
            selection_revision: snapshot.revision(),
            after_replace: false,
            planning: false,
            range_blocked: false,
            _subscriptions: vec![subscription, query_subscription, replacement_subscription],
            steps: vec![],
            results: None,
            cancel: Arc::new(AtomicBool::new(false)),
            task: None,
            input,
            replacement_input,
            replacement: String::new(),
            progress: anchor,
            focus_pending: true,
            replacement_focus_pending: false,
            more_open: false,
            notice: Default::default(),
        });
        self.keyboard.cancel();
        self.keyboard.dismiss_status();
        let bindings = [
            (
                "C-s",
                self.commands
                    .key("org-studio.document.isearch-forward")
                    .unwrap(),
            ),
            (
                "C-r",
                self.commands
                    .key("org-studio.document.isearch-backward")
                    .unwrap(),
            ),
        ];
        let _ = self.keyboard.install_transient(
            self.search.next_id,
            &bindings,
            crate::input::TransientPolicy {
                exit_after_command: false,
                exit_after_undefined: false,
                ..Default::default()
            },
            std::time::Instant::now(),
        );
        self.cancel_pending_document_focus(cx);
        self.status.dismiss_popover();
        self.start_search(cx);
    }

    pub(crate) fn search_is_open(&self) -> bool {
        self.search.session.is_some()
    }

    pub(super) fn search_show_source(&mut self, cx: &mut Context<Self>) {
        let Some(mut session) = self.search.session.take() else {
            return;
        };
        if let Some(panel) = self.reading_panel_for(session.pane) {
            panel.update(cx, |panel, cx| {
                panel.search_finish(false);
                cx.notify();
            });
        }
        // Keep the same query, range, mode, results and input entities across surfaces.
        self.show_editor(cx);
        session.surface = PaneSurface::Editor;
        if let Some(editor) = self.editor(session.pane) {
            let editor = editor.read(cx);
            let snapshot = editor.session().read(cx).snapshot();
            session.selection = editor.selection();
            session.selection_revision = snapshot.revision();
            let (offset, fraction) = editor.top_source_anchor(&snapshot);
            session.scroll =
                snapshot.revision_range(crate::document::ByteRange::new(offset.0, offset.0));
            session.scroll_fraction = fraction;
            session.scroll_x = editor.scroll_x();
        }
        session.focus_pending = !session.mode.replacing();
        session.replacement_focus_pending = session.mode.replacing();
        session.more_open = false;
        if let Some(p) = &mut self.search.presentation {
            p.surface = PaneSurface::Editor;
        }
        self.search.session = Some(session);
        self.update_search_preview(cx);
        cx.notify();
    }

    pub(crate) fn close_search(&mut self, cancel: bool, cx: &mut Context<Self>) {
        let Some(s) = self.search.session.take() else {
            return;
        };
        if !s.query.pattern.is_empty() {
            self.search.history.retain(|q| q != &s.query.pattern);
            self.search.history.push(s.query.pattern.clone());
            if self.search.history.len() > 50 {
                self.search.history.remove(0);
            }
        }
        if let Some(editor) = self.editor(s.pane).filter(|e| {
            s.surface == PaneSurface::Editor && e.read(cx).session().read(cx).id() == s.document
        }) {
            editor.update(cx, |e, cx| {
                e.search_finish(cancel, cx);
                if cancel {
                    if let Ok(mapped) =
                        e.session()
                            .read(cx)
                            .map_range_to_current(RevisionRange::new(
                                s.selection_revision,
                                s.selection.range(),
                            ))
                    {
                        e.set_selection(
                            if s.selection.anchor() <= s.selection.head() {
                                Selection::new(mapped.range.start, mapped.range.end)
                            } else {
                                Selection::new(mapped.range.end, mapped.range.start)
                            },
                            cx,
                        );
                    }
                    if let Ok(scroll) = e.session().read(cx).map_range_to_current(s.scroll) {
                        e.restore_scroll(scroll.range.start, s.scroll_fraction, s.scroll_x, cx);
                    }
                } else if e.session().read(cx).id() == s.document
                    && e.session().read(cx).revision() == s.revision
                    && let Some(r) = s.current
                {
                    e.set_selection(
                        if s.mode.is_incremental() {
                            Selection::caret(if s.backwards { r.start } else { r.end })
                        } else {
                            Selection::new(r.start, r.end)
                        },
                        cx,
                    );
                }
            });
        }
        if let Some(panel) = self.reading_panel_for(s.pane) {
            panel.update(cx, |p, cx| {
                p.search_finish(cancel);
                cx.notify();
            });
        }
        if cancel
            && self.document_workspace.surface(s.pane) == PaneSurface::Reading
            && let Some(doc) = self.document_session()
            && let Ok(r) = doc.read(cx).map_range_to_current(s.scroll)
            && let Some(panel) = self.reading_panel_for(s.pane)
        {
            panel.update(cx, |p, cx| {
                p.scroll_to_source_offset_with_offset(r.range.start, px(s.scroll_fraction));
                cx.notify();
            });
        }
        self.keyboard.cancel();
        self.keyboard.dismiss_status();
        self.keyboard.clear_transient();
        if self.content_route == ContentRoute::Document
            && self.document_workspace.active_pane == s.pane
            && self
                .document_session()
                .is_some_and(|d| d.read(cx).id() == s.document)
        {
            self.request_document_focus(cx);
        }
        if let Some(presentation) = &mut self.search.presentation {
            if !cx.reduce_motion() && presentation.available_width > 0. {
                presentation.phase =
                    PresentationPhase::Returning(s.bar_snapshot(self.language, cx));
            } else {
                self.search.presentation = None;
            }
        }
        // Dropping Session releases the scan, subscriptions and native inputs now.
        cx.notify();
    }
}
