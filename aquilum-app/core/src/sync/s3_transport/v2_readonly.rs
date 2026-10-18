//! SYNC-05d2i: explicitly pinned, profile-bound, read-only V2 pack observer.
//!
//! This is NOT a RemoteStore, writer, signed-root authenticator, V1 migration,
//! persisted V2 anchor, File Gate publication or confirmed sync baseline.
//! Callers must supply the precise previously trusted root, its ETag and, for
//! later generations, its trusted immediate V2 predecessor. Merely computing
//! an expected root from today's untrusted S3 response is NOT sufficient.

use std::collections::{BTreeMap, BTreeSet};

use super::{AwsS3ProbeClient, S3TransportError};
use crate::sync::{
    DagError, OperationId, RevisionDag, RevisionId, RevisionOperation,
    RevisionRecord, S3ContractError, S3Keyspace, S3ManifestError,
    S3ManifestShardV1, S3ManifestV1, S3PackDescriptorV2,
    S3PackIndexRootV2, S3PackIndexShardV2, S3PackIndexV2Error,
    S3PackedHeadV2, S3RemoteAnchorScope, S3RevisionRefV1,
    MAX_MANIFEST_BYTES, MAX_SHARD_BYTES, MAX_PACK_V2_BYTES,
    MAX_PACK_V2_INDEX_BYTES, MAX_PACK_V2_ROOT_BYTES,
    MAX_PACK_V2_SHARD_BYTES,
};

#[derive(Debug)]
pub enum S3PinnedV2ReadError {
    ScopeMismatch,
    MissingTrustedRoot,
    InvalidAncestry,
    InvalidRevision,
    InvalidPackPayload,
    IncompleteDAG,
    RootChanged,
    Transport(S3TransportError),
    Contract(S3ContractError),
    Manifest(S3ManifestError),
    Index(S3PackIndexV2Error),
    Dag(DagError),
}

impl From<S3TransportError> for S3PinnedV2ReadError {
    fn from(e: S3TransportError) -> Self { Self::Transport(e) }
}
impl From<S3ContractError> for S3PinnedV2ReadError {
    fn from(e: S3ContractError) -> Self { Self::Contract(e) }
}
impl From<S3ManifestError> for S3PinnedV2ReadError {
    fn from(e: S3ManifestError) -> Self { Self::Manifest(e) }
}
impl From<S3PackIndexV2Error> for S3PinnedV2ReadError {
    fn from(e: S3PackIndexV2Error) -> Self { Self::Index(e) }
}
impl From<DagError> for S3PinnedV2ReadError {
    fn from(e: DagError) -> Self { Self::Dag(e) }
}

/// Diagnostic only. Never use this object to authorize downloading a file
/// into the vault, deleting a note, acknowledging a baseline or cloud GC.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3PinnedV2ReadPreview {
    pub generation: u64,
    pub verified_revisions: usize,
    pub verified_packs: usize,
    pub verified_pack_bytes: u64,
    pub document_head_count: usize,
    pub requires_explicit_file_gate_review: bool,
}

