use std::{
    collections::hash_map::DefaultHasher,
    ffi::OsString,
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{EntryId, EntryKind, EntryMetadata, FileEntry, FileResourceId};
use crate::navigation::CancellationToken;

pub struct ScanResult {
    pub directory: PathBuf,
    pub entries: Vec<FileEntry>,
}

#[derive(Debug)]
pub struct ScanError {
    pub directory: PathBuf,
    pub source: io::Error,
}

pub fn scan_directory(directory: PathBuf) -> Result<ScanResult, ScanError> {
    scan_directory_cancellable(directory, &CancellationToken::new())
}

pub fn scan_directory_cancellable(
    directory: PathBuf,
    cancellation: &CancellationToken,
) -> Result<ScanResult, ScanError> {
    let read_dir = fs::read_dir(&directory).map_err(|source| ScanError {
        directory: directory.clone(),
        source,
    })?;
    let mut entries = Vec::new();
    if let Some(parent) = directory.parent() {
        entries.push(parent_entry(parent));
    }
    for item in read_dir {
        if cancellation.is_cancelled() {
            return Err(ScanError {
                directory,
                source: io::Error::new(io::ErrorKind::Interrupted, "directory scan cancelled"),
            });
        }
        let item = item.map_err(|source| ScanError {
            directory: directory.clone(),
            source,
        })?;
        let path = item.path();
        let os_name = item.file_name();
        let metadata = fs::symlink_metadata(&path).ok();
        let file_type = metadata.as_ref().map(|metadata| metadata.file_type());
        let kind = classify(&path, file_type);
        let hidden = os_name.to_string_lossy().starts_with('.');
        let symlink_target = if matches!(kind, EntryKind::Symlink) {
            fs::read_link(&path).ok()
        } else {
            None
        };
        entries.push(FileEntry {
            id: entry_id(&path),
            resource_id: resource_id(metadata.as_ref()),
            path: Arc::from(path),
            display_name: Arc::from(os_name.to_string_lossy().as_ref()),
            os_name: Arc::new(os_name),
            kind,
            metadata: EntryMetadata {
                byte_len: metadata
                    .as_ref()
                    .filter(|_| !matches!(kind, EntryKind::Directory))
                    .map(|value| value.len()),
                modified: metadata.as_ref().and_then(|value| value.modified().ok()),
                hidden,
                symlink_target,
            },
        });
    }
    Ok(ScanResult { directory, entries })
}

fn parent_entry(parent: &Path) -> FileEntry {
    FileEntry {
        id: entry_id(parent),
        resource_id: fs::symlink_metadata(parent)
            .ok()
            .as_ref()
            .and_then(|metadata| resource_id(Some(metadata))),
        path: Arc::from(parent),
        os_name: Arc::new(OsString::from("..")),
        display_name: Arc::from(".."),
        kind: EntryKind::Parent,
        metadata: EntryMetadata {
            byte_len: None,
            modified: None,
            hidden: false,
            symlink_target: None,
        },
    }
}

fn classify(path: &Path, file_type: Option<fs::FileType>) -> EntryKind {
    if file_type.is_some_and(|kind| kind.is_symlink()) {
        return EntryKind::Symlink;
    }
    if file_type.is_some_and(|kind| kind.is_dir()) {
        return EntryKind::Directory;
    }
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("org") => EntryKind::OrgFile,
        Some("md" | "markdown") => EntryKind::Markdown,
        Some("png" | "jpg" | "jpeg" | "gif" | "webp") => EntryKind::Image,
        _ if crate::syntax_highlighting::language_for_path(path).is_some() => EntryKind::CodeFile,
        _ => EntryKind::RegularFile,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_classifies_supported_source_files() {
        for path in ["main.rs", "main.go", "main.py"] {
            assert_eq!(classify(Path::new(path), None), EntryKind::CodeFile);
        }
        assert_eq!(classify(Path::new("notes.org"), None), EntryKind::OrgFile);
        assert_eq!(classify(Path::new("image.png"), None), EntryKind::Image);
    }
}

fn entry_id(path: &Path) -> EntryId {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    EntryId(hasher.finish())
}

fn resource_id(metadata: Option<&fs::Metadata>) -> Option<FileResourceId> {
    #[cfg(unix)]
    if let Some(metadata) = metadata {
        let mut hasher = DefaultHasher::new();
        use std::os::unix::fs::MetadataExt;
        metadata.dev().hash(&mut hasher);
        metadata.ino().hash(&mut hasher);
        return Some(FileResourceId(hasher.finish()));
    }
    None
}
