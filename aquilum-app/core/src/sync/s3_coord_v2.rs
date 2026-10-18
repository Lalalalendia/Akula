//! SYNC-COORD-02a: bounded, read-only v2 delta-letter metadata contract.
//!
//! The existing verified V1 manifest/DAG remains the sole source of truth.
//! This contract cannot publish an S3 root, authenticate a remote writer,
//! verify pack payloads, advance a document baseline or write local files.
//! BLAKE3 alone is NOT a signature or a trusted anti-rollback root.

use std::collections::BTreeSet;
use serde::{Deserialize, Serialize};

use super::s3_manifest::is_blake3;
#[cfg(feature = "sync-s3")]
use super::S3PinnedRevisionSnapshot;
use super::{
    OperationId, RevisionId, S3HeadPointerV1, S3Keyspace,
    S3ManifestV1, S3RevisionRefV1, VaultId, MAX_REVISIONS,
};

const SCHEMA: u16 = 2;
pub const MAX_DELTA_ENVELOPE_BYTES: usize = 512 * 1024;
pub const MAX_DELTA_ENVELOPE_REVISIONS: usize = 1_024;
pub const MAX_DELTA_PACK_BYTES: u32 = 512 * 1024;
pub const MAX_DELTA_PACK_MEMBERS: usize = 128;
const MIN_PACK_HEADER: u64 = 10;
const MIN_PACK_MEMBER_OVERHEAD: u64 = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum S3DeltaEnvelopeV2Error {
    UnsupportedSchema,
    ForeignVault,
    Oversized,
    InvalidEncoding,
    InvalidAnchor,
    UntrustedSnapshot,
    MissingAncestry,
    InvalidRevision,
    DuplicateRevision,
    InvalidPackIndex,
    MissingPreviousRevision,
    RewrittenPreviousRevision,
    ChangedRevisionSet,
}

/// One immutable pack containing a contiguous slice of the sorted delta refs.
/// Real pack contents must separately match the whole pack hash, each revision
/// JSON hash, content blob hash and complete immutable revision DAG.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3DeltaPackRefV2 {
    pub pack_blake3: String,
    pub byte_length: u32,
    pub first_revision_index: u16,
    pub revision_count: u16,
}

/// Inert v2 sidecar; NOT a replacement for strict V1 refs/current.json.
/// All roots here must come from independently verified V1 snapshot evidence.
/// Pointer hashes cover canonical pointer JSON, never S3 ETags.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3DeltaEnvelopeV2 {
    schema: u16,
    vault_id: VaultId,
    generation: u64,
    previous_manifest_id: OperationId,
    previous_manifest_blake3: String,
    current_manifest_id: OperationId,
    current_manifest_blake3: String,
    previous_pointer_blake3: String,
    current_pointer_blake3: String,
    revisions: Vec<S3RevisionRefV1>,
    packs: Vec<S3DeltaPackRefV2>,
}

/// Advisory membership receipt. Pack existence, payload integrity, signer
/// authorization and current remote availability remain entirely unproven.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3DeltaEnvelopeCoverageV2 {
    pub generation: u64,
    pub added_revision_ids: Vec<RevisionId>,
    pub required_pack_count: usize,
    pub requires_verified_pack_payloads: bool,
    pub requires_full_apply_gate: bool,
}

impl S3DeltaEnvelopeV2 {
    pub fn new(
        ns: &S3Keyspace,
        previous: &S3HeadPointerV1,
        current: &S3HeadPointerV1,
        revisions: Vec<S3RevisionRefV1>,
        packs: Vec<S3DeltaPackRefV2>,
    ) -> Result<Self, S3DeltaEnvelopeV2Error> {
        let envelope = Self {
            schema: SCHEMA,
            vault_id: ns.vault_id(),
            generation: current.generation(),
            previous_manifest_id: previous.manifest_id(),
            previous_manifest_blake3: previous.manifest_blake3().to_owned(),
            current_manifest_id: current.manifest_id(),
            current_manifest_blake3: current.manifest_blake3().to_owned(),
            previous_pointer_blake3: pointer_hash(ns, previous)?,
            current_pointer_blake3: pointer_hash(ns, current)?,
            revisions,
            packs,
        };
        envelope.check(ns)?;
        envelope.check_pointer_lineage(ns, previous, current)?;
        envelope.encode(ns)?;
        Ok(envelope)
    }

