//! SYNC-05d2e: **manual** full-DAG MinIO scale test.
//!
//! Unlike the 10k-key LIST diagnostic, this creates 10,000 individually
//! pinned revision JSON objects AND 10,000 content blobs, then passes the
//! entire graph through the actual production verification path.
//! No user vault paths, real AWS credentials, GC, local filesystem writes,
//! auto-sync or acknowledgement of a durable baseline are involved.
//!
//! Kept out of the standard PR MinIO workflow: run explicitly in the
//! dedicated full-DAG scale workflow using disposable localhost MinIO.

use super::{AwsS3ProbeClient, S3ConnectionProfile, S3PutCondition};
use crate::sync::{
    DocumentId, OperationId, RevisionId, RevisionOperation, RevisionRecord,
    S3HeadPointerV1, S3Keyspace, S3ManifestShardV1, S3ManifestV1,
    S3RevisionRefV1, SyncRelativePath, VaultId, MAX_SHARD_ENTRIES,
};
use std::time::Instant;

const DOCUMENTS: usize = 2_500;
const REVISIONS_PER_DOCUMENT: usize = 4;
const REVISION_COUNT: usize = DOCUMENTS * REVISIONS_PER_DOCUMENT;
const PAYLOAD_BYTES: usize = 1_024;
const MAX_PARALLEL_UPLOADS: usize = 32;

