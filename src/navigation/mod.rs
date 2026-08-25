use std::{
    collections::{HashMap, VecDeque},
    hash::Hash,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

macro_rules! id_type {
    ($name:ident, $inner:ty) => {
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub $inner);
    };
}

id_type!(TransactionId, u64);
id_type!(ContentRevision, u64);
id_type!(ViewRevision, u64);
id_type!(HistoryEntryId, u64);
id_type!(PaneId, u64);
id_type!(SnapshotSchemaVersion, u32);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ContentKey {
    Directory(Arc<str>),
    Search(Arc<str>),
    Outline(Arc<str>),
    RecentFiles(Arc<str>),
    CommandCollection(Arc<str>),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ViewStateKey<P> {
    pub content: ContentKey,
    pub pane: PaneId,
    pub presentation: P,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AnchorAffinity {
    Exact,
    Before,
    After,
    #[default]
    Nearest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemAnchor<Primary, Fallback> {
    pub primary: Option<Primary>,
    pub fallback: Fallback,
    pub previous_rank: Option<usize>,
    pub affinity: AnchorAffinity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatchConfidence {
    Exact,
    StrongFallback,
    WeakFallback,
    Positional,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconcileReason {
    PrimaryId,
    FallbackIdentity,
    PreviousRank,
    DefaultItem,
    Missing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciledAnchor<I> {
    pub target: Option<I>,
    pub confidence: MatchConfidence,
    pub reason: ReconcileReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationCause {
    Enter,
    Up,
    Back,
    Forward,
    Refresh,
    ExternalOpen,
    RestoreSession,
    SortChanged,
    FilterChanged,
    FileSystemDelta,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionIntent<A> {
    Explicit(A),
    Restore,
    FirstSelectable,
    Preserve,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewportIntent<A> {
    Restore,
    EnsureVisible(A),
    Center(A),
    Preserve,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryIntent {
    Push,
    Replace,
    Traverse(HistoryEntryId),
    Ignore,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheIntent {
    PreferCache,
    Reload,
    CacheOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavigationIntent<L, A> {
    pub target: L,
    pub cause: NavigationCause,
    pub selection: SelectionIntent<A>,
    pub viewport: ViewportIntent<A>,
    pub history: HistoryIntent,
    pub cache: CacheIntent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum AnchorAlignment {
    #[default]
    Minimal,
    Start,
    Center,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewportAnchor<A> {
    pub item: A,
    pub offset_from_viewport_start: f32,
    pub alignment: AnchorAlignment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FocusTarget {
    List,
    Item,
    Auxiliary(Arc<str>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct InteractionState<A> {
    pub focus: FocusTarget,
    pub primary_selection: Option<A>,
    pub secondary_selection: Vec<A>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewSnapshot<A, P = ()> {
    pub interaction: InteractionState<A>,
    pub viewport: Option<ViewportAnchor<A>>,
    pub presentation: P,
    pub view_revision: ViewRevision,
}

#[derive(Clone, Debug)]
pub struct ContentSnapshot<I, Id> {
    pub revision: ContentRevision,
    ordered: Arc<[I]>,
    rank_by_id: HashMap<Id, usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuplicateItemId<Id>(pub Id);

impl<I, Id> ContentSnapshot<I, Id>
where
    Id: Clone + Eq + Hash,
{
    pub fn try_new(
        revision: ContentRevision,
        items: Vec<I>,
        id: impl Fn(&I) -> Id,
    ) -> Result<Self, DuplicateItemId<Id>> {
        let mut rank_by_id = HashMap::with_capacity(items.len());
        for (rank, item) in items.iter().enumerate() {
            let item_id = id(item);
            if rank_by_id.insert(item_id.clone(), rank).is_some() {
                return Err(DuplicateItemId(item_id));
            }
        }
        Ok(Self {
            revision,
            ordered: items.into(),
            rank_by_id,
        })
    }

    pub fn items(&self) -> &Arc<[I]> {
        &self.ordered
    }
    pub fn len(&self) -> usize {
        self.ordered.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ordered.is_empty()
    }
    pub fn rank_of(&self, id: &Id) -> Option<usize> {
        self.rank_by_id.get(id).copied()
    }
    pub fn item_at(&self, rank: usize) -> Option<&I> {
        self.ordered.get(rank)
    }
}

#[derive(Clone, Debug)]
pub struct CancellationToken(Arc<AtomicBool>);

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}
impl CancellationToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug)]
pub struct NavigationTransaction {
    pub id: TransactionId,
    pub cancellation: CancellationToken,
}

#[derive(Debug, Default)]
pub struct TransactionManager {
    next: u64,
    active: Option<NavigationTransaction>,
}

impl TransactionManager {
    pub fn begin(&mut self) -> NavigationTransaction {
        if let Some(active) = self.active.take() {
            active.cancellation.cancel();
        }
        self.next = self.next.wrapping_add(1);
        let transaction = NavigationTransaction {
            id: TransactionId(self.next),
            cancellation: CancellationToken::new(),
        };
        self.active = Some(transaction.clone());
        transaction
    }

    pub fn accepts(&self, id: TransactionId) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.id == id && !active.cancellation.is_cancelled())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEntry<L, S> {
    pub id: HistoryEntryId,
    pub parent: Option<HistoryEntryId>,
    pub location: L,
    pub snapshot: S,
    pub children: Vec<HistoryEntryId>,
}

#[derive(Clone, Debug)]
pub struct HistoryTimeline<L, S> {
    entries: HashMap<HistoryEntryId, HistoryEntry<L, S>>,
    current: Option<HistoryEntryId>,
    forward_choice: HashMap<HistoryEntryId, HistoryEntryId>,
    next: u64,
}

impl<L, S> Default for HistoryTimeline<L, S> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            current: None,
            forward_choice: HashMap::new(),
            next: 0,
        }
    }
}

impl<L: Clone, S: Clone> HistoryTimeline<L, S> {
    pub fn push(&mut self, location: L, snapshot: S) -> HistoryEntryId {
        self.next = self.next.wrapping_add(1);
        let id = HistoryEntryId(self.next);
        let parent = self.current;
        self.entries.insert(
            id,
            HistoryEntry {
                id,
                parent,
                location,
                snapshot,
                children: Vec::new(),
            },
        );
        if let Some(parent) = parent {
            self.entries
                .get_mut(&parent)
                .expect("history parent exists")
                .children
                .push(id);
            self.forward_choice.insert(parent, id);
        }
        self.current = Some(id);
        id
    }

    pub fn current(&self) -> Option<&HistoryEntry<L, S>> {
        self.current.and_then(|id| self.entries.get(&id))
    }
    pub fn peek_back(&self) -> Option<&HistoryEntry<L, S>> {
        self.current()
            .and_then(|entry| entry.parent)
            .and_then(|id| self.entries.get(&id))
    }
    pub fn peek_forward(&self) -> Option<&HistoryEntry<L, S>> {
        let current = self.current?;
        self.forward_choice
            .get(&current)
            .and_then(|id| self.entries.get(id))
    }
    pub fn traverse_to(&mut self, id: HistoryEntryId) -> bool {
        if !self.entries.contains_key(&id) {
            return false;
        }
        self.current = Some(id);
        true
    }
    pub fn update_current_snapshot(&mut self, snapshot: S) -> bool {
        let Some(current) = self.current else {
            return false;
        };
        let Some(entry) = self.entries.get_mut(&current) else {
            return false;
        };
        entry.snapshot = snapshot;
        true
    }
    pub fn back(&mut self) -> Option<&HistoryEntry<L, S>> {
        let parent = self.current().and_then(|entry| entry.parent)?;
        self.current = Some(parent);
        self.entries.get(&parent)
    }
    pub fn forward(&mut self) -> Option<&HistoryEntry<L, S>> {
        let current = self.current?;
        let next = self.forward_choice.get(&current).copied()?;
        self.current = Some(next);
        self.entries.get(&next)
    }
    pub fn entry(&self, id: HistoryEntryId) -> Option<&HistoryEntry<L, S>> {
        self.entries.get(&id)
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Clone, Debug)]
pub struct LocationMemory<K, S> {
    capacity: usize,
    values: HashMap<K, S>,
    recency: VecDeque<K>,
}

impl<K: Clone + Eq + Hash, S> LocationMemory<K, S> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            values: HashMap::new(),
            recency: VecDeque::new(),
        }
    }
    pub fn get(&mut self, key: &K) -> Option<&S> {
        if self.values.contains_key(key) {
            self.recency.retain(|candidate| candidate != key);
            self.recency.push_back(key.clone());
        }
        self.values.get(key)
    }
    pub fn insert(&mut self, key: K, value: S) {
        self.recency.retain(|candidate| candidate != &key);
        self.recency.push_back(key.clone());
        self.values.insert(key, value);
        while self.values.len() > self.capacity {
            if let Some(oldest) = self.recency.pop_front() {
                self.values.remove(&oldest);
            }
        }
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NavigationConstraints<A> {
    pub required_selection: Option<A>,
    pub preferred_viewport: Option<ViewportAnchor<A>>,
    pub must_be_visible: Vec<A>,
    pub user_scroll_lock: bool,
    pub focus_requirement: Option<FocusTarget>,
}

pub fn resolve_selection<A: Clone>(
    explicit: Option<A>,
    history: Option<A>,
    pane_memory: Option<A>,
    workspace_memory: Option<A>,
    fallback: Option<A>,
) -> Option<A> {
    explicit
        .or(history)
        .or(pane_memory)
        .or(workspace_memory)
        .or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_transaction_cancels_and_rejects_the_previous_one() {
        let mut manager = TransactionManager::default();
        let first = manager.begin();
        let second = manager.begin();
        assert!(first.cancellation.is_cancelled());
        assert!(!manager.accepts(first.id));
        assert!(manager.accepts(second.id));
    }

    #[test]
    fn branching_history_preserves_old_forward_paths() {
        let mut history = HistoryTimeline::default();
        let root = history.push("root", 0);
        let first = history.push("first", 1);
        history.back();
        let second = history.push("second", 2);
        assert_eq!(history.entry(root).unwrap().children, vec![first, second]);
        history.back();
        assert_eq!(history.forward().unwrap().id, second);
    }

    #[test]
    fn current_history_snapshot_is_captured_before_leaving() {
        let mut history = HistoryTimeline::default();
        history.push("root", 1);
        assert!(history.update_current_snapshot(9));
        assert_eq!(history.current().unwrap().snapshot, 9);
    }

    #[test]
    fn location_memory_is_bounded() {
        let mut memory = LocationMemory::new(2);
        memory.insert("a", 1);
        memory.insert("b", 2);
        memory.insert("c", 3);
        assert_eq!(memory.len(), 2);
        assert!(memory.get(&"a").is_none());
    }

    #[test]
    fn content_snapshot_resolves_ranks_in_constant_time() {
        let snapshot =
            ContentSnapshot::try_new(ContentRevision(1), vec![(7, "a"), (9, "b")], |item| item.0)
                .unwrap();
        assert_eq!(snapshot.rank_of(&9), Some(1));
        assert_eq!(snapshot.item_at(1), Some(&(9, "b")));
    }

    #[test]
    fn content_snapshot_rejects_duplicate_item_ids() {
        let snapshot =
            ContentSnapshot::try_new(ContentRevision(1), vec![(7, "a"), (7, "b")], |item| item.0);
        assert_eq!(snapshot.unwrap_err(), DuplicateItemId(7));
    }

    #[test]
    fn location_memory_reads_refresh_lru_recency() {
        let mut memory = LocationMemory::new(2);
        memory.insert("a", 1);
        memory.insert("b", 2);
        assert_eq!(memory.get(&"a"), Some(&1));
        memory.insert("c", 3);
        assert!(memory.get(&"a").is_some());
        assert!(memory.get(&"b").is_none());
    }

    #[test]
    fn selection_precedence_is_deterministic() {
        assert_eq!(
            resolve_selection(Some(1), Some(2), Some(3), Some(4), Some(5)),
            Some(1)
        );
        assert_eq!(
            resolve_selection(None, Some(2), Some(3), Some(4), Some(5)),
            Some(2)
        );
    }
}
