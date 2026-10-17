//! SYNC-DELTA-01k: verify revision retention in EVERY missed S3 generation.
//!
//! This checks the BLAKE3-linked, already bounded immutable manifest frames
//! returned by the remote-head walker, then checks each referenced immutable
//! shard (or reuses its exact previously verified reference). A valid newest
//! root alone is not proof that no intermediate tombstone/sibling was lost.
//!
//! IMPORTANT: this is a read-only, *metadata* retention gate. Final new JSON
//! and new blobs still require the existing selective verifier and complete
//! DAG closure. It does not authenticate a writer, re-prove the current
//! availability of prior blobs, or permit File Gate operations.

use std::collections::BTreeMap;

use crate::sync::{
    OperationId, RevisionId, S3HeadPointerV1, S3Keyspace,
    S3ManifestDeltaLimits, S3ManifestShardRefV1,
    S3ManifestShardV1, S3ManifestV1, S3RevisionRefV1,
    MAX_REVISIONS, MAX_SHARD_BYTES,
};
use crate::sync::s3_head_probe::{check_readback, BoundedReader};
use super::{
    S3HeadError, S3VerifiedRevisionMetadataStore,
    S3TransportError,
};
use super::selective_snapshot::S3SelectiveReadError;

const MAX_ADDITIONAL_HISTORY_SHARD_BYTES: usize = 64 * 1024 * 1024;

fn invalid_manifest() -> S3SelectiveReadError {
    S3SelectiveReadError::Head(S3HeadError::InvalidManifest)
}

fn verify_descriptor(
    shard: &S3ManifestShardV1,
    reference: &S3ManifestShardRefV1,
) -> Result<(), S3SelectiveReadError> {
    let items = &shard.revisions;
    if items.len() != reference.count as usize
        || items.first().map(|x| x.revision_id) != Some(reference.first_revision)
        || items.last().map(|x| x.revision_id) != Some(reference.last_revision)
    {
        return Err(invalid_manifest());
    }
    Ok(())
}

fn starting_shards(
    anchor: &S3ManifestV1,
    pinned: &S3VerifiedRevisionMetadataStore,
) -> Result<BTreeMap<OperationId, (S3ManifestShardRefV1, Vec<S3RevisionRefV1>)>, S3SelectiveReadError> {
    let mut entries = BTreeMap::<RevisionId, S3RevisionRefV1>::new();
    for (r, _) in pinned.verified_entries() {
        if entries.insert(r.revision_id, r.clone()).is_some() {
            return Err(invalid_manifest());
        }
    }
    if entries.len() != anchor.total_revisions as usize {
        return Err(invalid_manifest());
    }
    let mut shards = BTreeMap::new();
    let mut members = 0;
    for reference in &anchor.shards {
        let subset: Vec<_> = entries
            .range(reference.first_revision..=reference.last_revision)
            .map(|(_, item)| item.clone())
            .collect();
        if subset.len() != reference.count as usize
            || subset.first().map(|x| x.revision_id) != Some(reference.first_revision)
            || subset.last().map(|x| x.revision_id) != Some(reference.last_revision)
        {
            return Err(invalid_manifest());
        }
        members += subset.len();
        if shards.insert(reference.shard_id, (reference.clone(), subset)).is_some() {
            return Err(invalid_manifest());
        }
    }
    if members != entries.len() {
        return Err(invalid_manifest());
    }
    Ok(shards)
}

