use smallvec::SmallVec;
use std::sync::Arc;

use super::super::{
    display_map::{PreviewDisplayMap, PreviewLineKind, kind_color},
    projection::{ReadingCodeRow, VisualRowKind},
    style::{CodeBlockVariant, PreviewStyle, TableVariant},
    table::TableRowProjection,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::preview) enum PaintToken {
    BackgroundAlt,
    SurfaceElevated,
    CodeBackground,
    CodeBlockAccent,
    Border,
    QuoteBorder,
    Heading(u8),
}

impl PaintToken {
    pub(in crate::preview) fn resolve(self, style: PreviewStyle) -> u32 {
        let palette = style.palette;
        match self {
            Self::BackgroundAlt => palette.surface,
            Self::SurfaceElevated => palette.surface_elevated,
            Self::CodeBackground => palette.code_background,
            Self::CodeBlockAccent => palette.code_block_accent,
            Self::Border => palette.border,
            Self::QuoteBorder => palette.quote_border,
            Self::Heading(level) => palette.heading[level.min(3) as usize],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::preview) enum VisualPrimitive {
    Rect {
        x: f32,
        width: PrimitiveWidth,
        color: PaintToken,
    },
    VerticalLine {
        x: f32,
        color: PaintToken,
    },
    HorizontalLine {
        x: f32,
        width: PrimitiveWidth,
        color: PaintToken,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::preview) enum PrimitiveWidth {
    Fixed(f32),
    Remaining { right_inset: f32 },
}

impl PrimitiveWidth {
    pub(in crate::preview) fn resolve(self, width: f32, x: f32) -> f32 {
        match self {
            Self::Fixed(value) => value,
            Self::Remaining { right_inset } => (width - x - right_inset).max(0.0),
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::preview) enum VisualContent {
    Text,
    Table(TableRowProjection),
    None,
}

#[derive(Clone, Debug)]
pub(in crate::preview) struct VisualRowRecipe {
    pub(in crate::preview) kind: PreviewLineKind,
    pub(in crate::preview) color: u32,
    pub(in crate::preview) indent: f32,
    pub(in crate::preview) list_marker: Option<VisualListMarker>,
    pub(in crate::preview) folded_ellipsis: bool,
    pub(in crate::preview) primitives: SmallVec<[VisualPrimitive; 2]>,
    pub(in crate::preview) content: VisualContent,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::preview) struct VisualListMarker {
    pub(in crate::preview) label: Arc<str>,
    pub(in crate::preview) x: f32,
    pub(in crate::preview) width: f32,
    pub(in crate::preview) right_aligned: bool,
}

/// Resolves semantic paint outside either rendering backend. Callers provide
/// target-space scale inputs; no minimap state is read here.
pub(in crate::preview) fn resolve_visual_row(
    model: &PreviewDisplayMap,
    row: usize,
    target_width: f32,
    target_font_px: f32,
    style: PreviewStyle,
) -> VisualRowRecipe {
    let visual = model
        .projection
        .rows
        .get(row)
        .expect("preview row in bounds");
    let kind = model.row_kind(row);
    let semantic_indent = match visual.kind {
        VisualRowKind::Heading(_) => 6.0,
        VisualRowKind::Quote => 7.0,
        _ => 4.0,
    };
    let list_marker = match &visual.kind {
        VisualRowKind::List(marker) => {
            let scale = target_font_px / style.typography.body_size.max(1.0);
            let right_aligned = marker.checkbox.is_none()
                && (marker.marker.ends_with('.') || marker.marker.ends_with(')'));
            let label: Arc<str> = match marker.checkbox {
                Some(crate::org_syntax::list::CheckboxState::Empty) => Arc::from("□"),
                Some(crate::org_syntax::list::CheckboxState::Partial) => Arc::from("−"),
                Some(crate::org_syntax::list::CheckboxState::Checked) => Arc::from("✓"),
                None if right_aligned => marker.marker.clone(),
                None => Arc::from("•"),
            };
            let reading_width = if marker.checkbox.is_some() {
                style.spacing.checkbox_size.max(14.0)
            } else if right_aligned {
                (label.chars().count() as f32 * 9.0 + 8.0).max(30.0)
            } else {
                18.0
            };
            Some(VisualListMarker {
                label,
                x: 4.0 + f32::from(marker.indent).min(96.0) * scale,
                width: (reading_width * scale).max(1.0),
                right_aligned,
            })
        }
        _ => None,
    };
    let mut primitives = SmallVec::new();
    let content = match &visual.kind {
        VisualRowKind::Code(ReadingCodeRow::End) => VisualContent::None,
        VisualRowKind::Code(_) => {
            primitives.push(VisualPrimitive::Rect {
                x: 2.0,
                width: PrimitiveWidth::Remaining { right_inset: 2.0 },
                color: if style.variants.code_block == CodeBlockVariant::Card {
                    PaintToken::SurfaceElevated
                } else {
                    PaintToken::CodeBackground
                },
            });
            if style.variants.code_block == CodeBlockVariant::AccentBar {
                primitives.push(VisualPrimitive::VerticalLine {
                    x: 2.0,
                    color: PaintToken::CodeBlockAccent,
                });
            } else {
                primitives.push(VisualPrimitive::VerticalLine {
                    x: 2.0,
                    color: PaintToken::Border,
                });
                primitives.push(VisualPrimitive::VerticalLine {
                    x: (target_width - 2.0).max(2.0),
                    color: PaintToken::Border,
                });
            }
            VisualContent::Text
        }
        VisualRowKind::Table(table) => {
            if style.variants.table == TableVariant::Grid || table.is_header() {
                primitives.push(VisualPrimitive::Rect {
                    x: 2.0,
                    width: PrimitiveWidth::Remaining { right_inset: 2.0 },
                    color: PaintToken::BackgroundAlt,
                });
            }
            VisualContent::Table(table.clone())
        }
        VisualRowKind::Quote => {
            primitives.push(VisualPrimitive::VerticalLine {
                x: 2.0,
                color: PaintToken::QuoteBorder,
            });
            VisualContent::Text
        }
        VisualRowKind::Rule => {
            primitives.push(VisualPrimitive::HorizontalLine {
                x: 4.0,
                width: PrimitiveWidth::Remaining { right_inset: 4.0 },
                color: PaintToken::Border,
            });
            VisualContent::None
        }
        // Media is painted as a live image overlay. Its measured row height still
        // participates in layout, but a second raster placeholder would become a
        // visible empty block whenever the overlay and a cached tile swap frames.
        VisualRowKind::Image { .. } | VisualRowKind::Diagram(_) => VisualContent::None,
        VisualRowKind::Heading(level) => {
            primitives.push(VisualPrimitive::Rect {
                x: 3.0,
                width: PrimitiveWidth::Fixed(1.0),
                color: PaintToken::Heading(level.saturating_sub(1)),
            });
            VisualContent::Text
        }
        VisualRowKind::Hidden | VisualRowKind::Blank => VisualContent::None,
        VisualRowKind::Text | VisualRowKind::List(_) | VisualRowKind::Caption => {
            VisualContent::Text
        }
    };
    let indent = list_marker.as_ref().map_or_else(
        || {
            semantic_indent
                + (style.row_layout(visual.style_kind).padding_left * target_font_px
                    / style.typography.body_size)
                    .round()
        },
        |marker| marker.x + marker.width + 8.0 * target_font_px / style.typography.body_size,
    );
    VisualRowRecipe {
        kind,
        color: kind_color(kind, style),
        indent,
        list_marker,
        folded_ellipsis: matches!(visual.kind, VisualRowKind::Heading(_)),
        primitives,
        content,
    }
}
