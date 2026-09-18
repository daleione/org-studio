//! Block chrome: decorated block rows, their backgrounds and the source
//! gutter bars behind line-number cells.

use gpui::{BorderStyle, Bounds, Corners, Edges, PaintQuad, Pixels, fill, point, px, quad, rgba};

use crate::editor::syntax;
use crate::theme::Theme;

use super::state::PaintRow;
use super::{
    BLOCK_LEFT_INSET, BLOCK_RADIUS, BLOCK_RIGHT_INSET, BLOCK_TEXT_INSET, BLOCK_TEXT_RIGHT_PADDING,
    BLOCK_VERTICAL_INSET, SOURCE_GUTTER_INSET, SOURCE_GUTTER_WIDTH, SOURCE_TEXT_INSET,
};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct EditorBlockPaintRow {
    pub(super) block: Option<syntax::EditorBlockDecoration>,
    pub(super) top: f32,
    pub(super) bottom: f32,
    pub(super) active: bool,
    pub(super) folded: bool,
}

pub(super) fn editor_block_text_inset(block: Option<&syntax::EditorBlockDecoration>) -> f32 {
    block.map_or(0.0, |block| {
        if is_source_block_kind(&block.kind) {
            SOURCE_TEXT_INSET
        } else {
            BLOCK_TEXT_INSET
        }
    })
}

pub(super) fn is_source_block_kind(kind: &syntax::EditorBlockKind) -> bool {
    matches!(
        kind,
        syntax::EditorBlockKind::Source | syntax::EditorBlockKind::MarkdownFence
    )
}