/// Reject any offline history in which a revision, conflict sibling or
/// tombstone appeared in one generation and was subsequently omitted, even
/// if the *latest* snapshot would have passed complete DAG validation.
///
/// All frames are newest-first and already root/head checked by
/// walk_s3_delta_from_saved_anchor; this module additionally verifies their
/// exact predecessor references and remote pointer against the durable anchor.
/// Unchanged shard entries can be reused by exact descriptor identity; new
/// shard objects are fetched through bounded BLAKE3-verified GETs. Only the
/// previous generation's materialized shard map is retained, bounding memory.
///
/// Returns metadata proof only. It NEVER returns a writable RemoteStore.
pub(super) async fn verify_offline_retention<R: BoundedReader>(
    reader: &R,
    ns: &S3Keyspace,
    anchor: &S3ManifestV1,
    pinned: &S3VerifiedRevisionMetadataStore,
    current: &S3HeadPointerV1,
    frames_newest_first: &[Vec<u8>],
    limits: S3ManifestDeltaLimits,
) -> Result<(), S3SelectiveReadError> {
    if frames_newest_first.len() < 2 || frames_newest_first.len() > limits.max_frames
        || frames_newest_first.len() > 128
        || pinned.len() > MAX_REVISIONS
    {
        return Err(S3SelectiveReadError::VerifiedFullHistoryRequired);
    }
    let mut previous_shards = starting_shards(anchor, pinned)?;
    let mut previous_entries: BTreeMap<RevisionId, S3RevisionRefV1> =
        pinned.verified_entries().map(|(r, _)| (r.revision_id, r.clone())).collect();
    let mut known_descriptors: BTreeMap<OperationId, S3ManifestShardRefV1> =
        anchor.shards.iter().map(|r| (r.shard_id, r.clone())).collect();

    let mut previous_root_id = anchor.manifest_id;
    let mut previous_root_hash = blake3::hash(
        &anchor.encode(ns).map_err(S3HeadError::from)?
    ).to_hex().to_string();
    let mut generation = anchor.generation;
    let mut downloaded_shard_bytes = 0usize;
    let mut total_shard_refs = 0usize;

    for bytes in frames_newest_first.iter().rev() {
        let raw: S3ManifestV1 = serde_json::from_slice(bytes)
            .map_err(|_| invalid_manifest())?;
        let next = S3ManifestV1::decode(ns, raw.manifest_id, bytes)
            .map_err(S3HeadError::from)?;
        if next.previous_manifest_id != Some(previous_root_id)
            || next.previous_manifest_blake3.as_deref()
                != Some(previous_root_hash.as_str())
            || generation.checked_add(1) != Some(next.generation)
        {
            return Err(invalid_manifest());
        }
        total_shard_refs = total_shard_refs
            .checked_add(next.shards.len())
            .ok_or(S3SelectiveReadError::VerifiedFullHistoryRequired)?;
        if total_shard_refs > limits.max_total_shard_refs || next.total_revisions as usize > MAX_REVISIONS {
            return Err(S3SelectiveReadError::VerifiedFullHistoryRequired);
        }

        let mut current_shards = BTreeMap::new();
        let mut current_entries = BTreeMap::new();
        for reference in &next.shards {
            if let Some(earlier) = known_descriptors.get(&reference.shard_id) {
                if earlier != reference {
                    return Err(S3SelectiveReadError::ImmutableShardRewritten);
                }
            } else {
                known_descriptors.insert(reference.shard_id, reference.clone());
            }
            let revisions = if let Some((prev_descriptor, refs)) =
                previous_shards.get(&reference.shard_id)
            {
                if prev_descriptor != reference {
                    return Err(S3SelectiveReadError::ImmutableShardRewritten);
                }
                refs.clone()
            } else {
                let key = ns.immutable_manifest_key(reference.shard_id);
                let payload = reader.read_bounded(&key, MAX_SHARD_BYTES as u64).await
                    .map_err(S3SelectiveReadError::Transport)?;
                check_readback(&payload, MAX_SHARD_BYTES as u64)
                    .map_err(|_| invalid_manifest())?;
                if payload.blake3 != reference.shard_blake3 {
                    return Err(S3SelectiveReadError::Head(S3HeadError::IntegrityMismatch));
                }
                downloaded_shard_bytes = downloaded_shard_bytes
                    .checked_add(payload.bytes.len())
                    .ok_or(S3SelectiveReadError::VerifiedFullHistoryRequired)?;
                if downloaded_shard_bytes > MAX_ADDITIONAL_HISTORY_SHARD_BYTES {
                    return Err(S3SelectiveReadError::VerifiedFullHistoryRequired);
                }
                let shard = S3ManifestShardV1::decode(
                    ns, reference.shard_id, &payload.bytes,
                ).map_err(S3HeadError::from)?;
                verify_descriptor(&shard, reference)?;
                shard.revisions
            };
            if revisions.len() != reference.count as usize
                || revisions.first().map(|r|r.revision_id) != Some(reference.first_revision)
                || revisions.last().map(|r|r.revision_id) != Some(reference.last_revision)
            {
                return Err(invalid_manifest());
            }
            for revision in &revisions {
                if current_entries.insert(revision.revision_id, revision.clone()).is_some() {
                    return Err(invalid_manifest());
                }
            }
            if current_shards.insert(
                reference.shard_id, (reference.clone(), revisions),
            ).is_some() {
                return Err(invalid_manifest());
            }
        }
        if current_entries.len() != next.total_revisions as usize {
            return Err(invalid_manifest());
        }
        for (id, old) in &previous_entries {
            match current_entries.get(id) {
                None => return Err(S3SelectiveReadError::MissingPreviouslyVerifiedRevision),
                Some(new) if new != old => {
                    return Err(S3SelectiveReadError::ImmutableRevisionRewritten);
                }
                _ => {}
            }
        }
        previous_entries = current_entries;
        previous_shards = current_shards;
        generation = next.generation;
        previous_root_id = next.manifest_id;
        previous_root_hash = blake3::hash(bytes).to_hex().to_string();
    }
    if previous_root_id != current.manifest_id()
        || previous_root_hash != current.manifest_blake3()
        || generation != current.generation()
    {
        return Err(invalid_manifest());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{
        DocumentId, RevisionOperation, RevisionRecord,
        S3ManifestShardV1, SyncRelativePath, VaultId,
    };
    use super::super::{S3BoundedObject, S3ObservedObject};
    use std::collections::BTreeMap;

    struct Fixture {
        ns: S3Keyspace,
        old: S3ManifestV1,
        pinned: S3VerifiedRevisionMetadataStore,
        base: S3RevisionRefV1,
        sibling: S3RevisionRefV1,
        tombstone: S3RevisionRefV1,
    }

    fn reference(
        doc: DocumentId,
        id: RevisionId,
        parents: Vec<RevisionId>,
        tombstone: bool,
    ) -> (S3RevisionRefV1, Vec<u8>) {
        let op = if tombstone {
            RevisionOperation::Tombstone {
                path: SyncRelativePath::parse("notes/offline.md").unwrap(),
            }
        } else {
            RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/offline.md").unwrap(),
                blob_hash: blake3::hash(id.to_string().as_bytes()).to_hex().to_string(),
            }
        };
        let raw = serde_json::to_vec(&RevisionRecord {
            id, document_id: doc, parents, operation: op,
        }).unwrap();
        (S3RevisionRefV1 {
            revision_id: id,
            document_id: doc,
            revision_blake3: blake3::hash(&raw).to_hex().to_string(),
            blob_bytes: if tombstone { None } else { Some(16) },
        }, raw)
    }

    fn fixture() -> Fixture {
        let ns = S3Keyspace::new("akula/history", VaultId::new()).unwrap();
        let doc = DocumentId::new();
        let base_id = RevisionId::new();
        let sibling_id = RevisionId::new();
        let tomb_id = RevisionId::new();
        let (base, raw) = reference(doc, base_id, vec![], false);
        let (sibling, _) = reference(doc, sibling_id, vec![base_id], false);
        let (tombstone, _) = reference(doc, tomb_id, vec![base_id], true);
        let mut pinned = S3VerifiedRevisionMetadataStore::new(
            ns.vault_id(), "previous-snapshot".into(), 1024 * 1024,
        ).unwrap();
        pinned.accept(base.clone(), raw).unwrap();
        pinned.validate_complete_dag().unwrap();
        let old_shard = S3ManifestShardV1::new(&ns, OperationId::new(), vec![base.clone()]).unwrap();
        let old = S3ManifestV1::new(
            &ns, OperationId::new(), 1, None,
            vec![old_shard.reference(&ns).unwrap()],
        ).unwrap();
        Fixture {ns,old,pinned,base,sibling,tombstone}
    }

    fn generation(
        ns: &S3Keyspace,
        previous: &S3ManifestV1,
        mut entries: Vec<S3RevisionRefV1>,
    ) -> (S3ManifestV1, Vec<u8>, (String, S3BoundedObject)) {
        entries.sort_by_key(|r|r.revision_id);
        let shard = S3ManifestShardV1::new(
            ns, OperationId::new(), entries,
        ).unwrap();
        let bytes = shard.encode(ns).unwrap();
        let shard_ref = shard.reference(ns).unwrap();
        let root = S3ManifestV1::new(
            ns, OperationId::new(), previous.generation + 1,
            Some((
                previous.manifest_id,
                blake3::hash(&previous.encode(ns).unwrap()).to_hex().to_string(),
            )),
            vec![shard_ref],
        ).unwrap();
        let root_bytes = root.encode(ns).unwrap();
        let object = S3BoundedObject {
            object: S3ObservedObject {
                etag: "\"test-shard\"".into(),
                byte_length: bytes.len() as u64,
            },
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
        };
        (root,root_bytes,(ns.immutable_manifest_key(shard.shard_id), object))
    }

    struct Reader {
        objects: BTreeMap<String, S3BoundedObject>,
    }

    impl BoundedReader for Reader {
        async fn read_bounded(
            &self, key:&str, max_bytes:u64,
        ) -> Result<S3BoundedObject,S3TransportError> {
            let x = self.objects.get(key).ok_or(S3TransportError::NotFound)?;
            if x.bytes.len() as u64 > max_bytes {
                return Err(S3TransportError::ObjectTooLarge);
            }
            Ok(x.clone())
        }
    }

    fn head(ns:&S3Keyspace, root:&S3ManifestV1, bytes:&[u8]) -> S3HeadPointerV1 {
        S3HeadPointerV1::new(
            ns, root.generation, root.manifest_id,
            &blake3::hash(bytes).to_hex().to_string(),
            root.previous_manifest_id,
        ).unwrap()
    }

    #[tokio::test]
    async fn two_offline_generations_keep_tombstone_and_sibling() {
        let f=fixture();
        let (middle,middle_bytes,a) = generation(
            &f.ns,&f.old,vec![f.base.clone(),f.tombstone.clone()],
        );
        let (latest,latest_bytes,b) = generation(
            &f.ns,&middle,vec![
                f.base.clone(),f.tombstone.clone(),f.sibling.clone(),
            ],
        );
        let r=Reader {objects: BTreeMap::from([a,b])};
        verify_offline_retention(
            &r,&f.ns,&f.old,&f.pinned,
            &head(&f.ns,&latest,&latest_bytes),
            &[latest_bytes,middle_bytes],S3ManifestDeltaLimits::default(),
        ).await.unwrap();
    }

    #[tokio::test]
    async fn intermediate_tombstone_removed_in_final_is_hard_hold() {
        let f=fixture();
        let (middle,middle_bytes,a) = generation(
            &f.ns,&f.old,vec![f.base.clone(),f.tombstone.clone()],
        );
        let (latest,latest_bytes,b) = generation(
            &f.ns,&middle,vec![f.base.clone(),f.sibling.clone()],
        );
        let r=Reader {objects: BTreeMap::from([a,b])};
        assert!(matches!(
            verify_offline_retention(
                &r,&f.ns,&f.old,&f.pinned,
                &head(&f.ns,&latest,&latest_bytes),
                &[latest_bytes,middle_bytes],S3ManifestDeltaLimits::default(),
            ).await,
            Err(S3SelectiveReadError::MissingPreviouslyVerifiedRevision)
        ));
    }

    #[tokio::test]
    async fn intermediate_immutable_revision_rewrite_is_hard_hold() {
        let f=fixture();
        let (middle,middle_bytes,a) = generation(
            &f.ns,&f.old,vec![f.base.clone(),f.tombstone.clone()],
        );
        let mut rewritten=f.tombstone.clone();
        rewritten.revision_blake3=blake3::hash(b"forged-revision-json").to_hex().to_string();
        let (latest,latest_bytes,b) = generation(
            &f.ns,&middle,vec![f.base.clone(),rewritten],
        );
        let r=Reader {objects: BTreeMap::from([a,b])};
        assert!(matches!(
            verify_offline_retention(
                &r,&f.ns,&f.old,&f.pinned,
                &head(&f.ns,&latest,&latest_bytes),
                &[latest_bytes,middle_bytes],S3ManifestDeltaLimits::default(),
            ).await,
            Err(S3SelectiveReadError::ImmutableRevisionRewritten)
        ));
    }

    #[tokio::test]
    async fn missing_or_tampered_intermediate_shard_is_never_empty_remote() {
        let f=fixture();
        let (middle,middle_bytes,a) = generation(
            &f.ns,&f.old,vec![f.base.clone(),f.tombstone.clone()],
        );
        let (latest,latest_bytes,b) = generation(
            &f.ns,&middle,vec![f.base.clone(),f.tombstone.clone()],
        );
        let current=head(&f.ns,&latest,&latest_bytes);
        let frames=[latest_bytes.clone(),middle_bytes.clone()];
        let absent=Reader {objects:BTreeMap::from([b.clone()])};
        assert!(matches!(
            verify_offline_retention(
                &absent,&f.ns,&f.old,&f.pinned,&current,
                &frames,S3ManifestDeltaLimits::default(),
            ).await,
            Err(S3SelectiveReadError::Transport(S3TransportError::NotFound))
        ));
        let mut changed=a;
        changed.1.bytes.push(b'x');
        let corrupted=Reader {objects:BTreeMap::from([changed,b])};
        assert!(verify_offline_retention(
            &corrupted,&f.ns,&f.old,&f.pinned,&current,
            &frames,S3ManifestDeltaLimits::default(),
        ).await.is_err());
    }
}
