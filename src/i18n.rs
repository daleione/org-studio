use std::{collections::HashMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Language {
    English,
    Chinese,
}

impl Language {
    pub fn system() -> Self {
        let locale = sys_locale::get_locale().unwrap_or_default();
        if locale.to_ascii_lowercase().starts_with("zh") {
            Self::Chinese
        } else {
            Self::English
        }
    }

    pub fn text(self, key: &'static str) -> &'static str {
        fn load(source: &'static str) -> HashMap<String, String> {
            serde_json::from_str(source).expect("embedded localization must be valid JSON")
        }

        static ENGLISH: OnceLock<HashMap<String, String>> = OnceLock::new();
        static CHINESE: OnceLock<HashMap<String, String>> = OnceLock::new();
        let messages = match self {
            Self::English => ENGLISH.get_or_init(|| load(include_str!("../assets/i18n/en.json"))),
            Self::Chinese => {
                CHINESE.get_or_init(|| load(include_str!("../assets/i18n/zh-CN.json")))
            }
        };
        messages.get(key).map(String::as_str).unwrap_or(key)
    }
}
