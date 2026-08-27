use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    document::{RopeSnapshot, SharedTextSnapshot},
    org_syntax::{BlockArena, BlockId, BlockKind, parse},
};

use super::{
    DocumentFormat, LoadMetrics, PreviewDocument, build_markdown_table_styles, build_preview_rows,
    build_projection_snapshot, build_table_styles, markdown,
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

pub fn load_document(path: PathBuf) -> Result<PreviewDocument, (PathBuf, String)> {
    load_document_profiled(path)
}

pub fn load_document_profiled(path: PathBuf) -> Result<PreviewDocument, (PathBuf, String)> {
    load_document_profiled_impl(path, true)
}

pub fn load_document_profiled_without_display_map(
    path: PathBuf,
) -> Result<PreviewDocument, (PathBuf, String)> {
    load_document_profiled_impl(path, false)
}

fn load_document_profiled_impl(
    path: PathBuf,
    build_display_map: bool,
) -> Result<PreviewDocument, (PathBuf, String)> {
    let total_started = Instant::now();
    let read_started = Instant::now();
    let bytes = std::fs::read(&path).map_err(|error| (path.clone(), error.to_string()))?;
    let read = read_started.elapsed();
    let byte_count = bytes.len() as u64;
    let rope_started = Instant::now();
    let snapshot =
        RopeSnapshot::from_utf8(bytes).map_err(|error| (path.clone(), error.to_string()))?;
    let rope = rope_started.elapsed();
    let text: SharedTextSnapshot = Arc::new(snapshot);
    let parse_started = Instant::now();
    let format = match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("md" | "markdown") => DocumentFormat::Markdown,
        _ => DocumentFormat::Org,
    };
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

    let mut document = PreviewDocument {
        path,
        revision: text.revision(),
        text,
        format,
        blocks,
        markdown_blocks,
        projection,
        minimap: Arc::new(super::minimap::MinimapState::new()),
        layout: Arc::new(super::layout::LayoutState::new()),
        display_map: None,
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
    Ok(document)
}
