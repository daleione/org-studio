use super::{display_map, markdown, minimap, projection::PreviewProjectionSnapshot};
use crate::{
    document::{
        DocumentId, DocumentSession, PreparedReload, Revision, RevisionRange, SharedTextSnapshot,
        TextSnapshot, TextStatistics,
    },
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
    pub(in crate::preview) format: DocumentFormat,
    pub blocks: Arc<BlockArena>,
    pub(in crate::preview) markdown_blocks: Arc<Vec<markdown::MarkdownBlock>>,
    pub(in crate::preview) outline_paths: Arc<Vec<Option<Arc<str>>>>,
    pub(in crate::preview) projection: Arc<PreviewProjectionSnapshot>,
    pub(in crate::preview) display_map: Option<Arc<display_map::PreviewDisplayMap>>,
    pub statistics: TextStatistics,
    pub metrics: LoadMetrics,
}

/// A newly opened mutable document together with its first coherent preview projection.
pub struct LoadedDocument {
    session: DocumentSession,
    preview: PreviewSnapshot,
}

pub(crate) enum WorkspaceLoadedDocument {
    Source(DocumentSession),
    Preview(Box<LoadedDocument>),
}

impl From<LoadedDocument> for WorkspaceLoadedDocument {
    fn from(document: LoadedDocument) -> Self {
        Self::Preview(Box::new(document))
    }
}

impl LoadedDocument {
    pub(in crate::preview) fn new(
        session: DocumentSession,
        preview: PreviewSnapshot,
    ) -> Result<Self, String> {
        if session.id() != preview.document_id || session.revision() != preview.revision {
            return Err("loaded session and preview refer to different document versions".into());
        }
        Ok(Self { session, preview })
    }

    pub(in crate::preview) fn into_parts(self) -> (DocumentSession, PreviewSnapshot) {
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

pub(in crate::preview) struct ReloadedDocument {
    prepared: PreparedReload,
    preview: PreviewSnapshot,
}

pub(in crate::preview) enum WorkspaceReloadedDocument {
    Source(PreparedReload),
    Preview(Box<ReloadedDocument>),
}

impl From<ReloadedDocument> for WorkspaceReloadedDocument {
    fn from(document: ReloadedDocument) -> Self {
        Self::Preview(Box::new(document))
    }
}

impl ReloadedDocument {
    pub(in crate::preview) fn new(
        prepared: PreparedReload,
        preview: PreviewSnapshot,
    ) -> Result<Self, String> {
        let snapshot = prepared.snapshot();
        if snapshot.document_id() != preview.document_id || snapshot.revision() != preview.revision
        {
            return Err("prepared reload and preview refer to different document versions".into());
        }
        Ok(Self { prepared, preview })
    }

    pub(in crate::preview) fn into_parts(self) -> (PreparedReload, PreviewSnapshot) {
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
    pub fn derived_event(&self) -> DerivedEvent {
        DerivedEvent::Published {
            document_id: self.document_id,
            revision: self.revision,
        }
    }
}

pub struct InitialDocumentLoad {
    pub(in crate::preview) path: PathBuf,
    pub(in crate::preview) started_at: Instant,
    pub(in crate::preview) receiver:
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
            let result = super::loading::load_workspace_document(
                load_path,
                crate::app::DocumentMode::from_environment(
                    crate::settings::PreviewSettings::load().document_mode,
                ),
            );
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

pub(in crate::preview) fn configured_minimap_visible() -> bool {
    let preview_settings = crate::settings::PreviewSettings::load();
    std::env::var("ORG_STUDIO_MINIMAP")
        .ok()
        .and_then(|value| match value.as_str() {
            "1" | "true" | "on" => Some(true),
            "0" | "false" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(preview_settings.minimap_enabled)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DocumentFormat {
    Org,
    Markdown,
}

impl DocumentFormat {
    pub(crate) fn from_path(path: &std::path::Path) -> Self {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("md" | "markdown") => Self::Markdown,
            _ => Self::Org,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::preview) enum CodeRowRole {
    Open,
    Body,
    Close,
}

impl CodeRowRole {
    pub(in crate::preview) const fn is_boundary(self) -> bool {
        matches!(self, Self::Open | Self::Close)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PreviewRow {
    pub(super) block_id: BlockId,
    pub(super) content: RevisionRange,
    pub(super) continuation: bool,
    pub(super) show_line_number: bool,
    pub(super) blank: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LoadMetrics {
    pub bytes: u64,
    pub read: Duration,
    pub rope: Duration,
    pub parse: Duration,
    pub display_map: Duration,
    pub total: Duration,
}
