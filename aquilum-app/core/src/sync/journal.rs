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
    /// The caller provides a path under app-data, NOT the user vault and NOT
    /// the editor's existing document store.
    pub fn open(path: &Path) -> JournalResult<Self> {
        Self::prepare(Connection::open(path)?)
    }

    /// Never create or migrate a Sync database merely because the user
    /// opened Settings. An older or unknown schema is unavailable, not
    /// evidence that synchronization or recovery is healthy.
    pub(crate) fn open_existing_read_only(path: &Path) -> JournalResult<Self> {
        let connection = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.execute_batch(
            "PRAGMA query_only = ON;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 250;",
        )?;
        let version: i64 = connection.query_row(
            "PRAGMA user_version", [], |row| row.get(0),
        )?;
        if version != SCHEMA_VERSION {
            return Err(JournalError::UnsupportedSchema(version));
        }
        Ok(Self { connection })
    }

    /// Open ONLY an existing current-schema journal for a human-confirmed
    /// non-destructive review event. Never create or migrate on a UI click.
    /// The caller has already validated the host-controlled file location;
    /// cross-process path replacement is not an OS-level security boundary.
    pub(crate) fn open_existing_review_writer(path: &Path) -> JournalResult<Self> {
        let connection = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.execute_batch(
            "PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        let version: i64 = connection.query_row(
            "PRAGMA user_version", [], |row| row.get(0),
        )?;
        if version != SCHEMA_VERSION {
            return Err(JournalError::UnsupportedSchema(version));
        }
        Ok(Self { connection })
    }

    #[cfg(test)]
    fn memory() -> JournalResult<Self> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(connection: Connection) -> JournalResult<Self> {
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        match version {
            0 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_operations (
                    operation_id TEXT PRIMARY KEY NOT NULL,
                    vault_id TEXT NOT NULL,
                    document_id TEXT NOT NULL,
                    intent BLOB NOT NULL,
                    digest TEXT NOT NULL,
                    phase INTEGER NOT NULL CHECK(phase BETWEEN 0 AND 4),
                    staged_hash TEXT,
                    readback_hash TEXT,
                    provider_revision TEXT
                 );
                 CREATE UNIQUE INDEX one_inflight_per_document
                    ON sync_operations(vault_id, document_id) WHERE phase < 4;
                 CREATE TABLE sync_baselines (
                    vault_id TEXT NOT NULL,
                    document_id TEXT NOT NULL,
                    revision_id TEXT NOT NULL,
                    content_hash TEXT NOT NULL,
                    operation_id TEXT NOT NULL,
                    PRIMARY KEY(vault_id, document_id)
                 );
                 CREATE TABLE sync_local_publication_attempts (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_operations(operation_id),
                     vault_id TEXT NOT NULL,
                     path_collision_key TEXT NOT NULL,
                     relative_path TEXT NOT NULL,
                     expected_hash TEXT NOT NULL,
                     intent_digest TEXT NOT NULL,
                     UNIQUE (vault_id, path_collision_key)
                 );
                 CREATE TABLE sync_local_recovery_evidence (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     evidence_json BLOB NOT NULL,
                     evidence_digest TEXT NOT NULL
                 );
                 CREATE TABLE sync_local_recovery_copies (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_recovery_evidence(operation_id),
                     stage_bytes BLOB,
                     stage_hash TEXT,
                     final_bytes BLOB,
                     final_hash TEXT
                 );
                 CREATE TABLE sync_local_recovery_reviews (
                     operation_id TEXT NOT NULL
                         REFERENCES sync_local_recovery_copies(operation_id),
                     sequence INTEGER NOT NULL CHECK(sequence > 0),
                     review_json BLOB NOT NULL,
                     review_digest TEXT NOT NULL,
                     PRIMARY KEY(operation_id, sequence)
                 );
                 CREATE TABLE sync_local_native_publication_observations (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     observation_json BLOB NOT NULL,
                     observation_digest TEXT NOT NULL
                 );
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?,
            1 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_local_publication_attempts (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_operations(operation_id),
                     vault_id TEXT NOT NULL,
                     path_collision_key TEXT NOT NULL,
                     relative_path TEXT NOT NULL,
                     expected_hash TEXT NOT NULL,
                     intent_digest TEXT NOT NULL,
                     UNIQUE (vault_id, path_collision_key)
                 );
                 CREATE TABLE sync_local_recovery_evidence (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     evidence_json BLOB NOT NULL,
                     evidence_digest TEXT NOT NULL
                 );
                 CREATE TABLE sync_local_recovery_copies (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_recovery_evidence(operation_id),
                     stage_bytes BLOB,
                     stage_hash TEXT,
                     final_bytes BLOB,
                     final_hash TEXT
                 );
                 CREATE TABLE sync_local_recovery_reviews (
                     operation_id TEXT NOT NULL
                         REFERENCES sync_local_recovery_copies(operation_id),
                     sequence INTEGER NOT NULL CHECK(sequence > 0),
                     review_json BLOB NOT NULL,
                     review_digest TEXT NOT NULL,
                     PRIMARY KEY(operation_id, sequence)
                 );
                 CREATE TABLE sync_local_native_publication_observations (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     observation_json BLOB NOT NULL,
                     observation_digest TEXT NOT NULL
                 );
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?,
            2 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_local_recovery_evidence (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     evidence_json BLOB NOT NULL,
                     evidence_digest TEXT NOT NULL
                 );
                 CREATE TABLE sync_local_recovery_copies (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_recovery_evidence(operation_id),
                     stage_bytes BLOB,
                     stage_hash TEXT,
                     final_bytes BLOB,
                     final_hash TEXT
                 );
                 CREATE TABLE sync_local_recovery_reviews (
                     operation_id TEXT NOT NULL
                         REFERENCES sync_local_recovery_copies(operation_id),
                     sequence INTEGER NOT NULL CHECK(sequence > 0),
                     review_json BLOB NOT NULL,
                     review_digest TEXT NOT NULL,
                     PRIMARY KEY(operation_id, sequence)
                 );
                 CREATE TABLE sync_local_native_publication_observations (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     observation_json BLOB NOT NULL,
                     observation_digest TEXT NOT NULL
                 );
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?,
            3 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_local_recovery_copies (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_recovery_evidence(operation_id),
                     stage_bytes BLOB,
                     stage_hash TEXT,
                     final_bytes BLOB,
                     final_hash TEXT
                 );
                 CREATE TABLE sync_local_recovery_reviews (
                     operation_id TEXT NOT NULL
                         REFERENCES sync_local_recovery_copies(operation_id),
                     sequence INTEGER NOT NULL CHECK(sequence > 0),
                     review_json BLOB NOT NULL,
                     review_digest TEXT NOT NULL,
                     PRIMARY KEY(operation_id, sequence)
                 );
                 CREATE TABLE sync_local_native_publication_observations (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     observation_json BLOB NOT NULL,
                     observation_digest TEXT NOT NULL
                 );
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?,
            4 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_local_recovery_reviews (
                     operation_id TEXT NOT NULL
                         REFERENCES sync_local_recovery_copies(operation_id),
                     sequence INTEGER NOT NULL CHECK(sequence > 0),
                     review_json BLOB NOT NULL,
                     review_digest TEXT NOT NULL,
                     PRIMARY KEY(operation_id, sequence)
                 );
                 CREATE TABLE sync_local_native_publication_observations (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     observation_json BLOB NOT NULL,
                     observation_digest TEXT NOT NULL
                 );
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?,
            5 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_local_native_publication_observations (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_local_publication_attempts(operation_id),
                     observation_json BLOB NOT NULL,
                     observation_digest TEXT NOT NULL
                 );
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?,
            6 | 7 | SCHEMA_VERSION => {},
            other => return Err(JournalError::UnsupportedSchema(other)),
        }
        // Older migrations still reach v6. Upgrade atomically on the next
        // writer-open; read-only Settings never creates or migrates tables.
        if version < 7 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_existing_replace_attempts (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_operations(operation_id),
                     vault_id TEXT NOT NULL,
                     document_id TEXT NOT NULL,
                     path_collision_key TEXT NOT NULL,
                     baseline_revision TEXT NOT NULL,
                     evidence_json BLOB NOT NULL,
                     evidence_digest TEXT NOT NULL,
                     UNIQUE(vault_id,path_collision_key,baseline_revision)
                 );
                 PRAGMA user_version = 7;
                 COMMIT;",
            )?;
        }
        // V8 is an additive, write-once *forensic candidate*, not an S3
        // permission or cross-filesystem atomic transaction. A read-only
        // connection never creates or migrates this table.
        if version < 8 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE sync_existing_replace_receipt_candidates (
                     operation_id TEXT PRIMARY KEY NOT NULL
                         REFERENCES sync_existing_replace_attempts(operation_id),
                     candidate_json BLOB NOT NULL,
                     candidate_digest TEXT NOT NULL
                 );
                 PRAGMA user_version = 8;
                 COMMIT;",
            )?;
        }
        Ok(Self { connection })
    }

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
mod tests {
    use super::*;

