use super::{display_map, markdown, minimap, projection::ReadingProjection};
use crate::{
    document::{
        DocumentFormat, DocumentId, DocumentSession, PreparedReload, Revision, RevisionRange,
        SharedTextSnapshot, TextSnapshot, TextStatistics,
    },
    org_semantic::OrgAnalysisSnapshot,
    org_syntax::{BlockArena, BlockId},
};
use gpui::App;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

pub struct PreviewSnapshot {
    pub document_id: DocumentId,
    pub path: PathBuf,
    pub revision: Revision,
    pub text: SharedTextSnapshot,
    pub(crate) format: DocumentFormat,
    pub blocks: Arc<BlockArena>,
    pub(crate) semantic: Option<Arc<OrgAnalysisSnapshot>>,
    pub(crate) markdown_blocks: Arc<Vec<markdown::MarkdownBlock>>,
    pub(crate) outline_paths: Arc<Vec<Option<Arc<str>>>>,
    pub(crate) projection: Arc<ReadingProjection>,
    pub(crate) display_map: Option<Arc<display_map::PreviewDisplayMap>>,
    pub statistics: TextStatistics,
    pub metrics: LoadMetrics,
    pub(crate) update: DerivedUpdate,
}

impl PreviewSnapshot {
    /// Collect on demand when opening navigation, not during status repainting.
    pub(crate) fn outline_entries(&self) -> Vec<crate::document::OutlineEntry> {
        (0..self.projection.row_count())
            .filter_map(|index| {
                let visual = self.projection.rows.get(index)?;
                if !matches!(visual.kind, super::projection::VisualRowKind::Heading(_)) {
                    return None;
                }
                let row = self.projection.source_row(index)?;
                let level = match self.format {
                    DocumentFormat::Org => {
                        match self.blocks.nodes().get(row.block_id as usize)?.kind {
                            crate::org_syntax::BlockKind::Heading { level } => level,
                            _ => return None,
                        }
                    }
                    DocumentFormat::Markdown => {
                        match self.markdown_blocks.get(row.block_id as usize)?.kind {
                            markdown::MarkdownKind::Heading { level } => level,
                            _ => return None,
                        }
                    }
                };
                let title = self.text.copy_range(row.content.range);
                Some(crate::document::OutlineEntry {
                    title: title.trim().trim_start_matches(['*', '#']).trim().into(),
                    level,
                    line: self.text.line_of_byte(row.content.range.start) + 1,
                    source: row.content,
                })
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub(crate) enum DerivedUpdate {
    Full,
    Incremental {
        patch: super::projection::VisualPatch,
        reused_chunks: usize,
        total_chunks: usize,
    },
}

/// A newly opened mutable document together with its first coherent preview projection.
pub struct LoadedDocument {
    session: DocumentSession,
    preview: PreviewSnapshot,
}

pub(crate) enum WorkspaceLoadedDocument {
    Source(Box<DocumentSession>),
    Preview(Box<LoadedDocument>),
}

impl From<LoadedDocument> for WorkspaceLoadedDocument {
    fn from(document: LoadedDocument) -> Self {
        Self::Preview(Box::new(document))
    }
}

impl LoadedDocument {
    pub(crate) fn new(session: DocumentSession, preview: PreviewSnapshot) -> Result<Self, String> {
        if session.id() != preview.document_id || session.revision() != preview.revision {
            return Err("loaded session and preview refer to different document versions".into());
        }
        Ok(Self { session, preview })
    }

    pub(crate) fn into_parts(self) -> (DocumentSession, PreviewSnapshot) {
        (self.session, self.preview)
    }

    pub fn session(&self) -> &DocumentSession {
        &self.session
    }

    pub fn preview(&self) -> &PreviewSnapshot {
        &self.preview
    }

    pub fn into_preview(self) -> PreviewSnapshot {
        self.preview
    }
}

pub(crate) struct ReloadedDocument {
    prepared: PreparedReload,
    preview: PreviewSnapshot,
}

pub(crate) enum WorkspaceReloadedDocument {
    Source(PreparedReload),
    Preview(Box<ReloadedDocument>),
}

impl From<ReloadedDocument> for WorkspaceReloadedDocument {
    fn from(document: ReloadedDocument) -> Self {
        Self::Preview(Box::new(document))
    }
}

impl ReloadedDocument {
    pub(crate) fn new(prepared: PreparedReload, preview: PreviewSnapshot) -> Result<Self, String> {
        let snapshot = prepared.snapshot();
        if snapshot.document_id() != preview.document_id || snapshot.revision() != preview.revision
        {
            return Err("prepared reload and preview refer to different document versions".into());
        }
        Ok(Self { prepared, preview })
    }

    pub(crate) fn into_parts(self) -> (PreparedReload, PreviewSnapshot) {
        (self.prepared, self.preview)
    }
}

/// A small publication boundary from background derivation into the UI layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DerivedEvent {
    Published {
        document_id: DocumentId,
        revision: Revision,
    },
}

impl PreviewSnapshot {
    pub(crate) fn row_count(&self) -> usize {
        self.projection.row_count()
    }

    pub fn derived_event(&self) -> DerivedEvent {
        DerivedEvent::Published {
            document_id: self.document_id,
            revision: self.revision,
        }
    }
}

pub struct InitialDocumentLoad {
    pub(crate) path: PathBuf,
    pub(crate) started_at: Instant,
    pub(crate) receiver:
        async_channel::Receiver<Result<WorkspaceLoadedDocument, (PathBuf, String)>>,
}

/// Starts loading the command-line document before the native window is created. Small documents
/// are normally ready by the time GPUI constructs the view; large documents remain asynchronous.
/// This is deliberately independent of file size so both paths have identical parsing semantics.
pub fn preload_initial_document(path: PathBuf, cx: &App) -> InitialDocumentLoad {
    let started_at = Instant::now();
    let (sender, receiver) = async_channel::bounded(1);
    let load_path = path.clone();
    cx.background_executor()
        .spawn_with_priority(gpui::Priority::High, async move {
            if minimap::minimap_perf_enabled() {
                eprintln!("org_preview_initial_prefetch_start since_open_ms=0.000");
            }
            let result = super::loading::load_workspace_document(load_path, false);
            if minimap::minimap_perf_enabled() {
                eprintln!(
                    "org_preview_initial_prefetch_complete since_open_ms={:.3}",
                    started_at.elapsed().as_secs_f64() * 1000.0,
                );
            }
            let _ = sender.send(result).await;
        })
        .detach();
    InitialDocumentLoad {
        path,
        started_at,
        receiver,
    }
}

pub(crate) fn configured_minimap_visible(fallback: bool) -> bool {
    std::env::var("ORG_STUDIO_MINIMAP")
        .ok()
        .and_then(|value| match value.as_str() {
            "1" | "true" | "on" => Some(true),
            "0" | "false" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(fallback)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CodeRowRole {
    Open,
    Body,
    Close,
}

impl CodeRowRole {
    pub(crate) const fn is_boundary(self) -> bool {
        matches!(self, Self::Open | Self::Close)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PreviewRow {
    pub(crate) block_id: BlockId,
    pub(crate) content: RevisionRange,
    pub(crate) continuation: bool,
    pub(crate) blank: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LoadMetrics {
    pub bytes: u64,
    pub read: Duration,
    pub rope: Duration,
    pub parse: Duration,
    pub semantic: Duration,
    pub display_map: Duration,
    pub total: Duration,
    pub syntax_reparsed_bytes: u64,
    pub semantic_source_bytes: u64,
    pub full_syntax_fallback: bool,
}
