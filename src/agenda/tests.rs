use std::{path::PathBuf, sync::Arc};

use jiff::civil::Date;

use crate::{document::DocumentSnapshot, org_semantic::analyze, org_syntax};

use super::*;

pub(crate) fn task_record_for_workflow(
    file: FileId,
    path: PathBuf,
    start: u64,
    end: u64,
    level: u16,
    title: &str,
) -> TaskRecord {
    let bytes = std::fs::read(&path).unwrap();
    TaskRecord {
        key: TaskKey {
            file,
            local: 0,
            shard_generation: 1,
        },
        source: SourceLocator {
            file,
            path: Arc::new(path.clone()),
            version: SourceVersion::Disk(Arc::new(
                crate::document::FileStamp::from_loaded(&path, &bytes).unwrap(),
            )),
            heading_range: crate::document::ByteRange::new(start, end),
            title_range: crate::document::ByteRange::new(start, end),
            anchor: None,
            fingerprint: HeadingFingerprint {
                level,
                title: Arc::from(title),
                parent_title: None,
            },
        },
        level,
        title: Arc::from(title),
        todo: Arc::from("TODO"),
        todo_kind: crate::org_semantic::TodoStateKind::Open,
        priority: None,
        effective_tags: Arc::from([]),
        category: None,
        properties: Arc::from([]),
        parent: None,
        timestamps: Arc::from([]),
        allowed_todo_states: Arc::from([Arc::from("TODO"), Arc::from("DONE")]),
    }
}

#[test]
#[ignore = "explicit M5 performance gate"]
fn benchmark_10k_headings_across_50_files() {
    use std::time::Instant;
    let source = (0..200)
        .map(|index| format!("* TODO Task {index} :bench:\nSCHEDULED: <2026-09-06 Sun>\n"))
        .collect::<String>()
        .into_bytes();
    let started = Instant::now();
    let mut index = AgendaIndex::default();
    for file in 0..50 {
        let snapshot = DocumentSnapshot::from_utf8(source.clone()).unwrap();
        let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
        index.replace(shard_from_live(
            FileId(file + 1),
            1,
            Arc::new(PathBuf::from(format!("/tmp/m5-bench-{file}.org"))),
            &analysis,
        ));
    }
    let indexed = started.elapsed();
    let query_started = Instant::now();
    let result = QueryEngine::default().execute(
        index.snapshot(),
        &AgendaQuery::builtin(BuiltinQuery::Today, "2026-09-06".parse().unwrap()),
    );
    let queried = query_started.elapsed();
    eprintln!(
        "M5_BENCH headings=10000 files=50 index_ms={} query_ms={} rows={}",
        indexed.as_millis(),
        queried.as_millis(),
        result.rows.len()
    );
    assert_eq!(result.rows.len(), 10_000);
    assert!(indexed.as_secs() < 30);
    assert!(queried.as_secs() < 5);
}

fn fixture_shard() -> FileAgendaShard {
    let source = b"#+TODO: TODO NEXT WAIT | DONE\n#+CATEGORY: work\n* NEXT Ship release :product:\nDEADLINE: <2026-09-05 Sat>\n* WAIT Vendor\nSCHEDULED: <2026-09-03 Thu>\n* TODO Weekly\n<2026-09-01 Tue +1w>\n* TODO Backlog\n".to_vec();
    let snapshot = DocumentSnapshot::from_utf8(source).unwrap();
    let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
    shard_from_live(
        FileId(7),
        1,
        Arc::new(PathBuf::from("/notes/work.org")),
        &analysis,
    )
}

#[test]
fn calendar_query_uses_visible_month_instead_of_today_window() {
    let source = "* TODO Future\n<2026-10-20 Tue>\n* TODO Repeat\n<2026-09-01 Tue +1w>\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
    let shard = shard_from_live(
        FileId(1),
        1,
        Arc::new(PathBuf::from("/tmp/calendar.org")),
        &analysis,
    );
    let mut index = AgendaIndex::default();
    let mut query =
        AgendaQuery::builtin(BuiltinQuery::NextSevenDays, Date::new(2026, 9, 7).unwrap());
    query.window = Some((
        Date::new(2026, 10, 1).unwrap(),
        Date::new(2026, 10, 31).unwrap(),
    ));
    let result = QueryEngine::default().execute(index.replace(shard), &query);
    assert!(result.rows.iter().any(|row| row.title.as_ref() == "Future"));
    assert!(
        result.rows.iter().any(|row| row.title.as_ref() == "Repeat"
            && row.date == Some(Date::new(2026, 10, 20).unwrap()))
    );
    assert!(
        result
            .rows
            .iter()
            .all(|row| row.date.unwrap().month() == 10)
    );
}

