use super::super::{AgendaIndexSnapshot, FileId, TaskKey, TaskRecord};
use super::recovery::save_receipt;
use super::{
    RecoveryReceipt, RecoveryStage, WorkflowError, atomic_write, read_task_source, relevel_subtree,
    resume_recovery, to_usize,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RefileTarget {
    pub(crate) task: TaskKey,
    pub(crate) file: FileId,
    pub(crate) path: Arc<PathBuf>,
    pub(crate) title: Arc<str>,
    pub(crate) level: u16,
}

pub(crate) fn refile_targets(
    index: &AgendaIndexSnapshot,
    moving: TaskKey,
    search: &str,
    recent: &[TaskKey],
) -> Vec<RefileTarget> {
    let needle = search.trim().to_lowercase();
    let moving_record = index
        .files
        .iter()
        .flat_map(|file| file.tasks.iter())
        .find(|task| task.key == moving);
    let Some(moving_record) = moving_record else {
        return Vec::new();
    };
    let mut targets = index
        .files
        .iter()
        .flat_map(|file| file.tasks.iter())
        .filter(|task| {
            task.key != moving
                && !(task.source.file == moving.file
                    && task.source.heading_range.start >= moving_record.source.heading_range.start
                    && task.source.heading_range.end <= moving_record.source.heading_range.end)
                && (needle.is_empty() || task.title.to_lowercase().contains(&needle))
        })
        .map(|task| RefileTarget {
            task: task.key,
            file: task.key.file,
            path: task.source.path.clone(),
            title: task.title.clone(),
            level: task.level,
        })
        .collect::<Vec<_>>();
    targets.sort_by_key(|target| {
        recent
            .iter()
            .position(|key| *key == target.task)
            .unwrap_or(usize::MAX)
    });
    targets
}

pub(crate) fn cross_file_refile(
    source: &TaskRecord,
    target: &TaskRecord,
    receipt_path: &Path,
) -> Result<RecoveryReceipt, WorkflowError> {
    if source.source.path == target.source.path {
        return Err(WorkflowError::InvalidTarget);
    }
    let source_text = read_task_source(source)?;
    let destination_text = read_task_source(target)?;
    let range = to_usize(source.source.heading_range, source_text.len())?;
    let subtree = source_text
        .get(range.clone())
        .ok_or(WorkflowError::SourceChanged)?
        .to_owned();
    let copied = relevel_subtree(&subtree, source.level, target.level + 1);
    let mut receipt = RecoveryReceipt {
        source: source.source.path.as_ref().clone(),
        destination: target.source.path.as_ref().clone(),
        source_start: range.start,
        source_text: subtree,
        copied_text: copied,
        stage: RecoveryStage::Prepared,
    };
    save_receipt(receipt_path, &receipt)?;
    let insertion = to_usize(target.source.heading_range, destination_text.len())?.end;
    let mut destination_next = destination_text;
    if !destination_next.is_char_boundary(insertion) {
        return Err(WorkflowError::SourceChanged);
    }
    let separator = if insertion > 0 && !destination_next[..insertion].ends_with('\n') {
        "\n"
    } else {
        ""
    };
    destination_next.insert_str(insertion, &format!("{separator}{}", receipt.copied_text));
    atomic_write(&receipt.destination, destination_next.as_bytes())?;
    receipt.stage = RecoveryStage::Copied;
    save_receipt(receipt_path, &receipt)?;
    resume_recovery(receipt_path)
}
