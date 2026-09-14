//! Validation shared by tag editing and semantic consumers.
pub(crate) fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '@' | '#' | '%'))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tag_names_allow_unicode_but_exclude_cluster_delimiters() {
        assert!(valid_tag("中文_%@#"));
        assert!(!valid_tag("work:home"));
        assert!(!valid_tag(""));
        assert!(!valid_tag("two words"));
    }
}
