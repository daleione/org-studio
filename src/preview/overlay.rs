use std::{ops::Range, sync::Arc};

use crate::document::Revision;

use super::coordinates::SourcePoint;

/// Ephemeral editor state. It deliberately lives outside visual/layout snapshots,
/// so caret blinking and IME updates cannot invalidate minimap tiles.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[allow(dead_code)] // Consumed by the editor host planned after the read-only preview.
pub(in crate::preview) struct EditorOverlaySnapshot {
    pub(in crate::preview) revision: Revision,
    pub(in crate::preview) selections: Arc<[Range<u64>]>,
    pub(in crate::preview) caret: Option<SourcePoint>,
    pub(in crate::preview) ime: Option<Range<u64>>,
    pub(in crate::preview) paint_revision: u64,
}

#[allow(dead_code)]
impl EditorOverlaySnapshot {
    pub(in crate::preview) fn updating_caret(&self, caret: Option<SourcePoint>) -> Self {
        let mut next = self.clone();
        next.caret = caret;
        next.paint_revision = next.paint_revision.wrapping_add(1);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::ByteOffset;

    #[test]
    fn caret_updates_only_the_overlay_revision() {
        let overlay = EditorOverlaySnapshot::default();
        let updated = overlay.updating_caret(Some(SourcePoint {
            revision: Revision::INITIAL,
            offset: ByteOffset(12),
        }));
        assert_eq!(overlay.revision, updated.revision);
        assert_eq!(updated.paint_revision, 1);
        assert_eq!(updated.caret.unwrap().offset, ByteOffset(12));
    }
}
