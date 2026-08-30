use std::{ops::Range, sync::Arc};

use ropey::Rope;

mod revision;
mod transaction;

pub use revision::{
    EditLog, EditLogError, RangeMapError, Revision, RevisionDelta, RevisionRange, TextEditSummary,
};
pub use transaction::{DocumentBuffer, EditError, EditTransaction, TextEdit};

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct ByteOffset(pub u64);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ByteRange {
    pub start: ByteOffset,
    pub end: ByteOffset,
}

impl ByteRange {
    pub fn new(start: u64, end: u64) -> Self {
        Self {
            start: ByteOffset(start),
            end: ByteOffset(end),
        }
    }

    pub fn as_usize(self) -> Range<usize> {
        self.start.0 as usize..self.end.0 as usize
    }
}

pub struct TextChunk<'a> {
    pub start: ByteOffset,
    pub text: &'a str,
}

pub trait TextSnapshot: Send + Sync {
    fn revision(&self) -> Revision {
        Revision::INITIAL
    }

    fn revision_range(&self, range: ByteRange) -> RevisionRange {
        RevisionRange::new(self.revision(), range)
    }

    fn len_bytes(&self) -> u64;
    fn len_chars(&self) -> u64;
    fn len_lines(&self) -> u64;
    fn line_of_byte(&self, offset: ByteOffset) -> u64;
    fn chunk_at(&self, offset: ByteOffset) -> Option<TextChunk<'_>>;
    fn copy_range(&self, range: ByteRange) -> String;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextStatistics {
    pub bytes: u64,
    pub characters: u64,
    pub lines: u64,
}

impl TextStatistics {
    pub fn from_snapshot(snapshot: &dyn TextSnapshot) -> Self {
        Self {
            bytes: snapshot.len_bytes(),
            characters: snapshot.len_chars(),
            lines: snapshot.len_lines(),
        }
    }
}

pub type SharedTextSnapshot = Arc<dyn TextSnapshot>;

#[derive(Clone)]
pub struct RopeSnapshot {
    rope: Rope,
    revision: Revision,
}

impl RopeSnapshot {
    pub fn from_utf8(mut bytes: Vec<u8>) -> Result<Self, TextLoadError> {
        if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            bytes.drain(..3);
        }

        let text = String::from_utf8(bytes).map_err(|error| TextLoadError::InvalidUtf8 {
            valid_up_to: error.utf8_error().valid_up_to(),
        })?;

        Ok(Self {
            rope: Rope::from_str(&text),
            revision: Revision::INITIAL,
        })
    }

    pub fn with_revision(mut self, revision: Revision) -> Self {
        self.revision = revision;
        self
    }

    pub(super) fn from_rope(rope: Rope, revision: Revision) -> Self {
        Self { rope, revision }
    }
}

impl TextSnapshot for RopeSnapshot {
    fn revision(&self) -> Revision {
        self.revision
    }

    fn len_bytes(&self) -> u64 {
        self.rope.len_bytes() as u64
    }

    fn len_chars(&self) -> u64 {
        self.rope.len_chars() as u64
    }

    fn len_lines(&self) -> u64 {
        self.rope.len_lines() as u64
    }

    fn line_of_byte(&self, offset: ByteOffset) -> u64 {
        self.rope.byte_to_line(offset.0 as usize) as u64
    }

    fn chunk_at(&self, offset: ByteOffset) -> Option<TextChunk<'_>> {
        let offset = usize::try_from(offset.0).ok()?;
        if offset >= self.rope.len_bytes() {
            return None;
        }

        let (text, chunk_start, _, _) = self.rope.chunk_at_byte(offset);
        Some(TextChunk {
            start: ByteOffset(chunk_start as u64),
            text,
        })
    }

    fn copy_range(&self, range: ByteRange) -> String {
        let range = range.as_usize();
        let start = self.rope.byte_to_char(range.start);
        let end = self.rope.byte_to_char(range.end);
        self.rope.slice(start..end).to_string()
    }
}

#[derive(Debug)]
pub enum TextLoadError {
    InvalidUtf8 { valid_up_to: usize },
}

impl std::fmt::Display for TextLoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUtf8 { valid_up_to } => {
                write!(formatter, "file is not valid UTF-8 near byte {valid_up_to}")
            }
        }
    }
}

impl std::error::Error for TextLoadError {}

pub struct TextLine<'a> {
    pub range: ByteRange,
    pub text: std::borrow::Cow<'a, str>,
}

pub struct LineCursor<'a> {
    snapshot: &'a dyn TextSnapshot,
    offset: u64,
}

impl<'a> LineCursor<'a> {
    pub fn new(snapshot: &'a dyn TextSnapshot) -> Self {
        Self {
            snapshot,
            offset: 0,
        }
    }

    pub fn next_line(&mut self) -> Option<TextLine<'a>> {
        if self.offset >= self.snapshot.len_bytes() {
            return None;
        }

        let line_start = self.offset;
        let mut scratch = String::new();

        loop {
            let chunk = self.snapshot.chunk_at(ByteOffset(self.offset))?;
            let local_start = (self.offset - chunk.start.0) as usize;
            let remaining = &chunk.text[local_start..];

            if let Some(newline) = remaining.as_bytes().iter().position(|byte| *byte == b'\n') {
                let end = self.offset + newline as u64 + 1;
                let text_end = newline + 1;

                if scratch.is_empty() {
                    let text = &remaining[..text_end];
                    self.offset = end;
                    return Some(TextLine {
                        range: ByteRange::new(line_start, end),
                        text: std::borrow::Cow::Borrowed(text),
                    });
                }

                scratch.push_str(&remaining[..text_end]);
                self.offset = end;
                return Some(TextLine {
                    range: ByteRange::new(line_start, end),
                    text: std::borrow::Cow::Owned(scratch),
                });
            }

            scratch.push_str(remaining);
            self.offset += remaining.len() as u64;

            if self.offset >= self.snapshot.len_bytes() {
                return Some(TextLine {
                    range: ByteRange::new(line_start, self.offset),
                    text: std::borrow::Cow::Owned(scratch),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ByteRange, DocumentBuffer, EditTransaction, RopeSnapshot, TextEdit, TextSnapshot,
        TextStatistics,
    };

    #[test]
    fn snapshot_reports_unicode_characters_and_physical_lines() {
        let snapshot = RopeSnapshot::from_utf8("一a\n二🙂".as_bytes().to_vec()).unwrap();
        assert_eq!(snapshot.len_bytes(), 12);
        assert_eq!(snapshot.len_chars(), 5);
        assert_eq!(snapshot.len_lines(), 2);
        assert_eq!(
            TextStatistics::from_snapshot(&snapshot),
            TextStatistics {
                bytes: 12,
                characters: 5,
                lines: 2,
            }
        );
    }

    #[test]
    fn statistics_follow_the_latest_edit_snapshot() {
        let mut buffer = DocumentBuffer::from_utf8(b"one\ntwo".to_vec()).unwrap();
        let initial = buffer.snapshot();
        assert_eq!(TextStatistics::from_snapshot(&initial).characters, 7);

        buffer
            .commit(EditTransaction::new(
                buffer.revision(),
                vec![TextEdit::new(ByteRange::new(7, 7), "\nthree")],
            ))
            .unwrap();
        let edited = buffer.snapshot();
        assert_eq!(
            TextStatistics::from_snapshot(&edited),
            TextStatistics {
                bytes: 13,
                characters: 13,
                lines: 3,
            }
        );
    }
}
