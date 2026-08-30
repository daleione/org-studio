use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    document::{
        DocumentSession, DocumentSnapshot, ReloadRequest, SharedTextSnapshot, TextSnapshot,
        TextStatistics,
    },
    org_syntax::{BlockArena, BlockId, BlockKind, parse},
};

use super::{
    DocumentFormat, LoadMetrics, LoadedDocument, PreviewSnapshot, ReloadedDocument,
    build_markdown_table_styles, build_preview_rows, build_projection_snapshot, build_table_styles,
    markdown,
};

pub(super) fn resolve_image_path(document_path: &Path, source: &str) -> PathBuf {
    let source = PathBuf::from(source);
    if source.is_absolute() {
        source
    } else {
        document_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(source)
    }
}

pub(super) fn build_image_sizes(
    document_path: &Path,
    blocks: &BlockArena,
) -> HashMap<BlockId, (u32, u32)> {
    blocks
        .nodes()
        .iter()
        .enumerate()
        .filter_map(|(block_id, block)| {
            let BlockKind::Image { path } = &block.kind else {
                return None;
            };
            image::image_dimensions(resolve_image_path(document_path, path))
                .ok()
                .filter(|&(width, height)| width > 0 && height > 0)
                .map(|size| (block_id as BlockId, size))
        })
        .collect()
}

pub(super) fn build_markdown_image_sizes(
    document_path: &Path,
    blocks: &[markdown::MarkdownBlock],
) -> HashMap<BlockId, (u32, u32)> {
    blocks
        .iter()
        .enumerate()
        .filter_map(|(block_id, block)| {
            let markdown::MarkdownKind::Image { path } = &block.kind else {
                return None;
            };
            image::image_dimensions(resolve_image_path(document_path, path))
                .ok()
                .filter(|&(width, height)| width > 0 && height > 0)
                .map(|size| (block_id as BlockId, size))
        })
        .collect()
}

pub(super) fn fitted_image_size(
    source_width: u32,
    source_height: u32,
    available_width: f32,
) -> (f32, f32) {
    let scale = (available_width.min(960.0) / source_width as f32)
        .min(480.0 / source_height as f32)
        .min(1.0);
    (source_width as f32 * scale, source_height as f32 * scale)
}

pub fn load_document(path: PathBuf) -> Result<LoadedDocument, (PathBuf, String)> {
    load_document_profiled(path)
}

pub(crate) fn load_workspace_document(
    path: PathBuf,
    mode: crate::app::DocumentMode,
) -> Result<super::WorkspaceLoadedDocument, (PathBuf, String)> {
    let path = crate::document::resolve_symlink_target(&path);
    match mode {
        crate::app::DocumentMode::Source => {
            let bytes = std::fs::read(&path).map_err(|error| (path.clone(), error.to_string()))?;
            let session = DocumentSession::from_utf8(path.clone(), bytes)
                .map_err(|error| (path, error.to_string()))?;
            let snapshot = session.snapshot();
            snapshot
                .byte_to_utf16(crate::document::ByteOffset(snapshot.len_bytes()))
                .expect("document end is a valid coordinate");
            Ok(super::WorkspaceLoadedDocument::Source(session))
        }
        crate::app::DocumentMode::Split | crate::app::DocumentMode::Preview => load_document(path)
            .map(Box::new)
            .map(super::WorkspaceLoadedDocument::Preview),
    }
}

pub fn load_document_profiled(path: PathBuf) -> Result<LoadedDocument, (PathBuf, String)> {
    load_document_profiled_impl(path, true)
}

pub fn load_document_profiled_without_display_map(
    path: PathBuf,
) -> Result<LoadedDocument, (PathBuf, String)> {
    load_document_profiled_impl(path, false)
}

