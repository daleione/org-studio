use crate::app::WorkspaceWindow;
use gpui::{Entity, ParentElement, Styled, div, prelude::*, px, rgb};
use std::collections::BTreeMap;

impl super::AgendaHost {
    pub(crate) fn render(
        &self,
        workspace: Entity<WorkspaceWindow>,
        viewport_width: f32,
    ) -> gpui::Div {
        let language = self.language;
        let layout = super::layout::AgendaLayout::for_width(viewport_width);
        let compact = matches!(layout, super::layout::AgendaLayout::Compact);
        let facets = self
            .result
            .as_ref()
            .map(|r| r.facets.clone())
            .unwrap_or_default();
        let mut tag_counts = BTreeMap::<String, usize>::new();
        let mut source_counts = BTreeMap::<(crate::agenda::FileId, String), usize>::new();
        if let Some(result) = self.navigation_result.as_ref() {
            for row in result.rows.iter() {
                for tag in row.tags.iter() {
                    *tag_counts.entry(tag.to_string()).or_default() += 1;
                }
                let name = row
                    .source
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("—")
                    .to_owned();
                *source_counts.entry((row.source.file, name)).or_default() += 1;
            }
        }
        let sidebar_content = div()
            .w_full()
            .flex_none()
            .pt_4()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .relative()
                            .child(super::component::static_sidebar_item(
                                language,
                                workspace.clone(),
                                "收件箱",
                                "assets/icons/agenda/tray.svg",
                                Some(self.inbox_tasks().len()),
                                self.state.navigation.as_ref() == "收件箱",
                                compact,
                                crate::agenda::BuiltinQuery::Unscheduled,
                            ))
                            .when(!compact, |row| {
                                row.child(
                                    div()
                                        .absolute()
                                        .right(px(42.))
                                        .top(px(5.))
                                        .size(px(29.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(6.))
                                        .cursor_pointer()
                                        .child(
                                            gpui::svg()
                                                .data(super::icon::agenda_icon(
                                                    "assets/icons/agenda/plus.svg",
                                                ))
                                                .size(px(14.))
                                                .text_color(rgb(0x555960)),
                                        )
                                        .on_mouse_down(gpui::MouseButton::Left, {
                                            let target = workspace.clone();
                                            move |_, _, cx| {
                                                target.update(cx, |this, cx| {
                                                    this.dispatch_agenda_intent(
                                                        super::UiIntent::OpenCapture,
                                                        cx,
                                                    )
                                                });
                                            }
                                        }),
                                )
                            }),
                    )
                    .child(super::component::static_sidebar_item(
                        language,
                        workspace.clone(),
                        "Agenda",
                        "assets/icons/agenda/calendar-dots.svg",
                        None,
                        self.state.navigation.as_ref() == "Agenda",
                        compact,
                        crate::agenda::BuiltinQuery::NextSevenDays,
                    ))
                    .child(super::component::static_sidebar_item(
                        language,
                        workspace.clone(),
                        "Tasks",
                        "assets/icons/agenda/check-circle.svg",
                        None,
                        self.state.navigation.as_ref() == "Tasks",
                        compact,
                        crate::agenda::BuiltinQuery::Next,
                    ))
                    .child(super::component::static_sidebar_item(
                        language,
                        workspace.clone(),
                        "Projects",
                        "assets/icons/agenda/tree-structure.svg",
                        Some(self.projects().len()),
                        self.state.navigation.as_ref() == "Projects",
                        compact,
                        crate::agenda::BuiltinQuery::Next,
                    )),
            )
            .when(!compact, |side| {
                side.child(
                    super::component::sidebar_section_header(
                        workspace.clone(),
                        language.text("agenda.smart_views"),
                        super::state::SidebarSection::SmartViews,
                        self.state.smart_views_expanded,
                    )
                    .mt_3(),
                )
            })
            .when(compact || self.state.smart_views_expanded, |side| {
                side.child(
                    [
                        (
                            crate::agenda::BuiltinQuery::Today,
                            language.text("agenda.today"),
                            "assets/icons/agenda/phosphor-calendar.svg",
                            facets.today,
                        ),
                        (
                            crate::agenda::BuiltinQuery::NextSevenDays,
                            language.text("agenda.next_seven"),
                            "assets/icons/agenda/calendar.svg",
                            facets.next_seven_days,
                        ),
                        (
                            crate::agenda::BuiltinQuery::Overdue,
                            language.text("agenda.overdue"),
                            "assets/icons/agenda/clock.svg",
                            facets.overdue,
                        ),
                        (
                            crate::agenda::BuiltinQuery::Next,
                            language.text("agenda.next"),
                            "assets/icons/agenda/arrow-circle-right.svg",
                            facets.next,
                        ),
                        (
                            crate::agenda::BuiltinQuery::Waiting,
                            language.text("agenda.waiting"),
                            "assets/icons/agenda/hourglass.svg",
                            facets.waiting,
                        ),
                        (
                            crate::agenda::BuiltinQuery::Unscheduled,
                            language.text("agenda.unscheduled"),
                            "assets/icons/agenda/calendar-slash.svg",
                            facets.unscheduled,
                        ),
                    ]
                    .into_iter()
                    .fold(
                        div().flex_none().px_3().flex().flex_col().gap_1(),
                        |list, (query, label, icon, count)| {
                            list.child(super::component::sidebar_item(
                                workspace.clone(),
                                query,
                                label,
                                icon,
                                count,
                                self.state.navigation.as_ref() == label,
                                compact,
                            ))
                        },
                    ),
                )
            })
            .when(!compact, |side| {
                side.child(
                    super::component::sidebar_section_header(
                        workspace.clone(),
                        language.text("agenda.saved_views"),
                        super::state::SidebarSection::SavedViews,
                        self.state.saved_views_expanded,
                    )
                    .mt_4(),
                )
            })
            .when(!compact && self.state.saved_views_expanded, |side| {
                let names = self
                    .config
                    .saved_views
                    .iter()
                    .map(|view| view.name.clone())
                    .collect::<Vec<_>>();
                side.child(
                    div()
                        .flex_none()
                        .px_3()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .when(names.is_empty(), |list| {
                            list.child(
                                div()
                                    .px_3()
                                    .py_2()
                                    .text_size(px(12.))
                                    .text_color(rgb(0x9a9da3))
                                    .child(language.text("agenda.no_saved_views")),
                            )
                        })
                        .children(names.into_iter().enumerate().map(|(index, name)| {
                            let selected = self.state.navigation.as_ref() == name;
                            super::component::saved_view_item(
                                workspace.clone(),
                                name,
                                index,
                                selected,
                            )
                        })),
                )
            })
            .when(!compact, |side| {
                side.child(
                    super::component::sidebar_section_header(
                        workspace.clone(),
                        language.text("agenda.tags"),
                        super::state::SidebarSection::Tags,
                        self.state.tags_expanded,
                    )
                    .mt_4(),
                )
            })
            .when(!compact && self.state.tags_expanded, |side| {
                side.child(div().flex_none().px_3().flex().flex_col().children(
                    tag_counts.iter().map(|(tag, count)| {
                        let selected = self.state.tag_filter.as_deref() == Some(tag.as_str());
                        super::component::sidebar_filter_item(
                            workspace.clone(),
                            tag.clone(),
                            *count,
                            (!selected).then(|| tag.clone().into()),
                            None,
                            selected,
                            false,
                        )
                    }),
                ))
            })
            .when(!compact, |side| {
                side.child(
                    super::component::sidebar_section_header(
                        workspace.clone(),
                        language.text("agenda.source_files"),
                        super::state::SidebarSection::Sources,
                        self.state.sources_expanded,
                    )
                    .mt_4(),
                )
            })
            .when(!compact && self.state.sources_expanded, |side| {
                side.child(div().flex_none().px_3().pb_4().flex().flex_col().children(
                    source_counts.iter().map(|((file, name), count)| {
                        let selected = self.state.source_filter == Some(*file);
                        super::component::sidebar_filter_item(
                            workspace.clone(),
                            name.clone(),
                            *count,
                            None,
                            Some(*file),
                            selected,
                            true,
                        )
                    }),
                ))
            });
        let sidebar = div()
            .flex_none()
            .w(px(if compact {
                70.
            } else {
                super::style::SIDEBAR_WIDTH
            }))
            .h_full()
            .min_h_0()
            .id("agenda-sidebar-scroll")
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&self.sidebar_scroll)
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .bg(rgb(super::style::SIDEBAR))
            .border_r_1()
            .border_color(rgb(super::style::BORDER))
            .child(sidebar_content);

        let inspector = (self.state.sheet == super::state::AgendaSheet::Inspector)
            .then(|| {
                self.state
                    .selected
                    .and_then(|index| self.result.as_ref()?.rows.get(index))
            })
            .flatten()
            .map(|row| {
                let task_record = self.task(row.task);
                super::view::agenda_inspector(super::view::InspectorProps {
                    language,
                    workspace: workspace.clone(),
                    task: row.task,
                    title: row.title.clone(),
                    todo: row.todo.clone(),
                    priority: row.priority,
                    tags: row.tags.clone(),
                    allowed_todo_states: self
                        .task(row.task)
                        .map(|task| task.allowed_todo_states)
                        .unwrap_or_default(),
                    pending: self
                        .state
                        .inspector
                        .as_ref()
                        .is_some_and(|state| state.pending),
                    error: self
                        .state
                        .inspector
                        .as_ref()
                        .and_then(|state| state.error.clone()),
                    confirm_delete: self
                        .state
                        .inspector
                        .as_ref()
                        .is_some_and(|state| state.confirm_delete),
                    scroll_handle: self.inspector_scroll.clone(),
                    clock_active: task_record
                        .as_ref()
                        .is_some_and(|task| self.clock_is_active_for(task)),
                    habit: task_record.as_ref().and_then(|task| {
                        let is_habit = task
                            .effective_tags
                            .iter()
                            .any(|tag| tag.eq_ignore_ascii_case("habit"))
                            || task.properties.iter().any(|(key, value)| {
                                key.eq_ignore_ascii_case("STYLE")
                                    && value.eq_ignore_ascii_case("habit")
                            });
                        is_habit.then(|| {
                            crate::agenda::habit_stats(
                                jiff::Zoned::now().date(),
                                task.timestamps
                                    .iter()
                                    .filter(|timestamp| !timestamp.active)
                                    .map(|timestamp| timestamp.start_date),
                            )
                        })
                    }),
                })
            });
        let content = self.result.clone();

        use super::component::TaskColumns;
        let list_width = viewport_width
            - if compact {
                70.
            } else {
                super::style::SIDEBAR_WIDTH
            }
            - if inspector.is_some() { 360. } else { 0. };
        let task_columns = TaskColumns::for_width(
            list_width,
            self.result
                .as_ref()
                .is_some_and(|result| result.rows.iter().any(|row| !row.tags.is_empty())),
        );
        let columns = TaskColumns::row()
            .mx(px(TaskColumns::GUTTER + 1.))
            .h(px(super::style::COLUMN_HEADER_HEIGHT))
            .text_color(rgb(super::style::MUTED))
            .text_size(px(11.))
            .child(TaskColumns::cell(TaskColumns::CHECK))
            .when(task_columns.source, |h| {
                h.child(
                    TaskColumns::cell(TaskColumns::SOURCE).child(language.text("agenda.source")),
                )
            })
            .child(TaskColumns::cell(TaskColumns::TIME).child(language.text("agenda.time")))
            .when(task_columns.plan, |h| {
                h.child(
                    TaskColumns::cell(TaskColumns::plan_width(language))
                        .child(language.text("agenda.scheduled")),
                )
            })
            .child(
                TaskColumns::cell(TaskColumns::STATUS)
                    .text_center()
                    .child(language.text("agenda.status")),
            )
            .when(task_columns.priority, |h| {
                h.child(
                    TaskColumns::cell(TaskColumns::PRIORITY)
                        .text_center()
                        .child(language.text("agenda.priority_short")),
                )
            })
            .child(div().flex_1().min_w_0().child(language.text("agenda.task")))
            .when(task_columns.tags, |h| {
                h.child(TaskColumns::cell(TaskColumns::TAGS).child(language.text("agenda.tags")))
            });
        let toolbar = super::component::agenda_toolbar(
            workspace.clone(),
            &self.state,
            language,
            viewport_width
                - if compact {
                    70.
                } else {
                    super::style::SIDEBAR_WIDTH
                },
            self.search_input.clone(),
        );
        div()
            .size_full()
            .relative()
            .flex()
            .font_family("SF Pro Text")
            .text_color(rgb(super::style::INK))
            .when(self.state.source_context_menu.is_some(), |root| {
                let workspace = workspace.clone();
                root.on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                    workspace.update(cx, |this, cx| {
                        this.dispatch_agenda_intent(super::UiIntent::CloseSourceContextMenu, cx)
                    });
                })
            })
            .child(sidebar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(toolbar)
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .when(
                                        self.state.projection
                                            == super::state::AgendaProjection::List
                                            && matches!(
                                                self.state.workspace,
                                                super::state::AgendaWorkspace::Agenda
                                                    | super::state::AgendaWorkspace::Tasks
                                            ),
                                        |pane| pane.child(columns),
                                    )
                                    .child(
                                        match (self.state.workspace, self.state.projection, content)
                                        {
                                            (super::state::AgendaWorkspace::Inbox, _, _) => {
                                                super::view::inbox_view(
                                                    language,
                                                    workspace.clone(),
                                                    self.inbox_tasks(),
                                                    self.state.inbox_session.as_ref(),
                                                )
                                                .into_any_element()
                                            }
                                            (super::state::AgendaWorkspace::Projects, _, _) => {
                                                super::view::projects_view(
                                                    language,
                                                    workspace.clone(),
                                                    self.projects(),
                                                    self.state.selected_project,
                                                )
                                                .into_any_element()
                                            }
                                            (
                                                _,
                                                super::state::AgendaProjection::Calendar,
                                                Some(result),
                                            ) => super::view::agenda_calendar(
                                                language,
                                                workspace.clone(),
                                                result,
                                                self.state.calendar_range,
                                                self.state
                                                    .calendar_window(jiff::Zoned::now().date()),
                                                self.state.all_day_expanded,
                                            )
                                            .into_any_element(),
                                            (
                                                _,
                                                super::state::AgendaProjection::List,
                                                Some(result),
                                            ) => div()
                                                .flex_1()
                                                .min_h_0()
                                                .px(px(TaskColumns::GUTTER))
                                                .child(super::view::agenda_list(
                                                    language,
                                                    workspace.clone(),
                                                    result,
                                                    self.list_state.clone(),
                                                    self.state.selected,
                                                    self.state.collapsed_days.clone(),
                                                    task_columns,
                                                ))
                                                .into_any_element(),
                                            (
                                                _,
                                                super::state::AgendaProjection::Source,
                                                Some(result),
                                            ) => div()
                                                .flex_1()
                                                .min_h_0()
                                                .id("agenda-source-scroll")
                                                .overflow_y_scroll()
                                                .child(super::view::agenda_text(&result))
                                                .into_any_element(),
                                            (_, _, None) => super::component::empty_state(
                                                language.text("agenda.configure"),
                                            )
                                            .into_any_element(),
                                        },
                                    ),
                            )
                            .when_some(inspector, |body, inspector| {
                                body.child(inspector.flex_none())
                            }),
                    ),
            )
            .when_some(
                {
                    let targets = self
                        .state
                        .refile_task
                        .map(|task| {
                            crate::agenda::refile_targets(
                                &self.index.snapshot(),
                                task,
                                &self.state.refile_search,
                                &self.state.recent_refile_targets,
                            )
                        })
                        .unwrap_or_default();
                    super::view::workflow_overlay(
                        language,
                        self.state.overlay,
                        workspace.clone(),
                        &self.state.capture,
                        targets,
                        self.state.refile_selected,
                        &self.state.refile_search,
                        self.state.workflow_message.as_deref(),
                    )
                },
                |root, overlay| root.child(overlay),
            )
            .when(self.recovery_receipt.is_some(), |root| {
                root.child(
                    div()
                        .absolute()
                        .top(px(96.))
                        .right(px(22.))
                        .p_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(px(8.))
                        .bg(rgb(0xfff5e9))
                        .text_color(rgb(0x8e5a16))
                        .shadow_md()
                        .text_size(px(10.))
                        .child(language.text("agenda.recovery"))
                        .child(super::component::text_action_button(
                            workspace.clone(),
                            language.text("agenda.resume_move"),
                            super::UiIntent::ResumeRecovery,
                        ))
                        .child(super::component::text_action_button(
                            workspace.clone(),
                            language.text("agenda.keep_source"),
                            super::UiIntent::CleanupRecovery,
                        )),
                )
            })
            .when_some(self.state.source_context_menu, |root, menu| {
                root.child(super::component::source_context_menu(
                    language,
                    workspace.clone(),
                    menu,
                ))
            })
    }
}
