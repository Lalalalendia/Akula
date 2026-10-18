//! SYNC-COORD-02c: bounded multi-generation V2 sidecar verification.
//!
//! Every skipped immutable V1 manifest is BLAKE3-linked to the exact saved
//! profile-bound SQLite anchor, then INDEPENDENTLY FULL-PINNED (all revision
//! JSON, blobs and DAG) before its single-generation V2 letter and packs are
//! accepted. No current-head-only diff may skip an intermediate tombstone or
//! offline sibling. This conservative first implementation is intentionally
//! expensive: it establishes correctness, NOT a fast 2/9-GET sync claim.
//!
//! The entire path remains advisory/read-only: no writer authentication,
//! remote CAS, file mutation, deletion, GC or ConfirmedBaseline progression.

use std::collections::BTreeSet;

use super::{
    AwsS3ProbeClient, S3CoordSidecarReadError, S3PinnedRevisionSnapshot,
    S3VerifiedRevisionMetadataStore,
};
use crate::sync::{
    walk_s3_delta_from_saved_anchor, RevisionDag, RevisionId, RevisionRecord,
    S3AnchoredWalkError, S3DeltaEnvelopeV2, S3HeadPointerV1,
    S3ManifestDeltaBase, S3ManifestDeltaLimits, S3ManifestDeltaOutcome,
    S3ManifestV1, S3RemoteAnchorJournal, S3RemoteAnchorScope,
    MAX_DELTA_ENVELOPE_BYTES,
};

