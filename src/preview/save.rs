use std::{path::PathBuf, sync::Arc};

use gpui::Task;

#[derive(Clone, Debug)]
pub(crate) enum SaveStatus {
    Saving(Arc<str>),
    Error(Arc<str>),
}

#[derive(Clone, Debug)]
pub(crate) enum PendingTransition {
    Close,
    Quit,
    Open(PathBuf),
    Home,
}

#[derive(Clone, Debug, Default)]
pub(crate) enum SaveInteraction {
    #[default]
    Idle,
    GuardPrompt(PendingTransition),
    ConflictPrompt(Option<PendingTransition>),
    SaveAsPrompt(Option<PendingTransition>),
    Saving(Option<PendingTransition>),
    AllowCloseOnce,
}

#[derive(Default)]
pub(crate) struct SaveHost {
    pub(crate) task: Option<Task<()>>,
    pub(crate) dialog_task: Option<Task<()>>,
    pub(crate) status: Option<SaveStatus>,
    pub(crate) interaction: SaveInteraction,
    pub(crate) close_hook_installed: bool,
}
