use std::{sync::Arc, time::Duration};

use gpui::{
    FontStyle, FontWeight, HighlightStyle, ListAlignment, StyledText, actions, div, img, px, rgb,
};
mod action;
mod display_map;
mod document;
mod input;
pub(crate) mod layout;
mod loading;
mod view;
pub(crate) use crate::document::DocumentFormat;
pub(crate) use crate::syntax_highlighting::{CodeHighlightKind, CodeHighlightSpan, highlight_code};
pub(crate) use action::{
    CopyFeedbackState, PendingPreviewAction, PreviewAction, PreviewActionIdentity,
    PreviewActionTarget, PreviewActionVisualState,
};
use action::{checkbox_action, code_action, image_action, source_action_target};
pub(crate) use document::{
    CodeRowRole, DerivedUpdate, PreviewRow, ReloadedDocument, WorkspaceLoadedDocument,
    WorkspaceReloadedDocument, configured_minimap_visible,
};
pub use document::{
    DerivedEvent, InitialDocumentLoad, LoadMetrics, LoadedDocument, PreviewSnapshot,
    preload_initial_document,
};
pub(crate) use input::*;
pub(crate) use loading::{
    derive_preview_incremental, fitted_image_size, image_dimensions, load_workspace_document,
    reload_workspace_document, resolve_image_path, svg_dimensions,
};
pub use loading::{
    load_document, load_document_profiled, load_document_profiled_without_display_map,
};
use view::code_highlight_style;
pub(crate) use view::{ReadingRenderOptions, render_reading_document};
mod coordinates;
mod diagram;
mod fold_transition;
mod folding;
pub(crate) mod markdown;
pub(crate) mod minimap;
mod org_line;
mod projection;
mod reading_panel;
mod rows;
mod style;
mod table;
#[cfg(test)]
mod tests;
pub(crate) use diagram::DiagramLanguage;
use diagram::build_markdown_diagrams;
use fold_transition::{
    FoldDirection, FoldMeasurement, FoldSegment, FoldTransition, FoldTransitionInput,
    FoldTransitionPlan,
};
use folding::{
    GlobalVisibility, LocalCycleProjection, LocalVisibility, changed_range,
    cycle_markdown_subtree_visibility, cycle_org_subtree_visibility, global_markdown_visibility,
    global_org_visibility, visible_markdown_row_indices, visible_row_indices,
};
use projection::build_projection_snapshot;
pub(crate) use reading_panel::ReadingPreviewPanel;
use reading_panel::ReadingRenderState;
use rows::build_preview_rows;
use style::PreviewStyle;
pub use style::PreviewStyleId;
pub(crate) use style::preview_style;
use table::{build_markdown_table_styles, build_table_styles, render_table_row};

use crate::org_syntax::{
    BlockId, BlockKind, BlockNode,
    inline::{InlineKind, InlineSpan, InlineText, parse as parse_inline},
};

pub(in crate::preview) fn parse_document_inline(
    format: DocumentFormat,
    source: &str,
) -> InlineText {
    match format {
        DocumentFormat::Org => parse_inline(source),
        DocumentFormat::Markdown => markdown::parse_markdown_inline(source),
    }
}

