//! Product-level window ownership.
//!
//! Feature modules implement their focused command and rendering adapters, while the product shell
//! owns routing, document lifecycle handles, and window-scoped settings in one canonical type.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

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
pub enum PaneSurface {
    #[default]
    Editor,
    Reading,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PaneSide {
    #[default]
    Left,
    Right,
}

impl PaneSide {
    pub(crate) const fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WorkspaceLayout {
    #[default]
    Single,
    Split,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DocumentWorkspaceState {
    pub(crate) layout: WorkspaceLayout,
    pub(crate) active_pane: PaneSide,
    left_surface: PaneSurface,
    right_surface: PaneSurface,
}

impl Default for DocumentWorkspaceState {
    fn default() -> Self {
        Self {
            layout: WorkspaceLayout::Single,
            active_pane: PaneSide::Left,
            left_surface: PaneSurface::Editor,
            right_surface: PaneSurface::Reading,
        }
    }
}

impl DocumentWorkspaceState {
    pub(crate) const fn surface(self, pane: PaneSide) -> PaneSurface {
        match pane {
            PaneSide::Left => self.left_surface,
            PaneSide::Right => self.right_surface,
        }
    }

    pub(crate) fn set_surface(&mut self, pane: PaneSide, surface: PaneSurface) {
        match pane {
            PaneSide::Left => self.left_surface = surface,
            PaneSide::Right => self.right_surface = surface,
        }
    }

    pub(crate) const fn active_surface(self) -> PaneSurface {
        self.surface(self.active_pane)
    }

    pub(crate) const fn is_split(self) -> bool {
        matches!(self.layout, WorkspaceLayout::Split)
    }

    pub(crate) fn pane_is_visible(self, pane: PaneSide) -> bool {
        self.is_split() || self.active_pane == pane
    }

    pub(crate) fn shows(self, pane: PaneSide, surface: PaneSurface) -> bool {
        self.pane_is_visible(pane) && self.surface(pane) == surface
    }

    pub(crate) const fn needs_reading(self) -> bool {
        match self.layout {
            WorkspaceLayout::Single => matches!(self.active_surface(), PaneSurface::Reading),
            WorkspaceLayout::Split => {
                matches!(self.left_surface, PaneSurface::Reading)
                    || matches!(self.right_surface, PaneSurface::Reading)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DocumentViewPreferences {
    /// Left-pane share in basis points. Rendering applies viewport safety clamps without changing
    /// the stored preference.
    pub(crate) split_ratio: u16,
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
    pub(crate) editor_minimap_width_subscriptions: Vec<Subscription>,
    pub(crate) subscribed_document: Option<crate::document::DocumentId>,
    pub(crate) recent_documents: Vec<crate::recent_documents::RecentDocument>,
    pub(crate) home_error: Option<Arc<str>>,
    pub(crate) generation: u64,
    pub(crate) load_task: Option<Task<()>>,
    pub(crate) derived: DerivedHost,
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
    pub(crate) document_workspace: DocumentWorkspaceState,
    pub(crate) document_view_preferences: DocumentViewPreferences,
    pub(crate) split_resize: Option<crate::preview::SplitResizeSession>,
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
    pub(crate) sender: Option<async_channel::Sender<()>>,
    pub(crate) pending: Arc<Mutex<Option<crate::preview::derived::DerivedRequest>>>,
    pub(crate) latest: Option<Arc<crate::preview::PreviewSnapshot>>,
}
