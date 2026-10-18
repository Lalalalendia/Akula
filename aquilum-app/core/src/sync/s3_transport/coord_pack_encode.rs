//! SYNC-COORD-03a: pure production-compiled V2 delta-pack preparation.
//!
//! This performs no network I/O, journal mutation or File Gate operation.
//! A prepared pack is *not* permission to publish: a future host-owned
//! uploader must separately prove source authority, exclusive custody,
//! V1/V2 ancestor retention and provider-enforced CAS with lost-ACK recovery.
//! No mutable remote root key is derived or written by this module.

use super::coord_chain::MAX_OFFLINE_PACK_BYTES;
use super::pack_probe::{
    ingest_verified_pack, PackBuilder, PackDescriptor, PackProbeError,
    MAX_PACK_BYTES, MAX_PACK_MEMBERS,
};
use crate::sync::{
    RevisionDag, S3DeltaPackRefV2, S3Keyspace, S3RevisionRefV1,
    MAX_DELTA_ENVELOPE_REVISIONS, MAX_DELTA_PACK_BYTES, MAX_DELTA_PACK_MEMBERS,
};
use std::collections::BTreeSet;

/// One entirely checked immutable wire object and its V2 letter index entry.
/// The outer pack hash and every member JSON/content BLAKE3 are validated
/// before the enclosing batch is returned.
#[derive(Debug)]
#[allow(dead_code)]
pub(super) struct PreparedDeltaPackV2 {
    pub index: S3DeltaPackRefV2,
    pub descriptor: PackDescriptor,
    pub payload: Vec<u8>,
}

/// Complete sorted addition inventory. Nothing in this type authenticates a
/// writer, applies local files, acknowledges a revision, or opens an S3 client.
#[derive(Debug)]
#[allow(dead_code)]
pub(super) struct PreparedDeltaPacksV2 {
    pub references: Vec<S3RevisionRefV1>,
    pub packs: Vec<PreparedDeltaPackV2>,
    pub packed_bytes: usize,
}

fn flush_pack(
    ns: &S3Keyspace,
    builder: &mut PackBuilder,
    packs: &mut Vec<PreparedDeltaPackV2>,
    packed_bytes: &mut usize,
    packed_members: &mut usize,
) -> Result<(), PackProbeError> {
    if builder.is_empty() {
        return Ok(());
    }
    let (descriptor, payload) =
        std::mem::replace(builder, PackBuilder::new()).seal(ns)?;
    let next_bytes = packed_bytes.checked_add(payload.len())
        .ok_or(PackProbeError::TooLarge)?;
    if next_bytes as u64 > MAX_OFFLINE_PACK_BYTES
        || payload.len() > MAX_PACK_BYTES
    {
        return Err(PackProbeError::TooLarge);
    }
    let count = descriptor.references.len();
    let offset = u16::try_from(*packed_members).map_err(|_| PackProbeError::TooLarge)?;
    let member_count = u16::try_from(count).map_err(|_| PackProbeError::TooLarge)?;
    let byte_length = u32::try_from(payload.len()).map_err(|_| PackProbeError::TooLarge)?;
    if count == 0 || count > MAX_PACK_MEMBERS
        || byte_length > MAX_DELTA_PACK_BYTES
    {
        return Err(PackProbeError::TooLarge);
    }
    *packed_members = packed_members.checked_add(count)
        .ok_or(PackProbeError::TooLarge)?;
    *packed_bytes = next_bytes;
    let index = S3DeltaPackRefV2 {
        pack_blake3: descriptor.digest.clone(),
        byte_length,
        first_revision_index: offset,
        revision_count: member_count,
    };
    packs.push(PreparedDeltaPackV2 { index, descriptor, payload });
    Ok(())
}

