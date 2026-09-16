use crate::app::WorkspaceWindow;
use gpui::{
    CursorStyle, Entity, MouseButton, ParentElement, Styled, Window, div, prelude::*, px, rgb,
};
use std::{collections::BTreeMap, time::Instant};

fn stacked_height(item_count: usize, item_height: f32, gap: f32) -> f32 {
    item_count as f32 * item_height + item_count.saturating_sub(1) as f32 * gap
}

impl super::AgendaHost {
    pub(crate) fn render(
        &self,
        workspace: Entity<WorkspaceWindow>,
        viewport_width: f32,
        window_title: String,
        window: &Window,
        motion_enabled: bool,
        titlebar_inset: f32,
    ) -> gpui::Div {
        let language = self.language;
        let theme = crate::theme::current_theme();
        let full_sidebar_width = super::host::expanded_sidebar_width(
            viewport_width,
            self.sidebar_resize
                .map(|resize| resize.current_width)
                .unwrap_or(self.sidebar_width),
        );
        let now = Instant::now();
        let (sidebar_reveal, sidebar_animating) = self.sidebar_reveal_at(now);
        let (smart_views_reveal, smart_views_animating) =
            self.sidebar_section_reveal_at(super::state::SidebarSection::SmartViews, now);
        let (saved_views_reveal, saved_views_animating) =
            self.sidebar_section_reveal_at(super::state::SidebarSection::SavedViews, now);
        let (tags_reveal, tags_animating) =
            self.sidebar_section_reveal_at(super::state::SidebarSection::Tags, now);
        let (sources_reveal, sources_animating) =
            self.sidebar_section_reveal_at(super::state::SidebarSection::Sources, now);
        if sidebar_animating
            || smart_views_animating
            || saved_views_animating
            || tags_animating
            || sources_animating
        {
            // GPUI resolves the current view only while rendering. Scheduling this from the
            // scroll-wheel callback panics because that callback runs outside view rendering.
            window.request_animation_frame();
        }
        let sidebar_width = full_sidebar_width * sidebar_reveal;
        let resize_handle_width = super::style::SIDEBAR_RESIZE_HANDLE_WIDTH * sidebar_reveal;
        let facets = self
            .page_query
            .result
            .as_ref()
            .map(|r| r.facets.clone())
            .unwrap_or_default();
        let mut tag_counts = BTreeMap::<String, usize>::new();
        let mut source_counts = BTreeMap::<(crate::agenda::FileId, String), usize>::new();
        if let Some(result) = self.navigation_result.as_ref() {
            for entry in result.entries.iter() {
                let row = &entry.row;
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
        let saved_view_names = self
            .config
            .saved_views
            .iter()
            .map(|view| view.name.clone())
            .collect::<Vec<_>>();
        let smart_views_height = stacked_height(
            6,
            super::style::SIDEBAR_ITEM_HEIGHT,
            super::style::SIDEBAR_ITEM_GAP,
        );
        let saved_views_height = if saved_view_names.is_empty() {
            super::style::SIDEBAR_SAVED_VIEW_HEIGHT
        } else {
            stacked_height(
                saved_view_names.len(),
                super::style::SIDEBAR_SAVED_VIEW_HEIGHT,
                super::style::SIDEBAR_ITEM_GAP,
            )
        };
        let tags_height = stacked_height(tag_counts.len(), super::style::SIDEBAR_TAG_HEIGHT, 0.);
        let sources_height =
            stacked_height(source_counts.len(), super::style::SIDEBAR_SOURCE_HEIGHT, 0.)
                + super::style::SIDEBAR_SOURCE_BOTTOM_PADDING;
        let sidebar_content = div()
            .w_full()
            .flex_none()
            .pt(px(16.0))
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
                                super::component::StaticSidebarItem {
                                    language,
                                    workspace: workspace.clone(),
                                    label: "收件箱",
                                    icon: "assets/icons/agenda/tray.svg",
                                    count: Some(self.inbox_tasks().len()),
                                    selected: self.state.navigation.as_ref() == "收件箱",
                                    compact: false,
                                    query: crate::agenda::BuiltinQuery::Unscheduled,
                                },
                            ))
                            .child(
                                div()
                                    .id("agenda-open-capture")
                                    .absolute()
                                    .right(px(42.))
                                    .top(px(5.))
                                    .size(px(29.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(6.))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(gpui::rgb(theme.hover)))
                                    .child(
                                        gpui::svg()
                                            .data(super::icon::agenda_icon(
                                                "assets/icons/agenda/plus.svg",
                                            ))
                                            .size(px(14.))
                                            .text_color(rgb(theme.foreground_dim)),
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
                            ),
                    )
                    .child(super::component::static_sidebar_item(
                        super::component::StaticSidebarItem {
                            language,
                            workspace: workspace.clone(),
                            label: "Agenda",
                            icon: "assets/icons/agenda/calendar-dots.svg",
                            count: None,
                            selected: self.state.navigation.as_ref() == "Agenda",
                            compact: false,
                            query: crate::agenda::BuiltinQuery::NextSevenDays,
                        },
                    ))
                    .child(super::component::static_sidebar_item(
                        super::component::StaticSidebarItem {
                            language,
                            workspace: workspace.clone(),
                            label: "Tasks",
                            icon: "assets/icons/agenda/check-circle.svg",
                            count: None,
                            selected: self.state.navigation.as_ref() == "Tasks",
                            compact: false,
                            query: crate::agenda::BuiltinQuery::Next,
                        },
                    ))
                    .child(super::component::static_sidebar_item(
                        super::component::StaticSidebarItem {
                            language,
                            workspace: workspace.clone(),
                            label: "Projects",
                            icon: "assets/icons/agenda/tree-structure.svg",
                            count: Some(self.projects().len()),
                            selected: self.state.navigation.as_ref() == "Projects",
                            compact: false,
                            query: crate::agenda::BuiltinQuery::Next,
                        },
                    )),
            )
            .child(
                super::component::sidebar_section_header(
                    workspace.clone(),
                    language.text("agenda.smart_views"),
                    super::state::SidebarSection::SmartViews,
                    smart_views_reveal,
                )
                .mt_3(),
            )
            .child(super::component::sidebar_section_body(
                "smart-views",
                smart_views_reveal,
                smart_views_height,
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
                            false,
                        ))
                    },
                ),
            ))
            .child(
                super::component::sidebar_section_header(
                    workspace.clone(),
                    language.text("agenda.saved_views"),
                    super::state::SidebarSection::SavedViews,
                    saved_views_reveal,
                )
                .mt_4(),
            )
            .child(super::component::sidebar_section_body(
                "saved-views",
                saved_views_reveal,
                saved_views_height,
                div()
                    .flex_none()
                    .px_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(saved_view_names.is_empty(), |list| {
                        list.child(
                            div()
                                .px_3()
                                .py_2()
                                .text_size(px(12.))
                                .text_color(rgb(theme.foreground_muted))
                                .child(language.text("agenda.no_saved_views")),
                        )
                    })
                    .children(
                        saved_view_names
                            .into_iter()
                            .enumerate()
                            .map(|(index, name)| {
                                let selected = self.state.navigation.as_ref() == name;
                                super::component::saved_view_item(
                                    workspace.clone(),
                                    name,
                                    index,
                                    selected,
                                )
                            }),
                    ),
            ))
            .child(
                super::component::sidebar_section_header(
                    workspace.clone(),
                    language.text("agenda.tags"),
                    super::state::SidebarSection::Tags,
                    tags_reveal,
                )
                .mt_4(),
            )
            .child(super::component::sidebar_section_body(
                "tags",
                tags_reveal,
                tags_height,
                div()
                    .flex_none()
                    .px_3()
                    .flex()
                    .flex_col()
                    .children(tag_counts.iter().map(|(tag, count)| {
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
                    })),
            ))
            .child(
                super::component::sidebar_section_header(
                    workspace.clone(),
                    language.text("agenda.source_files"),
                    super::state::SidebarSection::Sources,
                    sources_reveal,
                )
                .mt_4(),
            )
            .child(super::component::sidebar_section_body(
                "sources",
                sources_reveal,
                sources_height,
                div().flex_none().px_3().pb_4().flex().flex_col().children(
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
                ),
            ));
        let sidebar_scroll_area = div()
            .id("agenda-sidebar-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&self.sidebar_scroll)
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(sidebar_content);
        let sidebar_panel = div()
            .relative()
            .left(px(-(1.0 - sidebar_reveal) * full_sidebar_width))
            .w(px(full_sidebar_width))
            .h_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(rgb(super::style::SIDEBAR()))
            .child(div().h(px(crate::app::TITLEBAR_HEIGHT)).flex_none())
            .child(sidebar_scroll_area);
        let sidebar = div()
            .flex_none()
            .w(px(sidebar_width))
            .h_full()
            .min_h_0()
            .overflow_hidden()
            .child(sidebar_panel);

        let inspector = (self.state.sheet == super::state::AgendaSheet::Inspector)
            .then(|| {
                self.state
                    .selected
                    .and_then(|index| self.page_query.result.as_ref()?.placement_entry(index))
                    .map(|(_, entry)| &entry.row)
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
        let content = self.page_query.result.clone();

        use super::component::TaskColumns;
        let list_width = viewport_width
            - sidebar_width
            - resize_handle_width
            - if inspector.is_some() { 360. } else { 0. };
        let task_columns = TaskColumns::for_width(
            list_width,
            self.page_query.result.as_ref().is_some_and(|result| {
                result
                    .entries
                    .iter()
                    .any(|entry| !entry.row.tags.is_empty())
            }),
        );
        let content_gutter = TaskColumns::GUTTER;
        let columns = TaskColumns::row()
            .mx(px(content_gutter + 1.))
            .h(px(super::style::COLUMN_HEADER_HEIGHT))
            .text_color(rgb(super::style::MUTED()))
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
        let toolbar = super::component::agenda_toolbar(super::component::AgendaToolbarProps {
            workspace: workspace.clone(),
            state: &self.state,
            language,
            window_width: viewport_width,
            main_content_offset: sidebar_width + resize_handle_width,
            window_title,
            search: self.search_input.clone(),
            motion_enabled,
            titlebar_inset,
        });
        let gesture_workspace = workspace.clone();
        div()
            .size_full()
            .relative()
            .flex()
            .font_family("SF Pro Text")
            .text_color(rgb(super::style::INK()))
            .on_scroll_wheel(move |event, _, cx| {
                if !event.delta.precise() {
                    return;
                }
                let delta = event.delta.pixel_delta(px(16.0));
                let outcome = gesture_workspace.update(cx, |this, cx| {
                    let outcome = this.agenda.handle_sidebar_swipe(
                        f32::from(delta.x),
                        f32::from(delta.y),
                        event.touch_phase,
                        Instant::now(),
                        motion_enabled,
                    );
                    if outcome.visibility_changed {
                        cx.notify();
                    }
                    outcome
                });
                if outcome.consumed {
                    cx.stop_propagation();
                }
            })
            .when(self.state.source_context_menu.is_some(), |root| {
                let workspace = workspace.clone();
                root.on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                    workspace.update(cx, |this, cx| {
                        this.dispatch_agenda_intent(super::UiIntent::CloseSourceContextMenu, cx)
                    });
                })
            })
            .child(sidebar)
            .child({
                let resize_workspace = workspace.clone();
                div()
                    .id("agenda-sidebar-resize-handle")
                    .w(px(resize_handle_width))
                    .h_full()
                    .flex_none()
                    .flex()
                    .justify_center()
                    .bg(rgb(super::style::SIDEBAR()))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                        cx.stop_propagation();
                        resize_workspace.update(cx, |this, cx| {
                            if event.click_count >= 2 {
                                this.agenda.sidebar_width = super::style::SIDEBAR_WIDTH;
                                this.agenda.sidebar_visible = true;
                                this.agenda.sidebar_resize = None;
                                this.agenda.sidebar_visibility_animation = None;
                            } else {
                                this.agenda.sidebar_visible = true;
                                this.agenda.sidebar_visibility_animation = None;
                                this.agenda.sidebar_resize =
                                    Some(super::host::SidebarResizeSession {
                                        start_pointer_x: f32::from(event.position.x),
                                        start_width: full_sidebar_width,
                                        current_width: full_sidebar_width,
                                    });
                            }
                            cx.notify();
                        });
                    })
            })
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
                                                .px(px(content_gutter))
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
                                                Some(_),
                                            ) => div()
                                                .flex_1()
                                                .min_h_0()
                                                .overflow_hidden()
                                                .children(self.text_editor.clone())
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
            .child(
                div()
                    .absolute()
                    .top(px(crate::app::TITLEBAR_HEIGHT))
                    .left_0()
                    .right_0()
                    .h(px(1.))
                    .bg(rgb(super::style::BORDER())),
            )
            .when_some(
                {
                    let targets = self
                        .state
                        .refile_task
                        .map(|task| {
                            crate::agenda::refile_targets(
                                &self.runtime.index.snapshot(),
                                task,
                                &self.state.refile_search,
                                &self.state.recent_refile_targets,
                            )
                        })
                        .unwrap_or_default();
                    super::view::workflow_overlay(super::view::WorkflowOverlay {
                        language,
                        kind: self.state.overlay,
                        workspace: workspace.clone(),
                        draft: &self.state.capture,
                        targets,
                        selected: self.state.refile_selected,
                        search: &self.state.refile_search,
                        message: self.state.workflow_message.as_deref(),
                    })
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
                        .bg(rgb(theme.hover))
                        .text_color(rgb(theme.warning))
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
            .when(self.sidebar_resize.is_some(), |root| {
                let move_workspace = workspace.clone();
                let finish_workspace = workspace.clone();
                root.child(
                    div()
                        .id("agenda-sidebar-resize-overlay")
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .left_0()
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_mouse_move(move |event, _, cx| {
                            if !event.dragging() {
                                return;
                            }
                            move_workspace.update(cx, |this, cx| {
                                let Some(session) = this.agenda.sidebar_resize else {
                                    return;
                                };
                                let width = super::host::resized_sidebar_width(
                                    viewport_width,
                                    session,
                                    f32::from(event.position.x),
                                );
                                if session.current_width != width {
                                    this.agenda.sidebar_resize =
                                        Some(super::host::SidebarResizeSession {
                                            current_width: width,
                                            ..session
                                        });
                                    cx.notify();
                                }
                            });
                        })
                        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                            finish_workspace.update(cx, |this, cx| {
                                if let Some(session) = this.agenda.sidebar_resize.take() {
                                    this.agenda.sidebar_width = session.current_width;
                                    cx.notify();
                                }
                            });
                        }),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::stacked_height;

    #[test]
    fn stacked_height_accounts_for_rows_and_inter_row_gaps() {
        assert_eq!(stacked_height(0, 39., 4.), 0.);
        assert_eq!(stacked_height(1, 39., 4.), 39.);
        assert_eq!(stacked_height(6, 39., 4.), 254.);
    }
}
