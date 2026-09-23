use std::path::{Path, PathBuf};

/// Resolve a local link against its source document, retaining its navigation target.
pub(crate) fn resolve_file_link(base: &Path, raw: &str) -> Option<(PathBuf, Option<String>)> {
    let target = raw.strip_prefix("attachment:").unwrap_or(raw);
    let (target, anchor) = split_link_target(target);
    let path = if target.starts_with("file://") {
        url::Url::parse(target).ok()?.to_file_path().ok()?
    } else {
        let target = target.strip_prefix("file:").unwrap_or(target);
        let target = if let Some(rest) = target.strip_prefix("~/") {
            PathBuf::from(std::env::var_os("HOME")?).join(rest)
        } else {
            PathBuf::from(target)
        };
        let base = std::path::absolute(base).ok()?;
        url::Url::from_file_path(base)
            .ok()?
            .join(target.to_str()?)
            .ok()?
            .to_file_path()
            .ok()?
    };
    let anchor = anchor.map(|anchor| {
        // URL parsing exposes a fragment in its encoded form; decode UTF-8 destinations.
        url::form_urlencoded::parse(
            format!("anchor={}", anchor.replace('+', "%2B").replace('&', "%26")).as_bytes(),
        )
        .next()
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default()
    });
    Some((path, anchor))
}

pub(crate) fn split_link_target(destination: &str) -> (&str, Option<&str>) {
    if let Some((path, search)) = destination.split_once("::") {
        return (path, Some(search));
    }
    if destination.starts_with('#') {
        return (destination, None);
    }
    destination
        .split_once('#')
        .map_or((destination, None), |(path, anchor)| (path, Some(anchor)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editable_file_links_open_in_app_but_binary_links_do_not() {
        let directory =
            std::env::temp_dir().join(format!("org-studio-file-link-kind-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for name in [
            "notes.org",
            "sample.rs",
            "notes.TXT",
            "config.toml",
            "photo.png",
        ] {
            std::fs::write(directory.join(name), "contents").unwrap();
        }
        for name in ["notes.org", "sample.rs", "notes.TXT", "config.toml"] {
            assert!(
                crate::preview::is_supported_document(&directory.join(name)),
                "{name}"
            );
        }
        assert!(!crate::preview::is_supported_document(
            &directory.join("photo.png")
        ));
        assert!(!crate::preview::is_supported_document(
            &directory.join("missing.txt")
        ));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn local_documents_keep_paths_and_navigation_targets() {
        let base = Path::new("/tmp/notes/main.org");
        for (raw, expected, anchor) in [
            (
                "file:other.org::*Heading",
                "/tmp/notes/other.org",
                Some("*Heading"),
            ),
            ("../other.md#section", "/tmp/other.md", Some("section")),
            (
                "file:///tmp/a%20b.MD#%E4%B8%AD%E6%96%87",
                "/tmp/a b.MD",
                Some("中文"),
            ),
            (
                "attachment:other.markdown",
                "/tmp/notes/other.markdown",
                None,
            ),
            ("a%23b.md#C++", "/tmp/notes/a#b.md", Some("C++")),
            ("other.md#A&B", "/tmp/notes/other.md", Some("A&B")),
        ] {
            assert_eq!(
                resolve_file_link(base, raw),
                Some((PathBuf::from(expected), anchor.map(str::to_owned)))
            );
        }
        assert!(resolve_file_link(base, "https://example.com/a.md").is_none());
    }
}
