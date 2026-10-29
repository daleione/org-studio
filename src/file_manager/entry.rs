use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EntryId(pub u64);

impl EntryId {
    pub fn for_path(path: &Path) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut hasher);
        Self(hasher.finish())
    }
}

/// Display paths distinguish hard links and aliases. Resource identity is a
/// rename fallback only when it identifies one displayed entry unambiguously.
pub(super) struct EntryLookup<'a> {
    paths: HashMap<&'a Path, &'a FileEntry>,
    resources: HashMap<FileResourceId, Option<&'a FileEntry>>,
}

impl<'a> EntryLookup<'a> {
    pub fn new(entries: impl IntoIterator<Item = &'a FileEntry>) -> Self {
        let mut paths = HashMap::new();
        let mut resources = HashMap::new();
        for entry in entries {
            paths.insert(entry.path.as_ref(), entry);
            if let Some(resource) = entry.resource_id {
                resources
                    .entry(resource)
                    .and_modify(|entry| *entry = None)
                    .or_insert(Some(entry));
            }
        }
        Self { paths, resources }
    }

    pub fn path(&self, path: &Path) -> Option<&'a FileEntry> {
        self.paths.get(path).copied()
    }

    pub fn resource(&self, resource: FileResourceId) -> Option<&'a FileEntry> {
        self.resources.get(&resource).copied().flatten()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileResourceId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryKind {
    Parent,
    Directory,
    OrgFile,
    Markdown,
    CodeFile,
    Text,
    Configuration,
    Data,
    Pdf,
    Image,
    Audio,
    Video,
    Archive,
    RegularFile,
    Symlink,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mark {
    Selected,
    Delete,
}

#[derive(Clone, Debug)]
pub struct EntryMetadata {
    pub byte_len: Option<u64>,
    pub modified: Option<SystemTime>,
    pub hidden: bool,
    pub symlink_target: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub id: EntryId,
    pub resource_id: Option<FileResourceId>,
    pub path: Arc<Path>,
    pub os_name: Arc<OsString>,
    pub display_name: Arc<str>,
    pub kind: EntryKind,
    pub metadata: EntryMetadata,
}

impl FileEntry {
    pub fn directory(path: PathBuf) -> Self {
        let name = path.file_name().unwrap_or(path.as_os_str()).to_os_string();
        Self {
            id: EntryId::for_path(&path),
            resource_id: None,
            os_name: Arc::new(name.clone()),
            display_name: name.to_string_lossy().into_owned().into(),
            path: path.into(),
            kind: EntryKind::Directory,
            metadata: EntryMetadata {
                byte_len: None,
                modified: None,
                hidden: false,
                symlink_target: None,
            },
        }
    }

    pub fn is_directory(&self) -> bool {
        matches!(self.kind, EntryKind::Parent | EntryKind::Directory)
    }
}
