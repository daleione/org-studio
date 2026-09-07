//! Immutable, UI-free cross-file Agenda domain layer.

mod clock;
mod command;
mod config;
mod habit;
mod index;
mod model;
mod query;
mod repeat;
mod source;
mod task_rules;
mod text;
mod workflow;

pub(crate) use clock::ClockStore;
pub(crate) use command::{AgendaCommand, AgendaEditError, TimestampTarget, prepare_edit};
pub(crate) use config::{AgendaConfig, AgendaConfigStore};
pub(crate) use habit::{HabitStats, habit_stats};
pub(crate) use index::{AgendaIndex, AgendaIndexSnapshot, FileAgendaShard};
pub(crate) use model::{
    AgendaDateKind, AgendaDayGroup, AgendaDiagnostic, AgendaFacets, AgendaResultSnapshot,
    AgendaRow, FileId, HeadingFingerprint, OrgAnchor, SourceLocator, SourceVersion, TaskKey,
    TaskRecord,
};
pub(crate) use query::{AgendaQuery, BuiltinQuery, QueryEngine, task_matches_text};
pub(crate) use repeat::{
    RepeatCompletionAction, TodoTransition, next_repeat_date, transition_subtree,
};
pub(crate) use source::{DiscoveredSource, discover_sources, shard_from_disk, shard_from_live};
pub(crate) use task_rules::{compatibility_diagnostics, project_blocked_reason};
pub(crate) use text::format_agenda_text;
pub(crate) use workflow::{
    CaptureDraft, CaptureTemplate, InboxSession, ProjectSummary, RecoveryStage, RefileTarget,
    WorkflowError, append_capture, capture_text, cleanup_recovery_duplicate, cross_file_refile,
    derive_projects, load_receipt, refile_targets, resume_recovery,
};

#[cfg(test)]
mod feature_tests;
#[cfg(test)]
mod tests;
