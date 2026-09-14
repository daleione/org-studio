//! Shared Org semantics derived from the lossless document and syntax layers.
//!
//! This module deliberately has no GPUI, editor, preview, or product-shell
//! dependency. Agenda, the editor, and reading mode must consume the same
//! interpretation of file-local Org configuration and headings.

mod extract;
mod file_config;
mod model;
mod timestamp;
pub(crate) mod timestamp_edit;

pub(crate) use extract::{analyze, analyze_incremental};
pub(crate) use file_config::parse_todo_directive;
pub(crate) use model::{OrgAnalysisSnapshot, OrgFileConfig, OrgHeading, TodoStateKind};
#[allow(unused_imports)]
pub(crate) use timestamp::{
    OrgTimestamp, Repeater, RepeaterMode, TimeUnit, TimestampKind, WarningPeriod,
};

#[cfg(test)]
mod tests;
