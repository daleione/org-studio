use super::TaskRecord;
use crate::document::ByteRange;
use std::{fs, io, path::Path, sync::Arc};

mod capture;
mod inbox;
mod projects;
mod recovery;
mod refile;
#[cfg(test)]
mod tests;

pub(crate) use capture::{CaptureDraft, CaptureTemplate, append_capture, capture_text};
pub(crate) use inbox::InboxSession;
pub(crate) use projects::{ProjectSummary, derive_projects};
pub(crate) use recovery::{
    RecoveryReceipt, RecoveryStage, cleanup_recovery_duplicate, load_receipt, resume_recovery,
};
pub(crate) use refile::{RefileTarget, cross_file_refile, refile_targets};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WorkflowError {
    Io(Arc<str>),
    InvalidInput(&'static str),
    SourceChanged,
    InvalidTarget,
    ReceiptCorrupt,
}

impl From<io::Error> for WorkflowError {
    fn from(value: io::Error) -> Self {
        Self::Io(Arc::from(value.to_string()))
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut temporary = path.to_path_buf();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    temporary.set_extension(format!("{extension}.{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)
}

fn read_task_source(task: &TaskRecord) -> Result<String, WorkflowError> {
    let super::SourceVersion::Disk(expected) = &task.source.version else {
        return Err(WorkflowError::SourceChanged);
    };
    let path = task.source.path.as_path();
    let bytes = fs::read(path)?;
    if crate::document::FileStamp::from_loaded(path, &bytes)? != **expected {
        return Err(WorkflowError::SourceChanged);
    }
    String::from_utf8(bytes).map_err(|error| WorkflowError::Io(Arc::from(error.to_string())))
}

fn to_usize(range: ByteRange, length: usize) -> Result<std::ops::Range<usize>, WorkflowError> {
    let start = usize::try_from(range.start.0).map_err(|_| WorkflowError::SourceChanged)?;
    let end = usize::try_from(range.end.0).map_err(|_| WorkflowError::SourceChanged)?;
    if start > end || end > length {
        return Err(WorkflowError::SourceChanged);
    }
    Ok(start..end)
}

fn relevel_subtree(source: &str, old_level: u16, new_level: u16) -> String {
    let delta = i32::from(new_level) - i32::from(old_level);
    source
        .split_inclusive('\n')
        .map(|line| {
            let stars = line.bytes().take_while(|byte| *byte == b'*').count();
            if stars > 0 && line.as_bytes().get(stars) == Some(&b' ') {
                format!(
                    "{}{}",
                    "*".repeat((stars as i32 + delta).max(1) as usize),
                    &line[stars..]
                )
            } else {
                line.to_owned()
            }
        })
        .collect()
}
