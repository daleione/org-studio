mod calendar;
mod inbox;
mod inspector;
mod list;
mod projects;
mod text;
mod workflows;

pub(crate) use calendar::agenda_calendar;
pub(crate) use inbox::inbox_view;
pub(crate) use inspector::{InspectorProps, agenda_inspector};
pub(crate) use list::agenda_list;
pub(crate) use projects::projects_view;
pub(crate) use text::agenda_text;
pub(crate) use workflows::{WorkflowOverlay, workflow_overlay};
