use std::sync::Arc;

use gpui::{ListAlignment, ListState, ScrollHandle, px};

use crate::agenda::{
    AgendaConfigStore, AgendaIndex, AgendaQuery, AgendaResultSnapshot, BuiltinQuery, QueryEngine,
};

use super::state::AgendaViewState;

pub(crate) struct AgendaHost {
    pub(super) language: crate::i18n::Language,
    pub(crate) search_input: Option<gpui::Entity<super::search::AgendaSearch>>,
    pub(super) index: AgendaIndex,
    pub(super) result: Option<Arc<AgendaResultSnapshot>>,
    pub(super) navigation_result: Option<Arc<AgendaResultSnapshot>>,
    pub(crate) state: AgendaViewState,
    pub(super) list_state: ListState,
    pub(super) sidebar_scroll: ScrollHandle,
    pub(super) inspector_scroll: ScrollHandle,
    pub(super) analysis_generation: u64,
    pub(super) query_generation: u64,
    query_engine: QueryEngine,
    pub(super) scan_progress: crate::file_watcher::InitialScanProgress,
    pub(super) config: crate::agenda::AgendaConfig,
    pub(super) recovery_receipt: Option<std::path::PathBuf>,
    pub(super) clock_store: crate::agenda::ClockStore,
    pub(super) clock_store_path: Option<std::path::PathBuf>,
    pub(super) runtime: super::runtime::AgendaRuntime,
}

impl AgendaHost {
    pub(crate) fn new() -> Self {
        let mut host = Self {
            language: crate::i18n::Language::system(),
            search_input: None,
            index: AgendaIndex::default(),
            result: None,
            navigation_result: None,
            state: AgendaViewState::default(),
            list_state: ListState::new(0, ListAlignment::Top, px(80.0)),
            sidebar_scroll: ScrollHandle::new(),
            inspector_scroll: ScrollHandle::new(),
            analysis_generation: 0,
            query_generation: 0,
            query_engine: QueryEngine::default(),
            scan_progress: crate::file_watcher::InitialScanProgress::default(),
            config: crate::agenda::AgendaConfig::default(),
            recovery_receipt: None,
            clock_store: crate::agenda::ClockStore::default(),
            clock_store_path: None,
            runtime: super::runtime::AgendaRuntime::default(),
        };
        host.load_initial();
        if std::env::var_os("ORG_STUDIO_AGENDA_SCROLL_SIDEBAR_BOTTOM").is_some() {
            host.sidebar_scroll.scroll_to_bottom();
        }
        if std::env::var_os("ORG_STUDIO_AGENDA_SCROLL_INSPECTOR_BOTTOM").is_some() {
            host.inspector_scroll.scroll_to_bottom();
        }
        if std::env::var_os("ORG_STUDIO_AGENDA_CALENDAR").is_some() {
            host.state.projection = super::state::AgendaProjection::Calendar;
            host.state.calendar_range =
                match std::env::var("ORG_STUDIO_AGENDA_CALENDAR_RANGE").as_deref() {
                    Ok("day") => super::state::CalendarRange::Day,
                    Ok("month") => super::state::CalendarRange::Month,
                    _ => super::state::CalendarRange::Week,
                };
        }
        host
    }

