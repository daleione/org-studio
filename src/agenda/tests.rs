use std::{path::PathBuf, sync::Arc, time::Instant};

use jiff::civil::Date;

use crate::{document::DocumentSnapshot, org_semantic::analyze, org_syntax};

use super::text::format_agenda_text;
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
        result.entries.len()
    );
    assert_eq!(result.entries.len(), 10_000);
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
    assert!(
        result
            .entries
            .iter()
            .any(|entry| entry.row.title.as_ref() == "Future")
    );
    assert!(
        result
            .entries
            .iter()
            .any(|entry| entry.row.title.as_ref() == "Repeat"
                && entry.row.date == Some(Date::new(2026, 10, 20).unwrap()))
    );
    assert!(
        result
            .entries
            .iter()
            .all(|entry| entry.row.date.unwrap().month() == 10)
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
    assert_eq!(today_result.entries.len(), 1);
    assert_eq!(today_result.entries[0].row.title.as_ref(), "Ship release");
    assert_eq!(today_result.facets.next, 1);
    assert_eq!(today_result.facets.waiting, 1);
    assert_eq!(today_result.facets.unscheduled, 1);
    let overdue = engine.execute(
        snapshot.clone(),
        &AgendaQuery::builtin(BuiltinQuery::Overdue, today),
    );
    assert_eq!(
        overdue
            .entries
            .iter()
            .map(|entry| entry.row.title.as_ref())
            .collect::<Vec<_>>(),
        ["Weekly", "Vendor"]
    );
    let unscheduled = engine.execute(
        snapshot,
        &AgendaQuery::builtin(BuiltinQuery::Unscheduled, today),
    );
    assert_eq!(unscheduled.entries.len(), 1);
    assert!(unscheduled.entries[0].occurrence.is_none());
    assert_eq!(unscheduled.placements[0].date, None);
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
        result
            .entries
            .iter()
            .any(|entry| entry.row.title.as_ref() == "Weekly"
                && entry.row.date == Some(Date::new(2026, 9, 8).unwrap()))
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
        .entries
        .iter()
        .map(|entry| &entry.row)
        .find(|row| row.title.as_ref() == "中文重叠事件")
        .unwrap();
    assert_eq!(range.time.unwrap().to_string(), "10:00:00");
    assert_eq!(range.end_time.unwrap().to_string(), "11:30:00");
    let cross_day = result
        .entries
        .iter()
        .map(|entry| &entry.row)
        .find(|row| row.title.as_ref() == "DST cross day")
        .unwrap();
    assert_eq!(cross_day.end_date, Some(Date::new(2026, 3, 9).unwrap()));
}

#[test]
fn cross_day_occurrence_has_clipped_daily_placements() {
    let source = "* TODO Trip\n<2026-09-06 Sun 23:00>--<2026-09-08 Tue 01:00>\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
    let shard = shard_from_live(
        FileId(10),
        1,
        Arc::new(PathBuf::from("/notes/trip.org")),
        &analysis,
    );
    let mut query =
        AgendaQuery::builtin(BuiltinQuery::NextSevenDays, "2026-09-07".parse().unwrap());
    query.window = Some(("2026-09-07".parse().unwrap(), "2026-09-08".parse().unwrap()));
    let mut index = AgendaIndex::default();
    let mut result = QueryEngine::default().execute(index.replace(shard), &query);
    result.query_id = Some(QueryId(44));
    assert_eq!(result.entries.len(), 1);
    let occurrence = result.entries[0].occurrence.as_ref().unwrap();
    assert_eq!(occurrence.start_date.to_string(), "2026-09-06");
    assert_eq!(occurrence.end_date.unwrap().to_string(), "2026-09-08");
    assert_eq!(result.placements.len(), 2);
    assert_eq!(result.placements[0].date.unwrap().to_string(), "2026-09-07");
    assert!(result.placements[0].continues_before);
    assert!(result.placements[0].continues_after);
    assert_eq!(result.placements[1].date.unwrap().to_string(), "2026-09-08");
    assert_eq!(
        result.placements[1].end_time.unwrap().to_string(),
        "01:00:00"
    );
    let reference = result.entry_ref(result.entries[0].key).unwrap();
    assert_eq!(
        result.resolve_entry(reference).unwrap().key,
        result.entries[0].key
    );
    let placement = result.placement_ref(result.placements[0].key).unwrap();
    assert_eq!(placement.entry, reference);
    let mut stale = reference;
    stale.request_generation += 1;
    assert!(result.resolve_entry(stale).is_none());
}

