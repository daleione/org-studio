use std::path::{Path, PathBuf};

use gpui::{Context, EventEmitter};

use super::{
    ByteRange, DocumentBuffer, DocumentId, DocumentSnapshot, EditError, EditTransaction, Revision,
    RevisionDelta, TextEditSummary, TextSnapshot, transaction::PreparedText,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DocumentEvent {
    Edited {
        document_id: DocumentId,
        delta: RevisionDelta,
    },
    Reloaded {
        document_id: DocumentId,
        delta: RevisionDelta,
    },
    Saved {
        document_id: DocumentId,
        revision: Revision,
    },
    DiskChanged {
        document_id: DocumentId,
        path: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SavePoint {
    document_id: DocumentId,
    revision: Revision,
}

impl SavePoint {
    pub fn document_id(self) -> DocumentId {
        self.document_id
    }

    pub fn revision(self) -> Revision {
        self.revision
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveAckError {
    DifferentDocument,
    FutureRevision { current: Revision, saved: Revision },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReloadError {
    Dirty,
    DifferentDocument,
    DifferentPath,
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    RevisionExhausted,
    Edit(EditError),
}

impl From<EditError> for ReloadError {
    fn from(error: EditError) -> Self {
        Self::Edit(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReloadRequest {
    document_id: DocumentId,
    path: PathBuf,
    base_revision: Revision,
    old_len: u64,
}

impl ReloadRequest {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn prepare(self, bytes: Vec<u8>) -> Result<PreparedReload, ReloadError> {
        let after = self
            .base_revision
            .checked_next()
            .ok_or(ReloadError::RevisionExhausted)?;
        let text = PreparedText::from_utf8(bytes)?;
        let delta = RevisionDelta::new(
            self.base_revision,
            after,
            vec![TextEditSummary::new(
                ByteRange::new(0, self.old_len),
                text.len_bytes(),
            )],
        )
        .map_err(EditError::EditLog)?;
        let snapshot = text.snapshot(self.document_id, after);
        Ok(PreparedReload {
            request: self,
            text,
            delta,
            snapshot,
        })
    }
}

pub struct PreparedReload {
    request: ReloadRequest,
    text: PreparedText,
    delta: RevisionDelta,
    snapshot: DocumentSnapshot,
}

impl PreparedReload {
    pub fn path(&self) -> &Path {
        &self.request.path
    }

    pub fn snapshot(&self) -> &DocumentSnapshot {
        &self.snapshot
    }
}

/// Owns the single mutable text buffer and file identity for an open document.
///
/// Views and workers receive immutable snapshots. Reload and save results carry an explicit
/// document identity and revision so delayed background work cannot mutate a newer document.
pub struct DocumentSession {
    path: PathBuf,
    buffer: DocumentBuffer,
    saved_revision: Revision,
}

impl EventEmitter<DocumentEvent> for DocumentSession {}

impl DocumentSession {
    pub fn from_utf8(path: PathBuf, bytes: Vec<u8>) -> Result<Self, EditError> {
        let buffer = DocumentBuffer::from_utf8(bytes)?;
        Ok(Self {
            path,
            saved_revision: buffer.revision(),
            buffer,
        })
    }

    pub fn id(&self) -> DocumentId {
        self.buffer.id()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn revision(&self) -> Revision {
        self.buffer.revision()
    }

    pub fn saved_revision(&self) -> Revision {
        self.saved_revision
    }

    pub fn is_dirty(&self) -> bool {
        self.revision() != self.saved_revision
    }

    pub fn snapshot(&self) -> DocumentSnapshot {
        self.buffer.snapshot()
    }

    pub fn save_point(&self) -> SavePoint {
        SavePoint {
            document_id: self.id(),
            revision: self.revision(),
        }
    }

    pub fn commit(
        &mut self,
        transaction: EditTransaction,
        cx: &mut Context<Self>,
    ) -> Result<RevisionDelta, EditError> {
        let delta = self.buffer.commit(transaction)?;
        cx.emit(DocumentEvent::Edited {
            document_id: self.id(),
            delta: delta.clone(),
        });
        Ok(delta)
    }

    pub fn mark_saved(
        &mut self,
        saved: SavePoint,
        cx: &mut Context<Self>,
    ) -> Result<bool, SaveAckError> {
        if saved.document_id != self.id() {
            return Err(SaveAckError::DifferentDocument);
        }
        if saved.revision > self.revision() {
            return Err(SaveAckError::FutureRevision {
                current: self.revision(),
                saved: saved.revision,
            });
        }
        if saved.revision <= self.saved_revision {
            return Ok(false);
        }
        self.saved_revision = saved.revision;
        cx.emit(DocumentEvent::Saved {
            document_id: self.id(),
            revision: saved.revision,
        });
        Ok(true)
    }

    pub fn disk_changed(&self, cx: &mut Context<Self>) {
        cx.emit(DocumentEvent::DiskChanged {
            document_id: self.id(),
            path: self.path.clone(),
        });
    }

    pub fn reload_request(&self) -> Result<ReloadRequest, ReloadError> {
        if self.is_dirty() {
            return Err(ReloadError::Dirty);
        }
        Ok(ReloadRequest {
            document_id: self.id(),
            path: self.path.clone(),
            base_revision: self.revision(),
            old_len: self.snapshot().len_bytes(),
        })
    }

    pub fn apply_reload(
        &mut self,
        prepared: PreparedReload,
        cx: &mut Context<Self>,
    ) -> Result<RevisionDelta, ReloadError> {
        if prepared.request.document_id != self.id() {
            return Err(ReloadError::DifferentDocument);
        }
        if prepared.request.path != self.path {
            return Err(ReloadError::DifferentPath);
        }
        if self.is_dirty() {
            return Err(ReloadError::Dirty);
        }
        if prepared.request.base_revision != self.revision() {
            return Err(ReloadError::StaleRevision {
                expected: self.revision(),
                actual: prepared.request.base_revision,
            });
        }
        let delta = prepared.delta;
        self.buffer.replace_prepared(prepared.text, delta.clone())?;
        self.saved_revision = delta.after;
        cx.emit(DocumentEvent::Reloaded {
            document_id: self.id(),
            delta: delta.clone(),
        });
        Ok(delta)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::document::{ByteRange, TextEdit, TextSnapshot};
    use gpui::AppContext;

    fn contents(session: &DocumentSession) -> String {
        let snapshot = session.snapshot();
        snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
    }

    #[gpui::test]
    fn session_emits_events_and_keeps_old_snapshots_readable(cx: &mut gpui::TestAppContext) {
        let path = PathBuf::from("notes.org");
        let session =
            cx.new(|_| DocumentSession::from_utf8(path.clone(), b"one\ntwo".to_vec()).unwrap());
        let events = Arc::new(Mutex::new(Vec::new()));
        session.update(cx, |_, cx| {
            let events = events.clone();
            cx.subscribe_self(move |_, event, _| events.lock().unwrap().push(event.clone()))
                .detach();
        });
        let before = cx.read(|cx| session.read(cx).snapshot());
        session.update(cx, |session, cx| {
            session
                .commit(
                    EditTransaction::new(
                        session.revision(),
                        vec![TextEdit::new(ByteRange::new(7, 7), "\nthree")],
                    ),
                    cx,
                )
                .unwrap();
        });
        assert_eq!(cx.read(|cx| contents(session.read(cx))), "one\ntwo\nthree");
        assert_eq!(
            before.copy_range(ByteRange::new(0, before.len_bytes())),
            "one\ntwo"
        );
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [DocumentEvent::Edited { .. }]
        ));
    }

    #[gpui::test]
    fn saving_an_older_snapshot_does_not_clear_newer_edits(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("notes.org"), b"one".to_vec()).unwrap()
        });
        let saved = session.update(cx, |session, cx| {
            session
                .commit(
                    EditTransaction::new(
                        session.revision(),
                        vec![TextEdit::new(ByteRange::new(3, 3), " two")],
                    ),
                    cx,
                )
                .unwrap();
            session.save_point()
        });
        session.update(cx, |session, cx| {
            session
                .commit(
                    EditTransaction::new(
                        session.revision(),
                        vec![TextEdit::new(ByteRange::new(7, 7), " three")],
                    ),
                    cx,
                )
                .unwrap();
            assert!(session.mark_saved(saved, cx).unwrap());
            assert_eq!(session.saved_revision(), Revision(1));
            assert_eq!(session.revision(), Revision(2));
            assert!(session.is_dirty());
        });
    }

    #[gpui::test]
    fn clean_reload_preserves_identity_and_rejects_stale_application(
        cx: &mut gpui::TestAppContext,
    ) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("notes.org"), b"old".to_vec()).unwrap()
        });
        let original_id = cx.read(|cx| session.read(cx).id());
        let request = cx.read(|cx| session.read(cx).reload_request().unwrap());
        let prepared = request.prepare(b"new text".to_vec()).unwrap();
        assert_eq!(prepared.snapshot().document_id(), original_id);
        session.update(cx, |session, cx| {
            session.apply_reload(prepared, cx).unwrap();
            assert_eq!(session.id(), original_id);
            assert_eq!(session.revision(), Revision(1));
            assert_eq!(session.saved_revision(), Revision(1));
            assert!(!session.is_dirty());
        });
        assert_eq!(cx.read(|cx| contents(session.read(cx))), "new text");

        let stale = cx
            .read(|cx| session.read(cx).reload_request().unwrap())
            .prepare(b"disk".to_vec())
            .unwrap();
        session.update(cx, |session, cx| {
            session
                .commit(
                    EditTransaction::new(
                        session.revision(),
                        vec![TextEdit::new(ByteRange::new(8, 8), " local")],
                    ),
                    cx,
                )
                .unwrap();
            assert_eq!(session.apply_reload(stale, cx), Err(ReloadError::Dirty));
        });
        assert_eq!(cx.read(|cx| contents(session.read(cx))), "new text local");
    }
}
