use super::{
    Arc, BlockKind, BlockNode, CodeHighlightKind, CodeHighlightSpan, DocumentFormat, FoldDirection,
    FoldSegment, FontStyle, FontWeight, HighlightStyle, InlineKind, InlineSpan, PreviewRow,
    PreviewSnapshot, PreviewStyle, StyledText, div, img, markdown, minimap, px, render_table_row,
    resolve_image_path, rgb,
};
use std::time::Instant;

mod code_block;
mod document;
mod markdown_block;
mod selectable_text;
mod styled_text;

#[derive(Clone, Copy)]
struct ReadingRowContext<'a> {
    available_width: f32,
    zoom: f32,
    style: PreviewStyle,
    table_scroll: Option<&'a gpui::ScrollHandle>,
}

#[derive(Clone)]
pub(in crate::preview) struct ReadingInteraction {
    pub(in crate::preview) panel: gpui::Entity<super::ReadingPreviewPanel>,
    dispatch: ReadingActionDispatcher,
    action_states: Arc<
        Vec<(
            super::PreviewActionIdentity,
            super::PreviewActionVisualState,
        )>,
    >,
    copy_feedback: Option<(crate::document::ByteRange, super::CopyFeedbackState)>,
    pub(in crate::preview) text_selection: Option<(
        super::reading_panel::ReadingTextPoint,
        super::reading_panel::ReadingTextPoint,
    )>,
}

pub(crate) type ReadingActionDispatcher = Arc<
    dyn Fn(
        super::PreviewAction,
        gpui::Entity<super::ReadingPreviewPanel>,
        &mut gpui::Window,
        &mut gpui::App,
    ),
>;

pub(crate) type ReadingMinimapWidthDispatcher =
    Arc<dyn Fn(minimap::MinimapWidthChange, &mut gpui::App)>;

struct ReadingRowHost<'a> {
    available_width: f32,
    zoom: f32,
    style: PreviewStyle,
    table_scroll_handles: &'a std::collections::HashMap<super::BlockId, gpui::ScrollHandle>,
    interaction: Option<&'a ReadingInteraction>,
    extra_bottom_padding: bool,
}

use code_block::render_code_row;
pub(in crate::preview) use document::reading_row_selection;
pub(crate) use document::{ReadingRenderOptions, render_reading_document};
use markdown_block::render_markdown_block;
pub(in crate::preview) use selectable_text::SelectableReadingText;
pub(super) use styled_text::code_highlight_style;
pub(in crate::preview) use styled_text::styled_inline_runs;