/// A complete immutable manifest ref index. Neither S3 LIST nor an ETag is
/// treated as a consistent listing or a BLAKE3 content digest.
async fn pinned_manifest_references(
    client: &AwsS3ProbeClient,
    ns: &S3Keyspace,
    id: OperationId,
    hash: &str,
) -> Result<(S3ManifestV1, BTreeMap<RevisionId, S3RevisionRefV1>), S3PinnedV2ReadError> {
    let raw = client.get_bounded(
        &ns.immutable_manifest_key(id), MAX_MANIFEST_BYTES as u64,
    ).await?;
    if raw.blake3 != hash {
        return Err(S3PinnedV2ReadError::InvalidRevision);
    }
    let manifest = S3ManifestV1::decode(ns, id, &raw.bytes)?;
    let mut references = BTreeMap::new();
    for shard_ref in &manifest.shards {
        let payload = client.get_bounded(
            &ns.immutable_manifest_key(shard_ref.shard_id),
            MAX_SHARD_BYTES as u64,
        ).await?;
        if payload.blake3 != shard_ref.shard_blake3 {
            return Err(S3PinnedV2ReadError::InvalidRevision);
        }
        let shard = S3ManifestShardV1::decode(
            ns, shard_ref.shard_id, &payload.bytes,
        )?;
        if shard.revisions.len() != shard_ref.count as usize
            || shard.revisions.first().map(|r| r.revision_id)
                != Some(shard_ref.first_revision)
            || shard.revisions.last().map(|r| r.revision_id)
                != Some(shard_ref.last_revision)
        {
            return Err(S3PinnedV2ReadError::InvalidRevision);
        }
        for reference in shard.revisions {
            if references.insert(reference.revision_id, reference).is_some() {
                return Err(S3PinnedV2ReadError::InvalidRevision);
            }
        }
    }
    if references.len() != manifest.total_revisions as usize {
        return Err(S3PinnedV2ReadError::IncompleteDAG);
    }
    Ok((manifest, references))
}

fn slice_at(bytes: &[u8], start: usize, len: usize)
    -> Result<&[u8], S3PinnedV2ReadError>
{
    let end = start.checked_add(len)
        .ok_or(S3PinnedV2ReadError::InvalidPackPayload)?;
    bytes.get(start..end).ok_or(S3PinnedV2ReadError::InvalidPackPayload)
}

/// Independently validate each frame after verifying the immutable outer
/// pack's BLAKE3. A maliciously rehashed envelope cannot rewrite a pinned
/// record or blob without an explicit reject.
fn verify_packed_members(
    bytes: &[u8],
    desc: &S3PackDescriptorV2,
    dag: &mut RevisionDag,
    seen: &mut BTreeSet<RevisionId>,
) -> Result<(), S3PinnedV2ReadError> {
    if bytes.len() != desc.pack_bytes as usize
        || blake3::hash(bytes).to_hex().as_str() != desc.pack_blake3
        || bytes.len() < 10
        || &bytes[..8] != b"AKPKEXP1"
    {
        return Err(S3PinnedV2ReadError::InvalidPackPayload);
    }
    let member_count = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
    if member_count != desc.members.len() {
        return Err(S3PinnedV2ReadError::InvalidPackPayload);
    }
    let mut cursor = 10_usize;
    for member in &desc.members {
        let frame = slice_at(bytes, cursor, 8)?;
        let revision_len = u32::from_le_bytes(frame[..4].try_into()
            .map_err(|_| S3PinnedV2ReadError::InvalidPackPayload)?) as usize;
        let blob_len = u32::from_le_bytes(frame[4..8].try_into()
            .map_err(|_| S3PinnedV2ReadError::InvalidPackPayload)?) as usize;
        let revision_offset = cursor.checked_add(8)
            .ok_or(S3PinnedV2ReadError::InvalidPackPayload)?;
        if revision_offset != member.revision_offset as usize
            || revision_len != member.revision_len as usize
            || member.reference.blob_bytes != member.blob_offset.map(|_| blob_len as u64)
        {
            return Err(S3PinnedV2ReadError::InvalidPackPayload);
        }
        let json = slice_at(bytes, revision_offset, revision_len)?;
        if blake3::hash(json).to_hex().as_str() != member.reference.revision_blake3 {
            return Err(S3PinnedV2ReadError::InvalidRevision);
        }
        let blob_offset = revision_offset.checked_add(revision_len)
            .ok_or(S3PinnedV2ReadError::InvalidPackPayload)?;
        if member.blob_offset.map(|x| x as usize)
            != member.reference.blob_bytes.map(|_| blob_offset)
        {
            return Err(S3PinnedV2ReadError::InvalidPackPayload);
        }
        let blob = slice_at(bytes, blob_offset, blob_len)?;
        cursor = blob_offset.checked_add(blob_len)
            .ok_or(S3PinnedV2ReadError::InvalidPackPayload)?;
        let record: RevisionRecord = serde_json::from_slice(json)
            .map_err(|_| S3PinnedV2ReadError::InvalidRevision)?;
        if record.id != member.reference.revision_id
            || record.document_id != member.reference.document_id
            || !seen.insert(record.id)
        {
            return Err(S3PinnedV2ReadError::InvalidRevision);
        }
        match &record.operation {
            RevisionOperation::Content { blob_hash, .. }
            | RevisionOperation::Rename { blob_hash, .. } => {
                if member.reference.blob_bytes != Some(blob_len as u64)
                    || blake3::hash(blob).to_hex().as_str() != blob_hash
                {
                    return Err(S3PinnedV2ReadError::InvalidPackPayload);
                }
            }
            RevisionOperation::Tombstone { .. } => {
                if !blob.is_empty() || member.reference.blob_bytes.is_some() {
                    return Err(S3PinnedV2ReadError::InvalidPackPayload);
                }
            }
        }
        dag.ingest(record)?;
    }
    if cursor != bytes.len() {
        return Err(S3PinnedV2ReadError::InvalidPackPayload);
    }
    Ok(())
}

