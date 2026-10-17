//! SYNC-DELTA-01g: selective, read-only verification of newly referenced
//! immutable S3 revision metadata and new content blobs.
//!
//! This is an *advisory preview*, NOT a fully pinned new remote snapshot,
//! File Gate admission, remote publication, or confirmed document baseline.
//! Previously pinned objects are reused only by exact immutable hash/identity
//! under a previously verified, profile-bound SQLite S3 remote anchor.
//! Earlier blobs are NOT re-probed for their *current availability*: the user
//! vault must not be mutated based on this preview. Before enabling user sync,
//! provide a separately approved retention and protected apply contract.
//!
//! This narrow initial slice supports unchanged head and exactly ONE complete
//! successor generation; multiple missed generations require verified-history
//! fallback so an intermediate sibling/tombstone can never disappear unseen.
//! No S3 ListObjectsV2, PUT, DELETE, GC or local filesystem mutation.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    AwsS3ProbeClient, S3HeadError, S3TransportError,
    S3VerifiedRevisionMetadataStore,
    MAX_S3_REVISION_METADATA_CACHE_BYTES,
};
use crate::sync::{
    RevisionId, RevisionOperation, RevisionRecord,
    S3AnchoredWalkError, S3ManifestDeltaBase, S3ManifestDeltaLimits,
    S3ManifestDeltaOutcome, S3ManifestShardRefV1, S3ManifestShardV1,
    S3ManifestV1, S3RemoteAnchorError, S3RemoteAnchorJournal,
    S3RemoteAnchorScope, S3HeadPointerV1, S3RevisionRefV1,
    MAX_MANIFEST_BYTES, MAX_SHARD_BYTES, MAX_REVISIONS,
    walk_s3_delta_from_saved_anchor,
};

const MAX_REVISION_BYTES: u64 = 64 * 1024;

/// A read-only count/provenance receipt; deliberately does NOT implement
/// RemoteStore or expose authority to plan local deletes, edits or baseline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3SelectiveRevisionPreview {
    pub remote_snapshot_id: String,
    pub total_revisions: usize,
    pub reused_shards: usize,
    pub checked_new_shards: usize,
    pub reused_revision_json: usize,
    pub downloaded_revision_json: usize,
    pub newly_verified_blobs: usize,
    pub new_revision_ids: Vec<RevisionId>,
    /// Always true: this is a metadata-only advisory record, *not* evidence
    /// of the ongoing availability of already-verified immutable S3 blobs.
    pub requires_full_apply_gate: bool,
}

#[derive(Debug)]
pub enum S3SelectiveReadError {
    Anchor(S3RemoteAnchorError),
    Walk(S3AnchoredWalkError),
    Head(S3HeadError),
    Transport(S3TransportError),
    CacheDoesNotMatchDurableAnchor,
    /// No complete immutable S3/manifest retention evidence for an offline
    /// chain longer than one generation. Never silently ignore intermediate
    /// tombstone or concurrent offline sibling records.
    VerifiedFullHistoryRequired,
    ImmutableRevisionRewritten,
    MissingPreviouslyVerifiedRevision,
    InconsistentContentLength,
    HeadChangedDuringVerification,
}

impl From<S3RemoteAnchorError> for S3SelectiveReadError {
    fn from(v: S3RemoteAnchorError) -> Self { Self::Anchor(v) }
}
impl From<S3AnchoredWalkError> for S3SelectiveReadError {
    fn from(v: S3AnchoredWalkError) -> Self { Self::Walk(v) }
}
impl From<S3HeadError> for S3SelectiveReadError {
    fn from(v: S3HeadError) -> Self { Self::Head(v) }
}
impl From<S3TransportError> for S3SelectiveReadError {
    fn from(v: S3TransportError) -> Self { Self::Transport(v) }
}

fn snapshot_id(head: &S3HeadPointerV1) -> String {
    format!("{}:{}", head.manifest_id(), head.manifest_blake3())
}

fn expected_blob(
    reference: &S3RevisionRefV1,
    json: &[u8],
) -> Result<Option<(String, u64)>, S3SelectiveReadError> {
    let record: RevisionRecord = serde_json::from_slice(json)
        .map_err(|_| S3SelectiveReadError::Head(S3HeadError::InvalidRevision))?;
    match record.operation {
        RevisionOperation::Content { blob_hash, .. }
        | RevisionOperation::Rename { blob_hash, .. } => {
            let length = reference.blob_bytes
                .ok_or(S3SelectiveReadError::Head(S3HeadError::InvalidRevision))?;
            Ok(Some((blob_hash, length)))
        }
        RevisionOperation::Tombstone { .. } if reference.blob_bytes.is_none() => Ok(None),
        _ => Err(S3SelectiveReadError::Head(S3HeadError::InvalidRevision)),
    }
}

fn check_shard_ref(
    shard: &S3ManifestShardV1,
    reference: &S3ManifestShardRefV1,
) -> Result<(), S3SelectiveReadError> {
    if shard.revisions.len() != reference.count as usize
        || shard.revisions.first().map(|r| r.revision_id)
            != Some(reference.first_revision)
        || shard.revisions.last().map(|r| r.revision_id)
            != Some(reference.last_revision)
    {
        return Err(S3SelectiveReadError::Head(S3HeadError::InvalidManifest));
    }
    Ok(())
}