/// 2,500 documents, each with one root, two sibling offline edits, and
/// either an explicit merge or a third concurrent edit. All revision JSONs
/// and all associated 1 KiB blobs are unique and uploaded conditionally.
///
/// A generation-2 root reuses the same 79 immutable shard references:
/// read_verified_head must verify ALL 10k revision JSON + 10k blobs, then
/// authenticate gen1 ancestry *without* redownloading the old blob bytes.
/// Instrumentation reports SDK method attempts, not TCP/TLS byte counts.
#[tokio::test]
#[ignore = "explicit opt-in disposable localhost MinIO 10k full DAG scale"]
async fn minio_full_10k_revision_dag_and_blob_readback() {
    let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT")
        .expect("localhost-only test MinIO endpoint required");
    assert!(
        endpoint.starts_with("http://127.0.0.1:")
            || endpoint.starts_with("http://localhost:"),
        "full-DAG load may only target the disposable loopback MinIO"
    );
    let ns = S3Keyspace::new(
        "akula-sync/full-dag-10k", VaultId::new(),
    ).unwrap();
    let profile = S3ConnectionProfile {
        bucket: "akula-10k-full-revision-dag".into(),
        region: "us-east-1".into(),
        endpoint: Some(endpoint),
        force_path_style: true,
    };
    let client = AwsS3ProbeClient::connect(profile, ns.clone()).await.unwrap();
    client.client.create_bucket()
        .bucket(&client.bucket).send().await.unwrap();

    let upload_started = Instant::now();
    let mut tasks = tokio::task::JoinSet::new();
    let mut references = Vec::with_capacity(REVISION_COUNT);
    let mut sibling_example: Option<(DocumentId, [RevisionId; 4])> = None;
    for doc_index in 0..DOCUMENTS {
        let document_id = DocumentId::new();
        let ids = [
            RevisionId::new(), RevisionId::new(),
            RevisionId::new(), RevisionId::new(),
        ];
        if doc_index == 1 {
            sibling_example = Some((document_id, ids));
        }
        let relative_path = SyncRelativePath::parse(
            &format!("notes/full-dag-{doc_index:04}.md"),
        ).unwrap();
        for edition in 0..REVISIONS_PER_DOCUMENT {
            let label = format!(
                "Akula full DAG / document {doc_index:04} / revision {edition}"
            );
            let mut blob = vec![b'.'; PAYLOAD_BYTES];
            blob[..label.len()].copy_from_slice(label.as_bytes());
            let blob_hash = blake3::hash(&blob).to_hex().to_string();
            let parents = match edition {
                0 => Vec::new(),
                1 | 2 => vec![ids[0]],
                3 if doc_index % 2 == 0 => vec![ids[1], ids[2]],
                _ => vec![ids[0]],
            };
            let record = RevisionRecord {
                id: ids[edition],
                document_id,
                parents,
                operation: RevisionOperation::Content {
                    path: relative_path.clone(),
                    blob_hash: blob_hash.clone(),
                },
            };
            let revision_json = serde_json::to_vec(&record).unwrap();
            references.push(S3RevisionRefV1 {
                revision_id: record.id,
                document_id,
                revision_blake3: blake3::hash(&revision_json)
                    .to_hex().to_string(),
                blob_bytes: Some(blob.len() as u64),
            });
            let worker = client.clone();
            let blob_key = ns.immutable_blob_key(&blob_hash).unwrap();
            let revision_key = ns.immutable_revision_key(record.id);
            tasks.spawn(async move {
                worker.put_conditionally(
                    &blob_key, &blob, S3PutCondition::IfAbsent,
                ).await.unwrap();
                worker.put_conditionally(
                    &revision_key, &revision_json, S3PutCondition::IfAbsent,
                ).await.unwrap();
            });
            if tasks.len() >= MAX_PARALLEL_UPLOADS {
                tasks.join_next().await.unwrap().unwrap();
            }
        }
    }
    while let Some(done) = tasks.join_next().await {
        done.unwrap();
    }
    let upload_elapsed = upload_started.elapsed();
    assert_eq!(references.len(), REVISION_COUNT);
    references.sort_by_key(|reference| reference.revision_id);

    let mut shards = Vec::new();
    for group in references.chunks(MAX_SHARD_ENTRIES) {
        let shard = S3ManifestShardV1::new(
            &ns, OperationId::new(), group.to_vec(),
        ).unwrap();
        client.store_manifest_object(
            shard.shard_id, &shard.encode(&ns).unwrap(),
        ).await.unwrap();
        shards.push(shard.reference(&ns).unwrap());
    }
    assert_eq!(shards.len(), 79);

    // Both generations have precisely the same verified references.
    // We still exercise real S3 If-None-Match and If-Match root conditions.
    let first = S3ManifestV1::new(
        &ns, OperationId::new(), 1, None, shards.clone(),
    ).unwrap();
    let first_hash = first.digest(&ns).unwrap();
    client.store_manifest_object(
        first.manifest_id, &first.encode(&ns).unwrap(),
    ).await.unwrap();
    let first_pointer = S3HeadPointerV1::new(
        &ns, 1, first.manifest_id, &first_hash, None,
    ).unwrap();
    let key = ns.current_head_key();
    let first_write = client.put_conditionally(
        &key, &first_pointer.encode(&ns).unwrap(),
        S3PutCondition::IfAbsent,
    ).await.unwrap();
    let second = S3ManifestV1::new(
        &ns, OperationId::new(), 2,
        Some((first.manifest_id, first_hash)), shards,
    ).unwrap();
    let second_hash = second.digest(&ns).unwrap();
    client.store_manifest_object(
        second.manifest_id, &second.encode(&ns).unwrap(),
    ).await.unwrap();
    let second_pointer = S3HeadPointerV1::new(
        &ns, 2, second.manifest_id, &second_hash,
        Some(first.manifest_id),
    ).unwrap();
    client.put_conditionally(
        &key, &second_pointer.encode(&ns).unwrap(),
        S3PutCondition::IfMatch(first_write.etag),
    ).await.unwrap();

    let io_start = client.test_io_snapshot();
    let verified_at = Instant::now();
    let verified = client.read_verified_head().await.unwrap().unwrap();
    let verify_elapsed = verified_at.elapsed();
    let io = client.test_io_snapshot().since(io_start);
    assert_eq!(verified.pointer, second_pointer);
    assert_eq!(verified.manifest, second);
    assert_eq!(verified.manifest.total_revisions as usize, REVISION_COUNT);

    // Both the immutable JSON and every blob MUST have been re-read.
    // This is not 10k S3 LIST keys: it is 20k verified actual objects.
    assert!(io.get >= (REVISION_COUNT as u64) * 2);
    assert!(io.get < (REVISION_COUNT as u64) * 2 + 512,
        "unexpected repeated full ancestor reads: {io:?}");
    assert_eq!(io.head, 2 * io.get);
    assert!(io.payload_bytes >= (REVISION_COUNT * PAYLOAD_BYTES) as u64);
    eprintln!(
        "AKULA_S3_FULL_DAG_10K upload_ms={} verify_ms={} documents={}          revisions={} blobs={} shard_count=79 sdk_head={} sdk_get={}          sdk_payload_bytes={} (SDK calls, not wire traffic)",
        upload_elapsed.as_millis(), verify_elapsed.as_millis(),
        DOCUMENTS, REVISION_COUNT, REVISION_COUNT,
        io.head, io.get, io.payload_bytes,
    );

    // Verify the complete immutable generation chain separately. It must
    // cost only manifest/shard reads (two generations), not 10k blob GETs.
    let ancestry_before = client.test_io_snapshot();
    let ancestry_at = Instant::now();
    let hops = client.verify_manifest_ancestors(&verified, 4).await.unwrap();
    let ancestry_ms = ancestry_at.elapsed().as_millis();
    let ancestry = client.test_io_snapshot().since(ancestry_before);
    assert_eq!(hops, 1);
    assert!(ancestry.get > 0 && ancestry.get < 300,
        "ancestor traversal downloaded old blobs again: {ancestry:?}");
    assert_eq!(ancestry.head, 2 * ancestry.get);
    eprintln!(
        "AKULA_S3_FULL_DAG_10K ancestry_ms={} ancestry_hops={}          sdk_ancestry_get={} sdk_ancestry_head={}",
        ancestry_ms, hops, ancestry.get, ancestry.head,
    );

    // Real sample of the unmerged sibling family: current S3 verification
    // must not silently collapse the three concurrent heads.
    let (document_id, ids) = sibling_example.unwrap();
    let mut example = crate::sync::RevisionDag::new();
    for id in ids {
        let json = client.get_bounded(
            &ns.immutable_revision_key(id), 64 * 1024,
        ).await.unwrap();
        let record: RevisionRecord = serde_json::from_slice(&json.bytes).unwrap();
        assert_eq!(record.document_id, document_id);
        example.ingest(record).unwrap();
    }
    assert_eq!(
        example.heads(document_id),
        [ids[1], ids[2], ids[3]].into_iter().collect(),
    );
}
