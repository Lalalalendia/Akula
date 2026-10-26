//! Stage 03: crash-recoverable, idempotent local sync operation journal.
//!
//! No I/O executor is invoked here. In particular, a persisted "applied"
//! phase is NOT sufficient to advance a baseline: independently verified
//! readback and a committed SQLite transaction are required. Operations
//! which may have been applied before a lost ACK MUST NOT be auto-replayed.
//!
//! This app-data database is deliberately separate from the editor's mutable
//! DocumentStore and from the immutable sync DAG blob cache.

use super::{DocumentId, OperationId, RevisionId, SyncRelativePath, VaultId};
use super::pinned_read::PinnedFileIdentity;
use super::local_recovery::{self, ObservedLocalFile, ReadOnlyLocalPublicationEvidence};
use super::recovery_review::{self, LocalRecoveryReviewEvent, LocalRecoveryReviewSnapshot,
    RecoveryReviewDisposition, RECOVERY_REVIEW_RETENTION};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::path::Path;

const SCHEMA_VERSION: i64 = 8;
mod existing_replace_attempt;
mod sqlite_lifecycle;
mod existing_replace_receipt_candidate;
mod publication_recovery;
mod phase_transitions;
pub(crate) use existing_replace_attempt::ReadOnlyExistingReplaceAttempt;
#[allow(unused_imports)]
pub(crate) use existing_replace_receipt_candidate::InertExistingReplaceReceiptCandidate;


#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalAction {
    /// Create immutable cloud object; never overwrite an existing object.
    UploadImmutable,
    /// Fetch and stage remote bytes; final local write needs a File Gate guard.
    StageDownload,
    /// Publish a verified new immutable head manifest with provider CAS.
    PublishHead,
    /// A logical tombstone proposal; NEVER delete local/remote files here.
    PublishTombstone,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "revision", rename_all = "snake_case")]
