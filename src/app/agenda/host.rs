use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{ListAlignment, ListState, ScrollHandle, px};

use crate::agenda::{
    AgendaConfigStore, AgendaQuery, AgendaResultSnapshot, BuiltinQuery, QueryEngine,
};
use crate::motion::{Easing, FOLD_MOTION, MotionSpec, Tween};

use super::state::AgendaViewState;

#[derive(Clone, Copy, Debug)]
pub(super) struct SidebarResizeSession {
    pub(super) start_pointer_x: f32,
    pub(super) start_width: f32,
    pub(super) current_width: f32,
}

const SIDEBAR_MOTION: MotionSpec =
    MotionSpec::new(Duration::from_millis(180), Easing::EaseOutCubic);
const SIDEBAR_SWIPE_AXIS_LOCK_PX: f32 = 8.0;
const SIDEBAR_SWIPE_THRESHOLD_PX: f32 = 48.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SidebarSwipeAxis {
    #[default]
    Undecided,
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SidebarSwipeSession {
    delta_x: f32,
    delta_y: f32,
    axis: SidebarSwipeAxis,
    triggered: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct SidebarSwipeOutcome {
    pub(super) consumed: bool,
    pub(super) visibility_changed: bool,
}

fn sidebar_visibility_after_swipe(visible: bool, delta_x: f32) -> Option<bool> {
    if visible && delta_x <= -SIDEBAR_SWIPE_THRESHOLD_PX {
        Some(false)
    } else if !visible && delta_x >= SIDEBAR_SWIPE_THRESHOLD_PX {
        Some(true)
    } else {
        None
    }
}

pub(super) fn expanded_sidebar_width(viewport_width: f32, desired: f32) -> f32 {
    let maximum = (viewport_width - super::style::MIN_CONTENT_WIDTH).clamp(
        super::style::SIDEBAR_MIN_WIDTH,
        super::style::SIDEBAR_MAX_WIDTH,
    );
    desired.clamp(super::style::SIDEBAR_MIN_WIDTH, maximum)
}

pub(super) fn resized_sidebar_width(
    viewport_width: f32,
    session: SidebarResizeSession,
    pointer_x: f32,
) -> f32 {
    expanded_sidebar_width(
        viewport_width,
        session.start_width + pointer_x - session.start_pointer_x,
    )
}

pub(crate) struct AgendaHost {
    pub(super) language: crate::i18n::Language,
    pub(crate) search_input: Option<gpui::Entity<super::search::AgendaSearch>>,
    pub(crate) text_editor: Option<gpui::Entity<crate::editor::SemanticEditor>>,
    pub(crate) text_view: crate::editor::GeneratedTextView<crate::agenda::AgendaPlacementRef>,
    pub(super) text_projection_version: Option<u64>,
    pub(crate) pending_text_task: Option<crate::agenda::TaskRecord>,
    pub(crate) pending_text_generation: Option<u64>,
    pub(super) text_generation: Option<(
        Option<crate::agenda::QueryId>,
        u64,
        crate::i18n::Language,
        jiff::civil::Date,
    )>,
    pub(super) navigation_result: Option<Arc<AgendaResultSnapshot>>,
    pub(crate) state: AgendaViewState,
    pub(super) list_state: ListState,
    pub(super) sidebar_scroll: ScrollHandle,
    pub(super) sidebar_visible: bool,
    pub(super) sidebar_width: f32,
    pub(super) sidebar_resize: Option<SidebarResizeSession>,
    pub(super) sidebar_visibility_animation: Option<Tween>,
    pub(super) sidebar_section_animations: [Option<Tween>; 4],
    pub(super) sidebar_swipe: Option<SidebarSwipeSession>,
    pub(super) inspector_scroll: ScrollHandle,
    pub(super) analysis_generation: u64,
    navigation_engine: QueryEngine,
    pub(super) page_query: super::query_session::AgendaQuerySession,
    pub(crate) text_query: super::query_session::AgendaQuerySession,
    pub(crate) query_runtime: super::query_session::AgendaQueryRuntime,
    pub(super) scan_progress: crate::file_watcher::InitialScanProgress,
    pub(super) config: crate::agenda::AgendaConfig,
    pub(super) recovery_receipt: Option<std::path::PathBuf>,
    pub(super) clock_store: crate::agenda::ClockStore,
    pub(super) clock_store_path: Option<std::path::PathBuf>,
    pub(super) runtime: super::runtime::AgendaRuntime,
}

impl AgendaHost {
    pub(crate) fn new() -> Self {
        let mut query_runtime = super::query_session::AgendaQueryRuntime::default();
        let page_query = query_runtime.open();
        let text_query = query_runtime.open();
        let mut host = Self {
            language: crate::i18n::Language::system(),
            search_input: None,
            text_editor: None,
            text_view: Default::default(),
            text_projection_version: None,
            pending_text_task: None,
            pending_text_generation: None,
            text_generation: None,
            navigation_result: None,
            state: AgendaViewState::default(),
            list_state: ListState::new(0, ListAlignment::Top, px(80.0)),
            sidebar_scroll: ScrollHandle::new(),
            sidebar_visible: true,
            sidebar_width: super::style::SIDEBAR_WIDTH,
            sidebar_resize: None,
            sidebar_visibility_animation: None,
            sidebar_section_animations: [None; 4],
            sidebar_swipe: None,
            inspector_scroll: ScrollHandle::new(),
            analysis_generation: 0,
            navigation_engine: QueryEngine::default(),
            page_query,
            text_query,
            query_runtime,
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

    pub(super) fn sidebar_reveal_at(&self, now: Instant) -> (f32, bool) {
        let Some(animation) = self.sidebar_visibility_animation else {
            return (if self.sidebar_visible { 1.0 } else { 0.0 }, false);
        };
        let sample = animation.sample(now);
        (sample.value, sample.active)
    }

    pub(super) fn sidebar_section_reveal_at(
        &self,
        section: super::state::SidebarSection,
        now: Instant,
    ) -> (f32, bool) {
        let expanded = match section {
            super::state::SidebarSection::SmartViews => self.state.smart_views_expanded,
            super::state::SidebarSection::SavedViews => self.state.saved_views_expanded,
            super::state::SidebarSection::Tags => self.state.tags_expanded,
            super::state::SidebarSection::Sources => self.state.sources_expanded,
        };
        let Some(animation) = self.sidebar_section_animations[sidebar_section_index(section)]
        else {
            return (if expanded { 1.0 } else { 0.0 }, false);
        };
        let sample = animation.sample(now);
        (sample.value, sample.active)
    }

    pub(super) fn toggle_sidebar_section(
        &mut self,
        section: super::state::SidebarSection,
        now: Instant,
        animate: bool,
    ) {
        let from = self.sidebar_section_reveal_at(section, now).0;
        self.state.toggle_sidebar_section(section);
        let to = match section {
            super::state::SidebarSection::SmartViews => self.state.smart_views_expanded,
            super::state::SidebarSection::SavedViews => self.state.saved_views_expanded,
            super::state::SidebarSection::Tags => self.state.tags_expanded,
            super::state::SidebarSection::Sources => self.state.sources_expanded,
        };
        self.sidebar_section_animations[sidebar_section_index(section)] = animate.then_some(
            Tween::new(now, from, if to { 1.0 } else { 0.0 }, FOLD_MOTION),
        );
    }

    fn set_sidebar_visible(&mut self, visible: bool, now: Instant, animate: bool) -> bool {
        if self.sidebar_visible == visible {
            return false;
        }
        let from = self.sidebar_reveal_at(now).0;
        self.sidebar_visible = visible;
        self.sidebar_resize = None;
        self.sidebar_visibility_animation = animate.then_some(Tween::new(
            now,
            from,
            if visible { 1.0 } else { 0.0 },
            SIDEBAR_MOTION,
        ));
        true
    }

    pub(super) fn handle_sidebar_swipe(
        &mut self,
        delta_x: f32,
        delta_y: f32,
        phase: gpui::TouchPhase,
        now: Instant,
        animate: bool,
    ) -> SidebarSwipeOutcome {
        if matches!(phase, gpui::TouchPhase::Started) {
            self.sidebar_swipe = Some(SidebarSwipeSession::default());
        }
        let mut swipe = self.sidebar_swipe.unwrap_or_default();
        swipe.delta_x += delta_x;
        swipe.delta_y += delta_y;

        if swipe.axis == SidebarSwipeAxis::Undecided
            && swipe.delta_x.abs().max(swipe.delta_y.abs()) >= SIDEBAR_SWIPE_AXIS_LOCK_PX
        {
            swipe.axis = if swipe.delta_x.abs() > swipe.delta_y.abs() * 1.2 {
                SidebarSwipeAxis::Horizontal
            } else {
                SidebarSwipeAxis::Vertical
            };
        }

        let mut outcome = SidebarSwipeOutcome {
            consumed: swipe.axis == SidebarSwipeAxis::Horizontal,
            visibility_changed: false,
        };
        if outcome.consumed
            && !swipe.triggered
            && let Some(visible) =
                sidebar_visibility_after_swipe(self.sidebar_visible, swipe.delta_x)
        {
            swipe.triggered = true;
            outcome.visibility_changed = self.set_sidebar_visible(visible, now, animate);
        }

        if matches!(phase, gpui::TouchPhase::Ended) {
            self.sidebar_swipe = None;
        } else {
            self.sidebar_swipe = Some(swipe);
        }
        outcome
    }

    pub(super) fn apply_initial_view(&mut self) {
        if std::env::var_os("ORG_STUDIO_AGENDA_SELECT_FIRST").is_some()
            && self
                .page_query
                .result
                .as_ref()
                .is_some_and(|result| !result.placements.is_empty())
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
                if let Some(index) = self.page_query.result.as_ref().and_then(|result| {
                    result.placements.iter().position(|placement| {
                        result.entries[placement.entry.0 as usize]
                            .row
                            .title
                            .as_ref()
                            == "每日回顾"
                    })
                }) {
                    self.state.select_row(index);
                }
            }
            Ok("repeat") => {
                if let Some((index, key)) = self.page_query.result.as_ref().and_then(|result| {
                    result
                        .placements
                        .iter()
                        .enumerate()
                        .find(|(_, placement)| {
                            result.entries[placement.entry.0 as usize]
                                .row
                                .title
                                .as_ref()
                                == "每日回顾"
                        })
                        .map(|(index, placement)| {
                            (index, result.entries[placement.entry.0 as usize].row.task)
                        })
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
        if result.index_generation != self.runtime.index.snapshot().generation {
            return;
        }
        self.list_state.reset(result.placement_groups.len());
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
        let navigation_result = self
            .navigation_engine
            .execute(self.runtime.index.snapshot(), &query);
        self.navigation_result = Some(Arc::new(navigation_result));
        query.tag = self.state.tag_filter.clone();
        query.source = self.state.source_filter;
        if self.state.browses_dates() {
            query.window = Some(self.state.calendar_window(today));
        }
        let result = self
            .page_query
            .execute(self.runtime.index.snapshot(), &query);
        debug_assert!(!self.page_query.is_invalid());
        self.publish(result);
    }

    pub(crate) fn requery_independent_text(&mut self) {
        let today = jiff::Zoned::now().date();
        let mut query = AgendaQuery::builtin(BuiltinQuery::NextSevenDays, today);
        query.window = Some((
            today,
            today
                .checked_add(jiff::Span::new().days(6))
                .unwrap_or(today),
        ));
        self.text_query
            .execute(self.runtime.index.snapshot(), &query);
        self.text_generation = None;
    }

    pub(super) fn refresh_sources(&mut self) {
        self.query_runtime
            .source_changed(&mut [&mut self.page_query, &mut self.text_query]);
        self.analysis_generation += 1;
        self.scan_progress.finished = false;
        self.runtime.worker.submit(super::runtime::ScanRequest {
            identities: self.runtime.identities.clone(),
            roots: self.config.sources.clone(),
            previous: self.runtime.index.snapshot(),
            generation: self.analysis_generation,
            live: self.runtime.live.clone(),
        });
    }

    pub(super) fn task(&self, key: crate::agenda::TaskKey) -> Option<crate::agenda::TaskRecord> {
        self.runtime
            .index
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
            self.page_query
                .result
                .as_ref()
                .and_then(|result| result.placement_entry(index))
                .map(|(_, entry)| entry.row.task)
        })
    }

    pub(crate) fn status_counts(&self) -> (usize, usize, u8, usize) {
        let total = self
            .page_query
            .result
            .as_ref()
            .map_or(0, |result| result.placements.len());
        (
            self.state.selected.map_or(0, |index| index + 1),
            total,
            self.scan_progress.percent(),
            self.page_query
                .result
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
        self.page_query
            .result
            .as_ref()
            .map_or(0, |result| result.placements.len())
    }

    pub(crate) fn close_top_layer(&mut self) -> bool {
        self.state.close_top_layer()
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        if self.state.projection == super::state::AgendaProjection::List {
            if let Some(result) = &self.page_query.result {
                self.state.move_visible_selection(delta, result);
                if let Some(index) = self.state.selected
                    && let Some(group) = result
                        .placement_groups
                        .iter()
                        .position(|group| group.placements.contains(&index))
                {
                    self.list_state.scroll_to_reveal_item(group);
                }
            }
            return;
        }
        self.state.move_selection(delta, self.row_count());
    }

    pub(super) fn toggle_day_group(&mut self, date: Option<jiff::civil::Date>) {
        let Some((index, group)) = self.page_query.result.as_ref().and_then(|result| {
            result
                .placement_groups
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
            .is_some_and(|selected| group.placements.contains(&selected))
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
            .and_then(|index| self.page_query.result.as_ref()?.placement_entry(index))
            .map(|(_, entry)| entry.row.task)
    }

    pub(crate) fn inbox_tasks(&self) -> Vec<crate::agenda::TaskRecord> {
        let configured = self.config.inbox.as_ref().map(|inbox| &inbox.file);
        let configured_heading = self
            .config
            .inbox
            .as_ref()
            .and_then(|inbox| inbox.heading.as_deref());
        self.runtime
            .index
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
        crate::agenda::derive_projects(&self.runtime.index.snapshot())
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

fn sidebar_section_index(section: super::state::SidebarSection) -> usize {
    match section {
        super::state::SidebarSection::SmartViews => 0,
        super::state::SidebarSection::SavedViews => 1,
        super::state::SidebarSection::Tags => 2,
        super::state::SidebarSection::Sources => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_width_is_clamped_and_resize_keeps_its_drag_origin() {
        assert_eq!(expanded_sidebar_width(1_200., 120.), 180.);
        assert_eq!(expanded_sidebar_width(1_200., 500.), 420.);
        assert_eq!(expanded_sidebar_width(700., 420.), 340.);

        let session = SidebarResizeSession {
            start_pointer_x: 220.,
            start_width: 220.,
            current_width: 280.,
        };
        assert_eq!(resized_sidebar_width(1_200., session, 300.), 300.);
    }

    #[test]
    fn horizontal_trackpad_swipes_hide_and_show_the_sidebar_once() {
        let now = Instant::now();
        let mut host = AgendaHost::new();
        let started = host.handle_sidebar_swipe(-10., 1., gpui::TouchPhase::Started, now, true);
        assert!(started.consumed);
        assert!(!started.visibility_changed);

        let hidden = host.handle_sidebar_swipe(-40., 1., gpui::TouchPhase::Moved, now, true);
        assert!(hidden.consumed);
        assert!(hidden.visibility_changed);
        assert!(!host.sidebar_visible);

        let repeated = host.handle_sidebar_swipe(-60., 0., gpui::TouchPhase::Moved, now, true);
        assert!(!repeated.visibility_changed);
        host.handle_sidebar_swipe(0., 0., gpui::TouchPhase::Ended, now, true);

        host.handle_sidebar_swipe(10., 1., gpui::TouchPhase::Started, now, true);
        let shown = host.handle_sidebar_swipe(40., 1., gpui::TouchPhase::Moved, now, true);
        assert!(shown.visibility_changed);
        assert!(host.sidebar_visible);
    }

    #[test]
    fn vertical_trackpad_scroll_does_not_toggle_or_get_consumed() {
        let now = Instant::now();
        let mut host = AgendaHost::new();
        let outcome = host.handle_sidebar_swipe(-10., -40., gpui::TouchPhase::Started, now, true);
        assert_eq!(outcome, SidebarSwipeOutcome::default());
        assert!(host.sidebar_visible);
    }

    #[test]
    fn sidebar_visibility_uses_a_smooth_bounded_animation() {
        let started_at = Instant::now();
        let mut host = AgendaHost::new();
        assert!(host.set_sidebar_visible(false, started_at, true));
        assert_eq!(host.sidebar_reveal_at(started_at), (1.0, true));

        let halfway = host
            .sidebar_reveal_at(started_at + SIDEBAR_MOTION.duration() / 2)
            .0;
        assert!(halfway > 0.0 && halfway < 1.0);
        assert_eq!(
            host.sidebar_reveal_at(started_at + SIDEBAR_MOTION.duration()),
            (0.0, false)
        );
    }

    #[test]
    fn sidebar_section_animation_is_smooth_and_reversible() {
        let started_at = Instant::now();
        let mut host = AgendaHost::new();
        let section = super::super::state::SidebarSection::SmartViews;
        assert!(host.state.smart_views_expanded);

        host.toggle_sidebar_section(section, started_at, true);
        assert!(!host.state.smart_views_expanded);
        assert_eq!(
            host.sidebar_section_reveal_at(section, started_at),
            (1.0, true)
        );

        let halfway_at = started_at + FOLD_MOTION.duration() / 2;
        let halfway = host.sidebar_section_reveal_at(section, halfway_at).0;
        assert!(halfway > 0.0 && halfway < 1.0);

        host.toggle_sidebar_section(section, halfway_at, true);
        assert!(host.state.smart_views_expanded);
        let reversed_from = host.sidebar_section_reveal_at(section, halfway_at).0;
        assert!((reversed_from - halfway).abs() < f32::EPSILON);
        assert_eq!(
            host.sidebar_section_reveal_at(section, halfway_at + FOLD_MOTION.duration()),
            (1.0, false)
        );
    }

    #[test]
    fn sidebar_section_animation_respects_reduced_motion() {
        let now = Instant::now();
        let mut host = AgendaHost::new();
        let section = super::super::state::SidebarSection::Tags;

        host.toggle_sidebar_section(section, now, false);

        assert!(!host.state.tags_expanded);
        assert_eq!(host.sidebar_section_reveal_at(section, now), (0.0, false));
    }

    #[test]
    fn sidebar_visibility_respects_reduced_motion() {
        let now = Instant::now();
        let mut host = AgendaHost::new();

        host.handle_sidebar_swipe(-10., 1., gpui::TouchPhase::Started, now, false);
        let hidden = host.handle_sidebar_swipe(-40., 1., gpui::TouchPhase::Moved, now, false);

        assert!(hidden.visibility_changed);
        assert!(!host.sidebar_visible);
        assert!(host.sidebar_visibility_animation.is_none());
        assert_eq!(host.sidebar_reveal_at(now), (0.0, false));
    }

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
        host.runtime.index = crate::agenda::AgendaIndex::default();
        let index = host.runtime.index.replace(crate::agenda::shard_from_live(
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
        host.page_query.result = Some(result.clone());
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
