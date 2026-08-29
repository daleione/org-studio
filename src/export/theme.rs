use crate::i18n::Language;

pub struct Theme {
    pub id: &'static str,
    pub name_en: &'static str,
    pub name_zh: &'static str,
    pub family_en: &'static str,
    pub family_zh: &'static str,
    pub appearance: ThemeAppearance,
    pub source: &'static str,
    pub thumbnail: &'static [u8],
}

impl Theme {
    pub fn name(&self, language: Language) -> &'static str {
        match language {
            Language::English => self.name_en,
            Language::Chinese => self.name_zh,
        }
    }

    pub fn family(&self, language: Language) -> &'static str {
        match language {
            Language::English => self.family_en,
            Language::Chinese => self.family_zh,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeAppearance {
    Light,
    Dark,
}

macro_rules! theme {
    ($id:literal, $name_en:literal, $name_zh:literal, $family_en:literal, $family_zh:literal, $appearance:ident) => {
        Theme {
            id: $id,
            name_en: $name_en,
            name_zh: $name_zh,
            family_en: $family_en,
            family_zh: $family_zh,
            appearance: ThemeAppearance::$appearance,
            source: include_str!(concat!("../../assets/export/themes/", $id, ".typ")),
            thumbnail: include_bytes!(concat!("../../assets/export/thumbnails/", $id, ".png")),
        }
    };
}

static THEMES: &[Theme] = &[
    theme!(
        "minimal-blue",
        "Minimal Blue",
        "简约蓝",
        "Minimal",
        "简约文档系",
        Light
    ),
    theme!(
        "lavender-dream",
        "Lavender Dream",
        "薰衣草梦",
        "Nature & Arts",
        "自然文艺系",
        Light
    ),
    theme!(
        "pixel-terminal",
        "Pixel Terminal",
        "像素终端",
        "Geek",
        "极客系",
        Dark
    ),
    theme!(
        "dedao-light",
        "Brand Light",
        "白色主题长图",
        "Brand Longform",
        "品牌长图系",
        Light
    ),
    theme!(
        "dedao-dark",
        "Brand Dark",
        "暗黑主题长图",
        "Brand Longform",
        "品牌长图系",
        Dark
    ),
    theme!(
        "clash-collage",
        "Clash Collage",
        "撞色拼贴",
        "Art Collage",
        "艺术拼贴系",
        Light
    ),
    theme!(
        "matisse-cutout",
        "Matisse Cutout",
        "马蒂斯剪纸",
        "Art Collage",
        "艺术拼贴系",
        Light
    ),
    theme!(
        "ember-glow",
        "Ember Glow",
        "余烬暖焰",
        "Dark",
        "暗调系",
        Dark
    ),
    theme!(
        "rational-grid",
        "Rational Grid",
        "理性格栅",
        "Art Collage",
        "艺术拼贴系",
        Light
    ),
    theme!("high-volt", "High Volt", "高压伏特", "Dark", "暗调系", Dark),
    theme!(
        "terra-nature",
        "Terra Nature",
        "大地自然",
        "Nature & Arts",
        "自然文艺系",
        Light
    ),
    theme!(
        "classified-brief",
        "Classified Brief",
        "机要简报",
        "Editorial",
        "编辑杂志系",
        Light
    ),
    theme!(
        "letterpress",
        "Letterpress",
        "铅字打字机",
        "Editorial",
        "编辑杂志系",
        Light
    ),
    theme!(
        "hockney-pool",
        "Hockney Pool",
        "霍克尼泳池",
        "Art Collage",
        "艺术拼贴系",
        Light
    ),
    theme!(
        "glacier-glass",
        "Glacier Glass",
        "冰川玻璃",
        "Dark",
        "暗调系",
        Dark
    ),
    theme!(
        "pin-waterfall",
        "Pin Waterfall",
        "拼趣瀑布",
        "Art Collage",
        "艺术拼贴系",
        Light
    ),
    theme!(
        "warm-editorial",
        "Warm Editorial",
        "暖调编辑",
        "Editorial",
        "编辑杂志系",
        Light
    ),
    theme!(
        "editorial-magazine",
        "Editorial Magazine",
        "编辑杂志",
        "Editorial",
        "编辑杂志系",
        Light
    ),
    theme!(
        "orange-journal",
        "Orange Journal",
        "暖橘手帐",
        "Nature & Arts",
        "自然文艺系",
        Light
    ),
    theme!(
        "bold-blue",
        "Bold Blue",
        "醒目蓝",
        "Minimal",
        "简约文档系",
        Light
    ),
    theme!(
        "github-style",
        "GitHub Style",
        "GitHub 风",
        "Minimal",
        "简约文档系",
        Light
    ),
    theme!(
        "bauhaus",
        "Bauhaus",
        "包豪斯",
        "Art Collage",
        "艺术拼贴系",
        Light
    ),
    theme!(
        "chinoiserie",
        "Chinoiserie",
        "中国风",
        "Nature & Arts",
        "自然文艺系",
        Light
    ),
    theme!(
        "ink-rhyme",
        "Ink Rhyme",
        "墨韵",
        "Nature & Arts",
        "自然文艺系",
        Light
    ),
    theme!(
        "sunset-orange",
        "Sunset Orange",
        "日落暖橙",
        "Nature & Arts",
        "自然文艺系",
        Light
    ),
];

pub fn themes() -> &'static [Theme] {
    THEMES
}

pub(super) fn theme(id: &str) -> Option<&'static Theme> {
    THEMES.iter().find(|theme| theme.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_all_copied_themes() {
        assert_eq!(themes().len(), 25);
        for theme in themes() {
            assert!(
                theme.source.contains("#let conf"),
                "{} has no conf",
                theme.id
            );
        }
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../assets/export/themes/manifest.json")).unwrap();
        let entries = manifest["themes"].as_array().unwrap();
        assert_eq!(entries.len(), themes().len());
        for theme in themes() {
            assert!(entries.iter().any(|entry| entry["id"] == theme.id));
        }
    }
}
