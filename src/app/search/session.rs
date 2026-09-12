use crate::{
    app::{PaneSide, PaneSurface, native_input::NativeInput},
    document::{ByteOffset, ByteRange, DocumentId, Revision, RevisionRange, Selection},
    search::{Completion, Query, Results},
};
use gpui::Entity;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
pub(crate) struct SearchHost {
    pub(super) session: Option<Session>,
    pub(crate) presentation: Option<super::presentation::Presentation>,
    pub(super) next_id: u64,
    pub(super) history: Vec<String>,
}
#[derive(Clone)]
pub(super) struct Step {
    pub(super) successful: bool,
    pub(super) query: String,
    pub(super) current: Option<ByteRange>,
    pub(super) backwards: bool,
    pub(super) boundary: bool,
}
pub(super) struct Session {
    pub(super) id: u64,
    pub(super) generation: u64,
    pub(super) document: DocumentId,
    pub(super) revision: Revision,
    pub(super) pane: PaneSide,
    pub(super) surface: PaneSurface,
    pub(super) preview_pending: bool,
    pub(super) pending_navigation: Vec<bool>,
    pub(super) mode: SearchMode,
    pub(super) backwards: bool,
    pub(super) boundary: bool,
    pub(super) query: Query,
    pub(super) scope: SearchScope,
    pub(super) current: Option<ByteRange>,
    pub(super) origin: RevisionRange,
    pub(super) scroll: RevisionRange,
    pub(super) scroll_fraction: f32,
    pub(super) scroll_x: f32,
    pub(super) selection: Selection,
    pub(super) selection_revision: Revision,
    pub(super) after_replace: bool,
    pub(super) planning: bool,
    pub(super) range_blocked: bool,
    pub(super) _subscriptions: Vec<gpui::Subscription>,
    pub(super) steps: Vec<Step>,
    pub(super) results: Option<MatchSet>,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) task: Option<gpui::Task<()>>,
    pub(super) input: Entity<NativeInput>,
    pub(super) replacement_input: Entity<NativeInput>,
    pub(super) replacement: String,
    pub(super) progress: ByteOffset,
    pub(super) focus_pending: bool,
    pub(super) replacement_focus_pending: bool,
    pub(super) more_open: bool,
    pub(super) notice: super::notice::Notice,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SearchScope {
    WholeDocument,
    Selection,
    FromAnchor,
}
impl SearchScope {
    pub(super) fn label(self, language: crate::i18n::Language) -> &'static str {
        match self {
            Self::WholeDocument => language.text("search.scope_full"),
            Self::Selection => language.text("search.scope_selection"),
            Self::FromAnchor => language.text("search.scope_anchor"),
        }
    }
}
pub(super) struct MatchSet {
    pub(super) matches: Arc<[ByteRange]>,
    pub(super) completion: Completion,
}
impl From<Results> for MatchSet {
    fn from(results: Results) -> Self {
        Self {
            matches: results.matches.into(),
            completion: results.completion,
        }
    }
}
impl Session {
    pub(super) fn current_index(&self) -> Option<usize> {
        let current = self.current?;
        let matches = &self.results.as_ref()?.matches;
        matches
            .binary_search_by_key(&(current.start, current.end), |r| (r.start, r.end))
            .ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SearchMode {
    Find,
    Incremental,
    Replace,
    QueryReplaceInput,
    QueryReplaceConfirm,
}
impl SearchMode {
    pub fn is_incremental(self) -> bool {
        self == Self::Incremental
    }
    pub fn replacing(self) -> bool {
        matches!(
            self,
            Self::Replace | Self::QueryReplaceInput | Self::QueryReplaceConfirm
        )
    }
    pub fn is_query_replace(self) -> bool {
        matches!(self, Self::QueryReplaceInput | Self::QueryReplaceConfirm)
    }
    pub fn confirming(self) -> bool {
        self == Self::QueryReplaceConfirm
    }
    pub fn stop_confirming(&mut self) {
        if self.confirming() {
            *self = Self::QueryReplaceInput;
        }
    }
}
