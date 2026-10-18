//! SYNC-COORD-03b: discover only V1-anchored V2 delta letters.
//!
//! One optional, write-once hint exists at a deterministic key derived from
//! an IMMUTABLE V1 manifest ID. The hint is NOT a trusted head or signature.
//! The reader first authenticates every missed V1 manifest against the saved
//! scoped SQLite checkpoint, then binds every hint to its exact canonical
//! generation, and finally runs the full 02e pack/member/history verifier.
//! Missing, stale, forged or foreign hints HOLD; never infer zero changes.

use serde::{Deserialize, Serialize};

use super::{
    AwsS3ProbeClient, S3CoordOfflineChainError, S3CoordSelectivePacksPreview,
    S3CoordSidecarReadError, S3PinnedRevisionSnapshot,
    S3VerifiedRevisionMetadataStore,
};
use super::coord_chain::MAX_OFFLINE_GENERATIONS;
use crate::sync::{
    walk_s3_delta_from_saved_anchor, OperationId, S3HeadPointerV1, S3Keyspace,
    S3ManifestDeltaBase, S3ManifestDeltaLimits, S3ManifestDeltaOutcome,
    S3ManifestV1, S3RemoteAnchorJournal, S3RemoteAnchorScope, VaultId,
};

const SCHEMA: u16 = 2;
const MAX_LOCATOR_BYTES: usize = 2048;
// Deliberately the generic V1 walker ceiling, NOT a new wire format.
// Only the separately named advisory long-offline API uses this budget.
// 128 * 256 remains inside S3ManifestDeltaLimits' 32,768 shard-ref cap.
const MAX_LONG_OFFLINE_GENERATIONS: usize = 128;

/// Only advisory immutable metadata. No local paths or credentials, no
/// publisher capability, no confirmed baseline or mutable current-root write.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct S3CoordLetterLocatorV2 {
    schema: u16,
    vault_id: VaultId,
    generation: u64,
    manifest_id: OperationId,
    manifest_blake3: String,
    letter_blake3: String,
}

impl S3CoordLetterLocatorV2 {
    pub(super) fn new(
        ns: &S3Keyspace,
        head: &S3HeadPointerV1,
        letter_hash: &str,
    ) -> Result<Self, S3CoordOfflineChainError> {
        let value = Self {
            schema: SCHEMA,
            vault_id: ns.vault_id(),
            generation: head.generation(),
            manifest_id: head.manifest_id(),
            manifest_blake3: head.manifest_blake3().to_owned(),
            letter_blake3: letter_hash.to_owned(),
        };
        if !value.matches(ns, head) {
            return Err(S3CoordOfflineChainError::InvalidChain);
        }
        Ok(value)
    }

    fn matches(&self, ns: &S3Keyspace, head: &S3HeadPointerV1) -> bool {
        self.schema == SCHEMA
            && self.vault_id == ns.vault_id()
            && self.generation >= 2
            && self.generation == head.generation()
            && self.manifest_id == head.manifest_id()
            && self.manifest_blake3 == head.manifest_blake3()
            && head.previous_manifest_id().is_some()
            && head.encode(ns).is_ok()
            && ns.immutable_blob_key(&self.letter_blake3).is_ok()
    }

    pub(super) fn encode(
        &self, ns: &S3Keyspace,
        head: &S3HeadPointerV1,
    ) -> Result<Vec<u8>, S3CoordOfflineChainError> {
        if !self.matches(ns, head) {
            return Err(S3CoordOfflineChainError::InvalidChain);
        }
        let raw = serde_json::to_vec(self)
            .map_err(|_| S3CoordOfflineChainError::InvalidChain)?;
        if raw.len() > MAX_LOCATOR_BYTES {
            return Err(S3CoordOfflineChainError::InvalidChain);
        }
        Ok(raw)
    }

    fn decode(
        ns: &S3Keyspace,
        head: &S3HeadPointerV1,
        bytes: &[u8],
    ) -> Result<Self, S3CoordOfflineChainError> {
        if bytes.len() > MAX_LOCATOR_BYTES {
            return Err(S3CoordOfflineChainError::InvalidChain);
        }
        let value: Self = serde_json::from_slice(bytes)
            .map_err(|_| S3CoordOfflineChainError::InvalidChain)?;
        if value.encode(ns, head)?.as_slice() != bytes {
            return Err(S3CoordOfflineChainError::InvalidChain);
        }
        Ok(value)
    }
}

