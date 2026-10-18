//! SYNC-05d2g: inert V2 immutable packed-index contract.
//!
//! This module performs NO cloud I/O, File Gate I/O, CAS publication, mutation,
//! user-facing synchronization or baseline advancement. In particular, the
//! existing V1 root reader must reject the V2 root schema instead of treating
//! a pack as an ordinary S3 blob or inferring an empty/deleted inventory.
//!
//! Trust chain, once a future S3 CAS reader authenticates the root bytes:
//!   guarded V2 root -> BLAKE3 index -> BLAKE3 index shards
//!       -> BLAKE3 immutable pack -> exact revision/blob member frames.
//!
//! A parsed index alone is NOT an authentication result. The verifier below
//! accepts only independently verified current-manifest revision references;
//! it does not download remote objects or trust a raw ListObjectsV2 listing.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    OperationId, RevisionId, S3Keyspace, S3ManifestV1,
    S3RevisionRefV1, VaultId,
};
use super::s3_manifest::is_blake3;

const SCHEMA_V2: u16 = 2;
pub const MAX_PACK_V2_ROOT_BYTES: usize = 4096;
pub const MAX_PACK_V2_INDEX_BYTES: usize = 128 * 1024;
pub const MAX_PACK_V2_SHARD_BYTES: usize = 128 * 1024;
pub const MAX_PACK_V2_SHARDS: usize = 256;
pub const MAX_PACK_V2_PACKS_PER_SHARD: usize = 2;
pub const MAX_PACK_V2_MEMBERS_PER_PACK: usize = 128;
pub const MAX_PACK_V2_BYTES: u32 = 512 * 1024;
const MAX_PACK_REVISION_BYTES: u32 = 64 * 1024;
const MAX_PACK_MEMBER_BLOB_BYTES: u32 = 32 * 1024;
const MAX_TOTAL_REVISIONS: usize = 32_768;
const FRAME_START: u32 = 10; // AKPKEXP1 + LE u16 member count.
const FRAME_HEADER: u32 = 8; // LE u32 revision length + LE u32 blob length.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum S3PackIndexV2Error {
    UnsupportedSchema,
    ForeignVault,
    InvalidRoot,
    InvalidIndex,
    InvalidShard,
    InvalidPack,
    InvalidMember,
    InvalidAncestry,
    Oversized,
    IntegrityMismatch,
    IncompleteCoverage,
}

/// Proposed V2 CAS root. Its digest is meaningful ONLY after a future
/// provider-enforced If-Match/If-None-Match read/publish protocol authenticates
/// the bytes at the exact `refs/current.json` key. No such writer is enabled.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3PackedHeadV2 {
    schema: u16,
    vault_id: VaultId,
    generation: u64,
    manifest_id: OperationId,
    manifest_blake3: String,
    index_id: OperationId,
    index_blake3: String,
    previous_root_blake3: Option<String>,
}

impl S3PackedHeadV2 {
    pub fn new(
        ns: &S3Keyspace,
        generation: u64,
        manifest_id: OperationId,
        manifest_blake3: &str,
        index_id: OperationId,
        index_blake3: &str,
        previous_root_blake3: Option<String>,
    ) -> Result<Self, S3PackIndexV2Error> {
        let value = Self {
            schema: SCHEMA_V2,
            vault_id: ns.vault_id(),
            generation,
            manifest_id,
            manifest_blake3: manifest_blake3.to_owned(),
            index_id,
            index_blake3: index_blake3.to_owned(),
            previous_root_blake3,
        };
        value.check(ns)?;
        Ok(value)
    }

    pub fn encode(&self, ns: &S3Keyspace) -> Result<Vec<u8>, S3PackIndexV2Error> {
        self.check(ns)?;
        bounded_json(self, MAX_PACK_V2_ROOT_BYTES, S3PackIndexV2Error::InvalidRoot)
    }

    pub fn decode(ns: &S3Keyspace, bytes: &[u8])
        -> Result<Self, S3PackIndexV2Error>
    {
        if bytes.len() > MAX_PACK_V2_ROOT_BYTES {
            return Err(S3PackIndexV2Error::Oversized);
        }
        let value: Self = serde_json::from_slice(bytes)
            .map_err(|_| S3PackIndexV2Error::InvalidRoot)?;
        value.check(ns)?;
        Ok(value)
    }

    pub fn digest(&self, ns: &S3Keyspace) -> Result<String, S3PackIndexV2Error> {
        Ok(blake3_hex(&self.encode(ns)?))
    }

    pub fn generation(&self) -> u64 { self.generation }

