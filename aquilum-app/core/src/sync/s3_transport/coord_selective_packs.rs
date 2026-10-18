//! SYNC-COORD-02e: advisory V1-anchored selective V2 pack reader.
//!
//! Unlike the conservative full-pin 02c reader, this uses previously pinned
//! immutable V1 metadata, reconstructs complete per-generation V1 shard
//! inventories, and verifies only newly added JSON/blob bytes from V2 packs.
//! The canonical saved-anchor manifest walker still proves ALL generations.
//! Previous blob availability is NOT re-proved, so the result can never be
//! a native File Gate or confirmed-baseline authority.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    AwsS3ProbeClient, S3CoordOfflineChainError, S3CoordSidecarReadError,
    S3PinnedRevisionSnapshot, S3VerifiedRevisionMetadataStore,
};
use super::coord_chain::{MAX_OFFLINE_GENERATIONS, MAX_OFFLINE_PACK_BYTES};
use crate::sync::{
    walk_s3_delta_from_saved_anchor, OperationId, RevisionDag, RevisionId,
    RevisionRecord, S3DeltaEnvelopeV2, S3HeadPointerV1,
    S3ManifestDeltaBase, S3ManifestDeltaLimits, S3ManifestDeltaOutcome,
    S3ManifestShardRefV1, S3ManifestShardV1, S3ManifestV1,
    S3RemoteAnchorJournal, S3RemoteAnchorScope, S3RevisionRefV1,
    S3Keyspace, MAX_DELTA_ENVELOPE_BYTES, MAX_SHARD_BYTES, MAX_REVISIONS,
};

const MAX_SELECTIVE_SHARD_BYTES: u64 = 32 * 1024 * 1024;
type KnownShards = BTreeMap<OperationId, (String, Vec<S3RevisionRefV1>)>;

/// Observational receipt only. It does not prove current availability of
/// previously pinned blobs, sender identity, local write safety or migration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3CoordSelectivePacksPreview {
    pub from_generation: u64,
    pub to_generation: u64,
    pub verified_generations: usize,
    pub total_revisions: usize,
    pub new_revision_ids: Vec<RevisionId>,
    pub verified_packs: usize,
    pub checked_pack_bytes: u64,
    pub downloaded_shards: usize,
    pub reused_shards: usize,
    pub checked_shard_bytes: u64,
    pub requires_full_apply_gate: bool,
}

/// The old shard object bytes are not fetched again, but can be
/// reconstructed *canonically* from the exact, prior full-pin inventory.
/// Hashing the canonical encoding and matching every old shard descriptor
/// prevents accidental reuse of unrelated, same-count cached metadata.
fn old_verified_shards(
    ns: &S3Keyspace,
    manifest: &S3ManifestV1,
    old_refs: &[S3RevisionRefV1],
) -> Result<KnownShards, S3CoordOfflineChainError> {
    if old_refs.len() != manifest.total_revisions as usize
        || old_refs.len() > MAX_REVISIONS
    {
        return Err(S3CoordOfflineChainError::InvalidChain);
    }
    let mut known = KnownShards::new();
    let mut accounted = 0usize;
    for shard_ref in &manifest.shards {
        let rows: Vec<_> = old_refs.iter()
            .filter(|row| row.revision_id >= shard_ref.first_revision
                && row.revision_id <= shard_ref.last_revision)
            .cloned().collect();
        let reconstructed = S3ManifestShardV1::new(
            ns, shard_ref.shard_id, rows.clone(),
        ).map_err(|_| S3CoordOfflineChainError::InvalidChain)?;
        if reconstructed.reference(ns)
            .map_err(|_| S3CoordOfflineChainError::InvalidChain)?
            != *shard_ref
        {
            return Err(S3CoordOfflineChainError::InvalidChain);
        }
        accounted += rows.len();
        known.insert(shard_ref.shard_id, (shard_ref.shard_blake3.clone(), rows));
    }
    if accounted != old_refs.len() {
        return Err(S3CoordOfflineChainError::InvalidChain);
    }
    Ok(known)
}

