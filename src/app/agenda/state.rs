use crate::agenda::{BuiltinQuery, FileId};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum AgendaWorkspace {
    Inbox,
    #[default]
    Agenda,
    Tasks,
    Projects,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgendaProjection {
    List,
    Calendar,
    Source,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CalendarRange {
    Day,
    Week,
    Month,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgendaOverlay {
    None,
    Capture,
    Refile,
    Repeat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgendaSheet {
    None,
    Inspector,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SidebarSection {
    SmartViews,
    SavedViews,
    Tags,
    Sources,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SourceContextMenu {
    pub(crate) file: FileId,
    pub(crate) position: gpui::Point<gpui::Pixels>,
}

pub(crate) struct AgendaViewState {
    pub(crate) search_expanded: bool,
    pub(crate) search_focus_pending: bool,
    pub(crate) date_picker: bool,
    pub(crate) picker_offset: i32,
    pub(crate) workspace: AgendaWorkspace,
    pub(crate) builtin: BuiltinQuery,
    pub(crate) navigation: Arc<str>,
    pub(crate) selected: Option<usize>,
    pub(crate) overlay: AgendaOverlay,
    pub(crate) sheet: AgendaSheet,
    pub(crate) inspector: Option<InspectorState>,
    pub(crate) search: Arc<str>,
    pub(crate) tag_filter: Option<Arc<str>>,
    pub(crate) source_filter: Option<FileId>,
    pub(crate) structured_todo: Option<Arc<str>>,
    pub(crate) structured_scheduled: Option<bool>,
    pub(crate) projection: AgendaProjection,
    pub(crate) calendar_range: CalendarRange,
    pub(crate) calendar_offset: i32,
    pub(crate) calendar_anchor: Option<jiff::civil::Date>,
    pub(crate) all_day_expanded: bool,
    pub(crate) collapsed_days: std::collections::BTreeSet<Option<jiff::civil::Date>>,
    pub(crate) smart_views_expanded: bool,
    pub(crate) saved_views_expanded: bool,
    pub(crate) tags_expanded: bool,
    pub(crate) sources_expanded: bool,
    pub(crate) source_context_menu: Option<SourceContextMenu>,
    pub(crate) capture: crate::agenda::CaptureDraft,
    pub(crate) inbox_session: Option<crate::agenda::InboxSession>,
    pub(crate) selected_project: usize,
    pub(crate) refile_task: Option<crate::agenda::TaskKey>,
    pub(crate) refile_search: String,
    pub(crate) refile_selected: usize,
    pub(crate) recent_refile_targets: Vec<crate::agenda::TaskKey>,
    pub(crate) workflow_message: Option<Arc<str>>,
    pub(crate) repeat_task: Option<crate::agenda::TaskKey>,
    pub(crate) repeat_target: Option<Arc<str>>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InspectorState {
    pub(crate) pending: bool,
    pub(crate) error: Option<std::sync::Arc<str>>,
    pub(crate) confirm_delete: bool,
}

impl AgendaViewState {
    pub(crate) fn clear_task_selection(&mut self) {
        self.selected = None;
        self.inspector = None;
        self.sheet = AgendaSheet::None;
    }

    pub(crate) fn browses_dates(&self) -> bool {
        self.workspace == AgendaWorkspace::Agenda && self.navigation.as_ref() == "Agenda"
    }

    pub(crate) fn period_anchor(&self, today: jiff::civil::Date) -> jiff::civil::Date {
        let anchor = self.calendar_anchor.unwrap_or(today);
        let offset = i64::from(self.calendar_offset);
        let span = match self.calendar_range {
            CalendarRange::Day => jiff::Span::new().days(offset),
            CalendarRange::Week => jiff::Span::new().weeks(offset),
            CalendarRange::Month => jiff::Span::new().months(offset),
        };
        if self.calendar_range == CalendarRange::Month {
            let month = anchor
                .first_of_month()
                .checked_add(span)
                .unwrap_or(anchor.first_of_month());
            return month
                .checked_add(
                    jiff::Span::new().days(i64::from(anchor.day().min(month.days_in_month()) - 1)),
                )
                .unwrap_or(month);
        }
        anchor.checked_add(span).unwrap_or(anchor)
    }

    pub(crate) fn calendar_window(
        &self,
        today: jiff::civil::Date,
    ) -> (jiff::civil::Date, jiff::civil::Date) {
        use jiff::Span;
        let today = self.calendar_anchor.unwrap_or(today);
        let (start, days) = match self.calendar_range {
            CalendarRange::Day => (
                today
                    .checked_add(Span::new().days(i64::from(self.calendar_offset)))
                    .unwrap_or(today),
                1,
            ),
            CalendarRange::Week => {
                let monday = today
                    .checked_sub(
                        Span::new().days(i64::from(today.weekday().to_monday_zero_offset())),
                    )
                    .unwrap_or(today);
                (
                    monday
                        .checked_add(Span::new().weeks(i64::from(self.calendar_offset)))
                        .unwrap_or(monday),
                    7,
                )
            }
            CalendarRange::Month => {
                let month = today
                    .first_of_month()
                    .checked_add(Span::new().months(i64::from(self.calendar_offset)))
                    .unwrap_or(today.first_of_month());
                let offset = i64::from(month.weekday().to_monday_zero_offset());
                let start = month.checked_sub(Span::new().days(offset)).unwrap_or(month);
                let days = ((offset + i64::from(month.days_in_month()) + 6) / 7) * 7;
                (start, days)
            }
        };
        (
            start,
            start
                .checked_add(Span::new().days(days - 1))
                .unwrap_or(start),
        )
    }
}

#[cfg(test)]
mod calendar_tests {
    use super::*;
    #[test]
    fn month_navigation_covers_six_week_month_and_year_boundary() {
        let mut state = AgendaViewState::default();
        state.calendar_range = CalendarRange::Month;
        let window = state.calendar_window("2026-03-15".parse().unwrap());
        assert_eq!(window.0.to_string(), "2026-02-23");
        assert_eq!(window.1.to_string(), "2026-04-05");
        state.calendar_offset = 1;
        let window = state.calendar_window("2026-12-31".parse().unwrap());
        assert_eq!(window.0.to_string(), "2026-12-28");
        assert_eq!(window.1.to_string(), "2027-01-31");
    }
}

impl Default for AgendaViewState {
    fn default() -> Self {
        Self {
            search_expanded: false,
            search_focus_pending: false,
            date_picker: false,
            picker_offset: 0,
            workspace: AgendaWorkspace::Agenda,
            builtin: BuiltinQuery::NextSevenDays,
            navigation: Arc::from("Agenda"),
            selected: None,
            overlay: AgendaOverlay::None,
            sheet: AgendaSheet::None,
            inspector: None,
            search: Arc::from(""),
            tag_filter: None,
            source_filter: None,
            structured_todo: None,
            structured_scheduled: None,
            projection: AgendaProjection::List,
            calendar_range: CalendarRange::Week,
            calendar_offset: 0,
            calendar_anchor: None,
            all_day_expanded: true,
            collapsed_days: Default::default(),
            smart_views_expanded: true,
            saved_views_expanded: false,
            tags_expanded: true,
            sources_expanded: true,
            source_context_menu: None,
            capture: crate::agenda::CaptureDraft {
                todo: "TODO".into(),
                ..Default::default()
            },
            inbox_session: None,
            selected_project: 0,
            refile_task: None,
            refile_search: String::new(),
            refile_selected: 0,
            recent_refile_targets: Vec::new(),
            workflow_message: None,
            repeat_task: None,
            repeat_target: None,
        }
    }
}

impl AgendaViewState {
    pub(crate) fn toggle_sidebar_section(&mut self, section: SidebarSection) {
        let expanded = match section {
            SidebarSection::SmartViews => &mut self.smart_views_expanded,
            SidebarSection::SavedViews => &mut self.saved_views_expanded,
            SidebarSection::Tags => &mut self.tags_expanded,
            SidebarSection::Sources => &mut self.sources_expanded,
        };
        *expanded = !*expanded;
    }

    pub(crate) fn select_row(&mut self, index: usize) {
        self.selected = Some(index);
        self.overlay = AgendaOverlay::None;
        self.sheet = AgendaSheet::Inspector;
        self.inspector = Some(InspectorState::default());
    }

    pub(crate) fn close_top_layer(&mut self) -> bool {
        if self.date_picker {
            self.date_picker = false;
            true
        } else if self.source_context_menu.take().is_some() {
            true
        } else if self.overlay != AgendaOverlay::None {
            self.overlay = AgendaOverlay::None;
            true
        } else if self.sheet != AgendaSheet::None {
            self.sheet = AgendaSheet::None;
            self.inspector = None;
            true
        } else if self.selected.take().is_some() {
            true
        } else {
            false
        }
    }

    pub(crate) fn move_selection(&mut self, delta: isize, row_count: usize) {
        if row_count == 0 {
            self.selected = None;
            return;
        }
        let current = self.selected.unwrap_or(0) as isize;
        self.selected = Some((current + delta).clamp(0, row_count as isize - 1) as usize);
    }

    pub(crate) fn move_visible_selection(
        &mut self,
        delta: isize,
        result: &crate::agenda::AgendaResultSnapshot,
    ) {
        let visible = result
            .groups
            .iter()
            .filter(|group| !self.collapsed_days.contains(&group.date))
            .flat_map(|group| group.rows.clone())
            .collect::<Vec<_>>();
        if visible.is_empty() {
            self.selected = None;
            return;
        }
        let position = self
            .selected
            .and_then(|selected| visible.iter().position(|index| *index == selected));
        let next = position.map_or(0, |position| {
            (position as isize + delta).clamp(0, visible.len() as isize - 1) as usize
        });
        self.selected = Some(visible[next]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sidebar_sections_toggle_independently() {
        let mut state = AgendaViewState::default();
        state.toggle_sidebar_section(SidebarSection::Tags);
        assert!(!state.tags_expanded);
        assert!(state.smart_views_expanded);
        assert!(!state.saved_views_expanded);
        assert!(state.sources_expanded);
        state.toggle_sidebar_section(SidebarSection::Tags);
        assert!(state.tags_expanded);
        state.toggle_sidebar_section(SidebarSection::SavedViews);
        assert!(state.saved_views_expanded);
    }

    #[test]
    fn layers_are_mutually_exclusive_and_escape_in_stack_order() {
        let mut state = AgendaViewState::default();
        state.overlay = AgendaOverlay::Capture;
        state.select_row(2);
        assert_eq!(state.overlay, AgendaOverlay::None);
        assert_eq!(state.sheet, AgendaSheet::Inspector);
        state.overlay = AgendaOverlay::Capture;
        assert!(state.close_top_layer());
        assert_eq!(state.sheet, AgendaSheet::Inspector);
        assert!(state.close_top_layer());
        assert!(state.inspector.is_none());
        assert!(state.close_top_layer());
        assert!(!state.close_top_layer());
    }
}
