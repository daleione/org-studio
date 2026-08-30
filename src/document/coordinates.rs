use std::sync::Arc;

use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

use super::{ByteOffset, ByteRange, CoordinateCheckpoint, DocumentSnapshot, TextSnapshot};

const COORDINATE_CHECKPOINT_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Utf16Offset(pub u64);

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LineIndex(pub u64);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Bias {
    Left,
    #[default]
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoordinateError {
    ByteOutOfBounds(ByteOffset),
    Utf16OutOfBounds(Utf16Offset),
    InvalidUtf8Boundary(ByteOffset),
    InvalidUtf16Boundary(Utf16Offset),
    LineOutOfBounds(LineIndex),
}

impl std::fmt::Display for CoordinateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "invalid document coordinate: {self:?}")
    }
}

impl std::error::Error for CoordinateError {}

impl DocumentSnapshot {
    pub fn is_char_boundary(&self, offset: ByteOffset) -> bool {
        let Ok(offset) = usize::try_from(offset.0) else {
            return false;
        };
        if offset == self.rope.len_bytes() {
            return true;
        }
        if offset > self.rope.len_bytes() {
            return false;
        }
        let (chunk, chunk_start, _, _) = self.rope.chunk_at_byte(offset);
        chunk.is_char_boundary(offset - chunk_start)
    }

    pub fn checked_byte_offset(&self, offset: ByteOffset) -> Result<ByteOffset, CoordinateError> {
        if offset.0 > self.len_bytes() {
            return Err(CoordinateError::ByteOutOfBounds(offset));
        }
        if !self.is_char_boundary(offset) {
            return Err(CoordinateError::InvalidUtf8Boundary(offset));
        }
        Ok(offset)
    }

    pub fn byte_to_utf16(&self, offset: ByteOffset) -> Result<Utf16Offset, CoordinateError> {
        self.checked_byte_offset(offset)?;
        let checkpoints = self.coordinate_checkpoints();
        let checkpoint =
            checkpoints[checkpoints.partition_point(|checkpoint| checkpoint.byte <= offset.0) - 1];
        let char_offset = self.rope.byte_to_char(offset.0 as usize);
        let suffix = self
            .rope
            .slice(checkpoint.character..char_offset)
            .chunks()
            .map(|chunk| chunk.encode_utf16().count() as u64)
            .sum::<u64>();
        Ok(Utf16Offset(checkpoint.utf16 + suffix))
    }

    pub fn utf16_to_byte(&self, offset: Utf16Offset) -> Result<ByteOffset, CoordinateError> {
        let checkpoints = self.coordinate_checkpoints();
        let checkpoint =
            checkpoints[checkpoints.partition_point(|checkpoint| checkpoint.utf16 <= offset.0) - 1];
        let mut utf16 = checkpoint.utf16;
        let mut bytes = checkpoint.byte;
        for chunk in self.rope.slice(checkpoint.character..).chunks() {
            for character in chunk.chars() {
                if utf16 == offset.0 {
                    return Ok(ByteOffset(bytes));
                }
                let width = character.len_utf16() as u64;
                if utf16 + width > offset.0 {
                    return Err(CoordinateError::InvalidUtf16Boundary(offset));
                }
                utf16 += width;
                bytes += character.len_utf8() as u64;
            }
        }
        if utf16 == offset.0 {
            Ok(ByteOffset(bytes))
        } else {
            Err(CoordinateError::Utf16OutOfBounds(offset))
        }
    }

    fn coordinate_checkpoints(&self) -> &Arc<[CoordinateCheckpoint]> {
        self.coordinate_index.get_or_init(|| {
            let mut checkpoints = vec![CoordinateCheckpoint {
                byte: 0,
                character: 0,
                utf16: 0,
            }];
            let mut byte = 0_u64;
            let mut character = 0_usize;
            let mut utf16 = 0_u64;
            let mut checkpoint_byte = 0_u64;
            for chunk in self.rope.chunks() {
                for scalar in chunk.chars() {
                    if byte - checkpoint_byte >= COORDINATE_CHECKPOINT_BYTES {
                        checkpoints.push(CoordinateCheckpoint {
                            byte,
                            character,
                            utf16,
                        });
                        checkpoint_byte = byte;
                    }
                    byte += scalar.len_utf8() as u64;
                    character += 1;
                    utf16 += scalar.len_utf16() as u64;
                }
            }
            Arc::from(checkpoints)
        })
    }

