use std::{collections::HashMap, sync::Arc};

use crate::{
    document::{ByteRange, DocumentId, Revision},
    org_syntax::SyntaxId,
};

use super::timestamp::OrgTimestamp;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TodoStateKind {
    Open,
    Done,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TodoState {
    pub(crate) keyword: Arc<str>,
    pub(crate) kind: TodoStateKind,
    pub(crate) fast_key: Option<char>,
    pub(crate) log_spec: Option<Arc<str>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TodoSequence {
    pub(crate) states: Arc<[TodoState]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OrgFileConfig {
    todo_index: HashMap<Arc<str>, TodoState>,
    todo_keywords: Arc<[Arc<str>]>,
    pub(crate) file_tags: Arc<[Arc<str>]>,
    pub(crate) category: Option<Arc<str>>,
    pub(crate) property_defaults: Arc<[(Arc<str>, Arc<str>)]>,
    pub(crate) archive_location: Option<Arc<str>>,
}

impl OrgFileConfig {
    pub(crate) fn new(
        todo_sequences: Vec<TodoSequence>,
        file_tags: Vec<Arc<str>>,
        category: Option<Arc<str>>,
        property_defaults: Vec<(Arc<str>, Arc<str>)>,
        archive_location: Option<Arc<str>>,
    ) -> Self {
        let todo_keywords = todo_sequences
            .iter()
            .flat_map(|sequence| sequence.states.iter().map(|state| state.keyword.clone()))
            .collect::<Vec<_>>()
            .into();
        let todo_index = todo_sequences
            .iter()
            .flat_map(|sequence| sequence.states.iter().cloned())
            .map(|state| (Arc::clone(&state.keyword), state))
            .collect();
        Self {
            todo_index,
            todo_keywords,
            file_tags: file_tags.into(),
            category,
            property_defaults: property_defaults.into(),
            archive_location,
        }
    }

    pub(crate) fn todo_state(&self, keyword: &str) -> Option<&TodoState> {
        self.todo_index.get(keyword)
    }

    pub(crate) fn todo_keywords(&self) -> Arc<[Arc<str>]> {
        self.todo_keywords.clone()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OrgHeading {
    pub(crate) syntax_id: SyntaxId,
    pub(crate) source: ByteRange,
    pub(crate) content: ByteRange,
    pub(crate) level: u16,
    pub(crate) parent: Option<SyntaxId>,
    pub(crate) todo: Option<TodoState>,
    pub(crate) priority: Option<char>,
    pub(crate) title: Arc<str>,
    pub(crate) tags: Arc<[Arc<str>]>,
    pub(crate) effective_tags: Arc<[Arc<str>]>,
    pub(crate) timestamps: Arc<[OrgTimestamp]>,
    pub(crate) properties: Arc<[(Arc<str>, Arc<str>)]>,
}

#[derive(Clone)]
pub(crate) struct OrgAnalysisSnapshot {
    pub(crate) document_id: DocumentId,
    pub(crate) revision: Revision,
    pub(crate) config: Arc<OrgFileConfig>,
    pub(crate) headings: Arc<[OrgHeading]>,
    pub(crate) metrics: SemanticMetrics,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SemanticMetrics {
    pub(crate) examined_headings: usize,
    pub(crate) extracted_headings: usize,
    pub(crate) reused_headings: usize,
    pub(crate) semantic_source_bytes: u64,
    pub(crate) full_config_rebuild: bool,
}