/// Compare an old verified cache with a canonical replacement shard and
/// reject any reuse of an immutable revision UUID with changed metadata.
fn cached_or_new(
    cache: &S3VerifiedRevisionMetadataStore,
    reference: &S3RevisionRefV1,
) -> Result<Option<Vec<u8>>, S3SelectiveReadError> {
    match cache.cached_revision(reference.revision_id) {
        Some((old, json)) if old == reference => Ok(Some(json.to_vec())),
        Some(_) => Err(S3SelectiveReadError::ImmutableRevisionRewritten),
        None => Ok(None),
    }
}

fn old_blobs(
    cache: &S3VerifiedRevisionMetadataStore,
) -> Result<BTreeMap<String, u64>, S3SelectiveReadError> {
    let mut blobs = BTreeMap::new();
    for (reference, json) in cache.verified_entries() {
        if let Some((hash, length)) = expected_blob(reference, json)? {
            if let Some(old) = blobs.insert(hash, length) {
                if old != length {
                    return Err(S3SelectiveReadError::InconsistentContentLength);
                }
            }
        }
    }
    Ok(blobs)
}

impl AwsS3ProbeClient {
    /// Read-only selective verification against an in-memory cache that was
    /// previously materialized by the existing FULL pinned S3 verifier.
    ///
    /// The cache's snapshot ID is checked against the DURABLE scope-bound
    /// SQLite anchor, not against any caller-supplied free-form checkpoint.
    /// We require one immediate successor generation, avoiding unverified
    /// intermediate history. Old verified JSON is reused by exact revision
    /// reference; new revision JSON and unique new blob bytes are checked by
    /// the canonical bounded transport. Entire resulting DAG and old-reference
    /// preservation are checked before returning any advisory preview.
    ///
    /// The caller MUST NOT turn this preview into deletion authority or a
    /// remote \`ConfirmedBaseline\`. Reuse does not prove old S3 objects are
    /// still reachable today, nor does it authenticate a remote writer.
    pub async fn inspect_selective_s3_revision_changes(
        &self,
        scope: &S3RemoteAnchorScope,
        journal: &S3RemoteAnchorJournal,
        previously_pinned: &S3VerifiedRevisionMetadataStore,
        max_metadata_bytes: usize,
        limits: S3ManifestDeltaLimits,
    ) -> Result<S3SelectiveRevisionPreview, S3SelectiveReadError> {
        // No network access until actual SDK transport matches scope and the
        // cache is demonstrably from the previously persisted S3 root.
        if !scope.matches_connection(self.bound_profile(), self.bound_keyspace()) {
            return Err(S3SelectiveReadError::CacheDoesNotMatchDurableAnchor);
        }
        let anchor = journal.load(scope)?
            .ok_or(S3SelectiveReadError::CacheDoesNotMatchDurableAnchor)?;
        if !previously_pinned.is_verified_for(
            self.keyspace.vault_id(), &snapshot_id(anchor.pointer()),
        ) {
            return Err(S3SelectiveReadError::CacheDoesNotMatchDurableAnchor);
        }
        if max_metadata_bytes == 0
            || max_metadata_bytes > MAX_S3_REVISION_METADATA_CACHE_BYTES
            || previously_pinned.total_bytes() > max_metadata_bytes
        {
            return Err(S3SelectiveReadError::Head(S3HeadError::SnapshotBudgetExceeded));
        }

        let delta = walk_s3_delta_from_saved_anchor(self, scope, journal, limits).await?;
        let remote = delta.observed_head.pointer();
        let remote_id = snapshot_id(remote);
        match delta.outcome {
            S3ManifestDeltaOutcome::HeadUnchanged => {
                return Ok(S3SelectiveRevisionPreview {
                    remote_snapshot_id: remote_id,
                    total_revisions: previously_pinned.len(),
                    reused_shards: 0,
                    checked_new_shards: 0,
                    reused_revision_json: previously_pinned.len(),
                    downloaded_revision_json: 0,
                    newly_verified_blobs: 0,
                    new_revision_ids: Vec::new(),
                    requires_full_apply_gate: true,
                });
            }
            S3ManifestDeltaOutcome::Complete {
                generations_traversed: 1,
                base: S3ManifestDeltaBase::VerifiedCheckpoint,
                ..
            } => {}
            _ => return Err(S3SelectiveReadError::VerifiedFullHistoryRequired),
        }

        // The walker already checked stable root and exact parent chain.
        // Pin the latest immutable root again and recheck it against the
        // walker's head; the final root read after selective I/O catches races.
        let raw = self.get_bounded(
            &self.keyspace.immutable_manifest_key(remote.manifest_id()),
            MAX_MANIFEST_BYTES as u64,
        ).await?;
        if raw.blake3 != remote.manifest_blake3() {
            return Err(S3SelectiveReadError::Head(S3HeadError::IntegrityMismatch));
        }
        let manifest = S3ManifestV1::decode(
            &self.keyspace, remote.manifest_id(), &raw.bytes,
        ).map_err(S3HeadError::from)?;
        if manifest.generation != remote.generation()
            || manifest.previous_manifest_id != Some(anchor.pointer().manifest_id())
            || manifest.previous_manifest_blake3.as_deref()
                != Some(anchor.pointer().manifest_blake3())
        {
            return Err(S3SelectiveReadError::Head(S3HeadError::InvalidManifest));
        }
        let prior = S3ManifestV1::decode(
            &self.keyspace, anchor.pointer().manifest_id(), anchor.manifest_bytes(),
        ).map_err(S3HeadError::from)?;
        let old_shards: BTreeMap<_,_> =
            prior.shards.iter().map(|r| (r.shard_id, r)).collect();

        // Entire old cache previously passed canonical pin_revision_snapshot.
        // Never re-fetch immutable unchanged blobs here; but do NOT produce
        // a new "fully current/pinned" RemoteStore or File Gate authorization.
        let mut known_blobs = old_blobs(previously_pinned)?;
        let mut next = S3VerifiedRevisionMetadataStore::new(
            self.keyspace.vault_id(), remote_id.clone(), max_metadata_bytes,
        )?;
        let mut seen = BTreeSet::new();
        let mut reused_shards = 0;
        let mut checked_new_shards = 0;
        let mut reused_revisions = 0;
        let mut fetched_revisions = 0;
        let mut checked_blobs = 0;
        let mut new_revision_ids = Vec::new();

        for shard_ref in &manifest.shards {
            let stable = old_shards.get(&shard_ref.shard_id)
                .is_some_and(|old| *old == shard_ref);
            if stable {
                let entries: Vec<_> = previously_pinned.verified_entries()
                    .filter(|(reference, _)| {
                        reference.revision_id >= shard_ref.first_revision
                            && reference.revision_id <= shard_ref.last_revision
                    })
                    .collect();
                if entries.len() != shard_ref.count as usize
                    || entries.first().map(|(r,_)|r.revision_id)
                        != Some(shard_ref.first_revision)
                    || entries.last().map(|(r,_)|r.revision_id)
                        != Some(shard_ref.last_revision)
                {
                    return Err(S3SelectiveReadError::Head(S3HeadError::InvalidManifest));
                }
                for (reference, json) in entries {
                    next.accept(reference.clone(), json.to_vec())?;
                    if !seen.insert(reference.revision_id) {
                        return Err(S3SelectiveReadError::Head(S3HeadError::InvalidRevision));
                    }
                    reused_revisions += 1;
                }
                reused_shards += 1;
                continue;
            }

            // Rebuilt shards can contain old revisions and tombstones.
            // Reuse old revision JSON only after matching its exact immutable
            // reference, not by file path, timestamp or blob size.
            checked_new_shards += 1;
            let payload = self.get_bounded(
                &self.keyspace.immutable_manifest_key(shard_ref.shard_id),
                MAX_SHARD_BYTES as u64,
            ).await?;
            if payload.blake3 != shard_ref.shard_blake3 {
                return Err(S3SelectiveReadError::Head(S3HeadError::IntegrityMismatch));
            }
            let shard = S3ManifestShardV1::decode(
                &self.keyspace, shard_ref.shard_id, &payload.bytes,
            ).map_err(S3HeadError::from)?;
            check_shard_ref(&shard, shard_ref)?;
            for reference in shard.revisions {
                if !seen.insert(reference.revision_id) {
                    return Err(S3SelectiveReadError::Head(S3HeadError::InvalidRevision));
                }
                if let Some(cached) = cached_or_new(previously_pinned, &reference)? {
                    next.accept(reference, cached)?;
                    reused_revisions += 1;
                    continue;
                }
                let raw = self.get_bounded(
                    &self.keyspace.immutable_revision_key(reference.revision_id),
                    MAX_REVISION_BYTES,
                ).await?;
                if raw.blake3 != reference.revision_blake3 {
                    return Err(S3SelectiveReadError::Head(S3HeadError::IntegrityMismatch));
                }
                next.accept(reference.clone(), raw.bytes.clone())?;

                if let Some((hash, length)) = expected_blob(&reference, &raw.bytes)? {
                    if let Some(old) = known_blobs.get(&hash) {
                        if *old != length {
                            return Err(S3SelectiveReadError::InconsistentContentLength);
                        }
                    } else {
                        let key = self.keyspace.immutable_blob_key(&hash)
                            .map_err(|_| S3SelectiveReadError::Head(S3HeadError::InvalidRevision))?;
                        let checked = self.digest_stream_bounded(&key, length).await?;
                        if checked.blake3 != hash || checked.object.byte_length != length {
                            return Err(S3SelectiveReadError::Head(S3HeadError::IntegrityMismatch));
                        }
                        known_blobs.insert(hash, length);
                        checked_blobs += 1;
                    }
                }
                new_revision_ids.push(reference.revision_id);
                fetched_revisions += 1;
            }
        }

        if next.len() != manifest.total_revisions as usize
            || next.len() > MAX_REVISIONS
        {
            return Err(S3SelectiveReadError::Head(S3HeadError::InvalidManifest));
        }
        // Publication is append-only. Not just identical shard refs: every
        // old revision, tombstone and both offline sibling heads MUST survive
        // any changed/resharded representation, with exact immutable metadata.
        for (reference, _) in previously_pinned.verified_entries() {
            if !seen.contains(&reference.revision_id) {
                return Err(S3SelectiveReadError::MissingPreviouslyVerifiedRevision);
            }
        }
        next.validate_complete_dag()?;

        let after = self.get_bounded(&self.keyspace.current_head_key(), 4096).await?;
        let current = S3HeadPointerV1::decode(&after.bytes, &self.keyspace)
            .map_err(|_| S3SelectiveReadError::Head(S3HeadError::InvalidManifest))?;
        if current != *remote || after.object.etag != delta.observed_head.etag() {
            return Err(S3SelectiveReadError::HeadChangedDuringVerification);
        }

        new_revision_ids.sort_unstable();
        Ok(S3SelectiveRevisionPreview {
            remote_snapshot_id: remote_id,
            total_revisions: next.len(),
            reused_shards,
            checked_new_shards,
            reused_revision_json: reused_revisions,
            downloaded_revision_json: fetched_revisions,
            newly_verified_blobs: checked_blobs,
            new_revision_ids,
            requires_full_apply_gate: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{DocumentId, SyncRelativePath, VaultId};
    use super::super::snapshot_store::S3VerifiedRevisionMetadataStore;

    fn fixture(
        doc: DocumentId,
        id: RevisionId,
        parents: Vec<RevisionId>,
        data: &[u8],
    ) -> (S3RevisionRefV1, Vec<u8>) {
        let record = RevisionRecord {
            id, document_id: doc, parents,
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/fixture.md").unwrap(),
                blob_hash: blake3::hash(data).to_hex().to_string(),
            },
        };
        let raw = serde_json::to_vec(&record).unwrap();
        (S3RevisionRefV1 {
            revision_id: id,
            document_id: doc,
            revision_blake3: blake3::hash(&raw).to_hex().to_string(),
            blob_bytes: Some(data.len() as u64),
        }, raw)
    }

    fn cached(refs: Vec<(S3RevisionRefV1, Vec<u8>)>) -> S3VerifiedRevisionMetadataStore {
        let mut store = S3VerifiedRevisionMetadataStore::new(
            VaultId::new(), "pinned-prior-root".into(), 2*1024*1024,
        ).unwrap();
        for (r, b) in refs { store.accept(r,b).unwrap(); }
        store.validate_complete_dag().unwrap();
        store
    }

    #[test]
    fn cache_reuses_identical_immutable_revision_only() {
        let doc = DocumentId::new();
        let id = RevisionId::new();
        let first = fixture(doc, id, vec![], b"old-content");
        let mut store = cached(vec![first.clone()]);
        assert_eq!(cached_or_new(&store, &first.0).unwrap(), Some(first.1.clone()));
        let rewritten = fixture(doc, id, vec![], b"malicious-rewrite");
        assert!(matches!(
            cached_or_new(&store, &rewritten.0),
            Err(S3SelectiveReadError::ImmutableRevisionRewritten)
        ));
        assert_eq!(old_blobs(&store).unwrap().len(), 1);
        let tombstone = RevisionRecord {
            id: RevisionId::new(),
            document_id: doc,
            parents: vec![id],
            operation: RevisionOperation::Tombstone {
                path: SyncRelativePath::parse("notes/fixture.md").unwrap(),
            },
        };
        let raw = serde_json::to_vec(&tombstone).unwrap();
        let tref = S3RevisionRefV1 {
            revision_id: tombstone.id, document_id: doc,
            revision_blake3: blake3::hash(&raw).to_hex().to_string(),
            blob_bytes: None,
        };
        assert!(expected_blob(&tref, &raw).unwrap().is_none());
        store.accept(tref,raw).unwrap_err(); // Complete cache cannot be mutated.
    }

    #[test]
    fn previous_complete_cache_preserves_two_conflicting_offline_heads() {
        let doc = DocumentId::new();
        let base = RevisionId::new();
        let left = RevisionId::new();
        let right = RevisionId::new();
        let old = cached(vec![
            fixture(doc, base, vec![], b"base"),
            fixture(doc, left, vec![base], b"laptop-a"),
            fixture(doc, right, vec![base], b"laptop-b"),
        ]);
        assert_eq!(old.len(), 3);
        for id in [base,left,right] {
            assert!(old.cached_revision(id).is_some());
        }
        assert!(old.is_verified_for(old.verified_revision_inventory().unwrap().vault_id, "pinned-prior-root"));
    }

    #[test]
    fn blob_lengths_and_tombstones_are_checked_without_writes() {
        let doc = DocumentId::new();
        let id = RevisionId::new();
        let (mut r, bytes) = fixture(doc, id, vec![], b"abc");
        assert_eq!(expected_blob(&r, &bytes).unwrap().unwrap().1, 3);
        r.blob_bytes = None;
        assert!(matches!(
            expected_blob(&r, &bytes),
            Err(S3SelectiveReadError::Head(S3HeadError::InvalidRevision))
        ));
    }

    /// End-to-end AWS SDK + disposable MinIO. Exercises the actual
    /// canonical full pinning of genesis, durable SQLite anchor, then
    /// *selective* verification of one newly published revision/blob.
    /// Never touches real buckets or user vaults.
    #[tokio::test]
    #[ignore = "requires isolated localhost MinIO and CI-only credentials"]
    async fn live_minio_selective_one_edit_reuses_unchanged_revision_and_blob() {
        use crate::sync::{
            OperationId, S3ConnectionProfile, S3Keyspace, S3PutCondition,
            S3ManifestShardV1, S3RemoteAnchorScope, S3RemoteAnchorJournal,
            S3HeadPointerV1, S3ManifestDeltaLimits,
        };
        use tempfile::tempdir;

        let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT")
            .expect("isolated localhost MinIO required");
        assert!(
            endpoint.starts_with("http://127.0.0.1:")
                || endpoint.starts_with("http://localhost:"),
        );
        let bucket = format!("akula-selective-{}", uuid::Uuid::new_v4().simple());
        let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new("us-east-1"))
            .load()
            .await;
        let sdk = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::config::Builder::from(&shared)
                .endpoint_url(endpoint.clone()).force_path_style(true).build(),
        );
        sdk.create_bucket().bucket(bucket.clone()).send().await.unwrap();
        let profile = S3ConnectionProfile {
            bucket, region: "us-east-1".into(),
            endpoint: Some(endpoint), force_path_style: true,
        };
        let ns = S3Keyspace::new("akula/selective", VaultId::new()).unwrap();
        let client = AwsS3ProbeClient::connect(profile.clone(), ns.clone())
            .await.unwrap();
        let scope = S3RemoteAnchorScope::new(
            "selective-ci", "verified-disposable-ci-minio",
            &profile, &ns,
        ).unwrap();

        let (old_ref, old_json) = fixture(
            DocumentId::new(), RevisionId::new(), vec![], b"old-blob",
        );
        let old_blob = b"old-blob";
        let old_record: RevisionRecord = serde_json::from_slice(&old_json).unwrap();
        let old_hash = match old_record.operation {
            RevisionOperation::Content { blob_hash, .. } => blob_hash,
            _ => unreachable!(),
        };
        let old_shard = S3ManifestShardV1::new(
            &ns, OperationId::new(), vec![old_ref.clone()],
        ).unwrap();
        let old_shard_bytes = old_shard.encode(&ns).unwrap();
        let old_shard_ref = old_shard.reference(&ns).unwrap();
        let old_manifest = S3ManifestV1::new(
            &ns, OperationId::new(), 1, None, vec![old_shard_ref.clone()],
        ).unwrap();
        let old_manifest_bytes = old_manifest.encode(&ns).unwrap();
        let old_head = S3HeadPointerV1::new(
            &ns, 1, old_manifest.manifest_id,
            &blake3::hash(&old_manifest_bytes).to_hex().to_string(), None,
        ).unwrap();

        for (key, bytes) in [
            (ns.immutable_blob_key(&old_hash).unwrap(), old_blob.to_vec()),
            (ns.immutable_revision_key(old_ref.revision_id), old_json.clone()),
            (ns.immutable_manifest_key(old_shard.shard_id), old_shard_bytes),
            (ns.immutable_manifest_key(old_manifest.manifest_id), old_manifest_bytes.clone()),
        ] {
            client.put_conditionally(&key, &bytes, S3PutCondition::IfAbsent)
                .await.unwrap();
        }
        let first_write = client.put_conditionally(
            &ns.current_head_key(), &old_head.encode(&ns).unwrap(),
            S3PutCondition::IfAbsent,
        ).await.unwrap();

        let old_cache = client.materialize_verified_revision_metadata_store(512*1024)
            .await.unwrap().unwrap();
        let verified_head = client.read_verified_head().await.unwrap().unwrap();
        let pinned = client.pin_revision_snapshot().await.unwrap().unwrap();
        let dir = tempdir().unwrap();
        let path = dir.path().join("selective-anchor.sqlite");
        let mut journal = S3RemoteAnchorJournal::open(&path).unwrap();
        journal.record_verified_remote(
            &scope, &verified_head, &pinned, &[old_manifest_bytes],
        ).unwrap();
        drop(journal);
        let journal = S3RemoteAnchorJournal::open(&path).unwrap();

        let new_data = b"brand-new-blob";
        let (new_ref, new_json) = fixture(
            DocumentId::new(), RevisionId::new(), vec![], new_data,
        );
        let new_record: RevisionRecord = serde_json::from_slice(&new_json).unwrap();
        let new_hash = match new_record.operation {
            RevisionOperation::Content { blob_hash, .. } => blob_hash,
            _ => unreachable!(),
        };
        let new_shard = S3ManifestShardV1::new(
            &ns, OperationId::new(), vec![new_ref.clone()],
        ).unwrap();
        let new_shard_bytes = new_shard.encode(&ns).unwrap();
        let mut shard_refs = vec![old_shard_ref, new_shard.reference(&ns).unwrap()];
        shard_refs.sort_by_key(|shard|shard.first_revision);
        let new_manifest = S3ManifestV1::new(
            &ns, OperationId::new(), 2,
            Some((old_manifest.manifest_id, old_head.manifest_blake3().to_owned())),
            shard_refs,
        ).unwrap();
        let new_manifest_bytes = new_manifest.encode(&ns).unwrap();
        let new_head = S3HeadPointerV1::new(
            &ns, 2, new_manifest.manifest_id,
            &blake3::hash(&new_manifest_bytes).to_hex().to_string(),
            Some(old_manifest.manifest_id),
        ).unwrap();

        for (key, bytes) in [
            (ns.immutable_revision_key(new_ref.revision_id), new_json),
            (ns.immutable_manifest_key(new_shard.shard_id), new_shard_bytes),
            (ns.immutable_manifest_key(new_manifest.manifest_id), new_manifest_bytes),
        ] {
            client.put_conditionally(&key, &bytes, S3PutCondition::IfAbsent)
                .await.unwrap();
        }
        client.put_conditionally(
            &ns.current_head_key(), &new_head.encode(&ns).unwrap(),
            S3PutCondition::IfMatch(first_write.etag),
        ).await.unwrap();

        // Even correctly linked manifest/revision JSON cannot claim a valid
        // candidate until its *new* content blob was fetched and hashed.
        assert!(matches!(
            client.inspect_selective_s3_revision_changes(
                &scope, &journal, &old_cache, 512*1024,
                S3ManifestDeltaLimits::default(),
            ).await,
            Err(S3SelectiveReadError::Transport(S3TransportError::NotFound)),
        ));
        client.put_conditionally(
            &ns.immutable_blob_key(&new_hash).unwrap(),
            new_data, S3PutCondition::IfAbsent,
        ).await.unwrap();

        let inspected = client.inspect_selective_s3_revision_changes(
            &scope, &journal, &old_cache, 512*1024,
            S3ManifestDeltaLimits::default(),
        ).await.unwrap();
        assert_eq!(inspected.total_revisions, 2);
        assert_eq!(inspected.reused_shards, 1);
        assert_eq!(inspected.checked_new_shards, 1);
        assert_eq!(inspected.reused_revision_json, 1);
        assert_eq!(inspected.downloaded_revision_json, 1);
        assert_eq!(inspected.newly_verified_blobs, 1);
        assert_eq!(inspected.new_revision_ids, vec![new_ref.revision_id]);
        assert!(inspected.requires_full_apply_gate);

        // Preview is read-only: prior durable observation remains unchanged.
        assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &old_head);
    }

}


