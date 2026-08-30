use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    Context, FontStyle, FontWeight, HighlightStyle, IntoElement, KeyDownEvent, ListAlignment,
    PathPromptOptions, Render, StyledText, Window, actions, div, img, px, rgb,
};
mod app;
mod display_map;
mod document;
mod export_ui;
mod highlighting;
mod input;
mod layout;
mod loading;
mod status_line;
mod view;
pub(crate) use document::DocumentFormat;
use document::{
    CodeRowRole, PreviewRow, ReloadedDocument, WorkspaceLoadedDocument, WorkspaceReloadedDocument,
    configured_minimap_visible,
};
pub use document::{
    DerivedEvent, InitialDocumentLoad, LoadMetrics, LoadedDocument, PreviewSnapshot,
    preload_initial_document,
};
use highlighting::{CodeHighlightKind, CodeHighlightSpan, highlight_code};
use input::*;
use loading::{
    fitted_image_size, load_workspace_document, reload_workspace_document, resolve_image_path,
};
pub use loading::{
    load_document, load_document_profiled, load_document_profiled_without_display_map,
};
use view::*;
mod command_window;
mod coordinates;
mod file_manager_host;
mod fold_transition;
mod folding;
mod markdown;
mod minimap;
#[cfg(test)]
mod org_line;
mod overlay;
mod panel;
mod projection;
mod rows;
mod table;
#[cfg(test)]
mod tests;
mod visual_recipe;
pub(crate) use app::ScrollBenchmark;
pub(crate) use export_ui::ExportHost;
pub(crate) use file_manager_host::FileManagerHost;
use fold_transition::{
    FoldDirection, FoldMeasurement, FoldSegment, FoldTransition, FoldTransitionInput,
    FoldTransitionPlan,
};
use folding::{
    GlobalVisibility, LocalCycleProjection, LocalVisibility, changed_range,
    cycle_markdown_subtree_visibility, cycle_org_subtree_visibility, global_markdown_visibility,
    global_org_visibility,
};
pub(crate) use panel::PreviewPanel;
use panel::PreviewRenderState;
use projection::build_projection_snapshot;
use rows::build_preview_rows;
pub(crate) use status_line::StatusLineHost;
use table::{build_markdown_table_styles, build_table_styles, render_table_row};

use crate::{
    app::WorkspaceWindow,
    command::{
        BuiltinCommand, CapabilitySet, CommandDispatcher, CommandImplementation, CommandKey,
        InvocationOrigin, PrefixArgument,
    },
    input::{EmacsOutcome, compile_input_profile},
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
const EXPORT_DOCUMENT_COMMAND: &str = "org-studio.document.export";
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
const GLOBAL_VISIBILITY_CYCLE_COMMAND: &str = "org-studio.preview.global-visibility-cycle";
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
const DIRED_CREATE_FILE_COMMAND: &str = "org-studio.dired.create-file";
const DIRED_CREATE_DIRECTORY_COMMAND: &str = "org-studio.dired.create-directory";
const DIRED_RENAME_COMMAND: &str = "org-studio.dired.rename";
const DIRED_COPY_COMMAND: &str = "org-studio.dired.copy";
const DIRED_MOVE_COMMAND: &str = "org-studio.dired.move";
const DIRED_TRASH_COMMAND: &str = "org-studio.dired.trash";
const DIRED_HELP_COMMAND: &str = "org-studio.dired.help";
const KEY_FEEDBACK_DURATION: Duration = Duration::from_secs(2);
// `ListState::measure_all` renders every row during the first layout pass. Keep that useful for
// genuinely small documents, but never let a file switch turn one UI frame into a full-document
// layout. Larger documents converge as their visible rows are measured by GPUI's virtual list.
const MAX_EAGER_LAYOUT_ROWS: usize = 128;
const LOCAL_FOLD_ANIMATION_DURATION: Duration = Duration::from_millis(160);

actions!(
    org_preview,
    [
        OpenDocument,
        ShowHome,
        ReloadDocument,
        ExportDocument,
        OpenFileManager,
        ReturnToDocument,
        ToggleSidebar,
        ToggleMinimap,
        UseEnglish,
        UseChinese
    ]
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContentRoute {
    Document,
    FileManager,
}

#[derive(Clone)]
enum DiredStatus {
    Working(Arc<str>),
    Success(Arc<str>),
    Error(Arc<str>),
}

impl DiredStatus {
    fn message(&self) -> Arc<str> {
        match self {
            Self::Working(message) | Self::Success(message) | Self::Error(message) => {
                message.clone()
            }
        }
    }
}

pub(crate) enum PreviewLoadState {
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
    session: gpui::Entity<crate::document::DocumentSession>,
    editor: gpui::Entity<crate::editor::SourceEditor>,
    panel: Option<gpui::Entity<PreviewPanel>>,
    reload_error: Option<Arc<str>>,
}

impl ReadyDocument {
    fn panel(&self) -> &gpui::Entity<PreviewPanel> {
        self.panel
            .as_ref()
            .expect("preview mode publishes a preview panel")
    }
}

impl PreviewLoadState {
    fn ready(&self) -> Option<&ReadyDocument> {
        match self {
            Self::Ready { document } => Some(document),
            Self::Loading { previous, .. } | Self::Failed { previous, .. } => previous.as_ref(),
            Self::Empty => None,
        }
    }

    fn take_ready(&mut self) -> Option<ReadyDocument> {
        match std::mem::replace(self, Self::Empty) {
            Self::Ready { document } => Some(document),
            Self::Loading { previous, .. } | Self::Failed { previous, .. } => previous,
            Self::Empty => None,
        }
    }

    fn ready_mut(&mut self) -> Option<&mut ReadyDocument> {
        match self {
            Self::Ready { document } => Some(document),
            Self::Loading { previous, .. } | Self::Failed { previous, .. } => previous.as_mut(),
            Self::Empty => None,
        }
    }
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

fn should_eagerly_measure_rows(row_count: usize) -> bool {
    row_count <= MAX_EAGER_LAYOUT_ROWS
}
