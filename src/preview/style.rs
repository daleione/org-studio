use std::hash::{Hash, Hasher};

use crate::i18n::Language;

use super::display_map::RowLayout;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum PreviewStyleId {
    #[default]
    Base,
    WarmClay,
}

impl PreviewStyleId {
    pub const ALL: [Self; 2] = [Self::Base, Self::WarmClay];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::WarmClay => "warm-clay",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "base" => Some(Self::Base),
            "warm-clay" => Some(Self::WarmClay),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::preview) enum TableVariant {
    Grid,
    HorizontalRules,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::preview) enum CodeBlockVariant {
    AccentBar,
    Card,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::preview) struct PreviewComponentVariants {
    pub(in crate::preview) table: TableVariant,
    pub(in crate::preview) code_block: CodeBlockVariant,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::preview) struct PreviewPalette {
    pub(in crate::preview) background: u32,
    pub(in crate::preview) surface: u32,
    pub(in crate::preview) surface_elevated: u32,
    pub(in crate::preview) hover: u32,
    pub(in crate::preview) foreground: u32,
    pub(in crate::preview) foreground_dim: u32,
    pub(in crate::preview) border: u32,
    pub(in crate::preview) border_strong: u32,
    pub(in crate::preview) heading: [u32; 4],
    pub(in crate::preview) accent: u32,
    pub(in crate::preview) accent_text: u32,
    pub(in crate::preview) accent_contrast: u32,
    pub(in crate::preview) code_background: u32,
    pub(in crate::preview) code_boundary_background: u32,
    pub(in crate::preview) code_block_accent: u32,
    pub(in crate::preview) code_foreground: u32,
    pub(in crate::preview) code_boundary: u32,
    pub(in crate::preview) quote: u32,
    pub(in crate::preview) quote_border: u32,
    pub(in crate::preview) link: u32,
    pub(in crate::preview) meta: u32,
    pub(in crate::preview) inline_code: u32,
    pub(in crate::preview) inline_code_background: u32,
    pub(in crate::preview) date: u32,
    pub(in crate::preview) keyword: u32,
    pub(in crate::preview) string: u32,
    pub(in crate::preview) comment: u32,
    pub(in crate::preview) type_name: u32,
    pub(in crate::preview) function: u32,
    pub(in crate::preview) constant: u32,
    pub(in crate::preview) number: u32,
    pub(in crate::preview) variable: u32,
    pub(in crate::preview) operator: u32,
    pub(in crate::preview) attribute: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::preview) struct PreviewTypography {
    pub(in crate::preview) body_family: &'static str,
    pub(in crate::preview) body_fallbacks: &'static [&'static str],
    pub(in crate::preview) code_family: &'static str,
    pub(in crate::preview) code_fallbacks: &'static [&'static str],
    pub(in crate::preview) body_size: f32,
    pub(in crate::preview) body_line_height: f32,
    pub(in crate::preview) heading_sizes: [f32; 4],
    pub(in crate::preview) heading_line_heights: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::preview) struct PreviewSpacing {
    pub(in crate::preview) content_min_width: f32,
    pub(in crate::preview) content_max_width: f32,
    pub(in crate::preview) wide_pane_fill: f32,
    pub(in crate::preview) horizontal_padding: f32,
    pub(in crate::preview) content_padding_top: f32,
    pub(in crate::preview) content_padding_bottom: f32,
    pub(in crate::preview) paragraph_min_height: f32,
    pub(in crate::preview) block_gap: f32,
    pub(in crate::preview) table_cell_x: f32,
    pub(in crate::preview) table_cell_y: f32,
    pub(in crate::preview) quote_line_width: f32,
    pub(in crate::preview) checkbox_size: f32,
    pub(in crate::preview) radius: f32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::preview) enum RowStyleKind {
    Heading(u8),
    Paragraph,
    Blank,
    List,
    Keyword,
    Image,
    Metadata,
    Code,
    RawCode,
    Quote,
    CompactQuote,
    Verse,
    Center,
    Special,
    Drawer,
    Rule,
    Hidden,
    Table,
    Caption,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::preview) struct PreviewStyle {
    pub id: PreviewStyleId,
    pub(in crate::preview) name_zh: &'static str,
    pub(in crate::preview) name_en: &'static str,
    pub(in crate::preview) palette: PreviewPalette,
    pub(in crate::preview) typography: PreviewTypography,
    pub(in crate::preview) spacing: PreviewSpacing,
    pub(in crate::preview) variants: PreviewComponentVariants,
}

