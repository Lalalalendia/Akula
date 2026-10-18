//! SYNC-COORD-02b: bounded immutable pack verification, read-only only.
//!
//! The pure parser is used by a V1-pinned advisory S3 sidecar verifier.
//! The bounded pack byte builder is also production-compiled as an inert
//! serializer for a future authorized sender. It has NO S3 I/O or ability
//! to publish a root, authenticate a writer or write through File Gate.
//! Each member and the envelope must match the caller's independently
//! verified immutable S3 manifest references. The in-memory test descriptor
//! is not a durable trust root and may NOT grant deletion, publication,
//! baseline acknowledgement, or access to local user files.

use crate::sync::{
    DagError, RevisionDag, RevisionId, RevisionOperation, RevisionRecord,
    S3RevisionRefV1,
};
use std::collections::BTreeSet;
use crate::sync::S3Keyspace;

const MAGIC: &[u8; 8] = b"AKPKEXP1";
pub(super) const MAX_PACK_BYTES: usize = 512 * 1024;
pub(super) const MAX_PACK_MEMBERS: usize = 128;
const MAX_PACK_REVISION_BYTES: usize = 64 * 1024;
const MAX_PACK_MEMBER_BLOB_BYTES: usize = 32 * 1024;
const HEADER_BYTES: usize = MAGIC.len() + 2;

#[derive(Clone, Debug)]
pub(super) struct PackDescriptor {
    pub key: String,
    pub digest: String,
    pub references: Vec<S3RevisionRefV1>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PackProbeError {
    Invalid,
    TooLarge,
    IntegrityMismatch,
    DuplicateRevision,
    Dag(DagError),
}

impl From<DagError> for PackProbeError {
    fn from(value: DagError) -> Self { Self::Dag(value) }
}

/// Fixed-size, memory-bounded encoding:
///   magic[8] | count:u16(le) | repeat(count) {
///     revision_len:u32(le) | blob_len:u32(le) | revision_json | blob_bytes
///   }
/// The pack does not include user filenames or any power to create/delete
/// an Akula file. The canonical RevisionRecord JSON is checked when read.
/// An internal pure encoder does not grant S3 PUT, writer authority or File Gate.
pub(super) struct PackBuilder {
    bytes: Vec<u8>,
    references: Vec<S3RevisionRefV1>,
}

impl PackBuilder {
    pub fn new() -> Self {
        let mut bytes = Vec::with_capacity(HEADER_BYTES);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&0u16.to_le_bytes());
        Self { bytes, references: Vec::new() }
    }

    pub fn len(&self) -> usize { self.references.len() }
    pub fn is_empty(&self) -> bool { self.references.is_empty() }

    pub fn push(
        &mut self,
        reference: S3RevisionRefV1,
        revision_json: &[u8],
        blob: &[u8],
    ) -> Result<(), PackProbeError> {
        if self.len() == MAX_PACK_MEMBERS {
            return Err(PackProbeError::TooLarge);
        }
        if revision_json.is_empty() || revision_json.len() > MAX_PACK_REVISION_BYTES
            || blob.len() > MAX_PACK_MEMBER_BLOB_BYTES
            || !matches!(reference.blob_bytes,
                Some(size) if size == blob.len() as u64)
                && !(reference.blob_bytes.is_none() && blob.is_empty())
            || blake3::hash(revision_json).to_hex().as_str() != reference.revision_blake3
        {
            return Err(PackProbeError::Invalid);
        }
        let next_size = self.bytes.len()
            .checked_add(8)
            .and_then(|n| n.checked_add(revision_json.len()))
            .and_then(|n| n.checked_add(blob.len()))
            .ok_or(PackProbeError::TooLarge)?;
        if next_size > MAX_PACK_BYTES {
            return Err(PackProbeError::TooLarge);
        }
        self.bytes.extend_from_slice(&(revision_json.len() as u32).to_le_bytes());
        self.bytes.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        self.bytes.extend_from_slice(revision_json);
        self.bytes.extend_from_slice(blob);
        self.references.push(reference);
        let member_count = self.references.len() as u16;
        self.bytes[MAGIC.len()..HEADER_BYTES]
            .copy_from_slice(&member_count.to_le_bytes());
        Ok(())
    }

    pub fn seal(self, keyspace: &S3Keyspace)
        -> Result<(PackDescriptor, Vec<u8>), PackProbeError>
    {
        if self.references.is_empty() {
            return Err(PackProbeError::Invalid);
        }
        let digest = blake3::hash(&self.bytes).to_hex().to_string();
        // Only derives an immutable content-addressed blob key. No network I/O,
        // writer authentication, provider publication or File Gate authority.
        let key = keyspace.immutable_blob_key(&digest)
            .map_err(|_| PackProbeError::Invalid)?;
        Ok((PackDescriptor { key, digest, references: self.references }, self.bytes))
    }
}

