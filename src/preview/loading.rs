use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    document::{
        DocumentSession, DocumentSnapshot, ReloadRequest, RevisionDelta, SharedTextSnapshot,
        TextSnapshot, TextStatistics,
    },
    org_semantic,
    org_syntax::{BlockArena, BlockId, BlockKind, SyntaxPatch, parse, parse_incremental},
};

use super::{
    DerivedUpdate, DocumentFormat, LoadMetrics, LoadedDocument, PreviewSnapshot, ReloadedDocument,
    build_markdown_diagrams, build_markdown_table_styles, build_preview_rows,
    build_projection_snapshot, build_table_styles, markdown,
    projection::{ReadingProjectionResources, build_visual_rows},
    rows::build_preview_rows_in_range,
};

const TABLE_DEPENDENCY: u8 = 1 << 0;
const CODE_DEPENDENCY: u8 = 1 << 1;

pub(crate) fn resolve_image_path(document_path: &Path, source: &str) -> PathBuf {
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

pub(crate) fn build_image_sizes(
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
            image_dimensions(&resolve_image_path(document_path, path))
                .ok()
                .filter(|&(width, height)| width > 0 && height > 0)
                .map(|size| (block_id as BlockId, size))
        })
        .collect()
}

pub(crate) fn build_markdown_image_sizes(
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
            image_dimensions(&resolve_image_path(document_path, path))
                .ok()
                .filter(|&(width, height)| width > 0 && height > 0)
                .map(|size| (block_id as BlockId, size))
        })
        .collect()
}

pub(crate) fn image_dimensions(path: &Path) -> Result<(u32, u32), String> {
    let is_svg = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
    if is_svg {
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        svg_dimensions(&bytes).ok_or_else(|| "SVG has no usable dimensions".to_owned())
    } else {
        image::image_dimensions(path).map_err(|error| error.to_string())
    }
}

fn svg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let source = std::str::from_utf8(bytes).ok()?;
    let document = roxmltree::Document::parse(source).ok()?;
    let svg = document.root_element();
    if !svg.tag_name().name().eq_ignore_ascii_case("svg") {
        return None;
    }
    if let Some(view_box) = svg
        .attribute("viewBox")
        .or_else(|| svg.attribute("viewbox"))
    {
        let values = view_box
            .split(|character: char| character.is_ascii_whitespace() || character == ',')
            .filter(|value| !value.is_empty())
            .map(str::parse::<f64>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        if values.len() == 4 {
            return positive_dimensions(values[2].abs(), values[3].abs());
        }
    }
    positive_dimensions(
        svg_length(svg.attribute("width")?)?,
        svg_length(svg.attribute("height")?)?,
    )
}

fn svg_length(value: &str) -> Option<f64> {
    let value = value.trim();
    let split = value
        .find(|character: char| !matches!(character, '0'..='9' | '.' | '+' | '-' | 'e' | 'E'))
        .unwrap_or(value.len());
    let number = value[..split].parse::<f64>().ok()?;
    let scale = match value[split..].trim().to_ascii_lowercase().as_str() {
        "" | "px" => 1.0,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        "q" => 96.0 / 101.6,
        _ => return None,
    };
    Some(number * scale)
}

fn positive_dimensions(width: f64, height: f64) -> Option<(u32, u32)> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some((
        width.round().clamp(1.0, f64::from(u32::MAX)) as u32,
        height.round().clamp(1.0, f64::from(u32::MAX)) as u32,
    ))
}

pub(crate) fn fitted_image_size(
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
    build_preview: bool,
) -> Result<super::WorkspaceLoadedDocument, (PathBuf, String)> {
    let path = crate::document::resolve_symlink_target(&path);
    if !build_preview {
        let bytes = std::fs::read(&path).map_err(|error| (path.clone(), error.to_string()))?;
        let session = DocumentSession::from_utf8(path.clone(), bytes)
            .map_err(|error| (path, error.to_string()))?;
        let snapshot = session.snapshot();
        snapshot
            .byte_to_utf16(crate::document::ByteOffset(snapshot.len_bytes()))
            .expect("document end is a valid coordinate");
        Ok(super::WorkspaceLoadedDocument::Source(Box::new(session)))
    } else {
        load_document(path)
            .map(Box::new)
            .map(super::WorkspaceLoadedDocument::Preview)
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
        None,
        PreviewBuild {
            byte_count,
            read,
            rope,
            total_started,
            build_display_map,
            syntax_patch: None,
            full_syntax_fallback: false,
        },
    );
    LoadedDocument::new(session, preview).map_err(|error| (path, error))
}

pub(crate) fn reload_document_profiled(
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
        None,
        PreviewBuild {
            byte_count,
            read,
            rope,
            total_started,
            build_display_map: true,
            syntax_patch: None,
            full_syntax_fallback: false,
        },
    );
    ReloadedDocument::new(prepared, preview).map_err(|error| (path, error))
}

pub(crate) fn reload_workspace_document(
    request: ReloadRequest,
    build_preview: bool,
) -> Result<super::WorkspaceReloadedDocument, (PathBuf, String)> {
    if !build_preview {
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
    } else {
        reload_document_profiled(request)
            .map(Box::new)
            .map(super::WorkspaceReloadedDocument::Preview)
    }
}

struct PreviewBuild<'a> {
    byte_count: u64,
    read: Duration,
    rope: Duration,
    total_started: Instant,
    build_display_map: bool,
    syntax_patch: Option<&'a SyntaxPatch>,
    full_syntax_fallback: bool,
}

struct IncrementalPreview {
    path: PathBuf,
    text: SharedTextSnapshot,
    format: DocumentFormat,
    blocks: Arc<BlockArena>,
    semantic: Option<Arc<org_semantic::OrgAnalysisSnapshot>>,
    markdown_blocks: Arc<Vec<markdown::MarkdownBlock>>,
    outline_paths: Arc<Vec<Option<Arc<str>>>>,
    projection: Arc<super::projection::ReadingProjection>,
    syntax_elapsed: Duration,
    semantic_elapsed: Duration,
    semantic_source_bytes: u64,
    syntax_reparsed_bytes: u64,
    patch: super::projection::VisualPatch,
    reused_chunks: usize,
    total_chunks: usize,
}

