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
pub(crate) enum TableVariant {
    Grid,
    HorizontalRules,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CodeBlockVariant {
    AccentBar,
    Card,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PreviewComponentVariants {
    pub(crate) table: TableVariant,
    pub(crate) code_block: CodeBlockVariant,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PreviewPalette {
    pub(crate) background: u32,
    pub(crate) surface: u32,
    pub(crate) surface_elevated: u32,
    pub(crate) hover: u32,
    pub(crate) foreground: u32,
    pub(crate) foreground_dim: u32,
    pub(crate) border: u32,
    pub(crate) border_strong: u32,
    pub(crate) heading: [u32; 4],
    pub(crate) accent: u32,
    pub(crate) accent_text: u32,
    pub(crate) accent_contrast: u32,
    pub(crate) code_background: u32,
    pub(crate) code_boundary_background: u32,
    pub(crate) code_block_accent: u32,
    pub(crate) code_foreground: u32,
    pub(crate) code_boundary: u32,
    pub(crate) quote: u32,
    pub(crate) quote_border: u32,
    pub(crate) link: u32,
    pub(crate) meta: u32,
    pub(crate) inline_code: u32,
    pub(crate) inline_code_background: u32,
    pub(crate) date: u32,
    pub(crate) keyword: u32,
    pub(crate) string: u32,
    pub(crate) comment: u32,
    pub(crate) type_name: u32,
    pub(crate) function: u32,
    pub(crate) constant: u32,
    pub(crate) number: u32,
    pub(crate) variable: u32,
    pub(crate) operator: u32,
    pub(crate) attribute: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PreviewTypography {
    pub(crate) body_family: &'static str,
    pub(crate) body_fallbacks: &'static [&'static str],
    pub(crate) code_family: &'static str,
    pub(crate) code_fallbacks: &'static [&'static str],
    pub(crate) body_size: f32,
    pub(crate) body_line_height: f32,
    pub(crate) heading_sizes: [f32; 4],
    pub(crate) heading_line_heights: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PreviewSpacing {
    pub(crate) content_min_width: f32,
    pub(crate) content_max_width: f32,
    pub(crate) wide_pane_fill: f32,
    pub(crate) horizontal_padding: f32,
    pub(crate) content_padding_top: f32,
    pub(crate) content_padding_bottom: f32,
    pub(crate) paragraph_min_height: f32,
    pub(crate) block_gap: f32,
    pub(crate) table_cell_x: f32,
    pub(crate) table_cell_y: f32,
    pub(crate) quote_line_width: f32,
    pub(crate) checkbox_size: f32,
    pub(crate) radius: f32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RowStyleKind {
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
pub(crate) struct PreviewStyle {
    pub id: PreviewStyleId,
    pub(crate) name_zh: &'static str,
    pub(crate) name_en: &'static str,
    pub(crate) palette: PreviewPalette,
    pub(crate) typography: PreviewTypography,
    pub(crate) spacing: PreviewSpacing,
    pub(crate) variants: PreviewComponentVariants,
}

impl PreviewStyle {
    pub fn name(self, language: Language) -> &'static str {
        match language {
            Language::Chinese => self.name_zh,
            Language::English => self.name_en,
        }
    }

    pub(crate) fn paint_key(self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.id.hash(&mut hasher);
        self.palette.hash(&mut hasher);
        hasher.finish()
    }

    pub(crate) fn layout_key(self) -> u64 {
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

    pub(crate) fn row_layout(self, kind: RowStyleKind) -> RowLayout {
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
        content_padding_bottom: 70.0,
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

// The reading styles own their document-identity palettes (typography, accent
// hues, spacing). Dark mode keeps each style's accent identity but swaps the
// neutral ramps to the product dark palette: cold blue-grey for Base, a warm
// dark ramp for Warm Clay so its character survives.
const BASE_STYLE_DARK: PreviewStyle = PreviewStyle {
    palette: PreviewPalette {
        background: 0x121d28,
        surface: 0x18232f,
        surface_elevated: 0x1e2b39,
        hover: 0x253444,
        foreground: 0xe6edf3,
        foreground_dim: 0x8b98a7,
        border: 0x253445,
        border_strong: 0x33455c,
        heading: [0x5cb8ff, 0x36d399, 0xc3e88d, 0xffcb6b],
        accent: 0x4da3ff,
        accent_text: 0x66b3ff,
        accent_contrast: 0x0b1118,
        code_background: 0x16222e,
        code_boundary_background: 0x1a2635,
        code_block_accent: 0x2f6394,
        code_foreground: 0xe6edf3,
        code_boundary: 0x536171,
        quote: 0xc5ced8,
        quote_border: 0x4da3ff,
        link: 0x4da3ff,
        meta: 0xf5b84b,
        inline_code: 0xffcb6b,
        inline_code_background: 0x18232f,
        date: 0x66b3ff,
        keyword: 0xc792ea,
        string: 0xc3e88d,
        comment: 0x637587,
        type_name: 0xffcb6b,
        function: 0x82aaff,
        constant: 0xf78c6c,
        number: 0xf78c6c,
        variable: 0x66b3ff,
        operator: 0x89ddff,
        attribute: 0x82aaff,
    },
    ..BASE_STYLE
};

const WARM_CLAY_STYLE_DARK: PreviewStyle = PreviewStyle {
    palette: PreviewPalette {
        background: 0x151210,
        surface: 0x1c1815,
        surface_elevated: 0x231e1a,
        hover: 0x2a241f,
        foreground: 0xe8e2da,
        foreground_dim: 0x9a938a,
        border: 0x332c26,
        border_strong: 0x4a413a,
        heading: [0xe8e2da, 0xded7cd, 0xd3cbbf, 0xc7bfb1],
        accent: 0xd98b6a,
        accent_text: 0xe0977a,
        accent_contrast: 0x151210,
        code_background: 0x1c1815,
        code_boundary_background: 0x262019,
        code_block_accent: 0xd98b6a,
        code_foreground: 0xe8e2da,
        code_boundary: 0x9a938a,
        quote: 0xcfc8bf,
        quote_border: 0xe6c1b2,
        link: 0xe0977a,
        meta: 0x9a938a,
        inline_code: 0xe88a63,
        inline_code_background: 0x231f1b,
        date: 0xd4a054,
        keyword: 0xd46a4a,
        string: 0x4fb87a,
        comment: 0x8a837a,
        type_name: 0xc4574a,
        function: 0x5f8fd4,
        constant: 0xd4a054,
        number: 0xd4a054,
        variable: 0xb08a54,
        operator: 0xcfc8bf,
        attribute: 0xe0977a,
    },
    ..WARM_CLAY_STYLE
};

pub(crate) fn preview_style(id: PreviewStyleId) -> &'static PreviewStyle {
    let dark = crate::theme::effective_theme_is_dark();
    match (id, dark) {
        (PreviewStyleId::Base, false) => &BASE_STYLE,
        (PreviewStyleId::Base, true) => &BASE_STYLE_DARK,
        (PreviewStyleId::WarmClay, false) => &WARM_CLAY_STYLE,
        (PreviewStyleId::WarmClay, true) => &WARM_CLAY_STYLE_DARK,
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
        let base = BASE_STYLE;
        let warm = WARM_CLAY_STYLE;
        assert_ne!(base.paint_key(), warm.paint_key());
        assert_ne!(base.layout_key(), warm.layout_key());
    }

    #[test]
    fn style_identity_does_not_hide_layout_rules() {
        let base = BASE_STYLE;
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
        let style = WARM_CLAY_STYLE;
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

    #[test]
    fn dark_variants_swap_the_neutral_ramp_and_keep_readable_contrast() {
        let base = BASE_STYLE_DARK;
        let warm = WARM_CLAY_STYLE_DARK;
        // Identity survives: same typography, spacing, variants and style ids.
        assert_eq!(base.typography, BASE_STYLE.typography);
        assert_eq!(base.spacing, BASE_STYLE.spacing);
        assert_eq!(base.variants, BASE_STYLE.variants);
        assert_eq!(warm.typography, WARM_CLAY_STYLE.typography);
        assert_eq!(warm.spacing, WARM_CLAY_STYLE.spacing);
        // Dark canvases must actually be dark.
        assert_eq!(base.palette.background, 0x121d28);
        assert_eq!(warm.palette.background, 0x151210);
        // Text and accent contrast stay readable on the dark ramp.
        for style in [base, warm] {
            assert!(contrast(style.palette.foreground, style.palette.background) >= 7.0);
            assert!(contrast(style.palette.accent_text, style.palette.background) >= 4.5);
            assert!(contrast(style.palette.accent_contrast, style.palette.accent) >= 4.5);
            assert!(contrast(style.palette.foreground, style.palette.code_background) >= 7.0);
        }
    }

    #[test]
    fn preview_style_dispatch_follows_the_effective_theme() {
        let _guard = crate::theme::THEME_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        crate::theme::set_theme_mode(crate::theme::ThemeMode::Auto);
        crate::theme::set_system_dark(false);
        assert_eq!(
            preview_style(PreviewStyleId::Base).palette.background,
            0xffffff
        );
        crate::theme::set_system_dark(true);
        assert_eq!(
            preview_style(PreviewStyleId::Base).palette.background,
            0x121d28
        );
        assert_eq!(
            preview_style(PreviewStyleId::WarmClay).palette.background,
            0x151210
        );
        crate::theme::set_system_dark(false);
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
