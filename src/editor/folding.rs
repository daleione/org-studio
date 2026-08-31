use crate::document::{
    DocumentSnapshot, LineCursor, LineIndex, RevisionDelta, RevisionRange, TextSnapshot,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum GlobalVisibility {
    #[default]
    All,
    Overview,
    Contents,
}

impl GlobalVisibility {
    pub(super) fn next(self) -> Self {
        match self {
            Self::All => Self::Overview,
            Self::Overview => Self::Contents,
            Self::Contents => Self::All,
        }
    }
}

#[derive(Default)]
pub(super) struct EditorFoldState {
    headings: Vec<FoldedHeading>,
    pub(super) global: GlobalVisibility,
}

struct FoldedHeading {
    source: RevisionRange,
    visibility: LocalVisibility,
}

#[derive(Clone, Copy)]
struct Heading {
    line: u64,
    level: usize,
    start: crate::document::ByteOffset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalVisibility {
    Folded,
    Children,
}

impl EditorFoldState {
    pub(super) fn toggle_heading(&mut self, snapshot: &DocumentSnapshot, line: u64) {
        let Ok(range) = snapshot.line_content_range(LineIndex(line)) else {
            return;
        };
        self.global = GlobalVisibility::All;
        let existing = self
            .headings
            .iter()
            .position(|heading| heading.source.range.start == range.start);
        match existing.map(|index| self.headings[index].visibility) {
            None => self.headings.push(FoldedHeading {
                source: snapshot.revision_range(range),
                visibility: LocalVisibility::Folded,
            }),
            Some(LocalVisibility::Folded) => {
                self.headings[existing.expect("fold index exists")].visibility =
                    LocalVisibility::Children;
            }
            Some(LocalVisibility::Children) => {
                self.headings.remove(existing.expect("fold index exists"));
            }
        }
    }

    pub(super) fn apply_delta(&mut self, delta: &RevisionDelta) {
        self.headings = self
            .headings
            .drain(..)
            .filter_map(|heading| {
                delta
                    .map_range(heading.source)
                    .ok()
                    .map(|source| FoldedHeading {
                        source,
                        visibility: heading.visibility,
                    })
            })
            .collect();
    }

    pub(super) fn cycle_global(&mut self) {
        self.headings.clear();
        self.global = self.global.next();
    }

    pub(super) fn expand_heading(&mut self, snapshot: &DocumentSnapshot, line: u64) {
        if self.global != GlobalVisibility::All {
            self.global = GlobalVisibility::All;
            self.headings.clear();
        } else if let Ok(range) = snapshot.line_content_range(LineIndex(line)) {
            self.headings
                .retain(|heading| heading.source.range.start != range.start);
        }
    }

    pub(super) fn hidden_ranges(&self, snapshot: &DocumentSnapshot) -> Vec<std::ops::Range<u64>> {
        if self.global == GlobalVisibility::All && self.headings.is_empty() {
            return Vec::new();
        }
        let headings = headings(snapshot);
        let mut hidden = Vec::new();
        match self.global {
            GlobalVisibility::All => {
                for folded in &self.headings {
                    if let Some((index, heading)) = headings
                        .iter()
                        .enumerate()
                        .find(|(_, heading)| heading.start == folded.source.range.start)
                    {
                        let end = headings[index + 1..]
                            .iter()
                            .find(|candidate| candidate.level <= heading.level)
                            .map_or(snapshot.len_lines(), |heading| heading.line);
                        if heading.line + 1 < end {
                            match folded.visibility {
                                LocalVisibility::Folded => hidden.push(heading.line + 1..end),
                                LocalVisibility::Children => {
                                    let children = headings[index + 1..]
                                        .iter()
                                        .take_while(|candidate| candidate.line < end)
                                        .filter(|candidate| candidate.level == heading.level + 1)
                                        .map(|heading| heading.line);
                                    let mut cursor = heading.line + 1;
                                    for child in children {
                                        if cursor < child {
                                            hidden.push(cursor..child);
                                        }
                                        cursor = child + 1;
                                    }
                                    if cursor < end {
                                        hidden.push(cursor..end);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            GlobalVisibility::Overview => {
                let top = headings.iter().map(|heading| heading.level).min();
                hide_non_matching(
                    snapshot.len_lines(),
                    &headings,
                    |level| Some(level) == top,
                    &mut hidden,
                );
            }
            GlobalVisibility::Contents => {
                hide_non_matching(snapshot.len_lines(), &headings, |_| true, &mut hidden);
            }
        }
        hidden.sort_by_key(|range| range.start);
        merge(hidden)
    }
}

fn headings(snapshot: &DocumentSnapshot) -> Vec<Heading> {
    let mut cursor = LineCursor::new(snapshot);
    let mut headings = Vec::new();
    let mut line = 0;
    while let Some(source) = cursor.next_line() {
        let text = source.text.as_ref();
        let mut end = text.len().min(256);
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        let trimmed = text[..end].trim_start();
        let level = trimmed.bytes().take_while(|byte| *byte == b'*').count();
        if level > 0 && trimmed.as_bytes().get(level) == Some(&b' ') {
            headings.push(Heading {
                line,
                level,
                start: source.range.start,
            });
        }
        line += 1;
    }
    headings
}

fn hide_non_matching(
    lines: u64,
    headings: &[Heading],
    keep: impl Fn(usize) -> bool,
    hidden: &mut Vec<std::ops::Range<u64>>,
) {
    let mut cursor = 0;
    for heading in headings {
        if cursor < heading.line {
            hidden.push(cursor..heading.line);
        }
        if !keep(heading.level) {
            hidden.push(heading.line..heading.line + 1);
        }
        cursor = heading.line + 1;
    }
    if cursor < lines {
        hidden.push(cursor..lines);
    }
}

fn merge(ranges: Vec<std::ops::Range<u64>>) -> Vec<std::ops::Range<u64>> {
    let mut merged: Vec<std::ops::Range<u64>> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ByteRange, DocumentBuffer, EditTransaction, TextEdit};

    #[test]
    fn local_and_global_visibility_hide_only_expected_source_lines() {
        let snapshot = DocumentSnapshot::from_utf8(
            b"intro\n* One\nbody\n** Child\nchild body\n* Two\ntail\n".to_vec(),
        )
        .unwrap();
        let mut folds = EditorFoldState::default();
        folds.toggle_heading(&snapshot, 1);
        assert_eq!(folds.hidden_ranges(&snapshot), vec![2..5]);
        folds.toggle_heading(&snapshot, 1);
        assert_eq!(folds.hidden_ranges(&snapshot), vec![2..3, 4..5]);
        folds.toggle_heading(&snapshot, 1);
        assert!(folds.hidden_ranges(&snapshot).is_empty());
        folds.toggle_heading(&snapshot, 1);
        folds.cycle_global();
        assert_eq!(folds.global, GlobalVisibility::Overview);
        assert_eq!(folds.hidden_ranges(&snapshot), vec![0..1, 2..5, 6..8]);
        folds.cycle_global();
        assert_eq!(folds.global, GlobalVisibility::Contents);
        assert_eq!(folds.hidden_ranges(&snapshot), vec![0..1, 2..3, 4..5, 6..8]);
        folds.cycle_global();
        assert!(folds.hidden_ranges(&snapshot).is_empty());
    }

    #[test]
    fn local_fold_follows_its_heading_across_edits_above_it() {
        let source = b"intro\n* One\nbody\n** Child\nchild body\n* Two\ntail\n".to_vec();
        let mut buffer = DocumentBuffer::from_utf8(source).unwrap();
        let snapshot = buffer.snapshot();
        let mut folds = EditorFoldState::default();
        folds.toggle_heading(&snapshot, 1);

        let delta = buffer
            .commit(EditTransaction::new(
                snapshot.revision(),
                vec![TextEdit::new(ByteRange::new(0, 0), "new\n")],
            ))
            .unwrap();
        folds.apply_delta(&delta);

        assert_eq!(folds.hidden_ranges(&buffer.snapshot()), vec![3..6]);
    }
}