#[test]
fn equal_timestamps_in_one_task_remain_distinct_entries() {
    let source = "* TODO Pair\n<2026-09-07 Mon 10:00> <2026-09-07 Mon 10:00>\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
    let shard = shard_from_live(
        FileId(11),
        1,
        Arc::new(PathBuf::from("/notes/pair.org")),
        &analysis,
    );
    let mut index = AgendaIndex::default();
    let result = QueryEngine::default().execute(
        index.replace(shard),
        &AgendaQuery::builtin(BuiltinQuery::Today, "2026-09-07".parse().unwrap()),
    );
    assert_eq!(result.entries.len(), 2);
    assert_ne!(
        result.entries[0].occurrence.as_ref().unwrap().timestamp,
        result.entries[1].occurrence.as_ref().unwrap().timestamp
    );
}

#[test]
fn placement_contract_distinguishes_midnight_end_and_inclusive_date_range() {
    let source = "* TODO Timed\n<2026-09-07 Mon 20:00>--<2026-09-08 Tue 00:00>\n* TODO Dates\n<2026-09-07 Mon>--<2026-09-08 Tue>\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
    let shard = shard_from_live(
        FileId(12),
        1,
        Arc::new(PathBuf::from("/notes/ranges.org")),
        &analysis,
    );
    let mut query =
        AgendaQuery::builtin(BuiltinQuery::NextSevenDays, "2026-09-07".parse().unwrap());
    query.window = Some(("2026-09-07".parse().unwrap(), "2026-09-08".parse().unwrap()));
    let mut index = AgendaIndex::default();
    let result = QueryEngine::default().execute(index.replace(shard), &query);
    let timed = result
        .entries
        .iter()
        .find(|entry| entry.row.title.as_ref() == "Timed")
        .unwrap();
    let dates = result
        .entries
        .iter()
        .find(|entry| entry.row.title.as_ref() == "Dates")
        .unwrap();
    assert_eq!(
        result
            .placements
            .iter()
            .filter(|item| item.entry == timed.key)
            .count(),
        1
    );
    assert_eq!(
        result
            .placements
            .iter()
            .filter(|item| item.entry == dates.key)
            .count(),
        2
    );
}

#[test]
fn repeated_occurrence_recomputes_its_real_end_before_placement() {
    let source = "* TODO Repeated trip\n<2026-09-01 Tue 23:00 +1w>--<2026-09-03 Thu 01:00>\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
    let shard = shard_from_live(
        FileId(13),
        1,
        Arc::new(PathBuf::from("/notes/repeated-range.org")),
        &analysis,
    );
    let mut query =
        AgendaQuery::builtin(BuiltinQuery::NextSevenDays, "2026-09-07".parse().unwrap());
    query.window = Some(("2026-09-07".parse().unwrap(), "2026-09-10".parse().unwrap()));
    let mut index = AgendaIndex::default();
    let result = QueryEngine::default().execute(index.replace(shard), &query);
    let occurrence = result
        .entries
        .iter()
        .find_map(|entry| entry.occurrence.as_ref())
        .unwrap();
    assert_eq!(occurrence.start_date.to_string(), "2026-09-08");
    assert_eq!(occurrence.end_date.unwrap().to_string(), "2026-09-10");
    assert_eq!(result.placements.len(), 3);
}

#[test]
fn text_projection_aligns_unicode_categories_by_display_width() {
    let mut index = AgendaIndex::default();
    let result = QueryEngine::default().execute(
        index.replace(fixture_shard()),
        &AgendaQuery::builtin(BuiltinQuery::Today, Date::new(2026, 9, 5).unwrap()),
    );
    let lines = format_agenda_text(&result, "Scheduled", "Deadline");
    assert!(
        lines
            .iter()
            .any(|line| line.text.contains("work:") && &line.text[line.todo.clone()] == "NEXT")
    );
}

