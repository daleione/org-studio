use std::sync::Arc;

use gpui::Task;

#[derive(Clone, Debug)]
pub(crate) enum SaveStatus {
    Saving,
    Success {
        document: crate::document::DocumentId,
        revision: crate::document::Revision,
        at: std::time::Instant,
    },
    Error(Arc<str>),
}

#[derive(Clone, Debug, Default)]
pub(crate) enum SaveInteraction {
    #[default]
    Idle,
    Prompt,
    Saving,
    AllowCloseOnce,
}

#[derive(Default)]
pub(crate) struct SaveHost {
    pub(crate) task: Option<Task<()>>,
    pub(crate) feedback_task: Option<Task<()>>,
    pub(crate) dialog_task: Option<Task<()>>,
    pub(crate) status: Option<SaveStatus>,
    pub(crate) interaction: SaveInteraction,
    pub(crate) close_hook_installed: bool,
}