    /// A protocol proposal for later CAS+journal integration. The old root
    /// is authenticated by caller, not by this in-memory predicate.
    pub fn is_direct_successor_of(
        &self, ns: &S3Keyspace, previous: &Self,
    ) -> bool {
        self.check(ns).is_ok()
            && previous.check(ns).is_ok()
            && self.vault_id == previous.vault_id
            && self.generation == previous.generation.saturating_add(1)
            && self.generation > previous.generation
            && self.manifest_id != previous.manifest_id
            && self.previous_root_blake3.as_deref()
                == previous.digest(ns).ok().as_deref()
    }

    fn check(&self, ns: &S3Keyspace) -> Result<(), S3PackIndexV2Error> {
        if self.schema != SCHEMA_V2 {
            return Err(S3PackIndexV2Error::UnsupportedSchema);
        }
        if self.vault_id != ns.vault_id() {
            return Err(S3PackIndexV2Error::ForeignVault);
        }
        if self.generation == 0
            || (self.generation == 1) != self.previous_root_blake3.is_none()
            || self.previous_root_blake3.as_deref().is_some_and(|h| !is_blake3(h))
            || !is_blake3(&self.manifest_blake3)
            || !is_blake3(&self.index_blake3)
            || self.index_id == self.manifest_id
        {
            return Err(S3PackIndexV2Error::InvalidRoot);
        }
        Ok(())
    }
}

/// Exact reference identity + verified frame offsets. A tombstone has no
/// content and `blob_offset = None`, but retains its revision JSON frame.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3PackMemberV2 {
    pub reference: S3RevisionRefV1,
    pub revision_offset: u32,
    pub revision_len: u32,
    pub blob_offset: Option<u32>,
}

/// Immutable content-addressed pack metadata; the future reader derives
/// its key from `ns.immutable_blob_key(pack_blake3)` and must independently
/// hash the fetched bytes and every internal member, not trust S3 ETags.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3PackDescriptorV2 {
    pub pack_blake3: String,
    pub pack_bytes: u32,
    pub members: Vec<S3PackMemberV2>,
}

impl S3PackDescriptorV2 {
    /// Derive canonical frame offsets for the existing AKPKEXP1 test format.
    /// Data bytes and their hashes must still be authenticated on download.
    pub fn from_lengths(
        pack_blake3: String,
        members: Vec<(S3RevisionRefV1, u32)>,
    ) -> Result<Self, S3PackIndexV2Error> {
        if members.is_empty() || members.len() > MAX_PACK_V2_MEMBERS_PER_PACK {
            return Err(S3PackIndexV2Error::InvalidPack);
        }
        let mut cursor = FRAME_START;
        let mut frames = Vec::with_capacity(members.len());
        for (reference, revision_len) in members {
            let revision_offset = cursor.checked_add(FRAME_HEADER)
                .ok_or(S3PackIndexV2Error::Oversized)?;
            let blob_offset_value = revision_offset.checked_add(revision_len)
                .ok_or(S3PackIndexV2Error::Oversized)?;
            let blob_bytes = reference.blob_bytes.unwrap_or(0);
            let blob_offset = reference.blob_bytes.map(|_| blob_offset_value);
            cursor = blob_offset_value.checked_add(
                u32::try_from(blob_bytes).map_err(|_| S3PackIndexV2Error::Oversized)?
            ).ok_or(S3PackIndexV2Error::Oversized)?;
            frames.push(S3PackMemberV2 {
                reference, revision_offset, revision_len, blob_offset,
            });
        }
        let desc = Self { pack_blake3, pack_bytes: cursor, members: frames };
        desc.check()?;
        Ok(desc)
    }

    fn check(&self) -> Result<(), S3PackIndexV2Error> {
        if !is_blake3(&self.pack_blake3)
            || self.pack_bytes > MAX_PACK_V2_BYTES
            || self.members.is_empty()
            || self.members.len() > MAX_PACK_V2_MEMBERS_PER_PACK
        {
            return Err(S3PackIndexV2Error::InvalidPack);
        }
        let mut cursor = FRAME_START;
        let mut seen = BTreeSet::new();
        for member in &self.members {
            let frame_start = cursor.checked_add(FRAME_HEADER)
                .ok_or(S3PackIndexV2Error::InvalidMember)?;
            if member.revision_len == 0
                || member.revision_len > MAX_PACK_REVISION_BYTES
                || member.revision_offset != frame_start
                || !is_blake3(&member.reference.revision_blake3)
                || !seen.insert(member.reference.revision_id)
            {
                return Err(S3PackIndexV2Error::InvalidMember);
            }
            let next = frame_start.checked_add(member.revision_len)
                .ok_or(S3PackIndexV2Error::InvalidMember)?;
            if member.reference.blob_bytes.unwrap_or(0)
                > u64::from(MAX_PACK_MEMBER_BLOB_BYTES)
                || member.blob_offset != member.reference.blob_bytes.map(|_| next)
            {
                return Err(S3PackIndexV2Error::InvalidMember);
            }
            cursor = next.checked_add(
                u32::try_from(member.reference.blob_bytes.unwrap_or(0))
                    .map_err(|_| S3PackIndexV2Error::InvalidMember)?
            ).ok_or(S3PackIndexV2Error::InvalidMember)?;
            if cursor > MAX_PACK_V2_BYTES {
                return Err(S3PackIndexV2Error::Oversized);
            }
        }
        if cursor != self.pack_bytes {
            return Err(S3PackIndexV2Error::InvalidPack);
        }
        Ok(())
    }
}

