use std::path::{Path, PathBuf};

use gpui::{Context, EventEmitter};

use super::{
    ByteOffset, ByteRange, DocumentBuffer, DocumentId, DocumentSnapshot, EditError, EditOrigin,
    EditTransaction, FileMetadata, FileStamp, HistoryOutcome, Revision, RevisionDelta, SaveOutcome,
    SaveRequest, SaveState, Selection, SyncState, TargetExpectation, TextEdit, TextEditSummary,
    TextSnapshot,
    transaction::PreparedText,
    undo::{HistoryStep, UndoHistory},
};

#[derive(Clone, Debug)]
pub struct SessionEdit {
    transaction: EditTransaction,
    before: Selection,
    after: Selection,
    origin: EditOrigin,
}

impl SessionEdit {
    pub fn new(
        transaction: EditTransaction,
        before: Selection,
        after: Selection,
        origin: EditOrigin,
    ) -> Self {
        Self {
            transaction,
            before,
            after,
            origin,
        }
    }
}

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
    PathChanged {
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
    NotSaving,
    DifferentTarget,
    FutureRevision { current: Revision, saved: Revision },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveStartError {
    AlreadySaving,
    Conflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiskChangeAction {
    Ignore,
    Recovered,
    Defer,
    Reload,
    Conflict,
    Missing,
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
        let stamp = FileStamp::from_loaded(&self.path, &bytes)
            .unwrap_or_else(|_| FileStamp::detached(&bytes));
        let metadata = FileMetadata::from_loaded(&self.path, &bytes);
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
            stamp,
            metadata,
        })
    }
}

pub struct PreparedReload {
    request: ReloadRequest,
    text: PreparedText,
    delta: RevisionDelta,
    snapshot: DocumentSnapshot,
    stamp: FileStamp,
    metadata: FileMetadata,
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
    history: UndoHistory,
    file: Box<SessionFileState>,
}

struct SessionFileState {
    metadata: FileMetadata,
    sync_state: SyncState,
    save_state: SaveState,
}

impl EventEmitter<DocumentEvent> for DocumentSession {}

