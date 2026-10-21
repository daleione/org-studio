//! Source block actions, anchored to the first body line while retaining
//! the opening declaration's byte offset for execution and copying.
use gpui::{TextStyle, Window};

use crate::theme::Theme;

use super::blocks::BlockChrome;
use super::state::SourceRunButtonPaint;
use super::*;

fn action_source_offset(
    row: &PaintRow,
    snapshot: &crate::document::DocumentSnapshot,
) -> Option<ByteOffset> {
    let block = row.block.as_ref()?;
    if block.is_first_source_body_line() {
        snapshot
            .line_range(LineIndex(row.hit.line.0.checked_sub(1)?))
            .ok()
            .map(|range| range.start)
    } else if block.kind == syntax::EditorBlockKind::MarkdownFence
        && block.edge == syntax::EditorBlockEdge::Open
    {
        Some(row.hit.range.start)
    } else {
        None
    }
}

fn block_bounds_for_row(row: &PaintRow, chrome: &BlockChrome) -> Option<Bounds<Pixels>> {
    let probe = point(
        chrome.content_right - px(BLOCK_RIGHT_INSET + 1.0),
        row.hit.origin_y + row.hit.line_height / 2.0,
    );
    chrome
        .backgrounds
        .iter()
        .find(|quad| quad.bounds.contains(&probe))
        .map(|quad| quad.bounds)
}

/// Builds the run glyph in the first body line's number cell. Hovering any
/// part of the body reveals it; execution feedback remains visible on exit.
pub(super) fn prepare_run_buttons(
    rows: &[PaintRow],
    chrome: &BlockChrome,
    snapshot: &crate::document::DocumentSnapshot,
    feedback: Option<crate::editor::SourceRunFeedback>,
    theme: &Theme,
    style: &TextStyle,
    window: &mut Window,
) -> Vec<SourceRunButtonPaint> {
    rows.iter()
        .filter(|row| {
            row.block
                .as_ref()
                .is_some_and(syntax::EditorBlockDecoration::is_first_source_body_line)
        })
        .filter_map(|row| {
            let source_offset = action_source_offset(row, snapshot)?;
            let body_bounds = block_bounds_for_row(row, chrome)?.intersect(&chrome.viewport);
            let button_size = SOURCE_RUN_BUTTON_SIZE.min(f32::from(row.hit.line_height) - 2.0);
            if button_size < 10.0 || row.hit.origin_y < chrome.viewport.top() {
                return None;
            }
            let button_left = chrome.left
                + px(editor_block_left_inset(&syntax::EditorBlockKind::Source)
                    + SOURCE_GUTTER_INSET
                    + (SOURCE_GUTTER_WIDTH - button_size) / 2.0);
            let button_top = row.hit.origin_y + (row.hit.line_height - px(button_size)) / 2.0;
            let bounds = Bounds::new(
                point(button_left, button_top),
                size(px(button_size), px(button_size)),
            );
            let interaction_bounds = Bounds::new(
                point(
                    button_left - px(SOURCE_RUN_BUTTON_HIT_SLOP),
                    button_top - px(SOURCE_RUN_BUTTON_HIT_SLOP),
                ),
                size(
                    px(button_size + SOURCE_RUN_BUTTON_HIT_SLOP * 2.0),
                    px(button_size + SOURCE_RUN_BUTTON_HIT_SLOP * 2.0),
                ),
            )
            .intersect(&body_bounds)
            .intersect(&Bounds::new(
                point(body_bounds.left(), row.hit.origin_y),
                size(body_bounds.size.width, row.hit.line_height),
            ));
            if interaction_bounds.size.width <= px(0.0) || interaction_bounds.size.height <= px(0.0)
            {
                return None;
            }
            let feedback = feedback.filter(|feedback| feedback.source_offset == source_offset);
            let visible = feedback.is_some() || body_bounds.contains(&window.mouse_position());
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
                Some(crate::editor::SourceRunPhase::Success) => ("✓", theme.success),
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
            let font_size = style.font_size.to_pixels(window.rem_size());
            Some(SourceRunButtonPaint {
                hit: crate::editor::SourceRunButtonHit {
                    source_offset,
                    body_bounds,
                    visible,
                    bounds: interaction_bounds,
                },
                line: row.hit.line,
                bounds,
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

// A truly empty or manually folded Org block has no visible first body
// line. Retain its existing copy entry on the declaration in that case.
fn declaration_copy_fallback(row: &PaintRow, snapshot: &crate::document::DocumentSnapshot) -> bool {
    row.block.as_ref().is_some_and(|block| {
        block.kind == syntax::EditorBlockKind::Source
            && block.edge == syntax::EditorBlockEdge::Open
            && (row.folded
                || snapshot
                    .line_content_range(LineIndex(row.hit.line.0 + 1))
                    .ok()
                    .is_none_or(|range| {
                        snapshot
                            .copy_range(range)
                            .trim()
                            .eq_ignore_ascii_case("#+end_src")
                    }))
    })
}

/// Copies from the first Org body line or the opening Markdown fence.
pub(super) fn prepare_copy_buttons(
    rows: &[PaintRow],
    chrome: &BlockChrome,
    snapshot: &crate::document::DocumentSnapshot,
    feedback: Option<(crate::document::Revision, crate::document::ByteOffset)>,
    window: &mut Window,
) -> Vec<crate::editor::source_copy::CopyButtonPaint> {
    let revision = snapshot.revision();
    rows.iter()
        .filter_map(|row| {
            let fallback = declaration_copy_fallback(row, snapshot);
            let source_offset = action_source_offset(row, snapshot)
                .or_else(|| fallback.then_some(row.hit.range.start))?;
            let org_source = row
                .block
                .as_ref()
                .is_some_and(|block| block.kind == syntax::EditorBlockKind::Source);
            let block_right = chrome.content_right - px(BLOCK_RIGHT_INSET);
            let right =
                block_right.min(chrome.viewport.right()) - px(SOURCE_COPY_BUTTON_RIGHT_INSET);
            if row.hit.origin_y < chrome.viewport.top() {
                return None;
            }
            let block = if fallback {
                Bounds::from_corners(
                    point(chrome.viewport.left(), row.hit.visible_top),
                    point(chrome.viewport.right(), row.hit.visible_bottom),
                )
            } else {
                block_bounds_for_row(row, chrome)?
            };
            let margin = px(if org_source { 1.0 } else { 5.0 });
            let height = if org_source {
                px(SOURCE_COPY_BUTTON_SIZE).min(row.hit.line_height - px(2.0))
            } else {
                px(SOURCE_COPY_BUTTON_SIZE)
            }
            .min(block.size.height - margin * 2.0);
            if height < px(14.) {
                return None;
            }
            let width = if org_source {
                height
            } else {
                px(SOURCE_COPY_BUTTON_SIZE)
            };
            let left = right - width;
            if left < chrome.viewport.left() {
                return None;
            }
            let top =
                (row.hit.origin_y + (row.hit.line_height - height) / 2.).max(block.top() + margin);
            let button = Bounds::new(point(left, top), size(width, height));
            if button.bottom() > chrome.viewport.bottom()
                || button.bottom() > block.bottom() - margin
            {
                return None;
            }
            Some(crate::editor::source_copy::CopyButtonPaint::new(
                button,
                revision,
                source_offset,
                feedback == Some((revision, source_offset)),
                window,
            ))
        })
        .collect()
}
