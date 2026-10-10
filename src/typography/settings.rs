/// Persistent defaults. Pane-local zoom is deliberately kept out of these settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontSettings {
    pub editor_font_family: Option<String>,
    pub editor_font_size: u16,
    pub reading_font_family: Option<String>,
    pub reading_font_size: Option<u16>,
    pub code_font_family: Option<String>,
    pub code_font_size: Option<u16>,
    pub ui_font_family: Option<String>,
    pub ui_font_size: u16,
    pub font_fallbacks: Vec<String>,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            editor_font_family: None,
            editor_font_size: Self::DEFAULT_EDITOR_SIZE,
            reading_font_family: None,
            reading_font_size: None,
            code_font_family: None,
            code_font_size: None,
            ui_font_family: None,
            ui_font_size: Self::DEFAULT_UI_SIZE,
            font_fallbacks: Vec::new(),
        }
    }
}

impl FontSettings {
    pub const MIN_SIZE: u16 = 5;
    pub const MAX_SIZE: u16 = 96;
    pub const DEFAULT_EDITOR_SIZE: u16 = 15;
    pub const DEFAULT_UI_SIZE: u16 = 14;

    pub(crate) fn description(key: &str) -> Option<&'static str> {
        Some(match key {
            "editor_font_family" => "Installed font family, or auto (editor default).",
            "editor_font_size" => "Editor base font size, 5..96 logical pixels; default 15.",
            "reading_font_family" => "Installed font family, or auto (reading style default).",
            "reading_font_size" => {
                "Reading body size, auto (style default) or 5..96 logical pixels; headings scale proportionally."
            }
            "code_font_family" => "Installed font family, or auto (reading style code default).",
            "code_font_size" => {
                "Reading code size, auto (style default) or 5..96 logical pixels, including inline code."
            }
            "ui_font_family" => "Installed font family, or auto (interface default).",
            "ui_font_size" => {
                "Interface base font size, 5..96 logical pixels; default 14. Labels scale proportionally."
            }
            "font_fallbacks" => {
                "Comma-separated fallback font families, or auto (platform defaults)."
            }
            _ => return None,
        })
    }

    pub(crate) fn parse_entry(&mut self, key: &str, value: &str) -> bool {
        let value = value.trim();
        match key {
            "editor_font_family" => self.editor_font_family = parse_family(value),
            "editor_font_size" => {
                self.editor_font_size = parse_size(value).unwrap_or(Self::DEFAULT_EDITOR_SIZE)
            }
            "reading_font_family" => self.reading_font_family = parse_family(value),
            "reading_font_size" => self.reading_font_size = parse_size(value),
            "code_font_family" => self.code_font_family = parse_family(value),
            "code_font_size" => self.code_font_size = parse_size(value),
            "ui_font_family" => self.ui_font_family = parse_family(value),
            "ui_font_size" => {
                self.ui_font_size = parse_size(value).unwrap_or(Self::DEFAULT_UI_SIZE)
            }
            "font_fallbacks" => self.font_fallbacks = parse_fallbacks(value),
            _ => return false,
        }
        true
    }

    pub(crate) fn entries(&self) -> [(&'static str, String); 9] {
        let family = |value: &Option<String>| value.clone().unwrap_or_else(|| "auto".into());
        let size =
            |value: Option<u16>| value.map_or_else(|| "auto".into(), |size| size.to_string());
        [
            ("editor_font_family", family(&self.editor_font_family)),
            ("editor_font_size", self.editor_font_size.to_string()),
            ("reading_font_family", family(&self.reading_font_family)),
            ("reading_font_size", size(self.reading_font_size)),
            ("code_font_family", family(&self.code_font_family)),
            ("code_font_size", size(self.code_font_size)),
            ("ui_font_family", family(&self.ui_font_family)),
            ("ui_font_size", self.ui_font_size.to_string()),
            (
                "font_fallbacks",
                if self.font_fallbacks.is_empty() {
                    "auto".into()
                } else {
                    self.font_fallbacks.join(",")
                },
            ),
        ]
    }
}

fn parse_family(value: &str) -> Option<String> {
    (!value.is_empty() && !value.eq_ignore_ascii_case("auto") && !value.contains(['\n', '\r']))
        .then(|| value.to_owned())
}

fn parse_size(value: &str) -> Option<u16> {
    value
        .parse()
        .ok()
        .filter(|size| (FontSettings::MIN_SIZE..=FontSettings::MAX_SIZE).contains(size))
}

fn parse_fallbacks(value: &str) -> Vec<String> {
    let mut families = Vec::<String>::new();
    if value.eq_ignore_ascii_case("auto") {
        return families;
    }
    for name in value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        if !families
            .iter()
            .any(|family| family.eq_ignore_ascii_case(name))
        {
            families.push(name.to_owned());
        }
    }
    families
}