fn finish_incremental_preview(
    parts: IncrementalPreview,
    previous: &PreviewSnapshot,
) -> Option<PreviewSnapshot> {
    let statistics = TextStatistics::from_snapshot(parts.text.as_ref());
    let mut document = PreviewSnapshot {
        document_id: parts.text.document_id(),
        path: parts.path,
        revision: parts.text.revision(),
        text: parts.text,
        format: parts.format,
        blocks: parts.blocks,
        semantic: parts.semantic,
        markdown_blocks: parts.markdown_blocks,
        outline_paths: parts.outline_paths,
        projection: parts.projection,
        display_map: None,
        statistics,
        metrics: LoadMetrics {
            bytes: statistics.bytes,
            read: Duration::ZERO,
            rope: Duration::ZERO,
            parse: parts.syntax_elapsed,
            semantic: parts.semantic_elapsed,
            display_map: Duration::ZERO,
            total: Duration::ZERO,
            syntax_reparsed_bytes: parts.syntax_reparsed_bytes,
            semantic_source_bytes: parts.semantic_source_bytes,
            full_syntax_fallback: false,
        },
        update: DerivedUpdate::Incremental {
            patch: parts.patch,
            reused_chunks: parts.reused_chunks,
            total_chunks: parts.total_chunks,
        },
    };
    let display_map_started = Instant::now();
    document.display_map = Some(Arc::new(super::display_map::build_display_map_reusing(
        &document,
        previous.display_map.as_deref()?,
    )));
    document.metrics.display_map = display_map_started.elapsed();
    Some(document)
}

