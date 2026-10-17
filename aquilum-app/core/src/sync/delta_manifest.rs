//! SYNC-DELTA-01c: inert, versioned immutable delta-manifest and chain proof.
//!
//! This module performs no network, file, or remote object mutations. Callers
//! must independently verify that the supplied head and checkpoint are bound
//! to one authorized vault, remote root and storage profile. Content hashes
//! prove consistency relative to that head, NOT writer authenticity/anti-rollback.
//! Missing history means "hold or obtain a separately verified full snapshot",
//! never permission to delete local bytes or advance a baseline.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{OperationId, RevisionId, S3ContractError, S3HeadPointerV1, S3Keyspace, VaultId};

const SCHEMA: u16 = 1;
pub const MAX_DELTA_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_REVISIONS_PER_MANIFEST: usize = 4096;
const MAX_DELTA_CHAIN_DEPTH: usize = 128;
const MAX_DELTA_CHAIN_REVISIONS: usize = 100_000;

fn valid_digest(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeltaManifestParentV1 {
    manifest_id: OperationId,
    manifest_blake3: String,
}

impl DeltaManifestParentV1 {
    pub fn new(manifest_id: OperationId, manifest_blake3: String) -> Result<Self, DeltaManifestError> {
        if !valid_digest(&manifest_blake3) {
            return Err(DeltaManifestError::InvalidPredecessor);
        }
        Ok(Self { manifest_id, manifest_blake3 })
    }

    pub fn manifest_id(&self) -> OperationId {
        self.manifest_id
    }

    pub fn manifest_blake3(&self) -> &str {
        &self.manifest_blake3
    }
}

/// Experimental immutable metadata envelope; not yet registered or published
/// as an active cloud wire feature. Contains opaque revision IDs, not paths.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeltaManifestV1 {
    schema: u16,
    vault_id: VaultId,
    manifest_id: OperationId,
    generation: u64,
    previous: Option<DeltaManifestParentV1>,
    revision_ids: Vec<RevisionId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeltaManifestError {
    Malformed,
    Oversized,
    UnsupportedSchema,
    ForeignVault,
    InvalidGeneration,
    InvalidPredecessor,
    NonCanonicalRevisions,
    TooManyRevisions,
}

impl DeltaManifestV1 {
    pub fn new(
        keyspace: &S3Keyspace,
        manifest_id: OperationId,
        generation: u64,
        previous: Option<DeltaManifestParentV1>,
        mut revision_ids: Vec<RevisionId>,
    ) -> Result<Self, DeltaManifestError> {
        revision_ids.sort_unstable();
        let manifest = Self {
            schema: SCHEMA,
            vault_id: keyspace.vault_id(),
            manifest_id,
            generation,
            previous,
            revision_ids,
        };
        manifest.validate(keyspace)?;
        Ok(manifest)
    }

    pub fn encode(&self, keyspace: &S3Keyspace) -> Result<Vec<u8>, DeltaManifestError> {
        self.validate(keyspace)?;
        let bytes = serde_json::to_vec(self).map_err(|_| DeltaManifestError::Malformed)?;
        if bytes.len() > MAX_DELTA_MANIFEST_BYTES {
            return Err(DeltaManifestError::Oversized);
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8], keyspace: &S3Keyspace) -> Result<Self, DeltaManifestError> {
        if bytes.len() > MAX_DELTA_MANIFEST_BYTES {
            return Err(DeltaManifestError::Oversized);
        }
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|_| DeltaManifestError::Malformed)?;
        manifest.validate(keyspace)?;
        Ok(manifest)
    }

    fn validate(&self, keyspace: &S3Keyspace) -> Result<(), DeltaManifestError> {
        if self.schema != SCHEMA {
            return Err(DeltaManifestError::UnsupportedSchema);
        }
        if self.vault_id != keyspace.vault_id() {
            return Err(DeltaManifestError::ForeignVault);
        }
        if self.generation == 0 || (self.generation == 1) != self.previous.is_none() {
            return Err(DeltaManifestError::InvalidGeneration);
        }
        if let Some(previous) = &self.previous {
            if previous.manifest_id == self.manifest_id
                || !valid_digest(&previous.manifest_blake3)
            {
                return Err(DeltaManifestError::InvalidPredecessor);
            }
        }
        if self.revision_ids.len() > MAX_REVISIONS_PER_MANIFEST {
            return Err(DeltaManifestError::TooManyRevisions);
        }
        if self.revision_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(DeltaManifestError::NonCanonicalRevisions);
        }
        Ok(())
    }

    pub fn manifest_id(&self) -> OperationId {
        self.manifest_id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn previous(&self) -> Option<&DeltaManifestParentV1> {
        self.previous.as_ref()
    }

    pub fn revision_ids(&self) -> &[RevisionId] {
        &self.revision_ids
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeltaChainLimits {
    pub max_manifests: usize,
    pub max_revision_ids: usize,
}

impl Default for DeltaChainLimits {
    fn default() -> Self {
        Self {
            max_manifests: MAX_DELTA_CHAIN_DEPTH,
            max_revision_ids: MAX_DELTA_CHAIN_REVISIONS,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeltaChainError {
    InvalidLimit,
    InvalidHead(S3ContractError),
    InvalidManifest(DeltaManifestError),
    HeadRollbackOrFork,
    ManifestHashMismatch,
    ManifestIdentityMismatch,
    HeadManifestMismatch,
    RepeatedRevision,
    TrailingManifestBytes,
    BrokenGenerationChain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeltaChainBase {
    VerifiedCheckpoint,
    Genesis,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeltaSnapshotReason {
    DepthLimit,
    RevisionLimit,
    NoCommonAncestor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeltaChainResult {
    /// A matching remote checkpoint says nothing about pending local edits.
    HeadUnchanged,
    /// A complete manifest sequence, NOT proof of local file/payload safety.
    Complete {
        revision_ids: Vec<RevisionId>,
        manifest_count: usize,
        base: DeltaChainBase,
    },
    /// Fetch this exact older immutable object and re-validate the whole chain.
    NeedOlderManifest {
        manifest_id: OperationId,
        expected_blake3: String,
        generation: u64,
    },
    RequiresVerifiedSnapshot(DeltaSnapshotReason),
}

/// Walk newest-first serialized immutable delta manifests until the confirmed
/// checkpoint or genesis. Relative BLAKE3 links are verified at each step.
///
/// Both head and checkpoint must have been independently verified and must be
/// bound to the SAME S3 profile/root. No network I/O or file effects occur here.
pub fn inspect_delta_manifest_chain(
    keyspace: &S3Keyspace,
    head: &S3HeadPointerV1,
    checkpoint: Option<&S3HeadPointerV1>,
    newest_first: &[Vec<u8>],
    limits: DeltaChainLimits,
) -> Result<DeltaChainResult, DeltaChainError> {
    if limits.max_manifests == 0 || limits.max_revision_ids == 0
        || limits.max_manifests > MAX_DELTA_CHAIN_DEPTH
        || limits.max_revision_ids > MAX_DELTA_CHAIN_REVISIONS
    {
        return Err(DeltaChainError::InvalidLimit);
    }
    head.encode(keyspace).map_err(DeltaChainError::InvalidHead)?;
    if let Some(known) = checkpoint {
        known.encode(keyspace).map_err(DeltaChainError::InvalidHead)?;
        if known.generation() > head.generation()
            || (known.generation() == head.generation() && known != head)
        {
            return Err(DeltaChainError::HeadRollbackOrFork);
        }
        if known == head {
            return if newest_first.is_empty() {
                Ok(DeltaChainResult::HeadUnchanged)
            } else {
                Err(DeltaChainError::TrailingManifestBytes)
            };
        }
    }

    if newest_first.len() > limits.max_manifests {
        return Ok(DeltaChainResult::RequiresVerifiedSnapshot(
            DeltaSnapshotReason::DepthLimit,
        ));
    }

    let mut next_id = head.manifest_id();
    let mut next_hash = head.manifest_blake3().to_owned();
    let mut next_generation = head.generation();
    let mut seen_revisions = BTreeSet::new();
    let mut batches = Vec::new();
    let mut count = 0usize;

    for (index, bytes) in newest_first.iter().enumerate() {
        if blake3::hash(bytes).to_hex().as_str() != next_hash {
            return Err(DeltaChainError::ManifestHashMismatch);
        }
        let manifest = DeltaManifestV1::decode(bytes, keyspace)
            .map_err(DeltaChainError::InvalidManifest)?;
        if manifest.manifest_id() != next_id || manifest.generation() != next_generation {
            return Err(DeltaChainError::ManifestIdentityMismatch);
        }
        if index == 0
            && manifest.previous().map(DeltaManifestParentV1::manifest_id)
                != head.previous_manifest_id()
        {
            return Err(DeltaChainError::HeadManifestMismatch);
        }

        count = count.saturating_add(manifest.revision_ids().len());
        if count > limits.max_revision_ids {
            return Ok(DeltaChainResult::RequiresVerifiedSnapshot(
                DeltaSnapshotReason::RevisionLimit,
            ));
        }
        for revision in manifest.revision_ids() {
            if !seen_revisions.insert(*revision) {
                return Err(DeltaChainError::RepeatedRevision);
            }
        }
        batches.push(manifest.revision_ids().to_vec());

        let Some(parent) = manifest.previous() else {
            if checkpoint.is_some() {
                return Ok(DeltaChainResult::RequiresVerifiedSnapshot(
                    DeltaSnapshotReason::NoCommonAncestor,
                ));
            }
            if index + 1 != newest_first.len() {
                return Err(DeltaChainError::TrailingManifestBytes);
            }
            return Ok(complete(batches, DeltaChainBase::Genesis));
        };

        next_generation = next_generation
            .checked_sub(1)
            .ok_or(DeltaChainError::BrokenGenerationChain)?;
        if next_generation == 0 {
            return Err(DeltaChainError::BrokenGenerationChain);
        }
        next_id = parent.manifest_id();
        next_hash = parent.manifest_blake3().to_owned();

        if let Some(known) = checkpoint {
            if next_generation == known.generation() {
                if next_id != known.manifest_id()
                    || next_hash != known.manifest_blake3()
                {
                    return Ok(DeltaChainResult::RequiresVerifiedSnapshot(
                        DeltaSnapshotReason::NoCommonAncestor,
                    ));
                }
                if index + 1 != newest_first.len() {
                    return Err(DeltaChainError::TrailingManifestBytes);
                }
                return Ok(complete(batches, DeltaChainBase::VerifiedCheckpoint));
            }
            if next_generation < known.generation() {
                return Err(DeltaChainError::BrokenGenerationChain);
            }
        }
    }

    if newest_first.len() == limits.max_manifests {
        Ok(DeltaChainResult::RequiresVerifiedSnapshot(
            DeltaSnapshotReason::DepthLimit,
        ))
    } else {
        Ok(DeltaChainResult::NeedOlderManifest {
            manifest_id: next_id,
            expected_blake3: next_hash,
            generation: next_generation,
        })
    }
}

fn complete(batches: Vec<Vec<RevisionId>>, base: DeltaChainBase) -> DeltaChainResult {
    let manifest_count = batches.len();
    DeltaChainResult::Complete {
        // Chronological manifest order. Intra-manifest revision IDs are
        // canonical, not topological; RevisionDag must still check ancestry.
        revision_ids: batches.into_iter().rev().flatten().collect(),
        manifest_count,
        base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> S3Keyspace {
        S3Keyspace::new("akula/delta", VaultId::new()).unwrap()
    }

    fn digest(bytes: &[u8]) -> String {
        blake3::hash(bytes).to_hex().to_string()
    }

    fn frame(
        ns: &S3Keyspace,
        generation: u64,
        previous: Option<(OperationId, String)>,
        changes: Vec<RevisionId>,
    ) -> (Vec<u8>, S3HeadPointerV1) {
        let id = OperationId::new();
        let previous = previous
            .map(|(parent_id, hash)| DeltaManifestParentV1::new(parent_id, hash).unwrap());
        let manifest = DeltaManifestV1::new(ns, id, generation, previous.clone(), changes).unwrap();
        let bytes = manifest.encode(ns).unwrap();
        let head = S3HeadPointerV1::new(
            ns, generation, id, &digest(&bytes),
            previous.map(|entry| entry.manifest_id()),
        ).unwrap();
        (bytes, head)
    }

    fn link(head: &S3HeadPointerV1) -> Option<(OperationId, String)> {
        Some((head.manifest_id(), head.manifest_blake3().to_owned()))
    }

    fn inspect(
        ns: &S3Keyspace,
        head: &S3HeadPointerV1,
        base: Option<&S3HeadPointerV1>,
        frames: &[Vec<u8>],
    ) -> Result<DeltaChainResult, DeltaChainError> {
        inspect_delta_manifest_chain(ns, head, base, frames, DeltaChainLimits::default())
    }

    #[test]
    fn unchanged_head_requires_no_manifest_read() {
        let ns = vault();
        let (bytes, head) = frame(&ns, 1, None, vec![RevisionId::new()]);
        assert_eq!(inspect(&ns, &head, Some(&head), &[]), Ok(DeltaChainResult::HeadUnchanged));
        assert_eq!(inspect(&ns, &head, Some(&head), &[bytes]),
            Err(DeltaChainError::TrailingManifestBytes));
    }

    #[test]
    fn seven_changes_do_not_require_ten_thousand_unmodified_revision_records() {
        let ns = vault();
        let (_, base) = frame(&ns, 1, None, vec![]);
        let changed = (0..7).map(|_| RevisionId::new()).collect::<Vec<_>>();
        let (frame2, head2) = frame(&ns, 2, link(&base), changed.clone());
        let mut expected = changed;
        expected.sort_unstable();
        assert_eq!(
            inspect(&ns, &head2, Some(&base), &[frame2]),
            Ok(DeltaChainResult::Complete {
                revision_ids: expected,
                manifest_count: 1,
                base: DeltaChainBase::VerifiedCheckpoint,
            })
        );
    }

    #[test]
    fn offline_chain_walks_multiple_generations_and_requests_missing_ancestor() {
        let ns = vault();
        let (_, base) = frame(&ns, 1, None, vec![RevisionId::new()]);
        let a = RevisionId::new();
        let b = RevisionId::new();
        let (bytes2, head2) = frame(&ns, 2, link(&base), vec![a]);
        let (bytes3, head3) = frame(&ns, 3, link(&head2), vec![b]);
        assert_eq!(
            inspect(&ns, &head3, Some(&base), &[bytes3.clone()]),
            Ok(DeltaChainResult::NeedOlderManifest {
                manifest_id: head2.manifest_id(),
                expected_blake3: head2.manifest_blake3().to_owned(),
                generation: 2,
            })
        );
        assert_eq!(
            inspect(&ns, &head3, Some(&base), &[bytes3, bytes2]),
            Ok(DeltaChainResult::Complete {
                revision_ids: vec![a, b],
                manifest_count: 2,
                base: DeltaChainBase::VerifiedCheckpoint,
            })
        );
    }

    #[test]
    fn first_sync_requires_full_genesis_chain_and_remains_read_only() {
        let ns = vault();
        let revision = RevisionId::new();
        let (bytes, head) = frame(&ns, 1, None, vec![revision]);
        assert_eq!(
            inspect(&ns, &head, None, &[bytes]),
            Ok(DeltaChainResult::Complete {
                revision_ids: vec![revision],
                manifest_count: 1,
                base: DeltaChainBase::Genesis,
            })
        );
    }

    #[test]
    fn unknown_schema_foreign_vault_extension_and_size_fail_closed() {
        let ns = vault();
        let (raw, _) = frame(&ns, 1, None, vec![]);
        let mut value: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        value["schema"] = 2.into();
        assert_eq!(
            DeltaManifestV1::decode(&serde_json::to_vec(&value).unwrap(), &ns),
            Err(DeltaManifestError::UnsupportedSchema)
        );
        value["schema"] = 1.into();
        value["wipe_local"] = true.into();
        assert_eq!(
            DeltaManifestV1::decode(&serde_json::to_vec(&value).unwrap(), &ns),
            Err(DeltaManifestError::Malformed)
        );
        assert_eq!(
            DeltaManifestV1::decode(&raw, &vault()),
            Err(DeltaManifestError::ForeignVault)
        );
        assert_eq!(
            DeltaManifestV1::decode(&vec![b' '; MAX_DELTA_MANIFEST_BYTES + 1], &ns),
            Err(DeltaManifestError::Oversized)
        );
    }

    #[test]
    fn corrupted_ancestor_is_rejected_and_missing_one_is_not_a_valid_delta() {
        let ns = vault();
        let (_, base) = frame(&ns, 1, None, vec![]);
        let (bytes2, head2) = frame(&ns, 2, link(&base), vec![RevisionId::new()]);
        let (bytes3, head3) = frame(&ns, 3, link(&head2), vec![RevisionId::new()]);
        let mut tampered = bytes2;
        tampered.push(b' ');
        assert_eq!(
            inspect(&ns, &head3, Some(&base), &[bytes3, tampered]),
            Err(DeltaChainError::ManifestHashMismatch)
        );
    }

    #[test]
    fn mismatched_head_parent_and_invalid_digest_fail_closed() {
        let ns = vault();
        let (_, base) = frame(&ns, 1, None, vec![]);
        let (bytes2, head2) = frame(&ns, 2, link(&base), vec![RevisionId::new()]);
        let bogus = S3HeadPointerV1::new(
            &ns, 2, head2.manifest_id(), head2.manifest_blake3(),
            Some(OperationId::new())
        ).unwrap();
        assert_eq!(
            inspect(&ns, &bogus, Some(&base), &[bytes2]),
            Err(DeltaChainError::HeadManifestMismatch)
        );
        assert_eq!(
            DeltaManifestParentV1::new(base.manifest_id(), "z".repeat(64)),
            Err(DeltaManifestError::InvalidPredecessor)
        );
    }

    #[test]
    fn repeated_revisions_across_generations_are_never_silently_deduplicated() {
        let ns = vault();
        let (_, base) = frame(&ns, 1, None, vec![]);
        let repeated = RevisionId::new();
        let (bytes2, head2) = frame(&ns, 2, link(&base), vec![repeated]);
        let (bytes3, head3) = frame(&ns, 3, link(&head2), vec![repeated]);
        assert_eq!(
            inspect(&ns, &head3, Some(&base), &[bytes3, bytes2]),
            Err(DeltaChainError::RepeatedRevision)
        );
    }

    #[test]
    fn rollback_fork_and_unrelated_baseline_hold_instead_of_rewriting_history() {
        let ns = vault();
        let (_, base) = frame(&ns, 1, None, vec![]);
        let (bytes2, current) = frame(&ns, 2, link(&base), vec![RevisionId::new()]);
        assert_eq!(
            inspect(&ns, &base, Some(&current), &[]),
            Err(DeltaChainError::HeadRollbackOrFork)
        );
        let (_, alternate) = frame(&ns, 2, link(&base), vec![RevisionId::new()]);
        assert_eq!(
            inspect(&ns, &current, Some(&alternate), &[]),
            Err(DeltaChainError::HeadRollbackOrFork)
        );
        let (_, unrelated) = frame(&ns, 1, None, vec![]);
        assert_eq!(
            inspect(&ns, &current, Some(&unrelated), &[bytes2]),
            Ok(DeltaChainResult::RequiresVerifiedSnapshot(
                DeltaSnapshotReason::NoCommonAncestor,
            ))
        );
    }

    #[test]
    fn bounded_chain_expires_to_verified_snapshot_not_partial_state() {
        let ns = vault();
        let (_, base) = frame(&ns, 1, None, vec![]);
        let (_, second) = frame(&ns, 2, link(&base), vec![]);
        let (bytes3, current) = frame(&ns, 3, link(&second), vec![]);
        assert_eq!(
            inspect_delta_manifest_chain(
                &ns, &current, Some(&base), &[bytes3],
                DeltaChainLimits { max_manifests: 1, max_revision_ids: 10 },
            ),
            Ok(DeltaChainResult::RequiresVerifiedSnapshot(
                DeltaSnapshotReason::DepthLimit,
            ))
        );
    }

    #[test]
    fn delta_manifest_metadata_has_no_user_file_paths_or_delete_authority() {
        let ns = vault();
        let (bytes, head) = frame(&ns, 1, None, vec![RevisionId::new()]);
        let metadata = String::from_utf8(bytes).unwrap();
        assert!(!metadata.contains(".md"));
        assert!(!metadata.contains("notes/"));
        assert!(!metadata.contains("delete_file"));
        assert_eq!(head.generation(), 1);
    }
}
