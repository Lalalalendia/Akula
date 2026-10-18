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
    DocumentId, OperationId, RevisionId, S3Keyspace, S3ManifestV1,
    S3RevisionRefV1, VaultId,
};
use super::s3_manifest::is_blake3;

const SCHEMA_V2: u16 = 2;
pub const MAX_PACK_V2_ROOT_BYTES: usize = 4096;
pub const MAX_PACK_V2_INDEX_BYTES: usize = 128 * 1024;
pub const MAX_PACK_V2_SHARD_BYTES: usize = 128 * 1024;
pub const MAX_PACK_V2_SHARDS: usize = 256;
pub const MAX_PACK_V2_PACKS_PER_SHARD: usize = 4;
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