fn build_preview(
    path: PathBuf,
    snapshot: DocumentSnapshot,
    org_blocks: Option<Arc<BlockArena>>,
    build: PreviewBuild<'_>,
) -> PreviewSnapshot {
    let statistics = TextStatistics::from_snapshot(&snapshot);
    let text: SharedTextSnapshot = Arc::new(snapshot);
    let parse_started = Instant::now();
    let format = DocumentFormat::from_path(&path);
    // Stage 1 publishes document semantics from the one shared syntax arena. Stage 2 below builds
    // the optional reading projection; Agenda can consume stage 1 without reparsing the file.
    let (blocks, semantic, semantic_elapsed, markdown_blocks, rows) = match format {
        DocumentFormat::Org => {
            let blocks = org_blocks.unwrap_or_else(|| Arc::new(parse(text.as_ref())));
            let semantic_started = Instant::now();
            let semantic = Arc::new(org_semantic::analyze(text.as_ref(), blocks.clone()));
            let semantic_elapsed = semantic_started.elapsed();
            let rows = Arc::new(build_preview_rows(text.as_ref(), &blocks));
            (
                blocks,
                Some(semantic),
                semantic_elapsed,
                Arc::new(Vec::new()),
                rows,
            )
        }
        DocumentFormat::Markdown => {
            let (markdown_blocks, rows) = markdown::parse_markdown(text.as_ref());
            (
                Arc::new(BlockArena::default()),
                None,
                Duration::ZERO,
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
    let diagrams = match format {
        DocumentFormat::Org => Arc::new(HashMap::new()),
        DocumentFormat::Markdown => {
            Arc::new(build_markdown_diagrams(text.as_ref(), &markdown_blocks))
        }
    };
    let projection = build_projection_snapshot(
        text.as_ref(),
        text.revision(),
        format,
        rows.clone(),
        &blocks,
        &markdown_blocks,
        ReadingProjectionResources {
            tables: &table_projections,
            images: &image_sizes,
            diagrams: &diagrams,
        },
    );
    let semantic_source_bytes = semantic
        .as_ref()
        .map_or(0, |semantic| semantic.metrics.semantic_source_bytes);
    let mut document = PreviewSnapshot {
        document_id: text.document_id(),
        path,
        revision: text.revision(),
        text,
        format,
        blocks,
        semantic,
        markdown_blocks,
        outline_paths,
        projection,
        display_map: None,
        statistics,
        metrics: LoadMetrics {
            bytes: build.byte_count,
            read: build.read,
            rope: build.rope,
            parse,
            semantic: semantic_elapsed,
            display_map: Duration::ZERO,
            total: Duration::ZERO,
            syntax_reparsed_bytes: build
                .syntax_patch
                .map_or(build.byte_count, |patch| patch.reparsed_bytes),
            semantic_source_bytes,
            full_syntax_fallback: build.full_syntax_fallback,
        },
        update: DerivedUpdate::Full,
    };
    if build.build_display_map {
        let display_map_started = Instant::now();
        document.display_map = Some(Arc::new(super::display_map::build_display_map(&document)));
        document.metrics.display_map = display_map_started.elapsed();
    }
    document.metrics.total = build.total_started.elapsed();
    document
}

#[cfg(test)]
pub(crate) fn derive_preview(path: PathBuf, snapshot: DocumentSnapshot) -> PreviewSnapshot {
    let bytes = snapshot.len_bytes();
    build_preview(
        path,
        snapshot,
        None,
        PreviewBuild {
            byte_count: bytes,
            read: Duration::ZERO,
            rope: Duration::ZERO,
            total_started: Instant::now(),
            build_display_map: true,
            syntax_patch: None,
            full_syntax_fallback: false,
        },
    )
}

pub(crate) fn derive_preview_incremental(
    path: PathBuf,
    snapshot: DocumentSnapshot,
    previous: Option<&PreviewSnapshot>,
    deltas: &[RevisionDelta],
) -> PreviewSnapshot {
    let byte_count = snapshot.len_bytes();
    let total_started = Instant::now();
    let syntax_started = Instant::now();
    let format = DocumentFormat::from_path(&path);
    let syntax = previous.and_then(|previous| {
        (format == DocumentFormat::Org && previous.format == DocumentFormat::Org)
            .then(|| parse_incremental(&snapshot, &previous.blocks, deltas))
            .flatten()
    });
    let (blocks, syntax_patch) = syntax
        .map(|(blocks, patch)| (Some(Arc::new(blocks)), Some(patch)))
        .unwrap_or((None, None));
    let syntax_elapsed = syntax_started.elapsed();
    let full_syntax_fallback = previous.is_some() && !deltas.is_empty() && syntax_patch.is_none();
    if let (Some(previous), Some(blocks), Some(syntax_patch)) =
        (previous, blocks.as_ref(), syntax_patch.as_ref())
        && let Some(mut document) = build_org_incremental(
            path.clone(),
            snapshot.clone(),
            previous,
            blocks.clone(),
            syntax_patch,
            deltas,
            syntax_elapsed,
        )
    {
        document.metrics.total = total_started.elapsed();
        return document;
    }
    if format == DocumentFormat::Markdown
        && let Some(previous) = previous.filter(|previous| {
            previous.format == DocumentFormat::Markdown
                && previous.document_id == snapshot.document_id()
        })
    {
        let previous_rows = (0..previous.projection.rows.len())
            .map(|index| previous.projection.source_row(index))
            .collect::<Option<Vec<_>>>();
        if let Some((markdown_blocks, rows, markdown_patch)) = previous_rows.and_then(|rows| {
            markdown::parse_markdown_incremental(
                &snapshot,
                &previous.markdown_blocks,
                &rows,
                deltas,
            )
        }) && let Some(mut document) = build_markdown_incremental(
            path.clone(),
            snapshot.clone(),
            previous,
            markdown_blocks,
            rows,
            &markdown_patch,
            deltas,
            syntax_started.elapsed(),
        ) {
            document.metrics.total = total_started.elapsed();
            return document;
        }
    }
    let syntax_elapsed = syntax_started.elapsed();
    let mut next = build_preview(
        path,
        snapshot,
        blocks,
        PreviewBuild {
            byte_count,
            read: Duration::ZERO,
            rope: Duration::ZERO,
            total_started,
            build_display_map: true,
            syntax_patch: syntax_patch.as_ref(),
            full_syntax_fallback,
        },
    );
    next.metrics.parse += syntax_elapsed;
    let Some(previous) = previous else {
        return next;
    };
    if previous.document_id != next.document_id
        || previous.format != next.format
        || deltas.is_empty()
        || deltas.first().map(|delta| delta.before) != Some(previous.revision)
        || deltas.last().map(|delta| delta.after) != Some(next.revision)
        || previous.projection.rows.len() != next.projection.rows.len()
        || previous.blocks.nodes().len() != next.blocks.nodes().len()
        || previous.markdown_blocks.len() != next.markdown_blocks.len()
    {
        return next;
    }
    let Ok(plan) = previous.projection.patch_plan(deltas) else {
        return next;
    };
    let Some(old_visual) = expanded_visual_range(previous, &next, plan.affected()) else {
        return next;
    };
    let Some(replacements) = next.projection.replacement_rows(old_visual.clone()) else {
        return next;
    };
    let Ok((projection, patch)) =
        previous
            .projection
            .apply_patch_chain_with_plan(deltas, old_visual, replacements, plan)
    else {
        return next;
    };
    let projection = Arc::new(projection);
    let reused_chunks = previous.projection.shared_chunk_count(&projection);
    let total_chunks = projection.chunk_count();
    next.projection = projection;
    next.update = DerivedUpdate::Incremental {
        patch,
        reused_chunks,
        total_chunks,
    };
    let display_map_started = Instant::now();
    if let Some(previous_map) = previous.display_map.as_deref() {
        next.display_map = Some(Arc::new(super::display_map::build_display_map_reusing(
            &next,
            previous_map,
        )));
    } else {
        next.display_map = Some(Arc::new(super::display_map::build_display_map(&next)));
    }
    next.metrics.display_map += display_map_started.elapsed();
    next.metrics.total = total_started.elapsed();
    next
}

#[allow(clippy::too_many_arguments)]
fn build_markdown_incremental(
    path: PathBuf,
    snapshot: DocumentSnapshot,
    previous: &PreviewSnapshot,
    markdown_blocks: Vec<markdown::MarkdownBlock>,
    rows: Vec<super::PreviewRow>,
    markdown_patch: &markdown::MarkdownPatch,
    deltas: &[RevisionDelta],
    syntax_elapsed: Duration,
) -> Option<PreviewSnapshot> {
    if previous.markdown_blocks.len() != markdown_blocks.len()
        || previous.projection.rows.len() != rows.len()
        || deltas.first()?.before != previous.revision
        || deltas.last()?.after != snapshot.revision()
    {
        return None;
    }
    let plan = previous.projection.patch_plan(deltas).ok()?;
    let old_visual =
        expanded_visual_range_by_dependency(&previous.projection, plan.affected(), |_| 0, |_| 0)?;
    let block_range = old_visual
        .clone()
        .filter_map(|row| {
            previous
                .projection
                .rows
                .get(row)
                .map(|row| row.block_id as usize)
        })
        .fold(None, |range: Option<std::ops::Range<usize>>, block| {
            Some(range.map_or(block..block + 1, |range| {
                range.start.min(block)..range.end.max(block + 1)
            }))
        })?;
    let text: SharedTextSnapshot = Arc::new(snapshot);
    let replacements = build_visual_rows(
        text.as_ref(),
        text.revision(),
        DocumentFormat::Markdown,
        &rows[block_range],
        &BlockArena::default(),
        &markdown_blocks,
        ReadingProjectionResources {
            tables: &HashMap::new(),
            images: &HashMap::new(),
            diagrams: &HashMap::new(),
        },
    );
    if replacements.len() != old_visual.len() {
        return None;
    }
    let (projection, patch) = previous
        .projection
        .apply_patch_chain_with_plan(deltas, old_visual, replacements, plan)
        .ok()?;
    let projection = Arc::new(projection);
    let reused_chunks = previous.projection.shared_chunk_count(&projection);
    let total_chunks = projection.chunk_count();
    let outline_changed = previous.markdown_blocks[markdown_patch.old_blocks.clone()]
        .iter()
        .zip(&markdown_blocks[markdown_patch.new_blocks.clone()])
        .any(|(old, new)| {
            (matches!(old.kind, markdown::MarkdownKind::Heading { .. })
                || matches!(new.kind, markdown::MarkdownKind::Heading { .. }))
                && (old.kind != new.kind
                    || previous.text.copy_range(old.source) != text.copy_range(new.source))
        });
    let markdown_blocks = Arc::new(markdown_blocks);
    let rows = Arc::new(rows);
    let blocks = Arc::new(BlockArena::default());
    let outline_paths = if outline_changed {
        build_outline_paths(
            text.as_ref(),
            DocumentFormat::Markdown,
            &blocks,
            &markdown_blocks,
            &rows,
        )
    } else {
        previous.outline_paths.clone()
    };
    finish_incremental_preview(
        IncrementalPreview {
            path,
            text,
            format: DocumentFormat::Markdown,
            blocks,
            semantic: None,
            markdown_blocks,
            outline_paths,
            projection,
            syntax_elapsed,
            semantic_elapsed: Duration::ZERO,
            semantic_source_bytes: 0,
            syntax_reparsed_bytes: markdown_patch.reparsed_bytes,
            patch,
            reused_chunks,
            total_chunks,
        },
        previous,
    )
}

fn build_org_incremental(
    path: PathBuf,
    snapshot: DocumentSnapshot,
    previous: &PreviewSnapshot,
    blocks: Arc<BlockArena>,
    syntax_patch: &SyntaxPatch,
    deltas: &[RevisionDelta],
    syntax_elapsed: Duration,
) -> Option<PreviewSnapshot> {
    if previous.format != DocumentFormat::Org
        || previous.document_id != snapshot.document_id()
        || previous.blocks.nodes().len() != blocks.nodes().len()
        || deltas.first()?.before != previous.revision
        || deltas.last()?.after != snapshot.revision()
    {
        return None;
    }
    let plan = previous.projection.patch_plan(deltas).ok()?;
    let old_visual = expanded_org_visual_range(previous, &blocks, plan.affected())?;
    let block_range = old_visual
        .clone()
        .filter_map(|row| {
            previous
                .projection
                .rows
                .get(row)
                .map(|row| row.block_id as usize)
        })
        .fold(None, |range: Option<std::ops::Range<usize>>, block| {
            Some(range.map_or(block..block + 1, |range| {
                range.start.min(block)..range.end.max(block + 1)
            }))
        })?;
    let unsafe_local_projection =
        |kind: &BlockKind| matches!(kind, BlockKind::TableRow | BlockKind::Image { .. });
    if blocks.nodes()[block_range.clone()]
        .iter()
        .any(|block| unsafe_local_projection(&block.kind))
        || previous.blocks.nodes()[block_range.clone()]
            .iter()
            .any(|block| unsafe_local_projection(&block.kind))
    {
        return None;
    }

    let text: SharedTextSnapshot = Arc::new(snapshot);
    let rows = build_preview_rows_in_range(text.as_ref(), &blocks, block_range);
    if rows.len() != old_visual.len() {
        return None;
    }
    // Keep semantic extraction ahead of the visual patch so both publications are based on the
    // same syntax patch and revision, without parsing Org a second time.
    let semantic_started = Instant::now();
    let semantic = previous.semantic.as_deref().map_or_else(
        || Arc::new(org_semantic::analyze(text.as_ref(), blocks.clone())),
        |previous| {
            Arc::new(org_semantic::analyze_incremental(
                text.as_ref(),
                blocks.clone(),
                previous,
                syntax_patch,
            ))
        },
    );
    let semantic_elapsed = semantic_started.elapsed();
    let semantic_source_bytes = semantic.metrics.semantic_source_bytes;
    let replacements = build_visual_rows(
        text.as_ref(),
        text.revision(),
        DocumentFormat::Org,
        &rows,
        &blocks,
        &[],
        ReadingProjectionResources {
            tables: &HashMap::new(),
            images: &HashMap::new(),
            diagrams: &HashMap::new(),
        },
    );
    let (projection, patch) = previous
        .projection
        .apply_patch_chain_with_plan(deltas, old_visual, replacements, plan)
        .ok()?;
    let projection = Arc::new(projection);
    let reused_chunks = previous.projection.shared_chunk_count(&projection);
    let total_chunks = projection.chunk_count();
    let outline_changed = previous.blocks.nodes()[syntax_patch.old_blocks.clone()]
        .iter()
        .zip(&blocks.nodes()[syntax_patch.new_blocks.clone()])
        .any(|(old, new)| {
            (matches!(old.kind, BlockKind::Heading { .. })
                || matches!(new.kind, BlockKind::Heading { .. }))
                && (old.kind != new.kind
                    || previous.text.copy_range(old.content) != text.copy_range(new.content))
        });
    let outline_paths = if outline_changed {
        build_outline_paths(text.as_ref(), DocumentFormat::Org, &blocks, &[], &rows)
    } else {
        previous.outline_paths.clone()
    };
    finish_incremental_preview(
        IncrementalPreview {
            path,
            text,
            format: DocumentFormat::Org,
            blocks,
            semantic: Some(semantic),
            markdown_blocks: Arc::new(Vec::new()),
            outline_paths,
            projection,
            syntax_elapsed,
            semantic_elapsed,
            semantic_source_bytes,
            syntax_reparsed_bytes: syntax_patch.reparsed_bytes,
            patch,
            reused_chunks,
            total_chunks,
        },
        previous,
    )
}

fn expanded_visual_range(
    previous: &PreviewSnapshot,
    next: &PreviewSnapshot,
    affected: std::ops::Range<usize>,
) -> Option<std::ops::Range<usize>> {
    match previous.format {
        DocumentFormat::Org => expanded_visual_range_by_dependency(
            &previous.projection,
            affected,
            |block| {
                if matches!(
                    previous.blocks.nodes().get(block).map(|node| &node.kind),
                    Some(BlockKind::TableRow)
                ) {
                    TABLE_DEPENDENCY
                } else {
                    0
                }
            },
            |block| {
                if matches!(
                    next.blocks.nodes().get(block).map(|node| &node.kind),
                    Some(BlockKind::TableRow)
                ) {
                    TABLE_DEPENDENCY
                } else {
                    0
                }
            },
        ),
        DocumentFormat::Markdown => expanded_visual_range_by_dependency(
            &previous.projection,
            affected,
            |block| markdown_dependency(previous.markdown_blocks.get(block)),
            |block| markdown_dependency(next.markdown_blocks.get(block)),
        ),
    }
}

fn expanded_org_visual_range(
    previous: &PreviewSnapshot,
    next: &BlockArena,
    affected: std::ops::Range<usize>,
) -> Option<std::ops::Range<usize>> {
    expanded_visual_range_by_dependency(
        &previous.projection,
        affected,
        |block| {
            if matches!(
                previous.blocks.nodes().get(block).map(|node| &node.kind),
                Some(BlockKind::TableRow)
            ) {
                TABLE_DEPENDENCY
            } else {
                0
            }
        },
        |block| {
            if matches!(
                next.nodes().get(block).map(|node| &node.kind),
                Some(BlockKind::TableRow)
            ) {
                TABLE_DEPENDENCY
            } else {
                0
            }
        },
    )
}

fn markdown_dependency(block: Option<&markdown::MarkdownBlock>) -> u8 {
    match block.map(|block| &block.kind) {
        Some(markdown::MarkdownKind::TableRow) => TABLE_DEPENDENCY,
        Some(markdown::MarkdownKind::Code { .. }) => CODE_DEPENDENCY,
        _ => 0,
    }
}

fn expanded_visual_range_by_dependency(
    projection: &super::projection::ReadingProjection,
    affected: std::ops::Range<usize>,
    old_dependency: impl Fn(usize) -> u8,
    new_dependency: impl Fn(usize) -> u8,
) -> Option<std::ops::Range<usize>> {
    if projection.rows.len() == 0 {
        return Some(0..0);
    }
    let visual_start = affected
        .start
        .saturating_sub(1)
        .min(projection.rows.len() - 1);
    let visual_end = affected
        .end
        .saturating_add(1)
        .max(visual_start + 1)
        .min(projection.rows.len());
    let mut block_start = projection.rows.get(visual_start)?.block_id as usize;
    let mut block_end = projection.rows.get(visual_end - 1)?.block_id as usize + 1;
    let dependency = |block| old_dependency(block) | new_dependency(block);
    while block_start > 0 && dependency(block_start) & dependency(block_start - 1) != 0 {
        block_start -= 1;
    }
    while dependency(block_end.saturating_sub(1)) & dependency(block_end) != 0 {
        block_end += 1;
    }
    let start = projection
        .rows
        .iter()
        .position(|row| row.block_id as usize >= block_start)
        .unwrap_or(projection.rows.len());
    let end = projection
        .rows
        .iter()
        .position(|row| row.block_id as usize >= block_end)
        .unwrap_or(projection.rows.len());
    Some(start..end)
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
    use crate::document::{ByteRange, DocumentBuffer, DocumentSnapshot, EditTransaction, TextEdit};

    #[test]
    fn svg_dimensions_prefer_the_view_box_and_preserve_its_aspect_ratio() {
        let svg = br#"<svg viewBox="0 0 573.307 105.18" width="573.307pt" height="105.18pt" xmlns="http://www.w3.org/2000/svg"></svg>"#;
        assert_eq!(svg_dimensions(svg), Some((573, 105)));
        assert_eq!(fitted_image_size(573, 105, 960.0), (573.0, 105.0));
    }

    #[test]
    fn svg_dimensions_convert_absolute_lengths_without_a_view_box() {
        let svg = br#"<svg width="24pt" height="12pt" xmlns="http://www.w3.org/2000/svg"></svg>"#;
        assert_eq!(svg_dimensions(svg), Some((32, 16)));
    }

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

    #[test]
    fn checkbox_state_edit_is_paint_only() {
        let mut buffer = DocumentBuffer::from_utf8(b"- [ ] task\n".to_vec()).unwrap();
        let before = buffer.snapshot();
        let previous = derive_preview(PathBuf::from("checkbox.org"), before.clone());
        let delta = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(ByteRange::new(3, 4), "X")],
            ))
            .unwrap();
        let next = derive_preview_incremental(
            PathBuf::from("checkbox.org"),
            buffer.snapshot(),
            Some(&previous),
            &[delta],
        );
        let DerivedUpdate::Incremental { patch, .. } = &next.update else {
            panic!("checkbox state should use an incremental projection");
        };

        assert!(
            patch
                .invalidation
                .contains(super::super::projection::InvalidationFlags::PAINT)
        );
        assert!(
            !patch
                .invalidation
                .contains(super::super::projection::InvalidationFlags::GEOMETRY)
        );
        assert_eq!(
            next.projection.revisions.geometry,
            previous.projection.revisions.geometry
        );
        assert_ne!(
            next.projection.revisions.paint,
            previous.projection.revisions.paint
        );
    }

    #[test]
    fn incremental_projection_matches_full_build_and_reuses_untouched_chunks() {
        let source = (0..400)
            .map(|index| format!("* Heading {index}\nbody {index}\n"))
            .collect::<String>();
        let mut buffer = DocumentBuffer::from_utf8(source.into_bytes()).unwrap();
        let before = buffer.snapshot();
        let previous = derive_preview(PathBuf::from("incremental.org"), before.clone());
        let needle = "body 200";
        let offset = before
            .copy_range(ByteRange::new(0, before.len_bytes()))
            .find(needle)
            .unwrap() as u64;
        let delta = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(ByteRange::new(offset + 5, offset + 8), "two")],
            ))
            .unwrap();
        let after = buffer.snapshot();
        let full = derive_preview(PathBuf::from("incremental.org"), after.clone());
        let incremental = derive_preview_incremental(
            PathBuf::from("incremental.org"),
            after,
            Some(&previous),
            &[delta],
        );

        let (reused_chunks, total_chunks, changed_row) = match &incremental.update {
            DerivedUpdate::Incremental {
                patch,
                reused_chunks,
                total_chunks,
            } => (*reused_chunks, *total_chunks, patch.new_visual.start),
            DerivedUpdate::Full => panic!("same-shape edit should use an incremental projection"),
        };
        assert!(reused_chunks > 0);
        assert!(reused_chunks < total_chunks);
        assert_eq!(
            incremental.projection.rows.len(),
            full.projection.rows.len()
        );
        for index in 0..full.projection.rows.len() {
            let expected = full.projection.source_row(index).unwrap();
            let actual = incremental.projection.source_row(index).unwrap();
            assert_eq!(actual.block_id, expected.block_id);
            assert_eq!(actual.continuation, expected.continuation);
            assert_eq!(actual.blank, expected.blank);
            assert_eq!(
                incremental.text.copy_range(actual.content.range),
                full.text.copy_range(expected.content.range)
            );
        }
        assert_eq!(
            incremental.projection.rows.get(0).unwrap().id,
            previous.projection.rows.get(0).unwrap().id
        );
        let last = incremental.projection.rows.len() - 1;
        assert_eq!(
            incremental.projection.rows.get(last).unwrap().id,
            previous.projection.rows.get(last).unwrap().id
        );
        assert_ne!(
            incremental
                .projection
                .rows
                .get(changed_row)
                .unwrap()
                .semantic_revision,
            previous.revision.0
        );
    }

    #[test]
    fn structural_edit_explicitly_falls_back_to_a_full_projection() {
        let mut buffer = DocumentBuffer::from_utf8(b"* Heading\nbody\n".to_vec()).unwrap();
        let before = buffer.snapshot();
        let previous = derive_preview(PathBuf::from("fallback.org"), before.clone());
        let delta = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(ByteRange::new(15, 15), "\nnew block")],
            ))
            .unwrap();
        let next = derive_preview_incremental(
            PathBuf::from("fallback.org"),
            buffer.snapshot(),
            Some(&previous),
            &[delta],
        );
        assert!(matches!(next.update, DerivedUpdate::Full));
    }

    #[test]
    fn coalesced_typing_delta_chain_remains_incremental() {
        let source = (0..300)
            .map(|index| format!("* Heading {index}\nbody {index}\n"))
            .collect::<String>();
        let mut buffer = DocumentBuffer::from_utf8(source.into_bytes()).unwrap();
        let before = buffer.snapshot();
        let previous = derive_preview(PathBuf::from("coalesced.org"), before.clone());
        let caret = before
            .copy_range(ByteRange::new(0, before.len_bytes()))
            .find("body 150")
            .unwrap() as u64
            + 8;
        let first = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(ByteRange::new(caret, caret), "a")],
            ))
            .unwrap();
        let second = buffer
            .commit(EditTransaction::new(
                first.after,
                vec![TextEdit::new(ByteRange::new(caret + 1, caret + 1), "b")],
            ))
            .unwrap();
        let after = buffer.snapshot();
        let incremental = derive_preview_incremental(
            PathBuf::from("coalesced.org"),
            after.clone(),
            Some(&previous),
            &[first, second],
        );
        let full = derive_preview(PathBuf::from("coalesced.org"), after);
        assert!(matches!(
            incremental.update,
            DerivedUpdate::Incremental { .. }
        ));
        for index in 0..full.projection.rows.len() {
            let expected = full.projection.source_row(index).unwrap();
            let actual = incremental.projection.source_row(index).unwrap();
            assert_eq!(
                incremental.text.copy_range(actual.content.range),
                full.text.copy_range(expected.content.range)
            );
        }
    }

    #[test]
    fn distant_coalesced_edits_cover_the_complete_delta_chain() {
        let source = (0..300)
            .map(|index| format!("* Heading {index}\nbody {index}\n"))
            .collect::<String>();
        let mut buffer = DocumentBuffer::from_utf8(source.into_bytes()).unwrap();
        let before = buffer.snapshot();
        let previous = derive_preview(PathBuf::from("distant.org"), before.clone());
        let text = before.copy_range(ByteRange::new(0, before.len_bytes()));
        let first_caret = text.find("body 20").unwrap() as u64 + 7;
        let second_caret = text.find("body 280").unwrap() as u64 + 8;
        let first = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(ByteRange::new(first_caret, first_caret), "a")],
            ))
            .unwrap();
        let second = buffer
            .commit(EditTransaction::new(
                first.after,
                vec![TextEdit::new(
                    ByteRange::new(second_caret + 1, second_caret + 1),
                    "b",
                )],
            ))
            .unwrap();
        let after = buffer.snapshot();
        let incremental = derive_preview_incremental(
            PathBuf::from("distant.org"),
            after.clone(),
            Some(&previous),
            &[first, second],
        );
        let full = derive_preview(PathBuf::from("distant.org"), after);
        let DerivedUpdate::Incremental { patch, .. } = &incremental.update else {
            panic!("same-shape distant edits should still produce a visual patch");
        };
        assert!(patch.old_visual.start < 50);
        assert!(patch.old_visual.end > 550);
        for index in 0..full.projection.rows.len() {
            let expected = full.projection.source_row(index).unwrap();
            let actual = incremental.projection.source_row(index).unwrap();
            assert_eq!(
                incremental.text.copy_range(actual.content.range),
                full.text.copy_range(expected.content.range)
            );
        }
    }

    #[test]
    fn markdown_plain_text_edit_uses_local_syntax_and_equivalent_visual_patch() {
        let source = (0..300)
            .map(|index| format!("# Heading {index}\nbody {index}\n\n"))
            .collect::<String>();
        let mut buffer = DocumentBuffer::from_utf8(source.into_bytes()).unwrap();
        let before = buffer.snapshot();
        let previous = derive_preview(PathBuf::from("fallback.md"), before.clone());
        let caret = before
            .copy_range(ByteRange::new(0, before.len_bytes()))
            .find("body 150")
            .unwrap() as u64
            + 8;
        let delta = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(ByteRange::new(caret, caret), "x")],
            ))
            .unwrap();
        let after = buffer.snapshot();
        let incremental = derive_preview_incremental(
            PathBuf::from("fallback.md"),
            after.clone(),
            Some(&previous),
            &[delta],
        );
        let full = derive_preview(PathBuf::from("fallback.md"), after);
        assert!(!incremental.metrics.full_syntax_fallback);
        assert!(incremental.metrics.syntax_reparsed_bytes < incremental.metrics.bytes / 10);
        assert!(matches!(
            incremental.update,
            DerivedUpdate::Incremental { .. }
        ));
        for index in 0..full.projection.rows.len() {
            let expected = full.projection.source_row(index).unwrap();
            let actual = incremental.projection.source_row(index).unwrap();
            assert_eq!(
                incremental.text.copy_range(actual.content.range),
                full.text.copy_range(expected.content.range)
            );
        }
    }

    #[test]
    fn markdown_fenced_code_edit_uses_the_explicit_full_syntax_boundary() {
        let source = "# Heading\n\n```rust\nfn main() {}\n```\nafter\n";
        let mut buffer = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
        let before = buffer.snapshot();
        let previous = derive_preview(PathBuf::from("fence.md"), before.clone());
        let caret = source.find("main").unwrap() as u64;
        let delta = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(ByteRange::new(caret, caret + 4), "entry")],
            ))
            .unwrap();
        let next = derive_preview_incremental(
            PathBuf::from("fence.md"),
            buffer.snapshot(),
            Some(&previous),
            &[delta],
        );
        assert!(next.metrics.full_syntax_fallback);
    }

    #[test]
    fn markdown_fence_creation_patches_the_complete_downstream_dependency() {
        let source = "# Heading\nnot a fence\nalpha\nbeta\n";
        let mut buffer = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
        let before = buffer.snapshot();
        let previous = derive_preview(PathBuf::from("new-fence.md"), before.clone());
        let start = source.find("not a fence").unwrap() as u64;
        let delta = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(
                    ByteRange::new(start, start + "not a fence".len() as u64),
                    "```rust",
                )],
            ))
            .unwrap();
        let after = buffer.snapshot();
        let incremental = derive_preview_incremental(
            PathBuf::from("new-fence.md"),
            after.clone(),
            Some(&previous),
            &[delta],
        );
        let full = derive_preview(PathBuf::from("new-fence.md"), after);
        assert!(incremental.metrics.full_syntax_fallback);
        for index in 0..full.projection.rows.len() {
            let actual = incremental.projection.rows.get(index).unwrap();
            let expected = full.projection.rows.get(index).unwrap();
            assert_eq!(
                std::mem::discriminant(&actual.kind),
                std::mem::discriminant(&expected.kind)
            );
            assert_eq!(actual.code_language, expected.code_language);
            assert_eq!(
                incremental.text.copy_range(
                    incremental
                        .projection
                        .source_row(index)
                        .unwrap()
                        .content
                        .range
                ),
                full.text
                    .copy_range(full.projection.source_row(index).unwrap().content.range)
            );
        }
    }

    #[test]
    fn table_edit_patches_the_complete_geometry_dependency_group() {
        let source = "* Table\n| Name | Value |\n|------+-------|\n| one  | x     |\n| two  | y     |\nafter\n";
        let mut buffer = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
        let before = buffer.snapshot();
        let previous = derive_preview(PathBuf::from("table.org"), before.clone());
        let value = source.find("| x").unwrap() as u64 + 2;
        let delta = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(
                    ByteRange::new(value, value + 1),
                    "a much wider value",
                )],
            ))
            .unwrap();
        let after = buffer.snapshot();
        let incremental = derive_preview_incremental(
            PathBuf::from("table.org"),
            after.clone(),
            Some(&previous),
            &[delta],
        );
        let full = derive_preview(PathBuf::from("table.org"), after);
        let mut table_rows = 0;
        for index in 0..full.projection.rows.len() {
            let expected = &full.projection.rows.get(index).unwrap().kind;
            let actual = &incremental.projection.rows.get(index).unwrap().kind;
            if let (
                super::super::projection::VisualRowKind::Table(expected),
                super::super::projection::VisualRowKind::Table(actual),
            ) = (expected, actual)
            {
                table_rows += 1;
                assert!(actual.table().same_geometry(expected.table()));
            }
        }
        assert_eq!(table_rows, 4);
    }

    #[test]
    fn source_language_edit_patches_every_reading_code_row() {
        let source = "* Code\n#+begin_src rust\nfn one() {}\nfn two() {}\n#+end_src\nafter\n";
        let mut buffer = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
        let before = buffer.snapshot();
        let previous = derive_preview(PathBuf::from("code.org"), before.clone());
        let language = source.find("rust").unwrap() as u64;
        let delta = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(
                    ByteRange::new(language, language + 4),
                    "python",
                )],
            ))
            .unwrap();
        let incremental = derive_preview_incremental(
            PathBuf::from("code.org"),
            buffer.snapshot(),
            Some(&previous),
            &[delta],
        );
        let code_rows = incremental
            .projection
            .rows
            .iter()
            .filter(|row| matches!(row.kind, super::super::projection::VisualRowKind::Code(_)))
            .collect::<Vec<_>>();
        assert_eq!(code_rows.len(), 2);
        assert!(
            code_rows
                .iter()
                .all(|row| row.code_language.as_deref() == Some("python"))
        );
    }

    #[test]
    fn plantuml_content_edit_rebuilds_the_rendered_diagram() {
        use super::super::{diagram::DiagramProjection, projection::VisualRowKind};

        let source = "before\n```plantuml\n@startuml\nAlice -> Bob: hello\n@enduml\n```\nafter\n";
        let mut buffer = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
        let before = buffer.snapshot();
        let previous = derive_preview(PathBuf::from("diagram.md"), before.clone());
        let previous_svg = previous
            .projection
            .rows
            .iter()
            .find_map(|row| match &row.kind {
                VisualRowKind::Diagram(DiagramProjection::Ready { image, .. }) => {
                    Some(image.bytes().to_vec())
                }
                _ => None,
            })
            .expect("initial diagram renders");
        let label = source.find("hello").unwrap() as u64;
        let delta = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(
                    ByteRange::new(label, label + "hello".len() as u64),
                    "updated",
                )],
            ))
            .unwrap();

        let next = derive_preview_incremental(
            PathBuf::from("diagram.md"),
            buffer.snapshot(),
            Some(&previous),
            &[delta],
        );
        let next_svg = next
            .projection
            .rows
            .iter()
            .find_map(|row| match &row.kind {
                VisualRowKind::Diagram(DiagramProjection::Ready { image, .. }) => {
                    Some(image.bytes())
                }
                _ => None,
            })
            .expect("edited diagram renders");

        assert!(matches!(next.update, DerivedUpdate::Incremental { .. }));
        assert_ne!(next_svg, previous_svg);
    }

    #[test]
    fn markdown_text_edit_keeps_incremental_projection_with_a_distant_diagram() {
        let source =
            "alpha\nbeta\ngamma\n\n```plantuml\n@startuml\nAlice -> Bob\n@enduml\n```\n\nomega\n";
        let mut buffer = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
        let before = buffer.snapshot();
        let previous = derive_preview(PathBuf::from("diagram.md"), before.clone());
        let delta = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(ByteRange::new(0, 5), "ALPHA")],
            ))
            .unwrap();

        let next = derive_preview_incremental(
            PathBuf::from("diagram.md"),
            buffer.snapshot(),
            Some(&previous),
            &[delta],
        );

        assert!(matches!(next.update, DerivedUpdate::Incremental { .. }));
        assert!(next.projection.rows.iter().any(|row| matches!(
            row.kind,
            super::super::projection::VisualRowKind::Diagram(_)
        )));
    }

    #[cfg(feature = "benchmarks")]
    #[test]
    fn reading_fifty_mib_incremental_probe() {
        let path = std::env::var_os("ORG_STUDIO_READING_FIXTURE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/perf-fixtures/org-preview-50m.org"));
        let target_p95_ms = std::env::var("ORG_STUDIO_READING_TARGET_P95_MS")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(150.0);
        let bytes = std::fs::read(&path).expect("run scripts/perf/org-preview prepare 50 first");
        let mut buffer = DocumentBuffer::from_utf8(bytes).unwrap();
        let before = buffer.snapshot();
        let mut previous = derive_preview(path.clone(), before.clone());
        let mut caret = before.len_bytes() / 2;
        while !before.is_char_boundary(crate::document::ByteOffset(caret)) {
            caret -= 1;
        }
        let mut samples = Vec::with_capacity(20);
        let mut max_reparsed_bytes = 0;
        let mut min_reused_chunks = usize::MAX;
        let mut total_chunks = 0;
        for _ in 0..20 {
            let delta = buffer
                .commit(EditTransaction::new(
                    buffer.revision(),
                    vec![TextEdit::new(ByteRange::new(caret, caret), "x")],
                ))
                .unwrap();
            caret += 1;
            let started = Instant::now();
            let next = derive_preview_incremental(
                path.clone(),
                buffer.snapshot(),
                Some(&previous),
                &[delta],
            );
            samples.push(started.elapsed());
            assert!(next.metrics.syntax_reparsed_bytes < next.metrics.bytes / 100);
            let (reused_chunks, next_total_chunks) = match &next.update {
                DerivedUpdate::Incremental {
                    reused_chunks,
                    total_chunks,
                    ..
                } => (*reused_chunks, *total_chunks),
                DerivedUpdate::Full => {
                    panic!("50 MiB single-character edit should remain incremental")
                }
            };
            max_reparsed_bytes = max_reparsed_bytes.max(next.metrics.syntax_reparsed_bytes);
            min_reused_chunks = min_reused_chunks.min(reused_chunks);
            total_chunks = next_total_chunks;
            previous = next;
        }
        samples.sort_unstable();
        let percentile_ms = |percentile: f64| {
            let index = ((samples.len() - 1) as f64 * percentile).ceil() as usize;
            samples[index].as_secs_f64() * 1_000.0
        };
        eprintln!(
            "reading_50m samples={} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3} max_reparsed_bytes={} total_bytes={} min_reused_chunks={} total_chunks={} last_parse_ms={:.3} last_display_map_ms={:.3}",
            samples.len(),
            percentile_ms(0.50),
            percentile_ms(0.95),
            percentile_ms(0.99),
            max_reparsed_bytes,
            previous.metrics.bytes,
            min_reused_chunks,
            total_chunks,
            previous.metrics.parse.as_secs_f64() * 1_000.0,
            previous.metrics.display_map.as_secs_f64() * 1_000.0,
        );
        assert!(
            percentile_ms(0.95) <= target_p95_ms,
            "50 MiB incremental p95 {:.3} ms exceeds {:.3} ms budget",
            percentile_ms(0.95),
            target_p95_ms,
        );
        assert!(
            min_reused_chunks.saturating_mul(100) >= total_chunks.saturating_mul(99),
            "50 MiB incremental projection must reuse at least 99% of chunks",
        );
    }

    #[cfg(feature = "benchmarks")]
    #[test]
    fn reading_ten_mib_markdown_incremental_gate() {
        const TARGET_BYTES: usize = 10 * 1024 * 1024;
        let section = "# Stable heading\nbody text for incremental markdown editing\n\n";
        let mut source = String::with_capacity(TARGET_BYTES + section.len());
        while source.len() < TARGET_BYTES {
            source.push_str(section);
        }
        let mut buffer = DocumentBuffer::from_utf8(source.into_bytes()).unwrap();
        let before = buffer.snapshot();
        let mut previous = derive_preview(PathBuf::from("large.md"), before.clone());
        let middle = before.len_bytes() / 2;
        let prefix = before.copy_range(ByteRange::new(0, middle));
        let caret = prefix
            .rfind("body text")
            .expect("fixture contains body text") as u64
            + 5;
        let mut samples = Vec::with_capacity(10);
        for caret in (caret..).take(10) {
            let delta = buffer
                .commit(EditTransaction::new(
                    buffer.revision(),
                    vec![TextEdit::new(ByteRange::new(caret, caret), "x")],
                ))
                .unwrap();
            let started = Instant::now();
            let next = derive_preview_incremental(
                PathBuf::from("large.md"),
                buffer.snapshot(),
                Some(&previous),
                &[delta],
            );
            samples.push(started.elapsed());
            assert!(!next.metrics.full_syntax_fallback);
            assert!(next.metrics.syntax_reparsed_bytes < 512);
            assert!(matches!(next.update, DerivedUpdate::Incremental { .. }));
            previous = next;
        }
        samples.sort_unstable();
        let p95_ms = samples[9].as_secs_f64() * 1_000.0;
        eprintln!("reading_markdown_10m samples=10 p95_ms={p95_ms:.3}");
        assert!(
            p95_ms <= 150.0,
            "10 MiB Markdown incremental p95 {p95_ms:.3} ms exceeds 150 ms budget"
        );
    }
}
