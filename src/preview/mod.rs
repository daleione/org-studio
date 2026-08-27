use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    Context, FocusHandle, FontStyle, FontWeight, HighlightStyle, IntoElement, KeyDownEvent,
    ListAlignment, ListOffset, ListState, PathPromptOptions, Render, StyledText, Subscription,
    Task, Window, actions, div, img, px, rgb,
};
mod app;
mod display_map;
mod document;
mod highlighting;
mod input;
mod layout;
mod loading;
mod view;
use document::{DocumentFormat, PreviewRow, configured_minimap_visible, schedule_document_prewarm};
pub use document::{InitialDocumentLoad, LoadMetrics, PreviewDocument, preload_initial_document};
use highlighting::{CodeHighlightKind, CodeHighlightSpan, highlight_code};
use input::*;
use loading::{fitted_image_size, resolve_image_path};
pub use loading::{
    load_document, load_document_profiled, load_document_profiled_without_display_map,
};
use view::*;
mod command_window;
mod coordinates;
mod file_manager_host;
mod folding;
mod markdown;
mod minimap;
#[cfg(test)]
mod org_line;
mod overlay;
mod projection;
mod rows;
mod table;
#[cfg(test)]
mod tests;
mod visual_recipe;
use app::ScrollBenchmark;
use folding::{changed_range, visible_markdown_row_indices, visible_row_indices};
use projection::build_projection_snapshot;
use rows::build_preview_rows;
use table::{build_markdown_table_styles, build_table_styles, render_table_row};

use crate::{
    command::{
        BuiltinCommand, CapabilitySet, CommandDispatcher, CommandImplementation, CommandKey,
        CommandRegistry, InvocationOrigin, PrefixArgument,
    },
    input::{ContextSet, EmacsOutcome, KeyboardRouter, compile_input_profile},
    keymap::KeyStroke,
    org_syntax::{
        BlockId, BlockKind, BlockNode,
        inline::{InlineKind, InlineSpan, InlineText, parse as parse_inline},
    },
    theme::current_theme,
};

const OPEN_DOCUMENT_COMMAND: &str = "org-studio.workspace.open-file";
const SHOW_HOME_COMMAND: &str = "org-studio.workspace.show-home";
const RELOAD_DOCUMENT_COMMAND: &str = "org-studio.document.reload";
const QUIT_APPLICATION_COMMAND: &str = "org-studio.application.quit";
const SCROLL_FORWARD_COMMAND: &str = "org-studio.preview.scroll-forward";
const SCROLL_BACKWARD_COMMAND: &str = "org-studio.preview.scroll-backward";
const BEGINNING_COMMAND: &str = "org-studio.preview.beginning";
const END_COMMAND: &str = "org-studio.preview.end";
const OPEN_FILE_MANAGER_COMMAND: &str = "org-studio.file-manager.open";
const OPEN_DEFAULT_DIRED_COMMAND: &str = "org-studio.dired.open-default";
const RETURN_DOCUMENT_COMMAND: &str = "org-studio.file-manager.return-document";
const TOGGLE_SIDEBAR_COMMAND: &str = "org-studio.file-manager.toggle-sidebar";
const TOGGLE_MINIMAP_COMMAND: &str = "org-studio.preview.toggle-minimap";
const DIRED_NEXT_COMMAND: &str = "org-studio.dired.next-line";
const DIRED_PREVIOUS_COMMAND: &str = "org-studio.dired.previous-line";
const DIRED_OPEN_COMMAND: &str = "org-studio.dired.find-file";
const DIRED_UP_COMMAND: &str = "org-studio.dired.up-directory";
const DIRED_BACK_COMMAND: &str = "org-studio.dired.history-back";
const DIRED_FORWARD_COMMAND: &str = "org-studio.dired.history-forward";
const DIRED_MARK_COMMAND: &str = "org-studio.dired.mark";
const DIRED_UNMARK_COMMAND: &str = "org-studio.dired.unmark";
const DIRED_UNMARK_ALL_COMMAND: &str = "org-studio.dired.unmark-all";
const DIRED_INVERT_COMMAND: &str = "org-studio.dired.invert-marks";
const DIRED_DELETE_COMMAND: &str = "org-studio.dired.flag-delete";
const DIRED_EXECUTE_COMMAND: &str = "org-studio.dired.execute";
const DIRED_HELP_COMMAND: &str = "org-studio.dired.help";
const KEY_FEEDBACK_DURATION: Duration = Duration::from_secs(2);
const MAX_EXACT_SCROLL_LAYOUT_ROWS: usize = 4096;

