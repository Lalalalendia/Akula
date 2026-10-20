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


/// E3f: full, real HTTP/SigV4 S3 read of a disposable V2 object graph.
/// A test-only writer seeds the isolated MinIO bucket; the measured
/// `inspect_explicitly_pinned_v2_read_only` phase performs only GETs.
/// This is NOT Yandex provider identity, signed owner approval, or Sync.
#[tokio::test]
#[ignore = "requires disposable loopback MinIO; run explicitly in E3f CI"]
async fn live_minio_full_pinned_v2_get_and_corruption_hold() {
    use crate::sync::{
        OperationId, S3ConnectionProfile, S3ManifestShardV1, S3ManifestV1,
        S3PackIndexRootV2, S3PackIndexShardV2, S3RemoteAnchorScope,
        S3PutCondition, VaultId,
    };

    let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT")
        .expect("a disposable loopback MinIO endpoint is mandatory");
    assert!(
        endpoint.starts_with("http://127.0.0.1:")
            || endpoint.starts_with("http://localhost:"),
        "only an explicitly supplied loopback MinIO may be used"
    );
    let ns = S3Keyspace::new("akula/e3f-live-full-v2", VaultId::new()).unwrap();
    let profile = S3ConnectionProfile {
        bucket: "akula-e3f-v2-test".to_owned(),
        region: "us-east-1".to_owned(),
        endpoint: Some(endpoint),
        force_path_style: true,
    };
    let client = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
    // The ONLY network writer is the test fixture setup, against a throwaway
    // local bucket that cannot be selected by an end-user's saved S3 profile.
    client.client.create_bucket()
        .bucket(&profile.bucket).send().await.unwrap();
    let scope = S3RemoteAnchorScope::new(
        "e3f-ephemeral-profile", "local-test-only-minio-account",
        &profile, &ns,
    ).unwrap();

    let doc = crate::sync::DocumentId::new();
    let revision = RevisionId::new();
    let content = b"offline synthetic markdown: E3f";
    let record = RevisionRecord {
        id: revision, document_id: doc, parents: vec![],
        operation: RevisionOperation::Content {
            path: crate::sync::SyncRelativePath::parse("notes/e3f-test.md").unwrap(),
            blob_hash: blake3::hash(content).to_hex().to_string(),
        },
    };
    let record_json = serde_json::to_vec(&record).unwrap();
    let reference = S3RevisionRefV1 {
        revision_id: revision,
        document_id: doc,
        revision_blake3: blake3::hash(&record_json).to_hex().to_string(),
        blob_bytes: Some(content.len() as u64),
    };
    let mut pack_bytes = b"AKPKEXP1".to_vec();
    pack_bytes.extend_from_slice(&1_u16.to_le_bytes());
    pack_bytes.extend_from_slice(&(record_json.len() as u32).to_le_bytes());
    pack_bytes.extend_from_slice(&(content.len() as u32).to_le_bytes());
    pack_bytes.extend_from_slice(&record_json);
    pack_bytes.extend_from_slice(content);
    let pack_digest = blake3::hash(&pack_bytes).to_hex().to_string();
    let descriptor = S3PackDescriptorV2::from_lengths(
        pack_digest.clone(), vec![(reference.clone(), record_json.len() as u32)],
    ).unwrap();

    let manifest_shard = S3ManifestShardV1::new(
        &ns, OperationId::new(), vec![reference],
    ).unwrap();
    let manifest = S3ManifestV1::new(
        &ns, OperationId::new(), 1, None,
        vec![manifest_shard.reference(&ns).unwrap()],
    ).unwrap();
    let index_shard = S3PackIndexShardV2::new(
        &ns, OperationId::new(), vec![descriptor],
    ).unwrap();
    let index = S3PackIndexRootV2::new(
        &ns, OperationId::new(), 1,
        manifest.manifest_id, manifest.digest(&ns).unwrap(),
        vec![index_shard.reference(&ns).unwrap()],
    ).unwrap();
    let root = S3PackedHeadV2::new(
        &ns, 1, manifest.manifest_id, &manifest.digest(&ns).unwrap(),
        index.index_id, &index.digest(&ns).unwrap(), None,
    ).unwrap();

    for (key, data) in [
        (ns.immutable_blob_key(&pack_digest).unwrap(), pack_bytes.clone()),
        (ns.immutable_manifest_key(manifest_shard.shard_id),
            manifest_shard.encode(&ns).unwrap()),
        (ns.immutable_manifest_key(manifest.manifest_id),
            manifest.encode(&ns).unwrap()),
        (ns.immutable_manifest_key(index_shard.shard_id),
            index_shard.encode(&ns).unwrap()),
        (ns.immutable_manifest_key(index.index_id),
            index.encode(&ns).unwrap()),
    ] {
        client.put_conditionally(&key, &data, S3PutCondition::IfAbsent)
            .await.unwrap();
    }
    let root_bytes = root.encode(&ns).unwrap();
    let head_key = ns.current_head_key();
    let initial = client.put_conditionally(
        &head_key, &root_bytes, S3PutCondition::IfAbsent,
    ).await.unwrap();

    let before = client.test_io_snapshot();
    let inspected = client.inspect_explicitly_pinned_v2_read_only(
        &scope, &root, &initial.etag, None,
    ).await.unwrap();
    let counts = client.test_io_snapshot().since(before);
    assert_eq!(inspected.generation, 1);
    assert_eq!(inspected.verified_revisions, 1);
    assert_eq!(inspected.verified_packs, 1);
    assert_eq!(inspected.document_head_count, 1);
    assert!(inspected.attests_exact_root(&scope, &root, &initial.etag));
    assert!(counts.get >= 7, "must fetch root twice and all immutable objects");
    assert_eq!(counts.head, 0, "pinned V2 read must not LIST or HEAD by default");

    // Wrong ETag fails even when a cloud server returns the exact same bytes.
    let before_wrong = client.test_io_snapshot();
    assert!(matches!(
        client.inspect_explicitly_pinned_v2_read_only(
            &scope, &root, "\"\"wrong-etag\"\"", None,
        ).await,
        Err(S3PinnedV2ReadError::RootChanged)
    ));
    assert_eq!(client.test_io_snapshot().since(before_wrong).get, 1);

    // A rogue writer mutates a supposedly immutable pack, retaining the root.
    // Outer BLAKE3 must reject it even though S3 serves a successful GET.
    let pack_key = ns.immutable_blob_key(&pack_digest).unwrap();
    let old = client.head(&pack_key).await.unwrap();
    let mut corrupted = pack_bytes.clone();
    *corrupted.last_mut().unwrap() ^= 1;
    client.put_conditionally(
        &pack_key, &corrupted, S3PutCondition::IfMatch(old.etag),
    ).await.unwrap();
    assert!(matches!(
        client.inspect_explicitly_pinned_v2_read_only(
            &scope, &root, &initial.etag, None,
        ).await,
        Err(S3PinnedV2ReadError::InvalidPackPayload)
    ));

    // Finally, a replaced current-head object may not be accepted under the
    // previous independent pin, even if no manifest traversal is attempted.
    let mut altered = root_bytes.clone();
    altered.push(b' ');
    client.put_conditionally(
        &head_key, &altered, S3PutCondition::IfMatch(initial.etag.clone()),
    ).await.unwrap();
    assert!(matches!(
        client.inspect_explicitly_pinned_v2_read_only(
            &scope, &root, &initial.etag, None,
        ).await,
        Err(S3PinnedV2ReadError::RootChanged)
    ));
}