pub enum RemoteGuard {
    Absent,
    Matches(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationIntent {
    pub operation_id: OperationId,
    pub vault_id: VaultId,
    pub document_id: DocumentId,
    /// Stable profile binding: changing cloud roots/providers needs explicit migration.
    pub profile_id: String,
    pub revision_id: RevisionId,
    pub action: JournalAction,
    /// Logical vault-relative path; no native absolute paths in the journal.
    pub path: SyncRelativePath,
    /// Opaque provider object ID; never interpreted as a filesystem path.
    pub remote_object_id: String,
    /// Provider-native atomic compare-and-swap guard or create-if-absent.
    /// No implicit overwrite without an explicit precondition.
    pub remote_guard: Option<RemoteGuard>,
    /// BLAKE3 of expected readback bytes, not an S3 ETag.
    pub expected_content_hash: String,
    /// Caller-confirmed previous baseline; None means first sync.
    pub expected_baseline: Option<RevisionId>,
}

impl OperationIntent {
    fn validate(&self) -> JournalResult<()> {
        let hash = &self.expected_content_hash;
        if self.remote_object_id.is_empty()
            || self.remote_object_id.chars().any(char::is_control)
            || self.profile_id.is_empty()
            || self.profile_id.chars().any(char::is_control)
            || hash.len() != 64
            || !hash.bytes().all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
        {
            return Err(JournalError::InvalidIntent);
        }
        match (&self.action, &self.remote_guard) {
            (JournalAction::StageDownload, None) => {}
            (JournalAction::UploadImmutable, Some(RemoteGuard::Absent)) => {}
            (JournalAction::PublishHead | JournalAction::PublishTombstone,
                Some(RemoteGuard::Absent)) => {}
            (JournalAction::PublishHead | JournalAction::PublishTombstone,
                Some(RemoteGuard::Matches(revision))) if !revision.is_empty() => {}
            _ => return Err(JournalError::InvalidIntent),
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalPhase {
    Pending,
    Staged,
    Applied,
    Verified,
    Committed,
}

impl JournalPhase {
    fn db(self) -> i64 {
        match self {
            Self::Pending => 0,
            Self::Staged => 1,
            Self::Applied => 2,
            Self::Verified => 3,
            Self::Committed => 4,
        }
    }

    fn parse(code: i64) -> JournalResult<Self> {
        match code {
            0 => Ok(Self::Pending),
            1 => Ok(Self::Staged),
            2 => Ok(Self::Applied),
            3 => Ok(Self::Verified),
            4 => Ok(Self::Committed),
            _ => Err(JournalError::CorruptedRecord),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalRecord {
    pub intent: OperationIntent,
    pub phase: JournalPhase,
    pub staged_hash: Option<String>,
    pub readback_hash: Option<String>,
    pub provider_revision: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfirmedBaseline {
    pub vault_id: VaultId,
    pub document_id: DocumentId,
    pub profile_id: String,
    pub revision_id: RevisionId,
    pub content_hash: String,
    pub operation_id: OperationId,
}

/// Immutable, diagnostic-only crash observation. Even matching expected
/// BLAKE3 bytes + a remembered file ID are NOT evidence of a durable Windows
/// rename, exclusive provenance or permission to mutate a vault.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct LocalRecoveryEvidenceCase {
    pub operation_id: OperationId,
    pub vault_id: VaultId,
    pub relative_path: SyncRelativePath,
    pub expected_content_hash: String,
    pub evidence: ReadOnlyLocalPublicationEvidence,
}

/// Private app-data only. This is an immutable *copy*, NOT an OS quarantine
/// move or permission to remove the stage/final name in the user vault.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalRecoveryArchivedCopy {
    pub operation_id: OperationId,
    pub stage_bytes: Option<Vec<u8>>,
    pub final_bytes: Option<Vec<u8>>,
}

/// A durable, read-only inspection ticket for an attempted local publication.
/// Never treat it as an authorization to retry, remove, quarantine physically,
/// or confirm a remote baseline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalPublicationReview {
    pub operation_id: OperationId,
    pub vault_id: VaultId,
    pub path: SyncRelativePath,
    pub expected_content_hash: String,
}

/// Immutable app-data report from the successful, uninterrupted Unix
/// stage fsync + atomic no-replace + pinned readback + parent fsync path.
/// Does not prove protection against external writers or grant baseline.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeLocalPublicationObservation {
    pub operation_id: OperationId,
    pub vault_id: VaultId,
    pub path: SyncRelativePath,
    pub expected_content_hash: String,
    pub parent: PinnedFileIdentity,
    pub final_file: PinnedFileIdentity,
    pub byte_length: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryAction {
    /// No remote/local side-effect permitted before staging evidence exists.
    ResumeStaging,
    /// The side effect might have occurred with a lost acknowledgement.
    /// Read back actual bytes and provider revision before choosing anything.
    InspectBeforeRetry,
    /// A verified item can request an idempotent transactional commit.
    /// The journal will still check baseline CAS in the commit transaction.
    RetryCommit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryItem {
    pub record: JournalRecord,
    pub action: RecoveryAction,
}

#[derive(Debug)]
pub enum JournalError {
    Sqlite(rusqlite::Error),
    InvalidJson(serde_json::Error),
    UnsupportedSchema(i64),
    InvalidIntent,
    IntentCollision,
    BusyDocument,
    BaselineChanged,
    ProfileChanged,
    MissingOperation,
    InvalidTransition,
    ReadbackMismatch,
    CorruptedRecord,
    /// An unconfirmed local download cannot be promoted into an authoritative
    /// Applied/Verified/Committed baseline by the generic remote journal API.
    /// A future native durability/provenance gate needs a separate receipt.
    LocalDownloadNeedsNativeReceipt,
    /// Read-only recovery evidence is intentionally write-once. Observing a
    /// different OS file identity cannot replace a previous incident record.
    RecoveryEvidenceAlreadyRecorded,
    /// The first preserved copy is immutable even if a later observation matches.
    RecoveryCopyAlreadyRecorded,
    /// A forensic archive may only be stored in host-selected app-data, not inside the vault.
    UnsafeArchiveLocation,
    /// Missing immutable archive means the incident cannot be reviewed yet.
    MissingRecoveryArchive,
    /// The operator must reconfirm when the displayed archive fingerprint differs.
    StaleRecoveryReview,
    /// Unsupported action or unbounded/control-character note.
    InvalidRecoveryReview,
    /// The durable record already claims this operation or portable target.
    /// Never automatically replay an uncertain local publication.
    PublicationAttemptAlreadyRecorded,
    NativePublicationObservationAlreadyRecorded,
    ExistingReplaceAttemptAlreadyRecorded,
    /// A forensic, non-authoritative V8 receipt candidate is immutable.
    ExistingReplaceReceiptCandidateAlreadyRecorded,
}

impl From<rusqlite::Error> for JournalError {
    fn from(error: rusqlite::Error) -> Self { Self::Sqlite(error) }
}
impl From<serde_json::Error> for JournalError {
    fn from(error: serde_json::Error) -> Self { Self::InvalidJson(error) }
}

pub type JournalResult<T> = Result<T, JournalError>;

pub struct SqliteSyncJournal {
    connection: Connection,
}

impl SqliteSyncJournal {
    pub fn get(&self, id: OperationId) -> JournalResult<Option<JournalRecord>> {
        read_operation(&self.connection, id)
    }

    pub fn baseline(&self, vault: VaultId, document: DocumentId)
        -> JournalResult<Option<ConfirmedBaseline>>
    {
        load_baseline(&self.connection, vault, document)
    }

}

fn read_operation(connection: &Connection, id: OperationId)
    -> JournalResult<Option<JournalRecord>>
{
    let data: Option<(String, String, Vec<u8>, String, i64, Option<String>,
        Option<String>, Option<String>)> = connection.query_row(
        "SELECT vault_id, document_id, intent, digest, phase,
                staged_hash, readback_hash, provider_revision
         FROM sync_operations WHERE operation_id = ?1",
        [id.to_string()],
        |row| Ok((
            row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
            row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?,
        )),
    ).optional()?;
    let Some((vault_id, document_id, payload, digest, phase,
        staged_hash, readback_hash, provider_revision)) = data else {
        return Ok(None);
    };
    if blake3::hash(&payload).to_hex().as_str() != digest {
        return Err(JournalError::CorruptedRecord);
    }
    let intent: OperationIntent = serde_json::from_slice(&payload)?;
    intent.validate()?;
    if intent.operation_id != id || intent.vault_id.to_string() != vault_id
        || intent.document_id.to_string() != document_id
    {
        return Err(JournalError::CorruptedRecord);
    }
    let phase = JournalPhase::parse(phase)?;
    if (phase.db() >= JournalPhase::Staged.db()
            && staged_hash.as_deref() != Some(intent.expected_content_hash.as_str()))
        || (phase.db() >= JournalPhase::Verified.db()
            && (readback_hash.as_deref() != Some(intent.expected_content_hash.as_str())
                || provider_revision.as_deref().is_none_or(str::is_empty)))
    {
        return Err(JournalError::CorruptedRecord);
    }
    Ok(Some(JournalRecord { intent, phase, staged_hash, readback_hash, provider_revision }))
}

fn load_baseline(connection: &Connection, vault: VaultId, document: DocumentId)
    -> JournalResult<Option<ConfirmedBaseline>>
{
    let row: Option<(String, String, String)> = connection.query_row(
        "SELECT revision_id, content_hash, operation_id FROM sync_baselines
         WHERE vault_id = ?1 AND document_id = ?2",
        params![vault.to_string(), document.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional()?;
    let Some((revision, content_hash, operation)) = row else { return Ok(None) };
    let revision_id = revision.parse().map_err(|_| JournalError::CorruptedRecord)?;
    let operation_id = operation.parse().map_err(|_| JournalError::CorruptedRecord)?;
    let recorded = read_operation(connection, operation_id)?
        .ok_or(JournalError::CorruptedRecord)?;
    if recorded.phase != JournalPhase::Committed
        || recorded.intent.revision_id != revision_id
        || recorded.intent.vault_id != vault
        || recorded.intent.document_id != document
        || recorded.intent.expected_content_hash != content_hash
    {
        return Err(JournalError::CorruptedRecord);
    }
    Ok(Some(ConfirmedBaseline { vault_id: vault, document_id: document,
        profile_id: recorded.intent.profile_id,
        revision_id, content_hash, operation_id }))
}

#[cfg(test)]
mod tests;
