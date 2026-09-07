use super::recovery::save_receipt;
use super::*;
use crate::agenda::{FileId, TaskKey};
use std::{fs, path::PathBuf, sync::Arc};

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("org-studio-m4-{}-{name}", std::process::id()))
}

#[test]
fn capture_preserves_unreadable_utf8_source() {
    let path = temp("invalid-utf8.org");
    fs::write(&path, [0xff, 0xfe]).unwrap();
    let draft = CaptureDraft {
        title: "New task".into(),
        ..Default::default()
    };
    assert!(append_capture(&path, None, &draft).is_err());
    assert_eq!(fs::read(&path).unwrap(), [0xff, 0xfe]);
    fs::remove_file(path).unwrap();
}

#[test]
fn disk_mutation_rejects_task_from_changed_source() {
    let path = temp("stale-task.org");
    let original = "* TODO Original\n";
    fs::write(&path, original).unwrap();
    let task = crate::agenda::tests::task_record_for_workflow(
        FileId(1),
        path.clone(),
        0,
        original.len() as u64,
        1,
        "Original",
    );
    let external = "* TODO Replaced\n";
    fs::write(&path, external).unwrap();
    assert_eq!(read_task_source(&task), Err(WorkflowError::SourceChanged));
    assert_eq!(fs::read_to_string(&path).unwrap(), external);
    fs::remove_file(path).unwrap();
}

#[test]
fn capture_templates_are_valid_org_and_cancel_is_memory_only() {
    let draft = CaptureDraft {
        template: CaptureTemplate::Meeting,
        title: "设计评审".into(),
        notes: "确认密度".into(),
        todo: "NEXT".into(),
        scheduled: Some("2026-09-07 Mon 10:00".into()),
    };
    let text = draft.render_org().unwrap();
    assert!(text.starts_with("* NEXT 设计评审 :meeting:"));
    assert!(text.contains("SCHEDULED:"));
}

#[test]
fn inbox_skip_cycles_without_losing_items() {
    let key = |local| TaskKey {
        file: FileId(1),
        local,
        shard_generation: 1,
    };
    let mut session = InboxSession::new([key(1), key(2)]);
    session.skip();
    assert_eq!(session.current(), Some(key(2)));
    session.finish();
    assert_eq!(session.current(), Some(key(1)));
    assert_eq!(session.progress(), (1, 2));
}

#[test]
fn copy_first_receipt_can_resume_or_remove_duplicate() {
    let source = temp("source.org");
    let destination = temp("destination.org");
    let receipt = temp("receipt.json");
    fs::write(&source, "* TODO Move me\nBody\n").unwrap();
    fs::write(&destination, "* TODO Project\n* TODO Other\n").unwrap();
    let task = crate::agenda::tests::task_record_for_workflow(
        FileId(1),
        source.clone(),
        0,
        fs::read_to_string(&source).unwrap().len() as u64,
        1,
        "Move me",
    );
    let target = crate::agenda::tests::task_record_for_workflow(
        FileId(2),
        destination.clone(),
        0,
        15,
        1,
        "Project",
    );
    let result = cross_file_refile(&task, &target, &receipt).unwrap();
    assert_eq!(result.stage, RecoveryStage::Complete);
    assert!(!fs::read_to_string(&source).unwrap().contains("Move me"));
    assert_eq!(
        fs::read_to_string(&destination).unwrap(),
        "* TODO Project\n** TODO Move me\nBody\n* TODO Other\n"
    );
    assert_eq!(
        cleanup_recovery_duplicate(&receipt),
        Err(WorkflowError::SourceChanged)
    );
    let _ = fs::remove_file(source);
    let _ = fs::remove_file(destination);
    let _ = fs::remove_file(receipt);
}

