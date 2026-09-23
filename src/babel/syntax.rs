use crate::{
    document::{ByteOffset, ByteRange, DocumentSnapshot, TextSnapshot},
    org_syntax::{BlockArena, BlockKind},
};

pub(super) fn existing_result_range(
    snapshot: &DocumentSnapshot,
    blocks: &BlockArena,
    source_index: usize,
) -> ByteRange {
    let source = &blocks.nodes()[source_index];
    let mut index = source_index + 1;
    while blocks.nodes().get(index).is_some_and(|candidate| {
        candidate.parent == source.parent && matches!(candidate.kind, BlockKind::BlankLine)
    }) {
        index += 1;
    }
    let Some(marker) = blocks.nodes().get(index).filter(|candidate| {
        candidate.parent == source.parent
            && matches!(candidate.kind, BlockKind::Keyword)
            && is_results_marker(&snapshot.copy_range(candidate.content))
    }) else {
        return ByteRange::new(source.source.end.0, source.source.end.0);
    };
    let mut end = marker.source.end;
    let mut payload_index = index + 1;
    // Affiliated keywords such as `#+ATTR_ORG:` sit between the marker and the
    // image; they belong to the results block so a re-run may rewrite them.
    while let Some(keyword) = blocks.nodes().get(payload_index).filter(|candidate| {
        candidate.parent == source.parent
            && matches!(candidate.kind, BlockKind::Keyword)
            && candidate.source.start >= end
            && crate::org_syntax::attributes::is_affiliated_keyword(
                &snapshot.copy_range(candidate.content),
            )
    }) {
        end = keyword.source.end;
        payload_index += 1;
    }
    if let Some(payload) = blocks
        .nodes()
        .get(payload_index)
        .filter(|candidate| candidate.parent == source.parent && candidate.source.start >= end)
    {
        let payload_source = snapshot.copy_range(payload.source);
        let line = payload_source
            .split_inclusive('\n')
            .next()
            .unwrap_or(&payload_source);
        let trimmed = line.trim();
        if trimmed.starts_with("[[file:") && trimmed.ends_with("]]") {
            end = ByteOffset(payload.source.start.0 + line.len() as u64);
        } else if matches!(payload.kind, BlockKind::FixedWidth) {
            // The syntax tree stores each fixed-width result line separately.
            // Replacing only the first node leaves the remaining stdout behind.
            for line in blocks.nodes()[payload_index..].iter() {
                if line.parent != source.parent
                    || !matches!(line.kind, BlockKind::FixedWidth)
                    || line.source.start != end
                {
                    break;
                }
                end = line.source.end;
            }
        }
    }
    ByteRange {
        start: source.source.end,
        end,
    }
}

/// Keeps an `#+ATTR_ORG:` line attached to the results image.
pub(super) fn preserved_result_attributes(existing: &str) -> Option<String> {
    existing
        .lines()
        .map(str::trim_end)
        .find(|line| crate::org_syntax::attributes::is_attr_org_line(line))
        .map(str::to_owned)
}

fn is_results_marker(source: &str) -> bool {
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

pub(super) fn header_values<'a>(tokens: &'a [String], name: &str) -> Vec<&'a str> {
    let Some(start) = tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case(name))
    else {
        return Vec::new();
    };
    tokens[start + 1..]
        .iter()
        .take_while(|token| !token.starts_with(':'))
        .map(String::as_str)
        .collect()
}

pub(super) fn tokenize_header(source: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in source.trim().chars() {
        if escaped {
            token.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else {
                token.push(character);
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character.is_whitespace() {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
        } else {
            token.push(character);
        }
    }
    if escaped {
        token.push('\\');
    }
    if quote.is_some() {
        return Err("Unclosed quote in source block header".to_owned());
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    Ok(tokens)
}
