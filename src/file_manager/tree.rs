//! File-system outline state. Rows are a cached projection of DirectoryStore,
//! never another directory scanner or a collection of Dired sessions.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use super::{
    DirectorySnapshot, DirectoryStore, EntryId, FileEntry, SortSpec, entry::EntryLookup,
    sort::compare_entries,
};

struct DirectoryProjection {
    revision: u64,
    sort: SortSpec,
    show_hidden: bool,
    entries: Arc<[Arc<FileEntry>]>,
}

#[derive(Clone, Debug)]
pub enum TreeRowKind {
    Entry(Arc<FileEntry>),
    Loading,
    Empty,
    Error(Arc<str>),
}

#[derive(Clone, Debug)]
pub struct TreeRow {
    pub kind: TreeRowKind,
    pub depth: usize,
    pub parent: PathBuf,
}

impl TreeRow {
    pub fn entry(&self) -> Option<&Arc<FileEntry>> {
        match &self.kind {
            TreeRowKind::Entry(entry) => Some(entry),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TreeSelection {
    #[default]
    Single,
    Extend,
    Toggle,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeMemory {
    pub root: PathBuf,
    pub expanded: Vec<PathBuf>,
    pub cursor: Option<PathBuf>,
    pub viewport: Option<(PathBuf, f32)>,
    pub sort: SortSpec,
    pub show_hidden: bool,
    pub auto_reveal: bool,
}

#[derive(Default)]
pub struct FileTree {
    root: Option<Arc<FileEntry>>,
    expanded: HashSet<PathBuf>,
    cursor: Option<EntryId>,
    selected: HashSet<EntryId>,
    range_anchor: Option<EntryId>,
    pending_selection: Option<PathBuf>,
    rows: Arc<[TreeRow]>,
    ranks: HashMap<EntryId, usize>,
    navigation: Vec<usize>,
    pub sort: SortSpec,
    pub show_hidden: bool,
    pub auto_reveal: bool,
    viewport: Option<(PathBuf, f32)>,
    memories: VecDeque<TreeMemory>,
    revision: u64,
    maximum_depth: usize,
    directories: HashMap<PathBuf, DirectoryProjection>,
}

impl FileTree {
    pub fn restore(memories: Vec<TreeMemory>) -> Self {
        let root = memories.first().map(|memory| memory.root.clone());
        let mut tree = Self {
            memories: memories.into_iter().take(32).collect(),
            ..Self::default()
        };
        if let Some(root) = root {
            tree.set_root(root);
        }
        tree
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_ref().map(|entry| entry.path.as_ref())
    }
    pub fn parent_root(&self) -> Option<&Path> {
        self.root()
            .and_then(Path::parent)
            .filter(|parent| !parent.as_os_str().is_empty())
    }
    pub fn rows(&self) -> &Arc<[TreeRow]> {
        &self.rows
    }
    pub fn cursor(&self) -> Option<EntryId> {
        self.cursor
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn maximum_depth(&self) -> usize {
        self.maximum_depth
    }
    pub fn rank(&self, id: EntryId) -> Option<usize> {
        self.ranks.get(&id).copied()
    }
    pub fn selected(&self, id: EntryId) -> bool {
        self.selected.contains(&id)
    }
    pub fn expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }
    pub fn entry(&self, id: EntryId) -> Option<&Arc<FileEntry>> {
        self.rows.get(self.rank(id)?)?.entry()
    }
    pub fn current(&self) -> Option<&Arc<FileEntry>> {
        self.entry(self.cursor?)
    }

    pub fn single_selected(&self) -> Option<&Arc<FileEntry>> {
        if self.selected.len() != 1 {
            return None;
        }
        self.entry(*self.selected.iter().next()?)
    }

    pub fn set_root(&mut self, path: PathBuf) {
        if self.root() == Some(path.as_path()) {
            return;
        }
        self.remember();
        let memory = self
            .memories
            .iter()
            .find(|memory| memory.root == path)
            .cloned();
        self.root = Some(Arc::new(FileEntry::directory(path.clone())));
        self.expanded = memory
            .as_ref()
            .map(|memory| memory.expanded.iter().cloned().collect())
            .unwrap_or_else(|| HashSet::from([path]));
        self.pending_selection = memory.as_ref().and_then(|memory| memory.cursor.clone());
        self.viewport = memory.as_ref().and_then(|memory| memory.viewport.clone());
        self.sort = memory
            .as_ref()
            .map(|memory| memory.sort)
            .unwrap_or_default();
        self.show_hidden = memory.as_ref().is_some_and(|memory| memory.show_hidden);
        self.auto_reveal = memory.as_ref().is_some_and(|memory| memory.auto_reveal);
        self.cursor = self.root.as_ref().map(|root| root.id);
        self.selected = self.cursor.into_iter().collect();
        self.range_anchor = self.cursor;
    }

    pub fn remember(&mut self) {
        let Some(root) = self.root() else {
            return;
        };
        let root = root.to_path_buf();
        let mut expanded = self.expanded.iter().cloned().collect::<Vec<_>>();
        expanded.sort();
        expanded.truncate(8192);
        let memory = TreeMemory {
            root: root.clone(),
            expanded,
            cursor: self
                .pending_selection
                .clone()
                .or_else(|| self.current().map(|entry| entry.path.to_path_buf())),
            viewport: self.viewport.clone(),
            sort: self.sort,
            show_hidden: self.show_hidden,
            auto_reveal: self.auto_reveal,
        };
        self.memories.retain(|memory| memory.root != root);
        self.memories.push_front(memory);
        self.memories.truncate(32);
    }

    pub fn memories(&mut self) -> Vec<TreeMemory> {
        self.remember();
        self.memories.iter().cloned().collect()
    }

    pub fn capture_viewport(&mut self, rank: usize, offset: f32) {
        if let Some(row) = self.rows.get(rank) {
            let path = row
                .entry()
                .map(|entry| entry.path.to_path_buf())
                .unwrap_or_else(|| row.parent.clone());
            self.viewport = Some((path, offset));
        }
    }

    pub fn viewport(&self) -> Option<(usize, f32)> {
        let (path, offset) = self.viewport.as_ref()?;
        path.ancestors().find_map(|path| {
            self.rank(EntryId::for_path(path))
                .map(|rank| (rank, *offset))
        })
    }

    pub fn reveal(&mut self, path: &Path) -> bool {
        let Some(root) = self.root() else {
            return false;
        };
        if !path.starts_with(root) {
            return false;
        }
        let root = root.to_path_buf();
        let mut parent = path.parent();
        while let Some(directory) = parent.filter(|directory| directory.starts_with(&root)) {
            self.expanded.insert(directory.to_path_buf());
            parent = directory.parent();
        }
        if path.strip_prefix(&root).is_ok_and(|relative| {
            relative
                .components()
                .any(|part| part.as_os_str().to_string_lossy().starts_with('.'))
        }) {
            self.show_hidden = true;
        }
        self.pending_selection = Some(path.to_path_buf());
        true
    }

    pub fn toggle(&mut self, id: EntryId) -> bool {
        let Some(entry) = self.entry(id).filter(|entry| entry.is_directory()).cloned() else {
            return false;
        };
        if !self.expanded.remove(entry.path.as_ref()) {
            self.expanded.insert(entry.path.to_path_buf());
        }
        true
    }

    pub fn set_expanded(&mut self, path: &Path, expanded: bool) {
        if expanded {
            if self.root().is_some_and(|root| path.starts_with(root)) {
                self.expanded.insert(path.to_path_buf());
            }
        } else {
            self.expanded.remove(path);
        }
    }

    pub fn collapse_all(&mut self) {
        self.expanded.clear();
        if let Some(root) = &self.root {
            self.expanded.insert(root.path.to_path_buf());
        }
    }

    pub fn select(&mut self, id: EntryId, mode: TreeSelection) {
        let Some(rank) = self.rank(id) else {
            return;
        };
        self.pending_selection = None;
        match mode {
            TreeSelection::Single => {
                self.selected.clear();
                self.selected.insert(id);
                self.range_anchor = Some(id);
            }
            TreeSelection::Toggle => {
                if !self.selected.remove(&id) {
                    self.selected.insert(id);
                }
                self.range_anchor = Some(id);
            }
            TreeSelection::Extend => {
                let start = self
                    .range_anchor
                    .and_then(|anchor| self.rank(anchor))
                    .unwrap_or(rank);
                self.selected = self.rows[start.min(rank)..=start.max(rank)]
                    .iter()
                    .filter_map(|row| row.entry().map(|entry| entry.id))
                    .collect();
            }
        }
        self.cursor = Some(id);
    }

    pub fn clear_selection(&mut self) {
        self.cursor = None;
        self.selected.clear();
        self.range_anchor = None;
        self.pending_selection = None;
    }

    pub fn move_cursor(&mut self, delta: i64, extend: bool) {
        let current = self
            .cursor
            .and_then(|id| self.rank(id))
            .and_then(|rank| self.navigation.binary_search(&rank).ok())
            .unwrap_or(0);
        let next = (current as i64)
            .saturating_add(delta)
            .clamp(0, self.navigation.len().saturating_sub(1) as i64) as usize;
        if let Some(id) = self
            .navigation
            .get(next)
            .and_then(|rank| self.rows[*rank].entry())
            .map(|entry| entry.id)
        {
            self.select(
                id,
                if extend {
                    TreeSelection::Extend
                } else {
                    TreeSelection::Single
                },
            );
        }
    }

    /// Returns whether expansion changed; the host schedules the directory read.
    pub fn move_hierarchy(&mut self, forward: bool) -> bool {
        let Some(entry) = self.current().cloned() else {
            return false;
        };
        if entry.is_directory() && self.expanded(&entry.path) != forward {
            return self.toggle(entry.id);
        }
        if forward {
            let rank = self.rank(entry.id).unwrap_or(0);
            if let Some(child) = self
                .rows
                .get(rank + 1)
                .and_then(TreeRow::entry)
                .filter(|child| child.path.parent() == Some(entry.path.as_ref()))
            {
                let id = child.id;
                self.select(id, TreeSelection::Single);
            }
        } else if let Some(parent) = entry.path.parent() {
            self.select(EntryId::for_path(parent), TreeSelection::Single);
        }
        false
    }

    pub fn type_ahead(&mut self, prefix: &str) {
        let prefix = prefix.to_lowercase();
        if prefix.chars().count() > 1
            && self
                .current()
                .is_some_and(|entry| entry.display_name.to_lowercase().starts_with(&prefix))
        {
            return;
        }
        let current = self.cursor.and_then(|id| self.rank(id)).unwrap_or(0);
        let next = self
            .rows
            .iter()
            .enumerate()
            .skip(current + 1)
            .chain(self.rows.iter().enumerate().take(current + 1))
            .find_map(|(_, row)| {
                row.entry()
                    .filter(|entry| entry.display_name.to_lowercase().starts_with(&prefix))
                    .map(|entry| entry.id)
            });
        if let Some(id) = next {
            self.select(id, TreeSelection::Single);
        }
    }

    pub fn operation_targets(&self) -> Arc<[Arc<Path>]> {
        self.rows
            .iter()
            .filter_map(TreeRow::entry)
            .filter(|entry| self.selected(entry.id))
            .map(|entry| entry.path.clone())
            .collect()
    }

    pub fn creation_directory(&self) -> Option<PathBuf> {
        self.current()
            .map(|entry| {
                if entry.is_directory() {
                    entry.path.to_path_buf()
                } else {
                    entry.path.parent().unwrap_or(&entry.path).to_path_buf()
                }
            })
            .or_else(|| self.root().map(Path::to_path_buf))
    }

    pub fn active_directories(&self) -> HashSet<PathBuf> {
        let mut active = self
            .rows
            .iter()
            .filter_map(TreeRow::entry)
            .filter(|entry| entry.is_directory() && self.expanded(&entry.path))
            .map(|entry| entry.path.to_path_buf())
            .collect::<HashSet<_>>();
        if let Some(root) = self.root() {
            active.insert(root.to_path_buf());
        }
        active
    }

    pub fn relocate(&mut self, source: &Path, destination: &Path) {
        let replace = |path: &Path| {
            path.strip_prefix(source)
                .ok()
                .map(|suffix| destination.join(suffix))
                .unwrap_or_else(|| path.to_path_buf())
        };
        self.expanded = self.expanded.iter().map(|path| replace(path)).collect();
        for memory in &mut self.memories {
            memory.root = replace(&memory.root);
            memory.expanded = memory.expanded.iter().map(|path| replace(path)).collect();
            memory.cursor = memory.cursor.as_deref().map(replace);
            memory.viewport = memory
                .viewport
                .as_ref()
                .map(|(path, offset)| (replace(path), *offset));
        }
        for entry in self.rows.iter().filter_map(TreeRow::entry) {
            if entry.path.starts_with(source) && self.selected.remove(&entry.id) {
                self.selected
                    .insert(EntryId::for_path(&replace(&entry.path)));
            }
        }
        if let Some(path) = &self.pending_selection {
            self.pending_selection = Some(replace(path));
        }
        if let Some((path, offset)) = &self.viewport {
            self.viewport = Some((replace(path), *offset));
        }
        if let Some(root) = self.root().filter(|root| root.starts_with(source)) {
            self.root = Some(Arc::new(FileEntry::directory(replace(root))));
        }
        if let Some(path) = self
            .current()
            .map(|entry| entry.path.to_path_buf())
            .filter(|path| path.starts_with(source))
        {
            self.pending_selection = Some(replace(&path));
        }
    }

    pub fn rebuild(&mut self, store: &DirectoryStore) {
        let old_current = self.current().cloned();
        let old_rank = self.cursor.and_then(|id| self.rank(id)).unwrap_or(0);
        let old_selected = self.cursor.is_some_and(|id| self.selected(id));
        // Repair paths before projecting children so an expanded directory
        // follows an external rename without silently following a hard link.
        let mut groups = HashMap::<PathBuf, Vec<&FileEntry>>::new();
        for row in self.rows.iter().skip(1) {
            if let Some(entry) = row.entry() {
                groups.entry(row.parent.clone()).or_default().push(entry);
            }
        }
        let mut renamed = Vec::new();
        for (parent, old_entries) in groups {
            if let Some(snapshot) = store.snapshot(&parent) {
                if self
                    .directories
                    .get(&parent)
                    .is_some_and(|cached| cached.revision == snapshot.revision)
                {
                    continue;
                }
                let old = EntryLookup::new(old_entries.iter().copied());
                let new = EntryLookup::new(snapshot.entries.iter().map(Arc::as_ref));
                for entry in old_entries {
                    if new.path(&entry.path).is_none()
                        && let Some(resource) = entry.resource_id
                        && old.resource(resource).is_some()
                        && let Some(replacement) = new.resource(resource)
                    {
                        renamed.push((entry.path.clone(), replacement.path.clone()));
                    }
                }
            }
        }
        for (source, destination) in renamed {
            self.relocate(&source, &destination);
        }
        if let Some(root) = self.root().map(Path::to_path_buf) {
            self.expanded.retain(|path| {
                path.starts_with(&root)
                    && path
                        .ancestors()
                        .take_while(|ancestor| *ancestor != root)
                        .all(|ancestor| {
                            ancestor.parent().is_none_or(|parent| {
                                !store.is_current(parent)
                                    || store.snapshot(parent).unwrap().entries.iter().any(|entry| {
                                        entry.is_directory() && entry.path.as_ref() == ancestor
                                    })
                            })
                        })
            });
        }
        if self.pending_selection.as_ref().is_some_and(|path| {
            path.parent().is_some_and(|parent| {
                store.is_current(parent)
                    && store
                        .snapshot(parent)
                        .unwrap()
                        .entries
                        .iter()
                        .all(|entry| entry.path.as_ref() != path)
            })
        }) {
            self.pending_selection = None;
        }
        let mut rows = Vec::new();
        if let Some(root) = self.root.clone() {
            rows.push(TreeRow {
                kind: TreeRowKind::Entry(root.clone()),
                depth: 0,
                parent: root.path.to_path_buf(),
            });
            self.append_children(&root, 1, store, &mut rows);
        }
        self.rows = rows.into();
        let active = self.active_directories();
        self.directories.retain(|path, _| active.contains(path));
        self.maximum_depth = self.rows.iter().map(|row| row.depth).max().unwrap_or(0);
        self.ranks = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(rank, row)| row.entry().map(|entry| (entry.id, rank)))
            .collect();
        self.navigation = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(rank, row)| row.entry().map(|_| rank))
            .collect();
        self.selected.retain(|id| self.ranks.contains_key(id));
        if let Some(target) = &self.pending_selection
            && let Some(entry) = self.entry(EntryId::for_path(target))
        {
            let id = entry.id;
            self.pending_selection = None;
            self.cursor = Some(id);
            self.selected = HashSet::from([id]);
        }
        if !self.cursor.is_some_and(|id| self.ranks.contains_key(&id)) {
            self.cursor = old_current
                .as_ref()
                .and_then(|old| {
                    // Deletion within an expanded, loaded directory keeps the
                    // closest sibling; collapsing a branch returns its parent.
                    if let Some(parent) = old.path.parent()
                        && self.expanded(parent)
                        && store.snapshot(parent).is_some()
                        && let Some(sibling) = self
                            .rows
                            .iter()
                            .enumerate()
                            .filter(|(_, row)| row.parent == parent && row.depth > 0)
                            .filter_map(|(rank, row)| row.entry().map(|entry| (rank, entry)))
                            .min_by_key(|(rank, _)| rank.abs_diff(old_rank))
                            .map(|(_, entry)| entry.id)
                    {
                        return Some(sibling);
                    }
                    // Prefer the visible ancestor when a branch is collapsed.
                    old.path
                        .ancestors()
                        .skip(1)
                        .find_map(|path| self.entry(EntryId::for_path(path)).map(|entry| entry.id))
                })
                .or_else(|| {
                    self.rows
                        .get(old_rank.min(self.rows.len().saturating_sub(1)))
                        .and_then(TreeRow::entry)
                        .map(|entry| entry.id)
                })
                .or_else(|| self.root.as_ref().map(|root| root.id));
            if old_selected && let Some(id) = self.cursor {
                self.selected.insert(id);
            }
        }
        if !self
            .range_anchor
            .is_some_and(|id| self.ranks.contains_key(&id))
        {
            self.range_anchor = self.cursor;
        }
        self.revision = self.revision.wrapping_add(1);
    }

    fn append_children(
        &mut self,
        entry: &FileEntry,
        depth: usize,
        store: &DirectoryStore,
        rows: &mut Vec<TreeRow>,
    ) {
        if !entry.is_directory() || !self.expanded(&entry.path) {
            return;
        }
        let status = if let Some(error) = store.error(&entry.path) {
            Some(TreeRowKind::Error(error.clone()))
        } else if store.snapshot(&entry.path).is_none() {
            Some(TreeRowKind::Loading)
        } else {
            None
        };
        let mut count = 0;
        if let Some(snapshot) = store.snapshot(&entry.path) {
            let children = self.directory_projection(snapshot);
            count = children.len();
            for child in children.iter() {
                rows.push(TreeRow {
                    kind: TreeRowKind::Entry(child.clone()),
                    depth,
                    parent: entry.path.to_path_buf(),
                });
                self.append_children(child, depth + 1, store, rows);
            }
        }
        if let Some(kind) = status.or_else(|| (count == 0).then_some(TreeRowKind::Empty)) {
            rows.push(TreeRow {
                kind,
                depth,
                parent: entry.path.to_path_buf(),
            });
        }
    }

    fn directory_projection(&mut self, snapshot: &DirectorySnapshot) -> Arc<[Arc<FileEntry>]> {
        if let Some(cached) = self.directories.get(&snapshot.directory)
            && cached.revision == snapshot.revision
            && cached.sort == self.sort
            && cached.show_hidden == self.show_hidden
        {
            return cached.entries.clone();
        }
        let mut children = snapshot
            .entries
            .iter()
            .filter(|entry| self.show_hidden || !entry.metadata.hidden)
            .cloned()
            .collect::<Vec<_>>();
        children.sort_by(|left, right| compare_entries(left, right, self.sort));
        let entries: Arc<[Arc<FileEntry>]> = children.into();
        self.directories.insert(
            snapshot.directory.clone(),
            DirectoryProjection {
                revision: snapshot.revision,
                sort: self.sort,
                show_hidden: self.show_hidden,
                entries: entries.clone(),
            },
        );
        entries
    }
}

#[cfg(test)]
mod tests;