/// One independently hashed <=128 KiB shard. Keep each shard small enough to
/// decode with a strict cap, unlike an unbounded global 10k-entry JSON array.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3PackIndexShardV2 {
    schema: u16,
    vault_id: VaultId,
    pub shard_id: OperationId,
    pub packs: Vec<S3PackDescriptorV2>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3PackShardRefV2 {
    pub shard_id: OperationId,
    pub shard_blake3: String,
    pub pack_count: u16,
    pub revision_count: u16,
}

impl S3PackIndexShardV2 {
    pub fn new(
        ns: &S3Keyspace, shard_id: OperationId,
        packs: Vec<S3PackDescriptorV2>,
    ) -> Result<Self, S3PackIndexV2Error> {
        let shard = Self { schema: SCHEMA_V2, vault_id: ns.vault_id(), shard_id, packs };
        shard.check(ns)?;
        Ok(shard)
    }

    pub fn encode(&self, ns: &S3Keyspace) -> Result<Vec<u8>, S3PackIndexV2Error> {
        self.check(ns)?;
        bounded_json(self, MAX_PACK_V2_SHARD_BYTES, S3PackIndexV2Error::InvalidShard)
    }

    pub fn decode(
        ns: &S3Keyspace, expected_id: OperationId, bytes: &[u8],
    ) -> Result<Self, S3PackIndexV2Error> {
        if bytes.len() > MAX_PACK_V2_SHARD_BYTES {
            return Err(S3PackIndexV2Error::Oversized);
        }
        let shard: Self = serde_json::from_slice(bytes)
            .map_err(|_| S3PackIndexV2Error::InvalidShard)?;
        shard.check(ns)?;
        if shard.shard_id != expected_id {
            return Err(S3PackIndexV2Error::InvalidShard);
        }
        Ok(shard)
    }

    pub fn reference(&self, ns: &S3Keyspace)
        -> Result<S3PackShardRefV2, S3PackIndexV2Error>
    {
        let bytes = self.encode(ns)?;
        Ok(S3PackShardRefV2 {
            shard_id: self.shard_id,
            shard_blake3: blake3_hex(&bytes),
            pack_count: self.packs.len() as u16,
            revision_count: self.packs.iter().map(|p| p.members.len() as u16).sum(),
        })
    }

    fn check(&self, ns: &S3Keyspace) -> Result<(), S3PackIndexV2Error> {
        if self.schema != SCHEMA_V2 {
            return Err(S3PackIndexV2Error::UnsupportedSchema);
        }
        if self.vault_id != ns.vault_id() {
            return Err(S3PackIndexV2Error::ForeignVault);
        }
        if self.packs.is_empty() || self.packs.len() > MAX_PACK_V2_PACKS_PER_SHARD {
            return Err(S3PackIndexV2Error::InvalidShard);
        }
        let mut pack_hashes = BTreeSet::new();
        let mut revisions = BTreeSet::new();
        for pack in &self.packs {
            pack.check()?;
            if !pack_hashes.insert(&pack.pack_blake3) {
                return Err(S3PackIndexV2Error::InvalidPack);
            }
            for member in &pack.members {
                if !revisions.insert(member.reference.revision_id) {
                    return Err(S3PackIndexV2Error::InvalidMember);
                }
            }
        }
        Ok(())
    }
}

/// Root of immutable index shards, pinned by exact digest in the separate
/// V2 root CAS pointer. The legacy V1 parser MUST NOT accept these bytes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3PackIndexRootV2 {
    schema: u16,
    vault_id: VaultId,
    pub index_id: OperationId,
    pub generation: u64,
    pub manifest_id: OperationId,
    pub manifest_blake3: String,
    pub total_revisions: u32,
    pub total_packs: u16,
    pub shards: Vec<S3PackShardRefV2>,
}

