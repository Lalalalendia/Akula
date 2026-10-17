//! SYNC-DELTA-01c: read-only diff and ancestor inspection for the EXISTING
//! S3ManifestV1 sharded index. Never introduce a second remote manifest format.
//!
//! Inputs are descriptive bytes from a known S3 vault. A root/checkpoint must
//! be independently authenticated and bound to the provider profile by the
//! caller. BLAKE3 verifies byte linkage relative to that root; it does not
//! prove publisher identity, local-file safety or permission to advance a
//! baseline. Revisions and blobs are verified by the canonical S3 pinning
//! pipeline, NOT by this lightweight candidate-selection helper.
//!
//! An old shard missing from a new index is not proof of file deletion:
//! re-sharding can replace the shard object while retaining all revisions.

use std::collections::{BTreeMap, BTreeSet};

use super::s3::{S3ContractError, S3HeadPointerV1, S3Keyspace};
use super::s3_manifest::{S3ManifestError, S3ManifestShardRefV1, S3ManifestV1};
use super::OperationId;

const MAX_FRAMES: usize = 128;
const MAX_TOTAL_SHARD_REFS: usize = 32_768;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct S3ManifestDeltaLimits {
    pub max_frames: usize,
    pub max_total_shard_refs: usize,
}

impl Default for S3ManifestDeltaLimits {
    fn default() -> Self {
        Self {
            max_frames: MAX_FRAMES,
            max_total_shard_refs: MAX_TOTAL_SHARD_REFS,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum S3ManifestDeltaError {
    InvalidLimits,
    InvalidHead(S3ContractError),
    InvalidManifest(S3ManifestError),
    RootRollbackOrFork,
    ManifestDigestMismatch,
    ManifestGenerationMismatch,
    RootPreviousMismatch,
    ImmutableShardIdReused,
    ExtraFrames,
    InvalidChain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum S3ManifestDeltaBase {
    VerifiedCheckpoint,
    Genesis,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum S3ManifestSnapshotReason {
    TooManyFrames,
    TooManyShardReferences,
    NoCommonAncestor,
}

/// Lightweight index candidate plan ONLY. The caller must still fetch and
/// verify each referenced shard, revision and blob before doing anything else.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3ManifestShardDelta {
    pub candidate_shards: Vec<S3ManifestShardRefV1>,
    pub reused_shards: usize,
    /// Old shard references no longer shared by the current root. This is
    /// re-sharding evidence, NEVER evidence of removed document revisions.
    pub superseded_shards: usize,
    /// Do not skip retention validation when an older shard was replaced.
    pub requires_verified_revision_retention: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum S3ManifestDeltaOutcome {
    /// Identical remote pointer: still check pending local edits and File Gate.
    HeadUnchanged,
    /// All manifest bytes form a verified predecessor chain to a checkpoint
    /// or genesis. This is NOT a durable sync or remote snapshot receipt.
    Complete {
        generations_traversed: usize,
        base: S3ManifestDeltaBase,
        shards: S3ManifestShardDelta,
    },
    /// Fetch exactly this immutable manifest and inspect the augmented chain.
    NeedOlderManifest {
        manifest_id: OperationId,
        expected_blake3: String,
        generation: u64,
    },
    /// Stop delta mode, use independently verified complete snapshot.
    RequiresVerifiedSnapshot(S3ManifestSnapshotReason),
}

/// Verify a chronological predecessor chain using the canonical sharded
/// S3ManifestV1 schema. Frames MUST be ordered newest-first, raw immutable
/// bytes. The prior checkpoint includes its already verified manifest bytes;
/// the client must separately bind the checkpoint to the provider profile.
///
/// No S3 network calls, local writes, deletion, or baseline advancement.
pub fn inspect_s3_manifest_delta(
    keyspace: &S3Keyspace,
    current_head: &S3HeadPointerV1,
    checkpoint: Option<(&S3HeadPointerV1, &[u8])>,
    newest_first: &[Vec<u8>],
    limits: S3ManifestDeltaLimits,
) -> Result<S3ManifestDeltaOutcome, S3ManifestDeltaError> {
    if limits.max_frames == 0
        || limits.max_frames > MAX_FRAMES
        || limits.max_total_shard_refs == 0
        || limits.max_total_shard_refs > MAX_TOTAL_SHARD_REFS
    {
        return Err(S3ManifestDeltaError::InvalidLimits);
    }
    current_head
        .encode(keyspace)
        .map_err(S3ManifestDeltaError::InvalidHead)?;

    let prior = if let Some((checkpoint_head, checkpoint_bytes)) = checkpoint {
        checkpoint_head
            .encode(keyspace)
            .map_err(S3ManifestDeltaError::InvalidHead)?;
        if blake3::hash(checkpoint_bytes).to_hex().as_str()
            != checkpoint_head.manifest_blake3()
        {
            return Err(S3ManifestDeltaError::ManifestDigestMismatch);
        }
        let prior = S3ManifestV1::decode(
            keyspace,
            checkpoint_head.manifest_id(),
            checkpoint_bytes,
        )
        .map_err(S3ManifestDeltaError::InvalidManifest)?;
        if prior.generation != checkpoint_head.generation()
            || prior.previous_manifest_id != checkpoint_head.previous_manifest_id()
        {
            return Err(S3ManifestDeltaError::ManifestGenerationMismatch);
        }
        if checkpoint_head.generation() > current_head.generation()
            || (checkpoint_head.generation() == current_head.generation()
                && checkpoint_head != current_head)
        {
            return Err(S3ManifestDeltaError::RootRollbackOrFork);
        }
        if checkpoint_head == current_head {
            return if newest_first.is_empty() {
                Ok(S3ManifestDeltaOutcome::HeadUnchanged)
            } else {
                Err(S3ManifestDeltaError::ExtraFrames)
            };
        }
        Some((checkpoint_head, prior))
    } else {
        None
    };

    if newest_first.len() > limits.max_frames {
        return Ok(S3ManifestDeltaOutcome::RequiresVerifiedSnapshot(
            S3ManifestSnapshotReason::TooManyFrames,
        ));
    }

    let mut expected_id = current_head.manifest_id();
    let mut expected_hash = current_head.manifest_blake3().to_owned();
    let mut expected_generation = current_head.generation();
    let mut latest: Option<S3ManifestV1> = None;
    let mut seen_ids = BTreeSet::new();
    let mut total_shards = 0usize;

    for (index, bytes) in newest_first.iter().enumerate() {
        if blake3::hash(bytes).to_hex().as_str() != expected_hash {
            return Err(S3ManifestDeltaError::ManifestDigestMismatch);
        }
        let manifest = S3ManifestV1::decode(keyspace, expected_id, bytes)
            .map_err(S3ManifestDeltaError::InvalidManifest)?;
        if manifest.generation != expected_generation {
            return Err(S3ManifestDeltaError::ManifestGenerationMismatch);
        }
        if !seen_ids.insert(manifest.manifest_id) {
            return Err(S3ManifestDeltaError::InvalidChain);
        }
        if index == 0
            && manifest.previous_manifest_id != current_head.previous_manifest_id()
        {
            return Err(S3ManifestDeltaError::RootPreviousMismatch);
        }
        if latest.is_none() {
            latest = Some(manifest.clone());
        }
        total_shards = total_shards.saturating_add(manifest.shards.len());
        if total_shards > limits.max_total_shard_refs {
            return Ok(S3ManifestDeltaOutcome::RequiresVerifiedSnapshot(
                S3ManifestSnapshotReason::TooManyShardReferences,
            ));
        }

        match (manifest.previous_manifest_id, manifest.previous_manifest_blake3) {
            (None, None) => {
                if prior.is_some() {
                    return Ok(S3ManifestDeltaOutcome::RequiresVerifiedSnapshot(
                        S3ManifestSnapshotReason::NoCommonAncestor,
                    ));
                }
                if index + 1 != newest_first.len() {
                    return Err(S3ManifestDeltaError::ExtraFrames);
                }
                return build_result(latest.as_ref().unwrap(), None, index + 1);
            }
            (Some(parent_id), Some(parent_hash)) => {
                expected_generation = expected_generation
                    .checked_sub(1)
                    .ok_or(S3ManifestDeltaError::InvalidChain)?;
                if expected_generation == 0 {
                    return Err(S3ManifestDeltaError::InvalidChain);
                }
                expected_id = parent_id;
                expected_hash = parent_hash;
            }
            _ => return Err(S3ManifestDeltaError::InvalidChain),
        }

        if let Some((checkpoint_head, checkpoint_manifest)) = &prior {
            if expected_generation == checkpoint_head.generation() {
                if expected_id != checkpoint_head.manifest_id()
                    || expected_hash != checkpoint_head.manifest_blake3()
                {
                    return Ok(S3ManifestDeltaOutcome::RequiresVerifiedSnapshot(
                        S3ManifestSnapshotReason::NoCommonAncestor,
                    ));
                }
                if index + 1 != newest_first.len() {
                    return Err(S3ManifestDeltaError::ExtraFrames);
                }
                return build_result(
                    latest.as_ref().unwrap(),
                    Some(checkpoint_manifest),
                    index + 1,
                );
            }
            if expected_generation < checkpoint_head.generation() {
                return Err(S3ManifestDeltaError::InvalidChain);
            }
        }
    }

    if newest_first.len() == limits.max_frames {
        Ok(S3ManifestDeltaOutcome::RequiresVerifiedSnapshot(
            S3ManifestSnapshotReason::TooManyFrames,
        ))
    } else {
        Ok(S3ManifestDeltaOutcome::NeedOlderManifest {
            manifest_id: expected_id,
            expected_blake3: expected_hash,
            generation: expected_generation,
        })
    }
}

fn build_result(
    latest: &S3ManifestV1,
    prior: Option<&S3ManifestV1>,
    generations: usize,
) -> Result<S3ManifestDeltaOutcome, S3ManifestDeltaError> {
    let mut previous = BTreeMap::new();
    if let Some(prior) = prior {
        for shard in &prior.shards {
            previous.insert(shard.shard_id, shard);
        }
    }
    let current_ids: BTreeSet<_> = latest.shards.iter().map(|s| s.shard_id).collect();
    let mut candidate_shards = Vec::new();
    let mut reused_shards = 0usize;

    for shard in &latest.shards {
        match previous.get(&shard.shard_id) {
            Some(old) if **old == *shard => reused_shards += 1,
            Some(_) => return Err(S3ManifestDeltaError::ImmutableShardIdReused),
            None => candidate_shards.push(shard.clone()),
        }
    }
    let superseded_shards = previous
        .keys()
        .filter(|id| !current_ids.contains(id))
        .count();

    Ok(S3ManifestDeltaOutcome::Complete {
        generations_traversed: generations,
        base: if prior.is_some() {
            S3ManifestDeltaBase::VerifiedCheckpoint
        } else {
            S3ManifestDeltaBase::Genesis
        },
        shards: S3ManifestShardDelta {
            candidate_shards,
            reused_shards,
            superseded_shards,
            requires_verified_revision_retention: superseded_shards != 0,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{RevisionId, VaultId};

    fn ns() -> S3Keyspace {
        S3Keyspace::new("akula/delta", VaultId::new()).unwrap()
    }

    fn digest(bytes: &[u8]) -> String {
        blake3::hash(bytes).to_hex().to_string()
    }

    fn frame(
        ns: &S3Keyspace,
        generation: u64,
        parent: Option<&S3HeadPointerV1>,
        shards: Vec<S3ManifestShardRefV1>,
    ) -> (S3ManifestV1, Vec<u8>, S3HeadPointerV1) {
        let id = OperationId::new();
        let link = parent.map(|p| (p.manifest_id(), p.manifest_blake3().to_string()));
        let root = S3ManifestV1::new(ns, id, generation, link, shards).unwrap();
        let bytes = root.encode(ns).unwrap();
        let head = S3HeadPointerV1::new(
            ns,
            generation,
            id,
            &digest(&bytes),
            parent.map(S3HeadPointerV1::manifest_id),
        )
        .unwrap();
        (root, bytes, head)
    }

    fn shard(first: RevisionId, last: RevisionId, count: u16) -> S3ManifestShardRefV1 {
        S3ManifestShardRefV1 {
            shard_id: OperationId::new(),
            shard_blake3: digest(first.to_string().as_bytes()),
            count,
            first_revision: first,
            last_revision: last,
        }
    }

    fn check(
        ns: &S3Keyspace,
        current: &S3HeadPointerV1,
        cp: Option<(&S3HeadPointerV1, &[u8])>,
        frames: &[Vec<u8>],
    ) -> Result<S3ManifestDeltaOutcome, S3ManifestDeltaError> {
        inspect_s3_manifest_delta(ns, current, cp, frames, S3ManifestDeltaLimits::default())
    }

    #[test]
    fn unchanged_confirmed_head_does_not_need_manifest_download() {
        let ns = ns();
        let (_, root_bytes, head) = frame(&ns, 1, None, vec![]);
        assert_eq!(
            check(&ns, &head, Some((&head, &root_bytes)), &[]),
            Ok(S3ManifestDeltaOutcome::HeadUnchanged),
        );
        assert_eq!(
            check(&ns, &head, Some((&head, &root_bytes)), &[root_bytes.clone()]),
            Err(S3ManifestDeltaError::ExtraFrames),
        );
    }

    #[test]
    fn new_shard_is_the_only_candidate_when_old_refs_are_reused() {
        let ns = ns();
        let mut ids = vec![RevisionId::new(), RevisionId::new()];
        ids.sort_unstable();
        let kept = shard(ids[0], ids[0], 1);
        let new = shard(ids[1], ids[1], 1);
        let (_, oldbytes, oldhead) = frame(&ns, 1, None, vec![kept.clone()]);
        let (_, newbytes, newhead) = frame(
            &ns, 2, Some(&oldhead), vec![kept.clone(), new.clone()],
        );
        let result = check(&ns, &newhead, Some((&oldhead, &oldbytes)), &[newbytes]);
        assert_eq!(
            result,
            Ok(S3ManifestDeltaOutcome::Complete {
                generations_traversed: 1,
                base: S3ManifestDeltaBase::VerifiedCheckpoint,
                shards: S3ManifestShardDelta {
                    candidate_shards: vec![new],
                    reused_shards: 1,
                    superseded_shards: 0,
                    requires_verified_revision_retention: false,
                },
            })
        );
    }

    #[test]
    fn offline_generations_request_exact_missing_ancestor_and_reuse_stable_shards() {
        let ns = ns();
        let base_shard = shard(RevisionId::new(), RevisionId::new(), 1);
        // One independent revision ref; its content is deliberately opaque.
        let (_, cpbytes, cphead) = frame(&ns, 1, None, vec![base_shard.clone()]);
        let (_, b2, h2) = frame(&ns, 2, Some(&cphead), vec![base_shard.clone()]);
        let (_, b3, h3) = frame(&ns, 3, Some(&h2), vec![base_shard.clone()]);
        assert_eq!(
            check(&ns, &h3, Some((&cphead, &cpbytes)), &[b3.clone()]),
            Ok(S3ManifestDeltaOutcome::NeedOlderManifest {
                manifest_id: h2.manifest_id(),
                expected_blake3: h2.manifest_blake3().to_string(),
                generation: 2,
            })
        );
        assert_eq!(
            check(&ns, &h3, Some((&cphead, &cpbytes)), &[b3, b2]),
            Ok(S3ManifestDeltaOutcome::Complete {
                generations_traversed: 2,
                base: S3ManifestDeltaBase::VerifiedCheckpoint,
                shards: S3ManifestShardDelta {
                    candidate_shards: vec![],
                    reused_shards: 1,
                    superseded_shards: 0,
                    requires_verified_revision_retention: false,
                },
            })
        );
    }

    #[test]
    fn bootstrap_never_grants_file_write_authority() {
        let ns = ns();
        let s = shard(RevisionId::new(), RevisionId::new(), 1);
        let (_, bytes, head) = frame(&ns, 1, None, vec![s.clone()]);
        assert_eq!(
            check(&ns, &head, None, &[bytes]),
            Ok(S3ManifestDeltaOutcome::Complete {
                generations_traversed: 1,
                base: S3ManifestDeltaBase::Genesis,
                shards: S3ManifestShardDelta {
                    candidate_shards: vec![s],
                    reused_shards: 0,
                    superseded_shards: 0,
                    requires_verified_revision_retention: false,
                },
            })
        );
    }

    #[test]
    fn missing_old_shard_means_repack_not_deletion_authority() {
        let ns = ns();
        let id = RevisionId::new();
        let original = shard(id, id, 1);
        let replacement = shard(id, id, 1);
        let (_, prev_bytes, prev_head) = frame(&ns, 1, None, vec![original]);
        let (_, new_bytes, head) = frame(&ns, 2, Some(&prev_head), vec![replacement.clone()]);
        assert_eq!(
            check(&ns, &head, Some((&prev_head, &prev_bytes)), &[new_bytes]),
            Ok(S3ManifestDeltaOutcome::Complete {
                generations_traversed: 1,
                base: S3ManifestDeltaBase::VerifiedCheckpoint,
                shards: S3ManifestShardDelta {
                    candidate_shards: vec![replacement],
                    reused_shards: 0,
                    superseded_shards: 1,
                    requires_verified_revision_retention: true,
                },
            })
        );
    }

    #[test]
    fn same_immutable_shard_id_with_different_digest_is_rejected() {
        let ns = ns();
        let s = shard(RevisionId::new(), RevisionId::new(), 1);
        let mut edited = s.clone();
        edited.shard_blake3 = digest(b"changed");
        let (_, cpbytes, cphead) = frame(&ns, 1, None, vec![s]);
        let (_, bytes, head) = frame(&ns, 2, Some(&cphead), vec![edited]);
        assert_eq!(
            check(&ns, &head, Some((&cphead, &cpbytes)), &[bytes]),
            Err(S3ManifestDeltaError::ImmutableShardIdReused)
        );
    }

    #[test]
    fn tampered_checkpoint_or_descendant_cannot_produce_an_accepted_plan() {
        let ns = ns();
        let (_, cpbytes, cphead) = frame(&ns, 1, None, vec![]);
        let (_, bytes, head) = frame(&ns, 2, Some(&cphead), vec![]);
        let mut tampered = bytes.clone();
        tampered.push(b' ');
        assert_eq!(
            check(&ns, &head, Some((&cphead, &cpbytes)), &[tampered]),
            Err(S3ManifestDeltaError::ManifestDigestMismatch)
        );
        let mut wrong_cp = cpbytes;
        wrong_cp.push(b' ');
        assert_eq!(
            check(&ns, &head, Some((&cphead, &wrong_cp)), &[bytes]),
            Err(S3ManifestDeltaError::ManifestDigestMismatch)
        );
    }

    #[test]
    fn fork_and_rollback_require_a_hold_or_verified_snapshot() {
        let ns = ns();
        let (_, cpbytes, cphead) = frame(&ns, 1, None, vec![]);
        let (_, bytes, head) = frame(&ns, 2, Some(&cphead), vec![]);
        assert_eq!(
            check(&ns, &cphead, Some((&head, &bytes)), &[]),
            Err(S3ManifestDeltaError::RootRollbackOrFork)
        );
        let (_, fork_bytes, fork_head) = frame(&ns, 1, None, vec![]);
        assert_eq!(
            check(&ns, &fork_head, Some((&cphead, &cpbytes)), &[]),
            Err(S3ManifestDeltaError::RootRollbackOrFork)
        );
        let (_, other_bytes, other_head) = frame(&ns, 1, None, vec![]);
        let (_, child, child_head) = frame(&ns, 2, Some(&other_head), vec![]);
        assert_eq!(
            check(&ns, &child_head, Some((&cphead, &cpbytes)), &[child]),
            Ok(S3ManifestDeltaOutcome::RequiresVerifiedSnapshot(
                S3ManifestSnapshotReason::NoCommonAncestor
            ))
        );
        let _ = other_bytes;
    }

    #[test]
    fn finite_chain_budget_returns_verified_snapshot_request() {
        let ns = ns();
        let (_, cpbytes, cphead) = frame(&ns, 1, None, vec![]);
        let (_, _mid, midhead) = frame(&ns, 2, Some(&cphead), vec![]);
        let (_, current_bytes, current_head) = frame(&ns, 3, Some(&midhead), vec![]);
        assert_eq!(
            inspect_s3_manifest_delta(
                &ns,
                &current_head,
                Some((&cphead, &cpbytes)),
                &[current_bytes],
                S3ManifestDeltaLimits {
                    max_frames: 1,
                    max_total_shard_refs: 10,
                },
            ),
            Ok(S3ManifestDeltaOutcome::RequiresVerifiedSnapshot(
                S3ManifestSnapshotReason::TooManyFrames,
            ))
        );
    }

    #[test]
    fn ten_thousand_unchanged_references_stay_in_reused_shards() {
        let ns = ns();
        // Deterministic spaced IDs leave room for seven revisions in one old
        // shard. This is a metadata-only benchmark, not remote blob proof.
        let ids = (1..=10_000_u128)
            .map(|n| RevisionId::from_uuid(uuid::Uuid::from_u128(n * 100)))
            .collect::<Vec<_>>();
        let old_shards = ids.chunks(128)
            .map(|chunk| shard(chunk[0], *chunk.last().unwrap(), chunk.len() as u16))
            .collect::<Vec<_>>();
        let (_, cpbytes, cphead) = frame(&ns, 1, None, old_shards.clone());

        let target = (5_000_usize - 1) / 128;
        let group = &ids[target * 128..(target + 1) * 128];
        let mut revised = group.to_vec();
        for offset in 1..=7 {
            revised.push(RevisionId::from_uuid(
                uuid::Uuid::from_u128(5_000 * 100 + offset),
            ));
        }
        revised.sort_unstable();
        let replacement = revised.chunks(128)
            .map(|part| shard(part[0], *part.last().unwrap(), part.len() as u16))
            .collect::<Vec<_>>();
        let mut current = old_shards.clone();
        current.splice(target..=target, replacement.clone());
        let (_, new_bytes, new_head) = frame(&ns, 2, Some(&cphead), current);
        match check(&ns, &new_head, Some((&cphead, &cpbytes)), &[new_bytes]).unwrap() {
            S3ManifestDeltaOutcome::Complete { shards, .. } => {
                assert_eq!(shards.candidate_shards, replacement);
                assert_eq!(shards.reused_shards, 78);
                assert_eq!(shards.superseded_shards, 1);
                assert!(shards.requires_verified_revision_retention);
            }
            other => panic!("expected candidate plan, got {other:?}"),
        }
    }
}
