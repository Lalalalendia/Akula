//! SYNC-05d: strict immutable, sharded manifest contracts.
//!
//! No local path, credential or native file authority is stored in the index.
//! A manifest is only *descriptive* until every referenced shard and revision
//! has been independently verified. ListObjectsV2 is never a snapshot.

use serde::{Deserialize, Serialize};
use super::{DocumentId, OperationId, RevisionId, S3Keyspace, VaultId};
use std::collections::BTreeSet;

const SCHEMA: u16 = 1;
pub const MAX_MANIFEST_BYTES: usize = 512 * 1024;
pub const MAX_SHARD_BYTES: usize = 128 * 1024;
pub const MAX_SHARDS: usize = 256;
pub const MAX_SHARD_ENTRIES: usize = 128;
pub const MAX_REVISIONS: usize = MAX_SHARDS * MAX_SHARD_ENTRIES;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum S3ManifestError {
    InvalidManifest,
    InvalidShard,
    InvalidRevision,
    ForeignVault,
    UnsupportedSchema,
    Oversized,
    InvalidAncestry,
    IntegrityMismatch,
}

pub(crate) fn is_blake3(value: &str) -> bool {
    value.len() == 64 &&
        value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Hash and declared size of one *immutable revision JSON*, not its mutable
/// document or note path. The BLAKE3 object is checked independently.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3RevisionRefV1 {
    pub revision_id: RevisionId,
    pub document_id: DocumentId,
    pub revision_blake3: String,
    /// Required for content/rename, None only for an actual tombstone.
    pub blob_bytes: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3ManifestShardV1 {
    schema: u16,
    vault_id: VaultId,
    pub shard_id: OperationId,
    pub revisions: Vec<S3RevisionRefV1>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3ManifestShardRefV1 {
    pub shard_id: OperationId,
    pub shard_blake3: String,
    pub count: u16,
    pub first_revision: RevisionId,
    pub last_revision: RevisionId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3ManifestV1 {
    schema: u16,
    vault_id: VaultId,
    pub manifest_id: OperationId,
    pub generation: u64,
    pub previous_manifest_id: Option<OperationId>,
    pub previous_manifest_blake3: Option<String>,
    pub total_revisions: u32,
    pub shards: Vec<S3ManifestShardRefV1>,
}

impl S3ManifestShardV1 {
    pub fn new(
        ns: &S3Keyspace,
        shard_id: OperationId,
        revisions: Vec<S3RevisionRefV1>,
    ) -> Result<Self, S3ManifestError> {
        let shard = Self {
            schema: SCHEMA, vault_id: ns.vault_id(), shard_id, revisions,
        };
        shard.check(ns)?;
        Ok(shard)
    }

    pub fn encode(&self, ns: &S3Keyspace) -> Result<Vec<u8>, S3ManifestError> {
        self.check(ns)?;
        let bytes = serde_json::to_vec(self).map_err(|_| S3ManifestError::InvalidShard)?;
        if bytes.len() > MAX_SHARD_BYTES { return Err(S3ManifestError::Oversized); }
        Ok(bytes)
    }

    pub fn decode(
        ns: &S3Keyspace,
        id: OperationId,
        bytes: &[u8],
    ) -> Result<Self, S3ManifestError> {
        if bytes.len() > MAX_SHARD_BYTES { return Err(S3ManifestError::Oversized); }
        let shard: Self = serde_json::from_slice(bytes)
            .map_err(|_| S3ManifestError::InvalidShard)?;
        shard.check(ns)?;
        if shard.shard_id != id { return Err(S3ManifestError::InvalidShard); }
        Ok(shard)
    }

    pub fn reference(&self, ns: &S3Keyspace)
        -> Result<S3ManifestShardRefV1, S3ManifestError>
    {
        let bytes = self.encode(ns)?;
        Ok(S3ManifestShardRefV1 {
            shard_id: self.shard_id,
            shard_blake3: blake3::hash(&bytes).to_hex().to_string(),
            count: self.revisions.len() as u16,
            first_revision: self.revisions[0].revision_id,
            last_revision: self.revisions.last().unwrap().revision_id,
        })
    }

    fn check(&self, ns: &S3Keyspace) -> Result<(), S3ManifestError> {
        if self.schema != SCHEMA { return Err(S3ManifestError::UnsupportedSchema); }
        if self.vault_id != ns.vault_id() { return Err(S3ManifestError::ForeignVault); }
        if self.revisions.is_empty() || self.revisions.len() > MAX_SHARD_ENTRIES {
            return Err(S3ManifestError::InvalidShard);
        }
        let mut prev = None;
        for revision in &self.revisions {
            if !is_blake3(&revision.revision_blake3)
                || prev.is_some_and(|last| last >= revision.revision_id)
            {
                return Err(S3ManifestError::InvalidRevision);
            }
            prev = Some(revision.revision_id);
        }
        Ok(())
    }
}

impl S3ManifestV1 {
    pub fn new(
        ns: &S3Keyspace,
        manifest_id: OperationId,
        generation: u64,
        previous: Option<(OperationId, String)>,
        shards: Vec<S3ManifestShardRefV1>,
    ) -> Result<Self, S3ManifestError> {
        let (previous_manifest_id, previous_manifest_blake3) = match previous {
            Some((id, hash)) => (Some(id), Some(hash)),
            None => (None, None),
        };
        let total_revisions = shards.iter().map(|s| s.count as u32).sum();
        let manifest = Self {
            schema: SCHEMA,
            vault_id: ns.vault_id(),
            manifest_id,
            generation,
            previous_manifest_id,
            previous_manifest_blake3,
            total_revisions,
            shards,
        };
        manifest.check(ns)?;
        Ok(manifest)
    }

    pub fn encode(&self, ns: &S3Keyspace) -> Result<Vec<u8>, S3ManifestError> {
        self.check(ns)?;
        let bytes = serde_json::to_vec(self).map_err(|_| S3ManifestError::InvalidManifest)?;
        if bytes.len() > MAX_MANIFEST_BYTES { return Err(S3ManifestError::Oversized); }
        Ok(bytes)
    }

    pub fn decode(
        ns: &S3Keyspace,
        id: OperationId,
        bytes: &[u8],
    ) -> Result<Self, S3ManifestError> {
        if bytes.len() > MAX_MANIFEST_BYTES { return Err(S3ManifestError::Oversized); }
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|_| S3ManifestError::InvalidManifest)?;
        manifest.check(ns)?;
        if manifest.manifest_id != id { return Err(S3ManifestError::InvalidManifest); }
        Ok(manifest)
    }

    pub fn digest(&self, ns: &S3Keyspace) -> Result<String, S3ManifestError> {
        Ok(blake3::hash(&self.encode(ns)?).to_hex().to_string())
    }

    /// Check the immediate immutable predecessor. The network reader must
    /// independently fetch and validate the actual prior manifest bytes.
    pub fn is_successor_of(&self, prior: &Self, prior_digest: &str) -> bool {
        self.vault_id == prior.vault_id
            && prior.generation.checked_add(1) == Some(self.generation)
            && self.previous_manifest_id == Some(prior.manifest_id)
            && self.previous_manifest_blake3.as_deref() == Some(prior_digest)
            && self.manifest_id != prior.manifest_id
    }

    fn check(&self, ns: &S3Keyspace) -> Result<(), S3ManifestError> {
        if self.schema != SCHEMA { return Err(S3ManifestError::UnsupportedSchema); }
        if self.vault_id != ns.vault_id() { return Err(S3ManifestError::ForeignVault); }
        if self.generation == 0
            || (self.generation == 1) != self.previous_manifest_id.is_none()
            || self.previous_manifest_id.is_some() != self.previous_manifest_blake3.is_some()
            || self.previous_manifest_id == Some(self.manifest_id)
            || self.previous_manifest_blake3.as_deref().is_some_and(|v| !is_blake3(v))
        { return Err(S3ManifestError::InvalidAncestry); }

        if self.shards.len() > MAX_SHARDS
            || self.total_revisions as usize > MAX_REVISIONS
        { return Err(S3ManifestError::Oversized); }

        let mut ids = BTreeSet::new();
        let mut last_revision = None;
        let mut count = 0_u32;
        for shard in &self.shards {
            if !ids.insert(shard.shard_id)
                || shard.shard_id == self.manifest_id
                || !is_blake3(&shard.shard_blake3)
                || shard.count == 0 || shard.count as usize > MAX_SHARD_ENTRIES
                || shard.first_revision > shard.last_revision
                || last_revision.is_some_and(|prev| prev >= shard.first_revision)
            { return Err(S3ManifestError::InvalidShard); }
            count += u32::from(shard.count);
            last_revision = Some(shard.last_revision);
        }
        if count != self.total_revisions { return Err(S3ManifestError::InvalidManifest); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns() -> S3Keyspace { S3Keyspace::new("akula-sync/manifest", VaultId::new()).unwrap() }
    fn digest(data: &[u8]) -> String { blake3::hash(data).to_hex().to_string() }

    #[test]
    fn strict_shard_and_root_codec_rejects_foreign_vault_extra_fields_and_bad_hash() {
        let layout = ns();
        let id = RevisionId::new();
        let shard_id = OperationId::new();
        let shard = S3ManifestShardV1::new(&layout, shard_id, vec![S3RevisionRefV1 {
            revision_id: id,
            document_id: DocumentId::new(),
            revision_blake3: digest(b"revision json"),
            blob_bytes: Some(5),
        }]).unwrap();
        let shard_bytes = shard.encode(&layout).unwrap();
        assert_eq!(S3ManifestShardV1::decode(&layout, shard_id, &shard_bytes), Ok(shard.clone()));
        assert_eq!(S3ManifestShardV1::decode(&ns(), shard_id, &shard_bytes),
            Err(S3ManifestError::ForeignVault));
        let wrong_id = OperationId::new();
        assert_eq!(S3ManifestShardV1::decode(&layout, wrong_id, &shard_bytes),
            Err(S3ManifestError::InvalidShard));
        let mut extra: serde_json::Value = serde_json::from_slice(&shard_bytes).unwrap();
        extra["unknown_authority"] = true.into();
        assert_eq!(S3ManifestShardV1::decode(&layout, shard_id,
            &serde_json::to_vec(&extra).unwrap()), Err(S3ManifestError::InvalidShard));

        let root = S3ManifestV1::new(&layout, OperationId::new(), 1,
            None, vec![shard.reference(&layout).unwrap()]).unwrap();
        let bytes = root.encode(&layout).unwrap();
        assert_eq!(S3ManifestV1::decode(&layout, root.manifest_id, &bytes), Ok(root.clone()));
        extra = serde_json::from_slice(&bytes).unwrap();
        extra["shards"][0]["shard_blake3"] = "invalid".into();
        assert_eq!(S3ManifestV1::decode(&layout, root.manifest_id,
            &serde_json::to_vec(&extra).unwrap()), Err(S3ManifestError::InvalidShard));
        extra = serde_json::from_slice(&bytes).unwrap();
        extra["schema"] = 2.into();
        assert_eq!(S3ManifestV1::decode(&layout, root.manifest_id,
            &serde_json::to_vec(&extra).unwrap()), Err(S3ManifestError::UnsupportedSchema));
    }

    #[test]
    fn immutable_manifest_parent_chain_is_checked() {
        let layout = ns();
        let first = S3ManifestV1::new(&layout, OperationId::new(), 1, None, vec![]).unwrap();
        let first_hash = first.digest(&layout).unwrap();
        let second = S3ManifestV1::new(&layout, OperationId::new(), 2,
            Some((first.manifest_id, first_hash.clone())), vec![]).unwrap();
        assert!(second.is_successor_of(&first, &first_hash));
        assert!(!first.is_successor_of(&second, &second.digest(&layout).unwrap()));
        assert_eq!(S3ManifestV1::new(&layout, OperationId::new(), 2, None, vec![]),
            Err(S3ManifestError::InvalidAncestry));
    }

    #[test]
    fn ten_thousand_revisions_fit_in_verified_shard_index() {
        let layout = ns();
        let mut refs = Vec::new();
        // UUID identities are sorted before composing the index; never
        // infer a stable pagination order from ListObjectsV2 timestamps.
        let mut ids: Vec<_> = (0..10_000).map(|_| RevisionId::new()).collect();
        ids.sort_unstable();
        for group in ids.chunks(MAX_SHARD_ENTRIES) {
            let rows = group.iter().copied().map(|id| S3RevisionRefV1 {
                revision_id: id,
                document_id: DocumentId::new(),
                revision_blake3: digest(id.to_string().as_bytes()),
                blob_bytes: None,
            }).collect();
            let shard = S3ManifestShardV1::new(&layout, OperationId::new(), rows).unwrap();
            let encoded = shard.encode(&layout).unwrap();
            assert!(encoded.len() < MAX_SHARD_BYTES);
            refs.push(shard.reference(&layout).unwrap());
        }
        let root = S3ManifestV1::new(&layout, OperationId::new(), 1, None, refs).unwrap();
        assert_eq!(root.total_revisions, 10_000);
        let blob = root.encode(&layout).unwrap();
        assert!(blob.len() < MAX_MANIFEST_BYTES);
        assert_eq!(S3ManifestV1::decode(&layout, root.manifest_id, &blob), Ok(root));
    }
}
