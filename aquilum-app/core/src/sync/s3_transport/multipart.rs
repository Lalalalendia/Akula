//! SYNC-05c: opt-in immutable S3 multipart transport and durable transfer receipts.
//!
//! This module cannot publish a manifest or advance the user document baseline.
//! The reader must be an already-authorized, stable, app-private staging handle,
//! not an arbitrarily opened vault path. All network I/O runs in a Core worker.
//! Incomplete multipart uploads require explicit review/lifecycle cleanup;
//! neither remote objects nor local vault files are deleted here.

use super::{classify, AwsS3ProbeClient, S3ObservedObject, S3PutCondition, S3TransportError};
use crate::sync::OperationId;
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use rusqlite::{params, Connection, OptionalExtension};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const PART_BYTES: usize = 8 * 1024 * 1024;
const READ_BUFFER_BYTES: usize = 128 * 1024;
const MAX_PARTS: u64 = 10_000;
const MAX_BLOB_BYTES: u64 = PART_BYTES as u64 * MAX_PARTS;
const SMALL_PUT_BYTES: u64 = 1024 * 1024;

#[derive(Debug)]
pub enum S3MultipartError {
    InvalidIntent,
    ContentMismatch,
    SourceChanged,
    ObjectTooLarge,
    IntentCollision,
    RecoveryRequired,
    InvalidTransition,
    Sqlite(rusqlite::Error),
    Source(std::io::Error),
    Transport(S3TransportError),
}

