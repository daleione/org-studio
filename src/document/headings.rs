use std::sync::Arc;

use crate::org_syntax::{BlockKind, parse};

use super::markdown::{atx_heading, fence_start, is_closing_fence};
use super::{ByteOffset, DocumentFormat, DocumentSnapshot, LineCursor, TextSnapshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DocumentHeading {
    pub(crate) line: u64,
    pub(crate) level: u16,
    pub(crate) start: ByteOffset,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct HeadingIndex {
    headings: Arc<[DocumentHeading]>,
}

impl HeadingIndex {
    pub(crate) fn parse(format: DocumentFormat, snapshot: &DocumentSnapshot) -> Self {
        let headings = match format {
            DocumentFormat::Org => parse(snapshot)
                .nodes()
                .iter()
                .filter_map(|node| match node.kind {
                    BlockKind::Heading { level } => Some(DocumentHeading {
                        line: snapshot.line_of_byte(node.source.start),
                        level,
                        start: node.source.start,
                    }),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            DocumentFormat::Markdown => markdown_headings(snapshot),
        };
        Self {
            headings: headings.into(),
        }
    }

    pub(crate) fn as_slice(&self) -> &[DocumentHeading] {
        &self.headings
    }

    pub(crate) fn heading_at_line(&self, line: u64) -> Option<DocumentHeading> {
        self.headings
            .binary_search_by_key(&line, |heading| heading.line)
            .ok()
            .map(|index| self.headings[index])
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.headings.is_empty()
    }
}

fn markdown_headings(snapshot: &DocumentSnapshot) -> Vec<DocumentHeading> {
    let mut headings = Vec::new();
    let mut cursor = LineCursor::new(snapshot);
    let mut fence = None::<(char, usize)>;
    let mut line_number = 0;
    while let Some(line) = cursor.next_line() {
        let logical = line.text.trim_end_matches(['\r', '\n']);
        let trimmed = logical.trim_start();
        let leading = logical.len() - trimmed.len();
        if let Some((marker, opening_count)) = fence {
            if leading <= 3 && is_closing_fence(trimmed, marker, opening_count) {
                fence = None;
            }
        } else if leading <= 3 {
            if let Some((marker, count, _)) = fence_start(trimmed) {
                fence = Some((marker, count));
            } else if let Some((level, _)) = atx_heading(trimmed) {
                headings.push(DocumentHeading {
                    line: line_number,
                    level,
                    start: line.range.start,
                });
            }
        }
        line_number += 1;
    }
    headings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_heading_index_ignores_fenced_heading_text() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"# One\n```md\n# literal\n```\n  ## Two\n####### not heading\n".to_vec(),
        )
        .unwrap();
        let index = HeadingIndex::parse(DocumentFormat::Markdown, &snapshot);
        assert_eq!(
            index.as_slice(),
            &[
                DocumentHeading {
                    line: 0,
                    level: 1,
                    start: ByteOffset(0),
                },
                DocumentHeading {
                    line: 4,
                    level: 2,
                    start: ByteOffset(26),
                },
            ]
        );
    }
}
