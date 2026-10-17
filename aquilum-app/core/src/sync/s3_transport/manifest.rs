//! SYNC-05d1: verified immutable S3 manifest graph and journaled root CAS.
//!
//! The sole mutable remote key is refs/current.json. Publication never deletes
//! remote data, mutates a user vault, acknowledges a document baseline or
//! automatically merges concurrent document edits. Conflict retains both
//! immutable proposals for a later explicit DAG reconciliation (05d2/05e).

use super::{AwsS3ProbeClient, S3PutCondition, S3TransportError};
use crate::sync::{
    DagError, OperationId, RevisionDag, RevisionOperation, RevisionRecord,
    S3HeadPointerV1, S3ManifestError, S3ManifestShardV1, S3ManifestV1,
    S3RevisionRefV1, VaultId, MAX_MANIFEST_BYTES, MAX_SHARD_BYTES, MAX_REVISIONS,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_REVISION_JSON_BYTES: u64 = 64 * 1024;
const MAX_ANCESTOR_HOPS: usize = 1_024;
// Never issue an unbounded request fan-out for an attacker-controlled DAG.
// Metadata is capped at 16 * 64 KiB in-flight; content streams are separately
// capped at four concurrent reads, regardless of manifest revision count.
const MAX_PARALLEL_REVISION_READS: usize = 16;
const MAX_PARALLEL_BLOB_STREAMS: usize = 4;

#[derive(Debug)]
pub enum S3HeadError {
    Manifest(S3ManifestError),
    Transport(S3TransportError),
    Sqlite(rusqlite::Error),
    Dag(DagError),
    InvalidRevision,
    InvalidManifest,
    IntegrityMismatch,
    ConflictingOperation,
    Conflict,
    RecoveryRequired,
    TooManyAncestors,
    InvalidCursor,
    /// Preparing the in-memory revision snapshot would exceed its strict cap.
    SnapshotBudgetExceeded,
}

impl From<S3ManifestError> for S3HeadError {
    fn from(error: S3ManifestError) -> Self { Self::Manifest(error) }
}
impl From<S3TransportError> for S3HeadError {
    fn from(error: S3TransportError) -> Self { Self::Transport(error) }
}
impl From<rusqlite::Error> for S3HeadError {
    fn from(error: rusqlite::Error) -> Self { Self::Sqlite(error) }
}
impl From<DagError> for S3HeadError {
    fn from(error: DagError) -> Self { Self::Dag(error) }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3VerifiedHead {
    pub pointer: S3HeadPointerV1,
    /// Opaque S3 revision validator, never a BLAKE3 digest.
    pub etag: String,
    /// The strict, fully validated index of referenced revisions.
    pub manifest: S3ManifestV1,
}

/// A verified, manifest-pinned read-only index. It is NOT the RemoteStore
/// adapter: file writes, deletes, and provider mutations are unavailable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3PinnedRevisionSnapshot {
    pub snapshot_id: String,
    revisions: Vec<S3RevisionRefV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3PinnedRevisionPage {
    pub snapshot_id: String,
    pub revisions: Vec<S3RevisionRefV1>,
    pub next_cursor: Option<String>,
    pub complete: bool,
}

impl S3PinnedRevisionSnapshot {
    pub fn len(&self) -> usize { self.revisions.len() }
    pub fn is_empty(&self) -> bool { self.revisions.is_empty() }

    /// Index-based paging is meaningful only for this exact *immutable*,
    /// cryptographically checked snapshot. A cursor from another generation,
    /// a repeated/empty cursor or a page over 1000 items is rejected.
    pub fn page(&self, cursor: Option<&str>, limit: usize)
        -> Result<S3PinnedRevisionPage, S3HeadError>
    {
        if !(1..=1000).contains(&limit) {
            return Err(S3HeadError::InvalidCursor);
        }
        let start = if let Some(cursor) = cursor {
            let prefix = format!("{}:", self.snapshot_id);
            let index = cursor.strip_prefix(&prefix)
                .filter(|v| !v.is_empty() && !v.starts_with('0'))
                .and_then(|v| v.parse::<usize>().ok())
                .ok_or(S3HeadError::InvalidCursor)?;
            if index == 0 || index >= self.revisions.len() {
                return Err(S3HeadError::InvalidCursor);
            }
            index
        } else { 0 };
        let end = start.saturating_add(limit).min(self.revisions.len());
        Ok(S3PinnedRevisionPage {
            snapshot_id: self.snapshot_id.clone(),
            revisions: self.revisions[start..end].to_vec(),
            next_cursor: if end < self.revisions.len() {
                Some(format!("{}:{end}", self.snapshot_id))
            } else { None },
            complete: end == self.revisions.len(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum S3HeadPublication {
    Published,
    Recovered,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum S3HeadAttemptPhase {
    Prepared,
    Published,
}

// A test-only stand-in for the verified pinning API. Production builds can
// obtain this type only through the existing complete S3 snapshot pipeline.
#[cfg(test)]
impl S3PinnedRevisionSnapshot {
    pub(crate) fn for_remote_anchor_test(snapshot_id: String, count: usize) -> Self {
        assert_eq!(count, 0, "only empty verified-DAG test fixtures are supported");
        Self { snapshot_id, revisions: Vec::new() }
    }
}

pub struct S3HeadJournal {
    db: Connection,
}

impl S3HeadJournal {
    /// Only caller-selected private app-data, never a user vault path.
    pub fn open(path: &Path) -> Result<Self, S3HeadError> {
        let db = Connection::open(path)?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;
             CREATE TABLE IF NOT EXISTS s3_head_attempts (
                 operation_id TEXT PRIMARY KEY NOT NULL,
                 vault_id TEXT NOT NULL,
                 head_key TEXT NOT NULL,
                 expected_etag TEXT,
                 pointer_json BLOB NOT NULL,
                 pointer_hash TEXT NOT NULL,
                 phase INTEGER NOT NULL CHECK(phase IN (0,1)),
                 published_etag TEXT
             );",
        )?;
        Ok(Self { db })
    }

    /// Persist the entire immutable CAS intent BEFORE a remote root PUT.
    /// Reusing an ID for different bytes, ETag or vault fails closed.
    fn begin(
        &mut self,
        op: OperationId,
        vault: VaultId,
        key: &str,
        expected_etag: Option<&str>,
        pointer_bytes: &[u8],
    ) -> Result<bool, S3HeadError> {
        let hash = blake3::hash(pointer_bytes).to_hex().to_string();
        let tx = self.db.transaction()?;
        let original = tx.query_row(
            "SELECT vault_id, head_key, expected_etag, pointer_hash, pointer_json
               FROM s3_head_attempts WHERE operation_id=?1",
            params![op.to_string()],
            |row| Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?,
                row.get::<_, Vec<u8>>(4)?,
            )),
        ).optional()?;
        if let Some((old_vault, old_key, old_etag, old_hash, old_bytes)) = original {
            if old_vault != vault.to_string() || old_key != key
                || old_etag.as_deref() != expected_etag
                || old_hash != hash || old_bytes != pointer_bytes
            { return Err(S3HeadError::ConflictingOperation); }
            tx.commit()?;
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO s3_head_attempts
                (operation_id,vault_id,head_key,expected_etag,pointer_json,pointer_hash,phase)
             VALUES (?1,?2,?3,?4,?5,?6,0)",
            params![op.to_string(), vault.to_string(), key, expected_etag, pointer_bytes, hash],
        )?;
        tx.commit()?;
        Ok(true)
    }

    fn mark_published(&mut self, op: OperationId, etag: &str)
        -> Result<(), S3HeadError>
    {
        if etag.is_empty() { return Err(S3HeadError::InvalidManifest); }
        let count = self.db.execute(
            "UPDATE s3_head_attempts SET phase=1,published_etag=?2
               WHERE operation_id=?1 AND phase IN (0,1)",
            params![op.to_string(), etag],
        )?;
        if count != 1 { return Err(S3HeadError::ConflictingOperation); }
        Ok(())
    }

    pub fn phase(&self, op: OperationId)
        -> Result<Option<S3HeadAttemptPhase>, S3HeadError>
    {
        let state = self.db.query_row(
            "SELECT phase FROM s3_head_attempts WHERE operation_id=?1",
            params![op.to_string()], |row| row.get::<_, i64>(0),
        ).optional()?;
        match state {
            None => Ok(None),
            Some(0) => Ok(Some(S3HeadAttemptPhase::Prepared)),
            Some(1) => Ok(Some(S3HeadAttemptPhase::Published)),
            _ => Err(S3HeadError::InvalidManifest),
        }
    }
}

/// Validate every revision identity and retain immutable blob candidates.
/// Completion order can differ from manifest order: RevisionDag.ingest
/// explicitly holds children until all their parents are present.
fn accept_verified_revision(
    dag: &mut RevisionDag,
    unique_blobs: &mut BTreeMap<String, u64>,
    total: &mut usize,
    reference: S3RevisionRefV1,
    revision: RevisionRecord,
) -> Result<(), S3HeadError> {
    if revision.id != reference.revision_id
        || revision.document_id != reference.document_id
    {
        return Err(S3HeadError::InvalidRevision);
    }
    match &revision.operation {
        RevisionOperation::Tombstone { .. } if reference.blob_bytes.is_none() => {}
        RevisionOperation::Content { blob_hash, .. }
        | RevisionOperation::Rename { blob_hash, .. } => {
            let len = reference.blob_bytes.ok_or(S3HeadError::InvalidRevision)?;
            if let Some(previous) = unique_blobs.insert(blob_hash.clone(), len) {
                if previous != len {
                    return Err(S3HeadError::InvalidRevision);
                }
            }
        }
        _ => return Err(S3HeadError::InvalidRevision),
    }
    dag.ingest(revision)?;
    *total = total.checked_add(1).ok_or(S3HeadError::InvalidManifest)?;
    if *total > MAX_REVISIONS {
        return Err(S3HeadError::InvalidManifest);
    }
    Ok(())
}

impl AwsS3ProbeClient {
    async fn fetch_verified_revision(
        &self, reference: S3RevisionRefV1,
    ) -> Result<(S3RevisionRefV1, RevisionRecord), S3HeadError> {
        let key = self.keyspace.immutable_revision_key(reference.revision_id);
        let observed = self.get_bounded(&key, MAX_REVISION_JSON_BYTES).await?;
        if observed.blake3 != reference.revision_blake3 {
            return Err(S3HeadError::IntegrityMismatch);
        }
        let record: RevisionRecord = serde_json::from_slice(&observed.bytes)
            .map_err(|_| S3HeadError::InvalidRevision)?;
        if record.id != reference.revision_id
            || record.document_id != reference.document_id
        {
            return Err(S3HeadError::InvalidRevision);
        }
        Ok((reference, record))
    }

    /// Retrieve a BLAKE3-pinned immutable manifest and verify every shard,
    /// per-document revision DAG and referenced blob, with strict I/O bounds.
    /// Independent S3 objects may be checked concurrently, but any missing,
    /// malformed or stale object fails the entire manifest verification.
    /// No ListObjectsV2 discovery, local filesystem access, or baseline write.
    pub async fn read_verified_manifest(
        &self,
        manifest_id: OperationId,
        expected_hash: &str,
    ) -> Result<S3ManifestV1, S3HeadError> {
        let ns = &self.keyspace;
        let key = ns.immutable_manifest_key(manifest_id);
        let payload = self.get_bounded(&key, MAX_MANIFEST_BYTES as u64).await?;
        if payload.blake3 != expected_hash {
            return Err(S3HeadError::IntegrityMismatch);
        }
        let manifest = S3ManifestV1::decode(ns, manifest_id, &payload.bytes)?;
        let mut dag = RevisionDag::new();
        let mut unique_blobs = BTreeMap::new();
        let mut unique_revisions = BTreeSet::new();
        let mut total = 0_usize;

        for shard_ref in &manifest.shards {
            let skey = ns.immutable_manifest_key(shard_ref.shard_id);
            let shard_data = self.get_bounded(&skey, MAX_SHARD_BYTES as u64).await?;
            if shard_data.blake3 != shard_ref.shard_blake3 {
                return Err(S3HeadError::IntegrityMismatch);
            }
            let shard = S3ManifestShardV1::decode(
                ns, shard_ref.shard_id, &shard_data.bytes,
            )?;
            if shard.revisions.len() != shard_ref.count as usize
                || shard.revisions.first().map(|r| r.revision_id) != Some(shard_ref.first_revision)
                || shard.revisions.last().map(|r| r.revision_id) != Some(shard_ref.last_revision)
            {
                return Err(S3HeadError::InvalidManifest);
            }

            // JoinSet aborts remaining read-only tasks on any early return.
            // Never queue more than 16 revision JSON requests at once.
            let mut tasks = tokio::task::JoinSet::new();
            for reference in shard.revisions {
                if !unique_revisions.insert(reference.revision_id) {
                    return Err(S3HeadError::InvalidRevision);
                }
                let worker = self.clone();
                tasks.spawn(async move { worker.fetch_verified_revision(reference).await });
                if tasks.len() >= MAX_PARALLEL_REVISION_READS {
                    let (reference, record) = tasks.join_next().await
                        .ok_or(S3HeadError::InvalidManifest)?
                        .map_err(|_| S3HeadError::InvalidRevision)??;
                    accept_verified_revision(
                        &mut dag, &mut unique_blobs, &mut total, reference, record,
                    )?;
                }
            }
            while let Some(completed) = tasks.join_next().await {
                let (reference, record) = completed
                    .map_err(|_| S3HeadError::InvalidRevision)??;
                accept_verified_revision(
                    &mut dag, &mut unique_blobs, &mut total, reference, record,
                )?;
            }
        }

        if total != manifest.total_revisions as usize || dag.pending_len() != 0
            || dag.active_len() != total
        {
            return Err(S3HeadError::InvalidManifest);
        }

        // Verify distinct content-addressed blobs only after DAG closure.
        // At most four whole-object streaming digests are active concurrently.
        // Every worker performs HEAD/GET If-Match/HEAD + BLAKE3, not ETag trust.
        let mut streams = tokio::task::JoinSet::new();
        for (hash, len) in unique_blobs {
            let worker = self.clone();
            streams.spawn(async move {
                let bkey = worker.keyspace.immutable_blob_key(&hash)
                    .map_err(|_| S3HeadError::InvalidRevision)?;
                let verified = worker.digest_stream_bounded(&bkey, len).await?;
                if verified.blake3 != hash || verified.object.byte_length != len {
                    return Err(S3HeadError::IntegrityMismatch);
                }
                Ok::<(), S3HeadError>(())
            });
            if streams.len() >= MAX_PARALLEL_BLOB_STREAMS {
                streams.join_next().await
                    .ok_or(S3HeadError::InvalidManifest)?
                    .map_err(|_| S3HeadError::InvalidManifest)??;
            }
        }
        while let Some(completed) = streams.join_next().await {
            completed.map_err(|_| S3HeadError::InvalidManifest)??;
        }
        Ok(manifest)
    }

    /// Construct an exact revision identity map pinned to immutable shard
    /// digests. A successor MUST preserve every previously referenced DAG
    /// revision (including tombstones and both sides of a conflict). A lone
    /// correct-looking parent hash does not prevent destructive omission.
    async fn revision_index(
        &self, manifest: &S3ManifestV1,
    ) -> Result<BTreeMap<crate::sync::RevisionId, S3RevisionRefV1>, S3HeadError> {
        let mut result = BTreeMap::new();
        for reference in &manifest.shards {
            let bytes = self.get_bounded(
                &self.keyspace.immutable_manifest_key(reference.shard_id),
                MAX_SHARD_BYTES as u64,
            ).await?;
            if bytes.blake3 != reference.shard_blake3 {
                return Err(S3HeadError::IntegrityMismatch);
            }
            let shard = S3ManifestShardV1::decode(
                &self.keyspace, reference.shard_id, &bytes.bytes,
            )?;
            if shard.revisions.len() != reference.count as usize
                || shard.revisions.first().map(|r| r.revision_id) != Some(reference.first_revision)
                || shard.revisions.last().map(|r| r.revision_id) != Some(reference.last_revision)
            { return Err(S3HeadError::InvalidManifest); }
            for revision in shard.revisions {
                if result.insert(revision.revision_id, revision).is_some() {
                    return Err(S3HeadError::InvalidRevision);
                }
            }
        }
        if result.len() != manifest.total_revisions as usize {
            return Err(S3HeadError::InvalidManifest);
        }
        Ok(result)
    }

    async fn ensure_ancestor_revisions_retained(
        &self, parent: &S3ManifestV1, child: &S3ManifestV1,
    ) -> Result<(), S3HeadError> {
        let old = self.revision_index(parent).await?;
        let new = self.revision_index(child).await?;
        for (id, original) in old {
            if new.get(&id) != Some(&original) {
                // A missing, modified or duplicate ancestor is a hard hold,
                // even when the proposed manifest has a valid parent hash.
                return Err(S3HeadError::InvalidManifest);
            }
        }
        Ok(())
    }

    /// Root read is protected by HEAD/GET If-Match/HEAD; referent's exact
    /// immutable bytes and every listed DAG dependency are checked, too.
    pub async fn read_verified_head(&self)
        -> Result<Option<S3VerifiedHead>, S3HeadError>
    {
        let root_key = self.keyspace.current_head_key();
        let current = match self.get_bounded(&root_key, 4096).await {
            Ok(value) => value,
            Err(S3TransportError::NotFound) => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        let pointer = S3HeadPointerV1::decode(&current.bytes, &self.keyspace)
            .map_err(|_| S3HeadError::InvalidManifest)?;
        let manifest = self.read_verified_manifest(
            pointer.manifest_id(), pointer.manifest_blake3(),
        ).await?;
        if manifest.generation != pointer.generation()
            || manifest.previous_manifest_id != pointer.previous_manifest_id()
        { return Err(S3HeadError::InvalidManifest); }

        // Verify the direct immutable ancestry: no fabricated parent link.
        // A bounded full history traversal is used by the publication path.
        if manifest.generation > 1 {
            let (Some(parent_id), Some(parent_hash)) = (
                manifest.previous_manifest_id, manifest.previous_manifest_blake3.as_deref()
            ) else { return Err(S3HeadError::InvalidManifest); };
            let previous = self.read_verified_manifest(parent_id, parent_hash).await?;
            if !manifest.is_successor_of(&previous, parent_hash) {
                return Err(S3HeadError::InvalidManifest);
            }
            self.ensure_ancestor_revisions_retained(&previous, &manifest).await?;
        }
        Ok(Some(S3VerifiedHead {
            pointer, etag: current.object.etag, manifest,
        }))
    }

    async fn read_manifest_envelope(
        &self, id: OperationId, digest: &str,
    ) -> Result<S3ManifestV1, S3HeadError> {
        let data = self.get_bounded(
            &self.keyspace.immutable_manifest_key(id), MAX_MANIFEST_BYTES as u64,
        ).await?;
        if data.blake3 != digest { return Err(S3HeadError::IntegrityMismatch); }
        Ok(S3ManifestV1::decode(&self.keyspace, id, &data.bytes)?)
    }

    /// Verify a complete predecessor chain of *immutable manifests* (not raw
    /// S3 listings). Hitting max_hops produces Hold, never a false empty vault.
    pub async fn verify_manifest_ancestors(
        &self, head: &S3VerifiedHead, max_hops: usize,
    ) -> Result<usize, S3HeadError> {
        if max_hops == 0 || max_hops > MAX_ANCESTOR_HOPS {
            return Err(S3HeadError::TooManyAncestors);
        }
        let mut cursor = head.manifest.clone();
        let mut visited = BTreeSet::new();
        let mut hops = 0;
        while let (Some(id), Some(hash)) = (
            cursor.previous_manifest_id, cursor.previous_manifest_blake3.as_deref(),
        ) {
            if hops == max_hops || !visited.insert(id) {
                return Err(S3HeadError::TooManyAncestors);
            }
            let parent = self.read_verified_manifest(id, hash).await?;
            if !cursor.is_successor_of(&parent, hash) {
                return Err(S3HeadError::InvalidManifest);
            }
            self.ensure_ancestor_revisions_retained(&parent, &cursor).await?;
            cursor = parent;
            hops += 1;
        }
        Ok(hops)
    }

    /// Build a stable revision index ONLY after HEAD, manifest, every shard,
    /// every revision DAG edge and all referenced blobs have been verified.
    /// A later CAS changes the current head, not these immutable pages.
    pub async fn pin_revision_snapshot(&self)
        -> Result<Option<S3PinnedRevisionSnapshot>, S3HeadError>
    {
        let Some(head) = self.read_verified_head().await? else { return Ok(None); };
        self.verify_manifest_ancestors(&head, MAX_ANCESTOR_HOPS).await?;
        let refs = self.revision_index(&head.manifest).await?;
        Ok(Some(S3PinnedRevisionSnapshot {
            snapshot_id: format!("{}:{}", head.manifest.manifest_id,
                head.pointer.manifest_blake3()),
            revisions: refs.into_values().collect(),
        }))
    }

    /// Unknown/inaccessible roots are errors, NEVER an empty page. The caller
    /// may discard a pinned snapshot when false, but cannot use this alone as
    /// deletion authority.
    pub async fn is_snapshot_current(&self, snapshot: &S3PinnedRevisionSnapshot)
        -> Result<bool, S3HeadError>
    {
        let now = self.read_verified_head().await?;
        Ok(now.is_some_and(|head| {
            snapshot.snapshot_id == format!("{}:{}", head.manifest.manifest_id,
                head.pointer.manifest_blake3())
        }))
    }

    /// Only small manifest bodies. This is an immutable, guarded publish:
    /// when a server response is lost, full readback decides, not a blind retry.
    pub async fn store_manifest_object(
        &self, id: OperationId, bytes: &[u8],
    ) -> Result<String, S3HeadError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(S3HeadError::Manifest(S3ManifestError::Oversized));
        }
        let key = self.keyspace.immutable_manifest_key(id);
        // Every immutable manifest/shard has the same strict name policy.
        let write = self.put_conditionally(&key, bytes, S3PutCondition::IfAbsent).await;
        match write {
            Ok(_) | Err(S3TransportError::PreconditionFailed)
            | Err(S3TransportError::CommitUncertain) => {}
            Err(error) => return Err(error.into()),
        }
        let observed = self.get_bounded(&key, MAX_MANIFEST_BYTES as u64).await?;
        let digest = blake3::hash(bytes).to_hex().to_string();
        if observed.blake3 != digest || observed.bytes != bytes {
            return Err(S3HeadError::IntegrityMismatch);
        }
        Ok(digest)
    }

    /// The caller MUST provide a verified old head (or None for bootstrap).
    /// A never-before-seen operation is persisted BEFORE any mutable S3 PUT.
    /// An uncertain old attempt is only read back; NEVER blindly retried.
    pub async fn publish_head_cas(
        &self,
        journal: &mut S3HeadJournal,
        operation_id: OperationId,
        proposed: S3HeadPointerV1,
        previous: Option<&S3VerifiedHead>,
    ) -> Result<S3HeadPublication, S3HeadError> {
        if operation_id != proposed.manifest_id() {
            return Err(S3HeadError::ConflictingOperation);
        }
        let manifest = self.read_verified_manifest(
            proposed.manifest_id(), proposed.manifest_blake3(),
        ).await?;
        if manifest.generation != proposed.generation()
            || manifest.previous_manifest_id != proposed.previous_manifest_id()
        { return Err(S3HeadError::InvalidManifest); }

        match previous {
            None if proposed.generation() == 1 && manifest.previous_manifest_id.is_none() => {}
            Some(parent)
                if proposed.is_direct_successor_of(&parent.pointer)
                    && manifest.is_successor_of(
                        &parent.manifest, parent.pointer.manifest_blake3(),
                    ) => {}
            _ => return Err(S3HeadError::InvalidManifest),
        }
        if let Some(parent) = previous {
            self.ensure_ancestor_revisions_retained(&parent.manifest, &manifest).await?;
        }
        let expected_etag = previous.map(|head| head.etag.as_str());
        let key = self.keyspace.current_head_key();
        let payload = proposed.encode(&self.keyspace)
            .map_err(|_| S3HeadError::InvalidManifest)?;
        let fresh = journal.begin(operation_id, self.keyspace.vault_id(), &key,
            expected_etag, &payload)?;

        let now = self.read_verified_head().await?;
        if now.as_ref().is_some_and(|head| head.pointer == proposed) {
            journal.mark_published(operation_id, &now.unwrap().etag)?;
            return Ok(S3HeadPublication::Recovered);
        }
        if !fresh {
            // Lost ACK, changed head, or unresolved prior network action:
            // no replay even if current still equals the old baseline.
            return Err(S3HeadError::RecoveryRequired);
        }
        let guard = match (previous, now.as_ref()) {
            (None, None) => S3PutCondition::IfAbsent,
            (Some(old), Some(current))
                if old.etag == current.etag && old.pointer == current.pointer =>
                    S3PutCondition::IfMatch(old.etag.clone()),
            _ => return Err(S3HeadError::Conflict),
        };

        // Strict provider-enforced create/CAS. A 412 / lost ACK does not
        // authorize an overwrite or a second writer attempt.
        let put = self.put_conditionally(&key, &payload, guard).await;
        match put {
            Ok(_) | Err(S3TransportError::PreconditionFailed)
            | Err(S3TransportError::CommitUncertain) => {}
            Err(error) => return Err(error.into()),
        }

        // A concurrent client may have won. Only byte-exact current root and
        // a proven referenced manifest allow marking the SQLite attempt.
        let current = self.read_verified_head().await?;
        if let Some(verified) = current {
            if verified.pointer == proposed {
                journal.mark_published(operation_id, &verified.etag)?;
                return Ok(S3HeadPublication::Published);
            }
            return Err(S3HeadError::Conflict);
        }
        Err(S3HeadError::RecoveryRequired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{
        DocumentId, S3ConnectionProfile, S3Keyspace, S3RevisionRefV1, SyncRelativePath,
        RevisionId, VaultId,
    };

    #[test]
    fn bounded_parallel_verifier_keeps_out_of_order_parent_closure_and_blob_lengths() {
        assert_eq!(MAX_PARALLEL_REVISION_READS, 16);
        assert_eq!(MAX_PARALLEL_BLOB_STREAMS, 4);
        let document_id = DocumentId::new();
        let base_id = RevisionId::new();
        let child_id = RevisionId::new();
        let base_bytes = b"original";
        let child_bytes = b"updated";
        let base_hash = blake3::hash(base_bytes).to_hex().to_string();
        let child_hash = blake3::hash(child_bytes).to_hex().to_string();
        let base = RevisionRecord {
            id: base_id, document_id, parents: vec![],
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/a.md").unwrap(),
                blob_hash: base_hash.clone(),
            },
        };
        let child = RevisionRecord {
            id: child_id, document_id, parents: vec![base_id],
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/a.md").unwrap(),
                blob_hash: child_hash,
            },
        };
        let mut dag = RevisionDag::new();
        let mut blobs = BTreeMap::new();
        let mut total = 0;
        let mk_ref = |record: &RevisionRecord, bytes: &[u8]| S3RevisionRefV1 {
            revision_id: record.id, document_id,
            revision_blake3: blake3::hash(
                &serde_json::to_vec(record).unwrap(),
            ).to_hex().to_string(),
            blob_bytes: Some(bytes.len() as u64),
        };
        // Simulate child completing earlier than its ancestor's network read.
        accept_verified_revision(
            &mut dag, &mut blobs, &mut total,
            mk_ref(&child, child_bytes), child,
        ).unwrap();
        assert_eq!(dag.pending_len(), 1);
        accept_verified_revision(
            &mut dag, &mut blobs, &mut total,
            mk_ref(&base, base_bytes), base.clone(),
        ).unwrap();
        assert_eq!(dag.pending_len(), 0);
        assert_eq!(dag.active_len(), 2);
        assert_eq!(dag.heads(document_id), BTreeSet::from([child_id]));
        assert_eq!(total, 2);
        assert_eq!(blobs.get(&base_hash), Some(&(base_bytes.len() as u64)));
        let mut contradictory = mk_ref(&base, base_bytes);
        contradictory.blob_bytes = Some(999);
        assert!(matches!(accept_verified_revision(
            &mut dag, &mut blobs, &mut total, contradictory, base,
        ), Err(S3HeadError::InvalidRevision)));
    }

    /// Real, isolated 384-revision/384-blob DAG crossing three shard bounds.
    /// Upload uses 24 fixed workers; the verified reader uses 16 metadata and
    /// four streaming workers. Corrupting one blob must invalidate the entire
    /// manifest, even though the immutable root and all revisions still match.
    #[tokio::test]
    #[ignore = "disposable localhost MinIO only"]
    async fn minio_verified_384_revision_dag_parallel_and_corruption() {
        let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").unwrap();
        assert!(endpoint.starts_with("http://127.0.0.1:")
            || endpoint.starts_with("http://localhost:"));
        let ns = S3Keyspace::new("akula-sync/parallel-dag-ci", VaultId::new()).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "akula-parallel-dag-test".into(),
            region: "us-east-1".into(),
            endpoint: Some(endpoint),
            force_path_style: true,
        };
        let transport = AwsS3ProbeClient::connect(profile, ns.clone()).await.unwrap();
        transport.client.create_bucket().bucket(&transport.bucket).send()
            .await.unwrap();

        let mut refs = Vec::new();
        let mut uploads = tokio::task::JoinSet::new();
        let mut corrupt_target = None;
        for index in 0..384_u32 {
            let bytes = format!("parallel-proof-content-{index:04}").into_bytes();
            let content_hash = blake3::hash(&bytes).to_hex().to_string();
            if index == 17 {
                corrupt_target = Some((content_hash.clone(), bytes.len()));
            }
            let record = RevisionRecord {
                id: RevisionId::new(),
                document_id: DocumentId::new(),
                parents: Vec::new(),
                operation: RevisionOperation::Content {
                    path: SyncRelativePath::parse(
                        &format!("notes/check-{index:04}.md"),
                    ).unwrap(),
                    blob_hash: content_hash.clone(),
                },
            };
            let json = serde_json::to_vec(&record).unwrap();
            let reference = S3RevisionRefV1 {
                revision_id: record.id, document_id: record.document_id,
                revision_blake3: blake3::hash(&json).to_hex().to_string(),
                blob_bytes: Some(bytes.len() as u64),
            };
            refs.push(reference);
            let worker = transport.clone();
            let blob_key = ns.immutable_blob_key(&content_hash).unwrap();
            let revision_key = ns.immutable_revision_key(record.id);
            uploads.spawn(async move {
                worker.put_conditionally(
                    &blob_key, &bytes, S3PutCondition::IfAbsent,
                ).await.unwrap();
                worker.put_conditionally(
                    &revision_key, &json, S3PutCondition::IfAbsent,
                ).await.unwrap();
            });
            if uploads.len() >= 24 {
                uploads.join_next().await.unwrap().unwrap();
            }
        }
        while let Some(completed) = uploads.join_next().await {
            completed.unwrap();
        }

        refs.sort_by_key(|entry| entry.revision_id);
        let mut shards = Vec::new();
        for chunk in refs.chunks(128) {
            let shard = S3ManifestShardV1::new(
                &ns, OperationId::new(), chunk.to_vec(),
            ).unwrap();
            transport.store_manifest_object(
                shard.shard_id, &shard.encode(&ns).unwrap(),
            ).await.unwrap();
            shards.push(shard.reference(&ns).unwrap());
        }
        assert_eq!(shards.len(), 3);
        let root = S3ManifestV1::new(
            &ns, OperationId::new(), 1, None, shards,
        ).unwrap();
        let hash = root.digest(&ns).unwrap();
        transport.store_manifest_object(
            root.manifest_id, &root.encode(&ns).unwrap(),
        ).await.unwrap();

        let start = std::time::Instant::now();
        let verified = transport.read_verified_manifest(root.manifest_id, &hash)
            .await.unwrap();
        assert_eq!(verified, root);
        assert_eq!(verified.total_revisions, 384);
        eprintln!("fully verified 384 S3 revision+blob pairs across 3 shards in {:?}",
            start.elapsed());

        // Test-only direct S3 mutation in a disposable bucket: never an
        // application write path. BLAKE3 must detect equal-length corruption.
        let (target_hash, target_len) = corrupt_target.unwrap();
        let damaged = vec![b'!'; target_len];
        transport.client.put_object()
            .bucket(&transport.bucket)
            .key(ns.immutable_blob_key(&target_hash).unwrap())
            .body(aws_sdk_s3::primitives::ByteStream::from(damaged))
            .send().await.unwrap();
        assert!(matches!(
            transport.read_verified_manifest(root.manifest_id, &hash).await,
            Err(S3HeadError::IntegrityMismatch)
        ));
    }

    #[test]
    fn durable_root_intent_rejects_operation_id_reuse_and_recovers_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s3-head.sqlite");
        let mut journal = S3HeadJournal::open(&p).unwrap();
        let ns = S3Keyspace::new("akula-sync/head-test", VaultId::new()).unwrap();
        let op = OperationId::new();
        let bytes = b"{\"schema\":1}";
        assert!(journal.begin(op, ns.vault_id(), &ns.current_head_key(), None, bytes).unwrap());
        assert!(!journal.begin(op, ns.vault_id(), &ns.current_head_key(), None, bytes).unwrap());
        assert!(matches!(
            journal.begin(op, ns.vault_id(), &ns.current_head_key(), Some("stale"), bytes),
            Err(S3HeadError::ConflictingOperation)
        ));
        drop(journal);
        let mut again = S3HeadJournal::open(&p).unwrap();
        assert_eq!(again.phase(op).unwrap(), Some(S3HeadAttemptPhase::Prepared));
        again.mark_published(op, "\"etag\"").unwrap();
        assert_eq!(again.phase(op).unwrap(), Some(S3HeadAttemptPhase::Published));
    }

    #[test]
    fn pinned_10k_revision_pages_are_stable_and_reject_cross_snapshot_cursors() {
        let doc = DocumentId::new();
        let mut ids: Vec<_> = (0..10_000).map(|_| RevisionId::new()).collect();
        ids.sort_unstable();
        let refs: Vec<_> = ids.iter().map(|id| S3RevisionRefV1 {
            revision_id: *id,
            document_id: doc,
            revision_blake3: blake3::hash(id.to_string().as_bytes()).to_hex().to_string(),
            blob_bytes: None,
        }).collect();
        let snapshot = S3PinnedRevisionSnapshot {
            snapshot_id: "immutable-root:deadbeef".into(),
            revisions: refs.clone(),
        };
        let other = S3PinnedRevisionSnapshot {
            snapshot_id: "other-root:deadbeef".into(),
            revisions: refs,
        };
        let mut cursor = None;
        let mut seen = BTreeSet::new();
        let mut pages = 0;
        loop {
            let page = snapshot.page(cursor.as_deref(), 137).unwrap();
            assert_eq!(page.snapshot_id, snapshot.snapshot_id);
            for entry in page.revisions { assert!(seen.insert(entry.revision_id)); }
            pages += 1;
            if page.complete {
                assert!(page.next_cursor.is_none());
                break;
            }
            let next = page.next_cursor.unwrap();
            assert!(matches!(other.page(Some(&next), 137),
                Err(S3HeadError::InvalidCursor)));
            cursor = Some(next);
        }
        assert_eq!(seen.len(), 10_000);
        assert_eq!(pages, 73);
        for invalid in [
            "wrong-root:1", "immutable-root:deadbeef:0",
            "immutable-root:deadbeef:001", "immutable-root:deadbeef:10000",
            "immutable-root:deadbeef:-1", "immutable-root:deadbeef:nan",
        ] {
            assert!(matches!(snapshot.page(Some(invalid), 100),
                Err(S3HeadError::InvalidCursor)));
        }
        assert!(matches!(snapshot.page(None, 0), Err(S3HeadError::InvalidCursor)));
        assert!(matches!(snapshot.page(None, 1001), Err(S3HeadError::InvalidCursor)));
        assert!(snapshot.page(None, 100).unwrap().revisions.len() == 100);
    }

    /// Real immutable branch with independent blob + revision object. The
    /// losing client's bytes must remain retrievable after CAS rejection.
    async fn upload_sibling(
        client: &AwsS3ProbeClient,
        ns: &S3Keyspace,
        base: &RevisionRecord,
        content: &[u8],
    ) -> (RevisionRecord, S3RevisionRefV1) {
        let hash = blake3::hash(content).to_hex().to_string();
        client.put_conditionally(
            &ns.immutable_blob_key(&hash).unwrap(),
            content,
            S3PutCondition::IfAbsent,
        ).await.unwrap();
        let new_revision = RevisionRecord {
            id: RevisionId::new(),
            document_id: base.document_id,
            parents: vec![base.id],
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("Notes/hello.md").unwrap(),
                blob_hash: hash,
            },
        };
        let json = serde_json::to_vec(&new_revision).unwrap();
        client.put_conditionally(
            &ns.immutable_revision_key(new_revision.id),
            &json,
            S3PutCondition::IfAbsent,
        ).await.unwrap();
        let reference = S3RevisionRefV1 {
            revision_id: new_revision.id,
            document_id: base.document_id,
            revision_blake3: blake3::hash(&json).to_hex().to_string(),
            blob_bytes: Some(content.len() as u64),
        };
        (new_revision, reference)
    }

    /// Rebuild a sorted, content-addressed immutable index preserving each
    /// member's exact hash, never a client-side inferred mtime snapshot.
    async fn publish_revision_shard(
        client: &AwsS3ProbeClient,
        ns: &S3Keyspace,
        mut references: Vec<S3RevisionRefV1>,
    ) -> crate::sync::S3ManifestShardRefV1 {
        references.sort_by_key(|record| record.revision_id);
        let shard = S3ManifestShardV1::new(
            ns, OperationId::new(), references,
        ).unwrap();
        client.store_manifest_object(
            shard.shard_id, &shard.encode(ns).unwrap(),
        ).await.unwrap();
        shard.reference(ns).unwrap()
    }

    /// Deliberately expensive live provider stress: 10,000 actual S3 objects,
    /// 32 in-flight conditional PUTs, 10 diagnostic listing pages and bounded
    /// sample readback. The S3 LIST itself is NOT a consistent DAG snapshot.
    /// Keep this ignored and run only on an isolated disposable MinIO bucket.
    #[tokio::test]
    #[ignore = "explicit 10k-object stress on disposable MinIO only"]
    async fn minio_10k_cloud_objects_pagination() {
        let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").unwrap();
        assert!(endpoint.starts_with("http://127.0.0.1:")
            || endpoint.starts_with("http://localhost:"));
        let ns = S3Keyspace::new("akula-sync/stress10k", VaultId::new()).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "akula-stress-ten-thousand".into(),
            region: "us-east-1".into(),
            endpoint: Some(endpoint),
            force_path_style: true,
        };
        let transport = AwsS3ProbeClient::connect(profile, ns.clone()).await.unwrap();
        transport.client.create_bucket().bucket(&transport.bucket).send().await.unwrap();

        let mut tasks = tokio::task::JoinSet::new();
        for i in 0..10_000_u32 {
            let body = format!("akula-10k-object-{i:05}").into_bytes();
            let key = ns.immutable_blob_key(
                &blake3::hash(&body).to_hex().to_string(),
            ).unwrap();
            let client = transport.client.clone();
            let bucket = transport.bucket.clone();
            tasks.spawn(async move {
                let uploaded = client.put_object().bucket(bucket).key(key)
                    .if_none_match("*").body(aws_sdk_s3::primitives::ByteStream::from(body))
                    .send().await;
                assert!(uploaded.is_ok(), "conditional S3 PUT failed at object index {i}");
            });
            if tasks.len() >= 32 {
                tasks.join_next().await.unwrap().unwrap();
            }
        }
        while let Some(done) = tasks.join_next().await { done.unwrap(); }

        let mut cursor = None;
        let mut keys = BTreeSet::new();
        let mut pages = 0_u32;
        loop {
            let page = transport.diagnostic_list_page(cursor.as_deref(), 1000)
                .await.unwrap();
            // S3/MinIO may legally return a separate empty *final* page
            // when object count is an exact multiple of max_keys=1000.
            // Truncation and the cursor, NOT a presumed page count, decide.
            assert!(!page.keys.is_empty() || page.complete,
                "empty nonterminal S3 page cannot prove completeness");
            for key in page.keys { assert!(keys.insert(key), "duplicate S3 key"); }
            pages += 1;
            if page.complete {
                assert!(page.next_cursor.is_none());
                break;
            }
            cursor = Some(page.next_cursor.expect("required continuation token"));
            assert!(pages <= 10_001, "looping S3 continuation cursor");
        }
        assert_eq!(keys.len(), 10_000);
        assert!(pages >= 10 && pages <= 10_001);
        eprintln!("verified 10000 distinct MinIO S3 objects across {pages} pages");

        // Confirm that a 10k LIST did not conceal an absent or malformed
        // sampled object. For complete DAG readback every revision and shard
        // is checked separately by the manifest reader in smaller CI tests.
        for i in [0_u32, 1, 999, 1000, 5000, 9999] {
            let content = format!("akula-10k-object-{i:05}");
            let digest = blake3::hash(content.as_bytes()).to_hex().to_string();
            let key = ns.immutable_blob_key(&digest).unwrap();
            assert!(keys.contains(&key));
            let observed = transport.get_bounded(&key, 64).await.unwrap();
            assert_eq!(observed.bytes, content.as_bytes());
            assert_eq!(observed.blake3, digest);
        }
    }

    /// Disposable localhost MinIO only. Conflicting clients retain immutable
    /// manifest branches and neither can overwrite the winner's CAS.
    #[tokio::test]
    #[ignore = "requires isolated loopback MinIO"]
    async fn minio_two_clients_manifest_cas_and_lost_ack() {
        let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").unwrap();
        assert!(endpoint.starts_with("http://127.0.0.1:") ||
            endpoint.starts_with("http://localhost:"));
        let ns = S3Keyspace::new("akula-sync/head-ci", VaultId::new()).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "akula-manifest-test".into(),
            region: "us-east-1".into(),
            endpoint: Some(endpoint),
            force_path_style: true,
        };
        let first = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
        let second = AwsS3ProbeClient::connect(profile, ns.clone()).await.unwrap();
        first.client.create_bucket().bucket("akula-manifest-test").send().await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut j1 = S3HeadJournal::open(&dir.path().join("first.sqlite")).unwrap();
        let mut j2 = S3HeadJournal::open(&dir.path().join("second.sqlite")).unwrap();
        // No root is not an authorized *empty listing*. The caller has to
        // handle initial S3 bootstrap explicitly.
        assert!(first.materialize_verified_revision_metadata_store(256 * 1024)
            .await.unwrap().is_none());

        // First generation includes a real per-document immutable revision
        // and blob. HEAD/GET/BLAKE3 verifies *every* layer before root CAS.
        let blob = b"v1";
        let blob_hash = blake3::hash(blob).to_hex().to_string();
        let bkey = ns.immutable_blob_key(&blob_hash).unwrap();
        first.put_conditionally(&bkey, blob, S3PutCondition::IfAbsent).await.unwrap();
        let record = RevisionRecord {
            id: RevisionId::new(), document_id: DocumentId::new(),
            parents: vec![],
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("Notes/hello.md").unwrap(),
                blob_hash,
            },
        };
        let revbytes = serde_json::to_vec(&record).unwrap();
        let rid = record.id;
        first.put_conditionally(&ns.immutable_revision_key(rid),
            &revbytes, S3PutCondition::IfAbsent).await.unwrap();
        let shard = S3ManifestShardV1::new(&ns, OperationId::new(),
            vec![S3RevisionRefV1 {
                revision_id: rid, document_id: record.document_id,
                revision_blake3: blake3::hash(&revbytes).to_hex().to_string(),
                blob_bytes: Some(blob.len() as u64),
            }]).unwrap();
        let shard_bytes = shard.encode(&ns).unwrap();
        first.store_manifest_object(shard.shard_id, &shard_bytes).await.unwrap();
        let root = S3ManifestV1::new(&ns, OperationId::new(), 1,
            None, vec![shard.reference(&ns).unwrap()]).unwrap();
        first.store_manifest_object(root.manifest_id, &root.encode(&ns).unwrap()).await.unwrap();
        let ptr = S3HeadPointerV1::new(&ns, 1, root.manifest_id, &root.digest(&ns).unwrap(), None).unwrap();
        assert_eq!(first.publish_head_cas(&mut j1, root.manifest_id, ptr.clone(), None)
            .await.unwrap(), S3HeadPublication::Published);
        assert_eq!(second.read_verified_head().await.unwrap().unwrap().pointer, ptr);
        let before = first.read_verified_head().await.unwrap().unwrap();
        assert_eq!(first.verify_manifest_ancestors(&before, 100).await.unwrap(), 0);
        let pinned_generation_one = second.pin_revision_snapshot().await.unwrap().unwrap();
        assert_eq!(pinned_generation_one.len(), 1);
        assert_eq!(pinned_generation_one.page(None, 100).unwrap().revisions[0].revision_id, rid);
        assert!(second.is_snapshot_current(&pinned_generation_one).await.unwrap());

        // A correctly linked successor must not silently drop earlier
        // revisions. Neither an intact root hash nor a CAS guard makes
        // omission safe: the remote head must remain unchanged.
        let dangerous = S3ManifestV1::new(&ns, OperationId::new(), 2,
            Some((root.manifest_id, root.digest(&ns).unwrap())), vec![]).unwrap();
        first.store_manifest_object(
            dangerous.manifest_id, &dangerous.encode(&ns).unwrap(),
        ).await.unwrap();
        let unsafe_head = S3HeadPointerV1::new(&ns, 2, dangerous.manifest_id,
            &dangerous.digest(&ns).unwrap(), Some(root.manifest_id)).unwrap();
        assert!(matches!(first.publish_head_cas(
            &mut j1, dangerous.manifest_id, unsafe_head, Some(&before),
        ).await, Err(S3HeadError::InvalidManifest)));
        assert_eq!(first.read_verified_head().await.unwrap().unwrap().pointer, ptr);
        assert_eq!(j1.phase(dangerous.manifest_id).unwrap(), None);

        // Two independent clients modify the SAME document while offline.
        // They each keep the base revision and upload a different immutable
        // sibling, with different BLAKE3 blobs and independent revision IDs.
        let (winner_revision, winner_ref) =
            upload_sibling(&first, &ns, &record, b"alice edits while offline").await;
        let (loser_revision, loser_ref) =
            upload_sibling(&second, &ns, &record, b"bob edits while offline").await;
        let base_ref = shard.revisions[0].clone();
        let winner_shard = publish_revision_shard(
            &first, &ns, vec![base_ref.clone(), winner_ref.clone()],
        ).await;
        let loser_shard = publish_revision_shard(
            &second, &ns, vec![base_ref.clone(), loser_ref.clone()],
        ).await;

        let id_winner = OperationId::new();
        let winner = S3ManifestV1::new(&ns, id_winner, 2,
            Some((root.manifest_id, root.digest(&ns).unwrap())),
            vec![winner_shard]).unwrap();
        first.store_manifest_object(id_winner, &winner.encode(&ns).unwrap()).await.unwrap();
        let pw = S3HeadPointerV1::new(&ns, 2, id_winner,
            &winner.digest(&ns).unwrap(), Some(root.manifest_id)).unwrap();

        let id_loser = OperationId::new();
        let loser = S3ManifestV1::new(&ns, id_loser, 2,
            Some((root.manifest_id, root.digest(&ns).unwrap())),
            vec![loser_shard]).unwrap();
        second.store_manifest_object(id_loser, &loser.encode(&ns).unwrap()).await.unwrap();
        let pl = S3HeadPointerV1::new(&ns, 2, id_loser,
            &loser.digest(&ns).unwrap(), Some(root.manifest_id)).unwrap();

        assert_eq!(first.publish_head_cas(&mut j1, id_winner, pw.clone(), Some(&before))
            .await.unwrap(), S3HeadPublication::Published);
        assert!(matches!(second.publish_head_cas(&mut j2, id_loser, pl.clone(), Some(&before)).await,
            Err(S3HeadError::Conflict)));
        assert_eq!(first.read_verified_head().await.unwrap().unwrap().pointer, pw);
        // Old pinned pages remain unchanged, but must never be mistaken for
        // the new current generation after the winning CAS.
        assert!(!second.is_snapshot_current(&pinned_generation_one).await.unwrap());
        assert_eq!(pinned_generation_one.page(None, 1).unwrap().revisions[0].revision_id, rid);
        assert!(second.read_verified_manifest(id_loser, &loser.digest(&ns).unwrap()).await.is_ok());
        // A rejected CAS must not drop Bob's immutable revision or blob.
        // Its root is not current, but it can be recovered/reconciled later.
        let loser_blob = match &loser_revision.operation {
            RevisionOperation::Content { blob_hash, .. } => ns.immutable_blob_key(blob_hash).unwrap(),
            _ => unreachable!(),
        };
        assert_eq!(second.get_bounded(&loser_blob, 128).await.unwrap().bytes,
            b"bob edits while offline");
        assert_eq!(second.get_bounded(
            &ns.immutable_revision_key(loser_revision.id), 4096,
        ).await.unwrap().bytes, serde_json::to_vec(&loser_revision).unwrap());
        // A real failed CAS must generate a *read-only* plan against the
        // winner's freshly verified root. Both divergent edits are retained,
        // and the prepared loser receipt must remain unacknowledged.
        let mut offline_dag = RevisionDag::new();
        offline_dag.ingest(record.clone()).unwrap();
        offline_dag.ingest(loser_revision.clone()).unwrap();
        let local_inventory = crate::sync::RevisionInventory {
            vault_id: ns.vault_id(),
            snapshot_id: "pinned-offline-bob-scan".into(),
            complete: true,
            revisions: [record.id, loser_revision.id].into_iter().collect(),
        };
        let proposal = second.preview_replan_after_s3_cas_conflict(
            &before, &offline_dag, &local_inventory, 256 * 1024,
        ).await.unwrap();
        assert_eq!(proposal.exchange.to_remote.revisions, vec![loser_revision.id]);
        assert_eq!(proposal.exchange.to_local.revisions, vec![winner_revision.id]);
        assert_eq!(proposal.conflicting_heads.get(&record.document_id), Some(
            &[winner_revision.id, loser_revision.id].into_iter().collect()
        ));
        assert_eq!(j2.phase(id_loser).unwrap(), Some(S3HeadAttemptPhase::Prepared));
        assert_eq!(second.read_verified_head().await.unwrap().unwrap().pointer, pw);
        let current_head = second.read_verified_head().await.unwrap().unwrap();
        assert!(matches!(second.preview_replan_after_s3_cas_conflict(
            &current_head, &offline_dag, &local_inventory, 256 * 1024,
        ).await, Err(super::super::replan::S3ReplanError::UnchangedCurrentHead)));
        // An uncertain retry of the losing operation cannot publish on top of
        // the winner: caller must explicitly reconcile its proposed branch.
        assert!(matches!(second.publish_head_cas(&mut j2, id_loser, pl.clone(), Some(&before)).await,
            Err(S3HeadError::RecoveryRequired)));

        // Simulate lost HTTP acknowledgement after successful server PUT but
        // before journal mark_published; recovery must recognize exact root.
        let now = first.read_verified_head().await.unwrap().unwrap();
        // Reconcile the failed CAS into a later generation. Both sibling
        // revisions remain heads in the DAG; no silent last-writer-wins.
        let merged_shard = publish_revision_shard(&first, &ns,
            vec![base_ref, winner_ref, loser_ref],
        ).await;
        let final_manifest = S3ManifestV1::new(&ns, OperationId::new(), 3,
            Some((id_winner, winner.digest(&ns).unwrap())),
            vec![merged_shard]).unwrap();
        first.store_manifest_object(final_manifest.manifest_id,
            &final_manifest.encode(&ns).unwrap()).await.unwrap();
        let final_ptr = S3HeadPointerV1::new(&ns, 3, final_manifest.manifest_id,
            &final_manifest.digest(&ns).unwrap(), Some(id_winner)).unwrap();
        let final_bytes = final_ptr.encode(&ns).unwrap();
        let mut journal = S3HeadJournal::open(&dir.path().join("lost-ack.sqlite")).unwrap();
        journal.begin(final_manifest.manifest_id, ns.vault_id(),
            &ns.current_head_key(), Some(&now.etag), &final_bytes).unwrap();
        first.put_conditionally(&ns.current_head_key(), &final_bytes,
            S3PutCondition::IfMatch(now.etag.clone())).await.unwrap();
        drop(journal);
        let mut journal = S3HeadJournal::open(&dir.path().join("lost-ack.sqlite")).unwrap();
        assert_eq!(first.publish_head_cas(
            &mut journal, final_manifest.manifest_id, final_ptr.clone(), Some(&now),
        ).await.unwrap(), S3HeadPublication::Recovered);
        assert_eq!(journal.phase(final_manifest.manifest_id).unwrap(),
            Some(S3HeadAttemptPhase::Published));
        let observed = second.read_verified_head().await.unwrap().unwrap();
        assert_eq!(observed.pointer, final_ptr);
        assert_eq!(second.verify_manifest_ancestors(&observed, 100).await.unwrap(), 2);
        let snapshot = second.pin_revision_snapshot().await.unwrap().unwrap();
        assert_eq!(snapshot.len(), 3);
        let revision_ids: BTreeSet<_> = snapshot.page(None, 50).unwrap()
            .revisions.into_iter().map(|r| r.revision_id).collect();
        assert_eq!(revision_ids, BTreeSet::from([
            record.id, winner_revision.id, loser_revision.id,
        ]));
        // SYNC-05d2: materialize only the verified revision JSON in a
        // bounded synchronous RemoteStore. The provider must never expose
        // a mutation capability, despite the underlying SDK supporting CAS.
        let mut view = second.materialize_verified_revision_metadata_store(
            256 * 1024,
        ).await.unwrap().unwrap();
        // Real MinIO readback must bind the complete inventory to the same
        // S3 vault, with all three sibling/ancestor revisions preserved.
        let inventory = view.verified_revision_inventory().unwrap();
        assert_eq!(inventory.vault_id, ns.vault_id());
        assert!(inventory.complete);
        assert_eq!(inventory.revisions, revision_ids);
        let assembled = crate::sync::collect_verified_listing(&view, 1, 3).unwrap();
        assert_eq!(assembled.objects.len(), 3);
        assert_eq!(assembled.snapshot_id, view.snapshot_id());
        assert!(!crate::sync::RemoteStore::capabilities(&view).supports_guarded_mutation());
        for row in &assembled.objects {
            let verified_json = crate::sync::RemoteStore::read_bounded(
                &view, &row.id, row.byte_length,
            ).unwrap();
            assert_eq!(blake3::hash(&verified_json).to_hex().as_str(),
                row.content_hash.as_deref().unwrap());
            let _: RevisionRecord = serde_json::from_slice(&verified_json).unwrap();
        }
        assert_eq!(crate::sync::RemoteStore::put(
            &mut view, "revisions/forbidden", b"write",
            &crate::sync::RemotePrecondition::Absent,
        ), Err(crate::sync::RemoteStoreError::PreconditionUnsupported));
        assert_eq!(crate::sync::RemoteStore::delete(
            &mut view, "revisions/forbidden",
            &crate::sync::RemotePrecondition::Absent,
        ), Err(crate::sync::RemoteStoreError::PreconditionUnsupported));
        let mut recovered_dag = RevisionDag::new();
        for revision in [record, winner_revision.clone(), loser_revision.clone()] {
            recovered_dag.ingest(revision).unwrap();
        }
        assert_eq!(recovered_dag.heads(winner_revision.document_id),
            BTreeSet::from([winner_revision.id, loser_revision.id]));
        assert_eq!(second.get_bounded(&loser_blob, 128).await.unwrap().bytes,
            b"bob edits while offline");
    }
}