impl From<rusqlite::Error> for S3MultipartError {
    fn from(error: rusqlite::Error) -> Self { Self::Sqlite(error) }
}
impl From<std::io::Error> for S3MultipartError {
    fn from(error: std::io::Error) -> Self { Self::Source(error) }
}
impl From<S3TransportError> for S3MultipartError {
    fn from(error: S3TransportError) -> Self { Self::Transport(error) }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3MultipartIntent {
    pub operation_id: OperationId,
    /// Must be the content-addressed blob key in this client's vault.
    pub blob_key: String,
    pub expected_blake3: String,
    pub byte_length: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum S3MultipartPhase {
    Prepared,
    Uploading,
    Finalizing,
    Verified,
    Aborted,
}

impl S3MultipartPhase {
    fn from_db(value: i64) -> Result<Self, S3MultipartError> {
        match value {
            0 => Ok(Self::Prepared),
            1 => Ok(Self::Uploading),
            2 => Ok(Self::Finalizing),
            3 => Ok(Self::Verified),
            4 => Ok(Self::Aborted),
            _ => Err(S3MultipartError::InvalidTransition),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3MultipartStatus {
    pub phase: S3MultipartPhase,
    pub upload_id: Option<String>,
    pub completed_parts: u32,
    pub verified_etag: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum S3MultipartOutcome {
    UploadedVerified,
    RecoveredVerified,
    AlreadyVerified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3StreamDigest {
    pub object: S3ObservedObject,
    pub blake3: String,
}

/// App-private SQLite file (separate from the existing document baseline DB).
/// The caller supplies an app-data path; do not put this DB inside a vault.
/// Journaling an intent is a durable PRECONDITION, never a successful remote ACK.
pub struct S3MultipartJournal {
    db: Connection,
}

impl S3MultipartJournal {
    pub fn open(path: &Path) -> Result<Self, S3MultipartError> {
        let db = Connection::open(path)?;
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS s3_multipart_uploads (
                 operation_id TEXT PRIMARY KEY,
                 blob_key TEXT NOT NULL,
                 expected_hash TEXT NOT NULL,
                 byte_length INTEGER NOT NULL,
                 phase INTEGER NOT NULL CHECK(phase BETWEEN 0 AND 4),
                 upload_id TEXT,
                 verified_etag TEXT
             );
             CREATE UNIQUE INDEX IF NOT EXISTS one_active_s3_blob_upload
                ON s3_multipart_uploads(blob_key) WHERE phase IN (0,1,2);
             CREATE TABLE IF NOT EXISTS s3_multipart_parts (
                 operation_id TEXT NOT NULL REFERENCES s3_multipart_uploads(operation_id),
                 part_number INTEGER NOT NULL CHECK(part_number BETWEEN 1 AND 10000),
                 etag TEXT NOT NULL,
                 PRIMARY KEY (operation_id, part_number)
             );",
        )?;
        Ok(Self { db })
    }

    pub fn status(&self, id: OperationId) -> Result<Option<S3MultipartStatus>, S3MultipartError> {
        let row = self.db.query_row(
            "SELECT phase, upload_id, verified_etag
             FROM s3_multipart_uploads WHERE operation_id = ?1",
            params![id.to_string()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?)),
        ).optional()?;
        let Some((phase, upload_id, verified_etag)) = row else { return Ok(None); };
        let part_count: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM s3_multipart_parts WHERE operation_id = ?1",
            params![id.to_string()], |row| row.get(0),
        )?;
        Ok(Some(S3MultipartStatus {
            phase: S3MultipartPhase::from_db(phase)?,
            upload_id,
            completed_parts: u32::try_from(part_count)
                .map_err(|_| S3MultipartError::InvalidTransition)?,
            verified_etag,
        }))
    }

    /// Returns true only for a newly persisted operation. Replays never
    /// start a second PUT or multipart session without remote inspection.
    fn begin(&mut self, intent: &S3MultipartIntent) -> Result<bool, S3MultipartError> {
        let transaction = self.db.transaction()?;
        let prior = transaction.query_row(
            "SELECT blob_key, expected_hash, byte_length
             FROM s3_multipart_uploads WHERE operation_id = ?1",
            params![intent.operation_id.to_string()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?)),
        ).optional()?;
        if let Some((key, hash, len)) = prior {
            if key != intent.blob_key || hash != intent.expected_blake3
                || len != intent.byte_length as i64
            {
                return Err(S3MultipartError::IntentCollision);
            }
            transaction.commit()?;
            return Ok(false);
        }
        // The UNIQUE remote key also blocks competing local operations.
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO s3_multipart_uploads
             (operation_id, blob_key, expected_hash, byte_length, phase)
             VALUES (?1, ?2, ?3, ?4, 0)",
            params![intent.operation_id.to_string(), &intent.blob_key,
                &intent.expected_blake3, intent.byte_length as i64],
        )?;
        if inserted != 1 { return Err(S3MultipartError::IntentCollision); }
        transaction.commit()?;
        Ok(true)
    }

    fn mark_started(&mut self, id: OperationId, upload_id: &str)
        -> Result<(), S3MultipartError>
    {
        if upload_id.is_empty() { return Err(S3MultipartError::InvalidTransition); }
        self.transition(id, 0, 1, Some(upload_id))
    }

    fn record_part(&mut self, id: OperationId, number: i32, etag: &str)
        -> Result<(), S3MultipartError>
    {
        if !(1..=10_000).contains(&number) || etag.is_empty() {
            return Err(S3MultipartError::InvalidTransition);
        }
        if self.status(id)?.is_none_or(|s| s.phase != S3MultipartPhase::Uploading) {
            return Err(S3MultipartError::InvalidTransition);
        }
        self.db.execute(
            "INSERT INTO s3_multipart_parts(operation_id,part_number,etag)
             VALUES (?1,?2,?3)",
            params![id.to_string(), number, etag],
        )?;
        Ok(())
    }

    fn mark_finalizing(&mut self, id: OperationId, multipart: bool)
        -> Result<(), S3MultipartError>
    {
        self.transition(id, if multipart { 1 } else { 0 }, 2, None)
    }

    fn mark_verified(&mut self, id: OperationId, etag: &str)
        -> Result<(), S3MultipartError>
    {
        if etag.is_empty() { return Err(S3MultipartError::InvalidTransition); }
        let changed = self.db.execute(
            "UPDATE s3_multipart_uploads SET phase=3, verified_etag=?2
             WHERE operation_id=?1 AND phase IN (0,1,2,3)",
            params![id.to_string(), etag],
        )?;
        if changed != 1 { return Err(S3MultipartError::InvalidTransition); }
        Ok(())
    }

    fn mark_aborted(&mut self, id: OperationId) -> Result<(), S3MultipartError> {
        self.transition(id, 1, 4, None)
    }

    /// Only used when CreateMultipartUpload returned an ID, persisting the
    /// Uploading phase failed, and S3 confirmed AbortMultipartUpload.
    /// A simple Prepared state alone never authorizes this transition.
    fn mark_unstarted_aborted(&mut self, id: OperationId)
        -> Result<(), S3MultipartError>
    {
        self.transition(id, 0, 4, None)
    }

    fn transition(&mut self, id: OperationId, previous: i64, next: i64,
        upload_id: Option<&str>) -> Result<(), S3MultipartError>
    {
        let changed = self.db.execute(
            "UPDATE s3_multipart_uploads SET phase=?2,
                upload_id=COALESCE(?3,upload_id)
             WHERE operation_id=?1 AND phase=?4",
            params![id.to_string(), next, upload_id, previous],
        )?;
        if changed != 1 { return Err(S3MultipartError::InvalidTransition); }
        Ok(())
    }
}

/// Verify an existing cloud object with fixed memory, regardless of object
/// size. ETag is only a conditional read validator; BLAKE3 is the content proof.
impl AwsS3ProbeClient {
    pub async fn digest_stream_bounded(
        &self, key: &str, max_bytes: u64,
    ) -> Result<S3StreamDigest, S3TransportError> {
        self.checked_key(key)?;
        if max_bytes > MAX_BLOB_BYTES { return Err(S3TransportError::ObjectTooLarge); }
        let before = self.head(key).await?;
        if before.byte_length > max_bytes {
            return Err(S3TransportError::ObjectTooLarge);
        }
        let response = self.client.get_object().bucket(&self.bucket).key(key)
            .if_match(&before.etag).send().await
            .map_err(|e| classify(e.as_service_error().and_then(|v| v.code())))?;
        let mut stream = response.body;
        let mut total = 0_u64;
        let mut hasher = blake3::Hasher::new();
        while let Some(chunk) = stream.try_next().await
            .map_err(|_| S3TransportError::Unavailable)?
        {
            total = total.checked_add(chunk.len() as u64)
                .ok_or(S3TransportError::ObjectTooLarge)?;
            if total > max_bytes { return Err(S3TransportError::ObjectTooLarge); }
            hasher.update(&chunk);
        }
        let after = self.head(key).await?;
        if before != after || total != before.byte_length {
            return Err(S3TransportError::ChangedDuringRead);
        }
        Ok(S3StreamDigest {
            object: after,
            blake3: hasher.finalize().to_hex().to_string(),
        })
    }

    async fn verify_blob(&self, intent: &S3MultipartIntent)
        -> Result<S3ObservedObject, S3MultipartError>
    {
        let observed = self.digest_stream_bounded(
            &intent.blob_key, intent.byte_length,
        ).await?;
        if observed.blake3 != intent.expected_blake3
            || observed.object.byte_length != intent.byte_length
        {
            return Err(S3MultipartError::ContentMismatch);
        }
        Ok(observed.object)
    }

    /// Upload only content-addressed immutable blobs. Source must be a stable
    /// app-private read handle, never a path opened inside this transport.
    /// The journal is committed BEFORE any S3 call and never commits a sync
    /// baseline. Recovery only reads back; it does not blindly replay PUT.
    pub async fn upload_immutable_multipart<R: Read + Seek>(
        &self,
        journal: &mut S3MultipartJournal,
        intent: S3MultipartIntent,
        source: &mut R,
    ) -> Result<S3MultipartOutcome, S3MultipartError> {
        self.checked_key(&intent.blob_key)?;
        if intent.byte_length > MAX_BLOB_BYTES {
            return Err(S3MultipartError::ObjectTooLarge);
        }
        if self.keyspace.immutable_blob_key(&intent.expected_blake3).ok()
            .as_deref() != Some(intent.blob_key.as_str())
        {
            return Err(S3MultipartError::InvalidIntent);
        }
        if journal.status(intent.operation_id)?.is_some() {
            let saved = journal.begin(&intent)?; // detect identity collision
            debug_assert!(!saved);
            if let Some(status) = journal.status(intent.operation_id)? {
                if status.phase == S3MultipartPhase::Verified {
                    // A previous SQLite receipt is not proof the provider
                    // still retains the immutable bytes. Verify even a
                    // finished operation after restart or lost remote data.
                    let observed = self.verify_blob(&intent).await
                        .map_err(|_| S3MultipartError::RecoveryRequired)?;
                    if status.verified_etag.as_deref() != Some(observed.etag.as_str()) {
                        return Err(S3MultipartError::RecoveryRequired);
                    }
                    return Ok(S3MultipartOutcome::AlreadyVerified);
                }
            }
            // An earlier call might have completed before losing its ACK.
            // Remote absence or an inaccessible response REQUIRES review,
            // never a second create/multipart from the same operation id.
            return match self.verify_blob(&intent).await {
                Ok(remote) => {
                    journal.mark_verified(intent.operation_id, &remote.etag)?;
                    Ok(S3MultipartOutcome::RecoveredVerified)
                }
                Err(_) => Err(S3MultipartError::RecoveryRequired),
            };
        }

        // The supplied stream is checked BEFORE the first durable intent/S3
        // mutation. Upload hashing independently rechecks bytes as they leave.
        if hash_source(source, intent.byte_length)? != intent.expected_blake3 {
            return Err(S3MultipartError::ContentMismatch);
        }
        let fresh = journal.begin(&intent)?;
        if !fresh { return Err(S3MultipartError::RecoveryRequired); }

        // Pre-existing exact bytes may be safely reused. HEAD alone is
        // insufficient; read the full object and check its content digest.
        match self.head(&intent.blob_key).await {
            Ok(_) => {
                let remote = self.verify_blob(&intent).await?;
                journal.mark_verified(intent.operation_id, &remote.etag)?;
                return Ok(S3MultipartOutcome::RecoveredVerified);
            }
            Err(S3TransportError::NotFound) => {}
            Err(error) => return Err(error.into()),
        }

        if intent.byte_length <= SMALL_PUT_BYTES {
            journal.mark_finalizing(intent.operation_id, false)?;
            let mut bytes = vec![0_u8; intent.byte_length as usize];
            source.seek(SeekFrom::Start(0))?;
            source.read_exact(&mut bytes)?;
            if blake3::hash(&bytes).to_hex().as_str() != intent.expected_blake3 {
                return Err(S3MultipartError::SourceChanged);
            }
            // Both successful and ambiguous PUTs require independent readback.
            let _write = self.put_conditionally(
                &intent.blob_key, &bytes, S3PutCondition::IfAbsent,
            ).await;
            let remote = self.verify_blob(&intent).await?;
            journal.mark_verified(intent.operation_id, &remote.etag)?;
            return Ok(S3MultipartOutcome::UploadedVerified);
        }

        let created = self.client.create_multipart_upload()
            .bucket(&self.bucket).key(&intent.blob_key).send().await
            .map_err(|e| classify(e.as_service_error().and_then(|v| v.code())))?;
        let upload_id = created.upload_id()
            .filter(|id| !id.is_empty())
            .ok_or(S3MultipartError::RecoveryRequired)?
            .to_owned();
        // A process crash between remote creation and this SQLite write may
        // leave an orphan MPU; bucket abort-incomplete lifecycle is required.
        if let Err(error) = journal.mark_started(intent.operation_id, &upload_id) {
            // The server session exists, but the durable SQLite checkpoint
            // failed. We still have the UploadId in memory: abort it now.
            // A failed abort remains subject to the bucket MPU lifecycle gate.
            if self.client.abort_multipart_upload()
                .bucket(&self.bucket).key(&intent.blob_key)
                .upload_id(&upload_id).send().await.is_ok()
            {
                // If SQLite remains writable, release this failed transfer
                // without granting any upload or baseline confirmation.
                let _ = journal.mark_unstarted_aborted(intent.operation_id);
            }
            return Err(error);
        }
        if let Err(error) = source.seek(SeekFrom::Start(0)) {
            // A source can fail to rewind after preflight. Never leave a
            // successfully journaled session running without a best-effort
            // abort. No published S3 blob or local vault file is removed.
            self.abort_unfinished(
                journal, intent.operation_id, &intent.blob_key, &upload_id,
            ).await;
            return Err(S3MultipartError::Source(error));
        }

        let mut remaining = intent.byte_length;
        let mut parts = Vec::new();
        let mut sent_hasher = blake3::Hasher::new();
        let mut part_number = 1_i32;
        while remaining > 0 {
            let this_len = remaining.min(PART_BYTES as u64) as usize;
            let mut bytes = vec![0_u8; this_len];
            if let Err(error) = source.read_exact(&mut bytes) {
                self.abort_unfinished(journal, intent.operation_id, &intent.blob_key, &upload_id).await;
                return Err(S3MultipartError::Source(error));
            }
            sent_hasher.update(&bytes);
            let sent = self.client.upload_part()
                .bucket(&self.bucket).key(&intent.blob_key)
                .upload_id(&upload_id).part_number(part_number)
                .body(ByteStream::from(bytes)).send().await;
            let result = match sent {
                Ok(result) => result,
                Err(error) => {
                    self.abort_unfinished(journal, intent.operation_id, &intent.blob_key, &upload_id).await;
                    return Err(classify(error.as_service_error().and_then(|v| v.code())).into());
                }
            };
            let etag = match result.e_tag().filter(|s| !s.is_empty()) {
                Some(etag) => etag.to_owned(),
                None => {
                    self.abort_unfinished(journal, intent.operation_id, &intent.blob_key, &upload_id).await;
                    return Err(S3MultipartError::RecoveryRequired);
                }
            };
            if let Err(error) = journal.record_part(intent.operation_id, part_number, &etag) {
                self.abort_unfinished(journal, intent.operation_id, &intent.blob_key, &upload_id).await;
                return Err(error);
            }
            parts.push(CompletedPart::builder().part_number(part_number).e_tag(etag).build());
            remaining -= this_len as u64;
            part_number += 1;
        }
        if sent_hasher.finalize().to_hex().as_str() != intent.expected_blake3 {
            self.abort_unfinished(journal, intent.operation_id, &intent.blob_key, &upload_id).await;
            return Err(S3MultipartError::SourceChanged);
        }

        // The server MUST enforce atomic create-if-absent at COMPLETE, not
        // HEAD-before-COMPLETE. Never retry a lost completion ACK blindly.
        if let Err(error) = journal.mark_finalizing(intent.operation_id, true) {
            // We have not called Complete: the MPU can still be aborted.
            // Never publish after failing to durably record Finalizing.
            self.abort_unfinished(
                journal, intent.operation_id, &intent.blob_key, &upload_id,
            ).await;
            return Err(error);
        }
        let completed = CompletedMultipartUpload::builder().set_parts(Some(parts)).build();
        let _completion = self.client.complete_multipart_upload()
            .bucket(&self.bucket).key(&intent.blob_key).upload_id(&upload_id)
            .multipart_upload(completed).if_none_match("*").send().await;

        // On 409/412, transport loss or even an apparently successful result:
        // full stream readback and ETag stability decide, not the SDK receipt.
        let remote = self.verify_blob(&intent).await?;
        journal.mark_verified(intent.operation_id, &remote.etag)?;
        Ok(S3MultipartOutcome::UploadedVerified)
    }

    async fn abort_unfinished(
        &self, journal: &mut S3MultipartJournal, id: OperationId,
        key: &str, upload_id: &str,
    ) {
        // Only abort a session before COMPLETE; it cannot delete a published
        // object. Failed aborts remain in the recovery journal for inspection.
        if self.client.abort_multipart_upload().bucket(&self.bucket)
            .key(key).upload_id(upload_id).send().await.is_ok()
        {
            let _ = journal.mark_aborted(id);
        }
    }
}

/// Hash the actual source handle using fixed memory, checking EOF and rewinding.
fn hash_source<R: Read + Seek>(
    source: &mut R, expected_length: u64,
) -> Result<String, S3MultipartError> {
    if expected_length > MAX_BLOB_BYTES { return Err(S3MultipartError::ObjectTooLarge); }
    source.seek(SeekFrom::Start(0))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; READ_BUFFER_BYTES];
    let mut total = 0_u64;
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 { break; }
        total = total.checked_add(count as u64)
            .ok_or(S3MultipartError::ObjectTooLarge)?;
        if total > expected_length { return Err(S3MultipartError::SourceChanged); }
        hasher.update(&buffer[..count]);
    }
    source.seek(SeekFrom::Start(0))?;
    if total != expected_length { return Err(S3MultipartError::SourceChanged); }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{S3ConnectionProfile, S3Keyspace, VaultId};
    use std::io::{Cursor, Write};