    pub fn line_index_at(&self, offset: ByteOffset) -> Result<LineIndex, CoordinateError> {
        self.checked_byte_offset(offset)?;
        Ok(LineIndex(self.rope.byte_to_line(offset.0 as usize) as u64))
    }

    /// Returns the zero-based physical line and Unicode-scalar column for a byte offset.
    ///
    /// Rope metrics keep this query logarithmic in the document size and avoid copying the line,
    /// which is important for status updates on very large files and unusually long lines.
    pub fn line_and_column_at(
        &self,
        offset: ByteOffset,
    ) -> Result<(LineIndex, u64), CoordinateError> {
        self.checked_byte_offset(offset)?;
        let byte = offset.0 as usize;
        let line = self.rope.byte_to_line(byte);
        let line_start = self.rope.line_to_char(line);
        let column = self.rope.byte_to_char(byte) - line_start;
        Ok((LineIndex(line as u64), column as u64))
    }

    pub fn byte_at_line_column(
        &self,
        line: LineIndex,
        column: u64,
    ) -> Result<ByteOffset, CoordinateError> {
        let line_index =
            usize::try_from(line.0).map_err(|_| CoordinateError::LineOutOfBounds(line))?;
        if line_index >= self.rope.len_lines() {
            return Err(CoordinateError::LineOutOfBounds(line));
        }
        let start = self.rope.line_to_char(line_index);
        let content = self.line_content_range(line)?;
        let content_end = self.rope.byte_to_char(content.end.0 as usize);
        let target = start.saturating_add(column as usize).min(content_end);
        Ok(ByteOffset(self.rope.char_to_byte(target) as u64))
    }

    pub fn line_range(&self, line: LineIndex) -> Result<ByteRange, CoordinateError> {
        let line = usize::try_from(line.0).map_err(|_| CoordinateError::LineOutOfBounds(line))?;
        if line >= self.rope.len_lines() {
            return Err(CoordinateError::LineOutOfBounds(LineIndex(line as u64)));
        }
        let start = self.rope.line_to_byte(line) as u64;
        let end = if line + 1 < self.rope.len_lines() {
            self.rope.line_to_byte(line + 1) as u64
        } else {
            self.rope.len_bytes() as u64
        };
        Ok(ByteRange::new(start, end))
    }

    pub fn previous_grapheme_boundary(
        &self,
        offset: ByteOffset,
    ) -> Result<ByteOffset, CoordinateError> {
        self.checked_byte_offset(offset)?;
        if offset.0 == 0 {
            return Ok(offset);
        }
        self.grapheme_boundary(offset, false)
    }

    pub fn next_grapheme_boundary(
        &self,
        offset: ByteOffset,
    ) -> Result<ByteOffset, CoordinateError> {
        self.checked_byte_offset(offset)?;
        if offset.0 == self.len_bytes() {
            return Ok(offset);
        }
        self.grapheme_boundary(offset, true)
    }

    pub fn line_content_range(&self, line: LineIndex) -> Result<ByteRange, CoordinateError> {
        let mut range = self.line_range(line)?;
        if range.end > range.start && self.rope.byte(range.end.0 as usize - 1) == b'\n' {
            range.end.0 -= 1;
            if range.end > range.start && self.rope.byte(range.end.0 as usize - 1) == b'\r' {
                range.end.0 -= 1;
            }
        }
        Ok(range)
    }

