//! Real two-client/three-generation localhost MinIO acceptance of the
//! strictly read-only V1-pinned compact V2 offline letter + pack chain.
//! All buckets/vaults are disposable; no native Akula files are modified.

use super::*;
use std::collections::BTreeSet;
use std::time::Instant;
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
    // First measure the conservative reference: every missed V1 generation
    // is fully pinned before any V2 letter/pack is accepted.
    let chain_before = reader.test_io_snapshot();
    let chain_started = Instant::now();
    let accepted = reader.inspect_v2_offline_chain_against_pinned_v1(
        &scope, &journal, &pinned, &cache, &letters,
    ).await.unwrap();
    let chain_micros = chain_started.elapsed().as_micros();
    let chain_io = reader.test_io_snapshot().since(chain_before);
    assert_eq!(accepted.from_generation, 1);
    assert_eq!(accepted.to_generation, 3);
    assert_eq!(accepted.verified_generations, 2);
    assert_eq!(accepted.newly_verified_revisions, 2);
    assert_eq!(accepted.newly_verified_packs, 2);
    assert_eq!(accepted.new_revision_ids.len(), 2);
    assert!(accepted.new_revision_ids.contains(&changed.0.revision_id));
    assert!(accepted.new_revision_ids.contains(&tombstone.0.revision_id));
    assert!(accepted.requires_full_apply_gate);

    // 02e optimized reader: reconstruct each real immutable V1 shard
    // inventory and verify new revision JSON/blob bytes from the V2 packs,
    // without repeatedly loading prior individual objects.
    let packed_before = reader.test_io_snapshot();
    let packed_started = Instant::now();
    let packed = reader.inspect_v2_offline_selective_packs_against_pinned_v1(
        &scope, &journal, &pinned, &cache, &letters,
    ).await.unwrap();
    let packed_micros = packed_started.elapsed().as_micros();
    let packed_io = reader.test_io_snapshot().since(packed_before);
    assert_eq!(packed.from_generation, 1);
    assert_eq!(packed.to_generation, 3);
    assert_eq!(packed.verified_generations, 2);
    assert_eq!(packed.new_revision_ids.len(), 2);
    assert_eq!(packed.verified_packs, 2);
    assert_eq!(packed.total_revisions, 3);
    assert!(packed.requires_full_apply_gate);

    // 02d shadow experiment: reuse the SAME real MinIO root and durable
    // SQLite anchor, but compare the already-existing selective V1 advisory
    // reader against the independently full-verified V2 letter/pack chain.
    // Neither preview can authorize local file writes or a new baseline.
    let selective_before = reader.test_io_snapshot();
    let selective_started = Instant::now();
    let selected = reader.inspect_selective_s3_revision_changes(
        &scope, &journal, &cache,
        MAX_S3_REVISION_METADATA_CACHE_BYTES,
        S3ManifestDeltaLimits {
            max_frames: MAX_OFFLINE_GENERATIONS,
            max_total_shard_refs: MAX_OFFLINE_GENERATIONS * 256,
        },
    ).await.unwrap();
    let selective_micros = selective_started.elapsed().as_micros();
    let selective_io = reader.test_io_snapshot().since(selective_before);

    // Separately re-pin the latest complete V1 inventory as a third
    // reference. The selective/packed modes are NOT interchangeable apply
    // receipts: they reuse prior verified blobs and have weaker availability
    // guarantees than a fresh full remote pin.
    let full_before = reader.test_io_snapshot();
    let full_started = Instant::now();
    let full = reader.pin_revision_snapshot().await.unwrap().unwrap();
    let full_micros = full_started.elapsed().as_micros();
    let full_io = reader.test_io_snapshot().since(full_before);

    let chain_new: BTreeSet<_> =
        accepted.new_revision_ids.iter().copied().collect();
    let selective_new: BTreeSet<_> =
        selected.new_revision_ids.iter().copied().collect();
    let packed_new: BTreeSet<_> =
        packed.new_revision_ids.iter().copied().collect();
    assert_eq!(chain_new, packed_new, "packed reader omitted offline revision");
    assert_eq!(packed.total_revisions, accepted.newly_verified_revisions + pinned.len());
    let old_ids: BTreeSet<_> = pinned.references_for_coord_v2()
        .iter().map(|r| r.revision_id).collect();
    let full_ids: BTreeSet<_> = full.references_for_coord_v2()
        .iter().map(|r| r.revision_id).collect();
    let expected_full: BTreeSet<_> =
        old_ids.union(&chain_new).copied().collect();
    assert_eq!(chain_new.len(), accepted.new_revision_ids.len());
    assert!(old_ids.is_disjoint(&chain_new));
    assert_eq!(chain_new, selective_new, "selective reader lost an offline branch");
    assert_eq!(expected_full, full_ids, "full V1 inventory diverged");
    assert_eq!(selected.remote_snapshot_id, full.snapshot_id);
    assert_eq!(selected.total_revisions, full.len());
    assert!(selected.requires_full_apply_gate);

    // 02e differs from 02c in assurance: old blobs are NOT re-probed.
    println!("AKULA_COORD_02E_PARITY {}", serde_json::json!({
        "assurance": "advisory-shard-and-pack-membership-no-old-blob-availability",
        "conservative_chain": {
            "get": chain_io.get, "head": chain_io.head,
            "bytes": chain_io.payload_bytes, "elapsed_us": chain_micros
        },
        "selective_packs": {
            "get": packed_io.get, "head": packed_io.head,
            "bytes": packed_io.payload_bytes, "elapsed_us": packed_micros,
            "downloaded_shards": packed.downloaded_shards,
            "reused_shards": packed.reused_shards,
            "verified_packs": packed.verified_packs,
            "checked_pack_bytes": packed.checked_pack_bytes
        },
        "selective_v1": {
            "get": selective_io.get, "head": selective_io.head,
            "bytes": selective_io.payload_bytes, "elapsed_us": selective_micros
        },
        "full_current_v1": {
            "get": full_io.get, "head": full_io.head,
            "bytes": full_io.payload_bytes, "elapsed_us": full_micros
        },
        "new_revisions": chain_new.len(),
        "total_revisions": full.len(),
        "tombstone_and_sibling_retained": true,
        "requires_full_apply_gate": packed.requires_full_apply_gate
    }));

    // Machine-readable AWS SDK method attempts and received bytes, not
    // provider HTTP/TCP request counts. The three modes intentionally offer
    // DIFFERENT assurance; no speedup for full file sync is claimed here.
    println!("AKULA_COORD_02D_PARITY {}", serde_json::json!({
        "assurance": "read-only-shared-v1-anchor-no-file-gate",
        "from_generation": accepted.from_generation,
        "to_generation": accepted.to_generation,
        "total_revisions": full.len(),
        "new_revisions": chain_new.len(),
        "tombstone_and_sibling_retained": true,
        "conservative_chain": {
            "get": chain_io.get,
            "head": chain_io.head,
            "bytes": chain_io.payload_bytes,
            "elapsed_us": chain_micros
        },
        "selective_v1": {
            "get": selective_io.get,
            "head": selective_io.head,
            "bytes": selective_io.payload_bytes,
            "elapsed_us": selective_micros,
            "reused_revision_json": selected.reused_revision_json,
            "downloaded_revision_json": selected.downloaded_revision_json
        },
        "full_current_v1": {
            "get": full_io.get,
            "head": full_io.head,
            "bytes": full_io.payload_bytes,
            "elapsed_us": full_micros
        }
    }));

    // No missing/reordered generation may silently become an empty delta.
    assert!(reader.inspect_v2_offline_chain_against_pinned_v1(
        &scope, &journal, &pinned, &cache, &[letter2.clone()],
    ).await.is_err());
    assert!(reader.inspect_v2_offline_chain_against_pinned_v1(
        &scope, &journal, &pinned, &cache, &[letter3.clone(),letter2.clone()],
    ).await.is_err());

    // The optimized V2 pack reader must reject the same missing/reversed
    // immutable letters; never use the final manifest as a shortcut.
    assert!(reader.inspect_v2_offline_selective_packs_against_pinned_v1(
        &scope, &journal, &pinned, &cache, &[letter2.clone()],
    ).await.is_err());
    assert!(reader.inspect_v2_offline_selective_packs_against_pinned_v1(
        &scope, &journal, &pinned, &cache,
        &[letter3.clone(), letter2.clone()],
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
    // 02e must also HOLD when a later valid-looking generation hides an
    // earlier tombstone. The saved durable root remains unchanged.
    assert!(reader.inspect_v2_offline_selective_packs_against_pinned_v1(
        &scope, &journal, &pinned, &cache, &forged,
    ).await.is_err());
    // The selective reader must also inspect the full immutable history:
    // a plausible final root that silently drops the gen3 tombstone is HOLD.
    assert!(reader.inspect_selective_s3_revision_changes(
        &scope, &journal, &cache,
        MAX_S3_REVISION_METADATA_CACHE_BYTES,
        S3ManifestDeltaLimits {
            max_frames: MAX_OFFLINE_GENERATIONS,
            max_total_shard_refs: MAX_OFFLINE_GENERATIONS * 256,
        },
    ).await.is_err());
    assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &genesis_head);
}
