use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum DocumentFormat {
    Org,
    Markdown,
}

impl DocumentFormat {
    /// Format used by the document renderer. Unknown extensions retain the historical Org
    /// fallback so extensionless documents continue to render.
    pub(crate) fn from_path(path: &Path) -> Self {
        Self::detect(path).unwrap_or(Self::Org)
    }

    /// Strict format detection for format-specific editor commands.
    pub(crate) fn detect(path: &Path) -> Option<Self> {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("org") => Some(Self::Org),
            Some("md" | "markdown") => Some(Self::Markdown),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_editor_formats_without_changing_renderer_fallback() {
        assert_eq!(
            DocumentFormat::detect(Path::new("note.ORG")),
            Some(DocumentFormat::Org)
        );
        assert_eq!(
            DocumentFormat::detect(Path::new("note.Markdown")),
            Some(DocumentFormat::Markdown)
        );
        assert_eq!(DocumentFormat::detect(Path::new("note.txt")), None);
        assert_eq!(
            DocumentFormat::from_path(Path::new("note.txt")),
            DocumentFormat::Org
        );
    }
}
