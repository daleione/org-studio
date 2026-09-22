//! Row-local highlight quads: selection, hex swatches, tag pills, link hover
//! and search matches.

use gpui::{Bounds, PaintQuad, Pixels, fill, point, px, rgba};

use crate::document::ByteRange;
use crate::editor::{HitRow, highlight::RangeHighlight, syntax};

pub(super) fn push_selection_quads(
    quads: &mut Vec<PaintQuad>,
    hit: &HitRow,
    start: usize,
    end: usize,
    include_newline: bool,
    wrap_width: Pixels,
) {
    push_range_quads(
        quads,
        hit,
        start..end,
        wrap_width,
        include_newline,
        rgba((crate::theme::current_theme().accent << 8) | 0x4a),
    );
}

/// Paints the opaque swatch behind every hex literal in the row. Quads stay
/// in the background layer, so hover, search and selection highlights still
/// paint on top. Fill and glyph color are per-literal, while radius and padding
/// come from [`RangeHighlight`].
pub(super) fn push_swatch_quads(
    quads: &mut Vec<PaintQuad>,
    hit: &HitRow,
    spans: &[syntax::EditorSemanticSpan],
    wrap_width: Pixels,
    theme: &crate::theme::Theme,
) {
    for span in spans {
        let Some(literal) = span.swatch else {
            continue;
        };
        if span.bytes.start >= span.bytes.end {
            continue;
        }
        let fill = rgba(crate::org_syntax::color::swatch_fill(
            literal,
            theme.background,
        ));
        RangeHighlight::rounded(fill.into()).paint(quads, hit, span.bytes.clone(), wrap_width);
    }
}

/// Paints one rounded pill behind each Org tag span, keeping the `:` source
/// separators outside the fill so every byte of the raw heading stays visible.
pub(super) fn push_tag_pill_quads(
    quads: &mut Vec<PaintQuad>,
    hit: &HitRow,
    spans: &[syntax::EditorSemanticSpan],
    wrap_width: Pixels,
    theme: &crate::theme::Theme,
    active: Option<ByteRange>,
) {
    for span in spans.iter().filter(|span| span.pill) {
        let start = span.bytes.start.saturating_add(1);
        let end = span.bytes.end.saturating_sub(1);
        if start >= end {
            continue;
        }
        let source_start = hit.range.start.0 + hit.display.display_to_source(start) as u64;
        let source_end = hit.range.start.0 + hit.display.display_to_source(end) as u64;
        let hovered =
            active.is_some_and(|range| range.start.0 < source_end && range.end.0 > source_start);
        // Change the existing pill's fill, never paint a second rounded rectangle.
        let fill = rgba((theme.attribute << 8) | if hovered { 0x2a } else { 0x16 });
        RangeHighlight::rounded(fill.into()).paint(quads, hit, start..end, wrap_width);
    }
}

/// Highlights the link currently under the pointer.
pub(super) fn push_link_hover_quad(
    quads: &mut Vec<PaintQuad>,
    hit: &HitRow,
    link: &crate::editor::LinkHit,
    wrap_width: Pixels,
    theme: &crate::theme::Theme,
) {
    RangeHighlight::rounded(rgba((syntax::link_accent(&link.meta.kind, theme) << 8) | 0x26).into())
        .paint(quads, hit, link.display_range.clone(), wrap_width);
}

pub(super) fn push_search_quads(
    quads: &mut Vec<PaintQuad>,
    hit: &HitRow,
    start: usize,
    end: usize,
    wrap_width: Pixels,
    current: bool,
) {
    let color = rgba(if current {
        crate::theme::current_theme().search_current
    } else {
        crate::theme::current_theme().search_match
    });
    push_range_quads(quads, hit, start..end, wrap_width, false, color);
}

fn push_range_quads(
    quads: &mut Vec<PaintQuad>,
    hit: &HitRow,
    range: std::ops::Range<usize>,
    wrap_width: Pixels,
    include_newline: bool,
    color: gpui::Rgba,
) {
    let start = range.start;
    let end = range.end;
    if let Some(table) = &hit.table_layout {
        for mut bounds in table.range_bounds(start..end, hit.line_height) {
            bounds.origin += point(hit.text_origin_x, hit.origin_y);
            quads.push(fill(bounds, color));
        }
        if include_newline {
            let position = table.position_for_index(end, hit.line_height);
            quads.push(fill(
                Bounds::new(
                    point(hit.text_origin_x + position.x, hit.origin_y + position.y),
                    gpui::size(px(8.), hit.line_height),
                ),
                color,
            ));
        }
        return;
    }
    let line_height = hit.line_height;
    let start_position = hit.position_for_display_index(start).unwrap_or_default();
    let end_position = hit
        .position_for_display_index(end)
        .unwrap_or(start_position);
    let line_height_px = f32::from(line_height).max(1.0);
    let first_row = (f32::from(start_position.y) / line_height_px).round() as usize;
    let last_row = (f32::from(end_position.y) / line_height_px).round() as usize;
    for row in first_row..=last_row {
        let left = if row == first_row {
            start_position.x
        } else {
            Pixels::ZERO
        };
        let mut right = if row == last_row {
            end_position.x
        } else {
            wrap_width
        };
        if include_newline && row == last_row {
            right += px(8.0);
        }
        if right > left {
            quads.push(fill(
                Bounds::from_corners(
                    point(
                        hit.text_origin_x + left,
                        hit.origin_y + px(row as f32 * line_height_px),
                    ),
                    point(
                        hit.text_origin_x + right,
                        hit.origin_y + px((row + 1) as f32 * line_height_px),
                    ),
                ),
                color,
            ));
        }
    }
}
