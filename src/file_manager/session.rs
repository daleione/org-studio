use std::{collections::{HashMap, HashSet}, path::{Path, PathBuf}, sync::Arc};

use super::{EntryId, EntryKind, FileEntry, Mark, ScanResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortDirection { Ascending, Descending }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SortSpec { pub direction: SortDirection, pub directories_first: bool }

impl Default for SortSpec {
    fn default() -> Self { Self { direction: SortDirection::Ascending, directories_first: true } }
}

#[derive(Clone, Debug)]
pub struct OperationPlan {
    pub delete: Arc<[Arc<Path>]>,
}

pub struct DiredSession {
    directory: PathBuf,
    entries: Arc<Vec<FileEntry>>,
    cursor: Option<EntryId>,
    marks: HashMap<EntryId, Mark>,
    sort: SortSpec,
    show_hidden: bool,
    generation: u64,
}

impl DiredSession {
    pub fn empty(directory: PathBuf) -> Self {
        Self { directory, entries: Arc::new(Vec::new()), cursor: None, marks: HashMap::new(), sort: SortSpec::default(), show_hidden: true, generation: 0 }
    }

    pub fn directory(&self) -> &Path { &self.directory }
    pub fn entries(&self) -> &Arc<Vec<FileEntry>> { &self.entries }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn cursor(&self) -> Option<EntryId> { self.cursor }
    pub fn mark(&self, id: EntryId) -> Option<Mark> { self.marks.get(&id).copied() }
    pub fn marked_count(&self) -> usize { self.marks.len() }
    pub fn marks(&self) -> &HashMap<EntryId, Mark> { &self.marks }

    pub fn begin_scan(&mut self, directory: PathBuf) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.directory = directory;
        self.generation
    }

    pub fn apply_scan(&mut self, generation: u64, mut result: ScanResult) -> bool {
        if generation != self.generation || result.directory != self.directory { return false; }
        let old_cursor = self.cursor;
        sort_entries(&mut result.entries, self.sort);
        if !self.show_hidden { result.entries.retain(|entry| !entry.metadata.hidden); }
        let ids = result.entries.iter().map(|entry| entry.id).collect::<HashSet<_>>();
        self.marks.retain(|id, _| ids.contains(id));
        self.cursor = old_cursor.filter(|id| ids.contains(id)).or_else(|| result.entries.first().map(|entry| entry.id));
        self.entries = Arc::new(result.entries);
        true
    }

    pub fn selected(&self) -> Option<&FileEntry> {
        let cursor = self.cursor?;
        self.entries.iter().find(|entry| entry.id == cursor)
    }

    pub fn move_cursor(&mut self, delta: i64) {
        if self.entries.is_empty() { self.cursor = None; return; }
        let current = self.cursor.and_then(|id| self.entries.iter().position(|entry| entry.id == id)).unwrap_or(0) as i64;
        let index = current.saturating_add(delta).clamp(0, self.entries.len() as i64 - 1) as usize;
        self.cursor = Some(self.entries[index].id);
    }

    pub fn set_cursor(&mut self, id: EntryId) { if self.entries.iter().any(|entry| entry.id == id) { self.cursor = Some(id); } }
    pub fn mark_selected(&mut self, mark: Mark) { if let Some(id) = self.cursor { self.marks.insert(id, mark); self.move_cursor(1); } }
    pub fn unmark_selected(&mut self) { if let Some(id) = self.cursor { self.marks.remove(&id); self.move_cursor(1); } }
    pub fn unmark_all(&mut self) { self.marks.clear(); }
    pub fn invert_marks(&mut self) {
        for entry in self.entries.iter().filter(|entry| !matches!(entry.kind, EntryKind::Parent)) {
            if self.marks.remove(&entry.id).is_none() { self.marks.insert(entry.id, Mark::Selected); }
        }
    }
    pub fn deletion_plan(&self) -> OperationPlan {
        OperationPlan { delete: self.entries.iter().filter(|entry| self.marks.get(&entry.id) == Some(&Mark::Delete)).map(|entry| entry.path.clone()).collect::<Vec<_>>().into() }
    }
}

fn sort_entries(entries: &mut [FileEntry], spec: SortSpec) {
    entries.sort_by(|left, right| {
        if matches!(left.kind, EntryKind::Parent) { return std::cmp::Ordering::Less; }
        if matches!(right.kind, EntryKind::Parent) { return std::cmp::Ordering::Greater; }
        let directory_order = spec.directories_first.then(|| right.is_directory().cmp(&left.is_directory())).unwrap_or(std::cmp::Ordering::Equal);
        let name_order = left.display_name.to_lowercase().cmp(&right.display_name.to_lowercase());
        let order = directory_order.then(name_order);
        if spec.direction == SortDirection::Ascending { order } else { order.reverse() }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_manager::{EntryMetadata, ScanResult};
    use std::{ffi::OsString, time::SystemTime};

    fn entry(id: u64, name: &str, kind: EntryKind) -> FileEntry {
        FileEntry { id: EntryId(id), path: Arc::from(PathBuf::from(name)), os_name: Arc::new(OsString::from(name)), display_name: Arc::from(name), kind, metadata: EntryMetadata { byte_len: None, modified: Some(SystemTime::UNIX_EPOCH), hidden: false, symlink_target: None } }
    }

    #[test]
    fn sorts_directories_first_and_reconciles_cursor_and_marks() {
        let directory = PathBuf::from("/tmp/test");
        let mut session = DiredSession::empty(directory.clone());
        let generation = session.begin_scan(directory.clone());
        session.apply_scan(generation, ScanResult { directory: directory.clone(), entries: vec![entry(2, "z.org", EntryKind::OrgFile), entry(1, "folder", EntryKind::Directory)] });
        assert_eq!(session.entries()[0].display_name.as_ref(), "folder");
        session.mark_selected(Mark::Selected);
        let next_generation = session.begin_scan(directory.clone());
        session.apply_scan(next_generation, ScanResult { directory, entries: vec![entry(1, "folder", EntryKind::Directory)] });
        assert_eq!(session.marked_count(), 1);
    }

    #[test]
    fn rejects_stale_scan_and_builds_delete_plan_without_side_effects() {
        let directory = PathBuf::from("/tmp/test");
        let mut session = DiredSession::empty(directory.clone());
        let stale = session.begin_scan(directory.clone());
        let current = session.begin_scan(directory.clone());
        assert!(!session.apply_scan(stale, ScanResult { directory: directory.clone(), entries: vec![] }));
        session.apply_scan(current, ScanResult { directory, entries: vec![entry(3, "old.org", EntryKind::OrgFile)] });
        session.mark_selected(Mark::Delete);
        assert_eq!(session.deletion_plan().delete.len(), 1);
    }
}