/// Explicitly invoked disposable-MinIO performance experiment. This test is
/// ignored in the standard suite and records observed SDK HEAD/GET attempts,
/// payload bytes and wall time for full pinning versus selective PREVIEW.
///
/// Important: a selective preview is NOT a fresh full-availability proof or
/// File Gate receipt. Speed ratios are discovery-only, not end-to-end sync.
/// Existing 10k S3 pagination tests are not a substitute for this workload.
#[cfg(test)]
mod selective_ab_benchmark {
    use super::*;
    use super::super::{S3TestIoCounts, S3PutCondition};
    use crate::sync::{
        DocumentId, OperationId, S3ConnectionProfile, S3HeadPointerV1,
        S3Keyspace, S3ManifestDeltaLimits, S3ManifestShardRefV1,
        S3RemoteAnchorJournal, S3RemoteAnchorScope, SyncRelativePath,
        VaultId, MAX_SHARD_ENTRIES,
    };
    use std::time::Instant;

    struct InitialShard {
        reference: S3ManifestShardRefV1,
        entries: Vec<S3RevisionRefV1>,
    }

    fn base_id(index: usize) -> RevisionId {
        RevisionId::from_uuid(uuid::Uuid::from_u128(
            ((index as u128 + 1) << 64) | 0x4000_0000_0000_0001,
        ))
    }