fn load_document_profiled_impl(
    path: PathBuf,
    build_display_map: bool,
) -> Result<LoadedDocument, (PathBuf, String)> {
    let path = crate::document::resolve_symlink_target(&path);
    let total_started = Instant::now();
    let read_started = Instant::now();
    let bytes = std::fs::read(&path).map_err(|error| (path.clone(), error.to_string()))?;
    let read = read_started.elapsed();
    let byte_count = bytes.len() as u64;
    let rope_started = Instant::now();
    let session = DocumentSession::from_utf8(path.clone(), bytes)
        .map_err(|error| (path.clone(), error.to_string()))?;
    let snapshot = session.snapshot();
    let rope = rope_started.elapsed();
    let preview = build_preview(
        path.clone(),
        snapshot,
        byte_count,
        read,
        rope,
        total_started,
        build_display_map,
    );
    LoadedDocument::new(session, preview).map_err(|error| (path, error))
}

pub(in crate::preview) fn reload_document_profiled(
    request: ReloadRequest,
) -> Result<ReloadedDocument, (PathBuf, String)> {
    let total_started = Instant::now();
    let path = request.path().to_path_buf();
    let read_started = Instant::now();
    let bytes = std::fs::read(&path).map_err(|error| (path.clone(), error.to_string()))?;
    let read = read_started.elapsed();
    let byte_count = bytes.len() as u64;
    let rope_started = Instant::now();
    let prepared = request.prepare(bytes).map_err(|error| {
        (
            path.clone(),
            format!("reload preparation failed: {error:?}"),
        )
    })?;
    let snapshot = prepared.snapshot().clone();
    let rope = rope_started.elapsed();
    let preview = build_preview(
        path.clone(),
        snapshot,
        byte_count,
        read,
        rope,
        total_started,
        true,
    );
    ReloadedDocument::new(prepared, preview).map_err(|error| (path, error))
}

pub(in crate::preview) fn reload_workspace_document(
    request: ReloadRequest,
    mode: crate::app::DocumentMode,
) -> Result<super::WorkspaceReloadedDocument, (PathBuf, String)> {
    match mode {
        crate::app::DocumentMode::Source => {
            let path = request.path().to_path_buf();
            let bytes = std::fs::read(&path).map_err(|error| (path.clone(), error.to_string()))?;
            let prepared = request
                .prepare(bytes)
                .map_err(|error| (path, format!("reload preparation failed: {error:?}")))?;
            prepared
                .snapshot()
                .byte_to_utf16(crate::document::ByteOffset(prepared.snapshot().len_bytes()))
                .expect("document end is a valid coordinate");
            Ok(super::WorkspaceReloadedDocument::Source(prepared))
        }
        crate::app::DocumentMode::Split | crate::app::DocumentMode::Preview => {
            reload_document_profiled(request)
                .map(Box::new)
                .map(super::WorkspaceReloadedDocument::Preview)
        }
    }
}

fn build_preview(
    path: PathBuf,
    snapshot: DocumentSnapshot,
    byte_count: u64,
    read: Duration,
    rope: Duration,
    total_started: Instant,
    build_display_map: bool,
) -> PreviewSnapshot {
    let statistics = TextStatistics::from_snapshot(&snapshot);
    let text: SharedTextSnapshot = Arc::new(snapshot);
    let parse_started = Instant::now();
    let format = DocumentFormat::from_path(&path);
    let (blocks, markdown_blocks, rows) = match format {
        DocumentFormat::Org => {
            let blocks = Arc::new(parse(text.as_ref()));
            let rows = Arc::new(build_preview_rows(text.as_ref(), &blocks));
            (blocks, Arc::new(Vec::new()), rows)
        }
        DocumentFormat::Markdown => {
            let (markdown_blocks, rows) = markdown::parse_markdown(text.as_ref());
            (
                Arc::new(BlockArena::default()),
                Arc::new(markdown_blocks),
                Arc::new(rows),
            )
        }
    };
    let outline_paths =
        build_outline_paths(text.as_ref(), format, &blocks, &markdown_blocks, &rows);
    let parse = parse_started.elapsed();
    let table_projections = match format {
        DocumentFormat::Org => Arc::new(build_table_styles(text.as_ref(), &blocks)),
        DocumentFormat::Markdown => {
            Arc::new(build_markdown_table_styles(text.as_ref(), &markdown_blocks))
        }
    };
    let image_sizes = match format {
        DocumentFormat::Org => Arc::new(build_image_sizes(&path, &blocks)),
        DocumentFormat::Markdown => Arc::new(build_markdown_image_sizes(&path, &markdown_blocks)),
    };
    let projection = build_projection_snapshot(
        text.revision(),
        format,
        rows.clone(),
        &blocks,
        &markdown_blocks,
        &table_projections,
        &image_sizes,
    );

    let mut document = PreviewSnapshot {
        document_id: text.document_id(),
        path,
        revision: text.revision(),
        text,
        format,
        blocks,
        markdown_blocks,
        outline_paths,
        projection,
        display_map: None,
        statistics,
        metrics: LoadMetrics {
            bytes: byte_count,
            read,
            rope,
            parse,
            display_map: Duration::ZERO,
            total: Duration::ZERO,
        },
    };
    if build_display_map {
        let display_map_started = Instant::now();
        document.display_map = Some(Arc::new(super::display_map::build_display_map(&document)));
        document.metrics.display_map = display_map_started.elapsed();
    }
    document.metrics.total = total_started.elapsed();
    document
}

