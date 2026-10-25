use super::blocks::{EditorBlockPaintRow, editor_block_segments};
use super::cookie::{
    COOKIE_BAR_GAP_EM, COOKIE_BAR_MAX_HEIGHT, cookie_bar_bounds, cookie_bar_insets,
};
use super::minimap::build_minimap;
use super::minimap_layout::{prepare_minimap_layout_request, schedule_minimap_layout_preparation};
use super::minimap_raster::{apply_editor_minimap_media_dimensions, minimap_text_row};
use super::quads::{push_swatch_quads, table_hover_quads};
use super::rows::{MAX_ANIMATED_PAINT_LINES, animated_paint_lines};
use crate::document::{DocumentSession, DocumentSnapshot, TextSnapshot};
use crate::editor::{
    SemanticEditor,
    layout_map::EditorLayoutMap,
    minimap::RasterMedia,
    syntax::{
        EditorBlockDecoration, EditorBlockEdge, EditorBlockKind, EditorStyleId,
        EditorSyntaxService, SparseEditorStyleSnapshot, semantic_spans,
    },
};
use gpui::{AppContext, Modifiers, px};
use std::path::Path;

mod blocks;
mod decorations;
mod images;
mod minimap;
mod table_editing;
mod tables;

fn table_cell_center(
    editor: &gpui::Entity<SemanticEditor>,
    line: u64,
    column: usize,
    view: &gpui::VisualTestContext,
) -> gpui::Point<gpui::Pixels> {
    view.read(|cx| {
        let editor = editor.read(cx);
        let row = editor
            .hit_rows
            .iter()
            .find(|row| row.line.0 == line)
            .unwrap();
        let snapshot = editor.snapshot(cx);
        let range = snapshot.line_content_range(row.line).unwrap();
        let text = snapshot.copy_range(range);
        let format =
            crate::document::DocumentFormat::from_path(editor.session.read(cx).syntax_path());
        let cell = &crate::document::table::parse_line(&text, format).cells[column];
        let start = row
            .position_for_display_index(row.display.source_to_display(cell.raw_range.start))
            .unwrap();
        let end = row
            .position_for_display_index(row.display.source_to_display(cell.raw_range.end))
            .unwrap();
        gpui::point(
            row.text_origin_x + (start.x + end.x) / 2.,
            row.origin_y + start.y + px(5.),
        )
    })
}
