use super::notice::Notice;
use crate::{
    app::{PaneSurface, WorkspaceWindow},
    document::{ByteOffset, DocumentCommand, EditOrigin, Selection},
    search::{self, Completion},
};
use gpui::Context;

impl WorkspaceWindow {
    pub(super) fn search_toggle_replacement(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.search.session.as_mut() else {
            return;
        };
        if s.planning {
            return;
        }
        let expand = !s.mode.replacing();
        if s.mode.is_query_replace() || s.mode.confirming() {
            s.notice = Notice::None;
        }
        s.mode = if expand {
            super::session::SearchMode::Replace
        } else {
            super::session::SearchMode::Find
        };
        s.input.update(cx, |i, _| i.append_only = false);
        s.replacement_focus_pending = s.mode.replacing();
        s.focus_pending = !s.mode.replacing();
        cx.notify();
    }

    pub(super) fn search_replace(&mut self, all: bool, cx: &mut Context<Self>) {
        let Some(doc) = self.document_session().cloned() else {
            return;
        };
        let Some(s) = self.search.session.as_mut() else {
            return;
        };
        if self.document_workspace.surface(s.pane) != PaneSurface::Editor {
            s.notice = Notice::NeedsEditor;
            cx.notify();
            return;
        }
        let Some(results) = &s.results else { return };
        if all && results.completion != Completion::Complete {
            s.notice = Notice::Incomplete;
            cx.notify();
            return;
        }
        if s.range_blocked || doc.read(cx).revision() != s.revision {
            s.notice = Notice::DocumentChanged;
            cx.notify();
            return;
        }
        let ranges = if all {
            results
                .matches
                .iter()
                .copied()
                .filter(|r| {
                    !s.mode.is_query_replace() || s.current.is_some_and(|c| r.start >= c.start)
                })
                .collect::<Vec<_>>()
        } else {
            s.current.into_iter().collect()
        };
        if ranges.is_empty() {
            return;
        }
        let replacement = s.replacement.clone();
        let snapshot = doc.read(cx).snapshot();
        if s.planning {
            return;
        }
        s.planning = true;
        s.notice = Notice::Planning(ranges.len());
        let id = s.id;
        let generation = s.generation;
        let revision = s.revision;
        let query = s.query.clone();
        let replacement_len = replacement.len();
        let work = cx.background_executor().spawn(async move {
            search::replacement_plan(&snapshot, &query, &ranges, &replacement)
        });
        s.task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |w, cx| {
                let Some(doc) = w.document_session().cloned() else {
                    return;
                };
                let Some(s) = w.search.session.as_mut() else {
                    return;
                };
                if s.id != id
                    || s.generation != generation
                    || s.revision != revision
                    || doc.read(cx).revision() != revision
                    || doc.read(cx).id() != s.document
                {
                    return;
                }
                s.planning = false;
                match result {
                    Ok(plan) => w.commit_search_plan(plan, all, replacement_len, cx),
                    Err(e) => {
                        s.notice = Notice::PlanError(e);
                        cx.notify();
                    }
                }
            });
        }));
        cx.notify();
    }

    fn commit_search_plan(
        &mut self,
        plan: crate::document::EditTransaction,
        all: bool,
        replacement_len: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.document_session().cloned() else {
            return;
        };
        let Some(s) = self.search.session.as_mut() else {
            return;
        };
        let ranges = plan.edits.iter().map(|e| e.range).collect::<Vec<_>>();
        let before = s.selection;
        let end = if all {
            ranges[0].start.0
        } else {
            ranges[0].start.0 + replacement_len as u64
        };
        let after = Selection::caret(ByteOffset(end));
        match doc.update(cx, |d, cx| {
            d.edit(
                DocumentCommand::new(plan, before, after, EditOrigin::Other),
                cx,
            )
        }) {
            Ok(delta) => {
                let shift: i128 = delta
                    .edits
                    .iter()
                    .map(|e| e.new_len as i128 - e.old.len() as i128)
                    .sum();
                s.query.scope.end = ByteOffset((s.query.scope.end.0 as i128 + shift) as u64);
                s.progress = ByteOffset(end);
                s.after_replace = true;
                s.revision = delta.after;
                s.current = None;
                s.selection = after;
                s.selection_revision = delta.after;
                if all && s.mode.is_query_replace() {
                    s.mode.stop_confirming();
                    s.progress = s.query.scope.end;
                }
                self.start_search(cx);
            }
            Err(e) => {
                s.notice = Notice::EditError(e.to_string());
                cx.notify();
            }
        }
    }
}
