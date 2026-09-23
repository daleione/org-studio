use crate::{
    document::{ByteRange, DocumentSnapshot, TextSnapshot},
    org_syntax::{BlockArena, BlockKind},
};

use super::syntax::{header_argument, tokenize_header};

pub(super) struct SourceBlock {
    node_index: usize,
    pub(super) language: String,
    name: Option<String>,
    noweb_ref: Option<String>,
    pub(super) tokens: Result<Vec<String>, String>,
    pub(super) has_tangle_argument: bool,
    pub(super) body: String,
}

pub(super) fn expanded_execution_source(
    snapshot: &DocumentSnapshot,
    arena: &BlockArena,
    node_index: usize,
) -> Result<String, String> {
    let blocks = source_blocks(snapshot, arena);
    let index = blocks
        .iter()
        .position(|block| block.node_index == node_index)
        .ok_or_else(|| "Source block is no longer available".to_owned())?;
    expand(&blocks, index, &mut Vec::new())
}

pub(super) fn source_blocks(snapshot: &DocumentSnapshot, arena: &BlockArena) -> Vec<SourceBlock> {
    let text = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()));
    arena
        .nodes()
        .iter()
        .enumerate()
        .filter_map(|(node_index, node)| {
            let BlockKind::SourceBlock { language } = &node.kind else {
                return None;
            };
            let opening = &text[node.source.start.0 as usize..node.content.start.0 as usize];
            let tokens = tokenize_header(opening);
            let before = &text[..node.source.start.0 as usize];
            let previous = before
                .trim_end_matches(['\r', '\n'])
                .rsplit('\n')
                .next()
                .unwrap_or("")
                .trim();
            let name = previous
                .get(..7)
                .filter(|prefix| prefix.eq_ignore_ascii_case("#+name:"))
                .map(|_| previous[7..].trim().to_owned())
                .filter(|name| !name.is_empty());
            let has_tangle_argument = opening
                .split_whitespace()
                .any(|word| word.eq_ignore_ascii_case(":tangle"));
            let noweb_ref = tokens
                .as_ref()
                .ok()
                .and_then(|tokens| header_argument(tokens, ":noweb-ref"))
                .map(str::to_owned);
            Some(SourceBlock {
                node_index,
                language: language.as_deref().unwrap_or("").to_owned(),
                name,
                noweb_ref,
                tokens,
                has_tangle_argument,
                body: text[node.content.start.0 as usize..node.content.end.0 as usize].to_owned(),
            })
        })
        .collect()
}

pub(super) fn noweb_enabled(tokens: &[String], phase: &str) -> bool {
    header_argument(tokens, ":noweb")
        .is_some_and(|mode| mode.eq_ignore_ascii_case("yes") || mode.eq_ignore_ascii_case(phase))
}

pub(super) fn expand(
    blocks: &[SourceBlock],
    index: usize,
    stack: &mut Vec<usize>,
) -> Result<String, String> {
    blocks[index].tokens.as_ref().map_err(Clone::clone)?;
    if stack.contains(&index) {
        return Err(format!(
            "Noweb cycle involving {}",
            blocks[index]
                .name
                .as_deref()
                .or(blocks[index].noweb_ref.as_deref())
                .unwrap_or("source block")
        ));
    }
    stack.push(index);
    let body = &blocks[index].body;
    let mut output = String::new();
    let mut rest = body.as_str();
    while let Some(start) = rest.find("<<") {
        let Some(end) = rest[start + 2..].find(">>") else {
            break;
        };
        let name = &rest[start + 2..start + 2 + end];
        output.push_str(&rest[..start]);
        let references: Vec<_> = blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| {
                block.name.as_deref() == Some(name) || block.noweb_ref.as_deref() == Some(name)
            })
            .map(|(index, _)| index)
            .collect();
        if references.is_empty() {
            return Err(format!("Unknown Noweb reference <<{name}>>"));
        }
        let mut expanded = String::new();
        for reference in references {
            if !expanded.is_empty() && !expanded.ends_with('\n') {
                expanded.push('\n');
            }
            expanded.push_str(&expand(blocks, reference, stack)?);
        }
        let after_reference = &rest[start + 2 + end + 2..];
        if after_reference.starts_with('\n') && expanded.ends_with('\n') {
            expanded.pop();
        }
        let indent = output.rsplit('\n').next().unwrap_or("");
        if indent.chars().all(char::is_whitespace) && !indent.is_empty() {
            expanded = expanded.replace('\n', &format!("\n{indent}"));
        }
        output.push_str(&expanded);
        rest = after_reference;
    }
    output.push_str(rest);
    stack.pop();
    Ok(output)
}
