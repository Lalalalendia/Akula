//! SYNC-COORD-02b: optional read-only S3 delta-letter / verified-pack bridge.
//!
//! This deliberately starts from the FULL, independently verified V1 pinned
//! snapshots and profile-bound durable SQLite anchor. Optimizing away that
//! full current pin is a future proof step, NOT part of this integration.
//! A compact sidecar is only a transport hint: it cannot overrule the V1 DAG,
//! mutate a user file, confirm a baseline, delete, GC or publish remote state.
//! BLAKE3 still does not authenticate the remote writer or prevent rollback.

use std::collections::BTreeSet;

use super::{
    AwsS3ProbeClient, S3HeadError, S3PinnedRevisionSnapshot,
    S3TransportError, S3VerifiedRevisionMetadataStore,
};
use super::pack_probe::{
    ingest_verified_pack, PackDescriptor, MAX_PACK_BYTES,
};
use crate::sync::{
    RevisionDag, RevisionId, RevisionRecord, S3ContractError,
    S3DeltaEnvelopeV2, S3DeltaEnvelopeV2Error,
    S3HeadPointerV1, S3Keyspace, S3ManifestError, S3ManifestV1,
    S3RemoteAnchorError, S3RemoteAnchorJournal, S3RemoteAnchorScope,
    MAX_DELTA_ENVELOPE_BYTES,
};

#[derive(Debug)]
pub enum S3CoordSidecarReadError {
    ScopeMismatch,
    MissingPreviousAnchor,
    PreviousCacheNotPinned,
    MissingCurrentRemote,
    InvalidHistory,
    InvalidPackPayload,
    InvalidEnvelopeDigest,
    HeadChangedDuringRead,
    Anchor(S3RemoteAnchorError),
    Head(S3HeadError),
    Contract(S3ContractError),
    Manifest(S3ManifestError),
    Envelope(S3DeltaEnvelopeV2Error),
    Transport(S3TransportError),
}

impl From<S3RemoteAnchorError> for S3CoordSidecarReadError {
    fn from(e: S3RemoteAnchorError) -> Self { Self::Anchor(e) }
}
impl From<S3HeadError> for S3CoordSidecarReadError {
    fn from(e: S3HeadError) -> Self { Self::Head(e) }
}
impl From<S3ContractError> for S3CoordSidecarReadError {
    fn from(e: S3ContractError) -> Self { Self::Contract(e) }
}
impl From<S3ManifestError> for S3CoordSidecarReadError {
    fn from(e: S3ManifestError) -> Self { Self::Manifest(e) }
}
impl From<S3DeltaEnvelopeV2Error> for S3CoordSidecarReadError {
    fn from(e: S3DeltaEnvelopeV2Error) -> Self { Self::Envelope(e) }
}
impl From<S3TransportError> for S3CoordSidecarReadError {
    fn from(e: S3TransportError) -> Self { Self::Transport(e) }
}

/// An advisory read-only receipt, never a RemoteStore or a native File Gate
/// admission. It reports what the bound verifier actually downloaded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3CoordSidecarPreview {
    pub generation: u64,
    pub newly_verified_revisions: usize,
    pub newly_verified_packs: usize,
    pub checked_pack_bytes: u64,
    pub new_revision_ids: Vec<RevisionId>,
    pub requires_full_apply_gate: bool,
}

