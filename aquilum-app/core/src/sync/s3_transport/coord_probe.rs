//! SYNC-COORD-01: isolated, test-only experiment for coordinate Merkle trees,
//! small immutable change letters and bounded transaction packs.
//!
//! Not a cloud schema or production provider. Its in-memory old Merkle roots
//! are assumed to have been independently verified in a *test fixture*.
//! A real protocol still needs writer authentication, CAS-root publication,
//! durable per-generation proof, E2EE and a complete File Gate acceptance.
//! Never infer local deletion, confirm a baseline or write into a user vault.

use super::{AwsS3ProbeClient, S3ConnectionProfile, S3PutCondition, S3TestIoCounts};
use super::pack_probe::{
    ingest_verified_pack, PackBuilder, PackDescriptor, MAX_PACK_MEMBERS,
};
use crate::sync::{
    DocumentId, RevisionDag, RevisionId, RevisionOperation, RevisionRecord,
    S3Keyspace, S3RevisionRefV1, SyncRelativePath, VaultId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

const EXPERIMENT_SCHEMA: u16 = 1;
const MAX_COORD_NODE_BYTES: usize = 128 * 1024;
const MAX_LETTER_BYTES: usize = 512 * 1024;
const MAX_BENCH_REVISIONS: usize = 1_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CellRecord {
    document_id: DocumentId,
    /// Complete immutable revision history for this document in this
    /// experimental index (not only its latest visible Markdown contents).
    revisions: Vec<S3RevisionRefV1>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CellChild {
    slot: u8,
    digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum CellNode {
    Internal { fanout: u8, level: u8, children: Vec<CellChild> },
    Leaf { fanout: u8, level: u8, records: Vec<CellRecord> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LetterPack {
    digest: String,
    members: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LetterTree {
    fanout: u8,
    root_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangeLetter {
    schema: u16,
    vault_id: VaultId,
    /// Experimental fixture anchor, not a signed/CAS production trust root.
    prior_seven_root_digest: String,
    revisions: Vec<S3RevisionRefV1>,
    packs: Vec<LetterPack>,
    trees: Vec<LetterTree>,
}

#[derive(Debug, Eq, PartialEq)]
enum CellProbeError {
    InvalidNode,
    LostHistory,
    RewrittenRevision,
    DuplicateRevision,
    InvalidLetter,
    DigestMismatch,
    MissingReference,
}

struct BuiltTree {
    root: String,
    objects: BTreeMap<String, Vec<u8>>,
}

fn depth(fanout: u8) -> usize {
    match fanout {
        7 => 5, 16 => 4, 32 => 3,
        _ => panic!("experimental fanout must be 7, 16 or 32"),
    }
}

/// Synthetic hash address, not physical H3 geography. A stable DocumentId
/// and vault decide the address; paths, revisions, device and wall time don't.
fn address(ns: &S3Keyspace, id: DocumentId, fanout: u8) -> Vec<u8> {
    let mut hasher = blake3::Hasher::new_derive_key(
        "akula/sync/coordinate-probe/v1",
    );
    hasher.update(ns.vault_id().to_string().as_bytes());
    hasher.update(id.to_string().as_bytes());
    let digest = hasher.finalize();
    digest.as_bytes()[..depth(fanout)]
        .iter().map(|byte| byte % fanout).collect()
}

fn store_node(
    node: CellNode,
    objects: &mut BTreeMap<String, Vec<u8>>,
) -> String {
    let bytes = serde_json::to_vec(&node).unwrap();
    assert!(bytes.len() <= MAX_COORD_NODE_BYTES);
    let digest = blake3::hash(&bytes).to_hex().to_string();
    objects.insert(digest.clone(), bytes);
    digest
}

fn build_level(
    source: Vec<(Vec<u8>, CellRecord)>,
    fanout: u8,
    level: usize,
    objects: &mut BTreeMap<String, Vec<u8>>,
) -> String {
    if level == depth(fanout) {
        let mut records: Vec<_> = source.into_iter().map(|(_, r)| r).collect();
        records.sort_by_key(|r| r.document_id);
        return store_node(CellNode::Leaf {
            fanout, level: level as u8, records,
        }, objects);
    }
    let mut buckets = vec![Vec::new(); fanout as usize];
    for (coords, record) in source {
        buckets[coords[level] as usize].push((coords, record));
    }
    let mut children = Vec::new();
    for (slot, bucket) in buckets.into_iter().enumerate() {
        if !bucket.is_empty() {
            let digest = build_level(bucket, fanout, level + 1, objects);
            children.push(CellChild { slot: slot as u8, digest });
        }
    }
    store_node(CellNode::Internal {
        fanout, level: level as u8, children,
    }, objects)
}

fn build_tree(ns: &S3Keyspace, fanout: u8, rows: &[CellRecord]) -> BuiltTree {
    let mut objects = BTreeMap::new();
    let coords: Vec<_> = rows.iter()
        .map(|r| (address(ns, r.document_id, fanout), r.clone()))
        .collect();
    let root = build_level(coords, fanout, 0, &mut objects);
    BuiltTree { root, objects }
}

fn decode_checked(bytes: &[u8], digest: &str) -> Result<CellNode, CellProbeError> {
    if bytes.len() > MAX_COORD_NODE_BYTES
        || blake3::hash(bytes).to_hex().as_str() != digest
    {
        return Err(CellProbeError::DigestMismatch);
    }
    serde_json::from_slice(bytes).map_err(|_| CellProbeError::InvalidNode)
}

fn check_leaf_continuity(
    before: &[CellRecord],
    after: &[CellRecord],
    new: &mut BTreeMap<RevisionId, S3RevisionRefV1>,
) -> Result<(), CellProbeError> {
    let old: BTreeMap<_, _> = before.iter()
        .map(|r| (r.document_id, &r.revisions)).collect();
    let latest: BTreeMap<_, _> = after.iter()
        .map(|r| (r.document_id, &r.revisions)).collect();
    if old.len() != before.len() || latest.len() != after.len() {
        return Err(CellProbeError::InvalidNode);
    }
    for (doc, past) in &old {
        let Some(current) = latest.get(doc) else {
            return Err(CellProbeError::LostHistory);
        };
        for prior in *past {
            match current.iter().find(|r| r.revision_id == prior.revision_id) {
                Some(same) if same == prior => {}
                Some(_) => return Err(CellProbeError::RewrittenRevision),
                None => return Err(CellProbeError::LostHistory),
            }
        }
    }
    for current in after {
        let mut local = BTreeSet::new();
        for reference in &current.revisions {
            if reference.document_id != current.document_id
                || !local.insert(reference.revision_id)
            {
                return Err(CellProbeError::DuplicateRevision);
            }
            let is_old = old.get(&current.document_id)
                .is_some_and(|refs| refs.contains(reference));
            if !is_old
                && new.insert(reference.revision_id, reference.clone()).is_some()
            {
                return Err(CellProbeError::DuplicateRevision);
            }
        }
    }
    Ok(())
}

fn child_map(node: &CellNode, fanout: u8, level: usize)
    -> Result<BTreeMap<u8, String>, CellProbeError>
{
    let CellNode::Internal {
        fanout: f, level: l, children,
    } = node else { return Err(CellProbeError::InvalidNode) };
    if *f != fanout || usize::from(*l) != level {
        return Err(CellProbeError::InvalidNode);
    }
    let mut out = BTreeMap::new();
    for child in children {
        if child.slot >= fanout || child.digest.len() != 64
            || out.insert(child.slot, child.digest.clone()).is_some()
        {
            return Err(CellProbeError::InvalidNode);
        }
    }
    Ok(out)
}

async fn compare_remote_tree(
    reader: &AwsS3ProbeClient,
    ns: &S3Keyspace,
    fanout: u8,
    old: &BuiltTree,
    new_root: &str,
) -> Result<Vec<S3RevisionRefV1>, CellProbeError> {
    let mut stack = vec![(Some(old.root.clone()), new_root.to_owned(), 0usize)];
    let mut found = BTreeMap::new();
    while let Some((previous, current, level)) = stack.pop() {
        if previous.as_deref() == Some(current.as_str()) {
            continue;
        }
        let key = ns.immutable_blob_key(&current)
            .map_err(|_| CellProbeError::InvalidNode)?;
        let payload = reader.get_bounded(&key, MAX_COORD_NODE_BYTES as u64)
            .await.map_err(|_| CellProbeError::MissingReference)?;
        let new_node = decode_checked(&payload.bytes, &current)?;
        let old_node = previous
            .as_ref()
            .map(|h| old.objects.get(h).ok_or(CellProbeError::MissingReference)
                .and_then(|data| decode_checked(data, h)))
            .transpose()?;

        if level == depth(fanout) {
            let CellNode::Leaf {
                fanout: f, level: l, records,
            } = new_node else { return Err(CellProbeError::InvalidNode) };
            if f != fanout || usize::from(l) != level {
                return Err(CellProbeError::InvalidNode);
            }
            let earlier = match old_node {
                Some(CellNode::Leaf {
                    fanout: old_f, level: old_l, records: old_rows,
                }) if old_f == f && old_l == l => old_rows,
                None => Vec::new(),
                _ => return Err(CellProbeError::InvalidNode),
            };
            check_leaf_continuity(&earlier, &records, &mut found)?;
            continue;
        }
        let children = child_map(&new_node, fanout, level)?;
        let previous_children = match old_node {
            Some(node) => child_map(&node, fanout, level)?,
            None => BTreeMap::new(),
        };
        if previous_children.keys().any(|k| !children.contains_key(k)) {
            return Err(CellProbeError::LostHistory);
        }
        for (slot, digest) in children {
            stack.push((previous_children.get(&slot).cloned(), digest, level + 1));
        }
    }
    Ok(found.into_values().collect())
}

fn id_for(index: usize, generation: u128) -> RevisionId {
    RevisionId::from_uuid(uuid::Uuid::from_u128(
        ((index as u128 + 1) << 64) | (0x6_0000_0000 + generation),
    ))
}
fn doc_for(index: usize) -> DocumentId {
    DocumentId::from_uuid(uuid::Uuid::from_u128(
        ((index as u128 + 1) << 64) | 0x7_0000_0001,
    ))
}
fn note(index: usize, updated: bool) -> (S3RevisionRefV1, Vec<u8>, Vec<u8>) {
    let blob = format!(
        "# Test note {index:05}\nGeneration {}\n",
        if updated { 2 } else { 1 },
    ).into_bytes();
    let revision = RevisionRecord {
        id: id_for(index, if updated { 2 } else { 1 }),
        document_id: doc_for(index),
        parents: if updated { vec![id_for(index, 1)] } else { Vec::new() },
        operation: RevisionOperation::Content {
            path: SyncRelativePath::parse(
                &format!("notes/coord-{index:05}.md"),
            ).unwrap(),
            blob_hash: blake3::hash(&blob).to_hex().to_string(),
        },
    };
    let json = serde_json::to_vec(&revision).unwrap();
    let reference = S3RevisionRefV1 {
        revision_id: revision.id,
        document_id: revision.document_id,
        revision_blake3: blake3::hash(&json).to_hex().to_string(),
        blob_bytes: Some(blob.len() as u64),
    };
    (reference, json, blob)
}

fn verify_letter(
    bytes: &[u8], digest: &str, ns: &S3Keyspace, base: &str,
) -> Result<ChangeLetter, CellProbeError> {
    if bytes.len() > MAX_LETTER_BYTES
        || blake3::hash(bytes).to_hex().as_str() != digest
    {
        return Err(CellProbeError::DigestMismatch);
    }
    let letter: ChangeLetter = serde_json::from_slice(bytes)
        .map_err(|_| CellProbeError::InvalidLetter)?;
    if letter.schema != EXPERIMENT_SCHEMA
        || letter.vault_id != ns.vault_id()
        || letter.prior_seven_root_digest != base
        || letter.revisions.len() > MAX_BENCH_REVISIONS
    {
        return Err(CellProbeError::InvalidLetter);
    }
    let mut seen = BTreeSet::new();
    if !letter.revisions.iter().all(|r| seen.insert(r.revision_id)) {
        return Err(CellProbeError::DuplicateRevision);
    }
    let total: usize = letter.packs.iter().map(|p| p.members).sum();
    if total != letter.revisions.len()
        || letter.packs.iter().any(|p| p.members == 0
            || p.members > MAX_PACK_MEMBERS || p.digest.len() != 64)
        || letter.trees.len() != 3
        || letter.trees.iter().map(|t| t.fanout).collect::<Vec<_>>()
            != vec![7, 16, 32]
        || letter.trees.iter().any(|t| t.root_digest.len() != 64)
    {
        return Err(CellProbeError::InvalidLetter);
    }
    Ok(letter)
}

async fn read_letter(
    client: &AwsS3ProbeClient,
    ns: &S3Keyspace,
    hash: &str,
    prior_root: &str,
) -> ChangeLetter {
    let bytes = client.get_bounded(
        &ns.immutable_blob_key(hash).unwrap(), MAX_LETTER_BYTES as u64,
    ).await.unwrap();
    verify_letter(&bytes.bytes, hash, ns, prior_root).unwrap()
}

fn seeded_dag(old_json: &[Vec<u8>]) -> RevisionDag {
    let mut dag = RevisionDag::new();
    for json in old_json {
        let entry: RevisionRecord = serde_json::from_slice(json).unwrap();
        dag.ingest(entry).unwrap();
    }
    assert_eq!(dag.pending_len(), 0);
    dag
}

fn verify_individual(
    reference: &S3RevisionRefV1, json: &[u8], blob: &[u8],
) -> Result<RevisionRecord, CellProbeError> {
    if blake3::hash(json).to_hex().as_str() != reference.revision_blake3 {
        return Err(CellProbeError::DigestMismatch);
    }
    let parsed: RevisionRecord = serde_json::from_slice(json)
        .map_err(|_| CellProbeError::InvalidLetter)?;
    if parsed.id != reference.revision_id
        || parsed.document_id != reference.document_id
    {
        return Err(CellProbeError::InvalidLetter);
    }
    match &parsed.operation {
        RevisionOperation::Content { blob_hash, .. }
        | RevisionOperation::Rename { blob_hash, .. } => {
            if reference.blob_bytes != Some(blob.len() as u64)
                || blake3::hash(blob).to_hex().as_str() != blob_hash
            {
                return Err(CellProbeError::DigestMismatch);
            }
        }
        RevisionOperation::Tombstone { .. } => {
            if reference.blob_bytes.is_some() || !blob.is_empty() {
                return Err(CellProbeError::InvalidLetter);
            }
        }
    }
    Ok(parsed)
}

async fn read_individual(
    client: &AwsS3ProbeClient,
    ns: &S3Keyspace,
    refs: &[S3RevisionRefV1],
    dag: &mut RevisionDag,
) {
    for r in refs {
        let json = client.get_bounded(
            &ns.immutable_revision_key(r.revision_id), 64 * 1024,
        ).await.unwrap();
        let parsed: RevisionRecord = serde_json::from_slice(&json.bytes).unwrap();
        let blob_hash = match &parsed.operation {
            RevisionOperation::Content { blob_hash, .. }
            | RevisionOperation::Rename { blob_hash, .. } => blob_hash,
            RevisionOperation::Tombstone { .. } => {
                let verified = verify_individual(r, &json.bytes, &[]).unwrap();
                dag.ingest(verified).unwrap();
                continue;
            }
        };
        let blob = client.get_bounded(
            &ns.immutable_blob_key(blob_hash).unwrap(), 64 * 1024,
        ).await.unwrap();
        let checked = verify_individual(r, &json.bytes, &blob.bytes).unwrap();
        dag.ingest(checked).unwrap();
    }
    assert_eq!(dag.pending_len(), 0);
}

async fn read_packs(
    client: &AwsS3ProbeClient,
    ns: &S3Keyspace,
    letter: &ChangeLetter,
    dag: &mut RevisionDag,
) {
    let mut seen = BTreeSet::new();
    let mut cursor = 0;
    for p in &letter.packs {
        let end = cursor + p.members;
        let descriptor = PackDescriptor {
            key: ns.immutable_blob_key(&p.digest).unwrap(),
            digest: p.digest.clone(),
            references: letter.revisions[cursor..end].to_vec(),
        };
        let payload = client.get_bounded(
            &descriptor.key, 512 * 1024,
        ).await.unwrap();
        ingest_verified_pack(&payload.bytes, &descriptor, dag, &mut seen).unwrap();
        cursor = end;
    }
    assert_eq!(cursor, letter.revisions.len());
    assert_eq!(seen.len(), cursor);
    assert_eq!(dag.pending_len(), 0);
}

fn report(
    method: &str,
    notes: usize,
    edits: usize,
    counts: S3TestIoCounts,
    elapsed_ms: u128,
    published_objects: usize,
    published_bytes: usize,
    letter_bytes: usize,
) {
    eprintln!("AKULA_COORD_AB {}", serde_json::json!({
        "method": method,
        "notes": notes,
        "edits": edits,
        "attempted_s3_get": counts.get,
        "attempted_s3_head": counts.head,
        "downloaded_payload_bytes": counts.payload_bytes,
        "elapsed_ms": elapsed_ms,
        "fixture_immutable_objects_published": published_objects,
        "fixture_upload_bytes": published_bytes,
        "letter_bytes": letter_bytes,
        "assurance": "test-only-trusted-fixture-read-only-no-file-gate"
    }));
}

async fn upload(
    client: &AwsS3ProbeClient,
    objects: BTreeMap<String, Vec<u8>>,
) {
    let mut jobs = tokio::task::JoinSet::new();
    for (key, bytes) in objects {
        let c = client.clone();
        jobs.spawn(async move {
            c.put_conditionally(
                &key, &bytes, S3PutCondition::IfAbsent,
            ).await.unwrap();
        });
        if jobs.len() >= 32 {
            jobs.join_next().await.unwrap().unwrap();
        }
    }
    while let Some(task) = jobs.join_next().await {
        task.unwrap();
    }
}

#[tokio::test]
#[ignore = "explicit disposable localhost MinIO coordinate/pack A/B"]
async fn minio_coord_letter_pack_merkle_ab() {
    let count: usize = std::env::var("AQ_COORD_BENCH_NOTES")
        .unwrap_or_else(|_| "64".into()).parse().unwrap();
    let edits: usize = std::env::var("AQ_COORD_BENCH_EDITS")
        .unwrap_or_else(|_| "7".into()).parse().unwrap();
    assert!((7..=10_000).contains(&count));
    assert!(edits <= count && edits <= MAX_BENCH_REVISIONS);
    let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").unwrap();
    assert!(
        endpoint.starts_with("http://127.0.0.1:")
            || endpoint.starts_with("http://localhost:"),
        "never use a real S3 account or user bucket for experimental packs",
    );

    let profile = S3ConnectionProfile {
        bucket: format!("akula-coord-{}", uuid::Uuid::new_v4().simple()),
        region: "us-east-1".to_owned(),
        endpoint: Some(endpoint),
        force_path_style: true,
    };
    let ns = S3Keyspace::new("akula/coord-test", VaultId::new()).unwrap();
    let client = AwsS3ProbeClient::connect(profile, ns.clone()).await.unwrap();
    client.client.create_bucket()
        .bucket(&client.bucket).send().await.unwrap();

    let mut old = Vec::with_capacity(count);
    let mut later = Vec::with_capacity(count);
    let mut prior_json = Vec::with_capacity(count);
    let mut changes = Vec::with_capacity(edits);
    let mut new_payload = BTreeMap::new();
    let mut pack_builder = PackBuilder::new();
    let mut packs = Vec::new();

    for i in 0..count {
        let (base, json, _) = note(i, false);
        prior_json.push(json);
        let entry = CellRecord { document_id: base.document_id, revisions: vec![base] };
        old.push(entry.clone());
        later.push(entry);
    }
    for j in 0..edits {
        let i = j * count / edits;
        let (r, json, blob) = note(i, true);
        later[i].revisions.push(r.clone());
        changes.push(r.clone());
        // Legacy per-object payload fixture; only updated objects are
        // uploaded, old revisions are presumed verified in local cache.
        new_payload.insert(ns.immutable_revision_key(r.revision_id), json.clone());
        let parsed: RevisionRecord = serde_json::from_slice(&json).unwrap();
        let hash = match parsed.operation {
            RevisionOperation::Content { blob_hash, .. } => blob_hash,
            _ => unreachable!(),
        };
        new_payload.insert(ns.immutable_blob_key(&hash).unwrap(), blob.clone());
        pack_builder.push(r, &json, &blob).unwrap();
        if pack_builder.len() == MAX_PACK_MEMBERS {
            packs.push(std::mem::replace(&mut pack_builder, PackBuilder::new())
                .seal(&ns).unwrap());
        }
    }
    if !pack_builder.is_empty() {
        packs.push(pack_builder.seal(&ns).unwrap());
    }

    let mut old_trees = Vec::new();
    let mut new_trees = Vec::new();
    for f in [7, 16, 32] {
        old_trees.push(build_tree(&ns, f, &old));
        new_trees.push(build_tree(&ns, f, &later));
    }
    let letter = ChangeLetter {
        schema: EXPERIMENT_SCHEMA,
        vault_id: ns.vault_id(),
        prior_seven_root_digest: old_trees[0].root.clone(),
        revisions: changes.clone(),
        packs: packs.iter().map(|(p, _)| LetterPack {
            digest: p.digest.clone(), members: p.references.len(),
        }).collect(),
        trees: [7, 16, 32].into_iter().zip(new_trees.iter())
            .map(|(fanout, tree)| LetterTree {
                fanout, root_digest: tree.root.clone(),
            }).collect(),
    };
    let letter_bytes = serde_json::to_vec(&letter).unwrap();
    assert!(letter_bytes.len() <= MAX_LETTER_BYTES);
    let letter_digest = blake3::hash(&letter_bytes).to_hex().to_string();
    let letter_key = ns.immutable_blob_key(&letter_digest).unwrap();

    // Publish everything as TEST-ONLY immutable, content-addressed blobs.
    // There is no production S3 schema change or mutable current-root CAS.
    // S3 PUT/write cost is counted separately from all read measurements.
    let mut common_objects = new_payload;
    common_objects.insert(letter_key, letter_bytes.clone());
    for (descriptor, payload) in &packs {
        common_objects.insert(descriptor.key.clone(), payload.clone());
    }
    let common_count = common_objects.len();
    let common_bytes: usize = common_objects.values().map(Vec::len).sum();
    upload(&client, common_objects).await;

    let mut extra_writes = Vec::new();
    for (previous, changed) in old_trees.iter().zip(&new_trees) {
        let mut writes = BTreeMap::new();
        for (digest, bytes) in &changed.objects {
            if !previous.objects.contains_key(digest) {
                writes.insert(ns.immutable_blob_key(digest).unwrap(), bytes.clone());
            }
        }
        let count = writes.len();
        let bytes: usize = writes.values().map(Vec::len).sum();
        upload(&client, writes).await;
        extra_writes.push((count, bytes));
    }

    // The letter must retain all new immutable references and the correct
    // prior verified root; its digest is a TEST-ONLY caller-known commitment.
    let mut baseline = seeded_dag(&prior_json);
    let begin = client.test_io_snapshot();
    let started = Instant::now();
    let observed = read_letter(&client, &ns, &letter_digest, &old_trees[0].root).await;
    read_individual(&client, &ns, &observed.revisions, &mut baseline).await;
    assert_eq!(baseline.pending_len(), 0);
    assert_eq!(baseline.active_len(), count + edits);
    report("letter_individual", count, edits,
        client.test_io_snapshot().since(begin),
        started.elapsed().as_millis(), common_count, common_bytes,
        letter_bytes.len());

    let mut batch_dag = seeded_dag(&prior_json);
    let begin = client.test_io_snapshot();
    let started = Instant::now();
    let observed = read_letter(&client, &ns, &letter_digest, &old_trees[0].root).await;
    read_packs(&client, &ns, &observed, &mut batch_dag).await;
    assert_eq!(batch_dag.active_len(), count + edits);
    report("letter_pack", count, edits,
        client.test_io_snapshot().since(begin),
        started.elapsed().as_millis(),
        common_count, common_bytes, letter_bytes.len());

    for (i, f) in [7_u8, 16, 32].into_iter().enumerate() {
        let mut dag = seeded_dag(&prior_json);
        let begin = client.test_io_snapshot();
        let started = Instant::now();
        let observed = read_letter(&client, &ns, &letter_digest, &old_trees[0].root).await;
        let located = compare_remote_tree(
            &client, &ns, f, &old_trees[i],
            &observed.trees[i].root_digest,
        ).await.unwrap();
        let expected: BTreeMap<_, _> = observed.revisions.iter()
            .map(|r| (r.revision_id, r.clone())).collect();
        let received: BTreeMap<_, _> = located.iter()
            .map(|r| (r.revision_id, r.clone())).collect();
        assert_eq!(received, expected, "Merkle diff must preserve all changes");
        read_packs(&client, &ns, &observed, &mut dag).await;
        assert_eq!(dag.active_len(), count + edits);
        let (nodes, new_bytes) = extra_writes[i];
        report(&format!("merkle_{f}_pack"), count, edits,
            client.test_io_snapshot().since(begin),
            started.elapsed().as_millis(),
            common_count + nodes, common_bytes + new_bytes, letter_bytes.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_document_address_ignores_rename_and_revision_count() {
        let ns = S3Keyspace::new("akula/coord-local", VaultId::new()).unwrap();
        let a = doc_for(7);
        for f in [7, 16, 32] {
            assert_eq!(address(&ns, a, f), address(&ns, a, f));
            assert_eq!(address(&ns, a, f).len(), depth(f));
            assert!(address(&ns, a, f).iter().all(|digit| *digit < f));
        }
        assert_ne!(address(&ns, doc_for(7), 7), address(&ns, doc_for(8), 7));
    }

    #[test]
    fn roots_depend_on_revisions_not_input_order() {
        let ns = S3Keyspace::new("akula/coord-roots", VaultId::new()).unwrap();
        let mut rows: Vec<_> = (0..42).map(|i| {
            let (reference, _, _) = note(i, false);
            CellRecord { document_id: reference.document_id, revisions: vec![reference] }
        }).collect();
        for f in [7, 16, 32] {
            let original = build_tree(&ns, f, &rows);
            rows.reverse();
            let reordered = build_tree(&ns, f, &rows);
            assert_eq!(original.root, reordered.root);
            assert!(!original.objects.is_empty());
        }
    }

    #[test]
    fn an_intermediate_tombstone_cannot_be_hidden() {
        let (base, _, _) = note(2, false);
        let tomb = S3RevisionRefV1 {
            revision_id: id_for(2, 3),
            document_id: base.document_id,
            revision_blake3: blake3::hash(b"tombstone").to_hex().to_string(),
            blob_bytes: None,
        };
        let old = vec![CellRecord {
            document_id: base.document_id,
            revisions: vec![base.clone(), tomb],
        }];
        let latest = vec![CellRecord {
            document_id: base.document_id,
            revisions: vec![base],
        }];
        assert_eq!(check_leaf_continuity(&old, &latest, &mut BTreeMap::new()),
            Err(CellProbeError::LostHistory));
    }

    #[test]
    fn spoofed_revision_hash_and_duplicate_are_fail_closed() {
        let (original, _, _) = note(7, false);
        let mut rewrite = original.clone();
        rewrite.revision_blake3 = blake3::hash(b"fake").to_hex().to_string();
        let old = vec![CellRecord {
            document_id: original.document_id,
            revisions: vec![original.clone()],
        }];
        let latest = vec![CellRecord {
            document_id: original.document_id,
            revisions: vec![rewrite],
        }];
        assert_eq!(check_leaf_continuity(&old, &latest, &mut BTreeMap::new()),
            Err(CellProbeError::RewrittenRevision));
        assert_eq!(check_leaf_continuity(&old, &old, &mut BTreeMap::new()),
            Ok(()));
        let duplicate = vec![CellRecord {
            document_id: original.document_id,
            revisions: vec![original.clone(), original],
        }];
        assert_eq!(check_leaf_continuity(&[], &duplicate, &mut BTreeMap::new()),
            Err(CellProbeError::DuplicateRevision));
    }

    #[test]
    fn malformed_letter_or_missing_pack_members_rejected() {
        let ns = S3Keyspace::new("akula/coord-check", VaultId::new()).unwrap();
        let prior = "a".repeat(64);
        let (ref1, _, _) = note(3, true);
        let letter = ChangeLetter {
            schema: 1, vault_id: ns.vault_id(),
            prior_seven_root_digest: prior.clone(),
            revisions: vec![ref1],
            packs: vec![],
            trees: [7, 16, 32].map(|fanout| LetterTree {
                fanout, root_digest: "b".repeat(64),
            }).to_vec(),
        };
        let payload = serde_json::to_vec(&letter).unwrap();
        let digest = blake3::hash(&payload).to_hex().to_string();
        assert_eq!(verify_letter(&payload, &digest, &ns, &prior),
            Err(CellProbeError::InvalidLetter));
        let mut tampered = payload.clone();
        tampered.push(0);
        assert_eq!(verify_letter(&tampered, &digest, &ns, &prior),
            Err(CellProbeError::DigestMismatch));
    }
}
