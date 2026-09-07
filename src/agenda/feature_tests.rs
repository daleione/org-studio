use super::clock::ActiveClock;
use super::*;
use crate::org_semantic::{Repeater, RepeaterMode, TimeUnit, TodoStateKind};
use jiff::civil::Date;
use std::{fs, sync::Arc, time::SystemTime};
fn date(value: &str) -> Date {
    value.parse().unwrap()
}
#[test]
fn transition_adds_closed_logbook_and_tag_trigger() {
    let updated = transition_subtree(
        "* TODO Ship :old:\nBody\n",
        "TODO",
        &TodoTransition {
            target: Arc::from("DONE"),
            target_kind: TodoStateKind::Done,
            timestamp: Arc::from("2026-09-06 Sun 12:00"),
            log_state: true,
            add_tags: vec![Arc::from("done")].into(),
            remove_tags: vec![Arc::from("old")].into(),
        },
    )
    .unwrap();
    assert!(updated.starts_with("* DONE Ship :done:\n:LOGBOOK:"));
    assert!(updated.contains("CLOSED: [2026-09-06 Sun 12:00]"));
}
#[test]
fn repeat_modes_follow_org_semantics() {
    let scheduled = date("2026-09-01");
    let completed = date("2026-09-06");
    let make = |mode| Repeater {
        mode,
        value: 2,
        unit: TimeUnit::Day,
    };
    assert_eq!(
        next_repeat_date(scheduled, completed, make(RepeaterMode::Cumulative)),
        Some(date("2026-09-03"))
    );
    assert_eq!(
        next_repeat_date(scheduled, completed, make(RepeaterMode::CatchUp)),
        Some(date("2026-09-07"))
    );
    assert_eq!(
        next_repeat_date(scheduled, completed, make(RepeaterMode::Restart)),
        Some(date("2026-09-08"))
    );
}
#[test]
fn clock_recovers_one_active_session() {
    let path = std::env::temp_dir().join(format!("org-studio-clock-{}.json", std::process::id()));
    let store = ClockStore {
        pending: Vec::new(),
        active: Some(ActiveClock {
            file: "a.org".into(),
            heading_title: "A".into(),
            started_unix: 10,
        }),
    };
    store.save(&path).unwrap();
    let mut loaded = ClockStore::load(&path).unwrap();
    let log = loaded
        .stop(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(130))
        .unwrap();
    assert_eq!(log.duration_seconds(), 120);
    assert!(log.org_line().ends_with("00:02"));
    let _ = fs::remove_file(path);
}
#[test]
fn habit_is_bounded_to_28_days() {
    let today = date("2026-09-06");
    let stats = habit_stats(today, [today, date("2026-09-05"), date("2026-09-03")]);
    assert_eq!(
        (
            stats.completed,
            stats.current_streak,
            stats.best_streak,
            stats.completion_percent
        ),
        (3, 2, 2, 10)
    );
}
#[test]
fn unsupported_rules_are_diagnostic() {
    assert_eq!(
        compatibility_diagnostics("<2026-09-06 +1q> :BLOCKER: x").len(),
        2
    );
}

#[test]
fn ordered_project_explains_its_blocker() {
    let path = std::env::temp_dir().join("ordered-project.org");
    fs::write(&path, "* TODO Project\n").unwrap();
    let mut project = crate::agenda::tests::task_record_for_workflow(
        FileId(1),
        path.clone(),
        0,
        15,
        1,
        "Project",
    );
    project.properties = vec![(Arc::from("ORDERED"), Arc::from("t"))].into();
    let mut first = project.clone();
    first.title = Arc::from("First");
    let mut second = project.clone();
    second.title = Arc::from("Second");
    second.todo = Arc::from("NEXT");
    assert_eq!(
        project_blocked_reason(&project, &[first, second]).as_deref(),
        Some("ORDERED：先完成“First”")
    );
    let _ = fs::remove_file(path);
}
