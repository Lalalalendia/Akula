//! Real two-client/three-generation localhost MinIO acceptance of the
//! strictly read-only V1-pinned compact V2 offline letter + pack chain.
//! All buckets/vaults are disposable; no native Akula files are modified.

use super::*;
use super::super::{
    pack_probe::PackBuilder, S3ConnectionProfile, S3PutCondition,
};
use crate::sync::{
    DocumentId, OperationId, RevisionOperation, S3DeltaPackRefV2,
    S3Keyspace, S3ManifestShardV1, S3RevisionRefV1, SyncRelativePath,
    VaultId, MAX_S3_REVISION_METADATA_CACHE_BYTES,
};

type Item = (S3RevisionRefV1, Vec<u8>, Vec<u8>);

fn item(
    doc: DocumentId,
    id: RevisionId,
    parents: Vec<RevisionId>,
    content: Option<&[u8]>,
) -> Item {
    let blob = content.unwrap_or_default().to_vec();
    let operation = if let Some(bytes) = content {
        RevisionOperation::Content {
            path: SyncRelativePath::parse("notes/history.md").unwrap(),
            blob_hash: blake3::hash(bytes).to_hex().to_string(),
        }
    } else {
        RevisionOperation::Tombstone {
            path: SyncRelativePath::parse("notes/history.md").unwrap(),
        }
    };
    let json = serde_json::to_vec(&RevisionRecord {
        id, document_id: doc, parents, operation,
    }).unwrap();
    let reference = S3RevisionRefV1 {
        revision_id: id,
        document_id: doc,
        revision_blake3: blake3::hash(&json).to_hex().to_string(),
        blob_bytes: content.map(|bytes| bytes.len() as u64),
    };
    (reference, json, blob)
}

async fn upload_original(
    client: &AwsS3ProbeClient, ns: &S3Keyspace, record: &Item,
) {
    client.put_conditionally(
        &ns.immutable_revision_key(record.0.revision_id),
        &record.1, S3PutCondition::IfAbsent,
    ).await.unwrap();
    if record.0.blob_bytes.is_some() {
        let revision: RevisionRecord = serde_json::from_slice(&record.1).unwrap();
        let hash = match revision.operation {
            RevisionOperation::Content { blob_hash, .. }
            | RevisionOperation::Rename { blob_hash, .. } => blob_hash,
            _ => unreachable!(),
        };
        client.put_conditionally(
            &ns.immutable_blob_key(&hash).unwrap(),
            &record.2, S3PutCondition::IfAbsent,
        ).await.unwrap();
    }
}

async fn publish_manifest(
    client: &AwsS3ProbeClient, ns: &S3Keyspace,
    generation: u64, prior: Option<(OperationId,String)>,
    mut refs: Vec<S3RevisionRefV1>,
) -> S3ManifestV1 {
    refs.sort_by_key(|r| r.revision_id);
    let shard = S3ManifestShardV1::new(ns, OperationId::new(), refs).unwrap();
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

async fn publish_letter(
    client: &AwsS3ProbeClient, ns: &S3Keyspace,
    prior: &S3HeadPointerV1, current: &S3HeadPointerV1,
    items: &[Item],
) -> String {
    let mut builder = PackBuilder::new();
    for (r,json,blob) in items {
        builder.push(r.clone(), json, blob).unwrap();
    }
    let (refs, packs) = if items.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        let (descriptor, payload) = builder.seal(ns).unwrap();
        client.put_conditionally(
            &descriptor.key, &payload, S3PutCondition::IfAbsent,
        ).await.unwrap();
        (descriptor.references, vec![S3DeltaPackRefV2 {
            pack_blake3: descriptor.digest,
            byte_length: payload.len() as u32,
            first_revision_index: 0,
            revision_count: items.len() as u16,
        }])
    };
    let letter = S3DeltaEnvelopeV2::new(
        ns, prior, current, refs, packs,
    ).unwrap();
    let digest = letter.digest(ns).unwrap();
    client.put_conditionally(
        &ns.immutable_blob_key(&digest).unwrap(),
        &letter.encode(ns).unwrap(), S3PutCondition::IfAbsent,
    ).await.unwrap();
    digest
}

#[test]
fn chain_limits_are_strict_and_never_infer_an_empty_vault() {
    assert_eq!(MAX_OFFLINE_GENERATIONS, 8);
    assert_eq!(MAX_OFFLINE_PACK_BYTES, 16 * 1024 * 1024);
    assert!(MAX_OFFLINE_GENERATIONS <= S3ManifestDeltaLimits::default().max_frames);
}

