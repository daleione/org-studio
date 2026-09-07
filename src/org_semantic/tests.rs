use std::sync::Arc;

use crate::{
    document::{
        ByteRange, DocumentBuffer, DocumentSnapshot, EditTransaction, TextEdit, TextSnapshot,
    },
    org_syntax,
};

use super::{
    TimestampKind, TodoStateKind, analyze, analyze_incremental, parse_todo_directive,
    timestamp::{RepeaterMode, TimeUnit, parse_timestamps},
};

#[test]
fn todo_directive_preserves_state_boundary_fast_keys_and_logging() {
    let sequence =
        parse_todo_directive("#+TODO: PLAN(p) BUILD(b) WAIT(w@/!) | SHIPPED(s!) CANCELLED(c@)")
            .unwrap();
    assert_eq!(sequence.states.len(), 5);
    assert_eq!(sequence.states[2].keyword.as_ref(), "WAIT");
    assert_eq!(sequence.states[2].fast_key, Some('w'));
    assert_eq!(sequence.states[2].log_spec.as_deref(), Some("@/!"));
    assert_eq!(sequence.states[2].kind, TodoStateKind::Open);
    assert_eq!(sequence.states[3].kind, TodoStateKind::Done);
}

#[test]
fn sequence_without_separator_treats_only_the_last_state_as_done() {
    let sequence = parse_todo_directive("#+SEQ_TODO: OPEN(o) CLOSED(c)").unwrap();
    assert_eq!(sequence.states[0].kind, TodoStateKind::Open);
    assert_eq!(sequence.states[1].kind, TodoStateKind::Done);
}

#[test]
fn analysis_uses_custom_todo_states_and_inherits_tags() {
    let snapshot = DocumentSnapshot::from_utf8(
        b"#+TODO: PLAN(p) WAIT(w@) | SHIPPED(s!)\n* PLAN [#A] Project :work:\n** WAIT Child :next:\n*** TODO is title text\n"
            .to_vec(),
    )
    .unwrap();
    let blocks = Arc::new(org_syntax::parse(&snapshot));
    let analysis = analyze(&snapshot, blocks);

    assert_eq!(analysis.headings.len(), 3);
    assert_eq!(analysis.config.file_tags.len(), 0);
    assert_eq!(
        analysis.headings[0].todo.as_ref().unwrap().keyword.as_ref(),
        "PLAN"
    );
    assert_eq!(analysis.headings[0].priority, Some('A'));
    assert_eq!(analysis.headings[0].title.as_ref(), "Project");
    assert_eq!(
        analysis.headings[1]
            .effective_tags
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>(),
        ["work", "next"]
    );
    assert!(analysis.headings[2].todo.is_none());
    assert_eq!(analysis.headings[2].title.as_ref(), "TODO is title text");
}

#[test]
fn file_keywords_are_shared_semantic_configuration() {
    let snapshot = DocumentSnapshot::from_utf8(
        b"#+FILETAGS: :work:mac:\n#+CATEGORY: product\n#+PROPERTY: Effort_ALL 0:15 0:30\n#+ARCHIVE: archive/%s::\n* TODO Item\n".to_vec(),
    )
    .unwrap();
    let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
    assert_eq!(
        analysis
            .config
            .file_tags
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>(),
        ["work", "mac"]
    );
    assert_eq!(analysis.config.category.as_deref(), Some("product"));
    assert_eq!(
        analysis.headings[0]
            .effective_tags
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>(),
        ["work", "mac"]
    );
    assert_eq!(
        analysis.config.property_defaults[0],
        (Arc::from("Effort_ALL"), Arc::from("0:15 0:30"))
    );
    assert_eq!(
        analysis.config.archive_location.as_deref(),
        Some("archive/%s::")
    );
}

#[test]
fn default_states_match_org_defaults() {
    let snapshot = DocumentSnapshot::from_utf8(b"* TODO Open\n* DONE Closed\n".to_vec()).unwrap();
    let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
    assert_eq!(
        analysis.headings[0].todo.as_ref().unwrap().kind,
        TodoStateKind::Open
    );
    assert_eq!(
        analysis.headings[1].todo.as_ref().unwrap().kind,
        TodoStateKind::Done
    );
}

