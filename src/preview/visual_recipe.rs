use smallvec::SmallVec;

use super::{
    display_map::{PreviewDisplayMap, PreviewLineKind, kind_color},
    projection::{ReadingCodeRow, VisualRowKind},
    table::TableRowProjection,
};
use crate::theme::current_theme;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::preview) enum PaintToken {
    BackgroundAlt,
    CodeBackground,
    CodeBlockAccent,
    Border,
    Attribute,
    Heading(u8),
}

impl PaintToken {
    pub(in crate::preview) fn resolve(self) -> u32 {
        let theme = current_theme();
        match self {
            Self::BackgroundAlt => theme.background_alt,
            Self::CodeBackground => theme.code_background,
            Self::CodeBlockAccent => theme.code_block_accent,
            Self::Border => theme.border,
            Self::Attribute => theme.attribute,
            Self::Heading(level) => theme.heading[level.min(3) as usize],
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
    pub(in crate::preview) folded_ellipsis: bool,
    pub(in crate::preview) primitives: SmallVec<[VisualPrimitive; 2]>,
    pub(in crate::preview) content: VisualContent,
}

/// Resolves semantic paint outside either rendering backend. Callers provide
/// target-space scale inputs; no minimap state is read here.
pub(in crate::preview) fn resolve_visual_row(
    model: &PreviewDisplayMap,
    row: usize,
    target_width: f32,
    target_font_px: f32,
) -> VisualRowRecipe {
    let visual = model
        .projection
        .rows
        .get(row)
        .expect("preview row in bounds");
    let kind = model.row_kind(row);
    let semantic_indent = match visual.kind {
        VisualRowKind::Heading(_) | VisualRowKind::List(_) => 6.0,
        VisualRowKind::Quote => 7.0,
        _ => 4.0,
    };
    let mut primitives = SmallVec::new();
    let content = match &visual.kind {
        VisualRowKind::Code(ReadingCodeRow::End) => VisualContent::None,
        VisualRowKind::Code(_) => {
            primitives.push(VisualPrimitive::Rect {
                x: 2.0,
                width: PrimitiveWidth::Remaining { right_inset: 2.0 },
                color: PaintToken::CodeBackground,
            });
            primitives.push(VisualPrimitive::VerticalLine {
                x: 2.0,
                color: PaintToken::CodeBlockAccent,
            });
            VisualContent::Text
        }
        VisualRowKind::Table(table) => {
            primitives.push(VisualPrimitive::Rect {
                x: 2.0,
                width: PrimitiveWidth::Remaining { right_inset: 2.0 },
                color: PaintToken::BackgroundAlt,
            });
            VisualContent::Table(table.clone())
        }
        VisualRowKind::Quote => {
            primitives.push(VisualPrimitive::VerticalLine {
                x: 2.0,
                color: PaintToken::Heading(1),
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
        VisualRowKind::Image { dimensions } => {
            let fitted_width =
                dimensions.map_or(target_width.max(10.0) - 10.0, |(width, height)| {
                    super::fitted_image_size(width, height, target_width.max(10.0) - 10.0).0
                });
            primitives.push(VisualPrimitive::Rect {
                x: 5.0,
                width: PrimitiveWidth::Fixed(fitted_width),
                color: PaintToken::Attribute,
            });
            VisualContent::None
        }
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
    VisualRowRecipe {
        kind,
        color: kind_color(kind),
        indent: semantic_indent + (visual.layout.padding_left * target_font_px / 14.0).round(),
        folded_ellipsis: matches!(visual.kind, VisualRowKind::Heading(_)),
        primitives,
        content,
    }
}