impl S3PackIndexRootV2 {
    pub fn new(
        ns: &S3Keyspace, index_id: OperationId, generation: u64,
        manifest_id: OperationId, manifest_blake3: String,
        shards: Vec<S3PackShardRefV2>,
    ) -> Result<Self, S3PackIndexV2Error> {
        let total_revisions: u32 = shards.iter()
            .map(|r| u32::from(r.revision_count)).sum();
        let total_packs: u16 = shards.iter().map(|r| r.pack_count).sum();
        let index = Self {
            schema: SCHEMA_V2, vault_id: ns.vault_id(), index_id, generation,
            manifest_id, manifest_blake3, total_revisions, total_packs, shards,
        };
        index.check(ns)?;
        Ok(index)
    }

    pub fn encode(&self, ns: &S3Keyspace) -> Result<Vec<u8>, S3PackIndexV2Error> {
        self.check(ns)?;
        bounded_json(self, MAX_PACK_V2_INDEX_BYTES, S3PackIndexV2Error::InvalidIndex)
    }

    pub fn decode(
        ns: &S3Keyspace, expected_id: OperationId, bytes: &[u8],
    ) -> Result<Self, S3PackIndexV2Error> {
        if bytes.len() > MAX_PACK_V2_INDEX_BYTES {
            return Err(S3PackIndexV2Error::Oversized);
        }
        let index: Self = serde_json::from_slice(bytes)
            .map_err(|_| S3PackIndexV2Error::InvalidIndex)?;
        index.check(ns)?;
        if index.index_id != expected_id {
            return Err(S3PackIndexV2Error::InvalidIndex);
        }
        Ok(index)
    }

    pub fn digest(&self, ns: &S3Keyspace) -> Result<String, S3PackIndexV2Error> {
        Ok(blake3_hex(&self.encode(ns)?))
    }

    /// Verify exact, complete membership under the supplied V2 pointer.
    ///
    /// The CALLER must have authenticated `head` using S3's atomic root
    /// read/CAS protocol AND independently verified `manifest` plus ALL
    /// `trusted_revisions`. Nothing here performs S3 I/O or grants authority
    /// to fetch/delete a local note. Index bytes do not prove own provenance.
    pub fn verify_complete(
        &self,
        ns: &S3Keyspace,
        head: &S3PackedHeadV2,
        manifest: &S3ManifestV1,
        trusted_revisions: &[S3RevisionRefV1],
        shard_bytes: &[(OperationId, Vec<u8>)],
    ) -> Result<(), S3PackIndexV2Error> {
        self.check(ns)?;
        head.check(ns)?;
        if head.index_id != self.index_id
            || head.index_blake3 != self.digest(ns)?
            || head.manifest_id != self.manifest_id
            || head.manifest_blake3 != self.manifest_blake3
            || head.generation != self.generation
            || manifest.manifest_id != self.manifest_id
            || manifest.generation != self.generation
            || manifest.digest(ns).map_err(|_| S3PackIndexV2Error::InvalidIndex)?
                != self.manifest_blake3
        {
            return Err(S3PackIndexV2Error::IntegrityMismatch);
        }
        let mut reference_map = BTreeMap::new();
        for record in trusted_revisions {
            if reference_map.insert(record.revision_id, record.clone()).is_some() {
                return Err(S3PackIndexV2Error::IncompleteCoverage);
            }
        }
        if reference_map.len() != self.total_revisions as usize
            || reference_map.len() != manifest.total_revisions as usize
            || shard_bytes.len() != self.shards.len()
        {
            return Err(S3PackIndexV2Error::IncompleteCoverage);
        }
        let mut shard_map = BTreeMap::new();
        for (id, bytes) in shard_bytes {
            if shard_map.insert(*id, bytes.as_slice()).is_some() {
                return Err(S3PackIndexV2Error::InvalidShard);
            }
        }
        let mut found = BTreeMap::new();
        let mut pack_hashes = BTreeSet::new();
        for shard_ref in &self.shards {
            let bytes = shard_map.get(&shard_ref.shard_id)
                .ok_or(S3PackIndexV2Error::IncompleteCoverage)?;
            if blake3_hex(bytes) != shard_ref.shard_blake3 {
                return Err(S3PackIndexV2Error::IntegrityMismatch);
            }
            let shard = S3PackIndexShardV2::decode(ns, shard_ref.shard_id, bytes)?;
            let verified_ref = shard.reference(ns)?;
            if verified_ref != *shard_ref {
                return Err(S3PackIndexV2Error::InvalidShard);
            }
            for pack in &shard.packs {
                if !pack_hashes.insert(pack.pack_blake3.clone()) {
                    return Err(S3PackIndexV2Error::InvalidPack);
                }
                for member in &pack.members {
                    if found.insert(
                        member.reference.revision_id,
                        member.reference.clone(),
                    ).is_some() {
                        return Err(S3PackIndexV2Error::InvalidMember);
                    }
                }
            }
        }
        if found != reference_map {
            return Err(S3PackIndexV2Error::IncompleteCoverage);
        }
        Ok(())
    }

