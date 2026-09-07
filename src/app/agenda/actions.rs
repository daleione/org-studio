use std::sync::Arc;

use crate::{agenda::AgendaEditError, app::WorkspaceWindow};

use super::UiIntent;

impl WorkspaceWindow {
    pub(crate) fn open_agenda_source_file(
        &mut self,
        file: crate::agenda::FileId,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let path = self
            .agenda
            .index
            .snapshot()
            .files
            .iter()
            .find(|shard| shard.file == file)
            .map(|shard| shard.path.as_ref().clone());
        self.agenda.state.source_context_menu = None;
        let Some(path) = path else {
            cx.notify();
            return;
        };
        let already_open = self.document_path(cx).is_some_and(|current| {
            current == path
                || current
                    .canonicalize()
                    .ok()
                    .zip(path.canonicalize().ok())
                    .is_some_and(|(current, target)| current == target)
        });
        self.content_route = crate::app::ContentRoute::Document;
        self.request_document_focus(cx);
        // Reuse the live session, including unsaved edits and undo history.
        // Switching files must go through the existing save/discard/cancel guard.
        if !already_open {
            self.request_open(path, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn dispatch_agenda_intent(
        &mut self,
        intent: UiIntent,
        cx: &mut gpui::Context<Self>,
    ) {
        match intent {
            UiIntent::ToggleDayGroup(date) => self.agenda.toggle_day_group(date),
            UiIntent::ToggleSidebarSection(section) => {
                self.agenda.state.toggle_sidebar_section(section);
            }
            UiIntent::ShowSourceContextMenu(file, position) => {
                self.agenda.state.source_context_menu =
                    Some(super::state::SourceContextMenu { file, position });
            }
            UiIntent::CloseSourceContextMenu => {
                self.agenda.state.source_context_menu = None;
            }
            UiIntent::SelectNavigation(navigation, query) => {
                self.agenda.state.workspace = match navigation {
                    "收件箱" => super::state::AgendaWorkspace::Inbox,
                    "Projects" => super::state::AgendaWorkspace::Projects,
                    "Tasks" => super::state::AgendaWorkspace::Tasks,
                    _ => super::state::AgendaWorkspace::Agenda,
                };
                self.agenda.state.navigation = navigation.into();
                self.agenda.state.date_picker = false;
                if !self.agenda.state.browses_dates() {
                    self.agenda.state.projection = super::state::AgendaProjection::List;
                }
                self.agenda.state.tag_filter = None;
                self.agenda.state.source_filter = None;
                self.agenda.state.structured_todo = None;
                self.agenda.state.structured_scheduled = None;
                self.agenda.select_builtin(query);
            }
            UiIntent::SetProjection(projection) => {
                if projection == super::state::AgendaProjection::Calendar
                    && !self.agenda.state.browses_dates()
                {
                    return;
                }
                if self.agenda.state.projection != projection {
                    self.agenda.state.projection = projection;
                    self.agenda.requery();
                    if projection != super::state::AgendaProjection::List {
                        self.agenda.state.sheet = super::state::AgendaSheet::None;
                    }
                }
            }
            UiIntent::SetCalendarRange(range) => {
                if !self.agenda.state.browses_dates() {
                    return;
                }
                if self.agenda.state.calendar_range != range {
                    self.agenda.state.calendar_anchor =
                        Some(self.agenda.state.period_anchor(jiff::Zoned::now().date()));
                    self.agenda.state.calendar_range = range;
                    self.agenda.state.clear_task_selection();
                    self.agenda.state.calendar_offset = 0;
                    self.agenda.requery();
                }
            }
            UiIntent::ShiftCalendar(delta) => {
                if !self.agenda.state.browses_dates() {
                    return;
                }
                self.agenda.state.calendar_offset =
                    self.agenda.state.calendar_offset.saturating_add(delta);
                self.agenda.state.clear_task_selection();
                self.agenda.requery();
            }
            UiIntent::CalendarToday => {
                if !self.agenda.state.browses_dates() {
                    return;
                }
                if self.agenda.state.calendar_offset != 0
                    || self.agenda.state.calendar_anchor.is_some()
                {
                    self.agenda.state.calendar_offset = 0;
                    self.agenda.state.calendar_anchor = None;
                    self.agenda.state.clear_task_selection();
                    self.agenda.requery();
                }
            }
            UiIntent::ToggleAllDay => {
                self.agenda.state.all_day_expanded = !self.agenda.state.all_day_expanded;
            }
            UiIntent::ToggleSearch => {
                self.agenda.state.search_expanded = !self.agenda.state.search_expanded;
                self.agenda.state.search_focus_pending = self.agenda.state.search_expanded;
            }
            UiIntent::ToggleDatePicker => {
                self.agenda.state.date_picker = !self.agenda.state.date_picker;
                self.agenda.state.picker_offset = 0;
            }
            UiIntent::ShiftDatePicker(delta) => {
                self.agenda.state.picker_offset =
                    self.agenda.state.picker_offset.saturating_add(delta);
            }
            UiIntent::JumpToDate(date) => {
                if !self.agenda.state.browses_dates() {
                    return;
                }
                self.agenda.state.calendar_anchor = Some(date);
                self.agenda.state.clear_task_selection();
                self.agenda.state.calendar_offset = 0;
                self.agenda.state.date_picker = false;
                self.agenda.requery();
            }
            UiIntent::MoveOccurrence(index, date, time) => {
                let Some(row) = self
                    .agenda
                    .result
                    .as_ref()
                    .and_then(|result| result.rows.get(index))
                    .cloned()
                else {
                    return;
                };
                let Some(task) = self.agenda.task(row.task) else {
                    return;
                };
                let Some(mut timestamp) = task
                    .timestamps
                    .iter()
                    .find(|timestamp| {
                        timestamp.start_date == row.date.unwrap_or(timestamp.start_date)
                            && timestamp.start_time == row.time
                    })
                    .cloned()
                else {
                    return;
                };
                timestamp.start_date = date;
                timestamp.start_time = time;
                if timestamp.end_date.is_some() {
                    timestamp.end_date = Some(date);
                }
                if time.is_none() {
                    timestamp.end_time = None;
                }
                let target = match row.date_kind {
                    Some(crate::agenda::AgendaDateKind::Scheduled) => {
                        crate::agenda::TimestampTarget::Scheduled
                    }
                    Some(crate::agenda::AgendaDateKind::Deadline) => {
                        crate::agenda::TimestampTarget::Deadline
                    }
                    _ => crate::agenda::TimestampTarget::Plain,
                };
                self.dispatch_agenda_intent(
                    UiIntent::Apply(
                        row.task,
                        crate::agenda::AgendaCommand::SetTimestamp {
                            target,
                            value: Some(timestamp),
                        },
                    ),
                    cx,
                );
                return;
            }
            UiIntent::ResizeOccurrence(index, date, end_time) => {
                let Some(row) = self
                    .agenda
                    .result
                    .as_ref()
                    .and_then(|result| result.rows.get(index))
                    .cloned()
                else {
                    return;
                };
                if row.end_time.is_none() {
                    return;
                }
                let Some(task) = self.agenda.task(row.task) else {
                    return;
                };
                let Some(mut timestamp) = task
                    .timestamps
                    .iter()
                    .find(|timestamp| {
                        timestamp.start_date == row.date.unwrap_or(timestamp.start_date)
                            && timestamp.start_time == row.time
                    })
                    .cloned()
                else {
                    return;
                };
                timestamp.end_date = Some(date);
                timestamp.end_time = Some(end_time);
                let target = match row.date_kind {
                    Some(crate::agenda::AgendaDateKind::Scheduled) => {
                        crate::agenda::TimestampTarget::Scheduled
                    }
                    Some(crate::agenda::AgendaDateKind::Deadline) => {
                        crate::agenda::TimestampTarget::Deadline
                    }
                    _ => crate::agenda::TimestampTarget::Plain,
                };
                self.dispatch_agenda_intent(
                    UiIntent::Apply(
                        row.task,
                        crate::agenda::AgendaCommand::SetTimestamp {
                            target,
                            value: Some(timestamp),
                        },
                    ),
                    cx,
                );
                return;
            }
            UiIntent::SelectRow(index) => {
                self.agenda.state.select_row(index);
            }
            UiIntent::CloseInspector => {
                self.agenda.state.overlay = super::state::AgendaOverlay::None;
                self.agenda.state.selected = None;
                self.agenda.state.inspector = None;
                self.agenda.state.sheet = super::state::AgendaSheet::None;
            }
            UiIntent::CloseOverlay => {
                self.agenda.state.overlay = super::state::AgendaOverlay::None;
                self.agenda.state.refile_task = None;
                self.agenda.state.refile_search.clear();
                self.agenda.state.refile_selected = 0;
                self.agenda.state.workflow_message = None;
                self.focus_workspace_on_render = true;
            }
            UiIntent::RequestDelete => {
                self.agenda
                    .state
                    .inspector
                    .get_or_insert_default()
                    .confirm_delete = true;
            }
            UiIntent::CancelDelete => {
                self.agenda
                    .state
                    .inspector
                    .get_or_insert_default()
                    .confirm_delete = false;
            }
            UiIntent::ConfirmDelete(key) => {
                self.dispatch_agenda_intent(
                    UiIntent::Apply(key, crate::agenda::AgendaCommand::DeleteSubtree),
                    cx,
                );
                return;
            }
            UiIntent::Apply(key, operation) => {
                let Some(task) = self.agenda.task(key) else {
                    return self.agenda_error(AgendaEditError::SourceChanged, cx);
                };
                let result = self.apply_agenda_command(&task, &operation, cx);
                let inspector = self.agenda.state.inspector.get_or_insert_default();
                inspector.pending = false;
                inspector.error = result.err();
            }
            UiIntent::SetSearch(text) => {
                self.agenda.state.selected = None;
                self.agenda.state.inspector = None;
                self.agenda.state.search = text.into();
                self.agenda.requery();
            }
            UiIntent::SetTag(tag) => {
                self.agenda.state.navigation = Arc::from("tag");
                self.agenda.state.workspace = super::state::AgendaWorkspace::Tasks;
                self.agenda.state.projection = super::state::AgendaProjection::List;
                self.agenda.state.tag_filter = tag;
                self.agenda.state.source_filter = None;
                self.agenda.state.structured_todo = None;
                self.agenda.state.structured_scheduled = None;
                self.agenda.requery();
            }
            UiIntent::SetSource(source) => {
                self.agenda.state.navigation = Arc::from("source");
                self.agenda.state.workspace = super::state::AgendaWorkspace::Tasks;
                self.agenda.state.projection = super::state::AgendaProjection::List;
                self.agenda.state.source_filter = source;
                self.agenda.state.tag_filter = None;
                self.agenda.state.structured_todo = None;
                self.agenda.state.structured_scheduled = None;
                self.agenda.requery();
            }
            UiIntent::SelectSavedView(index) => {
                let Some(saved) = self.agenda.config.saved_views.get(index).cloned() else {
                    return;
                };
                self.agenda.state.navigation = Arc::from(saved.name);
                self.agenda.state.workspace = super::state::AgendaWorkspace::Tasks;
                self.agenda.state.projection = super::state::AgendaProjection::List;
                self.agenda.state.search = Arc::from(saved.query.text.unwrap_or_default());
                self.agenda.state.tag_filter = saved.query.tag.map(Arc::from);
                self.agenda.state.source_filter = saved.query.source.as_ref().and_then(|path| {
                    self.agenda
                        .index
                        .snapshot()
                        .files
                        .iter()
                        .find(|file| {
                            file.tasks
                                .first()
                                .is_some_and(|task| task.source.path.as_path() == path)
                        })
                        .map(|file| file.file)
                });
                self.agenda.state.structured_todo = saved.query.todo.clone().map(Arc::from);
                self.agenda.state.structured_scheduled = saved.query.scheduled;
                self.agenda.state.builtin = match saved.query.todo.as_deref() {
                    Some("NEXT") => crate::agenda::BuiltinQuery::Next,
                    Some("WAIT") | Some("WAITING") => crate::agenda::BuiltinQuery::Waiting,
                    _ if saved.query.scheduled == Some(false) => {
                        crate::agenda::BuiltinQuery::Unscheduled
                    }
                    _ => crate::agenda::BuiltinQuery::NextSevenDays,
                };
                self.agenda.requery();
            }
            UiIntent::OpenSource(key) => {
                let Some(task) = self.agenda.task(key) else {
                    return;
                };
                let anchor = task
                    .source
                    .anchor
                    .as_ref()
                    .map(|anchor| anchor.value.clone())
                    .unwrap_or_else(|| task.title.clone());
                self.open(task.source.path.as_ref().clone(), cx);
                self.pending_navigation = Some((self.generation, anchor));
            }
            UiIntent::OpenCapture => {
                self.agenda.state.capture = crate::agenda::CaptureDraft {
                    todo: "TODO".into(),
                    ..Default::default()
                };
                self.agenda.state.overlay = super::state::AgendaOverlay::Capture;
            }
            UiIntent::SetCaptureTemplate(template) => {
                self.agenda.state.capture.template = template;
            }
            UiIntent::SubmitCapture => {
                let Some(inbox) = self.agenda.config.inbox.clone() else {
                    self.agenda.state.workflow_message =
                        Some(Arc::from("请先在 agenda.json 配置 inbox 文件。"));
                    return cx.notify();
                };
                match self.capture_agenda(&inbox.file, inbox.heading.as_deref(), cx) {
                    Ok(saved) => {
                        self.agenda.state.overlay = super::state::AgendaOverlay::None;
                        self.agenda.state.workspace = super::state::AgendaWorkspace::Inbox;
                        self.agenda.state.navigation = Arc::from("收件箱");
                        self.agenda.state.workflow_message = Some(Arc::from(if saved {
                            "已保存到收件箱"
                        } else {
                            "已加入收件箱文档，等待保存"
                        }));
                        self.agenda.refresh_sources();
                    }
                    Err(error) => {
                        self.agenda.state.workflow_message =
                            Some(Arc::from(format!("Capture 失败：{error:?}")));
                    }
                }
            }
            UiIntent::StartInboxOrganize => {
                let keys = self.agenda.inbox_tasks().into_iter().map(|task| task.key);
                self.agenda.state.inbox_session = Some(crate::agenda::InboxSession::new(keys));
            }
            UiIntent::InboxSkip => {
                if let Some(session) = self.agenda.state.inbox_session.as_mut() {
                    session.skip();
                }
            }
            UiIntent::InboxFinish => {
                let key = self
                    .agenda
                    .state
                    .inbox_session
                    .as_ref()
                    .and_then(|session| session.current());
                if let Some(key) = key {
                    if let Some(task) = self.agenda.task(key) {
                        match self.apply_agenda_command(
                            &task,
                            &crate::agenda::AgendaCommand::SetTodo(Some(Arc::from("NEXT"))),
                            cx,
                        ) {
                            Ok(()) => {
                                if let Some(session) = self.agenda.state.inbox_session.as_mut() {
                                    session.finish();
                                }
                                self.agenda.refresh_sources();
                            }
                            Err(error) => {
                                self.agenda.state.workflow_message =
                                    Some(Arc::from(format!("整理失败，原文未改动：{error:?}")))
                            }
                        }
                    }
                }
            }
            UiIntent::ExitInboxOrganize => self.agenda.state.inbox_session = None,
            UiIntent::SelectProject(index) => self.agenda.state.selected_project = index,
            UiIntent::OpenProjectSource => {
                if let Some(project) = self
                    .agenda
                    .projects()
                    .get(self.agenda.state.selected_project)
                {
                    self.dispatch_agenda_intent(UiIntent::OpenSource(project.project.key), cx);
                    return;
                }
            }
            UiIntent::OpenRefile(task) => {
                self.agenda.state.refile_task = Some(task);
                self.agenda.state.refile_search.clear();
                self.agenda.state.refile_selected = 0;
                self.agenda.state.overlay = super::state::AgendaOverlay::Refile;
            }
            UiIntent::RefileNext(delta) => {
                let Some(task) = self.agenda.state.refile_task else {
                    return;
                };
                let targets = crate::agenda::refile_targets(
                    &self.agenda.index.snapshot(),
                    task,
                    &self.agenda.state.refile_search,
                    &self.agenda.state.recent_refile_targets,
                );
                if !targets.is_empty() {
                    self.agenda.state.refile_selected =
                        (self.agenda.state.refile_selected as i32 + delta)
                            .rem_euclid(targets.len() as i32) as usize;
                }
            }
            UiIntent::SubmitRefile => {
                let Some(source_key) = self.agenda.state.refile_task else {
                    return;
                };
                let targets = crate::agenda::refile_targets(
                    &self.agenda.index.snapshot(),
                    source_key,
                    &self.agenda.state.refile_search,
                    &self.agenda.state.recent_refile_targets,
                );
                let Some(target_ref) = targets.get(self.agenda.state.refile_selected) else {
                    return;
                };
                let Some(source) = self.agenda.task(source_key) else {
                    return;
                };
                let Some(target) = self.agenda.task(target_ref.task) else {
                    return;
                };
                let result = if source.key.file == target.key.file {
                    self.apply_agenda_command(
                        &source,
                        &crate::agenda::AgendaCommand::RefileSameFile(Box::new(target.clone())),
                        cx,
                    )
                    .map_err(crate::agenda::WorkflowError::Io)
                } else if let Some(receipt) = self.agenda.recovery_receipt_path() {
                    self.agenda_disk_task(&source, cx).and_then(|source| {
                        self.agenda_disk_task(&target, cx).and_then(|target| {
                            crate::agenda::cross_file_refile(&source, &target, &receipt).map(|_| ())
                        })
                    })
                } else {
                    Err(crate::agenda::WorkflowError::InvalidTarget)
                };
                match result {
                    Ok(()) => {
                        self.agenda
                            .state
                            .recent_refile_targets
                            .retain(|key| *key != target.key);
                        self.agenda
                            .state
                            .recent_refile_targets
                            .insert(0, target.key);
                        self.agenda.state.overlay = super::state::AgendaOverlay::None;
                        self.agenda.state.workflow_message = Some(Arc::from("移动完成"));
                        self.agenda.refresh_sources();
                    }
                    Err(error) => {
                        self.agenda.state.workflow_message =
                            Some(Arc::from(format!("移动未完成，原文已保留：{error:?}")))
                    }
                }
            }
            UiIntent::ResumeRecovery => {
                if let Some(path) = self.agenda.recovery_receipt.clone() {
                    match crate::agenda::resume_recovery(&path) {
                        Ok(_) => {
                            self.agenda.recovery_receipt = None;
                            self.agenda.state.workflow_message =
                                Some(Arc::from("已完成上次跨文件移动"));
                            self.agenda.refresh_sources();
                        }
                        Err(error) => {
                            self.agenda.state.workflow_message =
                                Some(Arc::from(format!("恢复失败，原文仍保留：{error:?}")))
                        }
                    }
                }
            }
            UiIntent::CleanupRecovery => {
                if let Some(path) = self.agenda.recovery_receipt.clone() {
                    match crate::agenda::cleanup_recovery_duplicate(&path) {
                        Ok(()) => {
                            self.agenda.recovery_receipt = None;
                            self.agenda.state.workflow_message =
                                Some(Arc::from("已清理目标文件的重复副本，源文保留"));
                            self.agenda.refresh_sources();
                        }
                        Err(error) => {
                            self.agenda.state.workflow_message =
                                Some(Arc::from(format!("清理失败：{error:?}")))
                        }
                    }
                }
            }
            UiIntent::ToggleClock(key) => self.toggle_agenda_clock(key, cx),
            UiIntent::RequestTodoTransition(key, target) => {
                let Some(task) = self.agenda.task(key) else {
                    return;
                };
                let completing_repeat = target.eq_ignore_ascii_case("DONE")
                    && task
                        .timestamps
                        .iter()
                        .any(|timestamp| timestamp.repeater.is_some());
                if completing_repeat {
                    self.agenda.state.repeat_task = Some(key);
                    self.agenda.state.repeat_target = Some(target);
                    self.agenda.state.overlay = super::state::AgendaOverlay::Repeat;
                } else {
                    self.dispatch_agenda_intent(
                        UiIntent::Apply(key, todo_transition(&task, target)),
                        cx,
                    );
                    return;
                }
            }
            UiIntent::ResolveRepeat(action) => {
                if action == crate::agenda::RepeatCompletionAction::Cancel {
                    self.agenda.state.overlay = super::state::AgendaOverlay::None;
                    self.agenda.state.repeat_task = None;
                    self.agenda.state.repeat_target = None;
                } else {
                    let Some(key) = self.agenda.state.repeat_task.take() else {
                        return;
                    };
                    let target = self
                        .agenda
                        .state
                        .repeat_target
                        .take()
                        .unwrap_or_else(|| Arc::from("DONE"));
                    self.agenda.state.overlay = super::state::AgendaOverlay::None;
                    let Some(task) = self.agenda.task(key) else {
                        return;
                    };
                    if action == crate::agenda::RepeatCompletionAction::Series {
                        self.dispatch_agenda_intent(
                            UiIntent::Apply(key, todo_transition(&task, target)),
                            cx,
                        );
                        return;
                    }
                    let Some(mut timestamp) = task
                        .timestamps
                        .iter()
                        .find(|timestamp| timestamp.repeater.is_some())
                        .cloned()
                    else {
                        return;
                    };
                    let today = jiff::Zoned::now().date();
                    if let Some(next) = crate::agenda::next_repeat_date(
                        timestamp.start_date,
                        today,
                        timestamp.repeater.expect("checked above"),
                    ) {
                        timestamp.start_date = next;
                        let target = match timestamp.kind {
                            crate::org_semantic::TimestampKind::Scheduled => {
                                crate::agenda::TimestampTarget::Scheduled
                            }
                            crate::org_semantic::TimestampKind::Deadline => {
                                crate::agenda::TimestampTarget::Deadline
                            }
                            _ => crate::agenda::TimestampTarget::Plain,
                        };
                        let repeat_to_state = task
                            .properties
                            .iter()
                            .find(|(property, _)| property.eq_ignore_ascii_case("REPEAT_TO_STATE"))
                            .map(|(_, value)| value.clone())
                            .unwrap_or_else(|| task.todo.clone());
                        let _ = target;
                        self.dispatch_agenda_intent(
                            UiIntent::Apply(
                                key,
                                crate::agenda::AgendaCommand::CompleteRepeat {
                                    timestamp,
                                    repeat_to_state,
                                    completed_at: Arc::from(
                                        jiff::Zoned::now()
                                            .strftime("%Y-%m-%d %a %H:%M")
                                            .to_string(),
                                    ),
                                },
                            ),
                            cx,
                        );
                        return;
                    }
                }
            }
        }
        cx.notify();
    }

    fn agenda_error(&mut self, error: AgendaEditError, cx: &mut gpui::Context<Self>) {
        let message: std::sync::Arc<str> = format!("{error:?}").into();
        let state = self.agenda.state.inspector.get_or_insert_default();
        state.pending = false;
        state.error = Some(message);
        cx.notify();
    }
}

#[cfg(test)]
mod toolbar_tests {
    use super::*;
    use crate::app::agenda::state::{AgendaProjection, CalendarRange};
    use gpui::AppContext;

    #[gpui::test]
    fn date_navigation_is_shared_by_projections_and_scoped_to_agenda(
        cx: &mut gpui::TestAppContext,
    ) {
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        workspace.update(cx, |workspace, cx| {
            let date = "2026-09-18".parse().unwrap();
            workspace.dispatch_agenda_intent(UiIntent::JumpToDate(date), cx);
            workspace.dispatch_agenda_intent(UiIntent::SetCalendarRange(CalendarRange::Day), cx);
            workspace.dispatch_agenda_intent(UiIntent::ShiftCalendar(1), cx);
            let window = workspace.agenda.state.calendar_window(date);
            assert_eq!(window.0.to_string(), "2026-09-19");
            workspace.agenda.state.search = "保留搜索".into();
            workspace
                .dispatch_agenda_intent(UiIntent::SetProjection(AgendaProjection::Calendar), cx);
            assert_eq!(workspace.agenda.state.calendar_window(date), window);
            assert_eq!(&*workspace.agenda.state.search, "保留搜索");
            workspace.dispatch_agenda_intent(UiIntent::SetProjection(AgendaProjection::List), cx);
            assert_eq!(workspace.agenda.state.calendar_window(date), window);
            workspace.dispatch_agenda_intent(
                UiIntent::SelectNavigation("Tasks", crate::agenda::BuiltinQuery::Next),
                cx,
            );
            workspace.dispatch_agenda_intent(UiIntent::ShiftCalendar(4), cx);
            workspace
                .dispatch_agenda_intent(UiIntent::SetProjection(AgendaProjection::Calendar), cx);
            assert_eq!(workspace.agenda.state.calendar_window(date), window);
            assert_eq!(workspace.agenda.state.projection, AgendaProjection::List);
            assert!(!workspace.agenda.state.browses_dates());
        });
    }
}

fn todo_transition(
    task: &crate::agenda::TaskRecord,
    target: Arc<str>,
) -> crate::agenda::AgendaCommand {
    let done = target.eq_ignore_ascii_case("DONE") || target.eq_ignore_ascii_case("CANCELLED");
    crate::agenda::AgendaCommand::TransitionTodo(crate::agenda::TodoTransition {
        target,
        target_kind: if done {
            crate::org_semantic::TodoStateKind::Done
        } else {
            crate::org_semantic::TodoStateKind::Open
        },
        timestamp: Arc::from("2026-09-06 Sun 00:00"),
        log_state: true,
        add_tags: Arc::from([]),
        remove_tags: if done {
            task.effective_tags
                .iter()
                .filter(|tag| tag.eq_ignore_ascii_case("waiting"))
                .cloned()
                .collect::<Vec<_>>()
                .into()
        } else {
            Arc::from([])
        },
    })
}

#[cfg(test)]
mod source_file_tests {
    use super::*;

    #[gpui::test]
    fn open_source_file_returns_to_document_without_reloading_live_session(
        cx: &mut gpui::TestAppContext,
    ) {
        let path =
            std::env::temp_dir().join(format!("agenda-open-source-{}.org", std::process::id()));
        std::fs::write(&path, "* TODO Original\n").unwrap();
        let loaded = crate::preview::load_workspace_document(path.clone(), false).unwrap();
        let shard =
            crate::agenda::shard_from_disk(crate::agenda::FileId(1), 1, path.clone()).unwrap();
        let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
        cx.update(|window, app| {
            workspace.update(app, |workspace, cx| {
                workspace.generation = 1;
                assert!(workspace.apply_load_result(1, Ok(loaded), cx));
                let session = workspace.document_session().unwrap().clone();
                session.update(cx, |session, cx| {
                    session
                        .apply_transient_edit(
                            crate::document::EditTransaction::new(
                                session.revision(),
                                vec![crate::document::TextEdit::new(
                                    crate::document::ByteRange::new(0, 0),
                                    "unsaved\n",
                                )],
                            ),
                            cx,
                        )
                        .unwrap();
                });
                workspace.agenda.index.replace(shard);
                workspace.content_route = crate::app::ContentRoute::Agenda;
                workspace.open_agenda_source_file(crate::agenda::FileId(1), window, cx);
                assert!(matches!(
                    workspace.content_route,
                    crate::app::ContentRoute::Document
                ));
                assert_eq!(workspace.generation, 1);
                assert_eq!(workspace.document_session().unwrap(), &session);
                assert!(session.read(cx).is_dirty());
                assert!(workspace.agenda.state.source_context_menu.is_none());
            });
        });
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        agenda::{AgendaCommand, FileId, TaskRecord, prepare_edit, shard_from_live},
        document::{
            ByteRange, DocumentCommand, DocumentSession, EditOrigin, EditTransaction, Selection,
            TextEdit, TextSnapshot,
        },
        org_semantic::analyze,
        org_syntax,
    };
    use gpui::AppContext;
    use std::{path::PathBuf, sync::Arc};

    fn contents(session: &DocumentSession) -> String {
        let snapshot = session.snapshot();
        snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
    }
    fn task(session: &DocumentSession) -> TaskRecord {
        let snapshot = session.snapshot();
        let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
        shard_from_live(
            FileId(1),
            1,
            Arc::new(session.path().to_path_buf()),
            &analysis,
        )
        .tasks[0]
            .clone()
    }

    #[gpui::test]
    fn metadata_edits_are_revision_safe_and_undoable(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                PathBuf::from("/tmp/m2-live.org"),
                b"#+TODO: TODO NEXT | DONE\n* TODO [#B] Original :old:\nBody\n".to_vec(),
            )
            .unwrap()
        });
        for operation in [
            AgendaCommand::SetTodo(Some(Arc::from("NEXT"))),
            AgendaCommand::SetPriority(Some('A')),
        ] {
            let prepared = cx.read(|cx| {
                prepare_edit(session.read(cx), &task(session.read(cx)), &operation).unwrap()
            });
            session.update(cx, |session, cx| {
                session.edit(prepared.command, cx).unwrap();
            });
        }
        assert_eq!(
            cx.read(|cx| contents(session.read(cx))),
            "#+TODO: TODO NEXT | DONE\n* NEXT [#A] Original :old:\nBody\n"
        );
        session.update(cx, |session, cx| {
            session.undo(cx).unwrap();
        });
        assert_eq!(
            cx.read(|cx| contents(session.read(cx))),
            "#+TODO: TODO NEXT | DONE\n* NEXT [#B] Original :old:\nBody\n"
        );
    }

