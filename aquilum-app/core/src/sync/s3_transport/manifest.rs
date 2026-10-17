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

impl AwsS3ProbeClient {
    /// Retrieve a BLAKE3-pinned immutable manifest and verify every shard,
    /// per-document revision DAG and referenced blob, with fixed read bounds.
    /// This does NOT use ListObjectsV2 or establish a local FS write right.
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
            let shard = S3ManifestShardV1::decode(ns, shard_ref.shard_id, &shard_data.bytes)?;
            if shard.revisions.len() != shard_ref.count as usize
                || shard.revisions.first().map(|r| r.revision_id) != Some(shard_ref.first_revision)
                || shard.revisions.last().map(|r| r.revision_id) != Some(shard_ref.last_revision)
            { return Err(S3HeadError::InvalidManifest); }
            for reference in shard.revisions {
                if !unique_revisions.insert(reference.revision_id) {
                    return Err(S3HeadError::InvalidRevision);
                }
                let rkey = ns.immutable_revision_key(reference.revision_id);
                let rev_data = self.get_bounded(&rkey, MAX_REVISION_JSON_BYTES).await?;
                if rev_data.blake3 != reference.revision_blake3 {
                    return Err(S3HeadError::IntegrityMismatch);
                }
                let revision: RevisionRecord = serde_json::from_slice(&rev_data.bytes)
                    .map_err(|_| S3HeadError::InvalidRevision)?;
                if revision.id != reference.revision_id
                    || revision.document_id != reference.document_id
                { return Err(S3HeadError::InvalidRevision); }
                match &revision.operation {
                    RevisionOperation::Tombstone { .. } if reference.blob_bytes.is_none() => {}
                    RevisionOperation::Content { blob_hash, .. }
                    | RevisionOperation::Rename { blob_hash, .. } => {
                        let bytes = reference.blob_bytes.ok_or(S3HeadError::InvalidRevision)?;
                        // A second reference to one content-addressed blob must
                        // declare the same length. Verify each distinct blob once.
                        match unique_blobs.insert(blob_hash.clone(), bytes) {
                            Some(previous) if previous != bytes => {
                                return Err(S3HeadError::InvalidRevision);
                            }
                            _ => {}
                        }
                    }
                    _ => return Err(S3HeadError::InvalidRevision),
                }
                dag.ingest(revision)?;
                total += 1;
                if total > MAX_REVISIONS { return Err(S3HeadError::InvalidManifest); }
            }
        }
        if total != manifest.total_revisions as usize || dag.pending_len() != 0
            || dag.active_len() != total
        { return Err(S3HeadError::InvalidManifest); }

        for (hash, len) in unique_blobs {
            let bkey = ns.immutable_blob_key(&hash)
                .map_err(|_| S3HeadError::InvalidRevision)?;
            let verified = self.digest_stream_bounded(&bkey, len).await?;
            if verified.blake3 != hash || verified.object.byte_length != len {
                return Err(S3HeadError::IntegrityMismatch);
            }
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
        assert_eq!(j2.phase(id_loser).unwrap(), Some(S3HeadAttemptPhase::Prepared));
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
