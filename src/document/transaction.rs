use std::sync::Arc;

use ropey::Rope;

use super::{
    ByteRange, EditLog, EditLogError, Revision, RevisionDelta, RopeSnapshot, TextEditSummary,
};

const DEFAULT_EDIT_LOG_CAPACITY: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEdit {
    pub range: ByteRange,
    pub replacement: String,
}

impl TextEdit {
    pub fn new(range: ByteRange, replacement: impl Into<String>) -> Self {
        Self {
            range,
            replacement: replacement.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditTransaction {
    pub base_revision: Revision,
    pub edits: Vec<TextEdit>,
}

impl EditTransaction {
    pub fn new(base_revision: Revision, edits: Vec<TextEdit>) -> Self {
        Self {
            base_revision,
            edits,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditError {
    InvalidUtf8 {
        valid_up_to: usize,
    },
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    RevisionExhausted,
    InvalidRange(ByteRange),
    InvalidUtf8Boundary(ByteRange),
    OverlappingEdits,
    EditLog(EditLogError),
}

pub struct DocumentBuffer {
    rope: Rope,
    revision: Revision,
    edit_log: EditLog,
}

impl DocumentBuffer {
    pub fn from_utf8(bytes: Vec<u8>) -> Result<Self, EditError> {
        Self::with_edit_log_capacity(bytes, DEFAULT_EDIT_LOG_CAPACITY)
    }

    pub fn with_edit_log_capacity(bytes: Vec<u8>, capacity: usize) -> Result<Self, EditError> {
        let text = String::from_utf8(bytes).map_err(|error| EditError::InvalidUtf8 {
            valid_up_to: error.utf8_error().valid_up_to(),
        })?;
        Ok(Self {
            rope: Rope::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text)),
            revision: Revision::INITIAL,
            edit_log: EditLog::new(capacity).map_err(EditError::EditLog)?,
        })
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn snapshot(&self) -> RopeSnapshot {
        RopeSnapshot::from_rope(self.rope.clone(), self.revision)
    }

    pub fn edit_log(&self) -> &EditLog {
        &self.edit_log
    }

    pub fn commit(&mut self, mut transaction: EditTransaction) -> Result<RevisionDelta, EditError> {
        if transaction.base_revision != self.revision {
            return Err(EditError::StaleRevision {
                expected: self.revision,
                actual: transaction.base_revision,
            });
        }
        let after = self
            .revision
            .checked_next()
            .ok_or(EditError::RevisionExhausted)?;
        transaction.edits.sort_by_key(|edit| edit.range.start);
        self.validate_edits(&transaction.edits)?;

        let summaries = transaction
            .edits
            .iter()
            .map(|edit| TextEditSummary::new(edit.range, edit.replacement.len() as u64))
            .collect::<Vec<_>>();
        let delta = RevisionDelta::new(self.revision, after, Arc::from(summaries))
            .map_err(EditError::EditLog)?;

        for edit in transaction.edits.iter().rev() {
            let start = self.rope.byte_to_char(edit.range.start.0 as usize);
            let end = self.rope.byte_to_char(edit.range.end.0 as usize);
            if start != end {
                self.rope.remove(start..end);
            }
            if !edit.replacement.is_empty() {
                self.rope.insert(start, &edit.replacement);
            }
        }
        self.edit_log
            .push(delta.clone())
            .map_err(EditError::EditLog)?;
        self.revision = after;
        Ok(delta)
    }

    fn validate_edits(&self, edits: &[TextEdit]) -> Result<(), EditError> {
        let len = self.rope.len_bytes() as u64;
        let mut previous: Option<ByteRange> = None;
        for edit in edits {
            if edit.range.start > edit.range.end || edit.range.end.0 > len {
                return Err(EditError::InvalidRange(edit.range));
            }
            if !is_char_boundary(&self.rope, edit.range.start.0)
                || !is_char_boundary(&self.rope, edit.range.end.0)
            {
                return Err(EditError::InvalidUtf8Boundary(edit.range));
            }
            if let Some(previous) = previous
                && (edit.range.start < previous.end
                    || (edit.range.start == previous.start
                        && edit.range.start == edit.range.end
                        && previous.start == previous.end))
            {
                return Err(EditError::OverlappingEdits);
            }
            previous = Some(edit.range);
        }
        Ok(())
    }
}

fn is_char_boundary(rope: &Rope, offset: u64) -> bool {
    let Ok(offset) = usize::try_from(offset) else {
        return false;
    };
    if offset == rope.len_bytes() {
        return true;
    }
    if offset > rope.len_bytes() {
        return false;
    }
    let (chunk, chunk_start, _, _) = rope.chunk_at_byte(offset);
    chunk.is_char_boundary(offset - chunk_start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ByteRange, RangeMapError, RevisionRange, TextSnapshot};

    fn text(snapshot: &RopeSnapshot) -> String {
        snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
    }

    #[test]
    fn commits_multiple_edits_atomically_and_publishes_a_delta() {
        let mut document = DocumentBuffer::from_utf8(b"alpha beta".to_vec()).unwrap();
        let delta = document
            .commit(EditTransaction::new(
                Revision::INITIAL,
                vec![
                    TextEdit::new(ByteRange::new(0, 5), "A"),
                    TextEdit::new(ByteRange::new(6, 10), "B"),
                ],
            ))
            .unwrap();
        assert_eq!(text(&document.snapshot()), "A B");
        assert_eq!(delta.before, Revision(0));
        assert_eq!(delta.after, Revision(1));
        assert_eq!(delta.edits.len(), 2);
    }

    #[test]
    fn rejects_stale_overlapping_and_non_utf8_boundary_edits_without_mutating() {
        let mut document = DocumentBuffer::from_utf8("a🦀b".as_bytes().to_vec()).unwrap();
        let original = text(&document.snapshot());
        let invalid =
            EditTransaction::new(Revision(0), vec![TextEdit::new(ByteRange::new(2, 3), "x")]);
        assert!(matches!(
            document.commit(invalid),
            Err(EditError::InvalidUtf8Boundary(_))
        ));
        assert_eq!(text(&document.snapshot()), original);
        assert_eq!(document.revision(), Revision(0));

        let overlapping = EditTransaction::new(
            Revision(0),
            vec![
                TextEdit::new(ByteRange::new(0, 1), ""),
                TextEdit::new(ByteRange::new(0, 1), ""),
            ],
        );
        assert_eq!(
            document.commit(overlapping),
            Err(EditError::OverlappingEdits)
        );
        assert!(matches!(
            document.commit(EditTransaction::new(Revision(9), Vec::new())),
            Err(EditError::StaleRevision { .. })
        ));
        assert_eq!(text(&document.snapshot()), original);
    }

    #[test]
    fn edit_log_maps_unchanged_projection_ranges_to_the_latest_snapshot() {
        let mut document = DocumentBuffer::from_utf8(b"table row".to_vec()).unwrap();
        let range = RevisionRange::new(Revision(0), ByteRange::new(6, 9));
        document
            .commit(EditTransaction::new(
                Revision(0),
                vec![TextEdit::new(ByteRange::new(0, 0), "new ")],
            ))
            .unwrap();
        assert_eq!(
            document
                .edit_log()
                .map_range(range, document.revision())
                .unwrap()
                .range,
            ByteRange::new(10, 13)
        );

        let touched = RevisionRange::new(Revision(1), ByteRange::new(10, 13));
        document
            .commit(EditTransaction::new(
                Revision(1),
                vec![TextEdit::new(ByteRange::new(11, 11), "x")],
            ))
            .unwrap();
        assert_eq!(
            document.edit_log().map_range(touched, document.revision()),
            Err(RangeMapError::Intersected)
        );
    }
}
