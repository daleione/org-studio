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
};

use echo_area::EchoAreaHost;
use export_ui::ExportHost;
use file_manager::FileManagerHost;
use save::SaveHost;
use status_line::StatusLineHost;
use workspace::ScrollBenchmark;

mod agenda;
mod command_window;
mod derived;
mod echo_area;
pub(crate) mod export_ui;
mod file_manager;
mod home;
mod overlays;
mod render;
mod save;
mod split_layout;
pub(crate) mod status_line;
mod workspace;

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
    split_initialized: bool,
}

impl Default for DocumentWorkspaceState {
    fn default() -> Self {
        Self {
            layout: WorkspaceLayout::Single,
            active_pane: PaneSide::Left,
            left_surface: PaneSurface::Editor,
            right_surface: PaneSurface::Reading,
            split_initialized: false,
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

    pub(crate) fn enter_split(&mut self) -> Option<(PaneSide, PaneSide)> {
        let inheritance =
            (!self.split_initialized).then(|| (self.active_pane, self.active_pane.other()));
        self.split_initialized = true;
        self.layout = WorkspaceLayout::Split;
        inheritance
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContentRoute {
    Document,
    FileManager,
    Agenda,
    AgendaText,
}

#[derive(Clone)]
pub(crate) enum DiredStatus {
    Working(Arc<str>),
    Success(Arc<str>),
    Error(Arc<str>),
}

impl DiredStatus {
    pub(crate) fn message(&self) -> Arc<str> {
        match self {
            Self::Working(message) | Self::Success(message) | Self::Error(message) => {
                message.clone()
            }
        }
    }
}

pub(crate) enum WorkspaceLoadState {
    Empty,
    Loading {
        path: PathBuf,
        previous: Option<ReadyDocument>,
    },
    Ready {
        document: ReadyDocument,
    },
    Failed {
        path: PathBuf,
        message: String,
        previous: Option<ReadyDocument>,
    },
}

#[derive(Clone)]
pub(crate) struct ReadyDocument {
    pub(crate) session: gpui::Entity<crate::document::DocumentSession>,
    pub(crate) editor_syntax: Arc<crate::editor::EditorSyntaxService>,
    pub(crate) editors: PanePair<Option<gpui::Entity<crate::editor::SemanticEditor>>>,
    pub(crate) readers: PanePair<Option<gpui::Entity<crate::preview::ReadingPreviewPanel>>>,
}

#[derive(Clone)]
pub(crate) struct PanePair<T> {
    pub(crate) left: T,
    pub(crate) right: T,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SurfaceAnchor {
    pub(crate) document_id: crate::document::DocumentId,
    pub(crate) source: crate::document::RevisionRange,
}

impl<T> PanePair<T> {
    pub(crate) fn get(&self, pane: PaneSide) -> &T {
        match pane {
            PaneSide::Left => &self.left,
            PaneSide::Right => &self.right,
        }
    }

    pub(crate) fn get_mut(&mut self, pane: PaneSide) -> &mut T {
        match pane {
            PaneSide::Left => &mut self.left,
            PaneSide::Right => &mut self.right,
        }
    }
}

impl WorkspaceLoadState {
    pub(crate) fn ready(&self) -> Option<&ReadyDocument> {
        match self {
            Self::Ready { document } => Some(document),
            Self::Loading { previous, .. } | Self::Failed { previous, .. } => previous.as_ref(),
            Self::Empty => None,
        }
    }

    pub(crate) fn take_ready(&mut self) -> Option<ReadyDocument> {
        match std::mem::replace(self, Self::Empty) {
            Self::Ready { document } => Some(document),
            Self::Loading { previous, .. } | Self::Failed { previous, .. } => previous,
            Self::Empty => None,
        }
    }

    pub(crate) fn ready_mut(&mut self) -> Option<&mut ReadyDocument> {
        match self {
            Self::Ready { document } => Some(document),
            Self::Loading { previous, .. } | Self::Failed { previous, .. } => previous.as_mut(),
            Self::Empty => None,
        }
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
    pub(crate) state: WorkspaceLoadState,
    pub(crate) echo: EchoAreaHost,
    pub(crate) document_subscription: Option<Subscription>,
    pub(crate) editor_minimap_width_subscriptions: Vec<Subscription>,
    pub(crate) subscribed_document: Option<crate::document::DocumentId>,
    pub(crate) recent_documents: Vec<crate::recent_documents::RecentDocument>,
    pub(crate) home_error: Option<Arc<str>>,
    pub(crate) generation: u64,
    pub(crate) pending_navigation: Option<(u64, Arc<str>)>,
    pub(crate) pending_surface_anchors: PanePair<Option<SurfaceAnchor>>,
    pub(crate) load_task: Option<Task<()>>,
    pub(crate) babel_task: Option<Task<()>>,
    pub(crate) babel_editor: Option<gpui::Entity<crate::editor::SemanticEditor>>,
    pub(crate) babel_request: u64,
    pub(crate) derived: DerivedHost,
    pub(crate) file_watch_task: Option<Task<()>>,
    pub(crate) file_watch_request: u64,
    pub(crate) file_watch_directory: Option<PathBuf>,
    pub(crate) file_watch_target: Option<crate::file_watcher::FileWatchTarget>,
    pub(crate) file_manager: FileManagerHost,
    pub(crate) agenda: agenda::AgendaHost,
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
    /// Route restored when the independently mounted generated result is closed.  The document
    /// itself remains in `state`, so this record never owns or resurrects a replaced session.
    pub(crate) agenda_text_return: Option<ContentRoute>,
    pub(crate) agenda_text_open: bool,
    pub(crate) document_workspace: DocumentWorkspaceState,
    pub(crate) document_view_preferences: DocumentViewPreferences,
    pub(crate) content_font_sizes: PanePair<crate::typography::ContentFontSize>,
    pub(crate) split_resize: Option<split_layout::ResizeSession>,
    pub(crate) soft_wrap: bool,
    pub(crate) minimap_visible: bool,
    pub(crate) minimap_thumb_visibility: crate::settings::MinimapThumbVisibility,
    pub(crate) minimap_width: Option<u16>,
    pub(crate) minimap_resize_preview: Option<f32>,
    pub(crate) reading_style: crate::preview::PreviewStyleId,
    pub(crate) status: StatusLineHost,
}

#[derive(Default)]
pub(crate) struct DerivedHost {
    pub(crate) task: Option<Task<()>>,
    pub(crate) sender: Option<async_channel::Sender<()>>,
    pub(crate) pending: Arc<Mutex<Option<derived::DerivedRequest>>>,
    pub(crate) latest: Option<Arc<crate::preview::PreviewSnapshot>>,
}
