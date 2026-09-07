use super::super::{AgendaIndexSnapshot, TaskKey, TaskRecord};
use crate::org_semantic::TodoStateKind;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Debug)]
pub(crate) struct ProjectSummary {
    pub(crate) project: TaskRecord,
    pub(crate) children: Arc<[TaskRecord]>,
    pub(crate) next_actions: Arc<[TaskRecord]>,
    pub(crate) done: usize,
    pub(crate) total: usize,
    pub(crate) stuck: bool,
    pub(crate) blocked_reason: Option<Arc<str>>,
}

pub(crate) fn derive_projects(index: &AgendaIndexSnapshot) -> Vec<ProjectSummary> {
    let tasks = index
        .files
        .iter()
        .flat_map(|file| file.tasks.iter())
        .cloned()
        .collect::<Vec<_>>();
    let by_parent = tasks
        .iter()
        .filter_map(|task| task.parent.map(|parent| (parent, task.clone())))
        .fold(
            HashMap::<TaskKey, Vec<TaskRecord>>::new(),
            |mut map, (parent, child)| {
                map.entry(parent).or_default().push(child);
                map
            },
        );
    tasks
        .iter()
        .filter_map(|project| {
            let children = by_parent.get(&project.key)?.clone();
            let done = children
                .iter()
                .filter(|task| matches!(task.todo_kind, TodoStateKind::Done))
                .count();
            let next_actions = children
                .iter()
                .filter(|task| {
                    !matches!(task.todo_kind, TodoStateKind::Done)
                        && task.todo.eq_ignore_ascii_case("NEXT")
                })
                .cloned()
                .collect::<Vec<_>>();
            let total = children.len();
            let blocked_reason = super::super::project_blocked_reason(project, &children);
            Some(ProjectSummary {
                project: project.clone(),
                children: children.into(),
                stuck: done < total && (next_actions.is_empty() || blocked_reason.is_some()),
                blocked_reason,
                next_actions: next_actions.into(),
                done,
                total,
            })
        })
        .collect()
}