    #[gpui::test]
    fn locator_maps_across_unrelated_edits(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                PathBuf::from("/tmp/m2-map.org"),
                b"Preamble\n* TODO Target\n".to_vec(),
            )
            .unwrap()
        });
        let target = cx.read(|cx| task(session.read(cx)));
        session.update(cx, |session, cx| {
            session
                .edit(
                    DocumentCommand::new(
                        EditTransaction::new(
                            session.revision(),
                            vec![TextEdit::new(ByteRange::new(0, 0), "new\n")],
                        ),
                        Selection::default(),
                        Selection::default(),
                        EditOrigin::Other,
                    ),
                    cx,
                )
                .unwrap();
        });
        let prepared = cx.read(|cx| {
            prepare_edit(
                session.read(cx),
                &target,
                &AgendaCommand::SetPriority(Some('A')),
            )
            .unwrap()
        });
        session.update(cx, |session, cx| {
            session.edit(prepared.command, cx).unwrap();
        });
        assert!(
            cx.read(|cx| contents(session.read(cx)))
                .contains("* TODO [#A] Target")
        );
    }

    #[gpui::test]
    fn timestamp_edit_resolves_offsets_after_unrelated_insertion(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                PathBuf::from("/tmp/agenda-timestamp-offset.org"),
                b"* TODO Target\nSCHEDULED: <2026-09-07>\n".to_vec(),
            )
            .unwrap()
        });
        let target = cx.read(|cx| task(session.read(cx)));
        session.update(cx, |session, cx| {
            session
                .edit(
                    DocumentCommand::new(
                        EditTransaction::new(
                            session.revision(),
                            vec![TextEdit::new(ByteRange::new(0, 0), "Preamble\n")],
                        ),
                        Selection::default(),
                        Selection::default(),
                        EditOrigin::Other,
                    ),
                    cx,
                )
                .unwrap();
        });
        let mut timestamp = target.timestamps[0].clone();
        timestamp.start_date = "2026-09-08".parse().unwrap();
        let prepared = cx.read(|cx| {
            prepare_edit(
                session.read(cx),
                &target,
                &AgendaCommand::SetTimestamp {
                    target: crate::agenda::TimestampTarget::Scheduled,
                    value: Some(timestamp),
                },
            )
            .unwrap()
        });
        session.update(cx, |session, cx| {
            session.edit(prepared.command, cx).unwrap();
        });
        assert_eq!(
            cx.read(|cx| contents(session.read(cx))),
            "Preamble\n* TODO Target\nSCHEDULED: <2026-09-08>\n"
        );
    }

    #[gpui::test]
    fn id_anchor_recovers_after_the_heading_range_was_replaced(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                PathBuf::from("/tmp/m2-anchor.org"),
                b"* TODO Original\n:PROPERTIES:\n:ID: stable-1\n:END:\n".to_vec(),
            )
            .unwrap()
        });
        let target = cx.read(|cx| task(session.read(cx)));
        assert_eq!(
            target
                .source
                .anchor
                .as_ref()
                .map(|anchor| anchor.value.as_ref()),
            Some("stable-1")
        );
        session.update(cx, |session, cx| {
            session
                .edit(
                    DocumentCommand::new(
                        EditTransaction::new(
                            session.revision(),
                            vec![TextEdit::new(target.source.title_range, "TODO Changed")],
                        ),
                        Selection::default(),
                        Selection::default(),
                        EditOrigin::Other,
                    ),
                    cx,
                )
                .unwrap();
        });
        let prepared = cx.read(|cx| {
            prepare_edit(
                session.read(cx),
                &target,
                &AgendaCommand::SetPriority(Some('A')),
            )
            .unwrap()
        });
        session.update(cx, |session, cx| {
            session.edit(prepared.command, cx).unwrap();
        });
        assert!(
            cx.read(|cx| contents(session.read(cx)))
                .contains("[#A] Changed")
        );
    }

    #[gpui::test]
    fn timestamp_clear_and_delete_are_undoable(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            DocumentSession::from_utf8(
                PathBuf::from("/tmp/m2-time.org"),
                b"* TODO Timed\nSCHEDULED: <2026-09-05 Sat>\n".to_vec(),
            )
            .unwrap()
        });
        let target = cx.read(|cx| task(session.read(cx)));
        let prepared = cx.read(|cx| {
            prepare_edit(
                session.read(cx),
                &target,
                &AgendaCommand::SetTimestamp {
                    target: crate::agenda::TimestampTarget::Scheduled,
                    value: None,
                },
            )
            .unwrap()
        });
        session.update(cx, |session, cx| {
            session.edit(prepared.command, cx).unwrap();
        });
        assert!(
            !cx.read(|cx| contents(session.read(cx)))
                .contains("2026-09-05")
        );
        let current = cx.read(|cx| task(session.read(cx)));
        let prepared = cx.read(|cx| {
            prepare_edit(session.read(cx), &current, &AgendaCommand::DeleteSubtree).unwrap()
        });
        session.update(cx, |session, cx| {
            session.edit(prepared.command, cx).unwrap();
            session.undo(cx).unwrap();
        });
        assert!(
            cx.read(|cx| contents(session.read(cx)))
                .contains("* TODO Timed")
        );
    }
}
