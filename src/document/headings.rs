use std::sync::{Arc, OnceLock};

use crate::org_syntax::{BlockKind, parse};

use super::markdown::{atx_heading, fence_close, fence_open};
use super::{ByteOffset, DocumentFormat, DocumentSnapshot, LineCursor, TextSnapshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DocumentHeading {
    pub(crate) line: u64,
    pub(crate) level: u16,
    pub(crate) start: ByteOffset,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OutlineEntry {
    pub(crate) title: Arc<str>,
    pub(crate) level: u16,
    pub(crate) line: u64,
    pub(crate) source: super::RevisionRange,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct HeadingIndex {
    headings: Arc<[DocumentHeading]>,
    paths: OnceLock<Arc<[Arc<str>]>>,
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
            paths: OnceLock::new(),
        }
    }

    // Built lazily from the same revision-cached index used by editor folding.
    fn paths(&self, snapshot: &DocumentSnapshot) -> Arc<[Arc<str>]> {
        if let Some(paths) = self.paths.get() {
            return paths.clone();
        }
        let mut stack: Vec<(u16, Arc<str>)> = Vec::new();
        let mut paths = Vec::with_capacity(self.headings.len());
        for heading in self.headings.iter() {
            while stack
                .last()
                .is_some_and(|(level, _)| *level >= heading.level)
            {
                stack.pop();
            }
            let title = snapshot
                .line_range(super::LineIndex(heading.line))
                .map(|range| snapshot.copy_range(range))
                .unwrap_or_default();
            let title = title.trim().trim_start_matches(['*', '#']).trim();
            let path: Arc<str> = match stack.last() {
                Some((_, parent)) => format!("{parent} / {title}").into(),
                None => title.into(),
            };
            stack.push((heading.level, path.clone()));
            paths.push(path);
        }
        let paths: Arc<[Arc<str>]> = paths.into();
        let _ = self.paths.set(paths.clone());
        paths
    }

    pub(crate) fn outline_at(
        &self,
        snapshot: &DocumentSnapshot,
        offset: ByteOffset,
    ) -> Option<Arc<str>> {
        let index = self
            .headings
            .partition_point(|heading| heading.start <= offset)
            .checked_sub(1)?;
        self.paths(snapshot).get(index).cloned()
    }

    pub(crate) fn outline_entries(&self, snapshot: &DocumentSnapshot) -> Vec<OutlineEntry> {
        self.headings
            .iter()
            .map(|heading| {
                let text = snapshot
                    .line_range(super::LineIndex(heading.line))
                    .map(|range| snapshot.copy_range(range))
                    .unwrap_or_default();
                OutlineEntry {
                    title: text.trim().trim_start_matches(['*', '#']).trim().into(),
                    level: heading.level,
                    line: heading.line + 1,
                    source: super::RevisionRange::new(
                        snapshot.revision(),
                        super::ByteRange::new(heading.start.0, heading.start.0),
                    ),
                }
            })
            .collect()
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
        if let Some((marker, opening_count)) = fence {
            if fence_close(logical, marker, opening_count) {
                fence = None;
            }
        } else if let Some((marker, count, _)) = fence_open(logical) {
            fence = Some((marker, count));
        } else if let Some((level, _)) = atx_heading(trimmed) {
            headings.push(DocumentHeading {
                line: line_number,
                level,
                start: line.range.start,
            });
        }
        line_number += 1;
    }
    headings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_entries_keep_literal_slashes_and_actual_heading_levels() {
        for (format, text) in [
            (
                DocumentFormat::Org,
                "* Parent / literal\nbody\n*** Child / detail\n",
            ),
            (
                DocumentFormat::Markdown,
                "# Parent / literal\nbody\n### Child / detail\n",
            ),
        ] {
            let snapshot = DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap();
            let index = HeadingIndex::parse(format, &snapshot);
            let entries = index.outline_entries(&snapshot);
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].title.as_ref(), "Parent / literal");
            assert_eq!(entries[1].title.as_ref(), "Child / detail");
            assert_eq!(entries[1].level, 3);
            assert_eq!(entries[1].line, 3);
            assert_eq!(
                entries[1].source.range.start.0,
                text.find("body").unwrap() as u64 + 5
            );
        }
    }

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