fn take<'a>(
    input: &'a [u8], cursor: &mut usize, count: usize,
) -> Result<&'a [u8], PackProbeError> {
    let end = cursor.checked_add(count).ok_or(PackProbeError::Invalid)?;
    let slice = input.get(*cursor..end).ok_or(PackProbeError::Invalid)?;
    *cursor = end;
    Ok(slice)
}

fn next_u32(bytes: &[u8], cursor: &mut usize) -> Result<usize, PackProbeError> {
    let raw: [u8; 4] = take(bytes, cursor, 4)?
        .try_into().map_err(|_| PackProbeError::Invalid)?;
    Ok(u32::from_le_bytes(raw) as usize)
}

/// Check a complete bounded S3 pack and ingest its verified members into a
/// discardable DAG. The caller must also check that the union of ALL packs'
/// member references is exactly the complete verified root-manifest index.
/// Partial/failed results are never a confirmed inventory or baseline.
pub(super) fn ingest_verified_pack(
    payload: &[u8],
    descriptor: &PackDescriptor,
    dag: &mut RevisionDag,
    seen: &mut BTreeSet<RevisionId>,
) -> Result<u64, PackProbeError> {
    if payload.len() > MAX_PACK_BYTES || payload.len() < HEADER_BYTES {
        return Err(PackProbeError::TooLarge);
    }
    if blake3::hash(payload).to_hex().as_str() != descriptor.digest {
        return Err(PackProbeError::IntegrityMismatch);
    }
    // A content-addressed object key must name the exact same digest.
    // The S3 client separately enforces the vault-scoped key namespace.
    if !descriptor.key.ends_with(&format!("/blobs/{}", descriptor.digest)) {
        return Err(PackProbeError::Invalid);
    }
    if payload.get(..MAGIC.len()) != Some(MAGIC.as_slice()) {
        return Err(PackProbeError::Invalid);
    }
    let count_bytes: [u8; 2] = payload[MAGIC.len()..HEADER_BYTES]
        .try_into().map_err(|_| PackProbeError::Invalid)?;
    let count = u16::from_le_bytes(count_bytes) as usize;
    if count == 0 || count > MAX_PACK_MEMBERS || count != descriptor.references.len() {
        return Err(PackProbeError::Invalid);
    }
    let mut cursor = HEADER_BYTES;
    let mut local = BTreeSet::new();
    let mut staged = Vec::with_capacity(count);
    let mut validated_bytes = 0_u64;
    for reference in &descriptor.references {
        let revision_len = next_u32(payload, &mut cursor)?;
        let blob_len = next_u32(payload, &mut cursor)?;
        if revision_len == 0 || revision_len > MAX_PACK_REVISION_BYTES
            || blob_len > MAX_PACK_MEMBER_BLOB_BYTES
            || !matches!(reference.blob_bytes, Some(n) if n == blob_len as u64)
                && !(reference.blob_bytes.is_none() && blob_len == 0)
        {
            return Err(PackProbeError::Invalid);
        }
        let revision_json = take(payload, &mut cursor, revision_len)?;
        let blob = take(payload, &mut cursor, blob_len)?;
        if blake3::hash(revision_json).to_hex().as_str() != reference.revision_blake3 {
            return Err(PackProbeError::IntegrityMismatch);
        }
        let record: RevisionRecord = serde_json::from_slice(revision_json)
            .map_err(|_| PackProbeError::Invalid)?;
        if record.id != reference.revision_id
            || record.document_id != reference.document_id
        {
            return Err(PackProbeError::Invalid);
        }
        match &record.operation {
            RevisionOperation::Content { blob_hash, .. }
            | RevisionOperation::Rename { blob_hash, .. } => {
                if reference.blob_bytes != Some(blob_len as u64) {
                    return Err(PackProbeError::Invalid);
                }
                if blake3::hash(blob).to_hex().as_str() != blob_hash {
                    return Err(PackProbeError::IntegrityMismatch);
                }
            }
            RevisionOperation::Tombstone { .. } => {
                if reference.blob_bytes.is_some() || blob_len != 0 {
                    return Err(PackProbeError::Invalid);
                }
            }
        }
        if !local.insert(record.id) || seen.contains(&record.id) {
            return Err(PackProbeError::DuplicateRevision);
        }
        validated_bytes = validated_bytes
            .checked_add((revision_len + blob_len) as u64)
            .ok_or(PackProbeError::TooLarge)?;
        staged.push(record);
    }
    if cursor != payload.len() {
        return Err(PackProbeError::Invalid);
    }
    // Apply only fully validated members. The DAG is a throwaway experiment
    // instance; failed insertions invalidate the entire collection.
    for record in staged {
        dag.ingest(record)?;
    }
    seen.extend(local);
    Ok(validated_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{DocumentId, RevisionOperation, SyncRelativePath, VaultId};

    fn fixture() -> (S3Keyspace, S3RevisionRefV1, Vec<u8>, Vec<u8>) {
        let ns = S3Keyspace::new("akula-sync/pack-proof-fixture", VaultId::new()).unwrap();
        let blob = b"small independent document".to_vec();
        let record = RevisionRecord {
            id: RevisionId::new(),
            document_id: DocumentId::new(),
            parents: vec![],
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/test.md").unwrap(),
                blob_hash: blake3::hash(&blob).to_hex().to_string(),
            },
        };
        let json = serde_json::to_vec(&record).unwrap();
        let reference = S3RevisionRefV1 {
            revision_id: record.id,
            document_id: record.document_id,
            revision_blake3: blake3::hash(&json).to_hex().to_string(),
            blob_bytes: Some(blob.len() as u64),
        };
        (ns, reference, json, blob)
    }

    #[test]
    fn proof_pack_roundtrip_and_all_individual_digests() {
        let (ns, reference, json, blob) = fixture();
        let mut builder = PackBuilder::new();
        builder.push(reference.clone(), &json, &blob).unwrap();
        let (descriptor, bytes) = builder.seal(&ns).unwrap();
        let mut dag = RevisionDag::new();
        let mut seen = BTreeSet::new();
        let hashed = ingest_verified_pack(&bytes, &descriptor, &mut dag, &mut seen).unwrap();
        assert_eq!(hashed, (json.len() + blob.len()) as u64);
        assert_eq!(dag.active_len(), 1);
        assert_eq!(dag.pending_len(), 0);
        assert_eq!(seen, BTreeSet::from([reference.revision_id]));
        assert_eq!(descriptor.key, ns.immutable_blob_key(&descriptor.digest).unwrap());
        assert_eq!(ingest_verified_pack(&bytes, &descriptor, &mut dag, &mut seen),
            Err(PackProbeError::DuplicateRevision));
    }

    #[test]
    fn packed_tombstone_and_offline_siblings_remain_distinct_heads() {
        let ns = S3Keyspace::new("akula-sync/packed-forks-fixture", VaultId::new()).unwrap();
        let doc = DocumentId::new();
        let root = RevisionId::new();
        let alice = RevisionId::new();
        let bob = RevisionId::new();
        let deleted = RevisionId::new();
        let mut builder = PackBuilder::new();
        for (id, parents, payload) in [
            (root, vec![], Some(&b"base"[..])),
            (alice, vec![root], Some(&b"alice edits"[..])),
            (bob, vec![root], Some(&b"bob edits"[..])),
            (deleted, vec![root], None),
        ] {
            let record = RevisionRecord {
                id, document_id: doc, parents,
                operation: match payload {
                    Some(blob) => RevisionOperation::Content {
                        path: SyncRelativePath::parse("notes/conflict.md").unwrap(),
                        blob_hash: blake3::hash(blob).to_hex().to_string(),
                    },
                    None => RevisionOperation::Tombstone {
                        path: SyncRelativePath::parse("notes/conflict.md").unwrap(),
                    },
                },
            };
            let json = serde_json::to_vec(&record).unwrap();
            let ref_v1 = S3RevisionRefV1 {
                revision_id: id,
                document_id: doc,
                revision_blake3: blake3::hash(&json).to_hex().to_string(),
                blob_bytes: payload.map(|b| b.len() as u64),
            };
            builder.push(ref_v1, &json, payload.unwrap_or_default()).unwrap();
        }
        let (descriptor, payload) = builder.seal(&ns).unwrap();
        let mut dag = RevisionDag::new();
        let mut seen = BTreeSet::new();
        ingest_verified_pack(&payload, &descriptor, &mut dag, &mut seen).unwrap();
        assert_eq!(dag.active_len(), 4);
        assert_eq!(dag.pending_len(), 0);
        assert_eq!(dag.heads(doc), BTreeSet::from([alice, bob, deleted]));
        assert_eq!(seen.len(), 4);
    }

    #[test]
    fn forged_pack_digest_member_or_length_cannot_validate() {
        let (ns, reference, json, blob) = fixture();
        let mut builder = PackBuilder::new();
        builder.push(reference.clone(), &json, &blob).unwrap();
        let (descriptor, bytes) = builder.seal(&ns).unwrap();
        let mut dag = RevisionDag::new();
        let mut seen = BTreeSet::new();

        let mut damaged = bytes.clone();
        *damaged.last_mut().unwrap() ^= 1;
        assert_eq!(ingest_verified_pack(&damaged, &descriptor, &mut dag, &mut seen),
            Err(PackProbeError::IntegrityMismatch));

        let mut attacker_rehashed = descriptor.clone();
        attacker_rehashed.digest = blake3::hash(&damaged).to_hex().to_string();
        // A changed digest with the *old* immutable object key is a distinct
        // protocol violation, rejected before member-level content checks.
        assert_eq!(ingest_verified_pack(&damaged, &attacker_rehashed, &mut dag, &mut seen),
            Err(PackProbeError::Invalid));
        // Even when the attacker also moves the payload under a key matching
        // the recomputed outer digest, the original per-member BLAKE3 still
        // rejects the tampered Markdown blob.
        attacker_rehashed.key = ns.immutable_blob_key(&attacker_rehashed.digest).unwrap();
        assert_eq!(ingest_verified_pack(&damaged, &attacker_rehashed, &mut dag, &mut seen),
            Err(PackProbeError::IntegrityMismatch));

        let mut wrong_ref = descriptor.clone();
        wrong_ref.references[0].revision_blake3 =
            blake3::hash(b"another revision").to_hex().to_string();
        assert_eq!(ingest_verified_pack(&bytes, &wrong_ref, &mut dag, &mut seen),
            Err(PackProbeError::IntegrityMismatch));

        let mut extra = bytes.clone();
        extra.push(b'!');
        let mut checked = descriptor.clone();
        checked.digest = blake3::hash(&extra).to_hex().to_string();
        checked.key = ns.immutable_blob_key(&checked.digest).unwrap();
        assert_eq!(ingest_verified_pack(&extra, &checked, &mut dag, &mut seen),
            Err(PackProbeError::Invalid));

        let mut truncated = bytes.clone();
        truncated.truncate(truncated.len() - 5);
        checked.digest = blake3::hash(&truncated).to_hex().to_string();
        checked.key = ns.immutable_blob_key(&checked.digest).unwrap();
        assert_eq!(ingest_verified_pack(&truncated, &checked, &mut dag, &mut seen),
            Err(PackProbeError::Invalid));
        assert_eq!(dag.active_len(), 0);
        assert!(seen.is_empty());
    }

    #[test]
    fn bounded_pack_rejects_too_many_members_and_duplicate_ids() {
        let (ns, reference, json, blob) = fixture();
        let mut builder = PackBuilder::new();
        for _ in 0..MAX_PACK_MEMBERS {
            builder.push(reference.clone(), &json, &blob).unwrap();
        }
        assert_eq!(builder.push(reference.clone(), &json, &blob),
            Err(PackProbeError::TooLarge));
        let (desc, bytes) = builder.seal(&ns).unwrap();
        assert_eq!(ingest_verified_pack(&bytes, &desc,
                &mut RevisionDag::new(), &mut BTreeSet::new()),
            Err(PackProbeError::DuplicateRevision));
        let mut oversized = PackBuilder::new();
        assert_eq!(oversized.push(reference, &json, &vec![0; MAX_PACK_MEMBER_BLOB_BYTES + 1]),
            Err(PackProbeError::Invalid));
        assert_eq!(PackBuilder::new().seal(&ns).unwrap_err(),
            PackProbeError::Invalid);
    }
    #[cfg(feature = "sync-text-delta")]
    #[test]
    fn new_text_delta_is_not_a_legacy_pack() {
        use crate::sync::text_delta_codec;
        let (ns, reference, _, _) = fixture();
        let target = text_delta_codec::encode(
            reference.revision_id,
            "A future independent text envelope".as_bytes(),
            None,
        ).unwrap();
        assert!(target.bytes.starts_with(b"AKTDLT01"));
        let digest = blake3::hash(&target.bytes).to_hex().to_string();
        let descriptor = PackDescriptor {
            key: ns.immutable_blob_key(&digest).unwrap(),
            digest,
            references: vec![reference],
        };
        assert_eq!(
            ingest_verified_pack(
                &target.bytes, &descriptor,
                &mut RevisionDag::new(), &mut BTreeSet::new(),
            ),
            Err(PackProbeError::Invalid),
            "a future content codec must never be misidentified as AKPKEXP1",
        );
    }

}