    fn intent(bytes: &[u8], ns: &S3Keyspace) -> S3MultipartIntent {
        let hash = blake3::hash(bytes).to_hex().to_string();
        S3MultipartIntent {
            operation_id: OperationId::new(),
            blob_key: ns.immutable_blob_key(&hash).unwrap(),
            expected_blake3: hash,
            byte_length: bytes.len() as u64,
        }
    }

    #[test]
    fn source_hash_is_streamed_bounded_and_rewound() {
        let input = b"abc";
        let mut reader = Cursor::new(input.to_vec());
        reader.set_position(2);
        assert_eq!(hash_source(&mut reader, 3).unwrap(),
            blake3::hash(input).to_hex().to_string());
        assert_eq!(reader.position(), 0);
        assert!(matches!(hash_source(&mut reader, 2),
            Err(S3MultipartError::SourceChanged)));
        assert!(matches!(hash_source(&mut reader, MAX_BLOB_BYTES + 1),
            Err(S3MultipartError::ObjectTooLarge)));
    }

    #[test]
    fn sqlite_recovery_is_durable_and_rejects_intent_reuse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("multipart.sqlite");
        let ns = S3Keyspace::new("akula-sync/test", VaultId::new()).unwrap();
        let op = intent(b"abc", &ns);
        let mut journal = S3MultipartJournal::open(&path).unwrap();
        assert!(journal.begin(&op).unwrap());
        assert!(!journal.begin(&op).unwrap());
        journal.mark_started(op.operation_id, "multipart-session-id").unwrap();
        journal.record_part(op.operation_id, 1, "\"part-etag\"").unwrap();
        assert!(matches!(journal.record_part(op.operation_id, 1, "\"changed\""),
            Err(S3MultipartError::Sqlite(_))));
        drop(journal);
        let mut journal = S3MultipartJournal::open(&path).unwrap();
        let status = journal.status(op.operation_id).unwrap().unwrap();
        assert_eq!(status.phase, S3MultipartPhase::Uploading);
        assert_eq!(status.upload_id.as_deref(), Some("multipart-session-id"));
        assert_eq!(status.completed_parts, 1);
        let mut altered = op.clone();
        altered.byte_length = 42;
        assert!(matches!(journal.begin(&altered),
            Err(S3MultipartError::IntentCollision)));
        let mut other = op.clone();
        other.operation_id = OperationId::new();
        assert!(matches!(journal.begin(&other),
            Err(S3MultipartError::IntentCollision)));
        assert!(journal.status(other.operation_id).unwrap().is_none());
        journal.mark_finalizing(op.operation_id, true).unwrap();
        journal.mark_verified(op.operation_id, "\"remote-etag\"").unwrap();
        assert_eq!(journal.status(op.operation_id).unwrap().unwrap().phase,
            S3MultipartPhase::Verified);
        // Content-addressed blobs are shared. Once the first operation has
        // completed, a second operation may independently verify the same key.
        assert!(journal.begin(&other).unwrap());
        assert_eq!(journal.status(other.operation_id).unwrap().unwrap().phase,
            S3MultipartPhase::Prepared);
    }

    #[test]
    fn journal_rejects_invalid_phases_and_does_not_promote_aborted_uploads() {
        let dir = tempfile::tempdir().unwrap();
        let ns = S3Keyspace::new("akula-sync/test", VaultId::new()).unwrap();
        let op = intent(b"never remotely written", &ns);
        let mut journal = S3MultipartJournal::open(&dir.path().join("phases.sqlite")).unwrap();
        assert!(journal.begin(&op).unwrap());
        assert!(matches!(
            journal.mark_finalizing(op.operation_id, true),
            Err(S3MultipartError::InvalidTransition)
        ));
        journal.mark_started(op.operation_id, "incomplete-test-mpu").unwrap();
        journal.mark_aborted(op.operation_id).unwrap();
        assert_eq!(
            journal.status(op.operation_id).unwrap().unwrap().phase,
            S3MultipartPhase::Aborted
        );
        assert!(matches!(
            journal.mark_verified(op.operation_id, "\"unexpected\""),
            Err(S3MultipartError::InvalidTransition)
        ));
        assert!(!journal.begin(&op).unwrap());
    }


    /// Allows the preflight BLAKE3 pass, then simulates disk/read failure
    /// after S3 CreateMultipartUpload has started. No editor/real vault IO.
    struct FailingSecondPass {
        reader: Cursor<Vec<u8>>,
        rewind_count: usize,
    }

    impl Read for FailingSecondPass {
        fn read(&mut self, data: &mut [u8]) -> std::io::Result<usize> {
            if self.rewind_count >= 3 {
                return Err(std::io::Error::other("synthetic source read failure"));
            }
            self.reader.read(data)
        }
    }

    impl Seek for FailingSecondPass {
        fn seek(&mut self, offset: SeekFrom) -> std::io::Result<u64> {
            if matches!(offset, SeekFrom::Start(0)) {
                self.rewind_count += 1;
            }
            self.reader.seek(offset)
        }
    }

    /// Run only against disposable loopback MinIO with fresh bucket credentials.
    #[tokio::test]
    #[ignore = "requires the isolated MinIO service in sync-s3-minio.yml"]
    async fn minio_multipart_64mib_and_recovery() {
        let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").unwrap();
        assert!(endpoint.starts_with("http://127.0.0.1:")
            || endpoint.starts_with("http://localhost:"));
        let ns = S3Keyspace::new("akula-sync/multipart-ci", VaultId::new()).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "akula-multipart-test".into(),
            region: "us-east-1".into(),
            endpoint: Some(endpoint),
            force_path_style: true,
        };
        let transport = AwsS3ProbeClient::connect(profile, ns.clone()).await.unwrap();
        transport.client.create_bucket().bucket("akula-multipart-test")
            .send().await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut reader = std::fs::OpenOptions::new().read(true).write(true).create_new(true)
            .open(dir.path().join("staged.bin")).unwrap();
        let block = vec![0xB7_u8; 1024 * 1024];
        for i in 0..65 {
            let mut part = block.clone();
            part[0] = i as u8;
            reader.write_all(&part).unwrap();
        }
        reader.write_all(b"end").unwrap();
        reader.sync_all().unwrap();
        let expected_len = 65_u64 * 1024 * 1024 + 3;
        let digest = hash_source(&mut reader, expected_len).unwrap();
        let op = S3MultipartIntent {
            operation_id: OperationId::new(),
            blob_key: ns.immutable_blob_key(&digest).unwrap(),
            expected_blake3: digest.clone(),
            byte_length: expected_len,
        };
        let journal_path = dir.path().join("transfer.sqlite");
        let mut journal = S3MultipartJournal::open(&journal_path).unwrap();
        assert_eq!(transport.upload_immutable_multipart(
            &mut journal, op.clone(), &mut reader,
        ).await.unwrap(), S3MultipartOutcome::UploadedVerified);
        assert_eq!(journal.status(op.operation_id).unwrap().unwrap().completed_parts, 9);
        assert_eq!(transport.digest_stream_bounded(&op.blob_key, expected_len)
            .await.unwrap().blake3, digest);
        assert_eq!(transport.upload_immutable_multipart(
            &mut journal, op.clone(), &mut reader,
        ).await.unwrap(), S3MultipartOutcome::AlreadyVerified);
        drop(journal);
        // Simulate lost completion ACK/checkpoint: another persisted operation
        // was finalizing, but the exact cloud blob is already fully available.
        let recovery_path = dir.path().join("recovery.sqlite");
        let mut recovery = S3MultipartJournal::open(&recovery_path).unwrap();
        assert!(recovery.begin(&op).unwrap());
        recovery.mark_finalizing(op.operation_id, false).unwrap();
        drop(recovery);
        let mut recovered = S3MultipartJournal::open(&recovery_path).unwrap();
        assert_eq!(transport.upload_immutable_multipart(
            &mut recovered, op.clone(), &mut reader,
        ).await.unwrap(), S3MultipartOutcome::RecoveredVerified);
        assert_eq!(recovered.status(op.operation_id).unwrap().unwrap().phase,
            S3MultipartPhase::Verified);

        // Untrusted same-key remote data must never be promoted merely
        // because HEAD says the key exists. Preserve the server object.
        let corrupt = intent(b"expected", &ns);
        transport.put_conditionally(
            &corrupt.blob_key, b"corrupt!", S3PutCondition::IfAbsent,
        ).await.unwrap();
        let mut genuine = Cursor::new(b"expected".to_vec());
        assert!(matches!(
            transport.upload_immutable_multipart(
                &mut recovered, corrupt.clone(), &mut genuine,
            ).await,
            Err(S3MultipartError::ContentMismatch)
        ));
        assert_eq!(recovered.status(corrupt.operation_id).unwrap().unwrap().phase,
            S3MultipartPhase::Prepared);
        assert_eq!(transport.get_bounded(&corrupt.blob_key, 32).await.unwrap().bytes,
            b"corrupt!");

        // A source with an unexpected digest must be rejected BEFORE the
        // operation is journaled and before any S3 transfer is started.
        let wrong = intent(b"original", &ns);
        let mut modified = Cursor::new(b"modified".to_vec());
        assert!(matches!(
            transport.upload_immutable_multipart(
                &mut recovered, wrong.clone(), &mut modified,
            ).await,
            Err(S3MultipartError::ContentMismatch)
        ));
        assert!(recovered.status(wrong.operation_id).unwrap().is_none());

        // A partially sent MPU with no published object is NOT replayed.
        // No create/put is attempted, even with the source still available.
        let interrupted = intent(b"pending", &ns);
        recovered.begin(&interrupted).unwrap();
        recovered.mark_started(interrupted.operation_id, "simulated-lost-mpu").unwrap();
        let mut pending_source = Cursor::new(b"pending".to_vec());
        assert!(matches!(
            transport.upload_immutable_multipart(
                &mut recovered, interrupted.clone(), &mut pending_source,
            ).await,
            Err(S3MultipartError::RecoveryRequired)
        ));
        assert_eq!(
            recovered.status(interrupted.operation_id).unwrap().unwrap().phase,
            S3MultipartPhase::Uploading
        );
        assert_eq!(transport.head(&interrupted.blob_key).await,
            Err(S3TransportError::NotFound));

        // Empty blobs are valid immutable content and never need an MPU.
        let empty = intent(b"", &ns);
        let mut empty_source = Cursor::new(Vec::new());
        assert_eq!(transport.upload_immutable_multipart(
            &mut recovered, empty.clone(), &mut empty_source,
        ).await.unwrap(), S3MultipartOutcome::UploadedVerified);
        assert_eq!(recovered.status(empty.operation_id).unwrap().unwrap().completed_parts, 0);
        assert_eq!(transport.digest_stream_bounded(&empty.blob_key, 0).await.unwrap().blake3,
            empty.expected_blake3);
        let mut independent = empty.clone();
        independent.operation_id = OperationId::new();
        let mut shared_source = Cursor::new(Vec::new());
        assert_eq!(transport.upload_immutable_multipart(
            &mut recovered, independent.clone(), &mut shared_source,
        ).await.unwrap(), S3MultipartOutcome::RecoveredVerified);
        assert_eq!(recovered.status(independent.operation_id).unwrap().unwrap().phase,
            S3MultipartPhase::Verified);
        // A provider may advertise conditional PUT while silently ignoring
        // multipart conditions. Prove If-None-Match:* is honored by COMPLETE,
        // and that the existing object cannot be overwritten.
        let guarded_key = ns.immutable_blob_key(
            &blake3::hash(b"original-guarded").to_hex().to_string(),
        ).unwrap();
        transport.put_conditionally(
            &guarded_key, b"original-guarded", S3PutCondition::IfAbsent,
        ).await.unwrap();
        let started = transport.client.create_multipart_upload()
            .bucket(&transport.bucket).key(&guarded_key).send().await.unwrap();
        let guarded_upload_id = started.upload_id().unwrap();
        let uploaded = transport.client.upload_part()
            .bucket(&transport.bucket).key(&guarded_key)
            .upload_id(guarded_upload_id).part_number(1)
            .body(ByteStream::from(b"untrusted replacement".to_vec()))
            .send().await.unwrap();
        let guarded_parts = CompletedMultipartUpload::builder().set_parts(Some(vec![
            CompletedPart::builder().part_number(1)
                .e_tag(uploaded.e_tag().unwrap()).build(),
        ])).build();
        let guarded_result = transport.client.complete_multipart_upload()
            .bucket(&transport.bucket).key(&guarded_key)
            .upload_id(guarded_upload_id).multipart_upload(guarded_parts)
            .if_none_match("*").send().await;
        let guard_error = guarded_result.err()
            .expect("MinIO must atomically refuse replacing an existing key");
        assert_eq!(guard_error.as_service_error().and_then(|e| e.code()),
            Some("PreconditionFailed"));
        assert_eq!(transport.get_bounded(&guarded_key, 128).await.unwrap().bytes,
            b"original-guarded");
        // A failed conditional Complete may leave an MPU session behind.
        // Abort the disposable session; never issue DeleteObject.
        let _ = transport.client.abort_multipart_upload()
            .bucket(&transport.bucket).key(&guarded_key)
            .upload_id(guarded_upload_id).send().await;

        // Exercise the transport's explicit abort path, including its
        // durable state transition. The object itself must remain absent.
        let cancelled = intent(b"cancelled-multipart", &ns);
        assert!(recovered.begin(&cancelled).unwrap());
        let begun = transport.client.create_multipart_upload()
            .bucket(&transport.bucket).key(&cancelled.blob_key).send().await.unwrap();
        let cancelled_upload_id = begun.upload_id().unwrap();
        recovered.mark_started(cancelled.operation_id, cancelled_upload_id).unwrap();
        transport.abort_unfinished(
            &mut recovered, cancelled.operation_id, &cancelled.blob_key,
            cancelled_upload_id,
        ).await;
        assert_eq!(recovered.status(cancelled.operation_id).unwrap().unwrap().phase,
            S3MultipartPhase::Aborted);
        assert_eq!(transport.head(&cancelled.blob_key).await,
            Err(S3TransportError::NotFound));
        // Inject actual source read failure AFTER CreateMultipartUpload.
        // The service must abort that session, leave no completed object,
        // and durably record Aborted, not count bytes as synchronized.
        let failed_bytes = vec![0x55_u8; SMALL_PUT_BYTES as usize + 1];
        let failed = intent(&failed_bytes, &ns);
        let mut failed_source = FailingSecondPass {
            reader: Cursor::new(failed_bytes),
            rewind_count: 0,
        };
        assert!(matches!(
            transport.upload_immutable_multipart(
                &mut recovered, failed.clone(), &mut failed_source,
            ).await,
            Err(S3MultipartError::Source(_))
        ));
        assert_eq!(recovered.status(failed.operation_id).unwrap().unwrap().phase,
            S3MultipartPhase::Aborted);
        assert_eq!(transport.head(&failed.blob_key).await,
            Err(S3TransportError::NotFound));
    }
}
