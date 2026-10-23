use crate::{
    document::{ByteOffset, ByteRange, DocumentSnapshot, TextSnapshot},
    org_syntax::{BlockArena, BlockKind},
};

/// Keeps an `#+ATTR_ORG:` line attached to the results image.
pub(super) fn preserved_result_attributes(existing: &str) -> Option<String> {
    existing
        .lines()
        .map(str::trim_end)
        .find(|line| crate::org_syntax::attributes::is_attr_org_line(line))
        .map(str::to_owned)
}

pub(super) fn result_range(
    snapshot: &DocumentSnapshot,
    blocks: &BlockArena,
    source_index: usize,
    file_result: bool,
) -> Result<(ByteRange, String, Option<String>), String> {
    let source = &blocks.nodes()[source_index];
    let name = super::headers::source_name(snapshot, blocks, source_index);
    if let Some(name) = &name {
        let count = blocks
            .nodes()
            .iter()
            .enumerate()
            .filter(|(index, node)| {
                matches!(node.kind, BlockKind::SourceBlock { .. })
                    && super::headers::source_name(snapshot, blocks, *index).as_ref() == Some(name)
            })
            .count();
        if count > 1 {
            return Err(format!("Duplicate Babel source block name: {name}"));
        }
    }
    let mut candidates = Vec::new();
    for (index, node) in blocks.nodes().iter().enumerate() {
        if !matches!(node.kind, BlockKind::Keyword) {
            continue;
        }
        let text = snapshot.copy_range(node.content);
        if !is_results_marker(&text) {
            continue;
        }
        let marker_name = text
            .split_once("]:")
            .or_else(|| text.split_once(':'))
            .map(|(_, value)| value.trim())
            .unwrap_or("");
        let adjacent = index > source_index
            && blocks.nodes()[source_index + 1..index].iter().all(|n| {
                matches!(n.kind, BlockKind::BlankLine)
                    || matches!(n.kind, BlockKind::Keyword)
                        && snapshot
                            .copy_range(n.content)
                            .trim()
                            .to_ascii_lowercase()
                            .starts_with("#+name:")
            });
        if name.as_deref().is_some_and(|name| name == marker_name)
            || adjacent && marker_name.is_empty()
        {
            candidates.push(index);
        } else if adjacent {
            return Err(
                "The adjacent Babel result has a different name; repair its ownership before running"
                    .into(),
            );
        }
    }
    if candidates.len() > 1 {
        return Err("Multiple results belong to this Babel entry; remove the duplicate results before running".into());
    }
    let Some(index) = candidates.first().copied() else {
        return Ok((
            ByteRange::new(source.source.end.0, source.source.end.0),
            String::new(),
            name,
        ));
    };
    let marker = &blocks.nodes()[index];
    let mut start = marker.source.start;
    let mut attributes = String::new();
    for node in blocks.nodes()[..index].iter().rev() {
        if node.source.end != start || !matches!(node.kind, BlockKind::Keyword) {
            break;
        }
        let text = snapshot.copy_range(node.content);
        if !crate::org_syntax::attributes::is_affiliated_keyword(&text) {
            break;
        }
        start = node.source.start;
        attributes.insert_str(0, &snapshot.copy_range(node.source));
    }
    if start >= source.source.end {
        let gap = snapshot.copy_range(ByteRange::new(source.source.end.0, start.0));
        if gap.trim().is_empty() {
            start = source.source.end;
        }
    }
    let mut end = marker.source.end;
    let mut payload = index + 1;
    while let Some(node) = blocks.nodes().get(payload) {
        let text = snapshot.copy_range(node.content);
        if node.source.start != end
            || !matches!(node.kind, BlockKind::Keyword)
            || !crate::org_syntax::attributes::is_affiliated_keyword(&text)
        {
            break;
        }
        attributes.push_str(&snapshot.copy_range(node.source));
        end = node.source.end;
        payload += 1;
    }
    if file_result && let Some(node) = blocks.nodes().get(payload) {
        let text = snapshot.copy_range(node.source);
        let first = text.split_inclusive('\n').next().unwrap_or(&text);
        if first.trim().starts_with("[[file:") && first.trim().ends_with("]]") {
            return Ok((
                ByteRange {
                    start,
                    end: ByteOffset(node.source.start.0 + first.len() as u64),
                },
                attributes,
                name,
            ));
        }
    }
    if let Some(node) = blocks.nodes().get(payload)
        && node.source.start == end
        && !matches!(
            node.kind,
            BlockKind::TableRow
                | BlockKind::FixedWidth
                | BlockKind::BlankLine
                | BlockKind::Heading { .. }
        )
    {
        return Err("The existing Babel result is not a table or its boundary is ambiguous; separate the following text with a blank line before running".into());
    }
    while let Some(node) = blocks.nodes().get(payload) {
        if node.source.start != end
            || !matches!(node.kind, BlockKind::TableRow | BlockKind::FixedWidth)
        {
            break;
        }
        end = node.source.end;
        payload += 1;
    }
    Ok((ByteRange { start, end }, attributes, name))
}

pub(super) fn is_results_marker(source: &str) -> bool {
    let lower = source.trim().to_ascii_lowercase();
    lower.starts_with("#+results:")
        || lower
            .strip_prefix("#+results[")
            .is_some_and(|rest| rest.contains("]:"))
}

pub(super) fn header_argument<'a>(tokens: &'a [String], name: &str) -> Option<&'a str> {
    tokens.windows(2).find_map(|pair| {
        pair[0]
            .eq_ignore_ascii_case(name)
            .then_some(pair[1].as_str())
    })
}

pub(super) fn tokenize_header(source: &str) -> Result<Vec<String>, String> {
    super::headers::tokenize(source)
}
