use std::sync::Arc;

use crate::agenda::{AgendaIndexSnapshot, AgendaQuery, AgendaResultSnapshot, QueryEngine, QueryId};

pub(crate) struct AgendaQuerySession {
    pub(super) id: QueryId,
    request_generation: u64,
    accepted_generation: u64,
    invalid: bool,
    closed: bool,
    visible: bool,
    source_generation: u64,
    engine: QueryEngine,
    pub(super) result: Option<Arc<AgendaResultSnapshot>>,
}

impl Drop for AgendaQuerySession {
    fn drop(&mut self) {
        self.close();
    }
}

impl AgendaQuerySession {
    pub(super) fn new(id: QueryId) -> Self {
        Self {
            id,
            request_generation: 0,
            accepted_generation: 0,
            invalid: true,
            closed: false,
            visible: true,
            source_generation: 0,
            engine: QueryEngine::default(),
            result: None,
        }
    }

    pub(super) fn execute(
        &mut self,
        index: Arc<AgendaIndexSnapshot>,
        query: &AgendaQuery,
    ) -> Arc<AgendaResultSnapshot> {
        self.request_generation += 1;
        let request = self.request_generation;
        let mut result = self.engine.execute(index, query);
        result.query_id = Some(self.id);
        result.query_generation = request;
        let result = Arc::new(result);
        self.accept(request, result.clone());
        result
    }

    fn accept(&mut self, request: u64, result: Arc<AgendaResultSnapshot>) -> bool {
        if self.closed
            || result.query_id != Some(self.id)
            || request != self.request_generation
            || request < self.accepted_generation
        {
            return false;
        }
        self.accepted_generation = request;
        self.invalid = false;
        self.result = Some(result);
        true
    }

    pub(super) fn note_source_change(&mut self, generation: u64) {
        if !self.closed && generation > self.source_generation {
            self.source_generation = generation;
            self.invalid = true;
        }
    }

    pub(crate) fn hide(&mut self) {
        self.visible = false;
    }

    /// Returns whether the caller must execute a fresh query before presenting this instance.
    pub(crate) fn resume(&mut self) -> bool {
        self.visible = true;
        self.invalid
    }

    pub(super) fn reopen(&mut self, id: QueryId) {
        *self = Self::new(id);
    }
    pub(super) fn is_invalid(&self) -> bool {
        self.invalid
    }
    pub(crate) fn close(&mut self) {
        self.closed = true;
        self.result = None;
    }
}

#[derive(Default)]
pub(crate) struct AgendaQueryRuntime {
    next_id: u64,
    source_generation: u64,
}

impl AgendaQueryRuntime {
    pub(super) fn open(&mut self) -> AgendaQuerySession {
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("Agenda query id exhausted");
        AgendaQuerySession::new(QueryId(self.next_id))
    }

    pub(crate) fn reopen(&mut self, session: &mut AgendaQuerySession) {
        let replacement = self.open();
        session.reopen(replacement.id);
    }

    pub(super) fn source_changed(&mut self, sessions: &mut [&mut AgendaQuerySession]) -> u64 {
        self.source_generation = self.source_generation.wrapping_add(1);
        for session in sessions {
            session.note_source_change(self.source_generation);
        }
        self.source_generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agenda::{AgendaFacets, AgendaQuery, BuiltinQuery};

    fn result(id: QueryId, generation: u64) -> Arc<AgendaResultSnapshot> {
        Arc::new(AgendaResultSnapshot {
            query_id: Some(id),
            query: Arc::new(AgendaQuery::builtin(
                BuiltinQuery::Today,
                "2026-09-07".parse().unwrap(),
            )),
            index_generation: 1,
            query_generation: generation,
            facets: AgendaFacets::default(),
            diagnostics: Arc::from([]),
            entries: Arc::from([]),
            placements: Arc::from([]),
            placement_groups: Arc::from([]),
        })
    }

    #[test]
    fn sessions_reject_cross_talk_stale_and_closed_results() {
        let mut first = AgendaQuerySession::new(QueryId(1));
        let mut second = AgendaQuerySession::new(QueryId(2));
        first.request_generation = 2;
        second.request_generation = 1;
        assert!(!first.accept(1, result(QueryId(1), 1)));
        assert!(!first.accept(2, result(QueryId(2), 2)));
        assert!(second.accept(1, result(QueryId(2), 1)));
        first.close();
        assert!(!first.accept(2, result(QueryId(1), 2)));
    }

    #[test]
    fn source_change_invalidates_sessions_independently() {
        let mut first = AgendaQuerySession::new(QueryId(1));
        let second = AgendaQuerySession::new(QueryId(2));
        first.invalid = false;
        first.note_source_change(1);
        assert!(first.is_invalid());
        assert!(second.is_invalid());
    }

    #[test]
    fn shared_runtime_isolates_queries_and_closed_reopened_instances() {
        let mut runtime = AgendaQueryRuntime::default();
        let mut page = runtime.open();
        let mut text = runtime.open();
        page.invalid = false;
        text.invalid = false;
        text.hide();
        runtime.source_changed(&mut [&mut page, &mut text]);
        assert!(page.is_invalid());
        assert!(text.resume());

        let old_id = text.id;
        let late = result(old_id, 1);
        text.close();
        runtime.reopen(&mut text);
        text.request_generation = 1;
        assert_ne!(text.id, old_id);
        assert!(!text.accept(1, late));

        page.request_generation = 1;
        assert!(page.accept(1, result(page.id, 1)));
        assert!(text.is_invalid());
    }
}
