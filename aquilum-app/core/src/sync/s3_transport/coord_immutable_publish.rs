//! SYNC-COORD-03c: bounded, immutable S3 V2 prepublication without root CAS.
//!
//! This stages ONLY content-addressed packs, their immutable letter and a
//! deterministic immutable manifest-ID locator. It never touches the mutable
//! S3 current root, native user files, File Gate or ConfirmedBaseline.
//! The caller still requires separately authorized local source custody,
//! authenticated remote publisher identity and crash-durable V1 root CAS.
//! Until those gates exist, this method is crate-internal and unused by UI.

use super::{
    AwsS3ProbeClient, S3HeadError, S3PinnedRevisionSnapshot, S3PutCondition,
    S3TransportError, S3VerifiedHead,
};
use super::coord_chain::MAX_OFFLINE_PACK_BYTES;
use super::coord_locator::S3CoordLetterLocatorV2;
use super::coord_pack_encode::PreparedDeltaPacksV2;
use crate::sync::{
    S3DeltaEnvelopeV2, S3DeltaEnvelopeV2Error, S3HeadPointerV1,
    S3ManifestV1, S3RemoteAnchorScope,
};

#[derive(Debug)]
#[allow(dead_code)]
pub(super) enum S3CoordImmutablePublishError {
    ScopeMismatch,
    InvalidPredecessor,
    InvalidCandidate,
    InvalidPack,
    RemoteRootChanged,
    ImmutableConflict,
    Head(S3HeadError),
    Envelope(S3DeltaEnvelopeV2Error),
    Transport(S3TransportError),
}
impl From<S3HeadError> for S3CoordImmutablePublishError {
    fn from(value: S3HeadError) -> Self { Self::Head(value) }
}
impl From<S3DeltaEnvelopeV2Error> for S3CoordImmutablePublishError {
    fn from(value: S3DeltaEnvelopeV2Error) -> Self { Self::Envelope(value) }
}
impl From<S3TransportError> for S3CoordImmutablePublishError {
    fn from(value: S3TransportError) -> Self { Self::Transport(value) }
}

/// Descriptive immutable upload result. Deliberately no method can publish
/// a root, grant writer rights, stage local files or confirm a baseline.
#[derive(Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(super) struct S3CoordImmutablePrepublishPreview {
    pub generation: u64,
    pub pack_count: usize,
    pub checked_pack_bytes: usize,
    pub letter_blake3: String,
    pub locator_blake3: String,
}

impl AwsS3ProbeClient {
    /// Reusing immutable content is allowed ONLY if server readback proves
    /// byte-exact equality. A precondition rejection or lost ACK never
    /// results in an unguarded overwrite or a blind retry of S3 PUT.
    async fn coord_store_immutable_exact(
        &self, key: &str, bytes: &[u8],
    ) -> Result<(), S3CoordImmutablePublishError> {
        if bytes.is_empty() || bytes.len() > 512 * 1024 {
            return Err(S3CoordImmutablePublishError::InvalidPack);
        }
        match self.put_conditionally(
            key, bytes, S3PutCondition::IfAbsent,
        ).await {
            Ok(_) | Err(S3TransportError::PreconditionFailed)
            | Err(S3TransportError::CommitUncertain) => {}
            Err(err) => return Err(err.into()),
        }
        let read = self.get_bounded(key, bytes.len() as u64).await?;
        if read.bytes != bytes
            || read.blake3 != blake3::hash(bytes).to_hex().to_string()
        {
            return Err(S3CoordImmutablePublishError::ImmutableConflict);
        }
        Ok(())
    }

