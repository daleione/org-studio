use std::sync::Arc;

use gpui::Task;

#[derive(Clone, Copy)]
pub(crate) enum AfterRename {
    Nothing,
    Save,
    SaveAs,
}

#[derive(Clone, Debug, Default)]
pub(crate) enum SaveInteraction {
    #[default]
    Idle,
    Prompt,
    Saving,
    Renaming {
        document: crate::document::DocumentId,
        target: std::path::PathBuf,
    },
    AllowCloseOnce,
}

#[derive(Default)]
pub(crate) struct SaveHost {
    pub(crate) task: Option<Task<()>>,
    pub(crate) dialog_task: Option<Task<()>>,
    /// Last save failure, surfaced transiently by the status line.
    pub(crate) error: Option<Arc<str>>,
    pub(crate) interaction: SaveInteraction,
    pub(crate) close_hook_installed: bool,
}