pub(super) fn editor_block_accent(
    kind: &syntax::EditorBlockKind,
    theme: &crate::theme::Theme,
) -> u32 {
    match kind {
        syntax::EditorBlockKind::Source => theme.meta,
        syntax::EditorBlockKind::Example => theme.attribute,
        syntax::EditorBlockKind::Quote => theme.string,
        syntax::EditorBlockKind::Verse => theme.function,
        syntax::EditorBlockKind::Center => theme.heading[2],
        syntax::EditorBlockKind::Comment => theme.comment,
        syntax::EditorBlockKind::Export => theme.type_name,
        syntax::EditorBlockKind::Special(_) => theme.foreground_dim,
        syntax::EditorBlockKind::MarkdownFence => theme.meta,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct EditorBlockPaintSegment {
    pub(super) kind: syntax::EditorBlockKind,
    pub(super) top: f32,
    pub(super) bottom: f32,
    pub(super) open: bool,
    pub(super) close: bool,
    pub(super) active: bool,
}

pub(super) fn editor_block_segments(
    rows: impl IntoIterator<Item = EditorBlockPaintRow>,
) -> Vec<EditorBlockPaintSegment> {
    let mut segments = Vec::new();
    let mut current: Option<EditorBlockPaintSegment> = None;
    for row in rows {
        let Some(block) = row.block else {
            if let Some(segment) = current.take() {
                segments.push(segment);
            }
            continue;
        };
        let starts_new = current.as_ref().is_some_and(|segment| {
            block.edge == syntax::EditorBlockEdge::Open
                || block.kind != segment.kind
                || (row.top - segment.bottom).abs() > 0.75
        });
        if starts_new && let Some(segment) = current.take() {
            segments.push(segment);
        }
        let open = block.edge == syntax::EditorBlockEdge::Open;
        let close = block.edge == syntax::EditorBlockEdge::Close || row.folded;
        let segment = current.get_or_insert(EditorBlockPaintSegment {
            kind: block.kind.clone(),
            top: row.top,
            bottom: row.bottom,
            open,
            close,
            active: row.active,
        });
        segment.bottom = segment.bottom.max(row.bottom);
        segment.open |= open;
        segment.close |= close;
        segment.active |= row.active;
        if close && let Some(segment) = current.take() {
            segments.push(segment);
        }
    }
    if let Some(segment) = current {
        segments.push(segment);
    }
    segments
}

pub(super) fn editor_block_horizontal_bounds(
    viewport_left: Pixels,
    viewport_right: Pixels,
    scroll_x: f32,
) -> (Pixels, Pixels) {
    let offset = px(scroll_x);
    (viewport_left - offset, viewport_right - offset)
}

pub(super) fn editor_block_backgrounds(
    rows: &[PaintRow],
    text_left: Pixels,
    text_right: Pixels,
    theme: &crate::theme::Theme,
) -> Vec<PaintQuad> {
    let left = text_left + px(BLOCK_LEFT_INSET);
    let right = text_right - px(BLOCK_RIGHT_INSET);
    if right <= left {
        return Vec::new();
    }
    editor_block_segments(rows.iter().map(|row| EditorBlockPaintRow {
        block: row.block.clone(),
        top: f32::from(row.hit.visible_top),
        bottom: f32::from(row.hit.visible_bottom),
        active: row.active,
        folded: row.folded,
    }))
    .into_iter()
    .filter_map(|segment| {
        let top = segment.top
            + if segment.open {
                BLOCK_VERTICAL_INSET
            } else {
                0.0
            };
        let bottom = segment.bottom
            - if segment.close {
                BLOCK_VERTICAL_INSET
            } else {
                0.0
            };
        if bottom <= top {
            return None;
        }
        let accent = editor_block_accent(&segment.kind, theme);
        let radius = px(BLOCK_RADIUS);
        let zero = px(0.0);
        let border = px(1.0);
        let corners = Corners {
            top_left: if segment.open { radius } else { zero },
            top_right: if segment.open { radius } else { zero },
            bottom_right: if segment.close { radius } else { zero },
            bottom_left: if segment.close { radius } else { zero },
        };
        let border_widths = Edges {
            top: if segment.open { border } else { zero },
            right: border,
            bottom: if segment.close { border } else { zero },
            left: border,
        };
        let fill_alpha = if segment.active { 0x11 } else { 0x0c };
        let border_alpha = if segment.active { 0x57 } else { 0x2e };
        Some(quad(
            Bounds::from_corners(point(left, px(top)), point(right, px(bottom))),
            corners,
            rgba((accent << 8) | fill_alpha),
            border_widths,
            rgba((accent << 8) | border_alpha),
            BorderStyle::default(),
        ))
    })
    .collect()
}

pub(super) fn editor_source_gutter_backgrounds(
    rows: &[PaintRow],
    text_left: Pixels,
    text_right: Pixels,
    theme: &crate::theme::Theme,
) -> Vec<PaintQuad> {
    let left = text_left + px(BLOCK_LEFT_INSET + SOURCE_GUTTER_INSET);
    let right = left + px(SOURCE_GUTTER_WIDTH);
    if right >= text_right - px(BLOCK_RIGHT_INSET) {
        return Vec::new();
    }
    editor_block_segments(rows.iter().map(|row| EditorBlockPaintRow {
        block: row.block.clone(),
        top: f32::from(row.hit.visible_top),
        bottom: f32::from(row.hit.visible_bottom),
        active: row.active,
        folded: row.folded,
    }))
    .into_iter()
    .filter(|segment| is_source_block_kind(&segment.kind))
    .filter_map(|segment| {
        let top = segment.top
            + if segment.open {
                BLOCK_VERTICAL_INSET + SOURCE_GUTTER_INSET
            } else {
                0.0
            };
        let bottom = segment.bottom
            - if segment.close {
                BLOCK_VERTICAL_INSET + SOURCE_GUTTER_INSET
            } else {
                0.0
            };
        if bottom <= top {
            return None;
        }
        let accent = editor_block_accent(&segment.kind, theme);
        let radius = px((BLOCK_RADIUS - SOURCE_GUTTER_INSET).max(0.0));
        let zero = px(0.0);
        let corners = Corners {
            top_left: if segment.open { radius } else { zero },
            top_right: zero,
            bottom_right: zero,
            bottom_left: if segment.close { radius } else { zero },
        };
        Some(quad(
            Bounds::from_corners(point(left, px(top)), point(right, px(bottom))),
            corners,
            rgba((accent << 8) | 0x06),
            Edges {
                top: zero,
                right: px(1.0),
                bottom: zero,
                left: zero,
            },
            rgba((accent << 8) | 0x24),
            BorderStyle::default(),
        ))
    })
    .collect()
}

pub(super) fn editor_row_background(
    style: syntax::EditorStyleId,
    active: bool,
    bounds: Bounds<Pixels>,
    theme: &crate::theme::Theme,
) -> Option<PaintQuad> {
    let color = match style {
        syntax::EditorStyleId::CodeBoundary | syntax::EditorStyleId::Code => return None,
        _ if active => theme.background_alt,
        _ => return None,
    };
    Some(fill(bounds, gpui::rgb(color)))
}

/// Block chrome geometry: the backgrounds drawn behind decorated blocks and
/// the horizontal bounds the source buttons align to.
pub(super) struct BlockChrome {
    pub(super) backgrounds: Vec<PaintQuad>,
    pub(super) left: Pixels,
    pub(super) content_right: Pixels,
    pub(super) viewport_left: Pixels,
    pub(super) viewport_right: Pixels,
}

/// Collects the block and source-gutter backgrounds for the painted rows.
pub(super) fn prepare_block_chrome(
    rows: &[PaintRow],
    bounds: Bounds<Pixels>,
    gutter_width: f32,
    minimap_layout_width: f32,
    horizontal_scroll: f32,
    theme: &Theme,
) -> BlockChrome {
    let viewport_text_left = bounds.left() + px(gutter_width);
    let viewport_text_right = bounds.right() - px(minimap_layout_width);
    let (block_left, block_minimum_right) =
        editor_block_horizontal_bounds(viewport_text_left, viewport_text_right, horizontal_scroll);
    let block_content_right = rows
        .iter()
        .filter(|row| row.block.is_some())
        .map(|row| {
            row.hit.text_origin_x
                + row.hit.visual_width()
                + px(BLOCK_TEXT_RIGHT_PADDING + BLOCK_RIGHT_INSET)
        })
        .fold(block_minimum_right, |right, candidate| right.max(candidate));
    let mut backgrounds = editor_block_backgrounds(rows, block_left, block_content_right, theme);
    backgrounds.extend(editor_source_gutter_backgrounds(
        rows,
        block_left,
        block_content_right,
        theme,
    ));
    BlockChrome {
        backgrounds,
        left: block_left,
        content_right: block_content_right,
        viewport_left: viewport_text_left,
        viewport_right: viewport_text_right,
    }
}
