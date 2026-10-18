//! Offline negative checks for the production-compiled V2 member verifier.

use super::*;
use crate::sync::{DocumentId, S3PackMemberV2, SyncRelativePath};

fn fixture() -> (S3PackDescriptorV2, Vec<u8>, [RevisionId; 4], DocumentId) {
    let document = DocumentId::new();
    let ids = [
        RevisionId::new(), RevisionId::new(),
        RevisionId::new(), RevisionId::new(),
    ];
    let mut bytes = b"AKPKEXP1".to_vec();
    bytes.extend_from_slice(&4_u16.to_le_bytes());
    let mut members = Vec::new();
    for (id, parents, content) in [
        (ids[0], vec![], Some(&b"initial"[..])),
        (ids[1], vec![ids[0]], Some(&b"left branch"[..])),
        (ids[2], vec![ids[0]], Some(&b"right branch"[..])),
        (ids[3], vec![ids[0]], None),
    ] {
        let operation = match content {
            Some(body) => RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/branch.md").unwrap(),
                blob_hash: blake3::hash(body).to_hex().to_string(),
            },
            None => RevisionOperation::Tombstone {
                path: SyncRelativePath::parse("notes/branch.md").unwrap(),
            },
        };
        let record = RevisionRecord {
            id, document_id: document, parents, operation,
        };
        let json = serde_json::to_vec(&record).unwrap();
        let blob = content.unwrap_or_default();
        bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&json);
        bytes.extend_from_slice(blob);
        members.push((
            S3RevisionRefV1 {
                revision_id: id, document_id: document,
                revision_blake3: blake3::hash(&json).to_hex().to_string(),
                blob_bytes: content.map(|x| x.len() as u64),
            },
            json.len() as u32,
        ));
    }
    let desc = S3PackDescriptorV2::from_lengths(
        blake3::hash(&bytes).to_hex().to_string(), members,
    ).unwrap();
    assert_eq!(desc.pack_bytes as usize, bytes.len());
    (desc, bytes, ids, document)
}

#[test]
fn verified_v2_pack_preserves_sibling_edits_and_tombstone_head() {
    let (desc, bytes, ids, doc) = fixture();
    let mut dag = RevisionDag::new();
    let mut seen = BTreeSet::new();
    verify_packed_members(&bytes, &desc, &mut dag, &mut seen).unwrap();
    assert_eq!(dag.active_len(), 4);
    assert_eq!(dag.pending_len(), 0);
    assert_eq!(seen.len(), 4);
    assert_eq!(dag.heads(doc), BTreeSet::from([ids[1], ids[2], ids[3]]));
    // An identical second pack must NOT silently count as another revision.
    assert!(matches!(
        verify_packed_members(&bytes, &desc, &mut dag, &mut seen),
        Err(S3PinnedV2ReadError::InvalidRevision)
    ));
}

#[test]
fn independently_rehashed_corrupt_blob_remains_rejected() {
    let (mut desc, mut bytes, _, _) = fixture();
    let pos = bytes.windows(b"left branch".len())
        .position(|b| b == b"left branch").unwrap();
    bytes[pos] = b'X';
    desc.pack_blake3 = blake3::hash(&bytes).to_hex().to_string();
    assert!(matches!(
        verify_packed_members(
            &bytes, &desc, &mut RevisionDag::new(), &mut BTreeSet::new(),
        ), Err(S3PinnedV2ReadError::InvalidPackPayload)
    ));
}

#[test]
fn offsets_trailing_bytes_and_foreign_identity_are_held() {
    let (desc, bytes, _ids, _) = fixture();
    let mut incorrect = desc.clone();
    incorrect.members[1].revision_offset += 1;
    assert!(matches!(
        verify_packed_members(
            &bytes, &incorrect, &mut RevisionDag::new(), &mut BTreeSet::new(),
        ), Err(S3PinnedV2ReadError::InvalidPackPayload)
    ));
    let mut extra = bytes.clone();
    extra.push(b'!');
    incorrect = desc.clone();
    incorrect.pack_blake3 = blake3::hash(&extra).to_hex().to_string();
    assert!(matches!(
        verify_packed_members(
            &extra, &incorrect, &mut RevisionDag::new(), &mut BTreeSet::new(),
        ), Err(S3PinnedV2ReadError::InvalidPackPayload)
    ));
    let mut wrong = desc;
    wrong.members[0].reference.document_id = DocumentId::new();
    assert!(matches!(
        verify_packed_members(
            &bytes, &wrong, &mut RevisionDag::new(), &mut BTreeSet::new(),
        ), Err(S3PinnedV2ReadError::InvalidRevision)
    ));
}

#[test]
fn manipulated_tombstone_cannot_gain_a_content_blob() {
    let (desc, bytes, _ids, _) = fixture();
    let mut changed = desc;
    let tombstone: &mut S3PackMemberV2 = changed.members.last_mut().unwrap();
    tombstone.reference.blob_bytes = Some(0);
    tombstone.blob_offset = Some(tombstone.revision_offset + tombstone.revision_len);
    assert!(matches!(
        verify_packed_members(
            &bytes, &changed, &mut RevisionDag::new(), &mut BTreeSet::new(),
        ), Err(S3PinnedV2ReadError::InvalidPackPayload)
    ));
}
