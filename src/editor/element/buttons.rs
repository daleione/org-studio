//! Source block action buttons: the in-gutter run button and the copy button
//! on a block's trailing edge.
use gpui::{TextStyle, Window};

use crate::theme::Theme;

use super::blocks::BlockChrome;
use super::state::SourceRunButtonPaint;
use super::*;

/// Builds the run glyph and its hitbox for every open source block.
pub(super) fn prepare_run_buttons(
    rows: &[PaintRow],
    chrome: &BlockChrome,
    feedback: Option<crate::editor::SourceRunFeedback>,
    theme: &Theme,
    style: &TextStyle,
    font_size: Pixels,
    window: &mut Window,
) -> Vec<SourceRunButtonPaint> {
    rows.iter()
        .filter(|row| {
            row.block.as_ref().is_some_and(|block| {
                block.kind == syntax::EditorBlockKind::Source
                    && block.edge == syntax::EditorBlockEdge::Open
            })
        })
        .filter_map(|row| {
            let button_left = chrome.left
                + px(BLOCK_LEFT_INSET
                    + SOURCE_GUTTER_INSET
                    + (SOURCE_GUTTER_WIDTH - SOURCE_RUN_BUTTON_SIZE) / 2.0);
            let row_height = f32::from(row.hit.visible_bottom - row.hit.visible_top);
            let button_top =
                row.hit.visible_top + px(((row_height - SOURCE_RUN_BUTTON_SIZE) / 2.0).max(0.0));
            let bounds = Bounds::new(
                point(button_left, button_top),
                size(px(SOURCE_RUN_BUTTON_SIZE), px(SOURCE_RUN_BUTTON_SIZE)),
            );
            let interaction_bounds = Bounds::new(
                point(
                    button_left - px(SOURCE_RUN_BUTTON_HIT_SLOP),
                    button_top - px(SOURCE_RUN_BUTTON_HIT_SLOP),
                ),
                size(
                    px(SOURCE_RUN_BUTTON_SIZE + SOURCE_RUN_BUTTON_HIT_SLOP * 2.0),
                    px(SOURCE_RUN_BUTTON_SIZE + SOURCE_RUN_BUTTON_HIT_SLOP * 2.0),
                ),
            );
            let interaction_left = interaction_bounds.left().max(chrome.viewport_left);
            let interaction_right = interaction_bounds.right().min(chrome.viewport_right);
            if interaction_right <= interaction_left {
                return None;
            }
            let interaction_bounds = Bounds::from_corners(
                point(interaction_left, interaction_bounds.top()),
                point(interaction_right, interaction_bounds.bottom()),
            );
            let feedback =
                feedback.filter(|feedback| feedback.source_offset == row.hit.range.start);
            let (icon_text, accent) = match feedback.map(|feedback| feedback.phase) {
                Some(crate::editor::SourceRunPhase::Running) => {
                    window.request_animation_frame();
                    let frames = ["◐", "◓", "◑", "◒"];
                    let frame = feedback
                        .map(|feedback| {
                            (feedback.started_at.elapsed().as_millis() / 90) as usize % frames.len()
                        })
                        .unwrap_or(0);
                    (frames[frame], theme.meta)
                }
                Some(crate::editor::SourceRunPhase::Success) => ("✓", theme.heading[2]),
                Some(crate::editor::SourceRunPhase::Failure) => ("!", theme.error),
                None => ("▶", theme.meta),
            };
            let icon: gpui::SharedString = icon_text.into();
            let run = TextRun {
                len: icon.len(),
                font: style.font(),
                color: rgba((accent << 8) | 0xd0).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            Some(SourceRunButtonPaint {
                source_offset: row.hit.range.start,
                bounds,
                interaction_bounds,
                hitbox: window.insert_hitbox(interaction_bounds, HitboxBehavior::Normal),
                accent,
                icon: window.text_system().shape_line(
                    icon,
                    px(f32::from(font_size) * SOURCE_RUN_ICON_FONT_SCALE),
                    &[run],
                    None,
                ),
            })
        })
        .collect()
}

/// Builds the copy button for every open source or fenced block.
pub(super) fn prepare_copy_buttons(
    rows: &[PaintRow],
    chrome: &BlockChrome,
    bounds: Bounds<Pixels>,
    revision: crate::document::Revision,
    feedback: Option<(crate::document::Revision, crate::document::ByteOffset)>,
    window: &mut Window,
) -> Vec<crate::editor::source_copy::CopyButtonPaint> {
    rows.iter()
        .filter(|row| {
            row.block.as_ref().is_some_and(|block| {
                matches!(
                    block.kind,
                    syntax::EditorBlockKind::Source | syntax::EditorBlockKind::MarkdownFence
                ) && block.edge == syntax::EditorBlockEdge::Open
            })
        })
        .filter_map(|row| {
            let block_right = chrome.content_right - px(BLOCK_RIGHT_INSET);
            let right = block_right.min(chrome.viewport_right) - px(10.);
            let left = right - px(26.);
            if left < chrome.viewport_left || row.hit.origin_y < bounds.top() {
                return None;
            }
            // Use the painted block boundary, including its top inset and
            // rounded corner, rather than the editor's outer text rectangle.
            let probe = point(right, row.hit.visible_top + px(BLOCK_VERTICAL_INSET + 1.));
            let block = chrome
                .backgrounds
                .iter()
                .find(|quad| quad.bounds.contains(&probe))?
                .bounds;
            let height = px(26.).min(block.size.height - px(10.));
            if height < px(14.) {
                return None;
            }
            let top =
                (row.hit.origin_y + (row.hit.line_height - height) / 2.).max(block.top() + px(5.));
            let button = Bounds::new(point(left, top), size(px(26.), height));
            if button.bottom() > bounds.bottom() || button.bottom() > block.bottom() - px(5.) {
                return None;
            }
            Some(crate::editor::source_copy::CopyButtonPaint::new(
                button,
                revision,
                row.hit.range.start,
                feedback == Some((revision, row.hit.range.start)),
                window,
            ))
        })
        .collect()
}