    fn intent(vault: VaultId, doc: DocumentId, revision: RevisionId,
        parent: Option<RevisionId>, bytes: &[u8]) -> OperationIntent {
        OperationIntent {
            operation_id: OperationId::new(),
            vault_id: vault,
            document_id: doc,
            profile_id: "s3-profile-1".into(),
            revision_id: revision,
            action: JournalAction::UploadImmutable,
            path: SyncRelativePath::parse("notes/demo.md").unwrap(),
            remote_object_id: format!("immutable/{}", revision),
            remote_guard: Some(RemoteGuard::Absent),
            expected_content_hash: blake3::hash(bytes).to_hex().to_string(),
            expected_baseline: parent,
        }
    }

    #[test]
    fn commit_requires_staging_application_verified_readback_and_atomic_baseline() {
        let mut j = SqliteSyncJournal::memory().unwrap();
        let vault = VaultId::new();
        let doc = DocumentId::new();
        let rev = RevisionId::new();
        let op = intent(vault, doc, rev, None, b"contents");
        let id = op.operation_id;
        assert_eq!(j.begin(op.clone()).unwrap(), JournalPhase::Pending);
        assert!(j.baseline(vault, doc).unwrap().is_none());
        assert!(matches!(j.commit(id), Err(JournalError::InvalidTransition)));
        assert_eq!(j.mark_staged(id, b"contents").unwrap(), JournalPhase::Staged);
        assert!(matches!(j.commit(id), Err(JournalError::InvalidTransition)));
        assert_eq!(j.mark_applied(id).unwrap(), JournalPhase::Applied);
        assert!(matches!(j.mark_verified(id, b"corruption", "provider:1"),
            Err(JournalError::ReadbackMismatch)));
        assert!(j.baseline(vault, doc).unwrap().is_none());
        assert_eq!(j.mark_verified(id, b"contents", "provider:1").unwrap(),
            JournalPhase::Verified);
        assert!(j.baseline(vault, doc).unwrap().is_none());
        assert_eq!(j.commit(id).unwrap(), JournalPhase::Committed);
        assert_eq!(j.commit(id).unwrap(), JournalPhase::Committed);
        assert_eq!(j.baseline(vault, doc).unwrap().unwrap().revision_id, rev);
        assert!(j.recovery_queue().unwrap().is_empty());
    }

    #[test]
    fn duplicate_operation_is_idempotent_and_uuid_collision_rejected() {
        let mut j = SqliteSyncJournal::memory().unwrap();
        let vault = VaultId::new();
        let doc = DocumentId::new();
        let rev = RevisionId::new();
        let op = intent(vault, doc, rev, None, b"contents");
        assert_eq!(j.begin(op.clone()).unwrap(), JournalPhase::Pending);
        assert_eq!(j.begin(op.clone()).unwrap(), JournalPhase::Pending);
        let mut tampered = op;
        tampered.remote_object_id = "another-object".into();
        assert!(matches!(j.begin(tampered), Err(JournalError::IntentCollision)));
    }