actions!(
    org_preview,
    [
        OpenDocument,
        ShowHome,
        ReloadDocument,
        OpenFileManager,
        ReturnToDocument,
        ToggleSidebar,
        ToggleMinimap
    ]
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContentRoute {
    Document,
    FileManager,
}

enum PreviewLoadState {
    Empty,
    Loading {
        path: PathBuf,
    },
    Ready {
        generation: u64,
        document: Arc<PreviewDocument>,
    },
    Failed {
        path: PathBuf,
        message: String,
    },
}

pub struct PreviewApp {
    focus_handle: Option<FocusHandle>,
    focus_lost_subscription: Option<Subscription>,
    commands: Arc<CommandRegistry>,
    keyboard: KeyboardRouter,
    key_context: ContextSet,
    state: PreviewLoadState,
    recent_documents: Vec<crate::recent_documents::RecentDocument>,
    home_error: Option<Arc<str>>,
    generation: u64,
    load_task: Option<Task<()>>,
    file_watch_task: Option<Task<()>>,
    file_watch_request: u64,
    picker_task: Option<Task<()>>,
    list_state: ListState,
    folded: Arc<HashSet<BlockId>>,
    visible_rows: Arc<Vec<usize>>,
    last_ready: Option<(u64, Arc<PreviewDocument>)>,
    opened_at: Option<Instant>,
    first_frame_scheduled: Option<u64>,
    scroll_benchmark: Option<ScrollBenchmark>,
    which_key_task: Option<Task<()>>,
    which_key_request: u64,
    key_feedback_task: Option<Task<()>>,
    key_feedback_request: u64,
    which_key_items: Arc<Vec<(Arc<str>, Arc<str>)>>,
    dired_help_visible: bool,
    content_route: ContentRoute,
    sidebar_visible: bool,
    minimap_visible: bool,
    minimap_thumb_visibility: crate::settings::MinimapThumbVisibility,
    minimap_width: Option<u16>,
    minimap_resize_preview: Option<f32>,
    presentation_revision: u64,
    viewport_revision_key: Option<(u32, u32)>,
    minimap_pending_seek: Option<(u64, ListOffset)>,
    minimap_seek_scheduled: bool,
    dired: Option<crate::file_manager::DiredSession>,
    dired_error: Option<Arc<str>>,
    dired_task: Option<Task<()>>,
    dired_list_state: ListState,
    sidebar_list_state: ListState,
    dired_pending_presentation: Option<(
        crate::navigation::TransactionId,
        crate::navigation::ViewRevision,
        usize,
        f32,
    )>,
    sidebar_pending_presentation: Option<(
        crate::navigation::TransactionId,
        crate::navigation::ViewRevision,
        usize,
        f32,
    )>,
    dired_presentation_scheduled: bool,
    dired_viewport_memory: HashMap<PathBuf, (usize, f32)>,
    sidebar_viewport_memory: HashMap<PathBuf, (usize, f32)>,
}

fn is_supported_document(path: &std::path::Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("org")
                    || extension.eq_ignore_ascii_case("md")
                    || extension.eq_ignore_ascii_case("markdown")
            })
}

fn accept_generation(current: u64, completed: u64) -> bool {
    current == completed
}
