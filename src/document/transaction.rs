use std::sync::{Arc, OnceLock};

use ropey::Rope;

use super::{
    ByteRange, CoordinateCheckpoint, DocumentId, DocumentSnapshot, EditLog, EditLogError, Revision,
    RevisionDelta, TextEditSummary,
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
    ReadOnly,
    InvalidTransientEdit,
    InvalidUtf8 {
        valid_up_to: usize,
    },
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    RevisionExhausted,
    EmptyTransaction,
    InvalidRange(ByteRange),
    InvalidUtf8Boundary(ByteRange),
    OverlappingEdits,
    EditLog(EditLogError),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadOnly => formatter.write_str("buffer is read-only"),
            Self::InvalidTransientEdit => formatter.write_str("invalid transient edit token"),
            Self::InvalidUtf8 { valid_up_to } => {
                write!(formatter, "file is not valid UTF-8 near byte {valid_up_to}")
            }
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "stale edit revision: expected {}, got {}",
                expected.0, actual.0
            ),
            Self::RevisionExhausted => formatter.write_str("document revision space exhausted"),
            Self::EmptyTransaction => formatter.write_str("edit transaction contains no edits"),
            Self::InvalidRange(range) => write!(formatter, "invalid byte range {range:?}"),
            Self::InvalidUtf8Boundary(range) => {
                write!(
                    formatter,
                    "byte range is not on UTF-8 boundaries: {range:?}"
                )
            }
            Self::OverlappingEdits => formatter.write_str("text edits overlap"),
            Self::EditLog(error) => write!(formatter, "invalid edit log update: {error:?}"),
        }
    }
}

impl std::error::Error for EditError {}

pub struct DocumentBuffer {
    id: DocumentId,
    rope: Rope,
    revision: Revision,
    edit_log: EditLog,
    coordinate_index: Arc<OnceLock<Arc<[CoordinateCheckpoint]>>>,
}

pub(super) struct PreparedText {
    rope: Rope,
    coordinate_index: Arc<OnceLock<Arc<[CoordinateCheckpoint]>>>,
}

impl PreparedText {
    pub(super) fn from_utf8(bytes: Vec<u8>) -> Result<Self, EditError> {
        let text = String::from_utf8(bytes).map_err(|error| EditError::InvalidUtf8 {
            valid_up_to: error.utf8_error().valid_up_to(),
        })?;
        Ok(Self {
            rope: Rope::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text)),
            coordinate_index: Arc::default(),
        })
    }

    pub(super) fn len_bytes(&self) -> u64 {
        self.rope.len_bytes() as u64
    }

    pub(super) fn snapshot(&self, document_id: DocumentId, revision: Revision) -> DocumentSnapshot {
        DocumentSnapshot::from_rope(
            document_id,
            self.rope.clone(),
            revision,
            self.coordinate_index.clone(),
        )
    }
}

impl DocumentBuffer {
    pub fn from_utf8(bytes: Vec<u8>) -> Result<Self, EditError> {
        Self::with_edit_log_capacity(bytes, DEFAULT_EDIT_LOG_CAPACITY)
    }

    pub fn with_edit_log_capacity(bytes: Vec<u8>, capacity: usize) -> Result<Self, EditError> {
        let prepared = PreparedText::from_utf8(bytes)?;
        Ok(Self {
            id: DocumentId::next(),
            rope: prepared.rope,
            revision: Revision::INITIAL,
            edit_log: EditLog::new(capacity).map_err(EditError::EditLog)?,
            coordinate_index: Arc::default(),
        })
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn id(&self) -> DocumentId {
        self.id
    }

    pub fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot::from_rope(
            self.id,
            self.rope.clone(),
            self.revision,
            self.coordinate_index.clone(),
        )
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
        if transaction.edits.is_empty() {
            return Err(EditError::EmptyTransaction);
        }
        let after = self
            .revision
            .checked_next()
            .ok_or(EditError::RevisionExhausted)?;
        transaction.edits.sort_by_key(|edit| edit.range.start);
        self.validate_edits(&transaction.edits)?;
        let coordinate_index = self.coordinate_index.get().map(|checkpoints| {
            transform_coordinate_index(
                self.id,
                self.revision,
                &self.rope,
                checkpoints,
                &transaction.edits,
            )
        });

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
        self.coordinate_index = coordinate_index.map_or_else(Arc::default, |checkpoints| {
            let index = Arc::new(OnceLock::new());
            let _ = index.set(Arc::from(checkpoints));
            index
        });
        Ok(delta)
    }

