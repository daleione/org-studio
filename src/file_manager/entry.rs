use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EntryId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryKind {
    Parent,
    Directory,
    OrgFile,
    Markdown,
    Image,
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
    pub path: Arc<Path>,
    pub os_name: Arc<OsString>,
    pub display_name: Arc<str>,
    pub kind: EntryKind,
    pub metadata: EntryMetadata,
}

impl FileEntry {
    pub fn is_directory(&self) -> bool {
        matches!(self.kind, EntryKind::Parent | EntryKind::Directory)
    }
}
