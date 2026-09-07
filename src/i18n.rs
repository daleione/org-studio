use std::{collections::HashMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Language {
    English,
    Chinese,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agenda_translations_have_matching_keys_and_placeholders() {
        let english: HashMap<String, String> =
            serde_json::from_str(include_str!("../assets/i18n/en.json")).unwrap();
        let chinese: HashMap<String, String> =
            serde_json::from_str(include_str!("../assets/i18n/zh-CN.json")).unwrap();
        let keys = |messages: &HashMap<String, String>| {
            messages
                .keys()
                .filter(|key| key.starts_with("agenda."))
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(keys(&english), keys(&chinese));
        let placeholders = |text: &str| {
            text.split('{')
                .skip(1)
                .filter_map(|part| part.split_once('}').map(|(name, _)| name.to_owned()))
                .collect::<std::collections::BTreeSet<_>>()
        };
        for key in keys(&english) {
            assert!(!english[&key].is_empty());
            assert!(!chinese[&key].is_empty());
            assert_eq!(
                placeholders(&english[&key]),
                placeholders(&chinese[&key]),
                "{key}"
            );
        }
        assert_eq!(
            Language::English.text("agenda.search"),
            "Search current view"
        );
        assert_eq!(Language::Chinese.text("agenda.search"), "搜索当前视图");
    }
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
