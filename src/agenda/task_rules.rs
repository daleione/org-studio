use super::TaskRecord;
use crate::org_semantic::TodoStateKind;
use std::sync::Arc;

pub(crate) fn compatibility_diagnostics(source: &str) -> Vec<Arc<str>> {
    let mut result = Vec::new();
    for token in source.split_whitespace() {
        let token = token.trim_end_matches(['>', ']']);
        if (token.starts_with('+') || token.starts_with(".+"))
            && token
                .chars()
                .last()
                .is_some_and(|unit| !matches!(unit, 'h' | 'd' | 'w' | 'm' | 'y'))
        {
            result.push(Arc::from(format!("暂不支持的重复单位：{token}")));
        }
    }
    if source.contains(":BLOCKER:") {
        result.push(Arc::from(
            "暂不支持自定义 :BLOCKER: 表达式；使用 ORDERED 或 WAITING",
        ));
    }
    result
}

pub(crate) fn project_blocked_reason(
    project: &TaskRecord,
    children: &[TaskRecord],
) -> Option<Arc<str>> {
    let ordered = project.properties.iter().any(|(key, value)| {
        key.eq_ignore_ascii_case("ORDERED")
            && matches!(value.to_ascii_lowercase().as_str(), "t" | "true" | "yes")
    });
    if ordered {
        let first_open = children
            .iter()
            .position(|task| !matches!(task.todo_kind, TodoStateKind::Done));
        if let Some(index) = first_open
            && children[index + 1..]
                .iter()
                .any(|task| task.todo.eq_ignore_ascii_case("NEXT"))
        {
            return Some(Arc::from(format!(
                "ORDERED：先完成“{}”",
                children[index].title
            )));
        }
    }
    children
        .iter()
        .any(|task| task.todo.eq_ignore_ascii_case("WAITING"))
        .then(|| Arc::from("存在 WAITING 依赖"))
}