#[test]
fn official_timestamp_forms_preserve_ranges_repeaters_and_warnings() {
    let text = "DEADLINE: <2005-10-01 Sat 09:00-10:30 ++1m -3d> CLOSED: [2005-09-30 Fri 18:00]";
    let timestamps = parse_timestamps(text, 17);
    assert_eq!(timestamps.len(), 2);
    assert_eq!(timestamps[0].kind, TimestampKind::Deadline);
    assert!(timestamps[0].active);
    assert_eq!(timestamps[0].start_date.to_string(), "2005-10-01");
    assert_eq!(timestamps[0].start_time.unwrap().to_string(), "09:00:00");
    assert_eq!(timestamps[0].end_time.unwrap().to_string(), "10:30:00");
    let repeater = timestamps[0].repeater.unwrap();
    assert_eq!(repeater.mode, RepeaterMode::CatchUp);
    assert_eq!(repeater.value, 1);
    assert_eq!(repeater.unit, TimeUnit::Month);
    assert_eq!(timestamps[0].warning.unwrap().value, 3);
    assert_eq!(timestamps[1].kind, TimestampKind::Closed);
    assert!(!timestamps[1].active);
    assert_eq!(
        &text[timestamps[0].source_range.start.0 as usize - 17
            ..timestamps[0].source_range.end.0 as usize - 17],
        "<2005-10-01 Sat 09:00-10:30 ++1m -3d>"
    );
}

#[test]
fn official_date_ranges_and_restart_repeaters_are_distinct() {
    let timestamps = parse_timestamps(
        "<2006-11-03 Fri>--<2006-11-06 Mon> <2019-04-05 Fri 08:00 .+1h>",
        0,
    );
    assert_eq!(timestamps.len(), 2);
    assert_eq!(timestamps[0].end_date.unwrap().to_string(), "2006-11-06");
    assert_eq!(timestamps[1].repeater.unwrap().mode, RepeaterMode::Restart);
}

#[test]
fn pinned_official_fixture_builds_one_coherent_semantic_snapshot() {
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/agenda/org-mode-437fd11-baseline.org"
    ));
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
    assert_eq!(analysis.headings.len(), 6);
    assert_eq!(
        analysis
            .headings
            .iter()
            .flat_map(|heading| heading.timestamps.iter())
            .count(),
        6
    );
    assert_eq!(analysis.document_id, snapshot.document_id());
    assert_eq!(analysis.revision, snapshot.revision());
    assert!(analysis.diagnostics.is_empty());
}

#[test]
fn incremental_semantics_match_full_and_reuse_unchanged_headings() {
    let source = (0..100)
        .map(|index| format!("* TODO Heading {index} :work:\nbody {index}\n"))
        .collect::<String>();
    let mut buffer = DocumentBuffer::from_utf8(source.into_bytes()).unwrap();
    let before = buffer.snapshot();
    let previous_blocks = Arc::new(org_syntax::parse(&before));
    let previous = analyze(&before, previous_blocks.clone());
    let all = before.copy_range(ByteRange::new(0, before.len_bytes()));
    let start = all.find("body 50").unwrap() as u64 + 5;
    let delta = buffer
        .commit(EditTransaction::new(
            before.revision(),
            vec![TextEdit::new(ByteRange::new(start, start + 2), "fifty")],
        ))
        .unwrap();
    let after = buffer.snapshot();
    let (blocks, patch) =
        org_syntax::parse_incremental(&after, &previous_blocks, &[delta]).unwrap();
    let incremental = analyze_incremental(&after, Arc::new(blocks), &previous, &patch);
    let full = analyze(&after, Arc::new(org_syntax::parse(&after)));

    let project = |analysis: &super::OrgAnalysisSnapshot| {
        analysis
            .headings
            .iter()
            .map(|heading| {
                (
                    heading.level,
                    heading.title.to_string(),
                    heading.todo.as_ref().map(|todo| todo.keyword.to_string()),
                    heading.effective_tags.to_vec(),
                    heading.timestamps.to_vec(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(project(&incremental), project(&full));
    assert!(
        incremental.metrics.reused_headings >= 97,
        "metrics: {:?}",
        incremental.metrics
    );
    assert!(incremental.metrics.extracted_headings <= 3);
    assert!(incremental.metrics.semantic_source_bytes < after.len_bytes() / 10);
}
#[test]
fn heading_tags_after_unicode_whitespace_do_not_panic() {
    let snapshot = crate::document::DocumentSnapshot::from_utf8(
        "* TODO 标题\u{3000}:标签:\n".as_bytes().to_vec(),
    )
    .unwrap();
    let analysis = super::analyze(
        &snapshot,
        std::sync::Arc::new(crate::org_syntax::parse(&snapshot)),
    );
    assert_eq!(analysis.headings[0].title.as_ref(), "标题");
    assert_eq!(analysis.headings[0].tags[0].as_ref(), "标签");
}
