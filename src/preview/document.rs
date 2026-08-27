use super::{
    display_map, loading::load_document, markdown, minimap, projection::PreviewProjectionSnapshot,
};
use crate::{
    document::{Revision, RevisionRange, SharedTextSnapshot},
    org_syntax::{BlockArena, BlockId},
};
use gpui::{App, BackgroundExecutor};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

pub struct PreviewDocument {
    pub path: PathBuf,
    pub revision: Revision,
    pub text: SharedTextSnapshot,
    pub(in crate::preview) format: DocumentFormat,
    pub blocks: Arc<BlockArena>,
    pub(in crate::preview) markdown_blocks: Arc<Vec<markdown::MarkdownBlock>>,
    pub(in crate::preview) projection: Arc<PreviewProjectionSnapshot>,
    pub(in crate::preview) minimap: Arc<minimap::MinimapState>,
    pub(in crate::preview) display_map: Option<Arc<display_map::PreviewDisplayMap>>,
    pub metrics: LoadMetrics,
}

pub struct InitialDocumentLoad {
    pub(in crate::preview) path: PathBuf,
    pub(in crate::preview) started_at: Instant,
    pub(in crate::preview) minimap_prewarm_scheduled: bool,
    pub(in crate::preview) receiver:
        async_channel::Receiver<Result<PreviewDocument, (PathBuf, String)>>,
}

/// Starts loading the command-line document before the native window is created. Small documents
/// are normally ready by the time GPUI constructs the view; large documents remain asynchronous.
/// This is deliberately independent of file size so both paths have identical parsing semantics.
pub fn preload_initial_document(path: PathBuf, cx: &App) -> InitialDocumentLoad {
    let started_at = Instant::now();
    let (sender, receiver) = async_channel::bounded(1);
    let load_path = path.clone();
    let minimap_prewarm_scheduled = configured_minimap_visible();
    let prewarm_executor = cx.background_executor().clone();
    if minimap_prewarm_scheduled {
        cx.background_executor()
            .spawn(async { minimap::prewarm_text_rasterizer() })
            .detach();
    }
    cx.background_executor()
        .spawn_with_priority(gpui::Priority::High, async move {
            if minimap::minimap_perf_enabled() {
                eprintln!("org_preview_initial_prefetch_start since_open_ms=0.000");
            }
            let result = load_document(load_path);
            schedule_document_prewarm(minimap_prewarm_scheduled, &result, &prewarm_executor);
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
        minimap_prewarm_scheduled,
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

pub(in crate::preview) fn schedule_document_prewarm(
    enabled: bool,
    result: &Result<PreviewDocument, (PathBuf, String)>,
    executor: &BackgroundExecutor,
) {
    if enabled
        && let Ok(document) = result
        && let Some(model) = document.display_map.clone()
    {
        executor
            .spawn(async move { minimap::prewarm_document_text(model) })
            .detach();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::preview) enum DocumentFormat {
    Org,
    Markdown,
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