impl PreviewStyle {
    pub fn name(self, language: Language) -> &'static str {
        match language {
            Language::Chinese => self.name_zh,
            Language::English => self.name_en,
        }
    }

    pub(in crate::preview) fn paint_key(self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.id.hash(&mut hasher);
        self.palette.hash(&mut hasher);
        hasher.finish()
    }

    pub(in crate::preview) fn layout_key(self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.typography.body_family.hash(&mut hasher);
        self.typography.body_fallbacks.hash(&mut hasher);
        self.typography.code_family.hash(&mut hasher);
        self.typography.code_fallbacks.hash(&mut hasher);
        self.typography.body_size.to_bits().hash(&mut hasher);
        self.typography.body_line_height.to_bits().hash(&mut hasher);
        for value in self.typography.heading_sizes {
            value.to_bits().hash(&mut hasher);
        }
        for value in self.typography.heading_line_heights {
            value.to_bits().hash(&mut hasher);
        }
        for value in [
            self.spacing.content_min_width,
            self.spacing.content_max_width,
            self.spacing.wide_pane_fill,
            self.spacing.horizontal_padding,
            self.spacing.content_padding_top,
            self.spacing.content_padding_bottom,
            self.spacing.paragraph_min_height,
            self.spacing.block_gap,
            self.spacing.table_cell_x,
            self.spacing.table_cell_y,
            self.spacing.quote_line_width,
            self.spacing.checkbox_size,
            self.spacing.radius,
        ] {
            value.to_bits().hash(&mut hasher);
        }
        self.variants.hash(&mut hasher);
        hasher.finish()
    }

    pub(in crate::preview) fn row_layout(self, kind: RowStyleKind) -> RowLayout {
        let body = || RowLayout {
            min_height: self.spacing.paragraph_min_height,
            ..RowLayout::text(self.typography.body_size, self.typography.body_line_height)
        };
        match kind {
            RowStyleKind::Heading(level) => {
                let index = level.saturating_sub(1).min(3) as usize;
                RowLayout {
                    margin_top: if level <= 2 {
                        self.spacing.block_gap
                    } else {
                        0.0
                    },
                    margin_bottom: self.spacing.block_gap * 0.35,
                    min_height: self.typography.heading_line_heights[index],
                    ..RowLayout::text(
                        self.typography.heading_sizes[index],
                        self.typography.heading_line_heights[index],
                    )
                }
            }
            RowStyleKind::Paragraph => body(),
            RowStyleKind::Blank => RowLayout {
                fixed_height: Some(self.spacing.block_gap),
                min_height: self.spacing.block_gap,
                ..RowLayout::text(self.typography.body_size, self.typography.body_line_height)
            },
            RowStyleKind::List => RowLayout {
                padding_left: 8.0,
                padding_top: 2.0,
                padding_bottom: 2.0,
                ..body()
            },
            RowStyleKind::Keyword | RowStyleKind::Metadata => RowLayout {
                min_height: self.typography.body_line_height,
                ..RowLayout::text(
                    (self.typography.body_size - 1.0).max(11.0),
                    (self.typography.body_line_height - 2.0).max(16.0),
                )
            },
            RowStyleKind::Image => RowLayout {
                padding_top: self.spacing.block_gap * 0.65,
                padding_bottom: self.spacing.block_gap * 0.65,
                ..body()
            },
            RowStyleKind::Code => RowLayout {
                padding_left: 16.0,
                padding_right: 16.0,
                padding_top: 3.0,
                padding_bottom: 3.0,
                min_height: (self.typography.body_line_height - 3.0).max(18.0),
                ..RowLayout::text(
                    (self.typography.body_size - 2.0).max(12.0),
                    (self.typography.body_line_height - 5.0).max(18.0),
                )
            },
            RowStyleKind::RawCode | RowStyleKind::Special => RowLayout {
                padding_left: 16.0,
                padding_right: 16.0,
                padding_top: 3.0,
                padding_bottom: 3.0,
                ..self.row_layout(RowStyleKind::Code)
            },
            RowStyleKind::Quote | RowStyleKind::CompactQuote => RowLayout {
                padding_left: 16.0,
                padding_right: 8.0,
                padding_top: if matches!(kind, RowStyleKind::CompactQuote) {
                    4.0
                } else {
                    8.0
                },
                padding_bottom: if matches!(kind, RowStyleKind::CompactQuote) {
                    4.0
                } else {
                    8.0
                },
                ..body()
            },
            RowStyleKind::Verse => RowLayout::text(13.0, 22.0),
            RowStyleKind::Center => body(),
            RowStyleKind::Drawer => RowLayout {
                padding_left: 12.0,
                padding_right: 12.0,
                padding_top: 2.0,
                padding_bottom: 2.0,
                ..self.row_layout(RowStyleKind::Metadata)
            },
            RowStyleKind::Rule => RowLayout {
                margin_top: self.spacing.block_gap,
                margin_bottom: self.spacing.block_gap,
                fixed_height: Some(1.0),
                ..body()
            },
            RowStyleKind::Hidden => RowLayout {
                min_height: 0.0,
                fixed_height: Some(0.0),
                ..RowLayout::text(self.typography.body_size, 0.0)
            },
            RowStyleKind::Table => RowLayout {
                min_height: self.typography.body_line_height,
                ..RowLayout::text(
                    (self.typography.body_size - 1.0).max(12.0),
                    (self.typography.body_line_height - 3.0).max(18.0),
                )
            },
            RowStyleKind::Caption => RowLayout {
                padding_top: 4.0,
                padding_bottom: 8.0,
                min_height: 18.0,
                ..RowLayout::text(
                    (self.typography.body_size - 3.0).max(11.0),
                    (self.typography.body_line_height - 7.0).max(17.0),
                )
            },
        }
    }
}

