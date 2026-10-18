//! Real disposable MinIO tests: V1 full pin -> V2 letter/pack sidecar.
//! No installed S3 credentials, user vault writes or automatic publication.

use super::*;
use super::super::{
    pack_probe::PackBuilder, S3ConnectionProfile, S3PutCondition,
};
use crate::sync::{
    DocumentId, OperationId, RevisionOperation, S3DeltaPackRefV2,
    S3Keyspace, S3ManifestShardV1, S3ManifestV1, S3RevisionRefV1,
    SyncRelativePath, VaultId, MAX_S3_REVISION_METADATA_CACHE_BYTES,
};

fn item(
    doc: DocumentId,
    id: RevisionId,
    parents: Vec<RevisionId>,
    content: Option<&[u8]>,
) -> (S3RevisionRefV1, Vec<u8>, Vec<u8>) {
    let blob = content.unwrap_or_default().to_vec();
    let operation = if let Some(bytes) = content {
        RevisionOperation::Content {
            path: SyncRelativePath::parse("notes/offline.md").unwrap(),
            blob_hash: blake3::hash(bytes).to_hex().to_string(),
        }
    } else {
        RevisionOperation::Tombstone {
            path: SyncRelativePath::parse("notes/offline.md").unwrap(),
        }
    };
    let record = RevisionRecord {
        id, document_id: doc, parents, operation,
    };
    let json = serde_json::to_vec(&record).unwrap();
    let reference = S3RevisionRefV1 {
        revision_id: id,
        document_id: doc,
        revision_blake3: blake3::hash(&json).to_hex().to_string(),
        blob_bytes: content.map(|bytes| bytes.len() as u64),
    };
    (reference, json, blob)
}

async fn upload(
    client: &AwsS3ProbeClient,
    ns: &S3Keyspace,
    record: &(S3RevisionRefV1, Vec<u8>, Vec<u8>),
) {
    client.put_conditionally(
        &ns.immutable_revision_key(record.0.revision_id),
        &record.1,
        S3PutCondition::IfAbsent,
    ).await.unwrap();
    if record.0.blob_bytes.is_some() {
        let json: RevisionRecord = serde_json::from_slice(&record.1).unwrap();
        let hash = match json.operation {
            RevisionOperation::Content { blob_hash, .. }
            | RevisionOperation::Rename { blob_hash, .. } => blob_hash,
            _ => unreachable!(),
        };
        client.put_conditionally(
            &ns.immutable_blob_key(&hash).unwrap(),
            &record.2,
            S3PutCondition::IfAbsent,
        ).await.unwrap();
    }
}

async fn put_shard_root(
    client: &AwsS3ProbeClient,
    ns: &S3Keyspace,
    generation: u64,
    prior: Option<(OperationId, String)>,
    refs: Vec<S3RevisionRefV1>,
) -> S3ManifestV1 {
    let mut refs = refs;
    refs.sort_by_key(|x| x.revision_id);
    let shard = S3ManifestShardV1::new(
        ns, OperationId::new(), refs,
    ).unwrap();
    client.put_conditionally(
        &ns.immutable_manifest_key(shard.shard_id),
        &shard.encode(ns).unwrap(), S3PutCondition::IfAbsent,
    ).await.unwrap();
    let root = S3ManifestV1::new(
        ns, OperationId::new(), generation, prior,
        vec![shard.reference(ns).unwrap()],
    ).unwrap();
    client.put_conditionally(
        &ns.immutable_manifest_key(root.manifest_id),
        &root.encode(ns).unwrap(), S3PutCondition::IfAbsent,
    ).await.unwrap();
    root
}