    pub(super) fn apply_initial_view(&mut self) {
        if std::env::var_os("ORG_STUDIO_AGENDA_SELECT_FIRST").is_some()
            && self
                .result
                .as_ref()
                .is_some_and(|result| !result.rows.is_empty())
        {
            self.state.select_row(0);
        }
        match std::env::var("ORG_STUDIO_AGENDA_M4_VIEW").as_deref() {
            Ok("inbox") => {
                self.state.workspace = super::state::AgendaWorkspace::Inbox;
                self.state.navigation = Arc::from("收件箱");
            }
            Ok("projects") => {
                self.state.workspace = super::state::AgendaWorkspace::Projects;
                self.state.navigation = Arc::from("Projects");
            }
            Ok("capture") => self.state.overlay = super::state::AgendaOverlay::Capture,
            Ok("refile") => {
                self.state.workspace = super::state::AgendaWorkspace::Inbox;
                self.state.navigation = Arc::from("收件箱");
                if let Some(task) = self.inbox_tasks().first() {
                    self.state.refile_task = Some(task.key);
                    self.state.overlay = super::state::AgendaOverlay::Refile;
                }
            }
            Ok("organize") => {
                self.state.workspace = super::state::AgendaWorkspace::Inbox;
                self.state.navigation = Arc::from("收件箱");
                self.state.inbox_session = Some(crate::agenda::InboxSession::new(
                    self.inbox_tasks().into_iter().map(|task| task.key),
                ));
            }
            _ => {}
        }
        match std::env::var("ORG_STUDIO_AGENDA_M5_VIEW").as_deref() {
            Ok("habit") => {
                if let Some(index) = self.result.as_ref().and_then(|result| {
                    result
                        .rows
                        .iter()
                        .position(|row| row.title.as_ref() == "每日回顾")
                }) {
                    self.state.select_row(index);
                }
            }
            Ok("repeat") => {
                if let Some((index, key)) = self.result.as_ref().and_then(|result| {
                    result
                        .rows
                        .iter()
                        .enumerate()
                        .find(|(_, row)| row.title.as_ref() == "每日回顾")
                        .map(|(index, row)| (index, row.task))
                }) {
                    self.state.select_row(index);
                    self.state.repeat_task = Some(key);
                    self.state.repeat_target = Some(Arc::from("DONE"));
                    self.state.overlay = super::state::AgendaOverlay::Repeat;
                }
            }
            _ => {}
        }
    }

