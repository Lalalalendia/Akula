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
use super::local_recovery::{self, ObservedLocalFile, ReadOnlyLocalPublicationEvidence};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::path::Path;

const SCHEMA_VERSION: i64 = 3;

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
    /// The durable record already claims this operation or portable target.
    /// Never automatically replay an uncertain local publication.
    PublicationAttemptAlreadyRecorded,
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
                 PRAGMA user_version = 3;
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
                 PRAGMA user_version = 3;
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
                 PRAGMA user_version = 3;
                 COMMIT;",
            )?,
            SCHEMA_VERSION => {},
            other => return Err(JournalError::UnsupportedSchema(other)),
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

    /// Claim a STAGED local-create attempt exactly once, durably BEFORE any
    /// filesystem stage/create/rename can begin. SQLite WAL + FULL and a
    /// single conditional INSERT provide a cross-process, no-replay fence.
    /// An earlier attempt on this UUID OR on the same portable vault path
    /// fails closed, even if its process died before touching the filesystem.
    ///
    /// Do not turn a successful insert into a "preserved" or baseline receipt.
    pub fn record_local_publication_attempt_once(
        &self, intent: &OperationIntent,
    ) -> JournalResult<()> {
        let existing = self.get(intent.operation_id)?
            .ok_or(JournalError::MissingOperation)?;
        if existing.intent != *intent {
            return Err(JournalError::IntentCollision);
        }
        if existing.phase != JournalPhase::Staged
            || existing.staged_hash.as_deref()
                != Some(intent.expected_content_hash.as_str())
            || intent.action != JournalAction::StageDownload
            || intent.remote_guard.is_some()
            || intent.expected_baseline.is_some()
            || self.baseline(intent.vault_id, intent.document_id)?.is_some()
        {
            return Err(JournalError::InvalidTransition);
        }
        let digest = blake3::hash(&serde_json::to_vec(intent)?).to_hex().to_string();
        // The WHERE clause revalidates phase, digest and staged hash at the
        // moment of durable publication admission, not only in an earlier GET.
        let changed = self.connection.execute(
            "INSERT OR IGNORE INTO sync_local_publication_attempts
             (operation_id,vault_id,path_collision_key,relative_path,expected_hash,intent_digest)
             SELECT ?1,?2,?3,?4,?5,?6 FROM sync_operations
             WHERE operation_id = ?1 AND phase = 1
               AND staged_hash = ?5 AND digest = ?6",
            params![
                intent.operation_id.to_string(), intent.vault_id.to_string(),
                intent.path.collision_key(), intent.path.as_str(),
                &intent.expected_content_hash, digest,
            ],
        )?;
        if changed != 1 {
            // Could be another worker, a previous crash, a name collision,
            // or a raced phase change. None justifies a retry.
            return Err(JournalError::PublicationAttemptAlreadyRecorded);
        }
        if !self.local_publication_review_queue()?.iter().any(|entry| {
            entry.operation_id == intent.operation_id
                && entry.vault_id == intent.vault_id
                && entry.path == intent.path
                && entry.expected_content_hash == intent.expected_content_hash
        }) {
            return Err(JournalError::CorruptedRecord);
        }
        Ok(())
    }

    /// Persisted recovery/quarantine *queue*, not a cleanup executor.
    /// Treat every returned entry as requiring human/host investigation.
    /// In particular, a missing stage or a same-hash destination is NOT
    /// proof a lost acknowledgment was safe to replay or commit.
    pub fn local_publication_review_queue(
        &self,
    ) -> JournalResult<Vec<LocalPublicationReview>> {
        let mut stmt = self.connection.prepare(
            "SELECT operation_id, vault_id, path_collision_key, relative_path,
                    expected_hash, intent_digest
             FROM sync_local_publication_attempts
             ORDER BY vault_id, path_collision_key, operation_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                row.get::<_, String>(4)?, row.get::<_, String>(5)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            let (raw_id, raw_vault, key, raw_path, hash, digest) = row?;
            let id: OperationId = raw_id.parse()
                .map_err(|_| JournalError::CorruptedRecord)?;
            let stored_vault: VaultId = raw_vault.parse()
                .map_err(|_| JournalError::CorruptedRecord)?;
            let record = self.get(id)?.ok_or(JournalError::CorruptedRecord)?;
            let parsed_path = SyncRelativePath::parse(&raw_path)
                .map_err(|_| JournalError::CorruptedRecord)?;
            if record.intent.vault_id != stored_vault
                || record.intent.path != parsed_path
                || record.intent.path.as_str() != raw_path
                || record.intent.path.collision_key() != key
                || record.intent.expected_content_hash != hash
                || blake3::hash(&serde_json::to_vec(&record.intent)?)
                    .to_hex().as_str() != digest
                || record.intent.action != JournalAction::StageDownload
                || record.phase != JournalPhase::Staged
            {
                return Err(JournalError::CorruptedRecord);
            }
            result.push(LocalPublicationReview {
                operation_id: id,
                vault_id: stored_vault,
                path: parsed_path,
                expected_content_hash: hash,
            });
        }
        Ok(result)
    }

    /// Compare a proposed non-authoritative incident snapshot with the
    /// immutable STAGED publication claim. No user file or Sync baseline is
    /// modified. This does not certify who created a matching file.
    fn check_local_recovery_case(&self, case: &LocalRecoveryEvidenceCase)
        -> JournalResult<()>
    {
        let claim = self.local_publication_review_queue()?.into_iter()
            .find(|entry| entry.operation_id == case.operation_id)
            .ok_or(JournalError::CorruptedRecord)?;
        if case.vault_id != claim.vault_id
            || case.relative_path != claim.path
            || case.expected_content_hash != claim.expected_content_hash
            || case.evidence.operation_id != case.operation_id
            || case.evidence.assessment != local_recovery::classify(
                &case.evidence.stage, &case.evidence.final_name,
            )
            || case.evidence.parent_identity.file == 0
        {
            return Err(JournalError::CorruptedRecord);
        }
        // This cap also protects deserialized, tampered evidence records
        // from being represented as verified, bounded local observations.
        for observed in [&case.evidence.stage, &case.evidence.final_name] {
            if let ObservedLocalFile::Present { identity, byte_length, .. } = observed {
                if identity.file == 0 || *byte_length > 32 * 1024 * 1024 {
                    return Err(JournalError::CorruptedRecord);
                }
            }
        }
        Ok(())
    }

    /// Make the FIRST observed recovery state durable in the *existing*
    /// app-data WAL/FULL SQLite journal. A later same-byte/different-OS-ID
    /// observation cannot overwrite it. No cleanup, replay or CAS receipt.
    pub(crate) fn record_local_recovery_case_once(
        &self, case: &LocalRecoveryEvidenceCase,
    ) -> JournalResult<()> {
        self.check_local_recovery_case(case)?;
        let payload = serde_json::to_vec(case)?;
        let digest = blake3::hash(&payload).to_hex().to_string();
        let count = self.connection.execute(
            "INSERT OR IGNORE INTO sync_local_recovery_evidence
                 (operation_id, evidence_json, evidence_digest)
             SELECT ?1, ?2, ?3 FROM sync_local_publication_attempts AS claim
             JOIN sync_operations AS operation
               ON claim.operation_id = operation.operation_id
             WHERE claim.operation_id = ?1 AND operation.phase = 1",
            params![case.operation_id.to_string(), &payload, &digest],
        )?;
        if count != 1 {
            return Err(JournalError::RecoveryEvidenceAlreadyRecorded);
        }
        let loaded = self.read_local_recovery_case(case.operation_id)?
            .ok_or(JournalError::CorruptedRecord)?;
        if loaded != *case {
            return Err(JournalError::CorruptedRecord);
        }
        Ok(())
    }

    /// Read back exact durable audit evidence. It remains a historical
    /// observation, not evidence that the filesystem still looks this way.
    pub(crate) fn read_local_recovery_case(
        &self, operation: OperationId,
    ) -> JournalResult<Option<LocalRecoveryEvidenceCase>> {
        let row: Option<(Vec<u8>, String)> = self.connection.query_row(
            "SELECT evidence_json, evidence_digest
             FROM sync_local_recovery_evidence WHERE operation_id = ?1",
            [operation.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        let Some((payload, expected_digest)) = row else {
            return Ok(None);
        };
        if expected_digest != blake3::hash(&payload).to_hex().as_str() {
            return Err(JournalError::CorruptedRecord);
        }
        let case: LocalRecoveryEvidenceCase = serde_json::from_slice(&payload)
            .map_err(|_| JournalError::CorruptedRecord)?;
        if case.operation_id != operation {
            return Err(JournalError::CorruptedRecord);
        }
        self.check_local_recovery_case(&case)?;
        Ok(Some(case))
    }

    /// Same operation ID and intent is idempotent; reusing the UUID for a
    /// different action is rejected. One in-flight operation per document
    /// prevents two workers from publishing incompatible baselines.
    pub fn begin(&mut self, intent: OperationIntent) -> JournalResult<JournalPhase> {
        intent.validate()?;
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = read_operation(&transaction, intent.operation_id)? {
            if existing.intent != intent {
                return Err(JournalError::IntentCollision);
            }
            return Ok(existing.phase);
        }
        let baseline = load_baseline(&transaction, intent.vault_id, intent.document_id)?;
        if baseline.as_ref().is_some_and(|b| b.profile_id != intent.profile_id) {
            return Err(JournalError::ProfileChanged);
        }
        if baseline.map(|b| b.revision_id) != intent.expected_baseline {
            return Err(JournalError::BaselineChanged);
        }

        let conflicting: Option<String> = transaction.query_row(
            "SELECT operation_id FROM sync_operations
             WHERE vault_id = ?1 AND document_id = ?2 AND phase < 4 LIMIT 1",
            params![intent.vault_id.to_string(), intent.document_id.to_string()],
            |row| row.get(0),
        ).optional()?;
        if conflicting.is_some() {
            return Err(JournalError::BusyDocument);
        }
        let payload = serde_json::to_vec(&intent)?;
        let digest = blake3::hash(&payload).to_hex().to_string();
        transaction.execute(
            "INSERT INTO sync_operations
             (operation_id,vault_id,document_id,intent,digest,phase)
             VALUES (?1,?2,?3,?4,?5,0)",
            params![
                intent.operation_id.to_string(),
                intent.vault_id.to_string(),
                intent.document_id.to_string(), payload, digest,
            ],
        )?;
        transaction.commit()?;
        Ok(JournalPhase::Pending)
    }

    /// \`staged_bytes\` must come from a verified immutable blob or a completed
    /// remote download. Computing the expected hash locally prevents recording
    /// a fake "stage complete" receipt for corrupted contents.
    /// A stage is metadata only: PR 04 must still obtain a protective copy
    /// before a destructive local File Gate operation.
    pub fn mark_staged(&mut self, id: OperationId, staged_bytes: &[u8])
        -> JournalResult<JournalPhase>
    {
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = read_operation(&transaction, id)?.ok_or(JournalError::MissingOperation)?;
        let digest = blake3::hash(staged_bytes).to_hex().to_string();
        if digest != record.intent.expected_content_hash {
            return Err(JournalError::ReadbackMismatch);
        }
        match record.phase {
            JournalPhase::Pending => {
                transaction.execute(
                    "UPDATE sync_operations SET phase = 1, staged_hash = ?2
                     WHERE operation_id = ?1 AND phase = 0",
                    params![id.to_string(), &digest],
                )?;
                transaction.commit()?;
                Ok(JournalPhase::Staged)
            }
            JournalPhase::Staged if record.staged_hash.as_deref() == Some(digest.as_str()) =>
                Ok(JournalPhase::Staged),
            _ => Err(JournalError::InvalidTransition),
        }
    }

    /// The caller must only invoke this after the IO executor has attempted
    /// the real provider/File Gate mutation. An uncertain response is NOT a
    /// receipt: recovery from Staged also demands inspection, never blind redo.
    pub fn mark_applied(&mut self, id: OperationId) -> JournalResult<JournalPhase> {
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = read_operation(&transaction, id)?.ok_or(JournalError::MissingOperation)?;
        // A remote download has no native, crash-durable local apply receipt.
        // Generic provider readback/hash must never turn it into a baseline.
        if record.intent.action == JournalAction::StageDownload {
            return Err(JournalError::LocalDownloadNeedsNativeReceipt);
        }
        match record.phase {
            JournalPhase::Staged if record.staged_hash.as_deref()
                == Some(record.intent.expected_content_hash.as_str()) => {
                transaction.execute(
                    "UPDATE sync_operations SET phase = 2
                     WHERE operation_id = ?1 AND phase = 1",
                    [id.to_string()],
                )?;
                transaction.commit()?;
                Ok(JournalPhase::Applied)
            }
            JournalPhase::Applied => Ok(JournalPhase::Applied),
            _ => Err(JournalError::InvalidTransition),
        }
    }

    /// Independently read-back bytes and the current opaque provider validator.
    /// The caller is responsible for fetching *actual destination bytes*;
    /// passing staged source bytes instead would violate the executor contract.
    pub fn mark_verified(&mut self, id: OperationId, observed_bytes: &[u8],
        provider_revision: &str) -> JournalResult<JournalPhase>
    {
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = read_operation(&transaction, id)?.ok_or(JournalError::MissingOperation)?;
        // A remote download has no native, crash-durable local apply receipt.
        // Generic provider readback/hash must never turn it into a baseline.
        if record.intent.action == JournalAction::StageDownload {
            return Err(JournalError::LocalDownloadNeedsNativeReceipt);
        }
        let digest = blake3::hash(observed_bytes).to_hex().to_string();
        if digest != record.intent.expected_content_hash || provider_revision.is_empty() {
            return Err(JournalError::ReadbackMismatch);
        }
        match record.phase {
            JournalPhase::Applied => {
                transaction.execute(
                    "UPDATE sync_operations SET phase = 3, readback_hash = ?2,
                     provider_revision = ?3 WHERE operation_id = ?1 AND phase = 2",
                    params![id.to_string(), &digest, provider_revision],
                )?;
                transaction.commit()?;
                Ok(JournalPhase::Verified)
            }
            JournalPhase::Verified if record.readback_hash.as_deref() == Some(digest.as_str())
                && record.provider_revision.as_deref() == Some(provider_revision) =>
                Ok(JournalPhase::Verified),
            _ => Err(JournalError::InvalidTransition),
        }
    }

    /// Commit baseline and terminal journal state IN ONE transaction. Recheck
    /// the expected previous baseline even if validation happened at begin().
    /// A failed SQLite transaction leaves the operation Verified and retryable.
    pub fn commit(&mut self, id: OperationId) -> JournalResult<JournalPhase> {
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = read_operation(&transaction, id)?.ok_or(JournalError::MissingOperation)?;
        // A remote download has no native, crash-durable local apply receipt.
        // Generic provider readback/hash must never turn it into a baseline.
        if record.intent.action == JournalAction::StageDownload {
            return Err(JournalError::LocalDownloadNeedsNativeReceipt);
        }
        if record.phase == JournalPhase::Committed {
            return Ok(JournalPhase::Committed);
        }
        if record.phase != JournalPhase::Verified
            || record.readback_hash.as_deref()
                != Some(record.intent.expected_content_hash.as_str())
            || record.provider_revision.as_deref().is_none_or(str::is_empty)
        {
            return Err(JournalError::InvalidTransition);
        }
        let intent = &record.intent;
        let baseline = load_baseline(&transaction, intent.vault_id, intent.document_id)?;
        if baseline.as_ref().is_some_and(|b| b.profile_id != intent.profile_id) {
            return Err(JournalError::ProfileChanged);
        }
        if baseline.map(|b| b.revision_id) != intent.expected_baseline {
            return Err(JournalError::BaselineChanged);
        }
        transaction.execute(
            "INSERT INTO sync_baselines (vault_id,document_id,revision_id,content_hash,operation_id)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(vault_id,document_id) DO UPDATE SET
                 revision_id = excluded.revision_id,
                 content_hash = excluded.content_hash,
                 operation_id = excluded.operation_id",
            params![
                intent.vault_id.to_string(),
                intent.document_id.to_string(),
                intent.revision_id.to_string(),
                &intent.expected_content_hash,
                id.to_string(),
            ],
        )?;
        transaction.execute(
            "UPDATE sync_operations SET phase = 4
             WHERE operation_id = ?1 AND phase = 3",
            [id.to_string()],
        )?;
        transaction.commit()?;
        Ok(JournalPhase::Committed)
    }

    /// Returns only unfinished actions. No automatic retry is executed here.
    /// On a lost ACK the item may still be Staged, so *both* Staged and
    /// Applied require remote readback before any further destructive action.
    pub fn recovery_queue(&self) -> JournalResult<Vec<RecoveryItem>> {
        let mut statement = self.connection.prepare(
            "SELECT operation_id FROM sync_operations WHERE phase < 4
             ORDER BY vault_id, document_id, operation_id"
        )?;
        let ids = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut result = Vec::new();
        for raw in ids {
            let id = raw?.parse().map_err(|_| JournalError::CorruptedRecord)?;
            let record = read_operation(&self.connection, id)?
                .ok_or(JournalError::CorruptedRecord)?;
            let action = if record.intent.action == JournalAction::StageDownload {
                // Local files may have changed outside Akula's in-process
                // File Gate; no retry or commit based on a prior hash.
                match record.phase {
                    JournalPhase::Pending => RecoveryAction::ResumeStaging,
                    JournalPhase::Staged | JournalPhase::Applied | JournalPhase::Verified =>
                        RecoveryAction::InspectBeforeRetry,
                    JournalPhase::Committed => return Err(JournalError::CorruptedRecord),
                }
            } else {
                match record.phase {
                    JournalPhase::Pending => RecoveryAction::ResumeStaging,
                    JournalPhase::Staged | JournalPhase::Applied =>
                        RecoveryAction::InspectBeforeRetry,
                    JournalPhase::Verified => RecoveryAction::RetryCommit,
                    JournalPhase::Committed => return Err(JournalError::CorruptedRecord),
                }
            };
            result.push(RecoveryItem { record, action });
        }
        Ok(result)
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
                "DROP TABLE sync_local_recovery_evidence;
                 DROP TABLE sync_local_publication_attempts;
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
                "DROP TABLE sync_local_recovery_evidence;
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
    fn newer_schema_must_never_be_overwritten() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 99;").unwrap();
        assert!(matches!(SqliteSyncJournal::prepare(conn),
            Err(JournalError::UnsupportedSchema(99))));
    }
}