impl AwsS3ProbeClient {
    /// Independently collect *descriptive V1 shard refs* for a specific
    /// immutable, BLAKE3-linked manifest generation. Only exact unchanged
    /// old shard IDs/digests may reuse previously verified references.
    /// Every novel shard is read through HEAD/If-Match GET/HEAD and verified
    /// against its whole V1 descriptor; reused UUIDs with changed digests HOLD.
    async fn coord_selective_inventory(
        &self,
        manifest: &S3ManifestV1,
        known: &mut KnownShards,
        downloaded: &mut usize,
        reused: &mut usize,
        bytes: &mut u64,
    ) -> Result<Vec<S3RevisionRefV1>, S3CoordOfflineChainError> {
        let ns = self.bound_keyspace();
        let mut rows = Vec::with_capacity(manifest.total_revisions as usize);
        for descriptor in &manifest.shards {
            if let Some((digest, cached)) = known.get(&descriptor.shard_id) {
                if digest != &descriptor.shard_blake3
                    || cached.len() != descriptor.count as usize
                    || cached.first().map(|r| r.revision_id)
                        != Some(descriptor.first_revision)
                    || cached.last().map(|r| r.revision_id)
                        != Some(descriptor.last_revision)
                {
                    return Err(S3CoordOfflineChainError::InvalidChain);
                }
                rows.extend(cached.iter().cloned());
                *reused += 1;
                continue;
            }
            let raw = self.get_bounded(
                &ns.immutable_manifest_key(descriptor.shard_id),
                MAX_SHARD_BYTES as u64,
            ).await.map_err(S3CoordSidecarReadError::from)?;
            let total = bytes.checked_add(raw.bytes.len() as u64)
                .ok_or(S3CoordOfflineChainError::ExceededOfflineBudget)?;
            if total > MAX_SELECTIVE_SHARD_BYTES {
                return Err(S3CoordOfflineChainError::ExceededOfflineBudget);
            }
            *bytes = total;
            if raw.blake3 != descriptor.shard_blake3 {
                return Err(S3CoordOfflineChainError::InvalidChain);
            }
            let shard = S3ManifestShardV1::decode(
                ns, descriptor.shard_id, &raw.bytes,
            ).map_err(|_| S3CoordOfflineChainError::InvalidChain)?;
            if shard.reference(ns)
                .map_err(|_| S3CoordOfflineChainError::InvalidChain)?
                != *descriptor
            {
                return Err(S3CoordOfflineChainError::InvalidChain);
            }
            rows.extend(shard.revisions.iter().cloned());
            known.insert(
                descriptor.shard_id,
                (descriptor.shard_blake3.clone(), shard.revisions),
            );
            *downloaded += 1;
        }
        if rows.len() != manifest.total_revisions as usize
            || rows.len() > MAX_REVISIONS
            || rows.windows(2).any(|pair|
                pair[0].revision_id >= pair[1].revision_id)
        {
            return Err(S3CoordOfflineChainError::InvalidChain);
        }
        Ok(rows)
    }