    fn grapheme_boundary(
        &self,
        offset: ByteOffset,
        forward: bool,
    ) -> Result<ByteOffset, CoordinateError> {
        let len = self.rope.len_bytes();
        let mut cursor = GraphemeCursor::new(offset.0 as usize, len, true);
        let mut lookup = if forward {
            offset.0 as usize
        } else {
            offset.0.saturating_sub(1) as usize
        };
        loop {
            let (chunk, chunk_start, _, _) = self.rope.chunk_at_byte(lookup);
            let result = if forward {
                cursor.next_boundary(chunk, chunk_start)
            } else {
                cursor.prev_boundary(chunk, chunk_start)
            };
            match result {
                Ok(Some(boundary)) => return Ok(ByteOffset(boundary as u64)),
                Ok(None) => return Ok(ByteOffset(if forward { len as u64 } else { 0 })),
                Err(GraphemeIncomplete::PreContext(context_end)) => {
                    let context_lookup = context_end.saturating_sub(1);
                    let (context, context_start, _, _) = self.rope.chunk_at_byte(context_lookup);
                    cursor.provide_context(context, context_start);
                }
                Err(GraphemeIncomplete::PrevChunk) => {
                    lookup = cursor.cur_cursor().saturating_sub(1);
                }
                Err(GraphemeIncomplete::NextChunk) => {
                    lookup = cursor.cur_cursor();
                }
                Err(GraphemeIncomplete::InvalidOffset) => {
                    return Err(CoordinateError::InvalidUtf8Boundary(offset));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentSnapshot;

    #[test]
    fn line_and_column_uses_character_columns_without_copying_text() {
        let snapshot = DocumentSnapshot::from_utf8("ab\n你😀z".as_bytes().to_vec()).unwrap();
        assert_eq!(
            snapshot.line_and_column_at(ByteOffset(3)).unwrap(),
            (LineIndex(1), 0)
        );
        assert_eq!(
            snapshot.line_and_column_at(ByteOffset(10)).unwrap(),
            (LineIndex(1), 2)
        );
    }

    #[test]
    fn checked_utf16_round_trip_rejects_surrogate_interior() {
        let snapshot = DocumentSnapshot::from_utf8("a🙂e\u{301}\n中".as_bytes().to_vec()).unwrap();
        for (byte, utf16) in [(0, 0), (1, 1), (5, 3), (8, 5), (9, 6), (12, 7)] {
            assert_eq!(
                snapshot.byte_to_utf16(ByteOffset(byte)).unwrap(),
                Utf16Offset(utf16)
            );
            assert_eq!(
                snapshot.utf16_to_byte(Utf16Offset(utf16)).unwrap(),
                ByteOffset(byte)
            );
        }
        assert_eq!(
            snapshot.utf16_to_byte(Utf16Offset(2)),
            Err(CoordinateError::InvalidUtf16Boundary(Utf16Offset(2)))
        );
    }

    #[test]
    fn grapheme_and_line_queries_preserve_combining_clusters() {
        let snapshot =
            DocumentSnapshot::from_utf8("a\r\ne\u{301}🙂\n".as_bytes().to_vec()).unwrap();
        assert_eq!(
            snapshot.line_content_range(LineIndex(1)).unwrap(),
            ByteRange::new(3, 10)
        );
        assert_eq!(
            snapshot.next_grapheme_boundary(ByteOffset(3)).unwrap(),
            ByteOffset(6)
        );
        assert_eq!(
            snapshot.previous_grapheme_boundary(ByteOffset(10)).unwrap(),
            ByteOffset(6)
        );
        assert_eq!(
            snapshot.previous_grapheme_boundary(ByteOffset(3)).unwrap(),
            ByteOffset(1)
        );
    }

    #[test]
    fn grapheme_queries_cross_rope_chunks_without_copying_a_long_line() {
        let mut text = "a".repeat(200_000);
        text.push_str("e\u{301}🙂");
        let snapshot = DocumentSnapshot::from_utf8(text.into_bytes()).unwrap();
        let end = ByteOffset(snapshot.len_bytes());
        let emoji = snapshot.previous_grapheme_boundary(end).unwrap();
        let combined = snapshot.previous_grapheme_boundary(emoji).unwrap();
        assert_eq!(snapshot.copy_range(ByteRange::new(emoji.0, end.0)), "🙂");
        assert_eq!(
            snapshot.copy_range(ByteRange::new(combined.0, emoji.0)),
            "e\u{301}"
        );
    }

    #[test]
    fn utf16_index_is_preserved_incrementally_across_edits() {
        use crate::document::{DocumentBuffer, EditTransaction, TextEdit};

        let text = "a🙂".repeat(100_000);
        let mut buffer = DocumentBuffer::from_utf8(text.into_bytes()).unwrap();
        let before = buffer.snapshot();
        let before_end = before
            .byte_to_utf16(ByteOffset(before.len_bytes()))
            .unwrap();
        buffer
            .commit(EditTransaction::new(
                buffer.revision(),
                vec![TextEdit::new(ByteRange::new(1, 1), "中🙂")],
            ))
            .unwrap();
        let after = buffer.snapshot();
        assert!(after.coordinate_index.get().is_some());
        assert_eq!(
            after.byte_to_utf16(ByteOffset(after.len_bytes())).unwrap(),
            Utf16Offset(before_end.0 + 3)
        );
        assert_eq!(
            after.utf16_to_byte(Utf16Offset(before_end.0 + 3)).unwrap(),
            ByteOffset(after.len_bytes())
        );
    }
}
