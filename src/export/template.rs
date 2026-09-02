use crate::i18n::Language;

pub struct ExportTemplate {
    pub id: &'static str,
    pub name_en: &'static str,
    pub name_zh: &'static str,
    pub family_en: &'static str,
    pub family_zh: &'static str,
    pub appearance: TemplateAppearance,
    pub source: &'static str,
    pub thumbnail: &'static [u8],
}

impl ExportTemplate {
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
pub enum TemplateAppearance {
    Light,
    Dark,
}

macro_rules! export_template {
    ($id:literal, $name_en:literal, $name_zh:literal, $family_en:literal, $family_zh:literal, $appearance:ident) => {
        ExportTemplate {
            id: $id,
            name_en: $name_en,
            name_zh: $name_zh,
            family_en: $family_en,
            family_zh: $family_zh,
            appearance: TemplateAppearance::$appearance,
            source: include_str!(concat!("../../assets/export/themes/", $id, ".typ")),
            thumbnail: include_bytes!(concat!("../../assets/export/thumbnails/", $id, ".png")),
        }
    };
}

static EXPORT_TEMPLATES: &[ExportTemplate] = &[
    export_template!(
        "minimal-blue",
        "Minimal Blue",
        "简约蓝",
        "Minimal",
        "简约文档系",
        Light
    ),
    export_template!(
        "lavender-dream",
        "Lavender Dream",
        "薰衣草梦",
        "Nature & Arts",
        "自然文艺系",
        Light
    ),
    export_template!(
        "pixel-terminal",
        "Pixel Terminal",
        "像素终端",
        "Geek",
        "极客系",
        Dark
    ),
    export_template!(
        "dedao-light",
        "Brand Light",
        "白色主题长图",
        "Brand Longform",
        "品牌长图系",
        Light
    ),
    export_template!(
        "dedao-dark",
        "Brand Dark",
        "暗黑主题长图",
        "Brand Longform",
        "品牌长图系",
        Dark
    ),
    export_template!(
        "clash-collage",
        "Clash Collage",
        "撞色拼贴",
        "Art Collage",
        "艺术拼贴系",
        Light
    ),
    export_template!(
        "matisse-cutout",
        "Matisse Cutout",
        "马蒂斯剪纸",
        "Art Collage",
        "艺术拼贴系",
        Light
    ),
    export_template!(
        "ember-glow",
        "Ember Glow",
        "余烬暖焰",
        "Dark",
        "暗调系",
        Dark
    ),
    export_template!(
        "rational-grid",
        "Rational Grid",
        "理性格栅",
        "Art Collage",
        "艺术拼贴系",
        Light
    ),
    export_template!("high-volt", "High Volt", "高压伏特", "Dark", "暗调系", Dark),
    export_template!(
        "terra-nature",
        "Terra Nature",
        "大地自然",
        "Nature & Arts",
        "自然文艺系",
        Light
    ),
    export_template!(
        "classified-brief",
        "Classified Brief",
        "机要简报",
        "Editorial",
        "编辑杂志系",
        Light
    ),
    export_template!(
        "letterpress",
        "Letterpress",
        "铅字打字机",
        "Editorial",
        "编辑杂志系",
        Light
    ),
    export_template!(
        "hockney-pool",
        "Hockney Pool",
        "霍克尼泳池",
        "Art Collage",
        "艺术拼贴系",
        Light
    ),
    export_template!(
        "glacier-glass",
        "Glacier Glass",
        "冰川玻璃",
        "Dark",
        "暗调系",
        Dark
    ),
    export_template!(
        "pin-waterfall",
        "Pin Waterfall",
        "拼趣瀑布",
        "Art Collage",
        "艺术拼贴系",
        Light
    ),
    export_template!(
        "warm-editorial",
        "Warm Editorial",
        "暖调编辑",
        "Editorial",
        "编辑杂志系",
        Light
    ),
    export_template!(
        "editorial-magazine",
        "Editorial Magazine",
        "编辑杂志",
        "Editorial",
        "编辑杂志系",
        Light
    ),
    export_template!(
        "orange-journal",
        "Orange Journal",
        "暖橘手帐",
        "Nature & Arts",
        "自然文艺系",
        Light
    ),
    export_template!(
        "bold-blue",
        "Bold Blue",
        "醒目蓝",
        "Minimal",
        "简约文档系",
        Light
    ),
    export_template!(
        "github-style",
        "GitHub Style",
        "GitHub 风",
        "Minimal",
        "简约文档系",
        Light
    ),
    export_template!(
        "bauhaus",
        "Bauhaus",
        "包豪斯",
        "Art Collage",
        "艺术拼贴系",
        Light
    ),
    export_template!(
        "chinoiserie",
        "Chinoiserie",
        "中国风",
        "Nature & Arts",
        "自然文艺系",
        Light
    ),
    export_template!(
        "ink-rhyme",
        "Ink Rhyme",
        "墨韵",
        "Nature & Arts",
        "自然文艺系",
        Light
    ),
    export_template!(
        "sunset-orange",
        "Sunset Orange",
        "日落暖橙",
        "Nature & Arts",
        "自然文艺系",
        Light
    ),
];

pub fn export_templates() -> &'static [ExportTemplate] {
    EXPORT_TEMPLATES
}

pub(super) fn export_template(id: &str) -> Option<&'static ExportTemplate> {
    EXPORT_TEMPLATES.iter().find(|template| template.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_all_copied_export_templates() {
        assert_eq!(export_templates().len(), 25);
        for template in export_templates() {
            assert!(
                template.source.contains("#let conf"),
                "{} has no conf",
                template.id
            );
        }
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../assets/export/themes/manifest.json")).unwrap();
        let entries = manifest["themes"].as_array().unwrap();
        assert_eq!(entries.len(), export_templates().len());
        for template in export_templates() {
            assert!(entries.iter().any(|entry| entry["id"] == template.id));
        }
    }
}
