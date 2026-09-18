//! Statistics-cookie progress bar: geometry, insets and quads.

use std::ops::Range;

use gpui::{BorderStyle, Bounds, Corners, Edges, PaintQuad, Pixels, point, px, quad, rgb, rgba};

use crate::editor::HitRow;

pub(super) const COOKIE_BAR_HEIGHT_EM: f32 = 0.15;
pub(super) const COOKIE_BAR_MIN_HEIGHT: f32 = 2.0;
pub(super) const COOKIE_BAR_MAX_HEIGHT: f32 = 4.0;
pub(super) const COOKIE_BAR_GAP_EM: f32 = 0.22;
// Stand-in when the text system cannot report the bracket ink boxes.
pub(super) const COOKIE_BAR_SIDE_INSET_EM: f32 = 0.12;
pub(super) const COOKIE_BAR_TRACK_ALPHA: u32 = 0x59;
pub(super) const COOKIE_BAR_TRACK_ALPHA_DARK: u32 = 0x66;
/// Draws the statistics-cookie progress bar: a track under the digits plus the
/// meta-colored fill for `ratio`.
pub(super) fn push_cookie_progress_quads(
    quads: &mut Vec<PaintQuad>,
    hit: &HitRow,
    range: Range<usize>,
    ratio: f32,
    theme: &crate::theme::Theme,
    window: &gpui::Window,
) {
    let Some(start) = hit.position_for_display_index(range.start) else {
        return;
    };
    let Some(end) = hit.position_for_display_index(range.end) else {
        return;
    };
    // A wrapped cookie would need its fill sliced per visual row; skip that rare case.
    if (f32::from(start.y) - f32::from(end.y)).abs() > f32::EPSILON {
        return;
    }
    let (left_inset, right_inset) = cookie_bar_insets(hit, &range, end.x - start.x, window);
    let left = hit.text_origin_x + start.x + left_inset;
    let right = hit.text_origin_x + end.x - right_inset;
    if right - left <= px(1.0) {
        return;
    }
    let row_top = hit.origin_y + start.y;
    let line_height = hit.line_height;
    let ascent = hit.layout.ascent();
    let descent = hit.layout.descent();
    let font_size = hit.layout.font_size();
    let (top, bottom) = cookie_bar_bounds(row_top, line_height, ascent, descent, font_size);
    let height = bottom - top;
    let radius = height / 2.0;
    let corners = Corners {
        top_left: radius,
        top_right: radius,
        bottom_right: radius,
        bottom_left: radius,
    };
    let track_alpha = if is_dark_theme(theme) {
        COOKIE_BAR_TRACK_ALPHA_DARK
    } else {
        COOKIE_BAR_TRACK_ALPHA
    };
    quads.push(quad(
        Bounds::from_corners(point(left, top), point(right, bottom)),
        corners,
        rgba((theme.meta << 8) | track_alpha),
        Edges::default(),
        rgba(0),
        BorderStyle::default(),
    ));
    let ratio = ratio.clamp(0.0, 1.0);
    let filled = left + px(f32::from(right - left) * ratio);
    if filled > left {
        quads.push(quad(
            Bounds::from_corners(point(left, top), point(filled, bottom)),
            corners,
            rgb(theme.meta),
            Edges::default(),
            rgba(0),
            BorderStyle::default(),
        ));
    }
}

/// Pulls the bar's ends from the bracket cells to the bracket strokes. The ink boxes
/// come from the shaped run's own font; the em inset stands in when they are missing.
pub(super) fn cookie_bar_insets(
    hit: &HitRow,
    range: &Range<usize>,
    token_width: Pixels,
    window: &gpui::Window,
) -> (Pixels, Pixels) {
    let font_size = hit.layout.font_size();
    // Never eat more than a third of the token per side.
    let max_inset = token_width * 0.35;
    let fallback = (font_size * COOKIE_BAR_SIDE_INSET_EM).min(max_inset);
    let ink = hit
        .layout
        .runs()
        .iter()
        .find(|run| run.glyphs.iter().any(|glyph| glyph.index == range.start))
        .and_then(|run| {
            let text_system = window.text_system();
            let open = text_system
                .typographic_bounds(run.font_id, font_size, '[')
                .ok()?;
            let close = text_system
                .typographic_bounds(run.font_id, font_size, ']')
                .ok()?;
            let advance = text_system.advance(run.font_id, font_size, ']').ok()?.width;
            Some((open.origin.x, advance - close.right()))
        });
    let (left, right) = ink.unwrap_or((fallback, fallback));
    // A font reporting nothing useful must not collapse or invert the bar.
    (
        left.clamp(px(0.0), max_inset),
        right.clamp(px(0.0), max_inset),
    )
}

/// Vertical placement: a gap below the digit baseline, bounded by the row's leading.
pub(super) fn cookie_bar_bounds(
    row_top: Pixels,
    line_height: Pixels,
    ascent: Pixels,
    descent: Pixels,
    font_size: Pixels,
) -> (Pixels, Pixels) {
    let line_height = line_height.max(px(1.0));
    let leading = ((line_height - ascent - descent) / 2.0).max(px(0.0));
    let height = (font_size * COOKIE_BAR_HEIGHT_EM)
        .clamp(px(COOKIE_BAR_MIN_HEIGHT), px(COOKIE_BAR_MAX_HEIGHT))
        .min(line_height);
    let baseline = row_top + leading + ascent;
    // The leading below the glyph box belongs to this row, so the bar may use it.
    let max_bottom = row_top + line_height + leading;
    let top = (baseline + font_size * COOKIE_BAR_GAP_EM)
        .min(max_bottom - height)
        .max(row_top);
    (top, top + height)
}

/// Whether the editor background is dark enough to need the stronger track alpha.
pub(super) fn is_dark_theme(theme: &crate::theme::Theme) -> bool {
    let background = theme.editor_background;
    let luminance = 0.2126 * f32::from((background >> 16) as u8)
        + 0.7152 * f32::from((background >> 8) as u8)
        + 0.0722 * f32::from(background as u8);
    luminance < 128.0
}
