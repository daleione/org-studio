//! Product-level window ownership.
//!
//! Feature modules implement their focused command and rendering adapters, while the product shell
//! owns routing, document lifecycle handles, and window-scoped settings in one canonical type.

use std::{path::PathBuf, sync::Arc, time::Instant};

use gpui::{FocusHandle, Subscription, Task};

use crate::{
    command::CommandRegistry,
    input::{ContextSet, KeyboardRouter},
    preview::{
        ContentRoute, ExportHost, FileManagerHost, PreviewLoadState, SaveHost, ScrollBenchmark,
        StatusLineHost,
    },
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DocumentMode {
    #[default]
    Source,
    Split,
    Preview,
}

impl DocumentMode {
    pub(crate) fn from_environment(fallback: Self) -> Self {
        std::env::var("ORG_STUDIO_DOCUMENT_MODE")
            .ok()
            .and_then(|mode| match mode.to_ascii_lowercase().as_str() {
                "source" => Some(Self::Source),
                "split" => Some(Self::Split),
                "preview" => Some(Self::Preview),
                _ => None,
            })
            .unwrap_or(fallback)
    }
}

pub struct WorkspaceWindow {
    pub(crate) language: crate::i18n::Language,
    pub(crate) focus_handle: Option<FocusHandle>,
    pub(crate) focus_workspace_on_render: bool,
    pub(crate) focus_lost_subscription: Option<Subscription>,
    pub(crate) commands: Arc<CommandRegistry>,
    pub(crate) keyboard: KeyboardRouter,
    pub(crate) key_context: ContextSet,
    pub(crate) state: PreviewLoadState,
    pub(crate) document_subscription: Option<Subscription>,
    pub(crate) subscribed_document: Option<crate::document::DocumentId>,
    pub(crate) recent_documents: Vec<crate::recent_documents::RecentDocument>,
    pub(crate) home_error: Option<Arc<str>>,
    pub(crate) generation: u64,
    pub(crate) load_task: Option<Task<()>>,
    pub(crate) derived: DerivedHost,
    pub(crate) split_scroll: SplitScrollHost,
    pub(crate) file_watch_task: Option<Task<()>>,
    pub(crate) file_watch_request: u64,
    pub(crate) file_watch_directory: Option<PathBuf>,
    pub(crate) file_watch_target: Option<crate::file_watcher::FileWatchTarget>,
    pub(crate) file_manager: FileManagerHost,
    pub(crate) picker_task: Option<Task<()>>,
    pub(crate) export: ExportHost,
    pub(crate) save: SaveHost,
    pub(crate) list_overdraw: f32,
    pub(crate) opened_at: Option<Instant>,
    pub(crate) first_frame_scheduled: Option<u64>,
    pub(crate) scroll_benchmark: Option<ScrollBenchmark>,
    pub(crate) which_key_task: Option<Task<()>>,
    pub(crate) which_key_request: u64,
    pub(crate) key_feedback_task: Option<Task<()>>,
    pub(crate) key_feedback_request: u64,
    pub(crate) which_key_items: Arc<Vec<(Arc<str>, Arc<str>)>>,
    pub(crate) content_route: ContentRoute,
    pub(crate) document_mode: DocumentMode,
    pub(crate) soft_wrap: bool,
    pub(crate) minimap_visible: bool,
    pub(crate) minimap_thumb_visibility: crate::settings::MinimapThumbVisibility,
    pub(crate) minimap_width: Option<u16>,
    pub(crate) minimap_resize_preview: Option<f32>,
    pub(crate) status: StatusLineHost,
}

#[derive(Default)]
pub(crate) struct DerivedHost {
    pub(crate) task: Option<Task<()>>,
    pub(crate) sender: Option<async_channel::Sender<crate::preview::derived::DerivedRequest>>,
    pub(crate) published: Option<(crate::document::DocumentId, crate::document::Revision)>,
}

#[derive(Default)]
pub(crate) struct SplitScrollHost {
    pub(crate) source_subscription: Option<Subscription>,
    pub(crate) panel_revision: Option<(crate::document::DocumentId, crate::document::Revision)>,
    pub(crate) source_anchor: Option<(crate::document::ByteOffset, u32)>,
    pub(crate) preview_anchor: Option<(crate::document::ByteOffset, u32)>,
}