    pub(super) fn recovery_receipt_path(&self) -> Option<std::path::PathBuf> {
        let directory = crate::settings::application_support_dir()?;
        let config_path = std::env::var_os("ORG_STUDIO_AGENDA_CONFIG")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| directory.join("agenda.json"));
        Some(
            config_path
                .parent()
                .unwrap_or(directory.as_path())
                .join("agenda-refile-recovery.json"),
        )
    }

    fn load_initial(&mut self) {
        let Some(directory) = crate::settings::application_support_dir() else {
            return;
        };
        let config_path = std::env::var_os("ORG_STUDIO_AGENDA_CONFIG")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| directory.join("agenda.json"));
        let Ok(mut config) = AgendaConfigStore::new(config_path.clone()).load() else {
            return;
        };
        if let Some(base) = config_path.parent() {
            for source in &mut config.sources {
                if source.is_relative() {
                    *source = base.join(&*source);
                }
            }
            if let Some(inbox) = config.inbox.as_mut()
                && inbox.file.is_relative()
            {
                inbox.file = base.join(&inbox.file);
            }
        }
        self.state.saved_views_expanded = !config.saved_views.is_empty();
        self.config = config.clone();
        let receipt = config_path
            .parent()
            .unwrap_or(directory.as_path())
            .join("agenda-refile-recovery.json");
        let clock_path = config_path
            .parent()
            .unwrap_or(directory.as_path())
            .join("agenda-active-clock.json");
        match crate::agenda::ClockStore::load(&clock_path) {
            Ok(store) => {
                self.clock_store = store;
                self.clock_store_path = Some(clock_path);
            }
            Err(error) => {
                self.state.workflow_message = Some(Arc::from(format!(
                    "计时恢复文件读取失败，已保留原文件：{error}"
                )))
            }
        }
        if self.clock_store.active.is_some() {
            self.state.workflow_message = Some(Arc::from(
                "检测到上次退出时仍在计时；可在任务详情中继续或停止并写入 LOGBOOK。",
            ));
        }
        if receipt.exists()
            && crate::agenda::load_receipt(&receipt)
                .is_ok_and(|receipt| receipt.stage != crate::agenda::RecoveryStage::Complete)
        {
            self.recovery_receipt = Some(receipt);
            self.state.workflow_message = Some(Arc::from(
                "检测到未完成的跨文件移动；可继续移动或清理重复副本。",
            ));
        }
        self.refresh_sources();
    }

    pub(super) fn publish(&mut self, result: Arc<AgendaResultSnapshot>) {
        if result.index_generation != self.index.snapshot().generation
            || result.query_generation < self.query_generation
        {
            return;
        }
        self.query_generation = result.query_generation;
        self.list_state.reset(result.groups.len());
        self.result = Some(result);
    }

    pub(super) fn select_builtin(&mut self, builtin: BuiltinQuery) {
        self.state.builtin = builtin;
        self.requery();
    }

    pub(crate) fn requery(&mut self) {
        let today = jiff::Zoned::now().date();
        let mut query = AgendaQuery::builtin(self.state.builtin, today);
        query.text = (!self.state.search.is_empty()).then(|| self.state.search.clone());
        query.todo = self.state.structured_todo.clone();
        query.scheduled = self.state.structured_scheduled;
        // Sidebar facets must not be derived from the already-filtered result: doing so
        // makes tags and sources disappear or change count as soon as they are clicked.
        let navigation_result = self.query_engine.execute(self.index.snapshot(), &query);
        self.navigation_result = Some(Arc::new(navigation_result));
        query.tag = self.state.tag_filter.clone();
        query.source = self.state.source_filter;
        if self.state.browses_dates() {
            query.window = Some(self.state.calendar_window(today));
        }
        let result = self.query_engine.execute(self.index.snapshot(), &query);
        self.publish(Arc::new(result));
    }

    pub(super) fn refresh_sources(&mut self) {
        self.analysis_generation += 1;
        self.scan_progress.finished = false;
        self.runtime.worker.submit(super::runtime::ScanRequest {
            identities: self.runtime.identities.clone(),
            roots: self.config.sources.clone(),
            previous: self.index.snapshot(),
            generation: self.analysis_generation,
            live: self.runtime.live.clone(),
        });
    }

    pub(super) fn task(&self, key: crate::agenda::TaskKey) -> Option<crate::agenda::TaskRecord> {
        self.index
            .snapshot()
            .files
            .iter()
            .find(|shard| shard.file == key.file)?
            .tasks
            .get(key.local as usize)
            .filter(|task| task.key == key)
            .cloned()
    }

    pub(crate) fn selected_task_key(&self) -> Option<crate::agenda::TaskKey> {
        self.state.selected.and_then(|index| {
            self.result
                .as_ref()
                .and_then(|result| result.rows.get(index))
                .map(|row| row.task)
        })
    }

    pub(crate) fn status_counts(&self) -> (usize, usize, u8, usize) {
        let total = self.result.as_ref().map_or(0, |result| result.rows.len());
        (
            self.state.selected.map_or(0, |index| index + 1),
            total,
            self.scan_progress.percent(),
            self.result
                .as_ref()
                .map_or(0, |result| result.diagnostics.len()),
        )
    }
    pub(crate) fn mode_name(&self) -> &'static str {
        match self.state.builtin {
            BuiltinQuery::Today => "Today",
            BuiltinQuery::NextSevenDays => "Next 7 Days",
            BuiltinQuery::Overdue => "Overdue",
            BuiltinQuery::Next => "NEXT",
            BuiltinQuery::Waiting => "Waiting",
            BuiltinQuery::Unscheduled => "Unscheduled",
        }
    }

    pub(crate) fn row_count(&self) -> usize {
        self.result.as_ref().map_or(0, |result| result.rows.len())
    }

    pub(crate) fn close_top_layer(&mut self) -> bool {
        self.state.close_top_layer()
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        if self.state.projection == super::state::AgendaProjection::List {
            if let Some(result) = &self.result {
                self.state.move_visible_selection(delta, result);
                if let Some(index) = self.state.selected
                    && let Some(group) = result
                        .groups
                        .iter()
                        .position(|group| group.rows.contains(&index))
                {
                    self.list_state.scroll_to_reveal_item(group);
                }
            }
            return;
        }
        self.state.move_selection(delta, self.row_count());
    }

    pub(super) fn toggle_day_group(&mut self, date: Option<jiff::civil::Date>) {
        let Some((index, group)) = self.result.as_ref().and_then(|result| {
            result
                .groups
                .iter()
                .enumerate()
                .find(|(_, group)| group.date == date)
        }) else {
            return;
        };
        if !self.state.collapsed_days.insert(date) {
            self.state.collapsed_days.remove(&date);
        } else if self
            .state
            .selected
            .is_some_and(|selected| group.rows.contains(&selected))
        {
            self.state.selected = None;
            self.state.inspector = None;
            self.state.sheet = super::state::AgendaSheet::None;
        }
        self.list_state.splice(index..index + 1, 1);
    }

    pub(crate) fn selected_task(&self) -> Option<crate::agenda::TaskKey> {
        self.state
            .selected
            .and_then(|index| self.result.as_ref()?.rows.get(index))
            .map(|row| row.task)
    }

    pub(crate) fn inbox_tasks(&self) -> Vec<crate::agenda::TaskRecord> {
        let configured = self.config.inbox.as_ref().map(|inbox| &inbox.file);
        let configured_heading = self
            .config
            .inbox
            .as_ref()
            .and_then(|inbox| inbox.heading.as_deref());
        self.index
            .snapshot()
            .files
            .iter()
            .flat_map(|file| file.tasks.iter())
            .filter(|task| {
                crate::agenda::task_matches_text(task, &self.state.search)
                    && !matches!(task.todo_kind, crate::org_semantic::TodoStateKind::Done)
                    && configured.is_some_and(|path| {
                        task.source.path.as_path() == path || task.source.path.ends_with(path)
                    })
                    && configured_heading.is_none_or(|heading| {
                        task.source.fingerprint.parent_title.as_deref() == Some(heading)
                    })
            })
            .cloned()
            .collect()
    }

    pub(crate) fn projects(&self) -> Vec<crate::agenda::ProjectSummary> {
        crate::agenda::derive_projects(&self.index.snapshot())
            .into_iter()
            .filter(|project| {
                crate::agenda::task_matches_text(&project.project, &self.state.search)
                    || project
                        .children
                        .iter()
                        .any(|task| crate::agenda::task_matches_text(task, &self.state.search))
            })
            .collect()
    }

    pub(crate) fn clock_is_active_for(&self, task: &crate::agenda::TaskRecord) -> bool {
        self.clock_store.active.as_ref().is_some_and(|active| {
            active.file == *task.source.path && active.heading_title == task.title.as_ref()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_folding_hides_selection_skips_rows_and_survives_refresh() {
        let snapshot = crate::document::DocumentSnapshot::from_utf8(
            b"* TODO One\n<2026-09-07>\n* TODO Two\n<2026-09-07>\n* TODO Three\n<2026-09-08>\n"
                .to_vec(),
        )
        .unwrap();
        let analysis =
            crate::org_semantic::analyze(&snapshot, Arc::new(crate::org_syntax::parse(&snapshot)));
        let mut host = AgendaHost::new();
        host.index = AgendaIndex::default();
        let index = host.index.replace(crate::agenda::shard_from_live(
            crate::agenda::FileId(1),
            1,
            Arc::new("/tmp/agenda-fold.org".into()),
            &analysis,
        ));
        let first = Some("2026-09-07".parse().unwrap());
        let second = Some("2026-09-08".parse().unwrap());
        let result = Arc::new(QueryEngine::default().execute(
            index,
            &AgendaQuery::builtin(BuiltinQuery::NextSevenDays, first.unwrap()),
        ));
        host.publish(result.clone());
        host.state.select_row(0);
        host.toggle_day_group(first);
        assert!(host.state.collapsed_days.contains(&first));
        assert!(host.state.selected.is_none());
        assert!(host.state.inspector.is_none());
        host.move_selection(1);
        assert_eq!(host.state.selected, Some(2));
        host.move_selection(-1);
        assert_eq!(host.state.selected, Some(2));
        host.publish(result);
        assert!(host.state.collapsed_days.contains(&first));
        host.toggle_day_group(second);
        host.move_selection(1);
        assert!(host.state.selected.is_none());
        host.toggle_day_group(first);
        host.move_selection(1);
        assert_eq!(host.state.selected, Some(0));
        assert!(host.state.collapsed_days.contains(&second));
    }
}