    fn check(&self, ns: &S3Keyspace) -> Result<(), S3PackIndexV2Error> {
        if self.schema != SCHEMA_V2 {
            return Err(S3PackIndexV2Error::UnsupportedSchema);
        }
        if self.vault_id != ns.vault_id() {
            return Err(S3PackIndexV2Error::ForeignVault);
        }
        if self.generation == 0
            || self.index_id == self.manifest_id
            || !is_blake3(&self.manifest_blake3)
            || self.shards.is_empty()
            || self.shards.len() > MAX_PACK_V2_SHARDS
            || self.total_revisions == 0
            || self.total_revisions as usize > MAX_TOTAL_REVISIONS
        {
            return Err(S3PackIndexV2Error::InvalidIndex);
        }
        let mut ids = BTreeSet::new();
        let mut total_revisions = 0_u32;
        let mut total_packs = 0_u32;
        for shard in &self.shards {
            if shard.shard_id == self.index_id
                || shard.shard_id == self.manifest_id
                || !ids.insert(shard.shard_id)
                || !is_blake3(&shard.shard_blake3)
                || shard.pack_count == 0
                || shard.pack_count as usize > MAX_PACK_V2_PACKS_PER_SHARD
                || shard.revision_count == 0
                || shard.revision_count as usize >
                    MAX_PACK_V2_PACKS_PER_SHARD * MAX_PACK_V2_MEMBERS_PER_PACK
            {
                return Err(S3PackIndexV2Error::InvalidShard);
            }
            total_packs += u32::from(shard.pack_count);
            total_revisions += u32::from(shard.revision_count);
        }
        if total_revisions != self.total_revisions
            || total_packs != u32::from(self.total_packs)
        {
            return Err(S3PackIndexV2Error::InvalidIndex);
        }
        Ok(())
    }
}

fn bounded_json<T: Serialize>(
    value: &T, max_bytes: usize, invalid: S3PackIndexV2Error,
) -> Result<Vec<u8>, S3PackIndexV2Error> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid)?;
    if bytes.len() > max_bytes {
        return Err(S3PackIndexV2Error::Oversized);
    }
    Ok(bytes)
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}