impl AwsS3ProbeClient {
    /// Independently pin the current V1 S3 snapshot, then verify a caller-
    /// specified immutable v2 letter and all its pack members against it.
    ///
    /// The digest is just a locator, not remote author authentication.
    /// The previous V1 metadata and its full pin must match the exact
    /// previously persisted, profile-bound S3RemoteAnchorJournal.
    ///
    /// This is intentionally not a fast production sync path: the canonical
    /// current V1 full pin runs BEFORE consuming the sidecar, including full
    /// old-object availability checks. The result never authorizes local
    /// File Gate mutation or deletion.
    pub async fn inspect_v2_sidecar_against_pinned_v1(
        &self,
        scope: &S3RemoteAnchorScope,
        journal: &S3RemoteAnchorJournal,
        previous_pinned: &S3PinnedRevisionSnapshot,
        previous_metadata: &S3VerifiedRevisionMetadataStore,
        letter_digest: &str,
    ) -> Result<S3CoordSidecarPreview, S3CoordSidecarReadError> {
        // Check real SDK profile and caller-selected namespace before issuing
        // any remote request, including the alleged v2 letter object.
        if !scope.matches_connection(self.bound_profile(), self.bound_keyspace()) {
            return Err(S3CoordSidecarReadError::ScopeMismatch);
        }
        let keyspace = self.bound_keyspace();
        let letter_key = keyspace.immutable_blob_key(letter_digest)
            .map_err(|_| S3CoordSidecarReadError::InvalidEnvelopeDigest)?;
        let anchor = journal.load(scope)?
            .ok_or(S3CoordSidecarReadError::MissingPreviousAnchor)?;
        let old_head = anchor.pointer();
        let expected_old_snapshot = format!(
            "{}:{}", old_head.manifest_id(), old_head.manifest_blake3(),
        );
        if previous_pinned.snapshot_id != expected_old_snapshot
            || !previous_metadata.is_verified_for(
                keyspace.vault_id(), &expected_old_snapshot,
            )
        {
            return Err(S3CoordSidecarReadError::PreviousCacheNotPinned);
        }
        if blake3::hash(anchor.manifest_bytes()).to_hex().as_str()
            != old_head.manifest_blake3()
        {
            return Err(S3CoordSidecarReadError::InvalidHistory);
        }
        let old_manifest = S3ManifestV1::decode(
            keyspace, old_head.manifest_id(), anchor.manifest_bytes(),
        )?;

        // This intentionally verifies the currently available *entire* V1
        // snapshot and history; a sidecar never substitutes a verified head.
        let observed = self.read_verified_head().await?
            .ok_or(S3CoordSidecarReadError::MissingCurrentRemote)?;
        let current_pinned = self.pin_revision_snapshot().await?
            .ok_or(S3CoordSidecarReadError::MissingCurrentRemote)?;

        let letter_object = self.get_bounded(
            &letter_key, MAX_DELTA_ENVELOPE_BYTES as u64,
        ).await?;
        if letter_object.blake3 != letter_digest {
            return Err(S3CoordSidecarReadError::InvalidEnvelopeDigest);
        }
        let letter = S3DeltaEnvelopeV2::decode(
            keyspace, &letter_object.bytes,
        )?;
        let coverage = letter.verify_pinned_v1_coverage(
            keyspace, old_head, &old_manifest, previous_pinned,
            &observed.pointer, &observed.manifest, &current_pinned,
        )?;

        let mut dag = RevisionDag::new();
        for (_, bytes) in previous_metadata.verified_entries() {
            let record: RevisionRecord = serde_json::from_slice(bytes)
                .map_err(|_| S3CoordSidecarReadError::InvalidHistory)?;
            dag.ingest(record)
                .map_err(|_| S3CoordSidecarReadError::InvalidHistory)?;
        }
        if dag.active_len() != previous_metadata.len() || dag.pending_len() != 0 {
            return Err(S3CoordSidecarReadError::InvalidHistory);
        }

        let mut seen = BTreeSet::new();
        let mut verified_pack_bytes = 0_u64;
        for pack in letter.packs() {
            let start = pack.first_revision_index as usize;
            let end = start + pack.revision_count as usize;
            let key = keyspace.immutable_blob_key(&pack.pack_blake3)?;
            let payload = self.get_bounded(&key, MAX_PACK_BYTES as u64).await?;
            if payload.blake3 != pack.pack_blake3
                || payload.object.byte_length != pack.byte_length as u64
            {
                return Err(S3CoordSidecarReadError::InvalidPackPayload);
            }
            let descriptor = PackDescriptor {
                key, digest: pack.pack_blake3.clone(),
                references: letter.revisions()[start..end].to_vec(),
            };
            let accepted = ingest_verified_pack(
                &payload.bytes, &descriptor, &mut dag, &mut seen,
            ).map_err(|_| S3CoordSidecarReadError::InvalidPackPayload)?;
            verified_pack_bytes = verified_pack_bytes.checked_add(accepted)
                .ok_or(S3CoordSidecarReadError::InvalidPackPayload)?;
        }

        let expected: BTreeSet<_> = coverage.added_revision_ids.iter()
            .copied().collect();
        if seen != expected
            || dag.pending_len() != 0
            || dag.active_len() != current_pinned.len()
        {
            return Err(S3CoordSidecarReadError::InvalidHistory);
        }

        // Final mutable pointer AND ETag must match the canonical full V1
        // observation. A CAS race invalidates even an otherwise valid pack.
        let after = self.get_bounded(&keyspace.current_head_key(), 4096).await?;
        let current = S3HeadPointerV1::decode(&after.bytes, keyspace)?;
        if current != observed.pointer || after.object.etag != observed.etag {
            return Err(S3CoordSidecarReadError::HeadChangedDuringRead);
        }

        Ok(S3CoordSidecarPreview {
            generation: coverage.generation,
            newly_verified_revisions: coverage.added_revision_ids.len(),
            newly_verified_packs: coverage.required_pack_count,
            checked_pack_bytes: verified_pack_bytes,
            new_revision_ids: coverage.added_revision_ids,
            requires_full_apply_gate: true,
        })
    }
}

#[cfg(test)]
#[path = "coord_sidecar_tests.rs"]
mod tests;