    fn updated_id(index: usize) -> RevisionId {
        RevisionId::from_uuid(uuid::Uuid::from_u128(
            ((index as u128 + 1) << 64) | 0x4000_0000_0000_0002,
        ))
    }

    fn document_id(index: usize) -> DocumentId {
        DocumentId::from_uuid(uuid::Uuid::from_u128(
            ((index as u128 + 1) << 64) | 0x5000_0000_0000_0001,
        ))
    }

    fn note(index: usize, edited: bool) -> (S3RevisionRefV1, Vec<u8>, Vec<u8>) {
        let blob = format!(
            "# Note {index:05}\n\nbench key: {index:05}; generation: {}\n",
            if edited { 2 } else { 1 },
        ).into_bytes();
        let id = if edited { updated_id(index) } else { base_id(index) };
        let record = RevisionRecord {
            id,
            document_id: document_id(index),
            parents: if edited { vec![base_id(index)] } else { Vec::new() },
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse(
                    &format!("notes/note-{index:05}.md"),
                ).unwrap(),
                blob_hash: blake3::hash(&blob).to_hex().to_string(),
            },
        };
        let json = serde_json::to_vec(&record).unwrap();
        let r = S3RevisionRefV1 {
            revision_id: id,
            document_id: record.document_id,
            revision_blake3: blake3::hash(&json).to_hex().to_string(),
            blob_bytes: Some(blob.len() as u64),
        };
        (r, json, blob)
    }

    async fn upload_objects(
        client: &AwsS3ProbeClient,
        entries: Vec<(String, Vec<u8>)>,
    ) {
        let mut workers = tokio::task::JoinSet::new();
        for (key, bytes) in entries {
            let client = client.clone();
            workers.spawn(async move {
                client.put_conditionally(
                    &key, &bytes, S3PutCondition::IfAbsent,
                ).await.expect("disposable MinIO immutable upload");
            });
            if workers.len() >= 32 {
                workers.join_next().await.unwrap().unwrap();
            }
        }
        while let Some(finished) = workers.join_next().await {
            finished.unwrap();
        }
    }

    async fn upload_shard(
        client: &AwsS3ProbeClient,
        ns: &S3Keyspace,
        entries: Vec<S3RevisionRefV1>,
    ) -> S3ManifestShardRefV1 {
        let shard = S3ManifestShardV1::new(ns, OperationId::new(), entries).unwrap();
        client.put_conditionally(
            &ns.immutable_manifest_key(shard.shard_id),
            &shard.encode(ns).unwrap(),
            S3PutCondition::IfAbsent,
        ).await.unwrap();
        shard.reference(ns).unwrap()
    }

    /// Preserve old shard IDs whose exact ranges do not contain any new IDs.
    /// Split only changed ranges into <=128-reference new shards, and create
    /// separate gap shards if new IDs fall between existing shard ranges.
    /// This is a benchmark fixture policy, NOT a production publisher change.
    async fn publish_stable_shards(
        client: &AwsS3ProbeClient,
        ns: &S3Keyspace,
        old: &[InitialShard],
        additions: Vec<S3RevisionRefV1>,
    ) -> Vec<S3ManifestShardRefV1> {
        let mut inside = vec![Vec::<S3RevisionRefV1>::new(); old.len()];
        let mut gaps = vec![Vec::<S3RevisionRefV1>::new(); old.len()+1];
        for reference in additions {
            let mut position = Some(reference);
            for (index, shard) in old.iter().enumerate() {
                let candidate = position.as_ref().unwrap();
                if candidate.revision_id < shard.reference.first_revision {
                    gaps[index].push(position.take().unwrap());
                    break;
                }
                if candidate.revision_id <= shard.reference.last_revision {
                    inside[index].push(position.take().unwrap());
                    break;
                }
            }
            if let Some(candidate) = position {
                gaps[old.len()].push(candidate);
            }
        }
        let mut result = Vec::new();
        for (index, shard) in old.iter().enumerate() {
            gaps[index].sort_by_key(|r| r.revision_id);
            for chunk in gaps[index].chunks(MAX_SHARD_ENTRIES) {
                result.push(upload_shard(client, ns, chunk.to_vec()).await);
            }
            if inside[index].is_empty() {
                result.push(shard.reference.clone());
            } else {
                let mut merged = shard.entries.clone();
                merged.append(&mut inside[index]);
                merged.sort_by_key(|r| r.revision_id);
                for chunk in merged.chunks(MAX_SHARD_ENTRIES) {
                    result.push(upload_shard(client, ns, chunk.to_vec()).await);
                }
            }
        }
        gaps[old.len()].sort_by_key(|r| r.revision_id);
        for chunk in gaps[old.len()].chunks(MAX_SHARD_ENTRIES) {
            result.push(upload_shard(client, ns, chunk.to_vec()).await);
        }
        // The canonical S3ManifestV1 constructor checks sorted disjoint ranges.
        result
    }

    fn report(
        label: &str,
        size: usize,
        updates: usize,
        counts: S3TestIoCounts,
        elapsed_ms: u128,
    ) {
        // JSON lines deliberately exclude credentials, paths, raw note text,
        // object URLs and any content-bearing S3 response data.
        eprintln!(
            "AKULA_DELTA_AB {}",
            serde_json::json!({
                "scenario": label,
                "notes": size,
                "new_revisions": updates,
                "elapsed_ms": elapsed_ms,
                "attempted_s3_head": counts.head,
                "attempted_s3_get": counts.get,
                "downloaded_payload_bytes": counts.payload_bytes,
                "security_scope": if label == "selective_preview" {
                    "advisory-verified-new-only"
                } else { "full-current-availability-pin" },
            })
        );
    }

    #[tokio::test]
    #[ignore = "manual disposable MinIO benchmark, S3 HTTP request attempts only"]
    async fn minio_selective_ab_notes_network_requests() {
        let size = std::env::var("AQ_DELTA_BENCH_NOTES")
            .unwrap_or_else(|_| "64".into()).parse::<usize>().unwrap();
        let updates = std::env::var("AQ_DELTA_BENCH_CHANGES")
            .unwrap_or_else(|_| "7".into()).parse::<usize>().unwrap();
        assert!((1..=10_000).contains(&size));
        assert!(updates <= size && updates <= 1_000);
        let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").unwrap();
        assert!(
            endpoint.starts_with("http://127.0.0.1:")
                || endpoint.starts_with("http://localhost:"),
            "only isolated disposable localhost MinIO is accepted",
        );

        let bucket = format!("akula-delta-ab-{}", uuid::Uuid::new_v4().simple());
        let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new("us-east-1")).load().await;
        let sdk = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::config::Builder::from(&shared)
                .endpoint_url(endpoint.clone()).force_path_style(true).build(),
        );
        sdk.create_bucket().bucket(bucket.clone()).send().await.unwrap();
        let profile = S3ConnectionProfile {
            bucket, region: "us-east-1".into(),
            endpoint: Some(endpoint), force_path_style: true,
        };
        let ns = S3Keyspace::new("akula/benchmark", VaultId::new()).unwrap();
        let client = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
        let scope = S3RemoteAnchorScope::new(
            "benchmark-ci", "verified-disposable-minio",
            &profile, &ns,
        ).unwrap();

        let mut uploads = Vec::with_capacity(size * 2);
        let mut refs = Vec::with_capacity(size);
        for i in 0..size {
            let (reference, json, blob) = note(i, false);
            let record: RevisionRecord = serde_json::from_slice(&json).unwrap();
            let hash = match record.operation {
                RevisionOperation::Content { blob_hash, .. } => blob_hash,
                _ => unreachable!(),
            };
            uploads.push((ns.immutable_revision_key(reference.revision_id), json));
            uploads.push((ns.immutable_blob_key(&hash).unwrap(), blob));
            refs.push(reference);
        }
        upload_objects(&client, uploads).await;
        refs.sort_by_key(|reference| reference.revision_id);

        let mut shards = Vec::new();
        for chunk in refs.chunks(MAX_SHARD_ENTRIES) {
            let entries = chunk.to_vec();
            let reference = upload_shard(&client, &ns, entries.clone()).await;
            shards.push(InitialShard { reference, entries });
        }
        let old_root = S3ManifestV1::new(
            &ns, OperationId::new(), 1, None,
            shards.iter().map(|s| s.reference.clone()).collect(),
        ).unwrap();
        let old_bytes = old_root.encode(&ns).unwrap();
        upload_objects(&client, vec![(
            ns.immutable_manifest_key(old_root.manifest_id), old_bytes.clone(),
        )]).await;
        let old_head = S3HeadPointerV1::new(
            &ns, 1, old_root.manifest_id,
            &blake3::hash(&old_bytes).to_hex().to_string(), None,
        ).unwrap();
        let old_put = client.put_conditionally(
            &ns.current_head_key(), &old_head.encode(&ns).unwrap(),
            S3PutCondition::IfAbsent,
        ).await.unwrap();

        // Setup intentionally performs the expensive FULL cold verification
        // to create a legitimate cache. It is outside both measured phases.
        let cache = client.materialize_verified_revision_metadata_store(
            MAX_S3_REVISION_METADATA_CACHE_BYTES,
        ).await.unwrap().unwrap();
        assert_eq!(cache.len(), size);
        let verified = client.read_verified_head().await.unwrap().unwrap();
        let pinned = client.pin_revision_snapshot().await.unwrap().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dbpath = tmp.path().join("prior-remote-observation.sqlite");
        let mut journal = S3RemoteAnchorJournal::open(&dbpath).unwrap();
        journal.record_verified_remote(
            &scope, &verified, &pinned, &[old_bytes],
        ).unwrap();
        drop(journal);
        let journal = S3RemoteAnchorJournal::open(&dbpath).unwrap();

        if updates != 0 {
            let mut new_refs = Vec::with_capacity(updates);
            let mut upload = Vec::with_capacity(updates * 2);
            for j in 0..updates {
                let i = j * size / updates;
                let (reference, json, blob) = note(i, true);
                let record: RevisionRecord = serde_json::from_slice(&json).unwrap();
                let hash = match record.operation {
                    RevisionOperation::Content { blob_hash, .. } => blob_hash,
                    _ => unreachable!(),
                };
                upload.push((ns.immutable_revision_key(reference.revision_id), json));
                upload.push((ns.immutable_blob_key(&hash).unwrap(), blob));
                new_refs.push(reference);
            }
            upload_objects(&client, upload).await;
            let successor_refs = publish_stable_shards(
                &client, &ns, &shards, new_refs,
            ).await;
            let new_root = S3ManifestV1::new(
                &ns, OperationId::new(), 2,
                Some((old_root.manifest_id, old_head.manifest_blake3().to_owned())),
                successor_refs,
            ).unwrap();
            upload_objects(&client, vec![(
                ns.immutable_manifest_key(new_root.manifest_id),
                new_root.encode(&ns).unwrap(),
            )]).await;
            let new_head = S3HeadPointerV1::new(
                &ns, 2, new_root.manifest_id, &new_root.digest(&ns).unwrap(),
                Some(old_root.manifest_id),
            ).unwrap();
            client.put_conditionally(
                &ns.current_head_key(), &new_head.encode(&ns).unwrap(),
                S3PutCondition::IfMatch(old_put.etag),
            ).await.unwrap();
        }

        let before_selective = client.test_io_snapshot();
        let started_selective = Instant::now();
        let preview = client.inspect_selective_s3_revision_changes(
            &scope, &journal, &cache,
            MAX_S3_REVISION_METADATA_CACHE_BYTES,
            S3ManifestDeltaLimits::default(),
        ).await.unwrap();
        let selective_ms = started_selective.elapsed().as_millis();
        let selective = client.test_io_snapshot().since(before_selective);
        assert_eq!(preview.total_revisions, size + updates);
        assert_eq!(preview.downloaded_revision_json, updates);
        assert_eq!(preview.newly_verified_blobs, updates);
        assert_eq!(preview.reused_revision_json, size);
        assert!(preview.requires_full_apply_gate);
        assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &old_head);
        report("selective_preview", size, updates, selective, selective_ms);

        let before_full = client.test_io_snapshot();
        let started_full = Instant::now();
        let pinned_now = client.pin_revision_snapshot().await.unwrap().unwrap();
        let full_ms = started_full.elapsed().as_millis();
        let full = client.test_io_snapshot().since(before_full);
        assert_eq!(pinned_now.len(), size + updates);
        report("full_verified_pin", size, updates, full, full_ms);
        // These differ in assurance; this is NOT an apples-to-apples
        // benchmark for permission to apply or delete user files.
        assert!(selective.get < full.get);
    }
}