pub(in crate::preview) fn derive_preview(
    path: PathBuf,
    snapshot: DocumentSnapshot,
) -> PreviewSnapshot {
    let bytes = snapshot.len_bytes();
    build_preview(
        path,
        snapshot,
        bytes,
        Duration::ZERO,
        Duration::ZERO,
        Instant::now(),
        true,
    )
}

fn build_outline_paths(
    text: &dyn crate::document::TextSnapshot,
    format: DocumentFormat,
    blocks: &BlockArena,
    markdown_blocks: &[markdown::MarkdownBlock],
    rows: &[super::PreviewRow],
) -> Arc<Vec<Option<Arc<str>>>> {
    match format {
        DocumentFormat::Org => {
            let mut paths: Vec<Option<Arc<str>>> = Vec::with_capacity(blocks.nodes().len());
            for node in blocks.nodes() {
                let parent = node
                    .parent
                    .and_then(|parent| paths.get(parent as usize).cloned().flatten());
                let path = if matches!(node.kind, BlockKind::Heading { .. }) {
                    let title = text.copy_range(node.content);
                    let title = title.trim();
                    if title.is_empty() {
                        parent
                    } else if let Some(parent) = parent {
                        Some(Arc::from(format!("{parent} / {title}")))
                    } else {
                        Some(Arc::from(title))
                    }
                } else {
                    parent
                };
                paths.push(path);
            }
            Arc::new(paths)
        }
        DocumentFormat::Markdown => {
            let mut current = None;
            let mut paths = Vec::with_capacity(markdown_blocks.len());
            for (index, block) in markdown_blocks.iter().enumerate() {
                if matches!(block.kind, markdown::MarkdownKind::Heading { .. })
                    && let Some(row) = rows.get(index)
                {
                    let title = text.copy_range(row.content.range);
                    let title = title.trim();
                    if !title.is_empty() {
                        current = Some(Arc::from(title));
                    }
                }
                paths.push(current.clone());
            }
            Arc::new(paths)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentSnapshot;

    #[test]
    fn markdown_outline_lookup_has_no_scrollback_limit() {
        let source = format!("# Long-lived heading\n{}", "body\n".repeat(700));
        let text = DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
        let (markdown_blocks, rows) = markdown::parse_markdown(&text);
        let paths = build_outline_paths(
            &text,
            DocumentFormat::Markdown,
            &BlockArena::default(),
            &markdown_blocks,
            &rows,
        );
        assert_eq!(
            paths.last().and_then(Option::as_deref),
            Some("Long-lived heading")
        );
    }

    #[test]
    fn org_outline_lookup_preserves_the_heading_path() {
        let text =
            DocumentSnapshot::from_utf8(b"* Parent\nbody\n** Child\nbody\n".to_vec()).unwrap();
        let blocks = parse(&text);
        let rows = build_preview_rows(&text, &blocks);
        let paths = build_outline_paths(&text, DocumentFormat::Org, &blocks, &[], &rows);
        let last_block = rows.last().unwrap().block_id as usize;
        assert_eq!(
            paths.get(last_block).and_then(Option::as_deref),
            Some("Parent / Child")
        );
    }
}
