use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use super::{EditError, EditTransaction, Revision, Selection, TextEdit};

const DEFAULT_HISTORY_ENTRIES: usize = 10_000;
const DEFAULT_HISTORY_BYTES: usize = 64 * 1024 * 1024;
const COALESCE_INTERVAL: Duration = Duration::from_millis(750);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditOrigin {
    Typing,
    Newline,
    DeleteBackward,
    DeleteForward,
    Paste,
    Cut,
    Ime,
    Other,
}

impl EditOrigin {
    fn coalesces(self) -> bool {
        matches!(
            self,
            Self::Typing | Self::DeleteBackward | Self::DeleteForward
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryOutcome {
    Applied(Selection),
    Empty,
}

#[derive(Clone, Debug)]
pub(super) struct HistoryStep {
    pub(super) forward: Vec<TextEdit>,
    pub(super) inverse: Vec<TextEdit>,
}

impl HistoryStep {
    fn byte_cost(&self) -> usize {
        self.forward
            .iter()
            .chain(&self.inverse)
            .map(|edit| edit.replacement.len())
            .sum()
    }
}

#[derive(Clone, Debug)]
struct UndoEntry {
    before: Selection,
    after: Selection,
    origin: EditOrigin,
    last_edit_at: Instant,
    steps: Vec<HistoryStep>,
    byte_cost: usize,
}

impl UndoEntry {
    fn new(
        step: HistoryStep,
        before: Selection,
        after: Selection,
        origin: EditOrigin,
        now: Instant,
    ) -> Self {
        let byte_cost = step.byte_cost();
        Self {
            before,
            after,
            origin,
            last_edit_at: now,
            steps: vec![step],
            byte_cost,
        }
    }
}

pub(super) struct UndoHistory {
    undo: VecDeque<UndoEntry>,
    redo: VecDeque<UndoEntry>,
    byte_cost: usize,
    max_entries: usize,
    max_bytes: usize,
}

impl Default for UndoHistory {
    fn default() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            byte_cost: 0,
            max_entries: DEFAULT_HISTORY_ENTRIES,
            max_bytes: DEFAULT_HISTORY_BYTES,
        }
    }
}

impl UndoHistory {
    pub(super) fn record(
        &mut self,
        step: HistoryStep,
        before: Selection,
        after: Selection,
        origin: EditOrigin,
    ) {
        let now = Instant::now();
        self.redo.clear();
        let can_coalesce = self.undo.back().is_some_and(|entry| {
            origin.coalesces()
                && entry.origin == origin
                && entry.after == before
                && now.duration_since(entry.last_edit_at) <= COALESCE_INTERVAL
        });
        if can_coalesce {
            let entry = self.undo.back_mut().expect("coalescing entry exists");
            let cost = step.byte_cost();
            entry.steps.push(step);
            entry.after = after;
            entry.last_edit_at = now;
            entry.byte_cost += cost;
            self.byte_cost += cost;
        } else {
            let entry = UndoEntry::new(step, before, after, origin, now);
            self.byte_cost += entry.byte_cost;
            self.undo.push_back(entry);
        }
        self.enforce_budget();
    }

    pub(super) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.byte_cost = 0;
    }

    pub(super) fn clear_redo(&mut self) {
        self.redo.clear();
    }

    pub(super) fn prepare_undo(
        &self,
        revision: Revision,
    ) -> Result<Option<(Vec<EditTransaction>, Selection)>, EditError> {
        let Some(entry) = self.undo.back() else {
            return Ok(None);
        };
        let mut transactions = Vec::with_capacity(entry.steps.len());
        let mut base = revision;
        for step in entry.steps.iter().rev() {
            transactions.push(EditTransaction::new(base, step.inverse.clone()));
            base = base.checked_next().ok_or(EditError::RevisionExhausted)?;
        }
        Ok(Some((transactions, entry.before)))
    }

    pub(super) fn complete_undo(&mut self) {
        let entry = self.undo.pop_back().expect("prepared undo entry exists");
        self.byte_cost = self.byte_cost.saturating_sub(entry.byte_cost);
        self.redo.push_back(entry);
    }

    pub(super) fn prepare_redo(
        &self,
        revision: Revision,
    ) -> Result<Option<(Vec<EditTransaction>, Selection)>, EditError> {
        let Some(entry) = self.redo.back() else {
            return Ok(None);
        };
        let mut transactions = Vec::with_capacity(entry.steps.len());
        let mut base = revision;
        for step in &entry.steps {
            transactions.push(EditTransaction::new(base, step.forward.clone()));
            base = base.checked_next().ok_or(EditError::RevisionExhausted)?;
        }
        Ok(Some((transactions, entry.after)))
    }

    pub(super) fn complete_redo(&mut self) {
        let entry = self.redo.pop_back().expect("prepared redo entry exists");
        self.byte_cost += entry.byte_cost;
        self.undo.push_back(entry);
    }

    fn enforce_budget(&mut self) {
        while self.undo.len() > self.max_entries || self.byte_cost > self.max_bytes {
            let removed = self.undo.pop_front().expect("history budget exceeded");
            self.byte_cost = self.byte_cost.saturating_sub(removed.byte_cost);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ByteOffset, ByteRange};

    #[test]
    fn revision_exhaustion_does_not_move_history_between_stacks() {
        let mut history = UndoHistory::default();
        history.record(
            HistoryStep {
                forward: vec![TextEdit::new(ByteRange::new(0, 0), "x")],
                inverse: vec![TextEdit::new(ByteRange::new(0, 1), "")],
            },
            Selection::caret(ByteOffset(0)),
            Selection::caret(ByteOffset(1)),
            EditOrigin::Typing,
        );
        assert_eq!(
            history.prepare_undo(Revision(u64::MAX)),
            Err(EditError::RevisionExhausted)
        );
        assert!(history.prepare_undo(Revision::INITIAL).unwrap().is_some());
    }
}