    pub(super) fn replace_prepared(
        &mut self,
        prepared: PreparedText,
        delta: RevisionDelta,
    ) -> Result<(), EditError> {
        if delta.before != self.revision {
            return Err(EditError::StaleRevision {
                expected: self.revision,
                actual: delta.before,
            });
        }
        self.edit_log
            .push(delta.clone())
            .map_err(EditError::EditLog)?;
        self.rope = prepared.rope;
        self.revision = delta.after;
        self.coordinate_index = prepared.coordinate_index;
        Ok(())
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

fn transform_coordinate_index(
    document_id: DocumentId,
    revision: Revision,
    rope: &Rope,
    checkpoints: &[CoordinateCheckpoint],
    edits: &[TextEdit],
) -> Vec<CoordinateCheckpoint> {
    struct CoordinateEdit<'a> {
        edit: &'a TextEdit,
        start_character: usize,
        start_utf16: u64,
        byte_delta: i128,
        character_delta: i128,
        utf16_delta: i128,
    }

    let snapshot = DocumentSnapshot::from_rope(document_id, rope.clone(), revision, {
        let index = Arc::new(OnceLock::new());
        let _ = index.set(Arc::from(checkpoints));
        index
    });
    let coordinate_edits = edits
        .iter()
        .map(|edit| {
            let start_character = rope.byte_to_char(edit.range.start.0 as usize);
            let end_character = rope.byte_to_char(edit.range.end.0 as usize);
            let start_utf16 = snapshot
                .byte_to_utf16(edit.range.start)
                .expect("validated edit boundary")
                .0;
            let end_utf16 = snapshot
                .byte_to_utf16(edit.range.end)
                .expect("validated edit boundary")
                .0;
            CoordinateEdit {
                edit,
                start_character,
                start_utf16,
                byte_delta: edit.replacement.len() as i128 - edit.range.len() as i128,
                character_delta: edit.replacement.chars().count() as i128
                    - (end_character - start_character) as i128,
                utf16_delta: edit.replacement.encode_utf16().count() as i128
                    - (end_utf16 - start_utf16) as i128,
            }
        })
        .collect::<Vec<_>>();
    let mut transformed = Vec::with_capacity(checkpoints.len() + edits.len());
    for checkpoint in checkpoints.iter().copied() {
        let mut byte_shift = 0_i128;
        let mut character_shift = 0_i128;
        let mut utf16_shift = 0_i128;
        let mut retained = true;
        for edit in &coordinate_edits {
            if checkpoint.byte <= edit.edit.range.start.0 {
                break;
            }
            if checkpoint.byte < edit.edit.range.end.0 {
                retained = false;
                break;
            }
            byte_shift += edit.byte_delta;
            character_shift += edit.character_delta;
            utf16_shift += edit.utf16_delta;
        }
        if retained {
            transformed.push(CoordinateCheckpoint {
                byte: (checkpoint.byte as i128 + byte_shift) as u64,
                character: (checkpoint.character as i128 + character_shift) as usize,
                utf16: (checkpoint.utf16 as i128 + utf16_shift) as u64,
            });
        }
    }

    let mut byte_shift = 0_i128;
    let mut character_shift = 0_i128;
    let mut utf16_shift = 0_i128;
    for edit in &coordinate_edits {
        let new_start_byte = (edit.edit.range.start.0 as i128 + byte_shift) as u64;
        let new_start_character = (edit.start_character as i128 + character_shift) as usize;
        let new_start_utf16 = (edit.start_utf16 as i128 + utf16_shift) as u64;
        let mut local_byte = 0_u64;
        let mut local_character = 0_usize;
        let mut local_utf16 = 0_u64;
        let mut last_checkpoint = 0_u64;
        for scalar in edit.edit.replacement.chars() {
            if local_byte - last_checkpoint >= 64 * 1024 {
                transformed.push(CoordinateCheckpoint {
                    byte: new_start_byte + local_byte,
                    character: new_start_character + local_character,
                    utf16: new_start_utf16 + local_utf16,
                });
                last_checkpoint = local_byte;
            }
            local_byte += scalar.len_utf8() as u64;
            local_character += 1;
            local_utf16 += scalar.len_utf16() as u64;
        }
        if local_byte > 0 {
            transformed.push(CoordinateCheckpoint {
                byte: new_start_byte + local_byte,
                character: new_start_character + local_character,
                utf16: new_start_utf16 + local_utf16,
            });
        }
        byte_shift += edit.byte_delta;
        character_shift += edit.character_delta;
        utf16_shift += edit.utf16_delta;
    }
    transformed.sort_unstable_by_key(|checkpoint| checkpoint.byte);
    transformed.dedup_by_key(|checkpoint| checkpoint.byte);
    transformed
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

    fn text(snapshot: &DocumentSnapshot) -> String {
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
    fn rejects_empty_transactions_without_advancing_revision() {
        let mut document = DocumentBuffer::from_utf8(b"unchanged".to_vec()).unwrap();
        assert_eq!(
            document.commit(EditTransaction::new(Revision::INITIAL, Vec::new())),
            Err(EditError::EmptyTransaction)
        );
        assert_eq!(document.revision(), Revision::INITIAL);
        assert!(document.edit_log().latest_revision().is_none());
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
