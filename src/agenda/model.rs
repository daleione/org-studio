use std::{ops::Range, path::PathBuf, sync::Arc};

use jiff::civil::{Date, Time};

use crate::{
    document::{ByteRange, DocumentId, FileStamp, Revision},
    org_semantic::{OrgTimestamp, TodoStateKind},
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct FileId(pub(crate) u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TaskKey {
    pub(crate) file: FileId,
    pub(crate) local: u32,
    pub(crate) shard_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct QueryId(pub(crate) u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct AgendaEntryKey(pub(crate) u32);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct AgendaPlacementKey(pub(crate) u32);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SourceVersion {
    Disk(Arc<FileStamp>),
    Live {
        document: DocumentId,
        revision: Revision,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceLocator {
    pub(crate) file: FileId,
    pub(crate) path: Arc<PathBuf>,
    pub(crate) version: SourceVersion,
    pub(crate) heading_range: ByteRange,
    pub(crate) title_range: ByteRange,
    pub(crate) anchor: Option<OrgAnchor>,
    pub(crate) fingerprint: HeadingFingerprint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OrgAnchor {
    pub(crate) kind: Arc<str>,
    pub(crate) value: Arc<str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HeadingFingerprint {
    pub(crate) level: u16,
    pub(crate) title: Arc<str>,
    pub(crate) parent_title: Option<Arc<str>>,
}

#[derive(Clone, Debug)]
pub(crate) struct TaskRecord {
    pub(crate) key: TaskKey,
    pub(crate) source: SourceLocator,
    pub(crate) level: u16,
    pub(crate) title: Arc<str>,
    pub(crate) todo: Arc<str>,
    pub(crate) todo_kind: TodoStateKind,
    pub(crate) priority: Option<char>,
    pub(crate) effective_tags: Arc<[Arc<str>]>,
    pub(crate) category: Option<Arc<str>>,
    pub(crate) properties: Arc<[(Arc<str>, Arc<str>)]>,
    pub(crate) parent: Option<TaskKey>,
    pub(crate) timestamps: Arc<[OrgTimestamp]>,
    pub(crate) allowed_todo_states: Arc<[Arc<str>]>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum AgendaDateKind {
    Plain,
    Scheduled,
    Deadline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AgendaTimestampIdentity {
    pub(crate) source_range: ByteRange,
    pub(crate) kind: AgendaDateKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgendaOccurrence {
    pub(crate) timestamp: AgendaTimestampIdentity,
    pub(crate) start_date: Date,
    pub(crate) start_time: Option<Time>,
    pub(crate) end_date: Option<Date>,
    pub(crate) end_time: Option<Time>,
}

#[derive(Clone, Debug)]
pub(crate) struct AgendaRow {
    pub(crate) task: TaskKey,
    pub(crate) source: SourceLocator,
    pub(crate) title: Arc<str>,
    pub(crate) todo: Arc<str>,
    pub(crate) priority: Option<char>,
    pub(crate) tags: Arc<[Arc<str>]>,
    pub(crate) category: Option<Arc<str>>,
    pub(crate) date: Option<Date>,
    pub(crate) time: Option<Time>,
    pub(crate) end_date: Option<Date>,
    pub(crate) end_time: Option<Time>,
    pub(crate) date_kind: Option<AgendaDateKind>,
    pub(crate) timestamp_range: Option<ByteRange>,
}

#[derive(Clone, Debug)]
pub(crate) struct AgendaEntry {
    pub(crate) key: AgendaEntryKey,
    pub(crate) row: AgendaRow,
    pub(crate) occurrence: Option<AgendaOccurrence>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AgendaEntryRef {
    pub(crate) query: QueryId,
    pub(crate) request_generation: u64,
    pub(crate) entry: AgendaEntryKey,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AgendaPlacementRef {
    pub(crate) entry: AgendaEntryRef,
    pub(crate) placement: AgendaPlacementKey,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgendaPlacement {
    pub(crate) key: AgendaPlacementKey,
    pub(crate) entry: AgendaEntryKey,
    pub(crate) date: Option<Date>,
    pub(crate) start_time: Option<Time>,
    pub(crate) end_time: Option<Time>,
    pub(crate) continues_before: bool,
    pub(crate) continues_after: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AgendaFacets {
    pub(crate) today: usize,
    pub(crate) next_seven_days: usize,
    pub(crate) overdue: usize,
    pub(crate) next: usize,
    pub(crate) waiting: usize,
    pub(crate) unscheduled: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgendaDiagnostic {
    pub(crate) path: Option<Arc<PathBuf>>,
    pub(crate) message: Arc<str>,
}

#[derive(Clone, Debug)]
pub(crate) struct AgendaResultSnapshot {
    pub(crate) query_id: Option<QueryId>,
    pub(crate) query: Arc<crate::agenda::AgendaQuery>,
    pub(crate) index_generation: u64,
    pub(crate) query_generation: u64,
    pub(crate) facets: AgendaFacets,
    pub(crate) diagnostics: Arc<[AgendaDiagnostic]>,
    pub(crate) entries: Arc<[AgendaEntry]>,
    pub(crate) placements: Arc<[AgendaPlacement]>,
    pub(crate) placement_groups: Arc<[AgendaPlacementGroup]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgendaPlacementGroup {
    pub(crate) date: Option<Date>,
    pub(crate) placements: Range<usize>,
}

impl AgendaResultSnapshot {
    pub(crate) fn placement_entry(&self, index: usize) -> Option<(&AgendaPlacement, &AgendaEntry)> {
        let placement = self.placements.get(index)?;
        let entry = self.entries.get(placement.entry.0 as usize)?;
        (entry.key == placement.entry).then_some((placement, entry))
    }

    pub(crate) fn entry_ref(&self, key: AgendaEntryKey) -> Option<AgendaEntryRef> {
        self.entries
            .get(key.0 as usize)
            .filter(|entry| entry.key == key)?;
        Some(AgendaEntryRef {
            query: self.query_id?,
            request_generation: self.query_generation,
            entry: key,
        })
    }

    pub(crate) fn resolve_entry(&self, reference: AgendaEntryRef) -> Option<&AgendaEntry> {
        if self.query_id != Some(reference.query)
            || self.query_generation != reference.request_generation
        {
            return None;
        }
        self.entries
            .get(reference.entry.0 as usize)
            .filter(|entry| entry.key == reference.entry)
    }

    pub(crate) fn placement_ref(&self, key: AgendaPlacementKey) -> Option<AgendaPlacementRef> {
        let placement = self.placements.get(key.0 as usize)?;
        let entry = self.entry_ref(placement.entry)?;
        (placement.key == key).then_some(AgendaPlacementRef {
            entry,
            placement: key,
        })
    }
}