#[test]
fn builtins_and_facets_share_one_traversal_result() {
    let mut index = AgendaIndex::default();
    let snapshot = index.replace(fixture_shard());
    let today = Date::new(2026, 9, 5).unwrap();
    let mut engine = QueryEngine::default();
    let today_result = engine.execute(
        snapshot.clone(),
        &AgendaQuery::builtin(BuiltinQuery::Today, today),
    );
    assert_eq!(today_result.rows.len(), 1);
    assert_eq!(today_result.rows[0].title.as_ref(), "Ship release");
    assert_eq!(today_result.facets.next, 1);
    assert_eq!(today_result.facets.waiting, 1);
    assert_eq!(today_result.facets.unscheduled, 1);
    let overdue = engine.execute(
        snapshot,
        &AgendaQuery::builtin(BuiltinQuery::Overdue, today),
    );
    assert_eq!(
        overdue
            .rows
            .iter()
            .map(|row| row.title.as_ref())
            .collect::<Vec<_>>(),
        ["Weekly", "Vendor"]
    );
}

#[test]
fn repeater_expands_only_inside_visible_query_window() {
    let mut index = AgendaIndex::default();
    let snapshot = index.replace(fixture_shard());
    let mut engine = QueryEngine::default();
    let result = engine.execute(
        snapshot,
        &AgendaQuery::builtin(BuiltinQuery::NextSevenDays, Date::new(2026, 9, 7).unwrap()),
    );
    assert!(
        result.rows.iter().any(|row| row.title.as_ref() == "Weekly"
            && row.date == Some(Date::new(2026, 9, 8).unwrap()))
    );
}

#[test]
fn calendar_occurrence_retains_time_range_and_cross_day_end() {
    let source = "* TODO 中文重叠事件\n<2026-03-08 Sun 10:00-11:30>\n* TODO DST cross day\n<2026-03-08 Sun 20:00>--<2026-03-09 Mon 09:00>\n".as_bytes().to_vec();
    let snapshot = DocumentSnapshot::from_utf8(source).unwrap();
    let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
    let shard = shard_from_live(
        FileId(9),
        1,
        Arc::new(PathBuf::from("/notes/calendar.org")),
        &analysis,
    );
    let mut index = AgendaIndex::default();
    let result = QueryEngine::default().execute(
        index.replace(shard),
        &AgendaQuery::builtin(BuiltinQuery::NextSevenDays, Date::new(2026, 3, 8).unwrap()),
    );
    let range = result
        .rows
        .iter()
        .find(|row| row.title.as_ref() == "中文重叠事件")
        .unwrap();
    assert_eq!(range.time.unwrap().to_string(), "10:00:00");
    assert_eq!(range.end_time.unwrap().to_string(), "11:30:00");
    let cross_day = result
        .rows
        .iter()
        .find(|row| row.title.as_ref() == "DST cross day")
        .unwrap();
    assert_eq!(cross_day.end_date, Some(Date::new(2026, 3, 9).unwrap()));
}

#[test]
fn text_projection_aligns_unicode_categories_by_display_width() {
    let mut index = AgendaIndex::default();
    let result = QueryEngine::default().execute(
        index.replace(fixture_shard()),
        &AgendaQuery::builtin(BuiltinQuery::Today, Date::new(2026, 9, 5).unwrap()),
    );
    let text = format_agenda_text(&result);
    assert!(text.contains("work: NEXT"));
}

#[test]
fn source_discovery_is_recursive_and_org_only() {
    let root =
        std::env::temp_dir().join(format!("org-studio-agenda-source-{}", std::process::id()));
    let nested = root.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(root.join("a.org"), "* TODO A\n").unwrap();
    std::fs::write(nested.join("b.ORG"), "* TODO B\n").unwrap();
    std::fs::write(nested.join("ignore.md"), "# no\n").unwrap();
    let found = discover_sources(&[root.clone()]).unwrap();
    assert_eq!(found.len(), 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn versioned_config_loads_from_disk() {
    let root =
        std::env::temp_dir().join(format!("org-studio-agenda-config-{}", std::process::id()));
    let store = AgendaConfigStore::new(root.join("agenda.json"));
    let config = AgendaConfig {
        version: 1,
        sources: vec![PathBuf::from("/notes")],
        inbox: None,
        saved_views: Vec::new(),
    };
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("agenda.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    assert_eq!(store.load().unwrap(), config);
    std::fs::remove_dir_all(root).unwrap();
}