#[test]
fn text_projection_preserves_org_prefixes_ranges_and_unicode_alignment() {
    let mut index = AgendaIndex::default();
    let mut result = QueryEngine::default().execute(
        index.replace(fixture_shard()),
        &AgendaQuery::builtin(BuiltinQuery::Today, Date::new(2026, 9, 5).unwrap()),
    );
    let mut entry = result.entries[0].clone();
    let row = &mut entry.row;
    row.category = Some(Arc::from("工作"));
    row.title = Arc::from("发布说明");
    row.todo = Arc::from("TODO");
    row.priority = Some('A');
    row.tags = Arc::from([Arc::from("发布")]);
    row.time = Some("10:00".parse().unwrap());
    row.end_time = Some("11:30".parse().unwrap());
    row.end_date = row.date;
    row.date_kind = Some(AgendaDateKind::Scheduled);
    let mut other = entry.clone();
    other.key = AgendaEntryKey(1);
    other.row.category = Some(Arc::from("work"));
    result.entries = Arc::from([entry, other]);
    let lines = format_agenda_text(&result, "Scheduled", "Deadline");
    for line in &lines {
        assert!(
            line.text
                .contains("10:00-11:30. Scheduled: TODO [#A] 发布说明")
        );
        assert_eq!(&line.text[line.todo.clone()], "TODO");
        assert_eq!(&line.text[line.priority.clone().unwrap()], "[#A]");
        assert_eq!(&line.text[line.tags.clone().unwrap()], ":发布:");
    }
    let column = |line: &super::text::AgendaTextLine| {
        unicode_width::UnicodeWidthStr::width(&line.text[..line.todo.start])
    };
    assert_eq!(column(&lines[0]), column(&lines[1]));
    let chinese = format_agenda_text(&result, "计划", "截止");
    assert!(chinese[0].text.contains("计划: TODO"));
    assert_eq!(&chinese[0].text[chinese[0].todo.clone()], "TODO");
}

#[test]
fn unified_projection_preserves_cross_day_membership_empty_dates_and_targets() {
    let source =
        "* TODO 跨日任务 :中文:very-long-tag:\n<2026-09-06 Sun 23:00>--<2026-09-08 Tue 01:00>\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
    let mut index = AgendaIndex::default();
    let mut query =
        AgendaQuery::builtin(BuiltinQuery::NextSevenDays, Date::new(2026, 9, 6).unwrap());
    query.window = Some((
        Date::new(2026, 9, 6).unwrap(),
        Date::new(2026, 9, 9).unwrap(),
    ));
    let result = QueryEngine::default().execute(
        index.replace(shard_from_live(
            FileId(71),
            1,
            Arc::new(PathBuf::from("/notes/中文.org")),
            &analysis,
        )),
        &query,
    );
    assert_eq!(result.entries.len(), 1);
    assert_eq!(result.placements.len(), 3);
    let started = Instant::now();
    let projection = project_agenda_text(
        &result,
        "计划",
        "截止",
        "无日期",
        query.window,
        |date, week| format!("{date}{}", if week { " W" } else { "" }),
    );
    assert!(started.elapsed().as_secs() < 5);
    let headers = projection
        .iter()
        .filter(|line| matches!(line, AgendaProjectedLine::Header { .. }))
        .count();
    let entries = projection
        .iter()
        .filter_map(|line| match line {
            AgendaProjectedLine::Entry {
                entry,
                placement,
                line,
            } => Some((*entry, *placement, line.text.as_str())),
            AgendaProjectedLine::Header { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(headers, 4, "the empty fourth date remains visible");
    assert_eq!(entries.len(), 3);
    assert!(entries.iter().all(|(entry, _, text)| {
        *entry == 0 && text.contains("跨日任务") && text.contains(":中文:very-long-tag:")
    }));
    assert_eq!(
        entries
            .iter()
            .map(|(_, placement, _)| placement.0)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
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