    /// Real S3 immutable prepublication, deliberately WITHOUT mutable root
    /// publication. Requires a previously full-pinned, profile-bound old V1
    /// snapshot and fully uploaded candidate V1 revision/shard objects.
    ///
    /// Before writing a V2 object, re-pin the entire proposed immutable V1
    /// manifest (including ALL old and new revision JSON, blobs and DAG)
    /// and verify exactly the same delta coverage and pack bytes as the
    /// 02c verifier. This first sender barrier is intentionally conservative.
    ///
    /// An invalid scope, missing V1 blob, old-history deletion, forged
    /// letter/pack, wrong ETag or missing genesis anchor fails closed.
    /// If the root changes during immutable staging, the returned error
    /// leaves only unreferenced immutable objects; no CAS is attempted.
    #[allow(dead_code)]
    pub(super) async fn prepublish_coord_v2_immutables(
        &self,
        scope: &S3RemoteAnchorScope,
        previous: &S3VerifiedHead,
        previous_pinned: &S3PinnedRevisionSnapshot,
        proposed: &S3HeadPointerV1,
        candidate: &S3ManifestV1,
        letter: &S3DeltaEnvelopeV2,
        prepared: &PreparedDeltaPacksV2,
    ) -> Result<S3CoordImmutablePrepublishPreview, S3CoordImmutablePublishError> {
        let ns = self.bound_keyspace();
        if !scope.matches_connection(self.bound_profile(), ns) {
            return Err(S3CoordImmutablePublishError::ScopeMismatch);
        }
        if !proposed.is_direct_successor_of(&previous.pointer)
            || !candidate.is_successor_of(
                &previous.manifest, previous.pointer.manifest_blake3(),
            )
            || proposed.manifest_id() != candidate.manifest_id
            || proposed.manifest_blake3() != candidate.digest(ns)
                .map_err(|_| S3CoordImmutablePublishError::InvalidCandidate)?
            || previous_pinned.snapshot_id != format!(
                "{}:{}", previous.pointer.manifest_id(),
                previous.pointer.manifest_blake3()
            )
        {
            return Err(S3CoordImmutablePublishError::InvalidPredecessor);
        }
        let declared = prepared.packs.iter()
            .try_fold(0_u64, |sum, pack| {
                sum.checked_add(pack.payload.len() as u64)
            }).ok_or(S3CoordImmutablePublishError::InvalidPack)?;
        if prepared.references.is_empty()
            || prepared.packs.is_empty()
            || declared == 0 || declared > MAX_OFFLINE_PACK_BYTES
            || declared != prepared.packed_bytes as u64
            || prepared.packs.len() != letter.packs().len()
            || prepared.references != letter.revisions()
            || prepared.packs.iter().zip(letter.packs()).any(|(pack, index)| {
                let start = pack.index.first_revision_index as usize;
                let end = start.checked_add(pack.index.revision_count as usize);
                &pack.index != index
                    || pack.index.pack_blake3 != pack.descriptor.digest
                    || ns.immutable_blob_key(&pack.index.pack_blake3)
                        .ok().as_deref() != Some(pack.descriptor.key.as_str())
                    || end.and_then(|end| prepared.references.get(start..end))
                        != Some(pack.descriptor.references.as_slice())
                    || blake3::hash(&pack.payload).to_hex().as_str()
                        != pack.index.pack_blake3
            })
        {
            return Err(S3CoordImmutablePublishError::InvalidPack);
        }

        // Establish that the mutable S3 head is still the independent
        // previously verified value BEFORE touching any immutable key.
        let initial = self.read_verified_head().await?
            .ok_or(S3CoordImmutablePublishError::RemoteRootChanged)?;
        if &initial != previous {
            return Err(S3CoordImmutablePublishError::RemoteRootChanged);
        }
        let (on_s3, current_pinned) = self
            .pin_historical_manifest_snapshot(
                candidate.manifest_id, proposed.manifest_blake3(),
            ).await?;
        if on_s3 != *candidate {
            return Err(S3CoordImmutablePublishError::InvalidCandidate);
        }
        let coverage = letter.verify_pinned_v1_coverage(
            ns, &previous.pointer, &previous.manifest, previous_pinned,
            proposed, candidate, &current_pinned,
        )?;
        let delta_ids: Vec<_> = prepared.references.iter()
            .map(|reference| reference.revision_id).collect();
        if delta_ids != coverage.added_revision_ids
            || coverage.required_pack_count != prepared.packs.len()
        {
            return Err(S3CoordImmutablePublishError::InvalidCandidate);
        }

        // Publication ordering is critical: bytes -> letter -> locator.
        // An unreachable orphan after network loss is preferable to a
        // canonical V1 root whose referenced delta locator is still absent.
        for pack in &prepared.packs {
            self.coord_store_immutable_exact(
                &pack.descriptor.key, &pack.payload,
            ).await?;
        }
        let letter_bytes = letter.encode(ns)?;
        let letter_digest = blake3::hash(&letter_bytes).to_hex().to_string();
        let letter_key = ns.immutable_blob_key(&letter_digest)
            .map_err(|_| S3CoordImmutablePublishError::InvalidCandidate)?;
        self.coord_store_immutable_exact(&letter_key, &letter_bytes).await?;

        let locator = S3CoordLetterLocatorV2::new(
            ns, proposed, &letter_digest,
        ).map_err(|_| S3CoordImmutablePublishError::InvalidCandidate)?;
        let locator_bytes = locator.encode(ns, proposed)
            .map_err(|_| S3CoordImmutablePublishError::InvalidCandidate)?;
        self.coord_store_immutable_exact(
            &ns.immutable_coord_letter_locator_key(proposed.manifest_id()),
            &locator_bytes,
        ).await?;

        // A concurrent V1 root CAS must not be disguised as successful
        // preparation for an obsolete parent. The caller still must use
        // the EXISTING crash-durable S3HeadJournal and publish_head_cas.
        let final_head = self.read_verified_head().await?
            .ok_or(S3CoordImmutablePublishError::RemoteRootChanged)?;
        if final_head != initial {
            return Err(S3CoordImmutablePublishError::RemoteRootChanged);
        }
        Ok(S3CoordImmutablePrepublishPreview {
            generation: proposed.generation(),
            pack_count: prepared.packs.len(),
            checked_pack_bytes: prepared.packed_bytes,
            letter_blake3: letter_digest,
            locator_blake3: blake3::hash(&locator_bytes).to_hex().to_string(),
        })
    }
}