const BASE_STYLE: PreviewStyle = PreviewStyle {
    id: PreviewStyleId::Base,
    name_zh: "基础",
    name_en: "Base",
    palette: PreviewPalette {
        background: 0xffffff,
        surface: 0xf5f5f7,
        surface_elevated: 0xffffff,
        hover: 0xececf0,
        foreground: 0x3a3a3c,
        foreground_dim: 0x9a9aa0,
        border: 0xd8d8dc,
        border_strong: 0xb8b8c0,
        heading: [0x3a81c3, 0x2d9574, 0x67a817, 0xa88412],
        accent: 0x3a81c3,
        accent_text: 0x2d6fa8,
        accent_contrast: 0xffffff,
        code_background: 0xf5f2f7,
        code_boundary_background: 0xece8ef,
        code_block_accent: 0xbeb2da,
        code_foreground: 0x4a4352,
        code_boundary: 0x827591,
        quote: 0x655370,
        quote_border: 0x3a81c3,
        link: 0x3a81c3,
        meta: 0x9f8766,
        inline_code: 0x087e8b,
        inline_code_background: 0xf1eef3,
        date: 0x715ab1,
        keyword: 0x3a81c3,
        string: 0x2d9574,
        comment: 0x258c96,
        type_name: 0xb23a63,
        function: 0x6c3163,
        constant: 0x4e3163,
        number: 0x9a7b10,
        variable: 0x715ab1,
        operator: 0x655370,
        attribute: 0xdc752f,
    },
    typography: PreviewTypography {
        body_family: "Menlo",
        body_fallbacks: &["PingFang SC", "Apple Color Emoji"],
        code_family: "Menlo",
        code_fallbacks: &[".SF NS Mono", "PingFang SC", "Apple Color Emoji"],
        body_size: 14.0,
        body_line_height: 22.0,
        heading_sizes: [22.0, 18.0, 15.0, 14.0],
        heading_line_heights: [24.0; 4],
    },
    spacing: PreviewSpacing {
        content_min_width: 960.0,
        content_max_width: 960.0,
        wide_pane_fill: 1.0,
        horizontal_padding: 48.0,
        content_padding_top: 4.0,
        content_padding_bottom: 8.0,
        paragraph_min_height: 24.0,
        block_gap: 20.0,
        table_cell_x: 12.0,
        table_cell_y: 8.0,
        quote_line_width: 2.0,
        checkbox_size: 16.0,
        radius: 0.0,
    },
    variants: PreviewComponentVariants {
        table: TableVariant::Grid,
        code_block: CodeBlockVariant::AccentBar,
    },
};