    pub fn encode(&self, ns: &S3Keyspace)
        -> Result<Vec<u8>, S3DeltaEnvelopeV2Error>
    {
        self.check(ns)?;
        let bytes = serde_json::to_vec(self)
            .map_err(|_| S3DeltaEnvelopeV2Error::InvalidEncoding)?;
        if bytes.len() > MAX_DELTA_ENVELOPE_BYTES {
            return Err(S3DeltaEnvelopeV2Error::Oversized);
        }
        Ok(bytes)
    }

    pub fn decode(ns: &S3Keyspace, bytes: &[u8])
        -> Result<Self, S3DeltaEnvelopeV2Error>
    {
        if bytes.len() > MAX_DELTA_ENVELOPE_BYTES {
            return Err(S3DeltaEnvelopeV2Error::Oversized);
        }
        let envelope: Self = serde_json::from_slice(bytes)
            .map_err(|_| S3DeltaEnvelopeV2Error::InvalidEncoding)?;
        envelope.check(ns)?;
        if envelope.encode(ns)?.as_slice() != bytes {
            return Err(S3DeltaEnvelopeV2Error::InvalidEncoding);
        }
        Ok(envelope)
    }

    pub fn digest(&self, ns: &S3Keyspace)
        -> Result<String, S3DeltaEnvelopeV2Error>
    {
        Ok(blake3::hash(&self.encode(ns)?).to_hex().to_string())
    }

    pub fn generation(&self) -> u64 { self.generation }
    pub fn revisions(&self) -> &[S3RevisionRefV1] { &self.revisions }
    pub fn packs(&self) -> &[S3DeltaPackRefV2] { &self.packs }

    /// Public read-only path is restricted to full V1 snapshots produced by
    /// the canonical pinned-S3 verification pipeline. Caller-defined slices
    /// of revision IDs are NOT a remote assurance boundary. Neither digest
    /// nor snapshot label is a writer-authenticated MAC.
    #[cfg(feature = "sync-s3")]
    pub fn verify_pinned_v1_coverage(
        &self,
        ns: &S3Keyspace,
        previous_head: &S3HeadPointerV1,
        previous_manifest: &S3ManifestV1,
        previous: &S3PinnedRevisionSnapshot,
        current_head: &S3HeadPointerV1,
        current_manifest: &S3ManifestV1,
        current: &S3PinnedRevisionSnapshot,
    ) -> Result<S3DeltaEnvelopeCoverageV2, S3DeltaEnvelopeV2Error> {
        if previous.snapshot_id != format!(
            "{}:{}", previous_head.manifest_id(), previous_head.manifest_blake3(),
        ) || current.snapshot_id != format!(
            "{}:{}", current_head.manifest_id(), current_head.manifest_blake3(),
        ) {
            return Err(S3DeltaEnvelopeV2Error::UntrustedSnapshot);
        }
        self.verify_v1_coverage(
            ns, previous_head, previous_manifest,
            previous.references_for_coord_v2(),
            current_head, current_manifest,
            current.references_for_coord_v2(),
        )
    }

