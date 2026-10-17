//! SYNC-04g-M1d-c: a non-destructive operator review protocol.
//!
//! It is impossible to infer a safe file mutation from a matching BLAKE3
//! hash. Every disposition here keeps the one-shot publication claim and all
//! original/archived bytes. This module has NO filesystem mutation methods.
//! The host must obtain an explicit review confirmation from future native UX;
//! these internal APIs are not, by themselves, user authentication.

use super::journal::{LocalRecoveryArchivedCopy, LocalRecoveryEvidenceCase};
use super::local_recovery::LocalRecoveryAssessment;
use super::{OperationId, SyncRelativePath, VaultId};
use serde::{Deserialize, Serialize};

pub(crate) const RECOVERY_REVIEW_RETENTION: &str = "indefinite_hold_no_automatic_purge_v1";
pub(crate) const MAX_REVIEW_NOTE_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoveryReviewDisposition {
    /// Continue holding every original, archived copy, SQLite claim and baseline.
    KeepBlocked,
    /// Flag for human investigation. Does NOT authorize an export, retry or move.
    EscalateManualReview,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalRecoveryReviewSnapshot {
    pub operation_id: OperationId,
    pub vault_id: VaultId,
    pub path: SyncRelativePath,
    pub assessment: LocalRecoveryAssessment,
    pub archive_fingerprint: String,
    pub review_count: u32,
    pub latest_event_digest: String,
    pub retention: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct LocalRecoveryReviewEvent {
    pub operation_id: OperationId,
    pub sequence: u32,
    pub archive_fingerprint: String,
    pub disposition: RecoveryReviewDisposition,
    pub note: String,
    pub previous_event_digest: String,
    pub retention: String,
}

pub(crate) fn archive_fingerprint(
    case: &LocalRecoveryEvidenceCase,
    copy: &LocalRecoveryArchivedCopy,
) -> Result<String, serde_json::Error> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"akula:sync:recovery-review:v1\0");
    hasher.update(&serde_json::to_vec(case)?);
    hasher.update(case.operation_id.as_uuid().as_bytes());
    // Domain separated: None and Some(empty) MUST yield different receipts.
    for bytes in [&copy.stage_bytes, &copy.final_bytes] {
        match bytes {
            None => { hasher.update(b"missing\0"); }
            Some(bytes) => {
                hasher.update(b"present\0");
                hasher.update(&(bytes.len() as u64).to_le_bytes());
                hasher.update(blake3::hash(bytes).as_bytes());
            }
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

pub(crate) fn validated_review_note(note: &str) -> Option<String> {
    let trimmed = note.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_REVIEW_NOTE_BYTES
        || trimmed.chars().any(char::is_control)
    {
        return None;
    }
    Some(trimmed.to_string())
}
