use std::{ops::Range, sync::Arc};

use ropey::Rope;

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
    fn len_bytes(&self) -> u64;
    fn chunk_at(&self, offset: ByteOffset) -> Option<TextChunk<'_>>;
    fn copy_range(&self, range: ByteRange) -> String;
}

pub type SharedTextSnapshot = Arc<dyn TextSnapshot>;

pub struct RopeSnapshot {
    rope: Rope,
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
        })
    }
}

impl TextSnapshot for RopeSnapshot {
    fn len_bytes(&self) -> u64 {
        self.rope.len_bytes() as u64
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