    /// Narrow crate-private advisory comparator for the SDK reader that
    /// independently reconstructs every V1 immutable shard inventory and
    /// verifies newly added revision JSON + blobs inside bounded V2 packs.
    /// This is NOT the public fully pinned assurance method above: older
    /// remote blob availability is reused from the durable prior anchor.
    /// No caller may turn this receipt into File Gate or baseline authority.
    #[cfg(feature = "sync-s3")]
    pub(crate) fn verify_shard_inventory_advisory(
        &self,
        ns: &S3Keyspace,
        previous_head: &S3HeadPointerV1,
        previous_manifest: &S3ManifestV1,
        previous_refs: &[S3RevisionRefV1],
        current_head: &S3HeadPointerV1,
        current_manifest: &S3ManifestV1,
        current_refs: &[S3RevisionRefV1],
    ) -> Result<S3DeltaEnvelopeCoverageV2, S3DeltaEnvelopeV2Error> {
        self.verify_v1_coverage(
            ns, previous_head, previous_manifest, previous_refs,
            current_head, current_manifest, current_refs,
        )
    }

    /// Private inventory comparison; callers require a separate canonical
    /// trust boundary (full pin or SDK-verified immutable shard chain).
    fn verify_v1_coverage(
        &self,
        ns: &S3Keyspace,
        previous_head: &S3HeadPointerV1,
        previous_manifest: &S3ManifestV1,
        previous_refs: &[S3RevisionRefV1],
        current_head: &S3HeadPointerV1,
        current_manifest: &S3ManifestV1,
        current_refs: &[S3RevisionRefV1],
    ) -> Result<S3DeltaEnvelopeCoverageV2, S3DeltaEnvelopeV2Error> {
        self.check(ns)?;
        self.check_pointer_lineage(ns, previous_head, current_head)?;
        if previous_manifest.generation != previous_head.generation()
            || current_manifest.generation != current_head.generation()
            || previous_manifest.manifest_id != previous_head.manifest_id()
            || current_manifest.manifest_id != current_head.manifest_id()
            || previous_manifest.digest(ns).map_err(|_| S3DeltaEnvelopeV2Error::InvalidAnchor)?
                != previous_head.manifest_blake3()
            || current_manifest.digest(ns).map_err(|_| S3DeltaEnvelopeV2Error::InvalidAnchor)?
                != current_head.manifest_blake3()
            || !current_manifest.is_successor_of(
                previous_manifest, previous_head.manifest_blake3(),
            )
        {
            return Err(S3DeltaEnvelopeV2Error::MissingAncestry);
        }

        checked_inventory(previous_refs, previous_manifest.total_revisions as usize)?;
        checked_inventory(current_refs, current_manifest.total_revisions as usize)?;
        let mut additions = Vec::new();
        let mut older = previous_refs.iter().peekable();
        for entry in current_refs {
            if older.peek().is_some_and(|prior| prior.revision_id < entry.revision_id) {
                return Err(S3DeltaEnvelopeV2Error::MissingPreviousRevision);
            }
            match older.peek() {
                Some(prior) if prior.revision_id == entry.revision_id => {
                    if *prior != entry {
                        return Err(S3DeltaEnvelopeV2Error::RewrittenPreviousRevision);
                    }
                    older.next();
                }
                _ => additions.push(entry.clone()),
            }
        }
        if older.next().is_some() {
            return Err(S3DeltaEnvelopeV2Error::MissingPreviousRevision);
        }
        if additions != self.revisions {
            return Err(S3DeltaEnvelopeV2Error::ChangedRevisionSet);
        }
        Ok(S3DeltaEnvelopeCoverageV2 {
            generation: self.generation,
            added_revision_ids: additions.iter().map(|r| r.revision_id).collect(),
            required_pack_count: self.packs.len(),
            requires_verified_pack_payloads: true,
            requires_full_apply_gate: true,
        })
    }

