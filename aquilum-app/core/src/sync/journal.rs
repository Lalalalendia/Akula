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
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::path::Path;

const SCHEMA_VERSION: i64 = 1;

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
#[serde(deny_unknown_fields)]
pub struct OperationIntent {
    pub operation_id: OperationId,
    pub vault_id: VaultId,
    pub document_id: DocumentId,
    pub revision_id: RevisionId,
    pub action: JournalAction,
    /// Logical vault-relative path; no native absolute paths in the journal.
    pub path: SyncRelativePath,
    /// Opaque provider object ID; never interpreted as a filesystem path.
    pub remote_object_id: String,
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
            || hash.len() != 64
            || !hash.bytes().all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
        {
            return Err(JournalError::InvalidIntent);
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
    pub revision_id: RevisionId,
    pub content_hash: String,
    pub operation_id: OperationId,
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
    MissingOperation,
    InvalidTransition,
    ReadbackMismatch,
    CorruptedRecord,
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
                 PRAGMA user_version = 1;
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
        if load_baseline(&transaction, intent.vault_id, intent.document_id)?
            .map(|b| b.revision_id) != intent.expected_baseline
        {
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
        if load_baseline(&transaction, intent.vault_id, intent.document_id)?
            .map(|b| b.revision_id) != intent.expected_baseline
        {
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
            let action = match record.phase {
                JournalPhase::Pending => RecoveryAction::ResumeStaging,
                JournalPhase::Staged | JournalPhase::Applied => RecoveryAction::InspectBeforeRetry,
                JournalPhase::Verified => RecoveryAction::RetryCommit,
                JournalPhase::Committed => return Err(JournalError::CorruptedRecord),
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
            revision_id: revision,
            action: JournalAction::UploadImmutable,
            path: SyncRelativePath::parse("notes/demo.md").unwrap(),
            remote_object_id: format!("immutable/{}", revision),
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
    fn newer_schema_must_never_be_overwritten() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 99;").unwrap();
        assert!(matches!(SqliteSyncJournal::prepare(conn),
            Err(JournalError::UnsupportedSchema(99))));
    }
}