    /// Bounded, read-only selective V2 chain against the SAME profile-bound
    /// persisted V1 anchor as 02c. Unlike 02c it does not fully re-read
    /// every historical revision and blob. It NEVER returns a RemoteStore,
    /// writable staging receipt, baseline advancement or deletion authority.
    pub async fn inspect_v2_offline_selective_packs_against_pinned_v1(
        &self,
        scope: &S3RemoteAnchorScope,
        journal: &S3RemoteAnchorJournal,
        previous_pinned: &S3PinnedRevisionSnapshot,
        previous_metadata: &S3VerifiedRevisionMetadataStore,
        letters_oldest_first: &[String],
    ) -> Result<S3CoordSelectivePacksPreview, S3CoordOfflineChainError> {
        if !scope.matches_connection(self.bound_profile(), self.bound_keyspace()) {
            return Err(S3CoordSidecarReadError::ScopeMismatch.into());
        }
        if letters_oldest_first.is_empty()
            || letters_oldest_first.len() > MAX_OFFLINE_GENERATIONS
        {
            return Err(S3CoordOfflineChainError::ExceededOfflineBudget);
        }
        let ns = self.bound_keyspace();
        let mut unique = BTreeSet::new();
        for digest in letters_oldest_first {
            ns.immutable_blob_key(digest)
                .map_err(S3CoordSidecarReadError::from)?;
            if !unique.insert(digest) {
                return Err(S3CoordOfflineChainError::InvalidChain);
            }
        }
        let anchor = journal.load(scope)
            .map_err(S3CoordSidecarReadError::from)?
            .ok_or(S3CoordSidecarReadError::MissingPreviousAnchor)?;
        let old_head = anchor.pointer();
        let old_id = format!(
            "{}:{}", old_head.manifest_id(), old_head.manifest_blake3(),
        );
        if previous_pinned.snapshot_id != old_id
            || !previous_metadata.is_verified_for(ns.vault_id(), &old_id)
            || blake3::hash(anchor.manifest_bytes()).to_hex().as_str()
                != old_head.manifest_blake3()
        {
            return Err(S3CoordSidecarReadError::PreviousCacheNotPinned.into());
        }
        let mut previous_manifest = S3ManifestV1::decode(
            ns, old_head.manifest_id(), anchor.manifest_bytes(),
        ).map_err(S3CoordSidecarReadError::from)?;
        let mut prior_refs = previous_pinned.references_for_coord_v2().to_vec();
        let mut cache_refs: Vec<_> = previous_metadata.verified_entries()
            .map(|(reference, _)| reference.clone()).collect();
        cache_refs.sort_by_key(|reference| reference.revision_id);
        if cache_refs != prior_refs {
            return Err(S3CoordSidecarReadError::PreviousCacheNotPinned.into());
        }
        let mut known = old_verified_shards(ns, &previous_manifest, &prior_refs)?;

        // Verify *all* immutable V1 predecessors and one stable final root,
        // before attempting to trust any letter or changed shard.
        let history = walk_s3_delta_from_saved_anchor(
            self, scope, journal, S3ManifestDeltaLimits {
                max_frames: MAX_OFFLINE_GENERATIONS,
                max_total_shard_refs: MAX_OFFLINE_GENERATIONS * 256,
            },
        ).await?;
        let generations = match &history.outcome {
            S3ManifestDeltaOutcome::Complete {
                generations_traversed,
                base: S3ManifestDeltaBase::VerifiedCheckpoint,
                ..
            } => *generations_traversed,
            _ => return Err(S3CoordOfflineChainError::InvalidChain),
        };
        if generations != letters_oldest_first.len()
            || history.verified_manifest_frames.len() != generations
        {
            return Err(S3CoordOfflineChainError::InvalidChain);
        }

        let mut dag = RevisionDag::new();
        for (_, json) in previous_metadata.verified_entries() {
            let record: RevisionRecord = serde_json::from_slice(json)
                .map_err(|_| S3CoordSidecarReadError::InvalidHistory)?;
            dag.ingest(record)
                .map_err(|_| S3CoordSidecarReadError::InvalidHistory)?;
        }
        if dag.pending_len() != 0 || dag.active_len() != prior_refs.len() {
            return Err(S3CoordSidecarReadError::InvalidHistory.into());
        }

        let mut previous_head = old_head.clone();
        let mut total_new = Vec::new();
        let mut packs = 0usize;
        let mut pack_bytes = 0_u64;
        let mut downloaded_shards = 0usize;
        let mut reused_shards = 0usize;
        let mut checked_shard_bytes = 0_u64;
        for (frame, digest) in history.verified_manifest_frames
            .iter().rev().zip(letters_oldest_first)
        {
            let preliminary: S3ManifestV1 = serde_json::from_slice(frame)
                .map_err(|_| S3CoordOfflineChainError::InvalidChain)?;
            let manifest = S3ManifestV1::decode(
                ns, preliminary.manifest_id, frame,
            ).map_err(S3CoordSidecarReadError::from)?;
            if !manifest.is_successor_of(
                &previous_manifest, previous_head.manifest_blake3(),
            ) {
                return Err(S3CoordOfflineChainError::InvalidChain);
            }
            let manifest_hash = blake3::hash(frame).to_hex().to_string();
            let current_head = S3HeadPointerV1::new(
                ns, manifest.generation, manifest.manifest_id,
                &manifest_hash, manifest.previous_manifest_id,
            ).map_err(S3CoordSidecarReadError::from)?;
            let new_refs = self.coord_selective_inventory(
                &manifest, &mut known,
                &mut downloaded_shards, &mut reused_shards,
                &mut checked_shard_bytes,
            ).await?;

            let key = ns.immutable_blob_key(digest)
                .map_err(S3CoordSidecarReadError::from)?;
            let raw = self.get_bounded(
                &key, MAX_DELTA_ENVELOPE_BYTES as u64,
            ).await.map_err(S3CoordSidecarReadError::from)?;
            if raw.blake3 != *digest {
                return Err(S3CoordSidecarReadError::InvalidEnvelopeDigest.into());
            }
            let letter = S3DeltaEnvelopeV2::decode(ns, &raw.bytes)
                .map_err(S3CoordSidecarReadError::from)?;
            let coverage = letter.verify_shard_inventory_advisory(
                ns, &previous_head, &previous_manifest, &prior_refs,
                &current_head, &manifest, &new_refs,
            ).map_err(S3CoordSidecarReadError::from)?;

            let declared: u64 = letter.packs().iter()
                .map(|pack| u64::from(pack.byte_length)).sum();
            if pack_bytes.checked_add(declared)
                .is_none_or(|sum| sum > MAX_OFFLINE_PACK_BYTES)
            {
                return Err(S3CoordOfflineChainError::ExceededOfflineBudget);
            }
            let expected: BTreeSet<_> = coverage.added_revision_ids.iter()
                .copied().collect();
            let (verified, received) = self.verify_letter_packs(
                &letter, &expected, &mut dag,
            ).await.map_err(S3CoordOfflineChainError::from)?;
            if received != declared || dag.pending_len() != 0
                || dag.active_len() != new_refs.len()
            {
                return Err(S3CoordSidecarReadError::InvalidHistory.into());
            }
            pack_bytes += received;
            packs += verified;
            total_new.extend(coverage.added_revision_ids);
            previous_head = current_head;
            previous_manifest = manifest;
            prior_refs = new_refs;
        }
        if previous_head != *history.observed_head.pointer() {
            return Err(S3CoordOfflineChainError::InvalidChain);
        }
        let after = self.get_bounded(
            &ns.current_head_key(), 4096,
        ).await.map_err(S3CoordSidecarReadError::from)?;
        let final_head = S3HeadPointerV1::decode(&after.bytes, ns)
            .map_err(S3CoordSidecarReadError::from)?;
        if final_head != previous_head
            || after.object.etag != history.observed_head.etag()
        {
            return Err(S3CoordOfflineChainError::HeadChangedDuringRead);
        }
        Ok(S3CoordSelectivePacksPreview {
            from_generation: old_head.generation(),
            to_generation: previous_head.generation(),
            verified_generations: generations,
            total_revisions: prior_refs.len(),
            new_revision_ids: total_new,
            verified_packs: packs,
            checked_pack_bytes: pack_bytes,
            downloaded_shards,
            reused_shards,
            checked_shard_bytes,
            requires_full_apply_gate: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_budget_is_bounded_and_receipt_never_grants_apply() {
        assert!(MAX_SELECTIVE_SHARD_BYTES <= 32 * 1024 * 1024);
        assert_eq!(MAX_OFFLINE_GENERATIONS, 8);
        assert_eq!(MAX_OFFLINE_PACK_BYTES, 16 * 1024 * 1024);
    }

    #[test]
    fn cached_shard_builder_refuses_invented_inventory() {
        let ns = S3Keyspace::new("akula/coord-02e-test", crate::sync::VaultId::new()).unwrap();
        let manifest = S3ManifestV1::new(
            &ns, OperationId::new(), 1, None, Vec::new(),
        ).unwrap();
        let wrong = S3RevisionRefV1 {
            revision_id: RevisionId::new(),
            document_id: crate::sync::DocumentId::new(),
            revision_blake3: blake3::hash(b"wrong").to_hex().to_string(),
            blob_bytes: Some(1),
        };
        assert!(old_verified_shards(&ns, &manifest, &[wrong]).is_err());
        assert!(old_verified_shards(&ns, &manifest, &[]).is_ok());
    }
}