    fn check_pointer_lineage(
        &self, ns: &S3Keyspace,
        previous: &S3HeadPointerV1, current: &S3HeadPointerV1,
    ) -> Result<(), S3DeltaEnvelopeV2Error> {
        if !current.is_direct_successor_of(previous) {
            return Err(S3DeltaEnvelopeV2Error::MissingAncestry);
        }
        if self.generation != current.generation()
            || self.previous_manifest_id != previous.manifest_id()
            || self.previous_manifest_blake3 != previous.manifest_blake3()
            || self.current_manifest_id != current.manifest_id()
            || self.current_manifest_blake3 != current.manifest_blake3()
            || self.previous_pointer_blake3 != pointer_hash(ns, previous)?
            || self.current_pointer_blake3 != pointer_hash(ns, current)?
        {
            return Err(S3DeltaEnvelopeV2Error::InvalidAnchor);
        }
        Ok(())
    }

    fn check(&self, ns: &S3Keyspace) -> Result<(), S3DeltaEnvelopeV2Error> {
        if self.schema != SCHEMA {
            return Err(S3DeltaEnvelopeV2Error::UnsupportedSchema);
        }
        if self.vault_id != ns.vault_id() {
            return Err(S3DeltaEnvelopeV2Error::ForeignVault);
        }
        if self.generation < 2
            || self.previous_manifest_id == self.current_manifest_id
            || [&self.previous_manifest_blake3, &self.current_manifest_blake3,
                &self.previous_pointer_blake3, &self.current_pointer_blake3]
                .iter().any(|hash| !is_blake3(hash))
        {
            return Err(S3DeltaEnvelopeV2Error::InvalidAnchor);
        }
        if self.revisions.len() > MAX_DELTA_ENVELOPE_REVISIONS {
            return Err(S3DeltaEnvelopeV2Error::Oversized);
        }
        let mut prior = None;
        for r in &self.revisions {
            if !is_blake3(&r.revision_blake3) {
                return Err(S3DeltaEnvelopeV2Error::InvalidRevision);
            }
            if prior.is_some_and(|id| id >= r.revision_id) {
                return Err(S3DeltaEnvelopeV2Error::DuplicateRevision);
            }
            prior = Some(r.revision_id);
        }
        let mut offset = 0usize;
        let mut hashes = BTreeSet::new();
        for p in &self.packs {
            let count = p.revision_count as usize;
            if p.first_revision_index as usize != offset
                || count == 0 || count > MAX_DELTA_PACK_MEMBERS
                || !is_blake3(&p.pack_blake3)
                || !hashes.insert(&p.pack_blake3)
                || p.byte_length == 0 || p.byte_length > MAX_DELTA_PACK_BYTES
            {
                return Err(S3DeltaEnvelopeV2Error::InvalidPackIndex);
            }
            let end = offset.checked_add(count)
                .ok_or(S3DeltaEnvelopeV2Error::InvalidPackIndex)?;
            let rows = self.revisions.get(offset..end)
                .ok_or(S3DeltaEnvelopeV2Error::InvalidPackIndex)?;
            let mut min = MIN_PACK_HEADER + MIN_PACK_MEMBER_OVERHEAD * count as u64;
            for row in rows {
                min = min.checked_add(row.blob_bytes.unwrap_or(0))
                    .ok_or(S3DeltaEnvelopeV2Error::InvalidPackIndex)?;
            }
            if (p.byte_length as u64) <= min {
                return Err(S3DeltaEnvelopeV2Error::InvalidPackIndex);
            }
            offset = end;
        }
        if offset != self.revisions.len() {
            return Err(S3DeltaEnvelopeV2Error::InvalidPackIndex);
        }
        Ok(())
    }
}

