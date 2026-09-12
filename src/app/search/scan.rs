use super::notice::Notice;
use crate::{
    app::WorkspaceWindow,
    document::{ByteOffset, TextSnapshot},
    search::{self, Completion},
};
use gpui::Context;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

impl WorkspaceWindow {
    pub(super) fn start_search(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.document_session().cloned() else {
            return;
        };
        let snapshot = doc.read(cx).snapshot();
        let Some(s) = self.search.session.as_mut() else {
            return;
        };
        s.cancel.store(true, Ordering::Relaxed);
        s.cancel = Arc::new(AtomicBool::new(false));
        s.generation += 1;
        s.planning = false;
        s.revision = snapshot.revision();
        s.results = None;
        s.notice = Notice::Scanning;
        let id = s.id;
        let generation = s.generation;
        let cancel = s.cancel.clone();
        let query = s.query.clone();
        if query.pattern.is_empty() {
            s.current = None;
            s.notice = Notice::None;
            self.update_search_preview(cx);
            cx.notify();
            return;
        }
        let (sender, receiver) = async_channel::bounded(1);
        let work = cx.background_executor().spawn(async move {
            let result =
                search::scan_progress(&snapshot, &query, &cancel, search::MATCH_LIMIT, |batch| {
                    let _ = sender.try_send(batch);
                });
            let _ = sender.send(result).await;
        });
        s.task = Some(cx.spawn(async move |weak, cx| {
            while let Ok(results) = receiver.recv().await {
                let _ = weak.update(cx, |this, cx| {
                    let Some(doc) = this.document_session() else {
                        return;
                    };
                    let document = doc.read(cx).id();
                    let revision = doc.read(cx).revision();
                    let Some(s) = this.search.session.as_mut() else {
                        return;
                    };
                    if s.id != id
                        || s.generation != generation
                        || s.document != document
                        || s.revision != revision
                        || results.completion == Completion::Cancelled
                    {
                        return;
                    }
                    if results.completion == Completion::Partial && s.backwards {
                        return;
                    }
                    let anchor = if s.mode.is_query_replace() || s.after_replace {
                        s.progress
                    } else {
                        s.origin.range.start
                    };
                    let last_success = s.current;
                    let keep = s.current.and_then(|old| {
                        let index = if s.backwards {
                            results.matches.binary_search_by_key(&old.end, |r| r.end)
                        } else {
                            results
                                .matches
                                .binary_search_by_key(&old.start, |r| r.start)
                        };
                        index.ok().map(|i| results.matches[i])
                    });
                    s.current = keep.or_else(|| {
                        let index = if s.backwards {
                            results
                                .matches
                                .partition_point(|r| r.end <= anchor)
                                .checked_sub(1)
                        } else {
                            Some(results.matches.partition_point(|r| r.start < anchor))
                        };
                        index.and_then(|i| results.matches.get(i)).copied()
                    });
                    if results.completion == Completion::Complete
                        && s.current.is_none()
                        && !s.mode.is_incremental()
                        && !s.mode.is_query_replace()
                        && !s.after_replace
                    {
                        s.current = if s.backwards {
                            results.matches.last().copied()
                        } else {
                            results.matches.first().copied()
                        };
                    }
                    s.notice = if results.completion == Completion::Partial {
                        Notice::Scanning
                    } else if results.matches.is_empty() {
                        Notice::NotFound
                    } else if s.current.is_none() {
                        Notice::Boundary
                    } else {
                        Notice::None
                    };
                    if s.mode.is_incremental() && results.matches.is_empty() {
                        s.current = last_success;
                    }
                    if results.completion != Completion::Partial {
                        s.after_replace = false;
                    }
                    let complete = results.completion != Completion::Partial;
                    s.results = Some(results.into());
                    let pending = if complete {
                        std::mem::take(&mut s.pending_navigation)
                    } else {
                        Vec::new()
                    };
                    this.update_search_preview(cx);
                    for backwards in pending {
                        this.search_navigate(backwards, cx);
                    }
                    cx.notify();
                });
            }
            work.await;
        }));
        self.update_search_preview(cx);
        cx.notify();
    }

    pub(super) fn search_document_changed(
        &mut self,
        delta: &crate::document::RevisionDelta,
        cx: &mut Context<Self>,
    ) {
        let Some(s) = self.search.session.as_mut() else {
            return;
        };
        if s.revision == delta.after {
            return;
        } // Our replacement already advanced the state atomically.
        s.cancel.store(true, Ordering::Relaxed);
        s.current = None;
        s.results = None;
        s.steps.clear();
        if s.scope != super::session::SearchScope::WholeDocument {
            let mapped = (s.revision == delta.before)
                .then(|| search::map_scope(s.query.scope, delta))
                .flatten();
            if let Some(scope) = mapped {
                let shift = scope.start.0 as i128 - s.query.scope.start.0 as i128;
                s.progress = ByteOffset((s.progress.0 as i128 + shift).max(0) as u64);
                s.query.scope = scope;
            } else {
                s.range_blocked = true;
                s.mode.stop_confirming();
                s.notice = Notice::ScopeInvalid;
            }
        }
        s.revision = delta.after;
        if s.range_blocked {
            self.update_search_preview(cx);
            cx.notify();
        } else {
            if let Some(doc) = self.document_session().cloned() {
                let s = self.search.session.as_mut().unwrap();
                if s.scope == super::session::SearchScope::WholeDocument {
                    s.query.scope =
                        crate::document::ByteRange::new(0, doc.read(cx).snapshot().len_bytes());
                }
                if let Ok(origin) = doc.read(cx).map_range_to_current(s.origin) {
                    s.origin = origin;
                }
            }
            self.start_search(cx);
        }
    }
}
