use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::navigation::{
    AnchorAffinity, CacheIntent, CancellationToken, ContentRevision, ContentSnapshot, FocusTarget,
    HistoryIntent, HistoryTimeline, InteractionState, ItemAnchor, LocationMemory, NavigationCause,
    NavigationIntent, SelectionIntent, TransactionId, TransactionManager, ViewRevision,
    ViewSnapshot, ViewportAnchor, ViewportIntent,
};

use super::{EntryId, EntryKind, FileEntry, FileResourceId, Mark, OperationPlan, ScanResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct PresentationSignature {
    pub descending: bool,
    pub directories_first: bool,
    pub show_hidden: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SortSpec {
    pub direction: SortDirection,
    pub directories_first: bool,
}
impl Default for SortSpec {
    fn default() -> Self {
        Self {
            direction: SortDirection::Ascending,
            directories_first: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileFallback {
    pub path: PathBuf,
    pub parent: PathBuf,
    pub name: OsString,
}

pub type FileAnchor = ItemAnchor<FileResourceId, FileFallback>;
pub type DiredViewSnapshot = ViewSnapshot<FileAnchor, PresentationSignature>;

#[derive(Clone, Debug)]
pub struct NavigationLoad {
    pub transaction_id: TransactionId,
    pub cancellation: CancellationToken,
    pub intent: NavigationIntent<PathBuf, FileAnchor>,
    history_snapshot: Option<DiredViewSnapshot>,
    preserved_snapshot: Option<DiredViewSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavigationCommit {
    pub transaction_id: TransactionId,
    pub view_revision: ViewRevision,
    pub item_count: usize,
    pub presentation_rank: Option<usize>,
    pub presentation_offset: f32,
}

#[derive(Clone, Debug)]
struct MarkedOperation {
    path: Arc<Path>,
    resource_id: Option<FileResourceId>,
    mark: Mark,
}

#[derive(Default)]
struct OperationPresentation {
    visible_marks: Arc<HashMap<EntryId, Mark>>,
    sorted_targets: Arc<[Arc<Path>]>,
}

pub struct DiredSession {
    directory: PathBuf,
    snapshot: Arc<ContentSnapshot<FileEntry, EntryId>>,
    cursor: Option<EntryId>,
    sort: SortSpec,
    show_hidden: bool,
    transactions: TransactionManager,
    next_content_revision: u64,
    view_revision: ViewRevision,
    viewport: Option<ViewportAnchor<FileAnchor>>,
    location_memory: LocationMemory<PathBuf, DiredViewSnapshot>,
    history: HistoryTimeline<PathBuf, DiredViewSnapshot>,
    operations: HashMap<EntryId, MarkedOperation>,
    operation_presentation: OperationPresentation,
}

impl DiredSession {
    pub fn empty(directory: PathBuf) -> Self {
        Self {
            directory,
            snapshot: Arc::new(
                ContentSnapshot::try_new(ContentRevision(0), Vec::new(), |entry: &FileEntry| {
                    entry.id
                })
                .expect("empty snapshot is valid"),
            ),
            cursor: None,
            sort: SortSpec::default(),
            show_hidden: true,
            transactions: TransactionManager::default(),
            next_content_revision: 0,
            view_revision: ViewRevision(0),
            viewport: None,
            location_memory: LocationMemory::new(512),
            history: HistoryTimeline::default(),
            operations: HashMap::new(),
            operation_presentation: OperationPresentation::default(),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn entries(&self) -> &Arc<[FileEntry]> {
        self.snapshot.items()
    }
    pub fn cursor(&self) -> Option<EntryId> {
        self.cursor
    }
    pub fn view_revision(&self) -> ViewRevision {
        self.view_revision
    }
    pub fn marked_count(&self) -> usize {
        self.operations.len()
    }
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn mark(&self, id: EntryId) -> Option<Mark> {
        self.operations.get(&id).map(|operation| operation.mark)
    }
    pub fn visible_marks(&self) -> Arc<HashMap<EntryId, Mark>> {
        self.operation_presentation.visible_marks.clone()
    }

    pub fn selected(&self) -> Option<&FileEntry> {
        self.cursor
            .and_then(|id| self.snapshot.rank_of(&id))
            .and_then(|rank| self.snapshot.item_at(rank))
    }

    pub fn intent_for(
        directory: PathBuf,
        cause: NavigationCause,
        selection: SelectionIntent<FileAnchor>,
    ) -> NavigationIntent<PathBuf, FileAnchor> {
        let viewport = match &selection {
            SelectionIntent::Explicit(anchor) => ViewportIntent::EnsureVisible(anchor.clone()),
            SelectionIntent::Restore => ViewportIntent::Restore,
            SelectionIntent::FirstSelectable | SelectionIntent::Preserve => {
                ViewportIntent::Preserve
            }
        };
        NavigationIntent {
            target: directory,
            cause,
            selection,
            viewport,
            history: if matches!(
                cause,
                NavigationCause::Refresh | NavigationCause::FileSystemDelta
            ) {
                HistoryIntent::Ignore
            } else {
                HistoryIntent::Push
            },
            cache: if matches!(cause, NavigationCause::Refresh) {
                CacheIntent::Reload
            } else {
                CacheIntent::PreferCache
            },
        }
    }

    pub fn begin_navigation(
        &mut self,
        intent: NavigationIntent<PathBuf, FileAnchor>,
    ) -> NavigationLoad {
        self.remember_current_view();
        let preserved_snapshot = self.current_view_snapshot();
        let transaction = self.transactions.begin();
        NavigationLoad {
            transaction_id: transaction.id,
            cancellation: transaction.cancellation,
            intent,
            history_snapshot: None,
            preserved_snapshot: Some(preserved_snapshot),
        }
    }

    pub fn begin_history_navigation(&mut self, forward: bool) -> Option<NavigationLoad> {
        self.remember_current_view();
        let entry = if forward {
            self.history.peek_forward()
        } else {
            self.history.peek_back()
        }?
        .clone();
        let transaction = self.transactions.begin();
        let anchor = entry.snapshot.interaction.primary_selection.clone();
        let intent = NavigationIntent {
            target: entry.location,
            cause: if forward {
                NavigationCause::Forward
            } else {
                NavigationCause::Back
            },
            selection: anchor
                .clone()
                .map(SelectionIntent::Explicit)
                .unwrap_or(SelectionIntent::FirstSelectable),
            viewport: ViewportIntent::Restore,
            history: HistoryIntent::Traverse(entry.id),
            cache: CacheIntent::PreferCache,
        };
        Some(NavigationLoad {
            transaction_id: transaction.id,
            cancellation: transaction.cancellation,
            intent,
            history_snapshot: Some(entry.snapshot),
            preserved_snapshot: None,
        })
    }

    pub fn apply_scan(
        &mut self,
        load: &NavigationLoad,
        mut result: ScanResult,
    ) -> Option<NavigationCommit> {
        if !self.transactions.accepts(load.transaction_id) || result.directory != load.intent.target
        {
            return None;
        }
        self.reconcile_operations(&result.directory, &result.entries);
        sort_entries(&mut result.entries, self.sort);
        if !self.show_hidden {
            result.entries.retain(|entry| !entry.metadata.hidden);
        }
        self.next_content_revision = self.next_content_revision.wrapping_add(1);
        let snapshot = ContentSnapshot::try_new(
            ContentRevision(self.next_content_revision),
            result.entries,
            |entry| entry.id,
        )
        .ok()?;
        self.snapshot = Arc::new(snapshot);
        self.directory = load.intent.target.clone();
        self.rebuild_operation_presentation();

        let remembered_view = load
            .history_snapshot
            .clone()
            .or_else(|| self.location_memory.get(&load.intent.target).cloned());
        let explicit = match &load.intent.selection {
            SelectionIntent::Explicit(anchor) => self.reconcile(anchor),
            _ => None,
        };
        let remembered = remembered_view
            .as_ref()
            .and_then(|snapshot| snapshot.interaction.primary_selection.as_ref())
            .and_then(|anchor| self.reconcile(anchor));
        let preserved = if matches!(load.intent.selection, SelectionIntent::Preserve) {
            self.cursor.filter(|id| self.snapshot.rank_of(id).is_some())
        } else {
            None
        };
        self.cursor = explicit
            .or(preserved)
            .or(remembered)
            .or_else(|| self.entries().first().map(|entry| entry.id));
        self.view_revision = ViewRevision(self.view_revision.0.wrapping_add(1));
        let viewport_source = match load.intent.viewport {
            ViewportIntent::Restore => remembered_view.as_ref(),
            ViewportIntent::Preserve => load.preserved_snapshot.as_ref(),
            ViewportIntent::EnsureVisible(_) | ViewportIntent::Center(_) => None,
        };
        let restored_viewport = viewport_source
            .and_then(|snapshot| snapshot.viewport.as_ref())
            .and_then(|viewport| {
                self.reconcile(&viewport.item)
                    .map(|id| (id, viewport.clone()))
            });
        self.viewport = restored_viewport
            .as_ref()
            .map(|(_, viewport)| viewport.clone());
        let view = self.current_view_snapshot();
        match load.intent.history {
            HistoryIntent::Push => {
                self.history.push(load.intent.target.clone(), view);
            }
            HistoryIntent::Traverse(id) => {
                self.history.traverse_to(id);
            }
            HistoryIntent::Replace | HistoryIntent::Ignore => {}
        }
        let (presentation_rank, presentation_offset) = restored_viewport
            .and_then(|(id, viewport)| {
                self.snapshot
                    .rank_of(&id)
                    .map(|rank| (Some(rank), viewport.offset_from_viewport_start))
            })
            .unwrap_or_else(|| (self.cursor.and_then(|id| self.snapshot.rank_of(&id)), 0.0));
        Some(NavigationCommit {
            transaction_id: load.transaction_id,
            view_revision: self.view_revision,
            item_count: self.snapshot.len(),
            presentation_rank,
            presentation_offset,
        })
    }

    pub fn presentation_is_current(&self, transaction: TransactionId, view: ViewRevision) -> bool {
        self.transactions.accepts(transaction) && self.view_revision == view
    }
    pub fn accepts_transaction(&self, transaction: TransactionId) -> bool {
        self.transactions.accepts(transaction)
    }

    pub fn anchor_for_path(&self, path: &Path) -> FileAnchor {
        let entry = self
            .entries()
            .iter()
            .find(|entry| entry.path.as_ref() == path);
        FileAnchor {
            primary: entry.and_then(|entry| entry.resource_id),
            fallback: FileFallback {
                path: path.to_path_buf(),
                parent: path.parent().unwrap_or(path).to_path_buf(),
                name: path.file_name().unwrap_or_default().to_os_string(),
            },
            previous_rank: entry.and_then(|entry| self.snapshot.rank_of(&entry.id)),
            affinity: AnchorAffinity::Nearest,
        }
    }

    pub fn capture_viewport(&mut self, rank: usize, offset_from_viewport_start: f32) {
        self.viewport = self.snapshot.item_at(rank).map(|entry| ViewportAnchor {
            item: self.anchor_for_path(&entry.path),
            offset_from_viewport_start,
            alignment: crate::navigation::AnchorAlignment::Start,
        });
    }

    fn reconcile(&self, anchor: &FileAnchor) -> Option<EntryId> {
        anchor
            .primary
            .and_then(|resource| {
                self.entries()
                    .iter()
                    .find(|entry| entry.resource_id == Some(resource))
                    .map(|entry| entry.id)
            })
            .or_else(|| {
                self.entries()
                    .iter()
                    .find(|entry| entry.path.as_ref() == anchor.fallback.path)
                    .map(|entry| entry.id)
            })
            .or_else(|| {
                self.entries()
                    .iter()
                    .find(|entry| entry.os_name.as_ref() == &anchor.fallback.name)
                    .map(|entry| entry.id)
            })
            .or_else(|| {
                anchor
                    .previous_rank
                    .and_then(|rank| nearest_rank(self.snapshot.len(), rank, anchor.affinity))
                    .and_then(|rank| self.snapshot.item_at(rank))
                    .map(|entry| entry.id)
            })
    }

    fn remember_current_view(&mut self) {
        if !self.snapshot.is_empty() {
            let snapshot = self.current_view_snapshot();
            self.location_memory
                .insert(self.directory.clone(), snapshot.clone());
            if self
                .history
                .current()
                .is_some_and(|entry| entry.location == self.directory)
            {
                self.history.update_current_snapshot(snapshot);
            }
        }
    }

    fn current_view_snapshot(&self) -> DiredViewSnapshot {
        let anchor = self
            .selected()
            .map(|entry| self.anchor_for_path(&entry.path));
        ViewSnapshot {
            interaction: InteractionState {
                focus: FocusTarget::List,
                primary_selection: anchor,
                secondary_selection: Vec::new(),
            },
            viewport: self.viewport.clone(),
            presentation: PresentationSignature {
                descending: self.sort.direction == SortDirection::Descending,
                directories_first: self.sort.directories_first,
                show_hidden: self.show_hidden,
            },
            view_revision: self.view_revision,
        }
    }

    pub fn move_cursor(&mut self, delta: i64) {
        if self.snapshot.is_empty() {
            self.cursor = None;
            return;
        }
        let current = self
            .cursor
            .and_then(|id| self.snapshot.rank_of(&id))
            .unwrap_or(0) as i64;
        let rank = current
            .saturating_add(delta)
            .clamp(0, self.snapshot.len() as i64 - 1) as usize;
        self.cursor = self.snapshot.item_at(rank).map(|entry| entry.id);
        self.view_revision = ViewRevision(self.view_revision.0.wrapping_add(1));
    }

    pub fn set_cursor(&mut self, id: EntryId) {
        if self.snapshot.rank_of(&id).is_some() {
            self.cursor = Some(id);
            self.view_revision = ViewRevision(self.view_revision.0.wrapping_add(1));
        }
    }

    pub fn mark_selected(&mut self, mark: Mark) {
        if let Some(entry) = self.selected().cloned() {
            if matches!(entry.kind, EntryKind::Parent) {
                return;
            }
            self.operations.insert(
                entry.id,
                MarkedOperation {
                    path: entry.path,
                    resource_id: entry.resource_id,
                    mark,
                },
            );
            self.rebuild_operation_presentation();
            self.move_cursor(1);
        }
    }
    pub fn unmark_selected(&mut self) {
        if let Some(id) = self.cursor {
            self.operations.remove(&id);
            self.rebuild_operation_presentation();
            self.move_cursor(1);
        }
    }
    pub fn unmark_all(&mut self) {
        self.operations.clear();
        self.rebuild_operation_presentation();
    }
    pub fn invert_marks(&mut self) {
        let entries = self.entries().clone();
        for entry in entries
            .iter()
            .filter(|entry| !matches!(entry.kind, EntryKind::Parent))
        {
            if self.operations.remove(&entry.id).is_none() {
                self.operations.insert(
                    entry.id,
                    MarkedOperation {
                        path: entry.path.clone(),
                        resource_id: entry.resource_id,
                        mark: Mark::Selected,
                    },
                );
            }
        }
        self.rebuild_operation_presentation();
    }
    pub fn deletion_plan(&self) -> OperationPlan {
        OperationPlan::trash(
            self.operations
                .values()
                .filter(|operation| operation.mark == Mark::Delete)
                .map(|operation| operation.path.clone())
                .collect::<Vec<_>>()
                .into(),
        )
    }

    pub fn operation_targets(&self) -> Arc<[Arc<Path>]> {
        if self.operations.is_empty() {
            return self
                .selected()
                .filter(|entry| !matches!(entry.kind, EntryKind::Parent))
                .map(|entry| vec![entry.path.clone()].into())
                .unwrap_or_else(|| Arc::from([]));
        }
        self.operation_presentation.sorted_targets.clone()
    }

    pub fn clear_completed_operations(
        &mut self,
        paths: &[Arc<Path>],
        destinations: &[(Arc<Path>, Arc<Path>)],
    ) {
        let completed = paths
            .iter()
            .map(Arc::as_ref)
            .chain(
                destinations
                    .iter()
                    .map(|(_, destination)| destination.as_ref()),
            )
            .collect::<HashSet<_>>();
        self.operations
            .retain(|_, operation| !completed.contains(operation.path.as_ref()));
        self.rebuild_operation_presentation();
    }

    fn reconcile_operations(&mut self, directory: &Path, entries: &[FileEntry]) {
        let by_resource = entries
            .iter()
            .filter_map(|entry| entry.resource_id.map(|resource| (resource, entry)))
            .collect::<HashMap<_, _>>();
        let by_path = entries
            .iter()
            .map(|entry| (entry.path.as_ref(), entry))
            .collect::<HashMap<_, _>>();
        let old_operations = std::mem::take(&mut self.operations);
        for (old_id, operation) in old_operations {
            if operation.path.parent() != Some(directory) {
                self.operations.insert(old_id, operation);
                continue;
            }
            let entry = operation
                .resource_id
                .and_then(|resource| by_resource.get(&resource).copied())
                .or_else(|| by_path.get(operation.path.as_ref()).copied());
            if let Some(entry) = entry {
                self.operations.insert(
                    entry.id,
                    MarkedOperation {
                        path: entry.path.clone(),
                        resource_id: entry.resource_id,
                        mark: operation.mark,
                    },
                );
            }
        }
    }

    fn rebuild_operation_presentation(&mut self) {
        let visible_marks = self
            .operations
            .iter()
            .filter(|(id, _)| self.snapshot.rank_of(id).is_some())
            .map(|(id, operation)| (*id, operation.mark))
            .collect();
        let mut sorted_targets = self
            .operations
            .values()
            .map(|operation| operation.path.clone())
            .collect::<Vec<_>>();
        sorted_targets.sort();
        self.operation_presentation = OperationPresentation {
            visible_marks: Arc::new(visible_marks),
            sorted_targets: sorted_targets.into(),
        };
    }
}

fn nearest_rank(len: usize, rank: usize, affinity: AnchorAffinity) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match affinity {
        AnchorAffinity::Before => rank.saturating_sub(1).min(len - 1),
        AnchorAffinity::After => rank.saturating_add(1).min(len - 1),
        AnchorAffinity::Exact | AnchorAffinity::Nearest => rank.min(len - 1),
    })
}

fn sort_entries(entries: &mut [FileEntry], spec: SortSpec) {
    entries.sort_by(|left, right| {
        if matches!(left.kind, EntryKind::Parent) {
            return std::cmp::Ordering::Less;
        }
        if matches!(right.kind, EntryKind::Parent) {
            return std::cmp::Ordering::Greater;
        }
        let directory_order = if spec.directories_first {
            right.is_directory().cmp(&left.is_directory())
        } else {
            std::cmp::Ordering::Equal
        };
        let name_order = left
            .display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase());
        let order = directory_order.then(name_order);
        if spec.direction == SortDirection::Ascending {
            order
        } else {
            order.reverse()
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_manager::EntryMetadata;
    use std::time::SystemTime;

    fn entry(id: u64, path: &Path, kind: EntryKind) -> FileEntry {
        let name = path.file_name().unwrap_or_default().to_os_string();
        FileEntry {
            id: EntryId(id),
            resource_id: Some(FileResourceId(id)),
            path: Arc::from(path),
            os_name: Arc::new(name.clone()),
            display_name: Arc::from(name.to_string_lossy().as_ref()),
            kind,
            metadata: EntryMetadata {
                byte_len: None,
                modified: Some(SystemTime::UNIX_EPOCH),
                hidden: false,
                symlink_target: None,
            },
        }
    }

    fn navigate(
        session: &mut DiredSession,
        directory: &Path,
        entries: Vec<FileEntry>,
        cause: NavigationCause,
        selection: SelectionIntent<FileAnchor>,
    ) -> NavigationCommit {
        let load = session.begin_navigation(DiredSession::intent_for(
            directory.to_path_buf(),
            cause,
            selection,
        ));
        session
            .apply_scan(
                &load,
                ScanResult {
                    directory: directory.to_path_buf(),
                    entries,
                },
            )
            .unwrap()
    }

    #[test]
    fn restores_location_memory_and_explicit_up_target() {
        let parent = PathBuf::from("/tmp/test");
        let child = parent.join("child");
        let note = child.join("note.org");
        let mut session = DiredSession::empty(child.clone());
        navigate(
            &mut session,
            &child,
            vec![entry(9, &note, EntryKind::OrgFile)],
            NavigationCause::Enter,
            SelectionIntent::FirstSelectable,
        );
        let child_anchor = session.anchor_for_path(&child);
        navigate(
            &mut session,
            &parent,
            vec![entry(7, &child, EntryKind::Directory)],
            NavigationCause::Up,
            SelectionIntent::Explicit(child_anchor),
        );
        assert_eq!(session.cursor(), Some(EntryId(7)));
        navigate(
            &mut session,
            &child,
            vec![entry(9, &note, EntryKind::OrgFile)],
            NavigationCause::Enter,
            SelectionIntent::Restore,
        );
        assert_eq!(session.cursor(), Some(EntryId(9)));
    }

    #[test]
    fn stale_transaction_and_presentation_are_rejected() {
        let directory = PathBuf::from("/tmp/test");
        let mut session = DiredSession::empty(directory.clone());
        let stale = session.begin_navigation(DiredSession::intent_for(
            directory.clone(),
            NavigationCause::Enter,
            SelectionIntent::FirstSelectable,
        ));
        let current = session.begin_navigation(DiredSession::intent_for(
            directory.clone(),
            NavigationCause::Refresh,
            SelectionIntent::Preserve,
        ));
        assert!(
            session
                .apply_scan(
                    &stale,
                    ScanResult {
                        directory: directory.clone(),
                        entries: Vec::new()
                    }
                )
                .is_none()
        );
        let item_path = directory.join("item.org");
        let commit = session
            .apply_scan(
                &current,
                ScanResult {
                    directory,
                    entries: vec![entry(1, &item_path, EntryKind::OrgFile)],
                },
            )
            .unwrap();
        assert!(session.presentation_is_current(commit.transaction_id, commit.view_revision));
        session.move_cursor(1);
        assert!(!session.presentation_is_current(commit.transaction_id, commit.view_revision));
    }

    #[test]
    fn operation_marks_survive_navigation_and_are_not_view_memory() {
        let first = PathBuf::from("/tmp/a.org");
        let second_dir = PathBuf::from("/tmp/other");
        let mut session = DiredSession::empty(PathBuf::from("/tmp"));
        navigate(
            &mut session,
            Path::new("/tmp"),
            vec![entry(1, &first, EntryKind::OrgFile)],
            NavigationCause::Enter,
            SelectionIntent::FirstSelectable,
        );
        session.mark_selected(Mark::Delete);
        navigate(
            &mut session,
            &second_dir,
            Vec::new(),
            NavigationCause::Enter,
            SelectionIntent::FirstSelectable,
        );
        assert_eq!(
            session.deletion_plan(),
            OperationPlan::Trash {
                sources: vec![Arc::<Path>::from(first)].into()
            }
        );
    }

    #[test]
    fn history_restores_the_entry_snapshot_not_latest_location_memory() {
        let first_dir = PathBuf::from("/tmp/first");
        let second_dir = PathBuf::from("/tmp/second");
        let first_item = first_dir.join("first.org");
        let second_item = second_dir.join("second.org");
        let mut session = DiredSession::empty(first_dir.clone());
        navigate(
            &mut session,
            &first_dir,
            vec![entry(1, &first_item, EntryKind::OrgFile)],
            NavigationCause::Enter,
            SelectionIntent::FirstSelectable,
        );
        navigate(
            &mut session,
            &second_dir,
            vec![entry(2, &second_item, EntryKind::OrgFile)],
            NavigationCause::Enter,
            SelectionIntent::FirstSelectable,
        );

        let load = session.begin_history_navigation(false).unwrap();
        let commit = session
            .apply_scan(
                &load,
                ScanResult {
                    directory: first_dir,
                    entries: vec![entry(1, &first_item, EntryKind::OrgFile)],
                },
            )
            .unwrap();

        assert_eq!(session.cursor(), Some(EntryId(1)));
        assert_eq!(commit.presentation_rank, Some(0));
    }

    #[test]
    fn navigation_does_not_commit_directory_or_history_before_load_succeeds() {
        let first_dir = PathBuf::from("/tmp/first");
        let second_dir = PathBuf::from("/tmp/second");
        let mut session = DiredSession::empty(first_dir.clone());
        navigate(
            &mut session,
            &first_dir,
            Vec::new(),
            NavigationCause::Enter,
            SelectionIntent::FirstSelectable,
        );
        navigate(
            &mut session,
            &second_dir,
            Vec::new(),
            NavigationCause::Enter,
            SelectionIntent::FirstSelectable,
        );

        let pending = session.begin_history_navigation(false).unwrap();
        assert_eq!(pending.intent.target, first_dir);
        assert_eq!(session.directory(), second_dir);
        let retry = session.begin_history_navigation(false).unwrap();
        assert_eq!(retry.intent.target, first_dir);
    }

    #[test]
    fn refresh_preserves_viewport_anchor_and_offset() {
        let directory = PathBuf::from("/tmp/test");
        let first = directory.join("a.org");
        let second = directory.join("b.org");
        let mut session = DiredSession::empty(directory.clone());
        navigate(
            &mut session,
            &directory,
            vec![
                entry(1, &first, EntryKind::OrgFile),
                entry(2, &second, EntryKind::OrgFile),
            ],
            NavigationCause::Enter,
            SelectionIntent::FirstSelectable,
        );
        session.capture_viewport(1, 7.5);
        let load = session.begin_navigation(DiredSession::intent_for(
            directory.clone(),
            NavigationCause::Refresh,
            SelectionIntent::Preserve,
        ));
        let commit = session
            .apply_scan(
                &load,
                ScanResult {
                    directory,
                    entries: vec![
                        entry(1, &first, EntryKind::OrgFile),
                        entry(2, &second, EntryKind::OrgFile),
                    ],
                },
            )
            .unwrap();
        assert_eq!(commit.presentation_rank, Some(1));
        assert_eq!(commit.presentation_offset, 7.5);
    }

    #[test]
    fn resource_identity_survives_a_path_rename() {
        let directory = PathBuf::from("/tmp/test");
        let old_path = directory.join("old.org");
        let new_path = directory.join("new.org");
        let mut old_entry = entry(1, &old_path, EntryKind::OrgFile);
        old_entry.resource_id = Some(FileResourceId(42));
        let mut session = DiredSession::empty(directory.clone());
        navigate(
            &mut session,
            &directory,
            vec![old_entry],
            NavigationCause::Enter,
            SelectionIntent::FirstSelectable,
        );
        session.mark_selected(Mark::Delete);

        let mut renamed_entry = entry(2, &new_path, EntryKind::OrgFile);
        renamed_entry.resource_id = Some(FileResourceId(42));
        let load = session.begin_navigation(DiredSession::intent_for(
            directory.clone(),
            NavigationCause::Refresh,
            SelectionIntent::Preserve,
        ));
        session
            .apply_scan(
                &load,
                ScanResult {
                    directory,
                    entries: vec![renamed_entry],
                },
            )
            .unwrap();
        assert_eq!(session.cursor(), Some(EntryId(2)));
        assert_eq!(session.mark(EntryId(2)), Some(Mark::Delete));
        assert_eq!(
            session.deletion_plan(),
            OperationPlan::Trash {
                sources: vec![Arc::<Path>::from(new_path)].into()
            }
        );
    }

    #[test]
    fn operation_completion_clears_a_mark_reconciled_to_its_destination() {
        let directory = PathBuf::from("/tmp/test");
        let old_path = directory.join("old.org");
        let new_path = directory.join("new.org");
        let mut old_entry = entry(1, &old_path, EntryKind::OrgFile);
        old_entry.resource_id = Some(FileResourceId(42));
        let mut session = DiredSession::empty(directory.clone());
        navigate(
            &mut session,
            &directory,
            vec![old_entry],
            NavigationCause::Enter,
            SelectionIntent::FirstSelectable,
        );
        session.mark_selected(Mark::Selected);

        let mut renamed_entry = entry(2, &new_path, EntryKind::OrgFile);
        renamed_entry.resource_id = Some(FileResourceId(42));
        navigate(
            &mut session,
            &directory,
            vec![renamed_entry],
            NavigationCause::FileSystemDelta,
            SelectionIntent::Preserve,
        );
        session.clear_completed_operations(
            &[Arc::from(old_path.as_path())],
            &[(Arc::from(old_path.as_path()), Arc::from(new_path.as_path()))],
        );

        assert_eq!(session.marked_count(), 0);
        assert!(session.visible_marks().is_empty());
    }

    #[test]
    fn refresh_drops_marks_for_files_that_disappeared_from_the_current_directory() {
        let directory = PathBuf::from("/tmp/test");
        let removed = directory.join("removed.org");
        let mut session = DiredSession::empty(directory.clone());
        navigate(
            &mut session,
            &directory,
            vec![entry(1, &removed, EntryKind::OrgFile)],
            NavigationCause::Enter,
            SelectionIntent::FirstSelectable,
        );
        session.mark_selected(Mark::Delete);
        navigate(
            &mut session,
            &directory,
            Vec::new(),
            NavigationCause::FileSystemDelta,
            SelectionIntent::Preserve,
        );
        assert_eq!(session.marked_count(), 0);
        assert_eq!(session.deletion_plan().item_count(), 0);
    }

    #[test]
    fn operation_targets_use_marks_or_fall_back_to_the_cursor() {
        let directory = PathBuf::from("/tmp/test");
        let first = directory.join("a.org");
        let second = directory.join("b.org");
        let mut session = DiredSession::empty(directory.clone());
        navigate(
            &mut session,
            &directory,
            vec![
                entry(1, &first, EntryKind::OrgFile),
                entry(2, &second, EntryKind::OrgFile),
            ],
            NavigationCause::Enter,
            SelectionIntent::FirstSelectable,
        );

        assert_eq!(
            session.operation_targets().as_ref(),
            &[first.clone().into()]
        );
        session.mark_selected(Mark::Selected);
        assert_eq!(session.operation_targets().as_ref(), &[first.into()]);
        let marks = session.visible_marks();
        assert!(Arc::ptr_eq(&marks, &session.visible_marks()));
        let targets = session.operation_targets();
        assert!(Arc::ptr_eq(&targets, &session.operation_targets()));
        session.unmark_all();
        assert_eq!(session.operation_targets().as_ref(), &[second.into()]);
    }
}