pub(super) const MAX_OFFLINE_GENERATIONS: usize = 8;
pub(super) const MAX_OFFLINE_PACK_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum S3CoordOfflineChainError {
    Sidecar(S3CoordSidecarReadError),
    Walk(S3AnchoredWalkError),
    InvalidChain,
    ExceededOfflineBudget,
    HeadChangedDuringRead,
}
impl From<S3CoordSidecarReadError> for S3CoordOfflineChainError {
    fn from(value: S3CoordSidecarReadError) -> Self { Self::Sidecar(value) }
}
impl From<S3AnchoredWalkError> for S3CoordOfflineChainError {
    fn from(value: S3AnchoredWalkError) -> Self { Self::Walk(value) }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3CoordOfflineChainPreview {
    pub from_generation: u64,
    pub to_generation: u64,
    pub verified_generations: usize,
    pub newly_verified_revisions: usize,
    pub newly_verified_packs: usize,
    pub checked_pack_bytes: u64,
    pub new_revision_ids: Vec<RevisionId>,
    /// Neither a fully current remote-store adapter nor File Gate permission.
    pub requires_full_apply_gate: bool,
}

impl AwsS3ProbeClient {
    /// Verify each explicitly supplied immutable letter on the exact V1
    /// generation chain since the durable S3 anchor, oldest to newest.
    ///
    /// Letter digest inputs are *locators*, not trusted authority: all roots
    /// are independently derived from the canonical V1 history walker, every
    /// intervening V1 state is fully pinned through AWS SDK, and every pack
    /// member must agree with that pin. Missing a generation or providing an
    /// extra/stale letter is a hard hold, not deletion authority.
    pub async fn inspect_v2_offline_chain_against_pinned_v1(
        &self,
        scope: &S3RemoteAnchorScope,
        journal: &S3RemoteAnchorJournal,
        previous_pinned: &S3PinnedRevisionSnapshot,
        previous_metadata: &S3VerifiedRevisionMetadataStore,
        letters_oldest_first: &[String],
    ) -> Result<S3CoordOfflineChainPreview, S3CoordOfflineChainError> {
        if !scope.matches_connection(self.bound_profile(), self.bound_keyspace()) {
            return Err(S3CoordSidecarReadError::ScopeMismatch.into());
        }
        if letters_oldest_first.is_empty()
            || letters_oldest_first.len() > MAX_OFFLINE_GENERATIONS
        {
            return Err(S3CoordOfflineChainError::ExceededOfflineBudget);
        }
        let ns = self.bound_keyspace();
        // Reject malformed locators and duplicate claimed letters before
        // network I/O. There is never an implicit remote-key enumeration.
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
        let expected_snapshot = format!(
            "{}:{}", old_head.manifest_id(), old_head.manifest_blake3(),
        );
        if previous_pinned.snapshot_id != expected_snapshot
            || !previous_metadata.is_verified_for(ns.vault_id(), &expected_snapshot)
        {
            return Err(S3CoordSidecarReadError::PreviousCacheNotPinned.into());
        }
        if blake3::hash(anchor.manifest_bytes()).to_hex().as_str()
            != old_head.manifest_blake3()
        {
            return Err(S3CoordSidecarReadError::InvalidHistory.into());
        }
        let mut previous_manifest = S3ManifestV1::decode(
            ns, old_head.manifest_id(), anchor.manifest_bytes(),
        ).map_err(S3CoordSidecarReadError::from)?;

        // The walker ensures the complete immutable predecessor chain and
        // compares the sole mutable V1 pointer/ETag before and after reading
        // it. Limits are STRICTER than the generic delta walker.
        let history = walk_s3_delta_from_saved_anchor(
            self, scope, journal, S3ManifestDeltaLimits {
                max_frames: MAX_OFFLINE_GENERATIONS,
                max_total_shard_refs: MAX_OFFLINE_GENERATIONS * 256,
            },
        ).await?;
        let verified_count = match &history.outcome {
            S3ManifestDeltaOutcome::Complete {
                generations_traversed,
                base: S3ManifestDeltaBase::VerifiedCheckpoint,
                ..
            } => *generations_traversed,
            _ => return Err(S3CoordOfflineChainError::InvalidChain),
        };
        if verified_count != letters_oldest_first.len()
            || history.verified_manifest_frames.len() != verified_count
        {
            return Err(S3CoordOfflineChainError::InvalidChain);
        }

        let mut previous_head = old_head.clone();
        let mut pinned = previous_pinned.clone();
        let mut dag = RevisionDag::new();
        for (_, json) in previous_metadata.verified_entries() {
            let record: RevisionRecord = serde_json::from_slice(json)
                .map_err(|_| S3CoordSidecarReadError::InvalidHistory)?;
            dag.ingest(record)
                .map_err(|_| S3CoordSidecarReadError::InvalidHistory)?;
        }
        if dag.pending_len() != 0 || dag.active_len() != pinned.len() {
            return Err(S3CoordSidecarReadError::InvalidHistory.into());
        }
        let mut total_new = Vec::new();
        let mut pack_count = 0usize;
        let mut pack_bytes = 0_u64;
        for (frame, digest) in history.verified_manifest_frames
            .iter().rev().zip(letters_oldest_first)
        {
            // Derive intermediate V1 pointer *only* from BLAKE3-checked,
            // history-linked immutable manifest bytes. It is never passed
            // in by the untrusted letter.
            let preliminary: S3ManifestV1 = serde_json::from_slice(frame)
                .map_err(|_| S3CoordOfflineChainError::InvalidChain)?;
            let current_manifest = S3ManifestV1::decode(
                ns, preliminary.manifest_id, frame,
            ).map_err(S3CoordSidecarReadError::from)?;
            if !current_manifest.is_successor_of(
                &previous_manifest, previous_head.manifest_blake3(),
            ) {
                return Err(S3CoordOfflineChainError::InvalidChain);
            }
            let current_digest = blake3::hash(frame).to_hex().to_string();
            let head = S3HeadPointerV1::new(
                ns, current_manifest.generation,
                current_manifest.manifest_id,
                &current_digest,
                current_manifest.previous_manifest_id,
            ).map_err(S3CoordSidecarReadError::from)?;

            // Critical: no metadata-only "snapshot" is invented from an
            // untrusted sidecar or arbitrary refs. Every V1 generation is
            // fully re-read, checked for bytes, DAG and blob availability.
            let (full, next_pinned) = self.pin_historical_manifest_snapshot(
                current_manifest.manifest_id, &current_digest,
            ).await.map_err(S3CoordSidecarReadError::from)?;
            if full != current_manifest {
                return Err(S3CoordOfflineChainError::InvalidChain);
            }

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
            let coverage = letter.verify_pinned_v1_coverage(
                ns, &previous_head, &previous_manifest, &pinned,
                &head, &current_manifest, &next_pinned,
            ).map_err(S3CoordSidecarReadError::from)?;

            let declared_pack_bytes: u64 = letter.packs()
                .iter().map(|pack| u64::from(pack.byte_length)).sum();
            if pack_bytes.checked_add(declared_pack_bytes)
                .is_none_or(|sum| sum > MAX_OFFLINE_PACK_BYTES)
            {
                return Err(S3CoordOfflineChainError::ExceededOfflineBudget);
            }
            let expected: BTreeSet<_> = coverage.added_revision_ids.iter()
                .copied().collect();
            let (verified_packs, received_bytes) =
                self.verify_letter_packs(&letter, &expected, &mut dag)
                    .await.map_err(S3CoordOfflineChainError::from)?;
            if received_bytes != declared_pack_bytes
                || dag.pending_len() != 0 || dag.active_len() != next_pinned.len()
            {
                return Err(S3CoordSidecarReadError::InvalidHistory.into());
            }
            pack_bytes += received_bytes;
            pack_count += verified_packs;
            total_new.extend(coverage.added_revision_ids);
            pinned = next_pinned;
            previous_manifest = current_manifest;
            previous_head = head;
        }

        if previous_head != *history.observed_head.pointer() {
            return Err(S3CoordOfflineChainError::InvalidChain);
        }
        // Guard race spanning the expensive full-historical pin and all pack
        // downloads; compare both the canonical pointer and opaque S3 ETag.
        let after = self.get_bounded(
            &ns.current_head_key(), 4096,
        ).await.map_err(S3CoordSidecarReadError::from)?;
        let current = S3HeadPointerV1::decode(&after.bytes, ns)
            .map_err(S3CoordSidecarReadError::from)?;
        if current != previous_head
            || after.object.etag != history.observed_head.etag()
        {
            return Err(S3CoordOfflineChainError::HeadChangedDuringRead);
        }
        Ok(S3CoordOfflineChainPreview {
            from_generation: old_head.generation(),
            to_generation: previous_head.generation(),
            verified_generations: verified_count,
            newly_verified_revisions: total_new.len(),
            newly_verified_packs: pack_count,
            checked_pack_bytes: pack_bytes,
            new_revision_ids: total_new,
            requires_full_apply_gate: true,
        })
    }
}

#[cfg(test)]
#[path = "coord_chain_tests.rs"]
mod tests;
