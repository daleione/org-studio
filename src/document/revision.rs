use std::{collections::VecDeque, sync::Arc};

use super::ByteRange;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Revision(pub u64);

impl Revision {
    pub const INITIAL: Self = Self(0);

    pub fn checked_next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextEditSummary {
    pub old: ByteRange,
    pub new_len: u64,
}

impl TextEditSummary {
    pub fn new(old: ByteRange, new_len: u64) -> Self {
        Self { old, new_len }
    }

    fn old_len(self) -> u64 {
        self.old.end.0.saturating_sub(self.old.start.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionDelta {
    pub before: Revision,
    pub after: Revision,
    pub edits: Arc<[TextEditSummary]>,
}

impl RevisionDelta {
    pub fn new(
        before: Revision,
        after: Revision,
        edits: impl Into<Arc<[TextEditSummary]>>,
    ) -> Result<Self, EditLogError> {
        if before.checked_next() != Some(after) {
            return Err(EditLogError::InvalidRevisionStep { before, after });
        }
        let edits = edits.into();
        let mut previous_end = 0;
        for (index, edit) in edits.iter().enumerate() {
            if edit.old.start > edit.old.end {
                return Err(EditLogError::InvalidRange(edit.old));
            }
            if index > 0
                && (edit.old.start.0 < previous_end
                    || (edit.old.start.0 == previous_end
                        && edit.old.start == edit.old.end
                        && edits[index - 1].old.start == edits[index - 1].old.end))
            {
                return Err(EditLogError::OverlappingEdits);
            }
            previous_end = edit.old.end.0;
        }
        Ok(Self {
            before,
            after,
            edits,
        })
    }

    pub fn map_range(&self, range: RevisionRange) -> Result<RevisionRange, RangeMapError> {
        if range.revision != self.before {
            return Err(RangeMapError::RevisionMismatch {
                expected: self.before,
                actual: range.revision,
            });
        }

        let mut start = range.range.start.0;
        let mut end = range.range.end.0;
        let original_start = start;
        let original_end = end;
        let mut shift = 0_i128;
        for edit in self.edits.iter().copied() {
            let edit_start = edit.old.start.0;
            let edit_end = edit.old.end.0;
            let insertion = edit_start == edit_end;
            let entirely_before = edit_end < original_start
                || (edit_end == original_start && (!insertion || original_start != original_end));
            let entirely_after = edit_start >= original_end;

            if entirely_before {
                shift += i128::from(edit.new_len) - i128::from(edit.old_len());
            } else if !entirely_after {
                return Err(RangeMapError::Intersected);
            }
        }
        start = apply_shift(start, shift).ok_or(RangeMapError::OffsetOverflow)?;
        end = apply_shift(end, shift).ok_or(RangeMapError::OffsetOverflow)?;
        Ok(RevisionRange {
            revision: self.after,
            range: ByteRange::new(start, end),
        })
    }
}

fn apply_shift(offset: u64, shift: i128) -> Option<u64> {
    u64::try_from(i128::from(offset) + shift).ok()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionRange {
    pub revision: Revision,
    pub range: ByteRange,
}

impl RevisionRange {
    pub fn new(revision: Revision, range: ByteRange) -> Self {
        Self { revision, range }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeMapError {
    RevisionMismatch {
        expected: Revision,
        actual: Revision,
    },
    Intersected,
    HistoryExpired,
    FutureRevision,
    OffsetOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditLogError {
    ZeroCapacity,
    InvalidRevisionStep {
        before: Revision,
        after: Revision,
    },
    NonContiguousRevision {
        expected: Revision,
        actual: Revision,
    },
    InvalidRange(ByteRange),
    OverlappingEdits,
}

#[derive(Clone, Debug)]
pub struct EditLog {
    capacity: usize,
    deltas: VecDeque<RevisionDelta>,
}

impl EditLog {
    pub fn new(capacity: usize) -> Result<Self, EditLogError> {
        if capacity == 0 {
            return Err(EditLogError::ZeroCapacity);
        }
        Ok(Self {
            capacity,
            deltas: VecDeque::with_capacity(capacity),
        })
    }

    pub fn push(&mut self, delta: RevisionDelta) -> Result<(), EditLogError> {
        if let Some(latest) = self.deltas.back()
            && latest.after != delta.before
        {
            return Err(EditLogError::NonContiguousRevision {
                expected: latest.after,
                actual: delta.before,
            });
        }
        if self.deltas.len() == self.capacity {
            self.deltas.pop_front();
        }
        self.deltas.push_back(delta);
        Ok(())
    }

    pub fn latest_revision(&self) -> Option<Revision> {
        self.deltas.back().map(|delta| delta.after)
    }

    pub fn can_append_without_expiring(&self, count: usize) -> bool {
        self.deltas.len().saturating_add(count) <= self.capacity
    }

    pub fn map_range(
        &self,
        mut range: RevisionRange,
        target: Revision,
    ) -> Result<RevisionRange, RangeMapError> {
        if target < range.revision {
            return Err(RangeMapError::FutureRevision);
        }
        if target == range.revision {
            return Ok(range);
        }
        let first = self.deltas.front().ok_or(RangeMapError::HistoryExpired)?;
        if range.revision < first.before {
            return Err(RangeMapError::HistoryExpired);
        }
        for delta in &self.deltas {
            if delta.before < range.revision {
                continue;
            }
            if delta.before != range.revision {
                return Err(RangeMapError::HistoryExpired);
            }
            range = delta.map_range(range)?;
            if range.revision == target {
                return Ok(range);
            }
            if range.revision > target {
                return Err(RangeMapError::FutureRevision);
            }
        }
        Err(RangeMapError::FutureRevision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(before: u64, old: (u64, u64), new_len: u64) -> RevisionDelta {
        RevisionDelta::new(
            Revision(before),
            Revision(before + 1),
            vec![TextEditSummary::new(ByteRange::new(old.0, old.1), new_len)],
        )
        .unwrap()
    }

    #[test]
    fn maps_ranges_after_edits_and_preserves_ranges_before_them() {
        let change = delta(0, (2, 4), 5);
        let before = RevisionRange::new(Revision(0), ByteRange::new(0, 2));
        let after = RevisionRange::new(Revision(0), ByteRange::new(8, 12));
        assert_eq!(
            change.map_range(before).unwrap().range,
            ByteRange::new(0, 2)
        );
        assert_eq!(
            change.map_range(after).unwrap().range,
            ByteRange::new(11, 15)
        );
    }

    #[test]
    fn rejects_ranges_intersected_by_replace_or_interior_insert() {
        let range = RevisionRange::new(Revision(0), ByteRange::new(5, 10));
        assert_eq!(
            delta(0, (7, 8), 1).map_range(range),
            Err(RangeMapError::Intersected)
        );
        assert_eq!(
            delta(0, (7, 7), 1).map_range(range),
            Err(RangeMapError::Intersected)
        );
    }

    #[test]
    fn boundary_insertions_follow_half_open_range_semantics() {
        let range = RevisionRange::new(Revision(0), ByteRange::new(5, 10));
        assert_eq!(
            delta(0, (5, 5), 2).map_range(range).unwrap().range,
            ByteRange::new(7, 12)
        );
        assert_eq!(
            delta(0, (10, 10), 2).map_range(range).unwrap().range,
            ByteRange::new(5, 10)
        );
    }

    #[test]
    fn maps_through_a_bounded_contiguous_history() {
        let mut log = EditLog::new(2).unwrap();
        log.push(delta(0, (0, 0), 2)).unwrap();
        log.push(delta(1, (20, 20), 1)).unwrap();
        let range = RevisionRange::new(Revision(0), ByteRange::new(5, 10));
        let mapped = log.map_range(range, Revision(2)).unwrap();
        assert_eq!(mapped.range, ByteRange::new(7, 12));

        log.push(delta(2, (30, 30), 1)).unwrap();
        assert_eq!(
            log.map_range(range, Revision(3)),
            Err(RangeMapError::HistoryExpired)
        );
    }

    #[test]
    fn rejects_overlapping_edits_and_revision_gaps() {
        let edits = vec![
            TextEditSummary::new(ByteRange::new(2, 5), 0),
            TextEditSummary::new(ByteRange::new(4, 6), 0),
        ];
        assert_eq!(
            RevisionDelta::new(Revision(0), Revision(1), edits),
            Err(EditLogError::OverlappingEdits)
        );
        let mut log = EditLog::new(2).unwrap();
        log.push(delta(0, (0, 0), 1)).unwrap();
        assert_eq!(
            log.push(delta(2, (0, 0), 1)),
            Err(EditLogError::NonContiguousRevision {
                expected: Revision(1),
                actual: Revision(2),
            })
        );
    }
}
