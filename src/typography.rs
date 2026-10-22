use std::sync::OnceLock;

mod settings;
mod text;

pub use settings::FontSettings;
pub(crate) use text::{
    FontRole, apply_font_runs, cjk_ranges, cjk_runs, font_overrides, role_text, styled_text,
    styled_text_with_overrides,
};

const SYSTEM_FONT_FAMILY: &str = ".SystemUIFont";

/// Font names live for the application lifetime. Reading styles can borrow them without leaking
/// strings or allocating per frame. Configuration edits take effect on the next launch.
pub(crate) struct ResolvedFonts {
    pub(crate) settings: FontSettings,
    pub(crate) editor_family: String,
    pub(crate) editor_cjk_family: Option<String>,
    pub(crate) table_family: String,
    pub(crate) reading_family: Option<String>,
    pub(crate) reading_cjk_family: Option<String>,
    pub(crate) code_family: Option<String>,
    pub(crate) code_cjk_family: Option<String>,
    pub(crate) ui_family: Option<String>,
    pub(crate) ui_cjk_family: Option<String>,
    pub(crate) fallbacks: Vec<String>,
    pub(crate) warnings: Vec<String>,
}

impl ResolvedFonts {
    fn resolve(settings: FontSettings, available: &[String]) -> Self {
        let find = |name: &str| {
            // GPUI's virtual system family need not appear in the installed-font list.
            let name = normalize_family(name);
            if name == SYSTEM_FONT_FAMILY {
                return Some(name.to_owned());
            }
            available
                .iter()
                .find(|family| family.eq_ignore_ascii_case(name))
                .cloned()
        };
        let mut warnings = Vec::new();
        let mut configured = |key: &str, name: &Option<String>| {
            name.as_ref().and_then(|name| {
                let found = find(name);
                if found.is_none() {
                    warnings.push(format!(
                        "{key}: '{name}' is unavailable; using the default font"
                    ));
                }
                found
            })
        };
        let editor_family = configured("editor_font_family", &settings.editor_font_family)
            .or_else(|| {
                ["JetBrains Mono", "Menlo", ".SF NS Mono", ".SystemUIFont"]
                    .into_iter()
                    .find_map(find)
            })
            .unwrap_or_else(|| SYSTEM_FONT_FAMILY.into());
        let table_family = configured("table_font_family", &settings.table_font_family)
            .or_else(|| find(FontSettings::DEFAULT_TABLE_FONT_FAMILY))
            .unwrap_or_else(|| editor_family.clone());
        let reading_family = configured("reading_font_family", &settings.reading_font_family);
        let editor_cjk_family =
            configured("editor_cjk_font_family", &settings.editor_cjk_font_family);
        let reading_cjk_family =
            configured("reading_cjk_font_family", &settings.reading_cjk_font_family);
        let code_family = configured("code_font_family", &settings.code_font_family);
        let code_cjk_family = configured("code_cjk_font_family", &settings.code_cjk_font_family);
        let ui_family = configured("ui_font_family", &settings.ui_font_family);
        let ui_cjk_family = configured("ui_cjk_font_family", &settings.ui_cjk_font_family);
        let mut fallbacks = Vec::new();
        for name in &settings.font_fallbacks {
            if let Some(family) = find(name) {
                append_fallback(&mut fallbacks, &family);
            } else {
                warnings.push(format!(
                    "font_fallbacks: '{name}' is unavailable; skipping it"
                ));
            }
        }
        Self {
            settings,
            editor_family,
            editor_cjk_family,
            table_family,
            reading_family,
            reading_cjk_family,
            code_family,
            code_cjk_family,
            ui_family,
            ui_cjk_family,
            fallbacks,
            warnings,
        }
    }

    pub(crate) fn editor_size(&self, zoom: ContentFontSize) -> f32 {
        self.settings.editor_font_size as f32 * zoom.scale()
    }

    pub(crate) fn ui_size(&self, default: f32) -> f32 {
        default * self.settings.ui_font_size as f32 / FontSettings::DEFAULT_UI_SIZE as f32
    }
}

static FONTS: OnceLock<ResolvedFonts> = OnceLock::new();

