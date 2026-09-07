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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgendaDateKind {
    Plain,
    Scheduled,
    Deadline,
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
    pub(crate) index_generation: u64,
    pub(crate) query_generation: u64,
    pub(crate) rows: Arc<[AgendaRow]>,
    pub(crate) facets: AgendaFacets,
    pub(crate) diagnostics: Arc<[AgendaDiagnostic]>,
    pub(crate) groups: Arc<[AgendaDayGroup]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgendaDayGroup {
    pub(crate) date: Option<Date>,
    pub(crate) rows: Range<usize>,
}
