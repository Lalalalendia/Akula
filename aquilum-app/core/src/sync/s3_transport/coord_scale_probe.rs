//! SYNC-COORD-02f: measured, read-only 10k-revision scale comparison.
//!
//! All writers below are confined to a random disposable loopback MinIO
//! bucket inside this ignored test. Production S3 V1/V2 wire paths, File Gate,
//! authenticated writer trust and local baselines are NEVER modified.

use super::{
    AwsS3ProbeClient, S3ConnectionProfile, S3PutCondition, S3TestIoCounts,
};
use crate::sync::{
    DocumentId, OperationId, RevisionId, RevisionOperation, RevisionRecord,
    S3DeltaEnvelopeV2, S3HeadPointerV1, S3Keyspace,
    S3ManifestDeltaLimits, S3ManifestShardRefV1, S3ManifestShardV1,
    S3ManifestV1, S3RemoteAnchorJournal, S3RemoteAnchorScope,
    S3RevisionRefV1, SyncRelativePath, VaultId,
    MAX_S3_REVISION_METADATA_CACHE_BYTES,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

const MAX_UPLOAD_CONCURRENCY: usize = 32;
const BASE_SHARD_SIZE: usize = 64;
const MAX_EDITS: usize = 1_000;
const MAX_NOTES: usize = 10_000;

type Shards = Vec<(S3ManifestShardRefV1, Vec<S3RevisionRefV1>)>;

#[derive(Clone)]
struct Entry {
    reference: S3RevisionRefV1,
    json: Vec<u8>,
    blob: Vec<u8>,
}

fn entry(
    doc: DocumentId,
    revision: RevisionId,
    parents: Vec<RevisionId>,
    note_index: usize,
    content: Option<Vec<u8>>,
) -> Entry {
    let path = SyncRelativePath::parse(
        &format!("notes/scale-{note_index:05}.md"),
    ).unwrap();
    let operation = if let Some(bytes) = &content {
        RevisionOperation::Content {
            path,
            blob_hash: blake3::hash(bytes).to_hex().to_string(),
        }
    } else {
        RevisionOperation::Tombstone { path }
    };
    let json = serde_json::to_vec(&RevisionRecord {
        id: revision, document_id: doc, parents, operation,
    }).unwrap();
    Entry {
        reference: S3RevisionRefV1 {
            revision_id: revision,
            document_id: doc,
            revision_blake3: blake3::hash(&json).to_hex().to_string(),
            blob_bytes: content.as_ref().map(|b| b.len() as u64),
        },
        json,
        blob: content.unwrap_or_default(),
    }
}

async fn upload_entries(client: &AwsS3ProbeClient, ns: &S3Keyspace, items: &[Entry]) {
    let mut work = tokio::task::JoinSet::new();
    for item in items.iter().cloned() {
        let worker = client.clone();
        let namespace = ns.clone();
        work.spawn(async move {
            worker.put_conditionally(
                &namespace.immutable_revision_key(item.reference.revision_id),
                &item.json, S3PutCondition::IfAbsent,
            ).await.unwrap();
            if item.reference.blob_bytes.is_some() {
                let revision: RevisionRecord =
                    serde_json::from_slice(&item.json).unwrap();
                let hash = match revision.operation {
                    RevisionOperation::Content { blob_hash, .. }
                    | RevisionOperation::Rename { blob_hash, .. } => blob_hash,
                    _ => unreachable!(),
                };
                worker.put_conditionally(
                    &namespace.immutable_blob_key(&hash).unwrap(),
                    &item.blob, S3PutCondition::IfAbsent,
                ).await.unwrap();
            }
        });
        if work.len() >= MAX_UPLOAD_CONCURRENCY {
            work.join_next().await.unwrap().unwrap();
        }
    }
    while let Some(done) = work.join_next().await {
        done.unwrap();
    }
}

async fn seed_initial_shards(
    client: &AwsS3ProbeClient, ns: &S3Keyspace, refs: &[S3RevisionRefV1],
) -> Shards {
    let mut sorted = refs.to_vec();
    sorted.sort_by_key(|r| r.revision_id);
    let mut shards = Vec::new();
    for group in sorted.chunks(BASE_SHARD_SIZE) {
        let shard = S3ManifestShardV1::new(
            ns, OperationId::new(), group.to_vec(),
        ).unwrap();
        client.put_conditionally(
            &ns.immutable_manifest_key(shard.shard_id),
            &shard.encode(ns).unwrap(), S3PutCondition::IfAbsent,
        ).await.unwrap();
        shards.push((shard.reference(ns).unwrap(), group.to_vec()));
    }
    shards
}

/// Preserve unaffected immutable shard IDs/digests; changes are placed into
/// the existing sorted UUID range of a shard. 64-item initial shards leave
/// headroom for 1000 additions without claiming an ideal production sharder.
/// Any overflow or ordering failure aborts rather than silently dropping refs.
async fn append_sharded(
    client: &AwsS3ProbeClient,
    ns: &S3Keyspace,
    prior: &Shards,
    added: &[S3RevisionRefV1],
) -> (Shards, usize) {
    let mut insertions: BTreeMap<usize, Vec<S3RevisionRefV1>> = BTreeMap::new();
    let mut unique = BTreeSet::new();
    for row in added {
        assert!(unique.insert(row.revision_id), "duplicate new UUID");
        let insertion = prior.partition_point(
            |(descriptor, _)| descriptor.last_revision < row.revision_id,
        );
        let index = insertion.min(prior.len() - 1);
        insertions.entry(index).or_default().push(row.clone());
    }
    let mut changed_shards = 0usize;
    let mut next = Vec::with_capacity(prior.len());
    for (i, (previous, previous_refs)) in prior.iter().enumerate() {
        if let Some(extra) = insertions.remove(&i) {
            let mut combined = previous_refs.clone();
            combined.extend(extra);
            combined.sort_by_key(|r| r.revision_id);
            assert!(combined.len() <= 128, "unexpected shard overflow");
            assert!(combined.windows(2).all(|w| w[0].revision_id < w[1].revision_id));
            let shard = S3ManifestShardV1::new(
                ns, OperationId::new(), combined.clone(),
            ).unwrap();
            client.put_conditionally(
                &ns.immutable_manifest_key(shard.shard_id),
                &shard.encode(ns).unwrap(), S3PutCondition::IfAbsent,
            ).await.unwrap();
            next.push((shard.reference(ns).unwrap(), combined));
            changed_shards += 1;
        } else {
            next.push((previous.clone(), previous_refs.clone()));
        }
    }
    assert!(insertions.is_empty());
    assert!(next.windows(2).all(|w| {
        w[0].0.last_revision < w[1].0.first_revision
    }));
    (next, changed_shards)
}

async fn publish_root(
    client: &AwsS3ProbeClient,
    ns: &S3Keyspace,
    generation: u64,
    parent: Option<&S3ManifestV1>,
    shards: &Shards,
) -> (S3ManifestV1, S3HeadPointerV1) {
    let previous = parent.map(|p| {
        (p.manifest_id, p.digest(ns).unwrap())
    });
    let root = S3ManifestV1::new(
        ns, OperationId::new(), generation, previous,
        shards.iter().map(|(d, _)| d.clone()).collect(),
    ).unwrap();
    let bytes = root.encode(ns).unwrap();
    let head = S3HeadPointerV1::new(
        ns, generation, root.manifest_id,
        &blake3::hash(&bytes).to_hex().to_string(),
        parent.map(|p| p.manifest_id),
    ).unwrap();
    client.put_conditionally(
        &ns.immutable_manifest_key(root.manifest_id),
        &bytes, S3PutCondition::IfAbsent,
    ).await.unwrap();
    (root, head)
}

async fn publish_letter_packs(
    client: &AwsS3ProbeClient,
    ns: &S3Keyspace,
    prior: &S3HeadPointerV1,
    current: &S3HeadPointerV1,
    additions: &[Entry],
) -> (String, usize, u64) {
    // Exercise the production-compiled pure V2 encoder on REAL
    // disposable MinIO objects. Only this test fixture owns any S3 PUT.
    let source: Vec<_> = additions.iter().map(|item| (
        item.reference.clone(), item.json.clone(), item.blob.clone(),
    )).collect();
    let prepared = super::coord_pack_encode::prepare_delta_packs_v2(
        ns, &source,
    ).unwrap();
    let count = prepared.packs.len();
    let indexes: Vec<_> =
        prepared.packs.iter().map(|p| p.index.clone()).collect();
    let mut bytes_published = prepared.packed_bytes as u64;
    for pack in &prepared.packs {
        client.put_conditionally(
            &pack.descriptor.key, &pack.payload, S3PutCondition::IfAbsent,
        ).await.unwrap();
    }
    let letter = S3DeltaEnvelopeV2::new(
        ns, prior, current, prepared.references, indexes,
    ).unwrap();
    let bytes = letter.encode(ns).unwrap();
    bytes_published += bytes.len() as u64;
    let digest = blake3::hash(&bytes).to_hex().to_string();
    client.put_conditionally(
        &ns.immutable_blob_key(&digest).unwrap(),
        &bytes, S3PutCondition::IfAbsent,
    ).await.unwrap();
    (digest, count, bytes_published)
}

#[derive(Clone, Copy)]
struct Measurement {
    io: S3TestIoCounts,
    micros: u128,
}
impl Measurement {
    fn as_json(self) -> serde_json::Value {
        serde_json::json!({
            "get": self.io.get, "head": self.io.head,
            "received_bytes": self.io.payload_bytes,
            "elapsed_us": self.micros
        })
    }
}
fn measured(
    client: &AwsS3ProbeClient, start: S3TestIoCounts, tick: Instant,
) -> Measurement {
    let io = client.test_io_snapshot().since(start);
    assert_eq!(io.head, io.get * 2, "SDK HEAD/GET invariant violated");
    Measurement { io, micros: tick.elapsed().as_micros() }
}

#[tokio::test]
#[ignore = "explicit, expensive isolated localhost MinIO S3 read-only 10k scale"]
async fn minio_coord_02f_10k_revision_offline_parity() {
    let notes: usize = std::env::var("AQ_COORD_02F_NOTES")
        .unwrap_or_else(|_| "64".into()).parse().unwrap();
    let edits: usize = std::env::var("AQ_COORD_02F_EDITS")
        .unwrap_or_else(|_| "7".into()).parse().unwrap();
    assert!((2..=MAX_NOTES).contains(&notes));
    assert!(matches!(edits, 0 | 7 | 1000));
    assert!(edits == 0 || edits <= notes);
    assert!(edits <= MAX_EDITS);
    let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").unwrap();
    assert!(endpoint.starts_with("http://127.0.0.1:")
        || endpoint.starts_with("http://localhost:"));
    let ns = S3Keyspace::new(
        "akula/sync-coord-02f-scale", VaultId::new(),
    ).unwrap();
    let profile = S3ConnectionProfile {
        bucket: format!("akula-coord-02f-{}", uuid::Uuid::new_v4().simple()),
        region: "us-east-1".into(),
        endpoint: Some(endpoint),
        force_path_style: true,
    };
    let writer = AwsS3ProbeClient::connect(profile.clone(), ns.clone())
        .await.unwrap();
    writer.client.create_bucket().bucket(&writer.bucket)
        .send().await.unwrap();
    let reader = AwsS3ProbeClient::connect(profile.clone(), ns.clone())
        .await.unwrap();
    let scope = S3RemoteAnchorScope::new(
        "coord-02f-ci", "disposable-two-client-s3-scale", &profile, &ns,
    ).unwrap();

    // 10k genuinely distinct Markdown documents, each with one immutable
    // revision JSON and individually verified content blob in real MinIO.
    let baseline: Vec<_> = (0..notes).map(|i| {
        let doc = DocumentId::new();
        let rev = RevisionId::new();
        let text = format!("# baseline note {i:05} content");
        entry(doc, rev, vec![], i, Some(text.into_bytes()))
    }).collect();
    let fixture_started = Instant::now();
    upload_entries(&writer, &ns, &baseline).await;
    let old_refs: Vec<_> = baseline.iter().map(|r| r.reference.clone()).collect();
    let shards = seed_initial_shards(&writer, &ns, &old_refs).await;
    let (gen1, head1) = publish_root(&writer, &ns, 1, None, &shards).await;
    let first_write = writer.put_conditionally(
        &ns.current_head_key(), &head1.encode(&ns).unwrap(),
        S3PutCondition::IfAbsent,
    ).await.unwrap();
    let published_genesis_ms = fixture_started.elapsed().as_millis();

    // Measure the complete initial full-verification work separately. No
    // warm-read savings are allowed to silently exclude this cold cost.
    let cold_before = reader.test_io_snapshot();
    let cold_started = Instant::now();
    let cache = reader.materialize_verified_revision_metadata_store(
        MAX_S3_REVISION_METADATA_CACHE_BYTES,
    ).await.unwrap().unwrap();
    let pinned = reader.pin_revision_snapshot().await.unwrap().unwrap();
    let observed = reader.read_verified_head().await.unwrap().unwrap();
    let cold_measure = measured(&reader, cold_before, cold_started);
    assert_eq!(cache.len(), notes);
    assert_eq!(pinned.len(), notes);
    assert_eq!(observed.pointer, head1);
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("verified-remote-anchor.sqlite");
    let mut journal = S3RemoteAnchorJournal::open(&db_path).unwrap();
    journal.record_verified_remote(
        &scope, &observed, &pinned, &[gen1.encode(&ns).unwrap()],
    ).unwrap();
    drop(journal);
    let journal = S3RemoteAnchorJournal::open(&db_path).unwrap();

    // No artificial generation or empty V2 letter is fabricated when the
    // head is unchanged. Report zero-delta S3-selective and full pin only.
    if edits == 0 {
        let sel_before = reader.test_io_snapshot();
        let sel_started = Instant::now();
        let sel = reader.inspect_selective_s3_revision_changes(
            &scope, &journal, &cache, MAX_S3_REVISION_METADATA_CACHE_BYTES,
            S3ManifestDeltaLimits { max_frames: 8, max_total_shard_refs: 8 * 256 },
        ).await.unwrap();
        let selective = measured(&reader, sel_before, sel_started);
        assert!(sel.new_revision_ids.is_empty());
        assert_eq!(sel.total_revisions, notes);
        assert!(sel.requires_full_apply_gate);

        let full_before = reader.test_io_snapshot();
        let full_started = Instant::now();
        let full = reader.pin_revision_snapshot().await.unwrap().unwrap();
        let full_read = measured(&reader, full_before, full_started);
        assert_eq!(pinned.references_for_coord_v2(), full.references_for_coord_v2());
        assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &head1);
        println!("AKULA_COORD_02F_SCALE {}", serde_json::json!({
            "notes": notes, "edits": 0, "offline_generations": 0,
            "assurance": "read-only-no-old-blob-refresh-on-selective-no-file-gate",
            "genesis_publish_ms": published_genesis_ms,
            "cold_anchor": cold_measure.as_json(),
            "unchanged_selective_v1": selective.as_json(),
            "full_current_v1": full_read.as_json(),
            "v2_letter_mode": "not-applicable-without-new-generation",
            "identical_inventory": true,
            "anchor_unmodified": true
        }));
        return;
    }

    let first_half = edits / 2;
    let mut gen2_additions = Vec::with_capacity(first_half);
    for i in 0..first_half {
        let original = &baseline[i];
        gen2_additions.push(entry(
            original.reference.document_id,
            RevisionId::new(), vec![original.reference.revision_id],
            i, Some(format!("edit gen2 note {i}").into_bytes()),
        ));
    }
    let mut gen3_additions = Vec::with_capacity(edits - first_half);
    // gen3 tombstones the genesis root of doc0, making it a *sibling*
    // of doc0's gen2 content revision. Other tombstones use distinct docs.
    for i in 0..(edits - first_half) {
        let index = if i == 0 { 0 } else { first_half + i - 1 };
        let original = &baseline[index];
        gen3_additions.push(entry(
            original.reference.document_id,
            RevisionId::new(), vec![original.reference.revision_id],
            index, None,
        ));
    }
    assert_eq!(gen2_additions.len() + gen3_additions.len(), edits);
    let edit_publish_started = Instant::now();
    upload_entries(&writer, &ns, &gen2_additions).await;
    let (shards2, changed_shards2) = append_sharded(
        &writer, &ns, &shards,
        &gen2_additions.iter().map(|r| r.reference.clone()).collect::<Vec<_>>(),
    ).await;
    let (gen2, head2) = publish_root(
        &writer, &ns, 2, Some(&gen1), &shards2,
    ).await;
    let second_write = writer.put_conditionally(
        &ns.current_head_key(), &head2.encode(&ns).unwrap(),
        S3PutCondition::IfMatch(first_write.etag),
    ).await.unwrap();
    let (letter2, pack_count2, pack_bytes2) = publish_letter_packs(
        &writer, &ns, &head1, &head2, &gen2_additions,
    ).await;
    upload_entries(&writer, &ns, &gen3_additions).await;
    let (shards3, changed_shards3) = append_sharded(
        &writer, &ns, &shards2,
        &gen3_additions.iter().map(|r| r.reference.clone()).collect::<Vec<_>>(),
    ).await;
    let (gen3, head3) = publish_root(
        &writer, &ns, 3, Some(&gen2), &shards3,
    ).await;
    writer.put_conditionally(
        &ns.current_head_key(), &head3.encode(&ns).unwrap(),
        S3PutCondition::IfMatch(second_write.etag),
    ).await.unwrap();
    let (letter3, pack_count3, pack_bytes3) = publish_letter_packs(
        &writer, &ns, &head2, &head3, &gen3_additions,
    ).await;
    let edit_publish_ms = edit_publish_started.elapsed().as_millis();
    let letters = [letter2, letter3];

    let limits = S3ManifestDeltaLimits {
        max_frames: 8, max_total_shard_refs: 8 * 256,
    };
    let packed_before = reader.test_io_snapshot();
    let packed_started = Instant::now();
    let packed = reader.inspect_v2_offline_selective_packs_against_pinned_v1(
        &scope, &journal, &pinned, &cache, &letters,
    ).await.unwrap();
    let packed_measure = measured(&reader, packed_before, packed_started);

    let selective_before = reader.test_io_snapshot();
    let selective_started = Instant::now();
    let selective = reader.inspect_selective_s3_revision_changes(
        &scope, &journal, &cache,
        MAX_S3_REVISION_METADATA_CACHE_BYTES, limits,
    ).await.unwrap();
    let selective_measure = measured(&reader, selective_before, selective_started);

    let chain_before = reader.test_io_snapshot();
    let chain_started = Instant::now();
    let chain = reader.inspect_v2_offline_chain_against_pinned_v1(
        &scope, &journal, &pinned, &cache, &letters,
    ).await.unwrap();
    let chain_measure = measured(&reader, chain_before, chain_started);

    let full_before = reader.test_io_snapshot();
    let full_started = Instant::now();
    let full = reader.pin_revision_snapshot().await.unwrap().unwrap();
    let full_measure = measured(&reader, full_before, full_started);

    let expected_new: BTreeSet<_> = gen2_additions.iter()
        .chain(gen3_additions.iter())
        .map(|r| r.reference.revision_id).collect();
    let packed_ids: BTreeSet<_> =
        packed.new_revision_ids.iter().copied().collect();
    let selective_ids: BTreeSet<_> =
        selective.new_revision_ids.iter().copied().collect();
    let chain_ids: BTreeSet<_> =
        chain.new_revision_ids.iter().copied().collect();
    assert_eq!(expected_new.len(), edits);
    assert_eq!(expected_new, packed_ids);
    assert_eq!(expected_new, selective_ids);
    assert_eq!(expected_new, chain_ids);
    let old_ids: BTreeSet<_> = pinned.references_for_coord_v2()
        .iter().map(|r| r.revision_id).collect();
    let final_ids: BTreeSet<_> = full.references_for_coord_v2()
        .iter().map(|r| r.revision_id).collect();
    assert!(old_ids.is_disjoint(&expected_new));
    assert_eq!(old_ids.union(&expected_new).copied().collect::<BTreeSet<_>>(), final_ids);
    assert_eq!(final_ids.len(), notes + edits);
    assert_eq!(packed.total_revisions, full.len());
    assert_eq!(selective.total_revisions, full.len());
    assert!(packed.requires_full_apply_gate);
    assert!(selective.requires_full_apply_gate);
    assert!(chain.requires_full_apply_gate);
    assert_eq!(packed.verified_generations, 2);
    assert_eq!(chain.verified_generations, 2);
    assert_eq!(packed.verified_packs, pack_count2 + pack_count3);
    assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &head1);

    // An omitted or reversed per-generation letter is NOT an optimization.
    assert!(reader.inspect_v2_offline_selective_packs_against_pinned_v1(
        &scope, &journal, &pinned, &cache, &letters[..1],
    ).await.is_err());
    assert!(reader.inspect_v2_offline_selective_packs_against_pinned_v1(
        &scope, &journal, &pinned, &cache,
        &[letters[1].clone(), letters[0].clone()],
    ).await.is_err());
    assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &head1);

    println!("AKULA_COORD_02F_SCALE {}", serde_json::json!({
        "notes": notes,
        "edits": edits,
        "offline_generations": 2,
        "assurance": "read-only-selective-reuses-old-blob-verification-no-file-gate",
        "identical_inventory": true,
        "anchor_unmodified": true,
        "genesis_publish_ms": published_genesis_ms,
        "cold_anchor": cold_measure.as_json(),
        "edit_publish_ms": edit_publish_ms,
        "publication": {
            "new_v1_revision_objects": edits,
            "new_v1_blob_objects": first_half,
            "new_v1_shard_objects": changed_shards2 + changed_shards3,
            "new_v2_pack_objects": pack_count2 + pack_count3,
            "v2_pack_and_letter_bytes": pack_bytes2 + pack_bytes3,
            "final_v1_shards": shards3.len()
        },
        "selective_v2_packs": {
            "io": packed_measure.as_json(),
            "downloaded_shards": packed.downloaded_shards,
            "reused_shards": packed.reused_shards,
            "verified_packs": packed.verified_packs,
            "checked_pack_bytes": packed.checked_pack_bytes
        },
        "selective_v1": selective_measure.as_json(),
        "conservative_chain_v2": chain_measure.as_json(),
        "full_current_v1": full_measure.as_json()
    }));
}