/// Initialize startup preferences before constructing editors or reading views.
/// Preferences are immutable until the next application launch.
pub fn init(settings: FontSettings, text_system: &gpui::TextSystem) {
    let fonts = ResolvedFonts::resolve(settings, &text_system.all_font_names());
    for warning in &fonts.warnings {
        eprintln!("org_studio_font_fallback {warning}");
    }
    assert!(
        FONTS.set(fonts).is_ok(),
        "font preferences must be initialized before rendering views"
    );
}

pub(crate) fn fonts() -> &'static ResolvedFonts {
    // Library users and tests keep the existing typography without scanning system fonts.
    FONTS.get_or_init(|| ResolvedFonts {
        settings: FontSettings::default(),
        editor_family: "JetBrains Mono".into(),
        editor_cjk_family: None,
        table_family: FontSettings::DEFAULT_TABLE_FONT_FAMILY.into(),
        reading_family: None,
        reading_cjk_family: None,
        code_family: None,
        code_cjk_family: None,
        ui_family: None,
        ui_cjk_family: None,
        fallbacks: Vec::new(),
        warnings: Vec::new(),
    })
}

pub(crate) fn editor_font() -> gpui::Font {
    font_with_fallbacks(
        &fonts().editor_family,
        &["Menlo", "PingFang SC", "Apple Color Emoji"],
    )
}

pub(crate) fn table_font() -> gpui::Font {
    let mut font = font_with_fallbacks(
        &fonts().table_family,
        &["Menlo", "PingFang SC", "Apple Color Emoji"],
    );
    font.features = gpui::FontFeatures::disable_ligatures();
    font
}

pub(crate) fn font_with_fallbacks(family: &str, defaults: &[&str]) -> gpui::Font {
    let mut font = gpui::font(normalize_family(family).to_owned());
    let mut names = fonts().fallbacks.clone();
    // Lazy StyledText overrides inherit this cascade when selecting a Chinese family.
    if [
        FontRole::Editor,
        FontRole::Reading,
        FontRole::Code,
        FontRole::Ui,
    ]
    .iter()
    .any(|role| role.cjk_family().is_some())
    {
        names.insert(0, normalize_family(family).to_owned());
    }
    for name in defaults {
        append_fallback(&mut names, name);
    }
    font.fallbacks = Some(gpui::FontFallbacks::from_fonts(names));
    font
}

pub(crate) fn ui_font(default: &str) -> gpui::Font {
    font_with_fallbacks(ui_family(default), &["PingFang SC", "Apple Color Emoji"])
}

pub(crate) fn ui_family(default: &str) -> &str {
    fonts().ui_family.as_deref().unwrap_or(default)
}

pub(crate) fn ui_px(default: f32) -> gpui::Pixels {
    gpui::px(ui_size(default))
}

pub(crate) fn ui_size(default: f32) -> f32 {
    fonts().ui_size(default)
}