impl AwsS3ProbeClient {
    /// A production-compiled read-only discovery entrypoint. It takes NO
    /// externally supplied letter hashes and performs NO S3 LIST.
    ///
    /// Every locator is derived from a trusted anchored V1 history frame
    /// and the complete 02e verifier rechecks all V1 inventories, V2 packs,
    /// parent DAG and the final mutable pointer/ETag before any preview.
    ///
    /// Missing hints are a hard HOLD, never a false empty update. Callers
    /// seeking a slow fallback must separately request the full V1 pin;
    /// this method does not silently weaken its own guarantee.
    pub async fn inspect_v2_offline_selective_packs_discovered(
        &self,
        scope: &S3RemoteAnchorScope,
        journal: &S3RemoteAnchorJournal,
        previous_pinned: &S3PinnedRevisionSnapshot,
        previous_metadata: &S3VerifiedRevisionMetadataStore,
    ) -> Result<S3CoordSelectivePacksPreview, S3CoordOfflineChainError> {
        self.inspect_v2_offline_selective_packs_discovered_bounded(
            scope, journal, previous_pinned, previous_metadata,
            MAX_OFFLINE_GENERATIONS,
        ).await
    }

    /// Explicit long-offline read-only inspection of 9..=128 missed V1
    /// generations, grouped into eight-generation LOCATOR windows without
    /// ever promoting a temporary checkpoint or changing the persisted
    /// original anchor. No native apply, writer authority or baseline.
    ///
    /// Global 16 MiB manifest/letter/pack and 32 MiB new-shard budgets still
    /// apply: more generations do not grant unlimited downloads. All V2
    /// letters and parent DAG are verified by the single 02e reader at end.
    pub async fn inspect_v2_long_offline_selective_packs_discovered(
        &self,
        scope: &S3RemoteAnchorScope,
        journal: &S3RemoteAnchorJournal,
        previous_pinned: &S3PinnedRevisionSnapshot,
        previous_metadata: &S3VerifiedRevisionMetadataStore,
    ) -> Result<S3CoordSelectivePacksPreview, S3CoordOfflineChainError> {
        self.inspect_v2_offline_selective_packs_discovered_bounded(
            scope, journal, previous_pinned, previous_metadata,
            MAX_LONG_OFFLINE_GENERATIONS,
        ).await
    }