/// Prepare a sorted, contiguous, bounded delta batch in memory only.
/// Allows up to 1024 additions, 128 members per pack, 512 KiB per pack
/// and 16 MiB aggregate pack bytes, matching the existing offline V2 reader.
///
/// The inputs must come from an independently admitted source. This function
/// rejects empty input (no fabricated no-change V2 letter), duplicate UUIDs,
/// tampered JSON and content, mismatched document IDs, malformed tombstones,
/// oversized members and all index/pack-budget overflows. It checks every
/// self-produced pack with the very same decoder used by the S3 reader before
/// returning *any* prepared objects.
#[allow(dead_code)]
pub(super) fn prepare_delta_packs_v2(
    ns: &S3Keyspace,
    entries: &[(S3RevisionRefV1, Vec<u8>, Vec<u8>)],
) -> Result<PreparedDeltaPacksV2, PackProbeError> {
    if entries.is_empty() {
        return Err(PackProbeError::Invalid);
    }
    if entries.len() > MAX_DELTA_ENVELOPE_REVISIONS {
        return Err(PackProbeError::TooLarge);
    }
    debug_assert_eq!(MAX_DELTA_PACK_MEMBERS, MAX_PACK_MEMBERS);
    let mut ordered: Vec<_> = entries.iter().collect();
    ordered.sort_by_key(|entry| entry.0.revision_id);
    if ordered.windows(2).any(|w| w[0].0.revision_id == w[1].0.revision_id) {
        return Err(PackProbeError::DuplicateRevision);
    }
    let references: Vec<_> =
        ordered.iter().map(|entry| entry.0.clone()).collect();
    let mut packs = Vec::new();
    let mut builder = PackBuilder::new();
    let mut packed_bytes = 0usize;
    let mut packed_members = 0usize;
    for (reference, json, blob) in ordered {
        if builder.len() == MAX_PACK_MEMBERS {
            flush_pack(
                ns, &mut builder, &mut packs, &mut packed_bytes,
                &mut packed_members,
            )?;
        }
        match builder.push(reference.clone(), json, blob) {
            Ok(()) => {}
            Err(PackProbeError::TooLarge) if !builder.is_empty() => {
                flush_pack(
                    ns, &mut builder, &mut packs, &mut packed_bytes,
                    &mut packed_members,
                )?;
                builder.push(reference.clone(), json, blob)?;
            }
            Err(error) => return Err(error),
        }
    }
    flush_pack(
        ns, &mut builder, &mut packs, &mut packed_bytes,
        &mut packed_members,
    )?;
    if packed_members != entries.len() || packs.is_empty() {
        return Err(PackProbeError::Invalid);
    }

    // Verify the entire batch with the actual production pack decoder.
    // A previously recorded parent may be outside this *delta*, so a pending
    // DAG parent is not by itself evidence of a corrupt new-member batch.
    // Complete old/new DAG closure belongs to the V1-anchored receiver.
    let mut dag = RevisionDag::new();
    let mut seen = BTreeSet::new();
    for pack in &packs {
        ingest_verified_pack(
            &pack.payload, &pack.descriptor, &mut dag, &mut seen,
        )?;
    }
    let expected: BTreeSet<_> =
        references.iter().map(|r| r.revision_id).collect();
    if seen != expected || dag.active_len() + dag.pending_len() < expected.len() {
        return Err(PackProbeError::Invalid);
    }
    Ok(PreparedDeltaPacksV2 { references, packs, packed_bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{
        DocumentId, RevisionId, RevisionOperation, RevisionRecord,
        SyncRelativePath, VaultId,
    };

    fn ns() -> S3Keyspace {
        S3Keyspace::new("akula/coord-03a-pure", VaultId::new()).unwrap()
    }

    fn fixture(
        i: usize, doc: DocumentId, id: RevisionId,
        parent: Option<RevisionId>, content: Option<Vec<u8>>,
    ) -> (S3RevisionRefV1, Vec<u8>, Vec<u8>) {
        let path = SyncRelativePath::parse(
            &format!("notes/pack-{i:04}.md"),
        ).unwrap();
        let operation = match &content {
            Some(bytes) => RevisionOperation::Content {
                path, blob_hash: blake3::hash(bytes).to_hex().to_string(),
            },
            None => RevisionOperation::Tombstone { path },
        };
        let record = RevisionRecord {
            id, document_id: doc,
            parents: parent.into_iter().collect(),
            operation,
        };
        let json = serde_json::to_vec(&record).unwrap();
        (
            S3RevisionRefV1 {
                revision_id: id, document_id: doc,
                revision_blake3: blake3::hash(&json).to_hex().to_string(),
                blob_bytes: content.as_ref().map(|v| v.len() as u64),
            },
            json, content.unwrap_or_default(),
        )
    }

    #[test]
    fn sorted_260_members_split_into_three_exact_nonoverlapping_packs() {
        let ns = ns();
        let mut rows: Vec<_> = (0..260).map(|i| {
            fixture(i, DocumentId::new(), RevisionId::new(), None,
                if i == 3 { None }
                else { Some(format!("note {i}: verified content").into_bytes()) })
        }).collect();
        rows.reverse();
        let prepared = prepare_delta_packs_v2(&ns, &rows).unwrap();
        assert_eq!(prepared.references.len(), 260);
        assert_eq!(prepared.packs.len(), 3);
        let mut bytes = 0usize;
        for (index, pack) in prepared.packs.iter().enumerate() {
            let (start, count) = match index {
                0 => (0u16, 128u16),
                1 => (128u16, 128u16),
                _ => (256u16, 4u16),
            };
            assert_eq!(pack.index.first_revision_index, start);
            assert_eq!(pack.index.revision_count, count);
            assert_eq!(pack.index.byte_length as usize, pack.payload.len());
            assert_eq!(pack.index.pack_blake3, pack.descriptor.digest);
            assert_eq!(pack.descriptor.references,
                prepared.references[start as usize..(start+count) as usize]);
            bytes += pack.payload.len();
        }
        assert_eq!(prepared.packed_bytes, bytes);
        assert!(prepared.packed_bytes as u64 <= MAX_OFFLINE_PACK_BYTES);
        assert!(prepared.references.windows(2)
            .all(|w| w[0].revision_id < w[1].revision_id));
        let mut dag = RevisionDag::new();
        let mut seen = BTreeSet::new();
        for pack in &prepared.packs {
            ingest_verified_pack(
                &pack.payload, &pack.descriptor, &mut dag, &mut seen,
            ).unwrap();
        }
        assert_eq!(dag.active_len(), 260);
        assert_eq!(dag.pending_len(), 0);
        assert_eq!(seen.len(), 260);
    }

    #[test]
    fn forged_content_and_revision_identity_fail_before_returning_any_batch() {
        let ns = ns();
        let first = fixture(0, DocumentId::new(), RevisionId::new(), None,
            Some(b"verified".to_vec()));
        let mut tampered_blob = first.clone();
        tampered_blob.2 = b"altered!".to_vec();
        assert_eq!(prepare_delta_packs_v2(&ns, &[tampered_blob]).unwrap_err(),
            PackProbeError::IntegrityMismatch);

        let mut changed_document = first.clone();
        changed_document.0.document_id = DocumentId::new();
        assert_eq!(prepare_delta_packs_v2(&ns, &[changed_document]).unwrap_err(),
            PackProbeError::Invalid);

        let dupes = vec![first.clone(), first];
        assert_eq!(prepare_delta_packs_v2(&ns, &dupes).unwrap_err(),
            PackProbeError::DuplicateRevision);
    }

    #[test]
    fn strict_zero_oversize_and_byte_capacity_boundaries() {
        let ns = ns();
        assert_eq!(prepare_delta_packs_v2(&ns, &[]).unwrap_err(),
            PackProbeError::Invalid);
        let row = fixture(1, DocumentId::new(), RevisionId::new(), None,
            Some(b"x".to_vec()));
        assert_eq!(prepare_delta_packs_v2(&ns, &vec![row.clone(); 1025]).unwrap_err(),
            PackProbeError::TooLarge);
        let mut oversized = row;
        oversized.2 = vec![0; 32 * 1024 + 1];
        assert_eq!(prepare_delta_packs_v2(&ns, &[oversized]).unwrap_err(),
            PackProbeError::Invalid);
    }
}
