use super::{WorkflowError, atomic_write};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum RecoveryStage {
    Prepared,
    Copied,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RecoveryReceipt {
    pub(crate) source: PathBuf,
    pub(crate) destination: PathBuf,
    pub(crate) source_start: usize,
    pub(crate) source_text: String,
    pub(crate) copied_text: String,
    pub(crate) stage: RecoveryStage,
}

pub(crate) fn resume_recovery(path: &Path) -> Result<RecoveryReceipt, WorkflowError> {
    let mut receipt = load_receipt(path)?;
    if receipt.stage == RecoveryStage::Complete {
        return Ok(receipt);
    }
    let destination = fs::read_to_string(&receipt.destination)?;
    if !destination.contains(&receipt.copied_text) {
        return Err(WorkflowError::SourceChanged);
    }
    if receipt.stage == RecoveryStage::Prepared {
        receipt.stage = RecoveryStage::Copied;
        save_receipt(path, &receipt)?;
    }
    let mut source = fs::read_to_string(&receipt.source)?;
    let end = receipt
        .source_start
        .saturating_add(receipt.source_text.len());
    if source.get(receipt.source_start..end) == Some(receipt.source_text.as_str()) {
        source.replace_range(receipt.source_start..end, "");
        atomic_write(&receipt.source, source.as_bytes())?;
    } else if source.contains(&receipt.source_text) {
        return Err(WorkflowError::SourceChanged);
    }
    receipt.stage = RecoveryStage::Complete;
    save_receipt(path, &receipt)?;
    Ok(receipt)
}

pub(crate) fn cleanup_recovery_duplicate(path: &Path) -> Result<(), WorkflowError> {
    let receipt = load_receipt(path)?;
    let source = fs::read_to_string(&receipt.source)?;
    if !source.contains(&receipt.source_text) {
        return Err(WorkflowError::SourceChanged);
    }
    let mut destination = fs::read_to_string(&receipt.destination)?;
    if let Some(start) = destination.rfind(&receipt.copied_text) {
        destination.replace_range(start..start + receipt.copied_text.len(), "");
        atomic_write(&receipt.destination, destination.as_bytes())?;
    }
    Ok(())
}

pub(crate) fn load_receipt(path: &Path) -> Result<RecoveryReceipt, WorkflowError> {
    serde_json::from_slice(&fs::read(path)?).map_err(|_| WorkflowError::ReceiptCorrupt)
}

pub(super) fn save_receipt(path: &Path, receipt: &RecoveryReceipt) -> Result<(), WorkflowError> {
    let bytes = serde_json::to_vec_pretty(receipt).map_err(|_| WorkflowError::ReceiptCorrupt)?;
    atomic_write(path, &bytes)?;
    Ok(())
}