impl DocumentSession {
    pub fn from_utf8(path: PathBuf, bytes: Vec<u8>) -> Result<Self, EditError> {
        let stamp =
            FileStamp::from_loaded(&path, &bytes).unwrap_or_else(|_| FileStamp::detached(&bytes));
        let metadata = FileMetadata::from_loaded(&path, &bytes);
        let buffer = DocumentBuffer::from_utf8(bytes)?;
        Ok(Self {
            path,
            saved_revision: buffer.revision(),
            buffer,
            history: UndoHistory::default(),
            file: Box::new(SessionFileState {
                metadata,
                sync_state: SyncState::InSync { stamp },
                save_state: SaveState::Idle,
            }),
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

    pub fn sync_state(&self) -> &SyncState {
        &self.file.sync_state
    }

    pub fn save_state(&self) -> &SaveState {
        &self.file.save_state
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

    pub(crate) fn apply_transient_edit(
        &mut self,
        transaction: EditTransaction,
        cx: &mut Context<Self>,
    ) -> Result<RevisionDelta, EditError> {
        let delta = self.buffer.commit(transaction)?;
        self.note_edit();
        self.history.clear_redo();
        cx.emit(DocumentEvent::Edited {
            document_id: self.id(),
            delta: delta.clone(),
        });
        Ok(delta)
    }

    pub fn edit(
        &mut self,
        edit: SessionEdit,
        cx: &mut Context<Self>,
    ) -> Result<RevisionDelta, EditError> {
        let snapshot = self.buffer.snapshot();
        let mut forward = edit.transaction.edits.clone();
        forward.sort_by_key(|text_edit| text_edit.range.start);
        let delta = self.buffer.commit(edit.transaction)?;
        self.note_edit();
        let inverse = inverse_edits(&snapshot, &forward);
        self.history.record(
            HistoryStep { forward, inverse },
            edit.before,
            edit.after,
            edit.origin,
        );
        self.emit_edited(delta.clone(), cx);
        Ok(delta)
    }

    pub(crate) fn finalize_transient_edit(
        &mut self,
        expected_revision: Revision,
        forward: Vec<TextEdit>,
        inverse: Vec<TextEdit>,
        before: Selection,
        after: Selection,
        origin: EditOrigin,
    ) -> Result<(), EditError> {
        if self.revision() != expected_revision {
            return Err(EditError::StaleRevision {
                expected: self.revision(),
                actual: expected_revision,
            });
        }
        self.history
            .record(HistoryStep { forward, inverse }, before, after, origin);
        Ok(())
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) -> Result<HistoryOutcome, EditError> {
        let Some((transactions, selection)) = self.history.prepare_undo(self.revision())? else {
            return Ok(HistoryOutcome::Empty);
        };
        for transaction in transactions {
            let delta = self.buffer.commit(transaction)?;
            self.note_edit();
            self.emit_edited(delta, cx);
        }
        self.history.complete_undo();
        Ok(HistoryOutcome::Applied(selection))
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) -> Result<HistoryOutcome, EditError> {
        let Some((transactions, selection)) = self.history.prepare_redo(self.revision())? else {
            return Ok(HistoryOutcome::Empty);
        };
        for transaction in transactions {
            let delta = self.buffer.commit(transaction)?;
            self.note_edit();
            self.emit_edited(delta, cx);
        }
        self.history.complete_redo();
        Ok(HistoryOutcome::Applied(selection))
    }

    fn emit_edited(&self, delta: RevisionDelta, cx: &mut Context<Self>) {
        cx.emit(DocumentEvent::Edited {
            document_id: self.id(),
            delta,
        });
    }

    fn note_edit(&mut self) {
        match &self.file.sync_state {
            SyncState::InSync { stamp } => {
                self.file.sync_state = SyncState::Dirty {
                    base: stamp.clone(),
                };
            }
            SyncState::Dirty { .. } | SyncState::Conflict { .. } | SyncState::Missing { .. } => {}
        }
    }

    pub fn save_request(&self, target: Option<PathBuf>) -> Result<SaveRequest, SaveStartError> {
        if !matches!(self.file.save_state, SaveState::Idle) {
            return Err(SaveStartError::AlreadySaving);
        }
        let target_path =
            super::resolve_symlink_target(&target.unwrap_or_else(|| self.path.clone()));
        let same_target = target_path == super::resolve_symlink_target(&self.path);
        if same_target && matches!(self.file.sync_state, SyncState::Conflict { .. }) {
            return Err(SaveStartError::Conflict);
        }
        let expected_target = if same_target {
            match &self.file.sync_state {
                SyncState::InSync { stamp }
                | SyncState::Dirty { base: stamp }
                | SyncState::Missing { base: stamp } => {
                    if matches!(self.file.sync_state, SyncState::Missing { .. }) {
                        TargetExpectation::Exact(None)
                    } else {
                        TargetExpectation::Exact(Some(stamp.clone()))
                    }
                }
                SyncState::Conflict { .. } => unreachable!("conflict save requires explicit force"),
            }
        } else {
            match std::fs::metadata(&target_path) {
                Ok(_) => TargetExpectation::CaptureAtStart,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    TargetExpectation::Exact(None)
                }
                Err(_) => TargetExpectation::CaptureAtStart,
            }
        };
        Ok(SaveRequest {
            document_id: self.id(),
            save_point: self.save_point(),
            snapshot: self.snapshot(),
            source_path: self.path.clone(),
            target_path,
            expected_target,
            metadata: self.file.metadata.clone(),
        })
    }

    pub fn newline_sequence(&self) -> &'static str {
        match self.file.metadata.newline {
            super::OriginalNewline::CrLf => "\r\n",
            super::OriginalNewline::None
            | super::OriginalNewline::Lf
            | super::OriginalNewline::Mixed => "\n",
        }
    }

    pub fn begin_save(&mut self, target: Option<PathBuf>) -> Result<SaveRequest, SaveStartError> {
        let request = self.save_request(target)?;
        self.file.save_state = SaveState::Saving {
            revision: request.revision(),
            target: request.target_path.clone(),
        };
        Ok(request)
    }

    pub fn begin_force_save(&mut self) -> Result<SaveRequest, SaveStartError> {
        if !matches!(self.file.save_state, SaveState::Idle) {
            return Err(SaveStartError::AlreadySaving);
        }
        let SyncState::Conflict { external, .. } = &self.file.sync_state else {
            return self.begin_save(None);
        };
        let request = SaveRequest {
            document_id: self.id(),
            save_point: self.save_point(),
            snapshot: self.snapshot(),
            source_path: self.path.clone(),
            target_path: super::resolve_symlink_target(&self.path),
            expected_target: TargetExpectation::Exact(Some(external.clone())),
            metadata: self.file.metadata.clone(),
        };
        self.file.save_state = SaveState::Saving {
            revision: request.revision(),
            target: request.target_path.clone(),
        };
        Ok(request)
    }

    pub fn cancel_save(&mut self, revision: Revision) {
        if matches!(
            self.file.save_state,
            SaveState::Saving {
                revision: active,
                ..
            } if active == revision
        ) {
            self.file.save_state = SaveState::Idle;
        }
    }

    pub fn finish_save(
        &mut self,
        outcome: SaveOutcome,
        cx: &mut Context<Self>,
    ) -> Result<bool, SaveAckError> {
        if outcome.document_id != self.id() {
            return Err(SaveAckError::DifferentDocument);
        }
        let SaveState::Saving {
            revision, target, ..
        } = &self.file.save_state
        else {
            return Err(SaveAckError::NotSaving);
        };
        if *revision != outcome.save_point.revision() || *target != outcome.target_path {
            return Err(SaveAckError::DifferentTarget);
        }
        if outcome.source_path != self.path {
            return Err(SaveAckError::DifferentTarget);
        }
        self.path = outcome.target_path;
        self.file.metadata = outcome.metadata;
        self.file.save_state = SaveState::Idle;
        self.file.sync_state = if self.revision() == outcome.save_point.revision() {
            SyncState::InSync {
                stamp: outcome.stamp,
            }
        } else {
            SyncState::Dirty {
                base: outcome.stamp,
            }
        };
        self.mark_saved(outcome.save_point, cx)
    }

    pub fn observe_disk(&mut self, observed: Option<FileStamp>) -> DiskChangeAction {
        if matches!(self.file.save_state, SaveState::Saving { .. }) {
            return DiskChangeAction::Defer;
        }
        let base = self.file.sync_state.base().clone();
        match observed {
            Some(stamp) if &stamp == self.file.sync_state.base() => {
                if matches!(
                    self.file.sync_state,
                    SyncState::Conflict { .. } | SyncState::Missing { .. }
                ) {
                    self.file.sync_state = if self.is_dirty() {
                        SyncState::Dirty { base: stamp }
                    } else {
                        SyncState::InSync { stamp }
                    };
                    DiskChangeAction::Recovered
                } else {
                    DiskChangeAction::Ignore
                }
            }
            None => {
                self.file.sync_state = SyncState::Missing { base };
                DiskChangeAction::Missing
            }
            Some(_) if !self.is_dirty() => DiskChangeAction::Reload,
            Some(external) => {
                self.file.sync_state = SyncState::Conflict { base, external };
                DiskChangeAction::Conflict
            }
        }
    }

    pub fn retarget_moved_file(
        &mut self,
        destination: PathBuf,
        stamp: FileStamp,
        permissions: Option<std::fs::Permissions>,
        cx: &mut Context<Self>,
    ) -> std::io::Result<()> {
        if !matches!(self.file.save_state, SaveState::Idle) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "cannot move a document while it is being saved",
            ));
        }
        self.file.metadata.permissions = permissions;
        self.path = destination.clone();
        self.file.sync_state = if self.is_dirty() {
            SyncState::Dirty { base: stamp }
        } else {
            SyncState::InSync { stamp }
        };
        cx.emit(DocumentEvent::PathChanged {
            document_id: self.id(),
            path: destination,
        });
        Ok(())
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
        self.file.metadata = prepared.metadata;
        self.file.sync_state = SyncState::InSync {
            stamp: prepared.stamp,
        };
        self.file.save_state = SaveState::Idle;
        self.history.clear();
        cx.emit(DocumentEvent::Reloaded {
            document_id: self.id(),
            delta: delta.clone(),
        });
        Ok(delta)
    }
}