fn pointer_hash(ns: &S3Keyspace, pointer: &S3HeadPointerV1)
    -> Result<String, S3DeltaEnvelopeV2Error>
{
    let bytes = pointer.encode(ns)
        .map_err(|_| S3DeltaEnvelopeV2Error::InvalidAnchor)?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn checked_inventory(refs: &[S3RevisionRefV1], expected: usize)
    -> Result<(), S3DeltaEnvelopeV2Error>
{
    if refs.len() != expected || refs.len() > MAX_REVISIONS {
        return Err(S3DeltaEnvelopeV2Error::ChangedRevisionSet);
    }
    let mut prior = None;
    for r in refs {
        if !is_blake3(&r.revision_blake3) {
            return Err(S3DeltaEnvelopeV2Error::InvalidRevision);
        }
        if prior.is_some_and(|id| id >= r.revision_id) {
            return Err(S3DeltaEnvelopeV2Error::DuplicateRevision);
        }
        prior = Some(r.revision_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{DocumentId, S3ManifestShardV1, S3ManifestShardRefV1};

    fn r(num: u128, doc: DocumentId, tombstone: bool) -> S3RevisionRefV1 {
        S3RevisionRefV1 {
            revision_id: RevisionId::from_uuid(uuid::Uuid::from_u128(num)),
            document_id: doc,
            revision_blake3: blake3::hash(&num.to_le_bytes()).to_hex().to_string(),
            blob_bytes: if tombstone { None } else { Some(24) },
        }
    }

    struct Fixture {
        ns: S3Keyspace,
        old_head: S3HeadPointerV1,
        old_root: S3ManifestV1,
        old: Vec<S3RevisionRefV1>,
        new_head: S3HeadPointerV1,
        new_root: S3ManifestV1,
        current: Vec<S3RevisionRefV1>,
        envelope: S3DeltaEnvelopeV2,
    }

    fn root(
        ns: &S3Keyspace, generation: u64,
        previous: Option<(OperationId,String)>, refs: &[S3RevisionRefV1],
    ) -> S3ManifestV1 {
        let shards: Vec<S3ManifestShardRefV1> = if refs.is_empty() {
            Vec::new()
        } else {
            let shard = S3ManifestShardV1::new(ns, OperationId::new(), refs.to_vec()).unwrap();
            vec![shard.reference(ns).unwrap()]
        };
        S3ManifestV1::new(ns, OperationId::new(), generation, previous, shards).unwrap()
    }

    fn fixture() -> Fixture {
        let ns = S3Keyspace::new("akula/coord-v2-unit", VaultId::new()).unwrap();
        let doc = DocumentId::new();
        let other = DocumentId::new();
        let base = r(11, doc, false);
        let sibling = r(12, doc, false);
        let tombstone = r(13, doc, true);
        let other_content = r(14, other, false);
        let old = vec![base.clone()];
        let current = vec![base, sibling.clone(), tombstone.clone(), other_content.clone()];
        let old_root = root(&ns, 1, None, &old);
        let old_digest = old_root.digest(&ns).unwrap();
        let old_head = S3HeadPointerV1::new(
            &ns, 1, old_root.manifest_id, &old_digest, None,
        ).unwrap();
        let new_root = root(
            &ns, 2, Some((old_root.manifest_id, old_digest)), &current,
        );
        let new_head = S3HeadPointerV1::new(
            &ns, 2, new_root.manifest_id, &new_root.digest(&ns).unwrap(),
            Some(old_root.manifest_id),
        ).unwrap();
        let pack = S3DeltaPackRefV2 {
            pack_blake3: blake3::hash(b"experimental pack").to_hex().to_string(),
            byte_length: 1_000, first_revision_index: 0, revision_count: 3,
        };
        let envelope = S3DeltaEnvelopeV2::new(
            &ns, &old_head, &new_head,
            vec![sibling, tombstone, other_content], vec![pack],
        ).unwrap();
        Fixture { ns, old_head, old_root, old, new_head, new_root, current, envelope }
    }

    fn verify(f: &Fixture, envelope: &S3DeltaEnvelopeV2)
        -> Result<S3DeltaEnvelopeCoverageV2, S3DeltaEnvelopeV2Error>
    {
        envelope.verify_v1_coverage(
            &f.ns, &f.old_head, &f.old_root, &f.old,
            &f.new_head, &f.new_root, &f.current,
        )
    }

    #[test]
    fn canonical_roundtrip_binds_both_v1_pointers_and_preserves_siblings_tombstone() {
        let f = fixture();
        let bytes = f.envelope.encode(&f.ns).unwrap();
        assert_eq!(S3DeltaEnvelopeV2::decode(&f.ns, &bytes).unwrap(), f.envelope);
        assert_eq!(f.envelope.digest(&f.ns).unwrap(),
            blake3::hash(&bytes).to_hex().to_string());
        let receipt = verify(&f, &f.envelope).unwrap();
        assert_eq!(receipt.generation, 2);
        assert_eq!(receipt.added_revision_ids.len(), 3);
        assert_eq!(receipt.required_pack_count, 1);
        assert!(receipt.requires_verified_pack_payloads);
        assert!(receipt.requires_full_apply_gate);
    }

    #[test]
    fn removed_tombstone_or_sibling_and_rewritten_parent_are_never_accepted() {
        let mut f = fixture();
        f.current.remove(2); // Tombstone vanished.
        assert!(verify(&f, &f.envelope).is_err());
        let mut f = fixture();
        f.current[0].revision_blake3 =
            blake3::hash(b"rewritten-old-revision").to_hex().to_string();
        assert!(verify(&f, &f.envelope).is_err());
        let mut f = fixture();
        f.current.remove(1); // One offline sibling vanished.
        assert!(verify(&f, &f.envelope).is_err());
    }

    #[test]
    fn changed_letter_or_duplicate_revision_ref_is_rejected() {
        let f = fixture();
        let mut fake = f.envelope.clone();
        fake.revisions[0].revision_blake3 =
            blake3::hash(b"rewritten-new-revision").to_hex().to_string();
        assert_eq!(verify(&f, &fake),
            Err(S3DeltaEnvelopeV2Error::ChangedRevisionSet));
        let mut dupe = f.envelope.clone();
        dupe.revisions[1].revision_id = dupe.revisions[0].revision_id;
        assert_eq!(dupe.encode(&f.ns),
            Err(S3DeltaEnvelopeV2Error::DuplicateRevision));
    }

    #[test]
    fn tampered_pack_offsets_lengths_and_duplicate_digest_fail_closed() {
        let f = fixture();
        let mut forged = f.envelope.clone();
        forged.packs[0].first_revision_index = 1;
        assert_eq!(forged.encode(&f.ns),
            Err(S3DeltaEnvelopeV2Error::InvalidPackIndex));
        let mut forged = f.envelope.clone();
        forged.packs[0].byte_length = 25;
        assert_eq!(forged.encode(&f.ns),
            Err(S3DeltaEnvelopeV2Error::InvalidPackIndex));
        let mut forged = f.envelope.clone();
        let digest = forged.packs[0].pack_blake3.clone();
        forged.packs = vec![
            S3DeltaPackRefV2 { pack_blake3: digest.clone(),
                byte_length: 1_000, first_revision_index: 0, revision_count: 1 },
            S3DeltaPackRefV2 { pack_blake3: digest,
                byte_length: 1_000, first_revision_index: 1, revision_count: 2 },
        ];
        assert_eq!(forged.encode(&f.ns),
            Err(S3DeltaEnvelopeV2Error::InvalidPackIndex));
    }

    #[test]
    fn foreign_vault_forward_schema_noncanonical_encoding_fail_closed() {
        let f = fixture();
        let bytes = f.envelope.encode(&f.ns).unwrap();
        let foreign = S3Keyspace::new("akula/coord-v2-unit", VaultId::new()).unwrap();
        assert_eq!(S3DeltaEnvelopeV2::decode(&foreign, &bytes),
            Err(S3DeltaEnvelopeV2Error::ForeignVault));
        let mut ahead = f.envelope.clone();
        ahead.schema = 3;
        assert_eq!(ahead.encode(&f.ns),
            Err(S3DeltaEnvelopeV2Error::UnsupportedSchema));
        let mut extra = bytes.clone();
        extra.pop();
        extra.extend_from_slice(b",\"injected\":true}");
        assert_eq!(S3DeltaEnvelopeV2::decode(&f.ns, &extra),
            Err(S3DeltaEnvelopeV2Error::InvalidEncoding));
        let mut spaced = bytes.clone();
        spaced.push(b'\n');
        assert_eq!(S3DeltaEnvelopeV2::decode(&f.ns, &spaced),
            Err(S3DeltaEnvelopeV2Error::InvalidEncoding));
    }

    #[test]
    fn rollback_wrong_root_or_unverified_successor_fail_closed() {
        let f = fixture();
        assert_eq!(f.envelope.verify_v1_coverage(
            &f.ns, &f.new_head, &f.new_root, &f.current,
            &f.old_head, &f.old_root, &f.old,
        ), Err(S3DeltaEnvelopeV2Error::MissingAncestry));
        let mut bogus = f.envelope.clone();
        bogus.current_manifest_blake3 = blake3::hash(b"foreign").to_hex().to_string();
        assert_eq!(verify(&f, &bogus), Err(S3DeltaEnvelopeV2Error::InvalidAnchor));
        let mut bogus = f.envelope.clone();
        bogus.previous_pointer_blake3 = blake3::hash(b"tamper").to_hex().to_string();
        assert_eq!(verify(&f, &bogus), Err(S3DeltaEnvelopeV2Error::InvalidAnchor));
    }

    #[cfg(feature = "sync-s3")]
    #[test]
    fn pinned_v1_snapshots_are_required_and_stale_snapshot_ids_fail_closed() {
        let f = fixture();
        let previous = S3PinnedRevisionSnapshot::for_coord_v2_test(
            format!("{}:{}", f.old_head.manifest_id(), f.old_head.manifest_blake3()),
            f.old.clone(),
        );
        let current = S3PinnedRevisionSnapshot::for_coord_v2_test(
            format!("{}:{}", f.new_head.manifest_id(), f.new_head.manifest_blake3()),
            f.current.clone(),
        );
        let receipt = f.envelope.verify_pinned_v1_coverage(
            &f.ns, &f.old_head, &f.old_root, &previous,
            &f.new_head, &f.new_root, &current,
        ).unwrap();
        assert_eq!(receipt.added_revision_ids.len(), 3);
        let forged = S3PinnedRevisionSnapshot::for_coord_v2_test(
            "foreign-snapshot:sha".into(), f.current.clone(),
        );
        assert_eq!(f.envelope.verify_pinned_v1_coverage(
            &f.ns, &f.old_head, &f.old_root, &previous,
            &f.new_head, &f.new_root, &forged,
        ), Err(S3DeltaEnvelopeV2Error::UntrustedSnapshot));
    }

    #[test]
    fn zero_changes_no_pack_but_still_requires_real_successor_manifest() {
        let ns = S3Keyspace::new("akula/coord-v2-unchanged", VaultId::new()).unwrap();
        let old_root = root(&ns, 1, None, &[]);
        let old_head = S3HeadPointerV1::new(
            &ns, 1, old_root.manifest_id, &old_root.digest(&ns).unwrap(), None,
        ).unwrap();
        let new_root = root(&ns, 2,
            Some((old_root.manifest_id, old_head.manifest_blake3().to_owned())), &[],
        );
        let new_head = S3HeadPointerV1::new(
            &ns, 2, new_root.manifest_id, &new_root.digest(&ns).unwrap(),
            Some(old_root.manifest_id),
        ).unwrap();
        let envelope = S3DeltaEnvelopeV2::new(
            &ns, &old_head, &new_head, Vec::new(), Vec::new(),
        ).unwrap();
        let result = envelope.verify_v1_coverage(
            &ns, &old_head, &old_root, &[], &new_head, &new_root, &[],
        ).unwrap();
        assert!(result.added_revision_ids.is_empty());
        assert!(envelope.packs().is_empty());
    }
}