    async fn inspect_v2_offline_selective_packs_discovered_bounded(
        &self,
        scope: &S3RemoteAnchorScope,
        journal: &S3RemoteAnchorJournal,
        previous_pinned: &S3PinnedRevisionSnapshot,
        previous_metadata: &S3VerifiedRevisionMetadataStore,
        max_generations: usize,
    ) -> Result<S3CoordSelectivePacksPreview, S3CoordOfflineChainError> {
        if max_generations == 0
            || max_generations > S3ManifestDeltaLimits::default().max_frames
        {
            return Err(S3CoordOfflineChainError::ExceededOfflineBudget);
        }
        if !scope.matches_connection(self.bound_profile(), self.bound_keyspace()) {
            return Err(S3CoordSidecarReadError::ScopeMismatch.into());
        }
        let ns = self.bound_keyspace();
        let anchor = journal.load(scope)
            .map_err(S3CoordSidecarReadError::from)?
            .ok_or(S3CoordSidecarReadError::MissingPreviousAnchor)?;
        let old_head = anchor.pointer();
        let old_id = format!("{}:{}", old_head.manifest_id(), old_head.manifest_blake3());
        if previous_pinned.snapshot_id != old_id
            || !previous_metadata.is_verified_for(ns.vault_id(), &old_id)
            || blake3::hash(anchor.manifest_bytes()).to_hex().as_str()
                != old_head.manifest_blake3()
        {
            return Err(S3CoordSidecarReadError::PreviousCacheNotPinned.into());
        }
        let history = walk_s3_delta_from_saved_anchor(
            self, scope, journal, S3ManifestDeltaLimits {
                max_frames: max_generations,
                max_total_shard_refs: max_generations * 256,
            },
        ).await?;
        let count = match &history.outcome {
            S3ManifestDeltaOutcome::Complete {
                generations_traversed,
                base: S3ManifestDeltaBase::VerifiedCheckpoint,
                ..
            } => *generations_traversed,
            _ => return Err(S3CoordOfflineChainError::InvalidChain),
        };
        if count == 0 || count > max_generations
            || history.verified_manifest_frames.len() != count
        {
            // An unchanged S3 head is NOT a synthetic V2 generation.
            return Err(S3CoordOfflineChainError::InvalidChain);
        }

        let mut prior = old_head.clone();
        let mut letters = Vec::with_capacity(count);
        // Reconstruct from the EXACT original saved anchor through windows.
        // A window boundary is only a batching boundary for locator GETs;
        // it is NOT a new trusted or durable sync checkpoint.
        let chronological: Vec<_> =
            history.verified_manifest_frames.iter().rev().collect();
        for window in chronological.chunks(MAX_OFFLINE_GENERATIONS) {
            for frame in window {
                let provisional: S3ManifestV1 = serde_json::from_slice(frame)
                    .map_err(|_| S3CoordOfflineChainError::InvalidChain)?;
                let manifest = S3ManifestV1::decode(
                    ns, provisional.manifest_id, frame,
                ).map_err(S3CoordSidecarReadError::from)?;
                let head = S3HeadPointerV1::new(
                    ns, manifest.generation, manifest.manifest_id,
                    &blake3::hash(frame).to_hex().to_string(),
                    manifest.previous_manifest_id,
                ).map_err(S3CoordSidecarReadError::from)?;
                if !head.is_direct_successor_of(&prior) {
                    return Err(S3CoordOfflineChainError::InvalidChain);
                }
                let key = ns.immutable_coord_letter_locator_key(manifest.manifest_id);
                let fetched = self.get_bounded(&key, MAX_LOCATOR_BYTES as u64)
                    .await.map_err(S3CoordSidecarReadError::from)?;
                let locator = S3CoordLetterLocatorV2::decode(
                    ns, &head, &fetched.bytes,
                )?;
                letters.push(locator.letter_blake3);
                prior = head;
            }
        }
        if prior != *history.observed_head.pointer() {
            return Err(S3CoordOfflineChainError::InvalidChain);
        }
        // Reverify canonical history, all old/new shards and every packed
        // member; never convert a discovered hint directly into authority.
        self.inspect_v2_offline_selective_packs_with_budget(
            scope, journal, previous_pinned, previous_metadata,
            &letters, max_generations,
        ).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (S3Keyspace, S3HeadPointerV1, String) {
        let ns = S3Keyspace::new("akula/coord-locator", VaultId::new()).unwrap();
        let head = S3HeadPointerV1::new(
            &ns, 2, OperationId::new(),
            &blake3::hash(b"manifest").to_hex().to_string(),
            Some(OperationId::new()),
        ).unwrap();
        let letter = blake3::hash(b"immutable letter").to_hex().to_string();
        (ns, head, letter)
    }

    #[test]
    fn long_offline_is_opt_in_and_capped_by_canonical_walk() {
        assert_eq!(MAX_OFFLINE_GENERATIONS, 8);
        assert_eq!(MAX_LONG_OFFLINE_GENERATIONS, 128);
        assert!(MAX_LONG_OFFLINE_GENERATIONS <= S3ManifestDeltaLimits::default().max_frames);
        assert_eq!(MAX_LONG_OFFLINE_GENERATIONS * 256,
            S3ManifestDeltaLimits::default().max_total_shard_refs);
        assert_eq!((9_usize + MAX_OFFLINE_GENERATIONS - 1) / MAX_OFFLINE_GENERATIONS, 2);
        assert_eq!((100_usize + MAX_OFFLINE_GENERATIONS - 1) / MAX_OFFLINE_GENERATIONS, 13);
    }

    #[test]
    fn canonical_locator_is_bound_to_exact_manifest_and_vault() {
        let (ns, head, letter) = fixture();
        let locator = S3CoordLetterLocatorV2::new(&ns, &head, &letter).unwrap();
        let bytes = locator.encode(&ns, &head).unwrap();
        assert_eq!(S3CoordLetterLocatorV2::decode(&ns, &head, &bytes).unwrap(), locator);
        assert!(ns.contains_key(&ns.immutable_coord_letter_locator_key(head.manifest_id())));

        let foreign = S3Keyspace::new("akula/coord-locator", VaultId::new()).unwrap();
        assert!(S3CoordLetterLocatorV2::decode(&foreign, &head, &bytes).is_err());
        let different = S3HeadPointerV1::new(
            &ns, 2, OperationId::new(), head.manifest_blake3(),
            head.previous_manifest_id(),
        ).unwrap();
        assert!(S3CoordLetterLocatorV2::decode(&ns, &different, &bytes).is_err());
    }

    #[test]
    fn rejects_noncanonical_corrupt_extra_schema_and_genesis() {
        let (ns, head, letter) = fixture();
        let locator = S3CoordLetterLocatorV2::new(&ns, &head, &letter).unwrap();
        let original = locator.encode(&ns, &head).unwrap();
        let mut spaced = original.clone();
        spaced.push(b' ');
        assert!(S3CoordLetterLocatorV2::decode(&ns, &head, &spaced).is_err());
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value["unexpected"] = serde_json::json!(true);
        assert!(S3CoordLetterLocatorV2::decode(
            &ns, &head, &serde_json::to_vec(&value).unwrap(),
        ).is_err());
        value.as_object_mut().unwrap().remove("unexpected");
        value["schema"] = serde_json::json!(3);
        assert!(S3CoordLetterLocatorV2::decode(
            &ns, &head, &serde_json::to_vec(&value).unwrap(),
        ).is_err());
        assert!(S3CoordLetterLocatorV2::new(&ns, &head, "not-a-hash").is_err());
        let genesis = S3HeadPointerV1::new(
            &ns, 1, OperationId::new(), head.manifest_blake3(), None,
        ).unwrap();
        assert!(S3CoordLetterLocatorV2::new(&ns, &genesis, &letter).is_err());
        assert!(!ns.contains_key(&format!(
            "{}/extra", ns.immutable_coord_letter_locator_key(head.manifest_id())
        )));
    }
}