#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{
        DocumentId, RevisionOperation, RevisionRecord, S3ContractError,
        S3HeadPointerV1, S3ManifestShardV1, S3ManifestV1, SyncRelativePath,
    };

    struct Fixture {
        ns: S3Keyspace,
        manifest: S3ManifestV1,
        references: Vec<S3RevisionRefV1>,
        index: S3PackIndexRootV2,
        head: S3PackedHeadV2,
        shards: Vec<(OperationId, Vec<u8>)>,
        pack_bytes: Vec<u8>,
    }

    fn fixture() -> Fixture {
        let ns = S3Keyspace::new("akula-sync/pack-v2-contract", VaultId::new()).unwrap();
        let document = DocumentId::new();
        let original = RevisionId::new();
        let left = RevisionId::new();
        let right = RevisionId::new();
        let deletion = RevisionId::new();
        let mut members = Vec::new();
        let mut raw_pack = b"AKPKEXP1".to_vec();
        raw_pack.extend_from_slice(&4_u16.to_le_bytes());
        for (revision, parents, payload) in [
            (original, vec![], Some(&b"original"[..])),
            (left, vec![original], Some(&b"offline left"[..])),
            (right, vec![original], Some(&b"offline right"[..])),
            (deletion, vec![original], None),
        ] {
            let record = RevisionRecord {
                id: revision,
                document_id: document,
                parents,
                operation: match payload {
                    Some(bytes) => RevisionOperation::Content {
                        path: SyncRelativePath::parse("notes/test.md").unwrap(),
                        blob_hash: blake3_hex(bytes),
                    },
                    None => RevisionOperation::Tombstone {
                        path: SyncRelativePath::parse("notes/test.md").unwrap(),
                    },
                },
            };
            let json = serde_json::to_vec(&record).unwrap();
            let blob = payload.unwrap_or_default();
            raw_pack.extend_from_slice(&(json.len() as u32).to_le_bytes());
            raw_pack.extend_from_slice(&(blob.len() as u32).to_le_bytes());
            raw_pack.extend_from_slice(&json);
            raw_pack.extend_from_slice(blob);
            members.push((
                S3RevisionRefV1 {
                    revision_id: revision,
                    document_id: document,
                    revision_blake3: blake3_hex(&json),
                    blob_bytes: payload.map(|b| b.len() as u64),
                },
                json.len() as u32,
            ));
        }
        let pack = S3PackDescriptorV2::from_lengths(
            blake3_hex(&raw_pack), members.clone(),
        ).unwrap();
        assert_eq!(pack.pack_bytes as usize, raw_pack.len());
        let shard = S3PackIndexShardV2::new(
            &ns, OperationId::new(), vec![pack],
        ).unwrap();
        let refs = {
            let mut refs: Vec<_> = members.into_iter().map(|(r, _)| r).collect();
            refs.sort_by_key(|reference| reference.revision_id);
            refs
        };
        let manifest_shard = S3ManifestShardV1::new(
            &ns, OperationId::new(), refs.clone(),
        ).unwrap();
        let manifest = S3ManifestV1::new(
            &ns, OperationId::new(), 1, None,
            vec![manifest_shard.reference(&ns).unwrap()],
        ).unwrap();
        let digest = manifest.digest(&ns).unwrap();
        let index = S3PackIndexRootV2::new(
            &ns, OperationId::new(), 1, manifest.manifest_id,
            digest.clone(), vec![shard.reference(&ns).unwrap()],
        ).unwrap();
        let head = S3PackedHeadV2::new(
            &ns, 1, manifest.manifest_id, &digest,
            index.index_id, &index.digest(&ns).unwrap(), None,
        ).unwrap();
        let shard_bytes = shard.encode(&ns).unwrap();
        Fixture {
            ns, manifest, references: refs, index, head,
            shards: vec![(shard.shard_id, shard_bytes)],
            pack_bytes: raw_pack,
        }
    }

    #[test]
    fn v2_root_index_shard_and_full_sibling_coverage_are_canonical() {
        let sample = fixture();
        let ns = &sample.ns;
        let root_bytes = sample.head.encode(ns).unwrap();
        let index_bytes = sample.index.encode(ns).unwrap();
        assert_eq!(S3PackedHeadV2::decode(ns, &root_bytes), Ok(sample.head.clone()));
        assert_eq!(S3PackIndexRootV2::decode(ns, sample.index.index_id, &index_bytes),
            Ok(sample.index.clone()));
        assert!(sample.index.verify_complete(ns, &sample.head, &sample.manifest,
            &sample.references, &sample.shards).is_ok());
        assert_eq!(sample.index.total_revisions, 4);
        assert_eq!(sample.index.total_packs, 1);
        assert!(sample.pack_bytes.len() < MAX_PACK_V2_BYTES as usize);
        let content_index = S3PackIndexShardV2::decode(
            ns, sample.shards[0].0, &sample.shards[0].1,
        ).unwrap();
        assert_eq!(content_index.packs[0].members.len(), 4);
        let tombstone = content_index.packs[0].members.iter()
            .find(|m| m.reference.blob_bytes.is_none()).unwrap();
        assert_eq!(tombstone.blob_offset, None);
        assert!(!content_index.packs[0].members.iter().any(|m|
            m.reference.revision_blake3.is_empty()));

        // Legacy readers cannot silently reinterpret an unknown V2 pointer
        // as an ordinary V1 root or turn its contents into a deletion signal.
        assert_eq!(
            S3HeadPointerV1::decode(&root_bytes, ns),
            Err(S3ContractError::InvalidRoot),
        );
    }

    #[test]
    fn foreign_vault_unknown_schema_or_fields_and_oversize_fail_closed() {
        let f = fixture();
        let other = S3Keyspace::new("akula-sync/pack-v2-contract", VaultId::new()).unwrap();
        assert_eq!(
            S3PackedHeadV2::decode(&other, &f.head.encode(&f.ns).unwrap()),
            Err(S3PackIndexV2Error::ForeignVault),
        );
        assert_eq!(
            S3PackIndexRootV2::decode(
                &other, f.index.index_id, &f.index.encode(&f.ns).unwrap()
            ), Err(S3PackIndexV2Error::ForeignVault),
        );
        assert_eq!(
            S3PackIndexShardV2::decode(
                &other, f.shards[0].0, &f.shards[0].1
            ), Err(S3PackIndexV2Error::ForeignVault),
        );
        for mut value in [
            serde_json::from_slice::<serde_json::Value>(&f.head.encode(&f.ns).unwrap()).unwrap(),
            serde_json::from_slice::<serde_json::Value>(&f.index.encode(&f.ns).unwrap()).unwrap(),
            serde_json::from_slice::<serde_json::Value>(&f.shards[0].1).unwrap(),
        ] {
            value["schema"] = 3.into();
            let edited = serde_json::to_vec(&value).unwrap();
            assert!(
                S3PackedHeadV2::decode(&f.ns, &edited).is_err()
                    && S3PackIndexRootV2::decode(
                        &f.ns, f.index.index_id, &edited
                    ).is_err()
                    && S3PackIndexShardV2::decode(
                        &f.ns, f.shards[0].0, &edited
                    ).is_err()
            );
        }
        let mut unknown: serde_json::Value =
            serde_json::from_slice(&f.head.encode(&f.ns).unwrap()).unwrap();
        unknown["allow_delete"] = true.into();
        assert_eq!(
            S3PackedHeadV2::decode(&f.ns, &serde_json::to_vec(&unknown).unwrap()),
            Err(S3PackIndexV2Error::InvalidRoot),
        );
        assert_eq!(
            S3PackedHeadV2::decode(&f.ns, &vec![0; MAX_PACK_V2_ROOT_BYTES + 1]),
            Err(S3PackIndexV2Error::Oversized),
        );
        assert_eq!(
            S3PackIndexRootV2::decode(&f.ns, f.index.index_id,
                &vec![0; MAX_PACK_V2_INDEX_BYTES + 1]),
            Err(S3PackIndexV2Error::Oversized),
        );
        assert_eq!(
            S3PackIndexShardV2::decode(&f.ns, f.shards[0].0,
                &vec![0; MAX_PACK_V2_SHARD_BYTES + 1]),
            Err(S3PackIndexV2Error::Oversized),
        );
    }

    #[test]
    fn one_bit_tamper_wrong_digest_missing_shard_and_fake_manifest_are_held() {
        let f = fixture();
        let mut tampered = f.shards.clone();
        *tampered[0].1.last_mut().unwrap() ^= 1;
        assert_eq!(
            f.index.verify_complete(&f.ns, &f.head, &f.manifest, &f.references, &tampered),
            Err(S3PackIndexV2Error::IntegrityMismatch),
        );
        assert_eq!(
            f.index.verify_complete(&f.ns, &f.head, &f.manifest, &f.references, &[]),
            Err(S3PackIndexV2Error::IncompleteCoverage),
        );
        let mut fake_pointer = f.head.clone();
        fake_pointer.index_blake3 = blake3_hex(b"substituted index");
        assert_eq!(
            f.index.verify_complete(&f.ns, &fake_pointer, &f.manifest,
                &f.references, &f.shards),
            Err(S3PackIndexV2Error::IntegrityMismatch),
        );
        let mut fake_manifest = f.manifest.clone();
        fake_manifest.generation += 1;
        assert_eq!(
            f.index.verify_complete(&f.ns, &f.head, &fake_manifest,
                &f.references, &f.shards),
            Err(S3PackIndexV2Error::IntegrityMismatch),
        );
    }

    #[test]
    fn pack_member_rewrite_missing_or_duplicate_reference_is_not_a_snapshot() {
        let f = fixture();
        let mut corrupted_shard = S3PackIndexShardV2::decode(
            &f.ns, f.shards[0].0, &f.shards[0].1,
        ).unwrap();
        // A changed member reference cannot borrow the identity/digest
        // of another verified revision even if attacker re-hashes its shard.
        corrupted_shard.packs[0].members[0].reference.document_id = DocumentId::new();
        let corrupted_bytes = corrupted_shard.encode(&f.ns).unwrap();
        let mut altered_index = f.index.clone();
        altered_index.shards[0].shard_blake3 = blake3_hex(&corrupted_bytes);
        let altered_head = S3PackedHeadV2::new(
            &f.ns, 1, f.manifest.manifest_id,
            &f.manifest.digest(&f.ns).unwrap(),
            altered_index.index_id, &altered_index.digest(&f.ns).unwrap(), None,
        ).unwrap();
        assert_eq!(
            altered_index.verify_complete(
                &f.ns, &altered_head, &f.manifest, &f.references,
                &[(corrupted_shard.shard_id, corrupted_bytes)],
            ), Err(S3PackIndexV2Error::IncompleteCoverage),
        );

        let shortened = &f.references[..f.references.len() - 1];
        assert_eq!(
            f.index.verify_complete(&f.ns, &f.head, &f.manifest, shortened, &f.shards),
            Err(S3PackIndexV2Error::IncompleteCoverage),
        );
        let mut repeated = f.references.clone();
        repeated.push(f.references[0].clone());
        assert_eq!(
            f.index.verify_complete(&f.ns, &f.head, &f.manifest, &repeated, &f.shards),
            Err(S3PackIndexV2Error::IncompleteCoverage),
        );
    }

    #[test]
    fn forged_offsets_oversize_duplicate_pack_or_shard_are_rejected() {
        let f = fixture();
        let shard = S3PackIndexShardV2::decode(
            &f.ns, f.shards[0].0, &f.shards[0].1,
        ).unwrap();
        let mut broken = shard.clone();
        broken.packs[0].members[0].revision_offset += 1;
        assert_eq!(broken.encode(&f.ns), Err(S3PackIndexV2Error::InvalidMember));
        let mut broken = shard.clone();
        broken.packs[0].members[0].blob_offset = Some(1);
        assert_eq!(broken.encode(&f.ns), Err(S3PackIndexV2Error::InvalidMember));
        let mut broken = shard.clone();
        broken.packs[0].pack_bytes += 1;
        assert_eq!(broken.encode(&f.ns), Err(S3PackIndexV2Error::InvalidPack));
        let mut broken = shard.clone();
        broken.packs.push(shard.packs[0].clone());
        assert_eq!(broken.encode(&f.ns), Err(S3PackIndexV2Error::InvalidPack));
        let mut bad_root = f.index.clone();
        bad_root.shards.push(bad_root.shards[0].clone());
        bad_root.total_revisions *= 2;
        bad_root.total_packs *= 2;
        assert_eq!(bad_root.encode(&f.ns), Err(S3PackIndexV2Error::InvalidShard));
        let mut pack = shard.packs[0].clone();
        pack.pack_blake3 = "X".repeat(64);
        assert_eq!(pack.check(), Err(S3PackIndexV2Error::InvalidPack));
        let mut pack = shard.packs[0].clone();
        pack.pack_bytes = MAX_PACK_V2_BYTES + 1;
        assert_eq!(pack.check(), Err(S3PackIndexV2Error::InvalidPack));
    }

    #[test]
    fn v2_root_chain_requires_authenticated_direct_parent_not_blind_cas_retry() {
        let f = fixture();
        assert!(!f.head.is_direct_successor_of(&f.ns, &f.head));
        let mut child = S3PackedHeadV2::new(
            &f.ns, 2, OperationId::new(), &blake3_hex(b"next manifest"),
            OperationId::new(), &blake3_hex(b"next index"),
            Some(f.head.digest(&f.ns).unwrap()),
        ).unwrap();
        assert!(child.is_direct_successor_of(&f.ns, &f.head));
        child.previous_root_blake3 = Some(blake3_hex(b"unrelated head"));
        assert!(!child.is_direct_successor_of(&f.ns, &f.head));
        assert_eq!(
            S3PackedHeadV2::new(
                &f.ns, 2, OperationId::new(), &blake3_hex(b"m"),
                OperationId::new(), &blake3_hex(b"i"), None,
            ), Err(S3PackIndexV2Error::InvalidRoot),
        );
        let other = S3Keyspace::new("akula-sync/pack-v2-contract", VaultId::new()).unwrap();
        assert!(!child.is_direct_successor_of(&other, &f.head));
    }

    #[test]
    fn pack_index_10k_small_revisions_fit_in_40_bounded_shards() {
        let ns = S3Keyspace::new("akula-sync/pack-v2-large", VaultId::new()).unwrap();
        let mut refs = Vec::new();
        let mut shards = Vec::new();
        for batch in 0..40 {
            let mut packs = Vec::new();
            for _ in 0..2 {
                let mut members = Vec::new();
                for _ in 0..125 {
                    if refs.len() == 10_000 { break; }
                    let r = S3RevisionRefV1 {
                        revision_id: RevisionId::new(),
                        document_id: DocumentId::new(),
                        revision_blake3: blake3_hex(format!("ref-{}", refs.len()).as_bytes()),
                        blob_bytes: Some(1024),
                    };
                    members.push((r.clone(), 200));
                    refs.push(r);
                }
                if !members.is_empty() {
                    packs.push(S3PackDescriptorV2::from_lengths(
                        blake3_hex(format!("pack-{batch}-{}", packs.len()).as_bytes()),
                        members,
                    ).unwrap());
                }
            }
            if !packs.is_empty() {
                let shard = S3PackIndexShardV2::new(
                    &ns, OperationId::new(), packs,
                ).unwrap();
                assert!(shard.encode(&ns).unwrap().len() < MAX_PACK_V2_SHARD_BYTES);
                shards.push(shard.reference(&ns).unwrap());
            }
        }
        assert_eq!(refs.len(), 10_000);
        assert_eq!(shards.len(), 40);
        let index = S3PackIndexRootV2::new(
            &ns, OperationId::new(), 1, OperationId::new(),
            blake3_hex(b"manifest fixture"), shards,
        ).unwrap();
        assert!(index.encode(&ns).unwrap().len() < MAX_PACK_V2_INDEX_BYTES);
        assert_eq!(index.total_revisions, 10_000);
        assert_eq!(index.total_packs, 80);
    }
}

// V2 read/conditional-write integration is deliberately restricted to
// disposable MinIO tests until persisted protocol migration and E2EE gates.
#[cfg(all(test, feature = "sync-s3"))]
#[path = "s3_pack_v2_minio_probe.rs"]
mod minio_probe;