#[tokio::test]
#[ignore = "explicit disposable localhost MinIO and no production File Gate"]
async fn live_minio_v2_sidecar_two_clients_and_corrupt_pack_fail_closed() {
    let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").unwrap();
    assert!(
        endpoint.starts_with("http://127.0.0.1:")
            || endpoint.starts_with("http://localhost:"),
        "never use a real S3 endpoint for this test"
    );
    let ns = S3Keyspace::new("akula/v2-sidecar-test", VaultId::new()).unwrap();
    let profile = S3ConnectionProfile {
        bucket: format!("akula-sidecar-{}", uuid::Uuid::new_v4().simple()),
        region: "us-east-1".into(), endpoint: Some(endpoint),
        force_path_style: true,
    };
    let sender = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
    sender.client.create_bucket()
        .bucket(&sender.bucket).send().await.unwrap();
    let reader = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
    let scope = S3RemoteAnchorScope::new(
        "coord-sidecar-ci", "verified-minio-identity", &profile, &ns,
    ).unwrap();

    let doc = DocumentId::new();
    let base_id = RevisionId::new();
    let base = item(doc, base_id, vec![], Some(b"# stable genesis"));
    upload(&sender, &ns, &base).await;
    let old_root = put_shard_root(
        &sender, &ns, 1, None, vec![base.0.clone()],
    ).await;
    let old_head = S3HeadPointerV1::new(
        &ns, 1, old_root.manifest_id, &old_root.digest(&ns).unwrap(), None,
    ).unwrap();
    let old_put = sender.put_conditionally(
        &ns.current_head_key(), &old_head.encode(&ns).unwrap(),
        S3PutCondition::IfAbsent,
    ).await.unwrap();

    // Consumer B independently fully verifies the remote V1 genesis.
    let cache = reader.materialize_verified_revision_metadata_store(
        MAX_S3_REVISION_METADATA_CACHE_BYTES,
    ).await.unwrap().unwrap();
    let prior_verified = reader.read_verified_head().await.unwrap().unwrap();
    let old_pin = reader.pin_revision_snapshot().await.unwrap().unwrap();
    assert_eq!(old_pin.len(), 1);
    assert_eq!(cache.len(), 1);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("private-remote-anchor.sqlite");
    let mut journal = S3RemoteAnchorJournal::open(&path).unwrap();
    journal.record_verified_remote(
        &scope, &prior_verified, &old_pin, &[old_root.encode(&ns).unwrap()],
    ).unwrap();

    // Independent sender creates two concurrent offline sibling revisions:
    // one content change and one tombstone, both must remain in the DAG.
    let changed = item(
        doc, RevisionId::new(), vec![base_id], Some(b"# offline edit"),
    );
    let deleted = item(doc, RevisionId::new(), vec![base_id], None);
    let mut changes = vec![changed, deleted];
    changes.sort_by_key(|x| x.0.revision_id);
    for change in &changes {
        upload(&sender, &ns, change).await;
    }

    // Sidecar pack and V1 independent JSON/blobs are both seeded. This test
    // proves safe reading, NOT yet reduced S3 PUTs or a v2 native publisher.
    let mut pack_builder = PackBuilder::new();
    for entry in &changes {
        pack_builder.push(
            entry.0.clone(), &entry.1, &entry.2,
        ).unwrap();
    }
    let (pack_desc, pack_bytes) = pack_builder.seal(&ns).unwrap();
    sender.put_conditionally(
        &pack_desc.key, &pack_bytes, S3PutCondition::IfAbsent,
    ).await.unwrap();

    let mut refs = vec![base.0.clone()];
    refs.extend(changes.iter().map(|r|r.0.clone()));
    let newer_root = put_shard_root(
        &sender, &ns, 2,
        Some((old_root.manifest_id, old_root.digest(&ns).unwrap())),
        refs,
    ).await;
    let newer_head = S3HeadPointerV1::new(
        &ns, 2, newer_root.manifest_id, &newer_root.digest(&ns).unwrap(),
        Some(old_root.manifest_id),
    ).unwrap();
    let new_put = sender.put_conditionally(
        &ns.current_head_key(), &newer_head.encode(&ns).unwrap(),
        S3PutCondition::IfMatch(old_put.etag),
    ).await.unwrap();

    let new_refs: Vec<S3RevisionRefV1> =
        changes.iter().map(|x| x.0.clone()).collect();
    let mut pack_ref = S3DeltaPackRefV2 {
        pack_blake3: pack_desc.digest.clone(),
        byte_length: pack_bytes.len() as u32,
        first_revision_index: 0,
        revision_count: 2,
    };
    let envelope = S3DeltaEnvelopeV2::new(
        &ns, &old_head, &newer_head,
        new_refs.clone(), vec![pack_ref.clone()],
    ).unwrap();
    let letter_bytes = envelope.encode(&ns).unwrap();
    let letter_digest = envelope.digest(&ns).unwrap();
    sender.put_conditionally(
        &ns.immutable_blob_key(&letter_digest).unwrap(),
        &letter_bytes, S3PutCondition::IfAbsent,
    ).await.unwrap();

    let checked = reader.inspect_v2_sidecar_against_pinned_v1(
        &scope, &journal, &old_pin, &cache, &letter_digest,
    ).await.unwrap();
    assert_eq!(checked.generation, 2);
    assert_eq!(checked.newly_verified_revisions, 2);
    assert_eq!(checked.newly_verified_packs, 1);
    assert_eq!(checked.checked_pack_bytes, pack_bytes.len() as u64);
    assert_eq!(checked.new_revision_ids.len(), 2);
    assert!(checked.requires_full_apply_gate);

    // A forged, perfectly well-formed letter naming an absent pack must
    // never be accepted as an empty remote result.
    pack_ref.pack_blake3 = blake3::hash(b"unavailable-pack").to_hex().to_string();
    let absent_letter = S3DeltaEnvelopeV2::new(
        &ns, &old_head, &newer_head,
        new_refs.clone(), vec![pack_ref.clone()],
    ).unwrap();
    let absent_hash = absent_letter.digest(&ns).unwrap();
    sender.put_conditionally(
        &ns.immutable_blob_key(&absent_hash).unwrap(),
        &absent_letter.encode(&ns).unwrap(),
        S3PutCondition::IfAbsent,
    ).await.unwrap();
    assert!(matches!(
        reader.inspect_v2_sidecar_against_pinned_v1(
            &scope, &journal, &old_pin, &cache, &absent_hash,
        ).await,
        Err(S3CoordSidecarReadError::Transport(S3TransportError::NotFound))
    ));

    // More devious: an attacker recomputes outer pack and letter hashes, but
    // the packed member's original content BLAKE3 no longer matches V1.
    let mut damaged = pack_bytes.clone();
    *damaged.last_mut().unwrap() ^= 1;
    pack_ref.pack_blake3 = blake3::hash(&damaged).to_hex().to_string();
    sender.put_conditionally(
        &ns.immutable_blob_key(&pack_ref.pack_blake3).unwrap(),
        &damaged, S3PutCondition::IfAbsent,
    ).await.unwrap();
    let tampered = S3DeltaEnvelopeV2::new(
        &ns, &old_head, &newer_head,
        new_refs, vec![pack_ref],
    ).unwrap();
    let tampered_hash = tampered.digest(&ns).unwrap();
    sender.put_conditionally(
        &ns.immutable_blob_key(&tampered_hash).unwrap(),
        &tampered.encode(&ns).unwrap(),
        S3PutCondition::IfAbsent,
    ).await.unwrap();
    assert!(matches!(
        reader.inspect_v2_sidecar_against_pinned_v1(
            &scope, &journal, &old_pin, &cache, &tampered_hash,
        ).await,
        Err(S3CoordSidecarReadError::InvalidPackPayload)
    ));

    // A remote generation jump also invalidates the old one-step letter,
    // even if all old revision refs are still present.
    let mut later_refs = vec![base.0.clone()];
    later_refs.extend(changes.iter().map(|x|x.0.clone()));
    let gen3 = put_shard_root(
        &sender, &ns, 3,
        Some((newer_root.manifest_id, newer_root.digest(&ns).unwrap())),
        later_refs,
    ).await;
    let head3 = S3HeadPointerV1::new(
        &ns, 3, gen3.manifest_id, &gen3.digest(&ns).unwrap(),
        Some(newer_root.manifest_id),
    ).unwrap();
    sender.put_conditionally(
        &ns.current_head_key(), &head3.encode(&ns).unwrap(),
        S3PutCondition::IfMatch(new_put.etag),
    ).await.unwrap();
    assert!(reader.inspect_v2_sidecar_against_pinned_v1(
        &scope, &journal, &old_pin, &cache, &letter_digest,
    ).await.is_err());
    assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &old_head);
}