const WARM_CLAY_STYLE: PreviewStyle = PreviewStyle {
    id: PreviewStyleId::WarmClay,
    name_zh: "暖陶",
    name_en: "Warm Clay",
    palette: PreviewPalette {
        background: 0xf9f9f7,
        surface: 0xf4f4f2,
        surface_elevated: 0xffffff,
        hover: 0xecece9,
        foreground: 0x2d2d2b,
        foreground_dim: 0x6b6b67,
        border: 0xdcdcd8,
        border_strong: 0x969692,
        heading: [0x2d2d2b, 0x323230, 0x3a3a37, 0x454541],
        accent: 0xcc7d5e,
        accent_text: 0xa95639,
        accent_contrast: 0x20201f,
        code_background: 0xfefefd,
        code_boundary_background: 0xf4f1ee,
        code_block_accent: 0xcc7d5e,
        code_foreground: 0x2d2d2b,
        code_boundary: 0x6b6b67,
        quote: 0x4f4f4b,
        quote_border: 0xe6c1b2,
        link: 0xa95639,
        meta: 0x6b6b67,
        inline_code: 0x8a3f27,
        inline_code_background: 0xf1efec,
        date: 0x8a5a16,
        keyword: 0xa83f24,
        string: 0x007a3d,
        comment: 0x6b6b67,
        type_name: 0x9a3f35,
        function: 0x365f9d,
        constant: 0x8a5a16,
        number: 0x8a5a16,
        variable: 0x815025,
        operator: 0x4f4f4b,
        attribute: 0xa95639,
    },
    typography: PreviewTypography {
        body_family: "System Font",
        body_fallbacks: &["PingFang SC", "Arial Unicode", "Apple Color Emoji"],
        code_family: "JetBrains Mono",
        code_fallbacks: &[".SF NS Mono", "Menlo", "PingFang SC", "Apple Color Emoji"],
        body_size: 16.0,
        body_line_height: 27.52,
        heading_sizes: [24.0, 20.0, 17.0, 16.0],
        heading_line_heights: [32.0, 28.0, 26.0, 24.0],
    },
    spacing: PreviewSpacing {
        content_min_width: 752.0,
        content_max_width: 1120.0,
        wide_pane_fill: 0.78,
        horizontal_padding: 32.0,
        content_padding_top: 36.0,
        content_padding_bottom: 70.0,
        paragraph_min_height: 28.0,
        block_gap: 12.0,
        table_cell_x: 14.0,
        table_cell_y: 10.0,
        quote_line_width: 4.0,
        checkbox_size: 13.0,
        radius: 8.0,
    },
    variants: PreviewComponentVariants {
        table: TableVariant::HorizontalRules,
        code_block: CodeBlockVariant::Card,
    },
};

pub(in crate::preview) fn preview_style(id: PreviewStyleId) -> &'static PreviewStyle {
    match id {
        PreviewStyleId::Base => &BASE_STYLE,
        PreviewStyleId::WarmClay => &WARM_CLAY_STYLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_ids_round_trip_and_unknown_values_do_not_alias() {
        for id in PreviewStyleId::ALL {
            assert_eq!(PreviewStyleId::parse(id.as_str()), Some(id));
        }
        assert_eq!(PreviewStyleId::parse("default"), None);
    }

    #[test]
    fn built_in_style_keys_are_distinct() {
        let base = *preview_style(PreviewStyleId::Base);
        let warm = *preview_style(PreviewStyleId::WarmClay);
        assert_ne!(base.paint_key(), warm.paint_key());
        assert_ne!(base.layout_key(), warm.layout_key());
    }

    #[test]
    fn style_identity_does_not_hide_layout_rules() {
        let base = *preview_style(PreviewStyleId::Base);
        let mut renamed = base;
        renamed.id = PreviewStyleId::WarmClay;
        assert_eq!(base.layout_key(), renamed.layout_key());
        for kind in [
            RowStyleKind::Heading(1),
            RowStyleKind::Heading(4),
            RowStyleKind::Paragraph,
            RowStyleKind::List,
            RowStyleKind::Code,
            RowStyleKind::Quote,
            RowStyleKind::Table,
        ] {
            assert_eq!(base.row_layout(kind), renamed.row_layout(kind));
        }
    }

    #[test]
    fn warm_clay_contract_and_readable_text_contrast_are_stable() {
        let style = *preview_style(PreviewStyleId::WarmClay);
        assert_eq!(style.name(Language::Chinese), "暖陶");
        assert_eq!(style.name(Language::English), "Warm Clay");
        assert_eq!(style.palette.background, 0xf9f9f7);
        assert_eq!(style.palette.accent, 0xcc7d5e);
        assert_eq!(style.typography.body_size, 16.0);
        assert!(style.typography.body_fallbacks.contains(&"PingFang SC"));
        assert!(style.typography.code_fallbacks.contains(&"Menlo"));
        assert_eq!(style.spacing.content_min_width, 752.0);
        assert_eq!(style.spacing.content_max_width, 1120.0);
        assert_eq!(style.spacing.wide_pane_fill, 0.78);
        assert_eq!(style.variants.table, TableVariant::HorizontalRules);
        assert_eq!(style.variants.code_block, CodeBlockVariant::Card);
        assert!(contrast(style.palette.foreground, style.palette.background) >= 7.0);
        assert!(contrast(style.palette.accent_text, style.palette.background) >= 4.5);
        assert!(contrast(style.palette.accent_contrast, style.palette.accent) >= 4.5);
    }

    fn contrast(left: u32, right: u32) -> f64 {
        fn luminance(color: u32) -> f64 {
            let channel = |shift: u32| {
                let value = f64::from(((color >> shift) & 0xff_u32) as u8) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
        }
        let (lighter, darker) = {
            let left = luminance(left);
            let right = luminance(right);
            if left >= right {
                (left, right)
            } else {
                (right, left)
            }
        };
        (lighter + 0.05) / (darker + 0.05)
    }
}
