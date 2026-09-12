use crate::{i18n::Language, search::ReplacementError};

#[derive(Default, Debug, PartialEq, Eq)]
pub(super) enum Notice {
    #[default]
    None,
    Scanning,
    NotFound,
    Boundary,
    WrapBoundary,
    ReplaceDone,
    Truncated,
    DocumentChanged,
    ScopeInvalid,
    NeedsEditor,
    Incomplete,
    Planning(usize),
    PlanError(ReplacementError),
    EditError(String),
}
impl Notice {
    pub fn detail(&self, language: Language) -> Option<String> {
        let key = match self {
            Self::None | Self::Scanning | Self::NotFound => return None,
            Self::Boundary => "search.boundary",
            Self::WrapBoundary => "search.wrap_boundary",
            Self::ReplaceDone => "search.replace_done",
            Self::Truncated => "search.truncated",
            Self::DocumentChanged => "search.document_changed",
            Self::ScopeInvalid => "search.scope_invalid",
            Self::NeedsEditor => "search.needs_editor",
            Self::Incomplete => "search.incomplete",
            Self::Planning(count) => {
                return Some(
                    language
                        .text("search.planning")
                        .replace("{count}", &count.to_string()),
                );
            }
            Self::PlanError(error) => match error {
                ReplacementError::PlanTooLarge => "search.plan_too_large",
                ReplacementError::InvalidRange => "search.invalid_range",
                ReplacementError::StaleMatch => "search.stale_match",
                ReplacementError::DocumentTooLarge => "search.document_too_large",
            },
            Self::EditError(error) => {
                return Some(
                    language
                        .text("search.edit_failed")
                        .replace("{error}", error),
                );
            }
        };
        Some(language.text(key).to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_notices_are_localized_at_display_time() {
        let notice = Notice::Planning(42);
        assert_eq!(
            notice.detail(Language::English).unwrap(),
            "Preparing to replace 42 matches…"
        );
        assert_eq!(
            notice.detail(Language::Chinese).unwrap(),
            "正在准备替换 42 处…"
        );
        for error in [
            ReplacementError::PlanTooLarge,
            ReplacementError::InvalidRange,
            ReplacementError::StaleMatch,
            ReplacementError::DocumentTooLarge,
        ] {
            let notice = Notice::PlanError(error);
            let en = notice.detail(Language::English).unwrap();
            let zh = notice.detail(Language::Chinese).unwrap();
            assert_ne!(en, zh);
            assert!(!en.starts_with("search."));
            assert!(!zh.starts_with("search."));
        }
        assert!(Notice::Scanning.detail(Language::English).is_none());
        assert!(Notice::NotFound.detail(Language::Chinese).is_none());
    }
}