pub(crate) const OPEN_DOCUMENT_COMMAND: &str = "org-studio.workspace.open-file";
pub(crate) const SHOW_HOME_COMMAND: &str = "org-studio.workspace.show-home";
pub(crate) const OPEN_AGENDA_COMMAND: &str = "org-studio.workspace.open-agenda";
pub(crate) const OPEN_AGENDA_TEXT_COMMAND: &str = "org-studio.workspace.open-agenda-text";
pub(crate) const RELOAD_DOCUMENT_COMMAND: &str = "org-studio.document.reload";
pub(crate) const SAVE_DOCUMENT_COMMAND: &str = "org-studio.document.save";
pub(crate) const SAVE_DOCUMENT_AS_COMMAND: &str = "org-studio.document.save-as";
const UNDO_DOCUMENT_COMMAND: &str = "org-studio.document.undo";
const REDO_DOCUMENT_COMMAND: &str = "org-studio.document.redo";
pub(crate) const EXPORT_DOCUMENT_COMMAND: &str = "org-studio.document.export";
pub(crate) const QUIT_APPLICATION_COMMAND: &str = "org-studio.application.quit";
const SCROLL_FORWARD_COMMAND: &str = "org-studio.preview.scroll-forward";
const SCROLL_BACKWARD_COMMAND: &str = "org-studio.preview.scroll-backward";
const BEGINNING_COMMAND: &str = "org-studio.preview.beginning";
const END_COMMAND: &str = "org-studio.preview.end";
const OPEN_FILE_MANAGER_COMMAND: &str = "org-studio.file-manager.open";
const OPEN_DEFAULT_DIRED_COMMAND: &str = "org-studio.dired.open-default";
const RETURN_DOCUMENT_COMMAND: &str = "org-studio.file-manager.return-document";
const TOGGLE_SIDEBAR_COMMAND: &str = "org-studio.file-manager.toggle-sidebar";
const TOGGLE_MINIMAP_COMMAND: &str = "org-studio.preview.toggle-minimap";
const SHOW_EDITOR_COMMAND: &str = "org-studio.workspace.show-editor";
const SHOW_READING_COMMAND: &str = "org-studio.workspace.show-reading";
const SHOW_SPLIT_COMMAND: &str = "org-studio.workspace.show-split";
const TOGGLE_SOFT_WRAP_COMMAND: &str = "org-studio.editor.toggle-soft-wrap";
const ORG_CONTEXT_COMMAND: &str = "org-studio.org.context-command";
const EXECUTE_SOURCE_BLOCK_COMMAND: &str = "org-studio.babel.execute-source-block";
const TOGGLE_INLINE_IMAGE_PREVIEWS_COMMAND: &str = "org-studio.org.toggle-inline-image-previews";
pub(crate) const INCREASE_CONTENT_FONT_SIZE_COMMAND: &str =
    "org-studio.view.increase-content-font-size";
pub(crate) const DECREASE_CONTENT_FONT_SIZE_COMMAND: &str =
    "org-studio.view.decrease-content-font-size";
pub(crate) const RESET_CONTENT_FONT_SIZE_COMMAND: &str = "org-studio.view.reset-content-font-size";
pub const DOCUMENT_WORKSPACE_KEY_CONTEXT: &str = "DocumentWorkspace";
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
pub(crate) const KEY_FEEDBACK_DURATION: Duration = Duration::from_secs(2);
// `ListState::measure_all` renders every row during the first layout pass. Keep that useful for
// genuinely small documents, but never let a file switch turn one UI frame into a full-document
// layout. Larger documents converge as their visible rows are measured by GPUI's virtual list.
const MAX_EAGER_LAYOUT_ROWS: usize = 128;
#[cfg(test)]
pub(crate) const LOCAL_FOLD_ANIMATION_DURATION: Duration = crate::motion::FOLD_MOTION.duration();

actions!(
    org_preview,
    [
        OpenDocument,
        ShowHome,
        ReloadDocument,
        SaveDocument,
        SaveDocumentAs,
        ExportDocument,
        QuitApplication,
        OpenFileManager,
        ReturnToDocument,
        ToggleSidebar,
        ToggleMinimap,
        ShowEditor,
        ShowReading,
        ShowSplit,
        ToggleSoftWrap,
        IncreaseContentFontSize,
        DecreaseContentFontSize,
        ResetContentFontSize,
        UseEnglish,
        UseChinese
    ]
);

pub(crate) fn is_supported_document(path: &std::path::Path) -> bool {
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

pub(crate) fn accept_generation(current: u64, completed: u64) -> bool {
    current == completed
}

fn should_eagerly_measure_rows(row_count: usize) -> bool {
    row_count <= MAX_EAGER_LAYOUT_ROWS
}
