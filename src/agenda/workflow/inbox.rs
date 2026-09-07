use super::super::TaskKey;
use std::{collections::VecDeque, sync::Arc};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InboxSession {
    pub(crate) items: Arc<[TaskKey]>,
    pub(crate) cursor: usize,
    pub(crate) processed: usize,
    skipped: VecDeque<TaskKey>,
}

impl InboxSession {
    pub(crate) fn new(items: impl IntoIterator<Item = TaskKey>) -> Self {
        Self {
            items: items.into_iter().collect::<Vec<_>>().into(),
            cursor: 0,
            processed: 0,
            skipped: VecDeque::new(),
        }
    }

    pub(crate) fn current(&self) -> Option<TaskKey> {
        self.items.get(self.cursor).copied()
    }

    pub(crate) fn skip(&mut self) {
        if let Some(task) = self.current() {
            self.skipped.push_back(task);
        }
        self.advance();
    }

    pub(crate) fn finish(&mut self) {
        self.processed += usize::from(self.current().is_some());
        self.advance();
    }

    fn advance(&mut self) {
        self.cursor += usize::from(self.cursor < self.items.len());
        if self.cursor == self.items.len() && !self.skipped.is_empty() {
            self.items = self.skipped.drain(..).collect::<Vec<_>>().into();
            self.cursor = 0;
        }
    }

    pub(crate) fn progress(&self) -> (usize, usize) {
        (
            self.processed,
            self.processed + self.items.len().saturating_sub(self.cursor) + self.skipped.len(),
        )
    }
}