#[tokio::test]
#[ignore = "explicit disposable localhost MinIO and no production File Gate"]
async fn live_minio_v2_two_offline_generations_retains_tombstone_and_rejects_gap() {
    let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").unwrap();
    assert!(endpoint.starts_with("http://127.0.0.1:")
        || endpoint.starts_with("http://localhost:"));
    let ns = S3Keyspace::new("akula/v2-offline-test", VaultId::new()).unwrap();
    let profile = S3ConnectionProfile {
        bucket: format!("akula-offline-v2-{}", uuid::Uuid::new_v4().simple()),
        region: "us-east-1".into(), endpoint: Some(endpoint),
        force_path_style: true,
    };
    let writer = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
    writer.client.create_bucket()
        .bucket(&writer.bucket).send().await.unwrap();
    let reader = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
    let scope = S3RemoteAnchorScope::new(
        "offline-v2-ci", "fully-verified-minio-identity", &profile, &ns,
    ).unwrap();

    let doc = DocumentId::new();
    let original_id = RevisionId::new();
    let original = item(doc, original_id, vec![], Some(b"# original"));
    upload_original(&writer, &ns, &original).await;
    let genesis = publish_manifest(
        &writer, &ns, 1, None, vec![original.0.clone()],
    ).await;
    let genesis_head = S3HeadPointerV1::new(
        &ns, 1, genesis.manifest_id, &genesis.digest(&ns).unwrap(), None,
    ).unwrap();
    let first_put = writer.put_conditionally(
        &ns.current_head_key(), &genesis_head.encode(&ns).unwrap(),
        S3PutCondition::IfAbsent,
    ).await.unwrap();

    // Reader independently full-pins genesis before going offline.
    let cache = reader.materialize_verified_revision_metadata_store(
        MAX_S3_REVISION_METADATA_CACHE_BYTES,
    ).await.unwrap().unwrap();
    let pinned = reader.pin_revision_snapshot().await.unwrap().unwrap();
    let previous_observation = reader.read_verified_head().await.unwrap().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let journal_path = temp.path().join("durable-anchor.sqlite");
    let mut journal = S3RemoteAnchorJournal::open(&journal_path).unwrap();
    journal.record_verified_remote(
        &scope, &previous_observation, &pinned,
        &[genesis.encode(&ns).unwrap()],
    ).unwrap();
    drop(journal);
    let journal = S3RemoteAnchorJournal::open(&journal_path).unwrap();

    // Gen2 creates an offline content sibling; Gen3 independently tombstones
    // original without deleting the sibling. No merge winner is invented.
    let changed = item(doc, RevisionId::new(),
        vec![original_id], Some(b"# concurrent content"));
    upload_original(&writer, &ns, &changed).await;
    let gen2 = publish_manifest(
        &writer, &ns, 2,
        Some((genesis.manifest_id, genesis.digest(&ns).unwrap())),
        vec![original.0.clone(), changed.0.clone()],
    ).await;
    let head2 = S3HeadPointerV1::new(
        &ns, 2, gen2.manifest_id, &gen2.digest(&ns).unwrap(),
        Some(genesis.manifest_id),
    ).unwrap();
    let second_put = writer.put_conditionally(
        &ns.current_head_key(), &head2.encode(&ns).unwrap(),
        S3PutCondition::IfMatch(first_put.etag),
    ).await.unwrap();
    let letter2 = publish_letter(
        &writer, &ns, &genesis_head, &head2, &[changed.clone()],
    ).await;

    let tombstone = item(doc, RevisionId::new(), vec![original_id], None);
    upload_original(&writer, &ns, &tombstone).await;
    let gen3 = publish_manifest(
        &writer, &ns, 3,
        Some((gen2.manifest_id, gen2.digest(&ns).unwrap())),
        vec![original.0.clone(), changed.0.clone(), tombstone.0.clone()],
    ).await;
    let head3 = S3HeadPointerV1::new(
        &ns, 3, gen3.manifest_id, &gen3.digest(&ns).unwrap(),
        Some(gen2.manifest_id),
    ).unwrap();
    let third_put = writer.put_conditionally(
        &ns.current_head_key(), &head3.encode(&ns).unwrap(),
        S3PutCondition::IfMatch(second_put.etag),
    ).await.unwrap();
    let letter3 = publish_letter(
        &writer, &ns, &head2, &head3, &[tombstone.clone()],
    ).await;

    let letters = vec![letter2.clone(), letter3.clone()];
    let accepted = reader.inspect_v2_offline_chain_against_pinned_v1(
        &scope, &journal, &pinned, &cache, &letters,
    ).await.unwrap();
    assert_eq!(accepted.from_generation, 1);
    assert_eq!(accepted.to_generation, 3);
    assert_eq!(accepted.verified_generations, 2);
    assert_eq!(accepted.newly_verified_revisions, 2);
    assert_eq!(accepted.newly_verified_packs, 2);
    assert_eq!(accepted.new_revision_ids.len(), 2);
    assert!(accepted.new_revision_ids.contains(&changed.0.revision_id));
    assert!(accepted.new_revision_ids.contains(&tombstone.0.revision_id));
    assert!(accepted.requires_full_apply_gate);

    // No missing/reordered generation may silently become an empty delta.
    assert!(reader.inspect_v2_offline_chain_against_pinned_v1(
        &scope, &journal, &pinned, &cache, &[letter2.clone()],
    ).await.is_err());
    assert!(reader.inspect_v2_offline_chain_against_pinned_v1(
        &scope, &journal, &pinned, &cache, &[letter3.clone(),letter2.clone()],
    ).await.is_err());

    // Gen4 is a valid-looking, self-consistent V1 DAG but *silently drops*
    // the earlier tombstone. A later no-op sidecar must NOT conceal it.
    let gen4 = publish_manifest(
        &writer, &ns, 4,
        Some((gen3.manifest_id, gen3.digest(&ns).unwrap())),
        vec![original.0.clone(), changed.0.clone()],
    ).await;
    let head4 = S3HeadPointerV1::new(
        &ns, 4, gen4.manifest_id, &gen4.digest(&ns).unwrap(),
        Some(gen3.manifest_id),
    ).unwrap();
    writer.put_conditionally(
        &ns.current_head_key(), &head4.encode(&ns).unwrap(),
        S3PutCondition::IfMatch(third_put.etag),
    ).await.unwrap();
    let no_change = publish_letter(&writer, &ns, &head3, &head4, &[]).await;
    let mut forged = letters.clone();
    forged.push(no_change);
    assert!(reader.inspect_v2_offline_chain_against_pinned_v1(
        &scope, &journal, &pinned, &cache, &forged,
    ).await.is_err());
    assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &genesis_head);
}