impl AwsS3ProbeClient {
    /// Full S3 V2 content inspection against *externally trusted* exact root
    /// bytes/ETag and the exact profile/account/vault scope.
    ///
    /// A generation >1 also needs the previous V2 root from the caller's
    /// independent trusted checkpoint. There is no API here for discovering
    /// a trusted root, recording one, upgrading V1 or publishing V2.
    pub async fn inspect_explicitly_pinned_v2_read_only(
        &self,
        scope: &S3RemoteAnchorScope,
        trusted_root: &S3PackedHeadV2,
        trusted_etag: &str,
        trusted_previous_root: Option<&S3PackedHeadV2>,
    ) -> Result<S3PinnedV2ReadPreview, S3PinnedV2ReadError> {
        // Refuse cross-account/profile or vault confusion BEFORE any network.
        if !scope.matches_connection(self.bound_profile(), self.bound_keyspace()) {
            return Err(S3PinnedV2ReadError::ScopeMismatch);
        }
        let ns = self.bound_keyspace();
        let expected = trusted_root.encode(ns)?;
        if trusted_etag.is_empty() || trusted_etag.len() > 256
            || trusted_etag.bytes().any(|c| c.is_ascii_control())
        {
            return Err(S3PinnedV2ReadError::MissingTrustedRoot);
        }
        if trusted_root.generation() == 1 {
            if trusted_previous_root.is_some() {
                return Err(S3PinnedV2ReadError::InvalidAncestry);
            }
        } else {
            let previous = trusted_previous_root
                .ok_or(S3PinnedV2ReadError::MissingTrustedRoot)?;
            if !trusted_root.is_direct_successor_of(ns, previous) {
                return Err(S3PinnedV2ReadError::InvalidAncestry);
            }
        }
        let key = ns.current_head_key();
        let first = self.get_bounded(&key, MAX_PACK_V2_ROOT_BYTES as u64).await?;
        if first.object.etag != trusted_etag || first.bytes != expected {
            return Err(S3PinnedV2ReadError::RootChanged);
        }
        let root = S3PackedHeadV2::decode(ns, &first.bytes)?;
        if &root != trusted_root {
            return Err(S3PinnedV2ReadError::RootChanged);
        }
        let (manifest, references) = pinned_manifest_references(
            self, ns, root.manifest_id(), root.manifest_blake3(),
        ).await?;
        if manifest.generation != root.generation() {
            return Err(S3PinnedV2ReadError::InvalidAncestry);
        }
        if let Some(previous) = trusted_previous_root {
            if manifest.previous_manifest_id != Some(previous.manifest_id())
                || manifest.previous_manifest_blake3.as_deref()
                    != Some(previous.manifest_blake3())
            {
                return Err(S3PinnedV2ReadError::InvalidAncestry);
            }
            let (older_manifest, older_refs) = pinned_manifest_references(
                self, ns, previous.manifest_id(), previous.manifest_blake3(),
            ).await?;
            if !manifest.is_successor_of(&older_manifest, previous.manifest_blake3())
                || !older_refs.iter().all(|(id, reference)|
                    references.get(id) == Some(reference))
            {
                return Err(S3PinnedV2ReadError::InvalidAncestry);
            }
        } else if manifest.previous_manifest_id.is_some()
            || manifest.previous_manifest_blake3.is_some()
        {
            return Err(S3PinnedV2ReadError::InvalidAncestry);
        }

        let index_raw = self.get_bounded(
            &ns.immutable_manifest_key(root.index_id()),
            MAX_PACK_V2_INDEX_BYTES as u64,
        ).await?;
        if index_raw.blake3 != root.index_blake3() {
            return Err(S3PinnedV2ReadError::InvalidPackPayload);
        }
        let index = S3PackIndexRootV2::decode(
            ns, root.index_id(), &index_raw.bytes,
        )?;
        let mut shards = Vec::with_capacity(index.shards.len());
        for reference in &index.shards {
            let data = self.get_bounded(
                &ns.immutable_manifest_key(reference.shard_id),
                MAX_PACK_V2_SHARD_BYTES as u64,
            ).await?;
            if data.blake3 != reference.shard_blake3 {
                return Err(S3PinnedV2ReadError::InvalidPackPayload);
            }
            shards.push((reference.shard_id, data.bytes));
        }
        index.verify_complete(
            ns, &root, &manifest,
            &references.into_values().collect::<Vec<_>>(),
            &shards,
        )?;
        let mut dag = RevisionDag::new();
        let mut seen = BTreeSet::new();
        let mut pack_bytes = 0_u64;
        let mut pack_count = 0_usize;
        for (id, bytes) in shards {
            let shard = S3PackIndexShardV2::decode(ns, id, &bytes)?;
            for desc in shard.packs {
                let object = self.get_bounded(
                    &ns.immutable_blob_key(&desc.pack_blake3)?,
                    MAX_PACK_V2_BYTES as u64,
                ).await?;
                verify_packed_members(&object.bytes, &desc, &mut dag, &mut seen)?;
                pack_bytes = pack_bytes.checked_add(object.bytes.len() as u64)
                    .ok_or(S3PinnedV2ReadError::InvalidPackPayload)?;
                pack_count += 1;
            }
        }
        if seen.len() != manifest.total_revisions as usize
            || dag.active_len() != seen.len() || dag.pending_len() != 0
        {
            return Err(S3PinnedV2ReadError::IncompleteDAG);
        }
        // Final root check fails closed on concurrent CAS or read-after-write
        // inconsistency even when immutable object hashes match.
        let last = self.get_bounded(&key, MAX_PACK_V2_ROOT_BYTES as u64).await?;
        if last.bytes != first.bytes || last.object.etag != first.object.etag {
            return Err(S3PinnedV2ReadError::RootChanged);
        }
        // HEAD count and DAG heads are advisory; no write/remove methods
        // are returned with the preview.
        let mut documents = BTreeSet::new();
        for id in seen {
            let record = dag.get(id).ok_or(S3PinnedV2ReadError::IncompleteDAG)?;
            documents.insert(record.document_id);
        }
        let head_count = documents.into_iter()
            .map(|doc| dag.heads(doc).len()).sum();
        Ok(S3PinnedV2ReadPreview {
            generation: root.generation(),
            verified_revisions: dag.active_len(),
            verified_packs: pack_count,
            verified_pack_bytes: pack_bytes,
            document_head_count: head_count,
            requires_explicit_file_gate_review: true,
        })
    }
}

#[cfg(test)]
#[path = "v2_readonly_tests.rs"]
mod tests;