pub(crate) fn cosmic_family(family: &'static str) -> cosmic_text::Family<'static> {
    match normalize_family(family) {
        SYSTEM_FONT_FAMILY => cosmic_text::Family::SansSerif,
        family => cosmic_text::Family::Name(family),
    }
}

fn normalize_family(name: &str) -> &str {
    if ["System Font", "system-ui", SYSTEM_FONT_FAMILY]
        .iter()
        .any(|alias| name.eq_ignore_ascii_case(alias))
    {
        SYSTEM_FONT_FAMILY
    } else {
        name
    }
}

fn append_fallback(families: &mut Vec<String>, name: &str) {
    let name = normalize_family(name);
    if !families
        .iter()
        .any(|family| family.eq_ignore_ascii_case(name))
    {
        families.push(name.to_owned());
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentFontSize(u16);

impl ContentFontSize {
    pub const MIN: u16 = FontSettings::MIN_SIZE;
    pub const DEFAULT: u16 = FontSettings::DEFAULT_EDITOR_SIZE;
    pub const MAX: u16 = FontSettings::MAX_SIZE;

    pub const fn new(value: u16) -> Self {
        if value < Self::MIN {
            Self(Self::MIN)
        } else if value > Self::MAX {
            Self(Self::MAX)
        } else {
            Self(value)
        }
    }

    pub const fn get(self) -> u16 {
        self.0
    }

    pub fn scale(self) -> f32 {
        self.0 as f32 / Self::DEFAULT as f32
    }

    pub const fn increase(self) -> Self {
        Self::new(self.0.saturating_add(1))
    }

    pub const fn decrease(self) -> Self {
        Self::new(self.0.saturating_sub(1))
    }

    pub const fn reset() -> Self {
        Self(Self::DEFAULT)
    }
}

impl Default for ContentFontSize {
    fn default() -> Self {
        Self::reset()
    }
}

#[cfg(test)]
mod tests {
    use super::{ContentFontSize, FontSettings, ResolvedFonts};

    #[test]
    fn unavailable_fonts_use_defaults_without_erasing_the_preference() {
        let settings = FontSettings {
            editor_font_family: Some("Missing Font".into()),
            editor_font_size: 21,
            reading_font_family: Some("pingfang sc".into()),
            code_font_family: Some("Missing Code Font".into()),
            ui_font_family: Some("SYSTEM-UI".into()),
            ui_font_size: 28,
            font_fallbacks: vec!["PingFang SC".into(), "Missing Fallback".into()],
            ..FontSettings::default()
        };
        let names = ["Menlo", "PingFang SC"].map(String::from);
        let resolved = ResolvedFonts::resolve(settings.clone(), &names);
        assert_eq!(resolved.editor_family, "Menlo");
        assert_eq!(resolved.reading_family.as_deref(), Some("PingFang SC"));
        assert_eq!(resolved.code_family, None);
        assert_eq!(resolved.ui_family.as_deref(), Some(".SystemUIFont"));
        assert_eq!(resolved.fallbacks, ["PingFang SC"]);
        assert_eq!(resolved.warnings.len(), 3);
        assert_eq!(resolved.settings, settings);
        assert_eq!(resolved.editor_size(ContentFontSize::reset()), 21.0);
        assert_eq!(resolved.editor_size(ContentFontSize::new(30)), 42.0);
        assert_eq!(resolved.ui_size(12.0), 24.0);
    }

    #[test]
    fn content_font_size_clamps_steps_and_resets() {
        assert_eq!(ContentFontSize::new(0).get(), 5);
        assert_eq!(ContentFontSize::new(u16::MAX).get(), 96);
        assert_eq!(ContentFontSize::new(5).decrease().get(), 5);
        assert_eq!(ContentFontSize::new(96).increase().get(), 96);
        assert_eq!(ContentFontSize::new(15).increase().get(), 16);
        assert_eq!(ContentFontSize::new(15).decrease().get(), 14);
        assert_eq!(ContentFontSize::reset().get(), 15);
        assert_eq!(ContentFontSize::reset().scale(), 1.0);
    }

    #[test]
    fn equivalent_fallback_names_resolve_once_and_keep_the_requested_order() {
        let settings = FontSettings {
            font_fallbacks: ["System Font", "system-ui", "Menlo", "MENLO"]
                .map(String::from)
                .into(),
            ..Default::default()
        };
        let resolved = ResolvedFonts::resolve(settings.clone(), &["Menlo".into()]);
        assert_eq!(resolved.fallbacks, [".SystemUIFont", "Menlo"]);
        assert_eq!(resolved.settings, settings);
        assert!(resolved.warnings.is_empty());
    }

    #[test]
    fn table_font_resolves_independently_and_falls_back_when_unavailable() {
        for (requested, installed, expected, warnings) in [
            (None, true, "LXGW WenKai Mono", 0),
            (Some("menlo"), true, "Menlo", 0),
            (Some("Missing Table Font"), true, "LXGW WenKai Mono", 1),
            (None, false, "Paper Mono", 0),
        ] {
            let settings = FontSettings {
                editor_font_family: Some("Paper Mono".into()),
                editor_cjk_font_family: Some("LXGW WenKai".into()),
                table_font_family: requested.map(String::from),
                ..Default::default()
            };
            let mut names = ["Paper Mono", "LXGW WenKai", "Menlo"]
                .map(String::from)
                .to_vec();
            if installed {
                names.push(FontSettings::DEFAULT_TABLE_FONT_FAMILY.into());
            }
            let resolved = ResolvedFonts::resolve(settings.clone(), &names);
            assert_eq!(resolved.table_family, expected);
            assert_eq!(resolved.editor_family, "Paper Mono");
            assert_eq!(resolved.editor_cjk_family.as_deref(), Some("LXGW WenKai"));
            assert_eq!(resolved.warnings.len(), warnings);
            assert_eq!(resolved.settings, settings);
        }
    }
}