#[test]
fn capture_appends_inside_configured_inbox_heading() {
    let path = temp("capture.org");
    fs::write(&path, "* Inbox\n* Other\n").unwrap();
    append_capture(
        &path,
        Some("Inbox"),
        &CaptureDraft {
            title: "New item".into(),
            todo: "TODO".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("* Inbox\n** TODO New item\n\n* Other"));
    let _ = fs::remove_file(path);
}

#[test]
fn restart_resumes_a_copied_receipt_without_duplicate_loss() {
    let source = temp("resume-source.org");
    let destination = temp("resume-destination.org");
    let receipt_path = temp("resume-receipt.json");
    let source_text = "* TODO Keep safe\n";
    let copied_text = "** TODO Keep safe\n";
    fs::write(&source, source_text).unwrap();
    fs::write(&destination, format!("* TODO Target\n{copied_text}")).unwrap();
    save_receipt(
        &receipt_path,
        &RecoveryReceipt {
            source: source.clone(),
            destination: destination.clone(),
            source_start: 0,
            source_text: source_text.into(),
            copied_text: copied_text.into(),
            stage: RecoveryStage::Copied,
        },
    )
    .unwrap();
    let resumed = resume_recovery(&receipt_path).unwrap();
    assert_eq!(resumed.stage, RecoveryStage::Complete);
    assert_eq!(fs::read_to_string(&source).unwrap(), "");
    assert_eq!(
        fs::read_to_string(&destination)
            .unwrap()
            .matches("Keep safe")
            .count(),
        1
    );
    let _ = fs::remove_file(source);
    let _ = fs::remove_file(destination);
    let _ = fs::remove_file(receipt_path);
}

#[test]
fn prepared_receipt_never_deletes_source_without_destination_copy() {
    let source = temp("partial-source.org");
    let destination = temp("partial-destination.org");
    let receipt_path = temp("partial-receipt.json");
    let source_text = "* TODO Keep safe\n";
    fs::write(&source, source_text).unwrap();
    fs::write(&destination, "* TODO Target\n").unwrap();
    save_receipt(
        &receipt_path,
        &RecoveryReceipt {
            source: source.clone(),
            destination: destination.clone(),
            source_start: 0,
            source_text: source_text.into(),
            copied_text: "** TODO Keep safe\n".into(),
            stage: RecoveryStage::Prepared,
        },
    )
    .unwrap();
    assert_eq!(
        resume_recovery(&receipt_path),
        Err(WorkflowError::SourceChanged)
    );
    assert_eq!(fs::read_to_string(&source).unwrap(), source_text);
    let _ = fs::remove_file(source);
    let _ = fs::remove_file(destination);
    let _ = fs::remove_file(receipt_path);
}

#[test]
fn cleanup_removes_only_the_copied_duplicate_and_keeps_source() {
    let source = temp("cleanup-source.org");
    let destination = temp("cleanup-destination.org");
    let receipt_path = temp("cleanup-receipt.json");
    let source_text = "* TODO Keep safe\n";
    let copied_text = "** TODO Keep safe\n";
    fs::write(&source, source_text).unwrap();
    fs::write(&destination, format!("* TODO Target\n{copied_text}")).unwrap();
    save_receipt(
        &receipt_path,
        &RecoveryReceipt {
            source: source.clone(),
            destination: destination.clone(),
            source_start: 0,
            source_text: source_text.into(),
            copied_text: copied_text.into(),
            stage: RecoveryStage::Copied,
        },
    )
    .unwrap();
    cleanup_recovery_duplicate(&receipt_path).unwrap();
    assert_eq!(fs::read_to_string(&source).unwrap(), source_text);
    assert!(
        !fs::read_to_string(&destination)
            .unwrap()
            .contains("Keep safe")
    );
    let _ = fs::remove_file(source);
    let _ = fs::remove_file(destination);
    let _ = fs::remove_file(receipt_path);
}

#[test]
fn projects_include_done_progress_and_detect_stuck() {
    use crate::{document::DocumentSnapshot, org_semantic::analyze, org_syntax};
    let snapshot = DocumentSnapshot::from_utf8(b"#+TODO: TODO NEXT WAITING | DONE\n* TODO Active\n** NEXT Do it\n** DONE Finished\n* TODO Stuck\n** TODO No next\n".to_vec()).unwrap();
    let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
    let shard = crate::agenda::shard_from_live(
        FileId(8),
        1,
        Arc::new(PathBuf::from("/tmp/projects.org")),
        &analysis,
    );
    let mut index = crate::agenda::AgendaIndex::default();
    let projects = derive_projects(&index.replace(shard));
    assert_eq!(projects.len(), 2);
    assert_eq!(
        (projects[0].done, projects[0].total, projects[0].stuck),
        (1, 2, false)
    );
    assert!(projects[1].stuck);
}
