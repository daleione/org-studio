//! Open document lifetimes and the statusline document switcher.
//!
//! The displayed document lives in WorkspaceLoadState. Inactive documents are moved here,
//! retaining their sessions and view entities; text and undo history are never copied.
use crate::{
    app::{DocumentWorkspaceState, ReadyDocument, WorkspaceWindow},
    document::{DocumentId, DocumentSession},
};
use gpui::{App, Context, Entity, Subscription, Task};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

mod navigation;
mod picker;
pub(crate) use picker::match_score;
mod pool;
mod quick_view;
mod review;
mod review_view;
#[cfg(test)]
mod tests;
mod view;
mod watch;

gpui::actions!(
    buffers,
    [
        NewDocument,
        NewDraft,
        SwitchBuffer,
        CloseBuffer,
        SaveBuffers,
        NextBuffer,
        PreviousBuffer,
        NavigateBack,
        NavigateForward
    ]
);

pub(crate) struct ParkedDocument {
    document: ReadyDocument,
    workspace: DocumentWorkspaceState,
    soft_wrap: bool,
    preview: Option<Arc<crate::preview::PreviewSnapshot>>,
}

#[derive(Default)]
pub(crate) struct BufferHost {
    navigation: navigation::NavigationHistory,
    parked: Vec<ParkedDocument>,
    open_order: Vec<OpenEntry>,
    pub(crate) panel: Option<Panel>,
    outgoing: Option<OutgoingPanel>,
    pub(crate) focus_pending: bool,
    pub(crate) pane: crate::app::PaneSide,
    pub(crate) available: f32,
    pub(crate) height: f32,
    pub(crate) scroll: gpui::ScrollHandle,
    pub(crate) loads: std::collections::HashMap<PathBuf, Task<()>>,
    pub(crate) watch_task: Option<Task<()>>,
    watch_paths: Vec<PathBuf>,
    pub(crate) draft_serial: u64,
    file_candidates: Vec<picker::Candidate>,
    file_task: Option<Task<()>>,
    file_request: u64,
    cycle: Vec<DocumentId>,
}

enum OpenEntry {
    Pending(PathBuf),
    Open(DocumentId),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerIntent {
    Switch,
    Close,
    File,
    New,
}

pub(crate) struct Picker {
    intent: PickerIntent,
    input: Entity<crate::app::native_input::NativeInput>,
    _subscription: Subscription,
    selected: usize,
    pending_confirm: bool,
    message: Option<String>,
    return_search: bool,
    markdown: bool,
}
pub(crate) enum Panel {
    Picker(Picker),
    Review(SaveReview),
}

enum OutgoingPanel {
    Picker,
    Review(Box<SaveReview>),
}

impl BufferHost {
    pub(crate) fn returning(&self) -> bool {
        self.outgoing.is_some()
    }

    pub(crate) fn finish_return(&mut self) {
        self.outgoing = None;
    }

    /// Reserve a position before loading so completion timing cannot reorder files.
    pub(crate) fn reserve_open(&mut self, path: &Path) {
        if !self
            .open_order
            .iter()
            .any(|entry| matches!(entry, OpenEntry::Pending(pending) if same_file(pending, path)))
        {
            self.open_order.push(OpenEntry::Pending(path.to_path_buf()));
        }
    }

    pub(crate) fn register_opened(&mut self, document: &DocumentSession) {
        let id = document.id();
        if let Some(entry) = self.open_order.iter_mut().find(|entry| match entry {
            OpenEntry::Open(opened) => *opened == id,
            OpenEntry::Pending(path) => document
                .file_path()
                .is_some_and(|opened| same_file(path, opened)),
        }) {
            *entry = OpenEntry::Open(id);
        } else {
            self.open_order.push(OpenEntry::Open(id));
        }
    }

    pub(crate) fn cancel_open(&mut self, path: &Path) {
        self.open_order.retain(
            |entry| !matches!(entry, OpenEntry::Pending(pending) if same_file(pending, path)),
        );
    }

    pub(crate) fn forget_opened(&mut self, id: DocumentId) {
        self.open_order
            .retain(|entry| !matches!(entry, OpenEntry::Open(opened) if *opened == id));
    }

    pub(crate) fn opened_ids(&self) -> impl Iterator<Item = DocumentId> + '_ {
        self.open_order.iter().filter_map(|entry| match entry {
            OpenEntry::Open(id) => Some(*id),
            OpenEntry::Pending(_) => None,
        })
    }

    pub(crate) fn inline_close_target(&self) -> Option<DocumentId> {
        match self.review()? {
            SaveReview {
                kind: ReviewKind::Close(id),
                inline: true,
                ..
            } => Some(*id),
            _ => None,
        }
    }

    pub(crate) fn input_composing(&self, cx: &App) -> bool {
        matches!(&self.panel, Some(Panel::Picker(picker)) if picker.input.read(cx).is_composing())
    }

    pub(crate) fn review(&self) -> Option<&SaveReview> {
        match &self.panel {
            Some(Panel::Review(review)) => Some(review),
            _ => None,
        }
    }

