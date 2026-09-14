//! Build picker values from a revision-consistent document snapshot.
use super::*;
use crate::{editor::links::internal_preview, links::LiteralDestination};

pub(super) fn priority_token(text: &str) -> Option<Range<usize>> {
    let start = text.find("[#")?;
    let end = start + text[start..].find(']')? + 1;
    let value = &text[start + 2..end - 1];
    let valid = (value.len() == 1 && value.as_bytes()[0].is_ascii_uppercase())
        || value.parse::<u8>().is_ok_and(|n| n <= 64);
    let prefix: Vec<_> = text[..start].split_whitespace().collect();
    (valid && (1..=2).contains(&prefix.len()) && prefix[0].chars().all(|c| c == '*'))
        .then_some(start..end)
}

pub(super) fn prepare(
    hit: &Hit,
    snapshot: &DocumentSnapshot,
    read_only: bool,
) -> Option<InlineValue> {
    match &hit.kind {
        Kind::Checkbox => None,
        Kind::Priority => {
            let config = crate::org_semantic::extract_file_config(snapshot);
            let line = snapshot
                .line_content_range(snapshot.line_index_at(hit.range.start).ok()?)
                .ok()?;
            let prefix = snapshot.copy_range(ByteRange::new(line.start.0, hit.range.start.0));
            if let Some(keyword) = prefix.split_whitespace().nth(1)
                && config.todo_state(keyword).is_none()
            {
                return None;
            }
            let mut choices = vec!["A".into(), "B".into(), "C".into()];
            let mut lines = crate::document::LineCursor::within(
                snapshot,
                ByteRange::new(0, snapshot.len_bytes()),
            )?;
            while let Some(line) = lines.next_line() {
                let text = line.text.trim();
                if let Some((key, rest)) = text.split_once(':')
                    && key.eq_ignore_ascii_case("#+PRIORITIES")
                {
                    let values: Vec<_> = rest.split_whitespace().collect();
                    if values.len() >= 2 {
                        if let (Ok(a), Ok(b)) = (values[0].parse::<u8>(), values[1].parse::<u8>()) {
                            if a <= b && b <= 64 {
                                choices = (a..=b).map(|n| n.to_string()).collect();
                            }
                        } else if values[0].len() == 1 && values[1].len() == 1 {
                            let (a, b) = (values[0].as_bytes()[0], values[1].as_bytes()[0]);
                            if a.is_ascii_uppercase() && b.is_ascii_uppercase() && a <= b {
                                choices = (a..=b).map(|n| (n as char).to_string()).collect();
                            }
                        }
                    }
                }
            }
            Some(InlineValue::Priority(PriorityValue {
                current: hit.source[2..hit.source.len() - 1].into(),
                choices,
            }))
        }
        Kind::Tags => {
            let analysis = crate::org_semantic::analyze(
                snapshot,
                Arc::new(crate::org_syntax::parse(snapshot)),
            );
            let heading = analysis
                .headings
                .iter()
                .find(|h| h.content.start <= hit.range.start && h.content.end >= hit.range.end)?;
            let inherited = heading
                .parent
                .and_then(|id| analysis.headings.iter().find(|h| h.syntax_id == id))
                .map_or(analysis.config.file_tags.as_ref(), |h| {
                    h.effective_tags.as_ref()
                });
            let mut available: Vec<String> = analysis
                .headings
                .iter()
                .flat_map(|h| h.tags.iter())
                .chain(analysis.config.file_tags.iter())
                .map(|t| t.to_string())
                .collect();
            let mut lines = crate::document::LineCursor::within(
                snapshot,
                ByteRange::new(0, snapshot.len_bytes()),
            )?;
            while let Some(line) = lines.next_line() {
                if let Some((key, rest)) = line.text.trim().split_once(':')
                    && key.eq_ignore_ascii_case("#+TAGS")
                {
                    available.extend(
                        rest.split_whitespace()
                            .map(|s| s.split('(').next().unwrap_or(s))
                            .filter(|s| crate::org_semantic::valid_tag(s))
                            .map(str::to_owned),
                    );
                }
            }
            available.sort();
            available.dedup();
            Some(InlineValue::Tags(TagsValue {
                local: heading.tags.iter().map(|s| s.to_string()).collect(),
                inherited: inherited.iter().map(|s| s.to_string()).collect(),
                available,
            }))
        }
        Kind::Link(link) => {
            let mut preview = Vec::new();
            let mut title = None;
            let mut can_open = !matches!(
                link.meta.kind,
                crate::links::LinkKind::Dangerous | crate::links::LinkKind::Other
            );
            if link.meta.kind == crate::links::LinkKind::Internal {
                let destination = crate::editor::links::internal_offset(snapshot, &link.meta.raw);
                can_open = destination.is_some();
                if let Some(offset) = destination {
                    (title, preview) = internal_preview(snapshot, offset);
                }
            }
            Some(InlineValue::Link(LinkValue {
                target: link.meta.raw.to_string(),
                title,
                preview,
                can_open,
                can_edit: !read_only
                    && LiteralDestination::parse(&hit.source, &link.meta.raw).is_some(),
            }))
        }
    }
}
