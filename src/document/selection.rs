use super::{ByteOffset, ByteRange, DocumentSnapshot, TextSnapshot};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Selection {
    anchor: ByteOffset,
    head: ByteOffset,
}

impl Selection {
    pub fn caret(offset: ByteOffset) -> Self {
        Self {
            anchor: offset,
            head: offset,
        }
    }

    pub fn new(anchor: ByteOffset, head: ByteOffset) -> Self {
        Self { anchor, head }
    }

    pub fn anchor(self) -> ByteOffset {
        self.anchor
    }

    pub fn head(self) -> ByteOffset {
        self.head
    }

    pub fn is_reversed(self) -> bool {
        self.head < self.anchor
    }

    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }

    pub fn range(self) -> ByteRange {
        if self.is_reversed() {
            ByteRange {
                start: self.head,
                end: self.anchor,
            }
        } else {
            ByteRange {
                start: self.anchor,
                end: self.head,
            }
        }
    }

    pub fn with_head(self, head: ByteOffset) -> Self {
        Self { head, ..self }
    }

    pub fn clamp(self, snapshot: &DocumentSnapshot) -> Self {
        let len = snapshot.len_bytes();
        let clamp = |offset: ByteOffset| {
            let mut value = offset.0.min(len);
            while value > 0 && !snapshot.is_char_boundary(ByteOffset(value)) {
                value -= 1;
            }
            ByteOffset(value)
        };
        Self::new(clamp(self.anchor), clamp(self.head))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_direction_and_normalizes_range() {
        let selection = Selection::new(ByteOffset(9), ByteOffset(3));
        assert!(selection.is_reversed());
        assert_eq!(selection.range(), ByteRange::new(3, 9));
        assert_eq!(selection.head(), ByteOffset(3));
    }
}
