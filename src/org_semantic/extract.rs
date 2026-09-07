use std::{collections::HashMap, sync::Arc};

use crate::{
    document::TextSnapshot,
    org_syntax::{BlockArena, BlockKind, SyntaxPatch},
};

use super::{
    file_config::extract_file_config,
    model::{OrgAnalysisSnapshot, OrgHeading, SemanticMetrics},
    timestamp::parse_timestamps,
};

pub(crate) fn analyze(snapshot: &dyn TextSnapshot, blocks: Arc<BlockArena>) -> OrgAnalysisSnapshot {
    analyze_impl(snapshot, blocks, None, None)
}

pub(crate) fn analyze_incremental(
    snapshot: &dyn TextSnapshot,
    blocks: Arc<BlockArena>,
    previous: &OrgAnalysisSnapshot,
    patch: &SyntaxPatch,
) -> OrgAnalysisSnapshot {
    analyze_impl(snapshot, blocks, Some(previous), Some(patch))
}

fn analyze_impl(
    snapshot: &dyn TextSnapshot,
    blocks: Arc<BlockArena>,
    previous: Option<&OrgAnalysisSnapshot>,
    patch: Option<&SyntaxPatch>,
) -> OrgAnalysisSnapshot {
    let config = Arc::new(extract_file_config(snapshot));
    let config_changed =
        previous.is_none_or(|previous| previous.config.as_ref() != config.as_ref());
    let previous_headings = previous
        .map(|previous| {
            previous
                .headings
                .iter()
                .map(|heading| (heading.syntax_id, heading))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let mut headings = Vec::<OrgHeading>::new();
    let mut arena_to_heading = vec![None::<usize>; blocks.nodes().len()];
    let mut reused_timestamps = Vec::new();
    let mut metrics = SemanticMetrics {
        full_config_rebuild: config_changed,
        ..SemanticMetrics::default()
    };

    for (block_index, node) in blocks.nodes().iter().enumerate() {
        let BlockKind::Heading { level } = node.kind else {
            continue;
        };
        metrics.examined_headings += 1;
        let parent_index = node
            .parent
            .and_then(|parent| arena_to_heading[parent as usize]);
        let parent_heading = parent_index.map(|index| headings[index].syntax_id);
        let reusable = (!config_changed
            && patch.is_some_and(|patch| !ranges_overlap(node.content, patch.new_source)))
        .then(|| previous_headings.get(&node.syntax_id).copied())
        .flatten();
        let (todo, priority, title, tags) = if let Some(previous) = reusable {
            metrics.reused_headings += 1;
            (
                previous.todo.clone(),
                previous.priority,
                Arc::clone(&previous.title),
                Arc::clone(&previous.tags),
            )
        } else {
            let source = snapshot.copy_range(node.content);
            metrics.semantic_source_bytes += source.len() as u64;
            metrics.extracted_headings += 1;
            let parsed = parse_heading_content(&source, &config);
            (parsed.todo, parsed.priority, parsed.title, parsed.tags)
        };
        let mut effective_tags = parent_index.map_or_else(
            || config.file_tags.to_vec(),
            |index| headings[index].effective_tags.to_vec(),
        );
        for tag in tags.iter() {
            if !effective_tags.iter().any(|inherited| inherited == tag) {
                effective_tags.push(Arc::clone(tag));
            }
        }
        let previous_heading = previous_headings.get(&node.syntax_id).copied();
        let can_reuse_timestamps = patch.is_some_and(|patch| {
            !ranges_overlap(node.source, patch.new_source) && previous_heading.is_some()
        });
        let timestamps = if can_reuse_timestamps {
            let previous_heading = previous_heading.expect("checked above");
            let shift = node.source.start.0 as i128 - previous_heading.source.start.0 as i128;
            previous_heading
                .timestamps
                .iter()
                .cloned()
                .map(|mut timestamp| {
                    timestamp.source_range.start.0 =
                        (timestamp.source_range.start.0 as i128 + shift) as u64;
                    timestamp.source_range.end.0 =
                        (timestamp.source_range.end.0 as i128 + shift) as u64;
                    timestamp
                })
                .collect::<Vec<_>>()
                .into()
        } else {
            Arc::from([])
        };
        arena_to_heading[block_index] = Some(headings.len());
        reused_timestamps.push(can_reuse_timestamps);
        headings.push(OrgHeading {
            syntax_id: node.syntax_id,
            source: node.source,
            content: node.content,
            level,
            parent: parent_heading,
            todo,
            priority,
            title,
            tags,
            effective_tags: effective_tags.into(),
            timestamps,
            properties: if can_reuse_timestamps {
                previous_heading
                    .map(|heading| heading.properties.clone())
                    .unwrap_or_default()
            } else {
                Arc::from([])
            },
        });
    }

    for (block_index, node) in blocks.nodes().iter().enumerate() {
        let heading_index = match node.kind {
            BlockKind::Heading { .. } => arena_to_heading[block_index],
            BlockKind::Planning | BlockKind::Paragraph | BlockKind::Drawer { .. } => node
                .parent
                .and_then(|parent| arena_to_heading[parent as usize]),
            _ => None,
        };
        let Some(heading_index) = heading_index else {
            continue;
        };
        if reused_timestamps[heading_index] {
            continue;
        }
        let source = snapshot.copy_range(node.content);
        metrics.semantic_source_bytes += source.len() as u64;
        let timestamps = parse_timestamps(&source, node.content.start.0);
        if !timestamps.is_empty() {
            let mut combined = headings[heading_index].timestamps.to_vec();
            combined.extend(timestamps);
            headings[heading_index].timestamps = combined.into();
        }
        if matches!(&node.kind, BlockKind::Drawer { name } if name.eq_ignore_ascii_case("PROPERTIES"))
        {
            let mut properties = headings[heading_index].properties.to_vec();
            for line in source.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix(':')
                    && let Some((key, value)) = rest.split_once(':')
                    && !key.is_empty()
                {
                    properties.push((Arc::from(key.to_ascii_uppercase()), Arc::from(value.trim())));
                }
            }
            headings[heading_index].properties = properties.into();
        }
    }

    OrgAnalysisSnapshot {
        document_id: snapshot.document_id(),
        revision: snapshot.revision(),
        blocks,
        config,
        headings: headings.into(),
        diagnostics: Arc::from([]),
        metrics,
    }
}

fn ranges_overlap(left: crate::document::ByteRange, right: crate::document::ByteRange) -> bool {
    left.start < right.end && right.start < left.end
}

struct ParsedHeading {
    todo: Option<super::model::TodoState>,
    priority: Option<char>,
    title: Arc<str>,
    tags: Arc<[Arc<str>]>,
}

fn parse_heading_content(content: &str, config: &super::model::OrgFileConfig) -> ParsedHeading {
    let mut rest = content.trim();
    let first_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let first = &rest[..first_end];
    let todo = config.todo_state(first).cloned();
    if todo.is_some() {
        rest = rest[first_end..].trim_start();
    }

    let priority = rest
        .strip_prefix("[#")
        .and_then(|value| value.chars().next())
        .filter(|_| rest.as_bytes().get(3) == Some(&b']'));
    if priority.is_some() {
        rest = rest[4..].trim_start();
    }

    let (title, tags) = split_tags(rest);
    ParsedHeading {
        todo,
        priority,
        title: Arc::from(title.trim_end()),
        tags: tags.into(),
    }
}

fn split_tags(text: &str) -> (&str, Vec<Arc<str>>) {
    let Some(start) = text
        .rfind(char::is_whitespace)
        .map(|index| index + text[index..].chars().next().unwrap().len_utf8())
    else {
        return if is_tag_set(text) {
            ("", tag_names(text))
        } else {
            (text, Vec::new())
        };
    };
    let candidate = &text[start..];
    if is_tag_set(candidate) {
        (&text[..start], tag_names(candidate))
    } else {
        (text, Vec::new())
    }
}

fn is_tag_set(text: &str) -> bool {
    text.len() >= 3
        && text.starts_with(':')
        && text.ends_with(':')
        && !text.bytes().any(|byte| byte.is_ascii_whitespace())
        && text[1..text.len() - 1]
            .split(':')
            .all(|tag| !tag.is_empty())
}

fn tag_names(text: &str) -> Vec<Arc<str>> {
    text.trim_matches(':').split(':').map(Arc::from).collect()
}
