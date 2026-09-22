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
mod pool;
mod quick_view;
mod review;
#[cfg(test)]
mod tests;
mod view;
mod watch;

gpui::actions!(
    buffers,
    [
        NewDocument,
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
    pub(crate) panel: Option<Panel>,
    pub(crate) returning: bool,
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

impl BufferHost {
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