    pub(crate) fn review_mut(&mut self) -> Option<&mut SaveReview> {
        match &mut self.panel {
            Some(Panel::Review(review)) => Some(review),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReviewKind {
    Save,
    Close(DocumentId),
    Quit,
    Window,
}

pub(crate) struct SaveReview {
    pub(crate) kind: ReviewKind,
    pub(crate) entries: Vec<ReviewEntry>,
    pub(crate) running: bool,
    pub(crate) error: Option<String>,
    pub(crate) selected: usize,
    pub(crate) inline: bool,
    focus: [gpui::FocusHandle; ReviewControl::ALL.len()],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReviewControl {
    All,
    List,
    Discard,
    Submit,
}

impl ReviewControl {
    const ALL: [Self; 4] = [Self::All, Self::List, Self::Discard, Self::Submit];
    const SINGLE: [Self; 2] = [Self::Discard, Self::Submit];
    const BATCH: [Self; 3] = [Self::All, Self::List, Self::Submit];
}

impl SaveReview {
    fn single_confirmation(&self) -> bool {
        self.kind != ReviewKind::Save && self.entries.len() == 1
    }

    fn controls(&self) -> &[ReviewControl] {
        if self.single_confirmation() {
            &ReviewControl::SINGLE
        } else {
            &ReviewControl::BATCH
        }
    }

    fn focus(&self, control: ReviewControl) -> &gpui::FocusHandle {
        &self.focus[control as usize]
    }

    fn selection_counts(&self) -> (usize, usize) {
        let save = self.entries.iter().filter(|e| e.save && !e.done).count();
        let discard = self.entries.iter().filter(|e| !e.save && !e.done).count();
        (save, discard)
    }

    fn destructive_submit(&self) -> bool {
        let (save, discard) = self.selection_counts();
        self.kind != ReviewKind::Save && save == 0 && discard > 0
    }

    fn default_control(&self) -> ReviewControl {
        if !self.single_confirmation() && self.destructive_submit() {
            ReviewControl::All
        } else {
            ReviewControl::Submit
        }
    }

    fn focused_control(&self, window: &gpui::Window) -> ReviewControl {
        self.controls()
            .iter()
            .copied()
            .find(|&control| self.focus(control).is_focused(window))
            .unwrap_or_else(|| self.default_control())
    }
}

pub(crate) struct ReviewEntry {
    pub(crate) id: DocumentId,
    pub(crate) revision: crate::document::Revision,
    pub(crate) save: bool,
    pub(crate) done: bool,
}

impl WorkspaceWindow {
    pub(crate) fn buffer_sessions(&self) -> impl Iterator<Item = Entity<DocumentSession>> + '_ {
        self.document_session().cloned().into_iter().chain(
            self.buffers
                .parked
                .iter()
                .map(|d| d.document.session.clone()),
        )
    }

    pub(crate) fn buffer_session(
        &self,
        id: DocumentId,
        cx: &App,
    ) -> Option<Entity<DocumentSession>> {
        self.buffer_sessions().find(|s| s.read(cx).id() == id)
    }

    pub(crate) fn buffer_for_path(&self, path: &Path, cx: &App) -> Option<Entity<DocumentSession>> {
        if let super::save::SaveInteraction::Renaming { document, target } = &self.save.interaction
            && same_file(target, path)
        {
            return self.buffer_session(*document, cx);
        }
        self.buffer_sessions().find(|s| {
            let s = s.read(cx);
            s.file_path().is_some_and(|p| same_file(p, path))
                || matches!(s.save_state(), crate::document::SaveState::Saving { target, .. } if same_file(target, path))
        })
    }

    pub(crate) fn buffer_text(&self, zh: &'static str, en: &'static str) -> &'static str {
        match self.language {
            crate::i18n::Language::Chinese => zh,
            crate::i18n::Language::English => en,
        }
    }

    pub(crate) fn buffer_busy(&self) -> bool {
        self.buffers.review().is_some_and(|r| r.running)
    }

    pub(crate) fn buffer_navigation_blocked(&self) -> bool {
        self.buffers
            .review()
            .is_some_and(|r| r.running && !r.inline)
    }
}

pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    let normalized = |p: &Path| {
        let absolute = if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(p)
        };
        if let (Some(parent), Some(name)) = (absolute.parent(), absolute.file_name())
            && let Ok(parent) = std::fs::canonicalize(parent)
        {
            return parent.join(name);
        }
        absolute
    };
    if normalized(a) == normalized(b) {
        return true;
    }
    if let (Ok(a), Ok(b)) = (std::fs::canonicalize(a), std::fs::canonicalize(b))
        && a == b
    {
        return true;
    }
    #[cfg(unix)]
    if let (Ok(a), Ok(b)) = (std::fs::metadata(a), std::fs::metadata(b)) {
        use std::os::unix::fs::MetadataExt;
        return a.dev() == b.dev() && a.ino() == b.ino();
    }
    false
}