fn inverse_edits(snapshot: &DocumentSnapshot, forward: &[TextEdit]) -> Vec<TextEdit> {
    let mut shift = 0_i128;
    forward
        .iter()
        .map(|edit| {
            let start = u64::try_from(i128::from(edit.range.start.0) + shift)
                .expect("validated edit shift remains in document bounds");
            let replacement_len = edit.replacement.len() as u64;
            let original = snapshot.copy_range(edit.range);
            shift += i128::from(replacement_len) - i128::from(edit.range.len());
            TextEdit::new(
                ByteRange {
                    start: ByteOffset(start),
                    end: ByteOffset(start + replacement_len),
                },
                original,
            )
        })
        .collect()
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
                .apply_transient_edit(
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
                .apply_transient_edit(
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
                .apply_transient_edit(
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
    fn editing_while_atomic_save_runs_keeps_newer_revision_dirty(cx: &mut gpui::TestAppContext) {
        let directory = std::env::temp_dir().join(format!(
            "org-studio-save-race-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("notes.org");
        std::fs::write(&path, b"one").unwrap();
        let session = cx.new(|_| {
            DocumentSession::from_utf8(path.clone(), std::fs::read(&path).unwrap()).unwrap()
        });
        let request = session.update(cx, |session, cx| {
            session
                .apply_transient_edit(
                    EditTransaction::new(
                        session.revision(),
                        vec![TextEdit::new(ByteRange::new(3, 3), " two")],
                    ),
                    cx,
                )
                .unwrap();
            session.begin_save(None).unwrap()
        });
        let outcome = crate::document::write_atomic(request).unwrap();
        session.update(cx, |session, cx| {
            session
                .apply_transient_edit(
                    EditTransaction::new(
                        session.revision(),
                        vec![TextEdit::new(ByteRange::new(7, 7), " three")],
                    ),
                    cx,
                )
                .unwrap();
            session.finish_save(outcome, cx).unwrap();
            assert_eq!(session.saved_revision(), Revision(1));
            assert_eq!(session.revision(), Revision(2));
            assert!(session.is_dirty());
            assert!(matches!(session.sync_state(), SyncState::Dirty { .. }));
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"one two");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[gpui::test]
    fn save_as_retargets_the_session_and_keeps_it_editable(cx: &mut gpui::TestAppContext) {
        let directory = std::env::temp_dir().join(format!(
            "org-studio-save-as-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let source = directory.join("source.org");
        let destination = directory.join("destination.org");
        std::fs::write(&source, b"base").unwrap();
        let session = cx.new(|_| {
            DocumentSession::from_utf8(source.clone(), std::fs::read(&source).unwrap()).unwrap()
        });
        let request = session.update(cx, |session, cx| {
            session
                .apply_transient_edit(
                    EditTransaction::new(
                        session.revision(),
                        vec![TextEdit::new(ByteRange::new(4, 4), " saved")],
                    ),
                    cx,
                )
                .unwrap();
            session.begin_save(Some(destination.clone())).unwrap()
        });
        let outcome = crate::document::write_atomic(request).unwrap();
        session.update(cx, |session, cx| {
            session.finish_save(outcome, cx).unwrap();
            assert_eq!(session.path(), destination);
            assert!(!session.is_dirty());
            session
                .apply_transient_edit(
                    EditTransaction::new(
                        session.revision(),
                        vec![TextEdit::new(ByteRange::new(10, 10), " again")],
                    ),
                    cx,
                )
                .unwrap();
            assert!(session.is_dirty());
        });
        assert_eq!(std::fs::read(&destination).unwrap(), b"base saved");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[gpui::test]
    fn disk_observations_suppress_self_writes_and_preserve_dirty_text(
        cx: &mut gpui::TestAppContext,
    ) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("notes.org"), b"base".to_vec()).unwrap()
        });
        session.update(cx, |session, cx| {
            let base = session.sync_state().base().clone();
            assert_eq!(
                session.observe_disk(Some(base.clone())),
                DiskChangeAction::Ignore
            );
            session
                .apply_transient_edit(
                    EditTransaction::new(
                        session.revision(),
                        vec![TextEdit::new(ByteRange::new(4, 4), " local")],
                    ),
                    cx,
                )
                .unwrap();
            let mut external = base.clone();
            external.content_hash ^= 1;
            assert_eq!(
                session.observe_disk(Some(external)),
                DiskChangeAction::Conflict
            );
            assert!(matches!(session.sync_state(), SyncState::Conflict { .. }));
            assert_eq!(contents(session), "base local");
            assert_eq!(
                session.observe_disk(Some(base.clone())),
                DiskChangeAction::Recovered
            );
            assert!(matches!(session.sync_state(), SyncState::Dirty { .. }));
            assert_eq!(session.observe_disk(None), DiskChangeAction::Missing);
            assert!(matches!(session.sync_state(), SyncState::Missing { .. }));
            assert_eq!(
                session.observe_disk(Some(base)),
                DiskChangeAction::Recovered
            );
            assert!(matches!(session.sync_state(), SyncState::Dirty { .. }));
        });
    }

    #[test]
    fn newline_sequence_follows_the_loaded_document_style() {
        let crlf = DocumentSession::from_utf8(
            PathBuf::from("windows.org"),
            b"first\r\nsecond\r\n".to_vec(),
        )
        .unwrap();
        let mixed =
            DocumentSession::from_utf8(PathBuf::from("mixed.org"), b"first\r\nsecond\n".to_vec())
                .unwrap();

        assert_eq!(crlf.newline_sequence(), "\r\n");
        assert_eq!(mixed.newline_sequence(), "\n");
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
                .apply_transient_edit(
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

    #[gpui::test]
    fn edit_history_coalesces_typing_and_restores_selection(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(PathBuf::from("notes.org"), b"x".to_vec()).unwrap()
        });
        for (offset, text) in [(1, "a"), (2, "🙂")] {
            session.update(cx, |session, cx| {
                let before = Selection::caret(ByteOffset(offset));
                let after = Selection::caret(ByteOffset(offset + text.len() as u64));
                session
                    .edit(
                        SessionEdit::new(
                            EditTransaction::new(
                                session.revision(),
                                vec![TextEdit::new(ByteRange::new(offset, offset), text)],
                            ),
                            before,
                            after,
                            EditOrigin::Typing,
                        ),
                        cx,
                    )
                    .unwrap();
            });
        }
        assert_eq!(cx.read(|cx| contents(session.read(cx))), "xa🙂");
        session.update(cx, |session, cx| {
            assert_eq!(
                session.undo(cx).unwrap(),
                HistoryOutcome::Applied(Selection::caret(ByteOffset(1)))
            );
        });
        assert_eq!(cx.read(|cx| contents(session.read(cx))), "x");
        session.update(cx, |session, cx| {
            assert_eq!(
                session.redo(cx).unwrap(),
                HistoryOutcome::Applied(Selection::caret(ByteOffset(6)))
            );
        });
        assert_eq!(cx.read(|cx| contents(session.read(cx))), "xa🙂");
    }
}