    #[test]
    fn two_workers_cannot_commit_simultaneously_to_one_document() {
        let mut j = SqliteSyncJournal::memory().unwrap();
        let vault = VaultId::new();
        let doc = DocumentId::new();
        let first = intent(vault, doc, RevisionId::new(), None, b"first");
        let second = intent(vault, doc, RevisionId::new(), None, b"second");
        assert_eq!(j.begin(first.clone()).unwrap(), JournalPhase::Pending);
        assert!(matches!(j.begin(second.clone()), Err(JournalError::BusyDocument)));
        j.mark_staged(first.operation_id, b"first").unwrap();
        j.mark_applied(first.operation_id).unwrap();
        j.mark_verified(first.operation_id, b"first", "rev-1").unwrap();
        j.commit(first.operation_id).unwrap();
        assert!(matches!(j.begin(second.clone()), Err(JournalError::BaselineChanged)));

        let third = intent(vault, doc, RevisionId::new(),
            Some(first.revision_id), b"third");
        assert_eq!(j.begin(third).unwrap(), JournalPhase::Pending);
    }

    #[test]
    fn process_restarts_never_auto_reapply_uncertain_side_effects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync-actions.sqlite");
        let vault = VaultId::new();
        let doc = DocumentId::new();
        let a = intent(vault, doc, RevisionId::new(), None, b"a");
        let b = intent(vault, DocumentId::new(), RevisionId::new(), None, b"b");
        let c = intent(vault, DocumentId::new(), RevisionId::new(), None, b"c");
        let d = intent(vault, DocumentId::new(), RevisionId::new(), None, b"d");
        {
            let mut j = SqliteSyncJournal::open(&path).unwrap();
            j.begin(a.clone()).unwrap();
            j.begin(b.clone()).unwrap();
            j.begin(c.clone()).unwrap();
            j.begin(d.clone()).unwrap();
            j.mark_staged(b.operation_id, b"b").unwrap();
            j.mark_staged(c.operation_id, b"c").unwrap();
            j.mark_applied(c.operation_id).unwrap();
            j.mark_staged(d.operation_id, b"d").unwrap();
            j.mark_applied(d.operation_id).unwrap();
            j.mark_verified(d.operation_id, b"d", "rev-d").unwrap();
        }
        let j = SqliteSyncJournal::open(&path).unwrap();
        let recovery = j.recovery_queue().unwrap();
        assert_eq!(recovery.len(), 4);
        assert_eq!(recovery.iter().find(|x| x.record.intent.operation_id == a.operation_id)
            .unwrap().action, RecoveryAction::ResumeStaging);
        assert_eq!(recovery.iter().find(|x| x.record.intent.operation_id == b.operation_id)
            .unwrap().action, RecoveryAction::InspectBeforeRetry);
        assert_eq!(recovery.iter().find(|x| x.record.intent.operation_id == c.operation_id)
            .unwrap().action, RecoveryAction::InspectBeforeRetry);
        assert_eq!(recovery.iter().find(|x| x.record.intent.operation_id == d.operation_id)
            .unwrap().action, RecoveryAction::RetryCommit);
        assert!(j.baseline(vault, doc).unwrap().is_none());
    }

    #[test]
    fn sqlite_failure_rolls_back_baseline_and_final_state_together() {
        let mut j = SqliteSyncJournal::memory().unwrap();
        let vault = VaultId::new();
        let doc = DocumentId::new();
        let op = intent(vault, doc, RevisionId::new(), None, b"verify");
        j.begin(op.clone()).unwrap();
        j.mark_staged(op.operation_id, b"verify").unwrap();
        j.mark_applied(op.operation_id).unwrap();
        j.mark_verified(op.operation_id, b"verify", "provider:1").unwrap();
        j.connection.execute_batch(
            "CREATE TRIGGER simulated_commit_failure BEFORE UPDATE ON sync_operations
             WHEN NEW.phase = 4 BEGIN SELECT RAISE(FAIL, 'power-loss simulation'); END;",
        ).unwrap();
        assert!(matches!(j.commit(op.operation_id), Err(JournalError::Sqlite(_))));
        assert!(j.baseline(vault, doc).unwrap().is_none());
        assert_eq!(j.get(op.operation_id).unwrap().unwrap().phase, JournalPhase::Verified);
        j.connection.execute_batch("DROP TRIGGER simulated_commit_failure;").unwrap();
        assert_eq!(j.commit(op.operation_id).unwrap(), JournalPhase::Committed);
        assert_eq!(j.baseline(vault, doc).unwrap().unwrap().revision_id, op.revision_id);
    }

    #[test]
    fn sqlite_corrupted_intent_or_baseline_fails_closed() {
        let mut j = SqliteSyncJournal::memory().unwrap();
        let vault = VaultId::new();
        let doc = DocumentId::new();
        let op = intent(vault, doc, RevisionId::new(), None, b"original");
        j.begin(op.clone()).unwrap();
        j.connection.execute("UPDATE sync_operations SET intent = ?1 WHERE operation_id = ?2",
            params![b"broken".as_slice(), op.operation_id.to_string()]).unwrap();
        assert!(matches!(j.get(op.operation_id), Err(JournalError::CorruptedRecord)));
    }

    #[test]
    fn wrong_hash_and_illegal_phase_transitions_never_advance_state() {
        let mut j = SqliteSyncJournal::memory().unwrap();
        let op = intent(VaultId::new(), DocumentId::new(),
            RevisionId::new(), None, b"valid");
        j.begin(op.clone()).unwrap();
        assert!(matches!(j.mark_staged(op.operation_id, b"invalid"),
            Err(JournalError::ReadbackMismatch)));
        assert!(matches!(j.mark_applied(op.operation_id), Err(JournalError::InvalidTransition)));
        assert!(matches!(j.mark_verified(op.operation_id, b"valid", "etag"),
            Err(JournalError::InvalidTransition)));
        assert_eq!(j.get(op.operation_id).unwrap().unwrap().phase, JournalPhase::Pending);
    }

    #[test]
    fn implicit_remote_overwrite_and_profile_changes_fail_closed() {
        let mut j = SqliteSyncJournal::memory().unwrap();
        let vault = VaultId::new();
        let document = DocumentId::new();
        let mut bad = intent(vault, document, RevisionId::new(), None, b"data");
        bad.remote_guard = None;
        assert!(matches!(j.begin(bad.clone()), Err(JournalError::InvalidIntent)));
        bad.remote_guard = Some(RemoteGuard::Matches("etag-1".into()));
        assert!(matches!(j.begin(bad), Err(JournalError::InvalidIntent)));
        let mut first = intent(vault, document, RevisionId::new(), None, b"one");
        first.action = JournalAction::PublishHead;
        first.remote_guard = Some(RemoteGuard::Matches("etag-1".into()));
        j.begin(first.clone()).unwrap();
        j.mark_staged(first.operation_id, b"one").unwrap();
        j.mark_applied(first.operation_id).unwrap();
        j.mark_verified(first.operation_id, b"one", "etag-2").unwrap();
        j.commit(first.operation_id).unwrap();
        let mut different_profile =
            intent(vault, document, RevisionId::new(), Some(first.revision_id), b"two");
        different_profile.profile_id = "google-drive-different-root".into();
        assert!(matches!(j.begin(different_profile), Err(JournalError::ProfileChanged)));
    }

    fn local_stage_intent(vault: VaultId, document: DocumentId, path: &str, bytes: &[u8])
        -> OperationIntent
    {
        let mut item = intent(vault, document, RevisionId::new(), None, bytes);
        item.action = JournalAction::StageDownload;
        item.remote_guard = None;
        item.path = SyncRelativePath::parse(path).unwrap();
        item
    }

    #[test]
    fn local_publication_claim_is_once_per_operation_and_portable_name() {
        let mut journal = SqliteSyncJournal::memory().unwrap();
        let vault = VaultId::new();
        let first = local_stage_intent(vault, DocumentId::new(), "notes/Café.md", b"one");
        let second = local_stage_intent(vault, DocumentId::new(),
            "notes/cafe\u{301}.md", b"two");
        journal.begin(first.clone()).unwrap();
        journal.begin(second.clone()).unwrap();
        journal.mark_staged(first.operation_id, b"one").unwrap();
        journal.mark_staged(second.operation_id, b"two").unwrap();

        journal.record_local_publication_attempt_once(&first).unwrap();
        assert!(matches!(
            journal.record_local_publication_attempt_once(&first),
            Err(JournalError::PublicationAttemptAlreadyRecorded)
        ));
        assert!(matches!(
            journal.record_local_publication_attempt_once(&second),
            Err(JournalError::PublicationAttemptAlreadyRecorded)
        ));
        let queue = journal.local_publication_review_queue().unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].operation_id, first.operation_id);
        assert_eq!(queue[0].path.as_str(), "notes/Café.md");
        assert!(journal.baseline(vault, first.document_id).unwrap().is_none());
        assert!(journal.baseline(vault, second.document_id).unwrap().is_none());
    }

    #[test]
    fn local_publication_claim_requires_a_verified_staged_intent() {
        let mut journal = SqliteSyncJournal::memory().unwrap();
        let staged = local_stage_intent(VaultId::new(), DocumentId::new(),
            "notes/new.md", b"verified");
        assert!(matches!(
            journal.record_local_publication_attempt_once(&staged),
            Err(JournalError::MissingOperation)
        ));
        journal.begin(staged.clone()).unwrap();
        assert!(matches!(
            journal.record_local_publication_attempt_once(&staged),
            Err(JournalError::InvalidTransition)
        ));
        journal.mark_staged(staged.operation_id, b"verified").unwrap();
        let mut collision = staged.clone();
        collision.expected_content_hash = blake3::hash(b"other").to_hex().to_string();
        assert!(matches!(
            journal.record_local_publication_attempt_once(&collision),
            Err(JournalError::IntentCollision)
        ));
        journal.record_local_publication_attempt_once(&staged).unwrap();
    }

    #[test]
    fn legacy_schema_one_migrates_without_losing_staged_operations() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sync.sqlite");
        let staged = local_stage_intent(VaultId::new(), DocumentId::new(),
            "notes/legacy.md", b"persist");
        {
            let mut journal = SqliteSyncJournal::open(&path).unwrap();
            journal.begin(staged.clone()).unwrap();
            journal.mark_staged(staged.operation_id, b"persist").unwrap();
            // Emulate the exact existing V1 tables/data, without the new table.
            journal.connection.execute_batch(
                "DROP TABLE sync_local_native_publication_observations;
                 DROP TABLE sync_local_recovery_reviews;
                 DROP TABLE sync_local_recovery_copies;
                 DROP TABLE sync_local_recovery_evidence;
                 DROP TABLE sync_local_publication_attempts;
                 DROP TABLE IF EXISTS sync_existing_replace_receipt_candidates;
                 DROP TABLE IF EXISTS sync_existing_replace_attempts;
                  PRAGMA user_version = 1;",
            ).unwrap();
        }
        let migrated = SqliteSyncJournal::open(&path).unwrap();
        let version: i64 = migrated.connection
            .query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(migrated.get(staged.operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged);
        migrated.record_local_publication_attempt_once(&staged).unwrap();
        drop(migrated);
        let reopened = SqliteSyncJournal::open(&path).unwrap();
        assert!(matches!(
            reopened.record_local_publication_attempt_once(&staged),
            Err(JournalError::PublicationAttemptAlreadyRecorded)
        ));
        let queue = reopened.local_publication_review_queue().unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].expected_content_hash, staged.expected_content_hash);
        assert!(reopened.baseline(staged.vault_id, staged.document_id)
            .unwrap().is_none());
    }

    #[test]
    fn local_publication_ledger_corruption_fails_closed() {
        let mut journal = SqliteSyncJournal::memory().unwrap();
        let staged = local_stage_intent(VaultId::new(), DocumentId::new(),
            "notes/new.md", b"payload");
        journal.begin(staged.clone()).unwrap();
        journal.mark_staged(staged.operation_id, b"payload").unwrap();
        journal.record_local_publication_attempt_once(&staged).unwrap();
        journal.connection.execute(
            "UPDATE sync_local_publication_attempts
             SET expected_hash = 'corrupted' WHERE operation_id = ?1",
            [staged.operation_id.to_string()],
        ).unwrap();
        assert!(matches!(
            journal.local_publication_review_queue(),
            Err(JournalError::CorruptedRecord)
        ));
    }

    #[test]
    fn separate_sqlite_connections_cannot_claim_the_same_local_publication() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sync.sqlite");
        let staged = local_stage_intent(VaultId::new(), DocumentId::new(),
            "notes/two-processes.md", b"claim");
        {
            let mut journal = SqliteSyncJournal::open(&path).unwrap();
            journal.begin(staged.clone()).unwrap();
            journal.mark_staged(staged.operation_id, b"claim").unwrap();
        }
        let first = SqliteSyncJournal::open(&path).unwrap();
        let second = SqliteSyncJournal::open(&path).unwrap();
        first.record_local_publication_attempt_once(&staged).unwrap();
        assert!(matches!(
            second.record_local_publication_attempt_once(&staged),
            Err(JournalError::PublicationAttemptAlreadyRecorded)
        ));
        assert_eq!(second.local_publication_review_queue().unwrap().len(), 1);
    }

    #[test]
    fn m1d_local_download_cannot_promote_to_confirmed_baseline() {
        let mut journal = SqliteSyncJournal::memory().unwrap();
        let vault = VaultId::new();
        let document = DocumentId::new();
        let staged = local_stage_intent(vault, document, "notes/remote.md", b"payload");
        journal.begin(staged.clone()).unwrap();
        journal.mark_staged(staged.operation_id, b"payload").unwrap();
        journal.record_local_publication_attempt_once(&staged).unwrap();
        assert!(matches!(
            journal.mark_applied(staged.operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)
        ));
        assert!(matches!(
            journal.mark_verified(staged.operation_id, b"payload", "fake-rev"),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)
        ));
        assert!(matches!(
            journal.commit(staged.operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)
        ));
        assert_eq!(journal.get(staged.operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged);
        assert!(journal.baseline(vault, document).unwrap().is_none());

        // Older or damaged databases might already contain a false Verified
        // state. The generic commit API must STILL refuse and the recovery
        // queue must not describe it as a safe RetryCommit.
        journal.connection.execute(
            "UPDATE sync_operations
             SET phase = 3, readback_hash = ?1, provider_revision = 'obsolete'
             WHERE operation_id = ?2",
            params![staged.expected_content_hash, staged.operation_id.to_string()],
        ).unwrap();
        assert!(matches!(
            journal.commit(staged.operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)
        ));
        assert_eq!(journal.recovery_queue().unwrap()[0].action,
            RecoveryAction::InspectBeforeRetry);
        assert!(journal.baseline(vault, document).unwrap().is_none());
    }

    fn m1d_test_evidence(staged: &OperationIntent) -> LocalRecoveryEvidenceCase {
        LocalRecoveryEvidenceCase {
            operation_id: staged.operation_id,
            vault_id: staged.vault_id,
            relative_path: staged.path.clone(),
            expected_content_hash: staged.expected_content_hash.clone(),
            evidence: ReadOnlyLocalPublicationEvidence {
                operation_id: staged.operation_id,
                parent_identity: super::super::PinnedFileIdentity { device: 1, file: 123 },
                stage: ObservedLocalFile::Present {
                    identity: super::super::PinnedFileIdentity { device: 1, file: 456 },
                    byte_length: 7,
                    matches_expected_hash: true,
                },
                final_name: ObservedLocalFile::Absent,
                assessment: local_recovery::LocalRecoveryAssessment::StagedOnly,
            },
        }
    }

    #[test]
    fn m1d_recovery_evidence_is_write_once_and_corruption_checked_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sync.sqlite");
        let staged = local_stage_intent(VaultId::new(), DocumentId::new(),
            "notes/recover.md", b"payload");
        let original = m1d_test_evidence(&staged);
        {
            let mut journal = SqliteSyncJournal::open(&path).unwrap();
            journal.begin(staged.clone()).unwrap();
            journal.mark_staged(staged.operation_id, b"payload").unwrap();
            journal.record_local_publication_attempt_once(&staged).unwrap();
            journal.record_local_recovery_case_once(&original).unwrap();
        }
        let reopened = SqliteSyncJournal::open(&path).unwrap();
        assert_eq!(reopened.read_local_recovery_case(staged.operation_id).unwrap(),
            Some(original.clone()));
        assert!(matches!(
            reopened.record_local_recovery_case_once(&original),
            Err(JournalError::RecoveryEvidenceAlreadyRecorded)
        ));
        let mut changed = original.clone();
        changed.evidence.stage = ObservedLocalFile::Absent;
        changed.evidence.assessment = local_recovery::LocalRecoveryAssessment::NoVisibleNames;
        assert!(matches!(
            reopened.record_local_recovery_case_once(&changed),
            Err(JournalError::RecoveryEvidenceAlreadyRecorded)
        ));
        assert_eq!(reopened.read_local_recovery_case(staged.operation_id).unwrap(),
            Some(original));
        reopened.connection.execute(
            "UPDATE sync_local_recovery_evidence SET evidence_digest='tampered'
             WHERE operation_id = ?1",
            [staged.operation_id.to_string()],
        ).unwrap();
        assert!(matches!(
            reopened.read_local_recovery_case(staged.operation_id),
            Err(JournalError::CorruptedRecord)
        ));
        assert!(reopened.baseline(staged.vault_id, staged.document_id)
            .unwrap().is_none());
    }

    #[test]
    fn m1d_recovery_evidence_requires_consistent_classification_and_ticket() {
        let mut journal = SqliteSyncJournal::memory().unwrap();
        let staged = local_stage_intent(VaultId::new(), DocumentId::new(),
            "notes/recover.md", b"payload");
        journal.begin(staged.clone()).unwrap();
        journal.mark_staged(staged.operation_id, b"payload").unwrap();
        let evidence = m1d_test_evidence(&staged);
        assert!(matches!(
            journal.record_local_recovery_case_once(&evidence),
            Err(JournalError::CorruptedRecord)
        ));
        journal.record_local_publication_attempt_once(&staged).unwrap();
        let mut forged = evidence.clone();
        forged.evidence.assessment = local_recovery::LocalRecoveryAssessment::NoVisibleNames;
        assert!(matches!(
            journal.record_local_recovery_case_once(&forged),
            Err(JournalError::CorruptedRecord)
        ));
        journal.record_local_recovery_case_once(&evidence).unwrap();
        assert!(journal.read_local_recovery_case(staged.operation_id).unwrap().is_some());
        assert!(journal.baseline(staged.vault_id, staged.document_id)
            .unwrap().is_none());
    }

    #[test]
    fn m1d_schema_two_migrates_without_losing_original_publication_claims() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync.sqlite");
        let staged = local_stage_intent(VaultId::new(), DocumentId::new(),
            "notes/old-claim.md", b"payload");
        {
            let mut j = SqliteSyncJournal::open(&path).unwrap();
            j.begin(staged.clone()).unwrap();
            j.mark_staged(staged.operation_id, b"payload").unwrap();
            j.record_local_publication_attempt_once(&staged).unwrap();
            j.connection.execute_batch(
                "DROP TABLE sync_local_native_publication_observations;
                 DROP TABLE sync_local_recovery_reviews;
                 DROP TABLE sync_local_recovery_copies;
                 DROP TABLE sync_local_recovery_evidence;
                 DROP TABLE IF EXISTS sync_existing_replace_receipt_candidates;
                 DROP TABLE IF EXISTS sync_existing_replace_attempts;
                  PRAGMA user_version = 2;",
            ).unwrap();
        }
        let upgraded = SqliteSyncJournal::open(&path).unwrap();
        assert_eq!(
            upgraded.connection.query_row::<i64,_,_>(
                "PRAGMA user_version", [], |row| row.get(0)
            ).unwrap(),
            SCHEMA_VERSION
        );
        assert_eq!(upgraded.local_publication_review_queue().unwrap().len(), 1);
        assert!(matches!(
            upgraded.record_local_publication_attempt_once(&staged),
            Err(JournalError::PublicationAttemptAlreadyRecorded)
        ));
        let evidence = m1d_test_evidence(&staged);
        upgraded.record_local_recovery_case_once(&evidence).unwrap();
        assert_eq!(upgraded.read_local_recovery_case(staged.operation_id).unwrap(),
            Some(evidence));
        assert!(upgraded.baseline(staged.vault_id, staged.document_id)
            .unwrap().is_none());
    }

    #[test]
    fn m1db_copy_is_write_once_and_tampering_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("sync.sqlite");
        let staged = local_stage_intent(
            VaultId::new(), DocumentId::new(), "notes/stage.md", b"payload",
        );
        let historical = m1d_test_evidence(&staged);
        {
            let mut j = SqliteSyncJournal::open(&db).unwrap();
            j.begin(staged.clone()).unwrap();
            j.mark_staged(staged.operation_id, b"payload").unwrap();
            j.record_local_publication_attempt_once(&staged).unwrap();
            j.record_local_recovery_case_once(&historical).unwrap();
            let archived = j.record_local_recovery_copy_once(
                staged.operation_id, Some(b"payload"), None,
            ).unwrap();
            assert_eq!(archived.stage_bytes.as_deref(), Some(&b"payload"[..]));
            assert_eq!(archived.final_bytes, None);
            assert!(matches!(
                j.record_local_recovery_copy_once(staged.operation_id, Some(b"payload"), None),
                Err(JournalError::RecoveryCopyAlreadyRecorded)
            ));
            assert!(matches!(
                j.record_local_recovery_copy_once(staged.operation_id, Some(b"changed"), None),
                Err(JournalError::CorruptedRecord)
            ));
        }
        let j = SqliteSyncJournal::open(&db).unwrap();
        assert_eq!(
            j.read_local_recovery_copy(staged.operation_id).unwrap().unwrap()
                .stage_bytes.as_deref(),
            Some(&b"payload"[..]),
        );
        j.connection.execute(
            "UPDATE sync_local_recovery_copies SET stage_bytes = ?1 WHERE operation_id = ?2",
            params![b"corrupt".as_slice(), staged.operation_id.to_string()],
        ).unwrap();
        assert!(matches!(
            j.read_local_recovery_copy(staged.operation_id),
            Err(JournalError::CorruptedRecord)
        ));
        assert!(j.baseline(staged.vault_id, staged.document_id).unwrap().is_none());
    }

    #[test]
    fn m1db_archive_requires_matching_historical_observation() {
        let mut j = SqliteSyncJournal::memory().unwrap();
        let staged = local_stage_intent(
            VaultId::new(), DocumentId::new(), "notes/stage.md", b"payload",
        );
        j.begin(staged.clone()).unwrap();
        j.mark_staged(staged.operation_id, b"payload").unwrap();
        j.record_local_publication_attempt_once(&staged).unwrap();
        assert!(matches!(
            j.record_local_recovery_copy_once(staged.operation_id, Some(b"payload"), None),
            Err(JournalError::CorruptedRecord)
        ));
        j.record_local_recovery_case_once(&m1d_test_evidence(&staged)).unwrap();
        for (stage, final_name) in [
            (None, None),
            (Some(&b"payload"[..]), Some(&b"unexpected"[..])),
            (Some(&b"wrong"[..]), None),
        ] {
            assert!(matches!(
                j.record_local_recovery_copy_once(staged.operation_id, stage, final_name),
                Err(JournalError::CorruptedRecord)
            ));
        }
        assert!(j.read_local_recovery_copy(staged.operation_id).unwrap().is_none());
    }

    #[test]
    fn m1db_schema_three_migrates_without_dropping_incident_or_claim() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("sync.sqlite");
        let staged = local_stage_intent(
            VaultId::new(), DocumentId::new(), "notes/stage.md", b"payload",
        );
        let case = m1d_test_evidence(&staged);
        {
            let mut journal = SqliteSyncJournal::open(&db).unwrap();
            journal.begin(staged.clone()).unwrap();
            journal.mark_staged(staged.operation_id, b"payload").unwrap();
            journal.record_local_publication_attempt_once(&staged).unwrap();
            journal.record_local_recovery_case_once(&case).unwrap();
            journal.connection.execute_batch(
                "DROP TABLE sync_local_native_publication_observations;
                 DROP TABLE sync_local_recovery_reviews;
                 DROP TABLE sync_local_recovery_copies;
                 DROP TABLE IF EXISTS sync_existing_replace_receipt_candidates;
                 DROP TABLE IF EXISTS sync_existing_replace_attempts;
                  PRAGMA user_version = 3;",
            ).unwrap();
        }
        let journal = SqliteSyncJournal::open(&db).unwrap();
        let version: i64 = journal.connection.query_row(
            "PRAGMA user_version", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(journal.read_local_recovery_case(staged.operation_id).unwrap(),
            Some(case));
        assert_eq!(journal.local_publication_review_queue().unwrap().len(), 1);
        assert!(matches!(
            journal.record_local_publication_attempt_once(&staged),
            Err(JournalError::PublicationAttemptAlreadyRecorded)
        ));
        assert!(journal.read_local_recovery_copy(staged.operation_id).unwrap().is_none());
        journal.record_local_recovery_copy_once(
            staged.operation_id, Some(b"payload"), None,
        ).unwrap();
        assert!(journal.read_local_recovery_copy(staged.operation_id).unwrap().is_some());
        assert!(journal.baseline(staged.vault_id, staged.document_id).unwrap().is_none());
    }

    fn m1dc_fixture(db: &Path) -> (SqliteSyncJournal, OperationIntent) {
        let mut journal = SqliteSyncJournal::open(db).unwrap();
        let intent = local_stage_intent(VaultId::new(), DocumentId::new(),
            "notes/crash-review.md", b"payload");
        journal.begin(intent.clone()).unwrap();
        journal.mark_staged(intent.operation_id, b"payload").unwrap();
        journal.record_local_publication_attempt_once(&intent).unwrap();
        journal.record_local_recovery_case_once(&m1d_test_evidence(&intent)).unwrap();
        journal.record_local_recovery_copy_once(
            intent.operation_id, Some(b"payload"), None,
        ).unwrap();
        (journal, intent)
    }

    #[test]
    fn m1dc_schema_four_upgrades_preserving_archive_and_review_eligibility() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("sync.sqlite");
        let (journal, intent) = m1dc_fixture(&db);
        journal.connection.execute_batch(
            "DROP TABLE sync_local_native_publication_observations;
                 DROP TABLE sync_local_recovery_reviews;
             DROP TABLE IF EXISTS sync_existing_replace_receipt_candidates;
                 DROP TABLE IF EXISTS sync_existing_replace_attempts;
                  PRAGMA user_version = 4;",
        ).unwrap();
        drop(journal);
        let mut upgraded = SqliteSyncJournal::open(&db).unwrap();
        let version: i64 = upgraded.connection.query_row(
            "PRAGMA user_version", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(upgraded.read_local_recovery_case(intent.operation_id).unwrap()
            .unwrap().operation_id, intent.operation_id);
        assert_eq!(upgraded.read_local_recovery_copy(intent.operation_id).unwrap()
            .unwrap().stage_bytes, Some(b"payload".to_vec()));
        assert_eq!(upgraded.local_publication_review_queue().unwrap().len(), 1);
        let screen = upgraded.recovery_review_snapshot(intent.operation_id).unwrap();
        assert_eq!(screen.review_count, 0);
        let event = upgraded.append_recovery_review(
            intent.operation_id, intent.vault_id, &intent.path,
            &screen.archive_fingerprint, 0, "genesis",
            RecoveryReviewDisposition::KeepBlocked, "Hold forensic evidence",
        ).unwrap();
        assert_eq!(event.sequence, 1);
        assert!(upgraded.baseline(intent.vault_id, intent.document_id).unwrap().is_none());
        drop(upgraded);
        let opened = SqliteSyncJournal::open(&db).unwrap();
        assert_eq!(opened.recovery_review_history(intent.operation_id).unwrap(), vec![event]);
        assert!(opened.baseline(intent.vault_id, intent.document_id).unwrap().is_none());
        assert!(matches!(
            opened.get(intent.operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged
        ));
    }

    #[test]
    fn m1dc_two_connections_refuse_stale_revision_and_preserve_chain() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sync.sqlite");
        let (first, intent) = m1dc_fixture(&db);
        drop(first);
        let mut one = SqliteSyncJournal::open(&db).unwrap();
        let mut two = SqliteSyncJournal::open(&db).unwrap();
        let screen = one.recovery_review_snapshot(intent.operation_id).unwrap();
        let first_event = one.append_recovery_review(
            intent.operation_id, intent.vault_id, &intent.path,
            &screen.archive_fingerprint, screen.review_count,
            &screen.latest_event_digest,
            RecoveryReviewDisposition::KeepBlocked, "Investigation pending",
        ).unwrap();
        assert!(matches!(
            two.append_recovery_review(
                intent.operation_id, intent.vault_id, &intent.path,
                &screen.archive_fingerprint, screen.review_count,
                &screen.latest_event_digest,
                RecoveryReviewDisposition::EscalateManualReview,
                "Race with previous decision",
            ),
            Err(JournalError::StaleRecoveryReview)
        ));
        let refreshed = two.recovery_review_snapshot(intent.operation_id).unwrap();
        let second_event = two.append_recovery_review(
            intent.operation_id, intent.vault_id, &intent.path,
            &refreshed.archive_fingerprint, refreshed.review_count,
            &refreshed.latest_event_digest,
            RecoveryReviewDisposition::EscalateManualReview,
            "Needs independent operator approval",
        ).unwrap();
        assert_eq!(second_event.sequence, 2);
        assert_eq!(second_event.previous_event_digest,
            refreshed.latest_event_digest);
        assert_eq!(two.recovery_review_history(intent.operation_id).unwrap(),
            vec![first_event.clone(), second_event]);
        assert_eq!(one.recovery_review_history(intent.operation_id).unwrap().len(), 2);
        assert_eq!(one.local_publication_review_queue().unwrap().len(), 1);
        assert!(one.baseline(intent.vault_id, intent.document_id).unwrap().is_none());
    }

    #[test]
    fn m1dc_corrupted_review_row_or_broken_sequence_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sync.sqlite");
        let (mut j, intent) = m1dc_fixture(&db);
        let screen = j.recovery_review_snapshot(intent.operation_id).unwrap();
        j.append_recovery_review(
            intent.operation_id, intent.vault_id, &intent.path,
            &screen.archive_fingerprint, 0, "genesis",
            RecoveryReviewDisposition::KeepBlocked, "Retain content",
        ).unwrap();
        j.connection.execute(
            "UPDATE sync_local_recovery_reviews
             SET review_digest='tampered'
             WHERE operation_id=?1 AND sequence=1",
            [intent.operation_id.to_string()],
        ).unwrap();
        assert!(matches!(
            j.recovery_review_history(intent.operation_id),
            Err(JournalError::CorruptedRecord)
        ));
        j.connection.execute(
            "DELETE FROM sync_local_recovery_reviews WHERE operation_id=?1",
            [intent.operation_id.to_string()],
        ).unwrap();
        let screen = j.recovery_review_snapshot(intent.operation_id).unwrap();
        j.append_recovery_review(
            intent.operation_id, intent.vault_id, &intent.path,
            &screen.archive_fingerprint, 0, "genesis",
            RecoveryReviewDisposition::KeepBlocked, "Retain content",
        ).unwrap();
        j.connection.execute(
            "UPDATE sync_local_recovery_reviews
             SET sequence=3
             WHERE operation_id=?1 AND sequence=1",
            [intent.operation_id.to_string()],
        ).unwrap();
        assert!(matches!(
            j.recovery_review_history(intent.operation_id),
            Err(JournalError::CorruptedRecord)
        ));
        assert!(j.baseline(intent.vault_id, intent.document_id).unwrap().is_none());
    }

    #[test]
    fn m1dc_missing_or_damaged_archive_is_not_eligible_for_review() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sync.sqlite");
        let (j, intent) = m1dc_fixture(&db);
        j.connection.execute(
            "DELETE FROM sync_local_recovery_copies WHERE operation_id=?1",
            [intent.operation_id.to_string()],
        ).unwrap();
        assert!(matches!(
            j.recovery_review_snapshot(intent.operation_id),
            Err(JournalError::MissingRecoveryArchive)
        ));
        assert!(j.baseline(intent.vault_id, intent.document_id).unwrap().is_none());
    }

    #[test]
    fn m1dd_existing_journal_inspector_is_strictly_sqlite_read_only() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("existing.sqlite3");
        let mut original = SqliteSyncJournal::open(&path).unwrap();
        let pending = local_stage_intent(VaultId::new(), DocumentId::new(),
            "notes/safe.md", b"byte content");
        original.begin(pending.clone()).unwrap();
        original.mark_staged(pending.operation_id, b"byte content").unwrap();
        original.record_local_publication_attempt_once(&pending).unwrap();
        drop(original);
        let readonly = SqliteSyncJournal::open_existing_read_only(&path).unwrap();
        assert_eq!(readonly.local_publication_count().unwrap(), 1);
        assert_eq!(readonly.local_publication_review_queue().unwrap().len(), 1);
        assert!(readonly.connection.execute(
            "UPDATE sync_operations SET phase = 4 WHERE operation_id = ?1",
            [pending.operation_id.to_string()],
        ).is_err());
        assert!(readonly.connection.execute(
            "DELETE FROM sync_local_publication_attempts", [],
        ).is_err());
        assert!(readonly.baseline(pending.vault_id, pending.document_id)
            .unwrap().is_none());
    }

    #[test]
    fn m1e4_schema_five_upgrades_without_losing_existing_crash_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync.sqlite");
        let (journal, staged) = m1dc_fixture(&path);
        assert!(journal.read_native_local_publication_observation(staged.operation_id)
            .unwrap().is_none());
        journal.connection.execute_batch(
            "DROP TABLE sync_local_native_publication_observations;
             DROP TABLE IF EXISTS sync_existing_replace_receipt_candidates;
                 DROP TABLE IF EXISTS sync_existing_replace_attempts;
                  PRAGMA user_version = 5;",
        ).unwrap();
        drop(journal);
        // A read-only Settings inspection cannot silently migrate the DB.
        assert!(matches!(
            SqliteSyncJournal::open_existing_read_only(&path),
            Err(JournalError::UnsupportedSchema(5))
        ));
        let migrated = SqliteSyncJournal::open(&path).unwrap();
        let version: i64 = migrated.connection.query_row(
            "PRAGMA user_version", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(migrated.read_local_recovery_case(staged.operation_id)
            .unwrap().unwrap().operation_id, staged.operation_id);
        assert_eq!(migrated.read_local_recovery_copy(staged.operation_id)
            .unwrap().unwrap().stage_bytes, Some(b"payload".to_vec()));
        assert_eq!(migrated.local_publication_review_queue().unwrap().len(), 1);
        assert!(migrated.read_native_local_publication_observation(staged.operation_id)
            .unwrap().is_none(), "schema migration cannot invent an OS receipt");
        assert!(migrated.baseline(staged.vault_id, staged.document_id)
            .unwrap().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn m1e4_write_once_unix_audit_corruption_never_releases_baseline() {
        let mut journal = SqliteSyncJournal::memory().unwrap();
        let staged = local_stage_intent(VaultId::new(), DocumentId::new(),
            "notes/unix-publish.md", b"durably synced");
        journal.begin(staged.clone()).unwrap();
        journal.mark_staged(staged.operation_id, b"durably synced").unwrap();
        journal.record_local_publication_attempt_once(&staged).unwrap();
        let obs = NativeLocalPublicationObservation {
            operation_id: staged.operation_id,
            vault_id: staged.vault_id,
            path: staged.path.clone(),
            expected_content_hash: staged.expected_content_hash.clone(),
            parent: PinnedFileIdentity { device: 10, file: 11 },
            final_file: PinnedFileIdentity { device: 10, file: 12 },
            byte_length: b"durably synced".len() as u64,
        };
        assert!(journal.read_native_local_publication_observation(staged.operation_id)
            .unwrap().is_none());
        journal.record_native_local_publication_observation_once(&obs).unwrap();
        assert_eq!(journal.read_native_local_publication_observation(staged.operation_id)
            .unwrap(), Some(obs.clone()));
        assert!(matches!(
            journal.record_native_local_publication_observation_once(&obs),
            Err(JournalError::NativePublicationObservationAlreadyRecorded)
        ));
        assert!(matches!(
            journal.mark_applied(staged.operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)
        ));
        assert!(matches!(
            journal.commit(staged.operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)
        ));
        assert!(journal.baseline(staged.vault_id, staged.document_id)
            .unwrap().is_none());

        journal.connection.execute(
            "UPDATE sync_local_native_publication_observations
             SET observation_json = ?1 WHERE operation_id = ?2",
            params![b"tampered".as_slice(), staged.operation_id.to_string()],
        ).unwrap();
        assert!(matches!(
            journal.read_native_local_publication_observation(staged.operation_id),
            Err(JournalError::CorruptedRecord)
        ));
        assert_eq!(journal.get(staged.operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged);
        assert!(journal.baseline(staged.vault_id, staged.document_id)
            .unwrap().is_none());
    }

    #[test]
    fn newer_schema_must_never_be_overwritten() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 99;").unwrap();
        assert!(matches!(SqliteSyncJournal::prepare(conn),
            Err(JournalError::UnsupportedSchema(99))));
    }
}
