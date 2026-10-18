//! SYNC-05d2h isolated live V2 root probe. Test compilation only.
//!
//! This is NOT a production publisher, data migrator or user file reader.
//! Every write is scoped to a randomly generated disposable localhost MinIO
//! vault and requires an actual server-enforced If-None-Match / If-Match.
//! The temporary SQLite WAL records only this fixture's uncertain root CAS.
//! No user workspace, deletion, GC, E2EE key custody or baseline is touched.

use super::*;
use crate::sync::{
    DocumentId, RevisionDag, RevisionOperation, RevisionRecord, S3ContractError,
    S3HeadPointerV1, S3ManifestShardV1, SyncRelativePath,
    S3RemoteAnchorScope, S3PinnedV2ReadError,
    MAX_MANIFEST_BYTES, MAX_SHARD_BYTES,
};
use crate::sync::s3_transport::{
    AwsS3ProbeClient, S3ConnectionProfile, S3PutCondition, S3TransportError,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

type ProbeResult<T> = Result<T, String>;

fn require(ok: bool, why: &str) -> ProbeResult<()> {
    if ok { Ok(()) } else { Err(why.to_owned()) }
}

fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[derive(Clone)]
struct SampleRevision {
    record: RevisionRecord,
    blob: Option<Vec<u8>>,
}

fn sample_revision(
    document_id: DocumentId,
    revision_id: RevisionId,
    parents: Vec<RevisionId>,
    content: Option<&[u8]>,
) -> SampleRevision {
    let path = SyncRelativePath::parse("notes/guarded-conflict.md").unwrap();
    let record = RevisionRecord {
        id: revision_id, document_id, parents,
        operation: match content {
            Some(bytes) => RevisionOperation::Content {
                path,
                blob_hash: digest(bytes),
            },
            None => RevisionOperation::Tombstone { path },
        },
    };
    SampleRevision { record, blob: content.map(|bytes| bytes.to_vec()) }
}

struct Candidate {
    head: S3PackedHeadV2,
    head_bytes: Vec<u8>,
    manifest: S3ManifestV1,
    pack_key: String,
    pack_hash: String,
}

async fn put_verified_immutable(
    client: &AwsS3ProbeClient,
    key: &str,
    bytes: &[u8],
) -> ProbeResult<()> {
    client.put_conditionally(key, bytes, S3PutCondition::IfAbsent)
        .await.map_err(|e| format!("immutable IfAbsent failed: {e:?}"))?;
    let observed = client.get_bounded(key, bytes.len() as u64).await
        .map_err(|e| format!("immutable readback failed: {e:?}"))?;
    require(observed.bytes == bytes && observed.blake3 == digest(bytes),
        "immutable readback bytes/digest do not match")
}

/// Each candidate owns a new immutable manifest, V2 index and pack. Even the
/// losing CAS branch is never remotely deleted or treated as local deletion.
async fn upload_candidate(
    client: &AwsS3ProbeClient,
    ns: &S3Keyspace,
    predecessor: Option<&Candidate>,
    revisions: &[SampleRevision],
) -> ProbeResult<Candidate> {
    require(!revisions.is_empty() && revisions.len() <= MAX_PACK_V2_MEMBERS_PER_PACK,
        "invalid candidate revision count")?;
    let mut packed = b"AKPKEXP1".to_vec();
    packed.extend_from_slice(&(revisions.len() as u16).to_le_bytes());
    let mut refs = Vec::new();
    let mut pack_members = Vec::new();
    for entry in revisions {
        let json = serde_json::to_vec(&entry.record)
            .map_err(|e| format!("revision serialization: {e}"))?;
        let body = entry.blob.as_deref().unwrap_or_default();
        let reference = S3RevisionRefV1 {
            revision_id: entry.record.id,
            document_id: entry.record.document_id,
            revision_blake3: digest(&json),
            blob_bytes: entry.blob.as_ref().map(|v| v.len() as u64),
        };
        packed.extend_from_slice(&(json.len() as u32).to_le_bytes());
        packed.extend_from_slice(&(body.len() as u32).to_le_bytes());
        packed.extend_from_slice(&json);
        packed.extend_from_slice(body);
        pack_members.push((reference.clone(), json.len() as u32));
        refs.push(reference);
    }
    let pack_hash = digest(&packed);
    let descriptor = S3PackDescriptorV2::from_lengths(pack_hash.clone(), pack_members)
        .map_err(|e| format!("invalid fixture pack offsets: {e:?}"))?;
    require(descriptor.pack_bytes as usize == packed.len(), "pack size mismatch")?;
    let pack_key = ns.immutable_blob_key(&pack_hash)
        .map_err(|e| format!("invalid pack key: {e:?}"))?;
    put_verified_immutable(client, &pack_key, &packed).await?;

    refs.sort_by_key(|r| r.revision_id);
    let shard = S3ManifestShardV1::new(ns, OperationId::new(), refs)
        .map_err(|e| format!("invalid V1 revision shard: {e:?}"))?;
    let shard_bytes = shard.encode(ns).map_err(|e| format!("{e:?}"))?;
    put_verified_immutable(
        client, &ns.immutable_manifest_key(shard.shard_id), &shard_bytes,
    ).await?;

    let generation = predecessor.map_or(1, |prior| prior.head.generation() + 1);
    let parent = predecessor.map(|prior| (
        prior.manifest.manifest_id,
        prior.manifest.digest(ns).unwrap(),
    ));
    let manifest = S3ManifestV1::new(
        ns, OperationId::new(), generation, parent,
        vec![shard.reference(ns).map_err(|e| format!("{e:?}"))?],
    ).map_err(|e| format!("invalid V1 manifest: {e:?}"))?;
    let manifest_bytes = manifest.encode(ns).map_err(|e| format!("{e:?}"))?;
    put_verified_immutable(
        client, &ns.immutable_manifest_key(manifest.manifest_id), &manifest_bytes,
    ).await?;
    let manifest_hash = digest(&manifest_bytes);

    let pack_shard = S3PackIndexShardV2::new(
        ns, OperationId::new(), vec![descriptor],
    ).map_err(|e| format!("invalid V2 pack shard: {e:?}"))?;
    let pack_shard_bytes = pack_shard.encode(ns).map_err(|e| format!("{e:?}"))?;
    put_verified_immutable(
        client, &ns.immutable_manifest_key(pack_shard.shard_id), &pack_shard_bytes,
    ).await?;
    let index = S3PackIndexRootV2::new(
        ns, OperationId::new(), generation, manifest.manifest_id,
        manifest_hash.clone(),
        vec![pack_shard.reference(ns).map_err(|e| format!("{e:?}"))?],
    ).map_err(|e| format!("invalid V2 index: {e:?}"))?;
    let index_bytes = index.encode(ns).map_err(|e| format!("{e:?}"))?;
    put_verified_immutable(
        client, &ns.immutable_manifest_key(index.index_id), &index_bytes,
    ).await?;

    let head = S3PackedHeadV2::new(
        ns, generation, manifest.manifest_id, &manifest_hash,
        index.index_id, &digest(&index_bytes),
        predecessor.map(|prior| prior.head.digest(ns).unwrap()),
    ).map_err(|e| format!("invalid V2 head: {e:?}"))?;
    let head_bytes = head.encode(ns).map_err(|e| format!("{e:?}"))?;
    if let Some(prior) = predecessor {
        require(head.is_direct_successor_of(ns, &prior.head),
            "invalid two-client V2 predecessor relation")?;
        require(manifest.is_successor_of(
            &prior.manifest, &prior.manifest.digest(ns).unwrap(),
        ), "V1 manifest predecessor was replaced")?;
    }
    Ok(Candidate { head, head_bytes, manifest, pack_key, pack_hash })
}

async fn pinned_manifest_refs(
    client: &AwsS3ProbeClient,
    ns: &S3Keyspace,
    id: OperationId,
    expected_hash: &str,
) -> ProbeResult<(S3ManifestV1, BTreeMap<RevisionId, S3RevisionRefV1>)> {
    let root = client.get_bounded(
        &ns.immutable_manifest_key(id), MAX_MANIFEST_BYTES as u64,
    ).await.map_err(|e| format!("manifest read failed: {e:?}"))?;
    require(root.blake3 == expected_hash, "manifest digest mismatch")?;
    let manifest = S3ManifestV1::decode(ns, id, &root.bytes)
        .map_err(|e| format!("invalid pinned manifest: {e:?}"))?;
    let mut references = BTreeMap::new();
    for shard_ref in &manifest.shards {
        let bytes = client.get_bounded(
            &ns.immutable_manifest_key(shard_ref.shard_id),
            MAX_SHARD_BYTES as u64,
        ).await.map_err(|e| format!("manifest shard read: {e:?}"))?;
        require(bytes.blake3 == shard_ref.shard_blake3,
            "manifest shard digest mismatch")?;
        let shard = S3ManifestShardV1::decode(ns, shard_ref.shard_id, &bytes.bytes)
            .map_err(|e| format!("invalid manifest shard: {e:?}"))?;
        require(
            shard.revisions.len() == shard_ref.count as usize
            && shard.revisions.first().map(|r| r.revision_id)
                == Some(shard_ref.first_revision)
            && shard.revisions.last().map(|r| r.revision_id)
                == Some(shard_ref.last_revision),
            "manifest shard boundaries mismatch",
        )?;
        for record in shard.revisions {
            require(references.insert(record.revision_id, record).is_none(),
                "duplicate pinned manifest revision")?;
        }
    }
    require(references.len() == manifest.total_revisions as usize,
        "manifest revision count mismatch")?;
    Ok((manifest, references))
}

/// Validate all framing, offsets and BLAKE3 member digests independently of
/// the caller's V2 index. No writable handles are returned.
fn verify_pack_members(
    bytes: &[u8],
    desc: &S3PackDescriptorV2,
    dag: &mut RevisionDag,
    seen: &mut BTreeSet<RevisionId>,
) -> ProbeResult<()> {
    require(bytes.len() == desc.pack_bytes as usize
        && digest(bytes) == desc.pack_blake3, "pack hash or length mismatch")?;
    require(bytes.len() >= 10 && &bytes[..8] == b"AKPKEXP1",
        "pack header mismatch")?;
    let count = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
    require(count == desc.members.len(), "pack member count mismatch")?;
    let mut cursor = 10_usize;
    for member in &desc.members {
        let head = bytes.get(cursor..cursor+8)
            .ok_or_else(|| "truncated pack frame".to_owned())?;
        let rev_len = u32::from_le_bytes(head[..4].try_into().unwrap()) as usize;
        let blob_len = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
        require(rev_len == member.revision_len as usize
            && member.revision_offset as usize == cursor + 8
            && member.reference.blob_bytes == if member.blob_offset.is_none() {
                None
            } else { Some(blob_len as u64) },
            "pack member frame offsets/lengths disagree")?;
        cursor += 8;
        let revision_json = bytes.get(cursor..cursor+rev_len)
            .ok_or_else(|| "truncated revision JSON".to_owned())?;
        require(digest(revision_json) == member.reference.revision_blake3,
            "revision JSON BLAKE3 mismatch")?;
        cursor += rev_len;
        require(member.blob_offset.map(|x| x as usize) == if
            member.reference.blob_bytes.is_some()
        {Some(cursor)} else {None}, "pack blob offset mismatch")?;
        let blob = bytes.get(cursor..cursor+blob_len)
            .ok_or_else(|| "truncated blob".to_owned())?;
        cursor += blob_len;
        let record: RevisionRecord = serde_json::from_slice(revision_json)
            .map_err(|e| format!("invalid revision JSON: {e}"))?;
        require(record.id == member.reference.revision_id
            && record.document_id == member.reference.document_id,
            "wrong revision or document identity")?;
        match &record.operation {
            RevisionOperation::Content { ref blob_hash, .. }
            | RevisionOperation::Rename { ref blob_hash, .. } => {
                require(member.reference.blob_bytes == Some(blob_len as u64)
                    && digest(blob).as_str() == blob_hash.as_str(), "blob content integrity failure")?;
            },
            RevisionOperation::Tombstone { .. } => {
                require(blob_len == 0 && member.reference.blob_bytes.is_none(),
                    "tombstone illegally acquired blob")?;
            },
        }
        require(seen.insert(record.id), "duplicate packed revision identity")?;
        dag.ingest(record).map_err(|e| format!("invalid packed DAG parent: {e:?}"))?;
    }
    require(cursor == bytes.len(), "pack has unreferenced trailing bytes")?;
    Ok(())
}

struct ObservedV2 {
    head: S3PackedHeadV2,
    etag: String,
    root_bytes: Vec<u8>,
    dag: RevisionDag,
}

/// A fully V2-pinned READ from the one test vault. Root ETag and bytes are
/// rechecked *after* all immutable references; modified S3 root means Hold.
/// An authenticated provider CAS root is required; ETags are never digests.
/// Not a RemoteStore, not an approval to write or delete local notes.
async fn read_verified_v2(
    client: &AwsS3ProbeClient,
    ns: &S3Keyspace,
) -> ProbeResult<ObservedV2> {
    let root_key = ns.current_head_key();
    let start = client.get_bounded(&root_key, MAX_PACK_V2_ROOT_BYTES as u64)
        .await.map_err(|e| format!("current V2 root read: {e:?}"))?;
    let head = S3PackedHeadV2::decode(ns, &start.bytes)
        .map_err(|e| format!("invalid remote V2 root: {e:?}"))?;
    // A legacy V1 client must HOLD, never infer an empty snapshot.
    require(S3HeadPointerV1::decode(&start.bytes, ns).is_err(),
        "V2 root was accepted by V1 reader")?;
    let (manifest, current_refs) = pinned_manifest_refs(
        client, ns, head.manifest_id, &head.manifest_blake3,
    ).await?;
    if let (Some(parent_id), Some(parent_hash)) = (
        manifest.previous_manifest_id, manifest.previous_manifest_blake3.as_deref()
    ) {
        let (parent, parent_refs) = pinned_manifest_refs(
            client, ns, parent_id, parent_hash,
        ).await?;
        require(manifest.is_successor_of(&parent, parent_hash),
            "invalid immutable manifest lineage")?;
        require(parent_refs.iter().all(|(id, ref_old)|
            current_refs.get(id) == Some(ref_old)),
            "a V2 generation dropped or rewrote existing revision history")?;
    } else {
        require(manifest.generation == 1, "missing immutable manifest ancestor")?;
    }

    let indexed = client.get_bounded(
        &ns.immutable_manifest_key(head.index_id), MAX_PACK_V2_INDEX_BYTES as u64,
    ).await.map_err(|e| format!("V2 pack index read: {e:?}"))?;
    require(indexed.blake3 == head.index_blake3, "V2 root/index BLAKE3 mismatch")?;
    let index = S3PackIndexRootV2::decode(ns, head.index_id, &indexed.bytes)
        .map_err(|e| format!("index rejected: {e:?}"))?;
    let mut shard_bytes = Vec::new();
    for r in &index.shards {
        let b = client.get_bounded(
            &ns.immutable_manifest_key(r.shard_id),
            MAX_PACK_V2_SHARD_BYTES as u64,
        ).await.map_err(|e| format!("V2 shard fetch: {e:?}"))?;
        require(b.blake3 == r.shard_blake3, "V2 shard BLAKE3 mismatch")?;
        shard_bytes.push((r.shard_id, b.bytes));
    }
    index.verify_complete(
        ns, &head, &manifest,
        &current_refs.values().cloned().collect::<Vec<_>>(),
        &shard_bytes,
    ).map_err(|e| format!("invalid V2 complete coverage: {e:?}"))?;

    let mut dag = RevisionDag::new();
    let mut seen = BTreeSet::new();
    for (id, bytes) in shard_bytes {
        let shard = S3PackIndexShardV2::decode(ns, id, &bytes)
            .map_err(|e| format!("invalid V2 shard: {e:?}"))?;
        for desc in shard.packs {
            let key = ns.immutable_blob_key(&desc.pack_blake3)
                .map_err(|e| format!("invalid pack key: {e:?}"))?;
            let packet = client.get_bounded(&key, MAX_PACK_V2_BYTES as u64)
                .await.map_err(|e| format!("pack payload read: {e:?}"))?;
            require(packet.blake3 == desc.pack_blake3,
                "packed S3 object BLAKE3 mismatch")?;
            verify_pack_members(&packet.bytes, &desc, &mut dag, &mut seen)?;
        }
    }
    require(seen.len() == manifest.total_revisions as usize
        && dag.active_len() == seen.len() && dag.pending_len() == 0
        && seen == current_refs.keys().copied().collect::<BTreeSet<_>>(),
        "incomplete packed DAG or unresolved parents")?;

    let end = client.get_bounded(&root_key, MAX_PACK_V2_ROOT_BYTES as u64)
        .await.map_err(|e| format!("current root post-read: {e:?}"))?;
    require(end.bytes == start.bytes && end.object.etag == start.object.etag,
        "current root changed during V2 verification")?;
    Ok(ObservedV2 {
        head, etag: start.object.etag, root_bytes: start.bytes, dag,
    })
}

/// Durable test-only WAL for uncertainty after a V2 CAS write. It holds
/// exact proposed bytes and old ETag. No committed sync baseline is stored.
struct V2ProbeJournal { db: Connection }

impl V2ProbeJournal {
    fn open(path: &Path) -> ProbeResult<Self> {
        let db = Connection::open(path).map_err(|e| format!("{e}"))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;
             CREATE TABLE IF NOT EXISTS v2_test_root_attempt (
               op TEXT PRIMARY KEY NOT NULL,
               vault TEXT NOT NULL, remote_key TEXT NOT NULL,
               expected_etag TEXT, root_hash TEXT NOT NULL,
               root_json BLOB NOT NULL, observed_etag TEXT
             );",
        ).map_err(|e| format!("{e}"))?;
        Ok(Self { db })
    }

    fn prepare(
        &mut self, op: OperationId, ns: &S3Keyspace,
        expected_etag: Option<&str>, root_json: &[u8],
    ) -> ProbeResult<()> {
        let proposed = S3PackedHeadV2::decode(ns, root_json)
            .map_err(|e| format!("invalid journal root: {e:?}"))?;
        require((proposed.generation() == 1) == expected_etag.is_none(),
            "V2 journal missing CAS condition")?;
        if let Some(value) = expected_etag {
            require(!value.is_empty() && value.len() <= 256,
                "invalid previous ETag")?;
        }
        let hash = digest(root_json);
        let tx = self.db.transaction().map_err(|e| format!("{e}"))?;
        let old: Option<(String,String,Option<String>,String,Vec<u8>)> = tx.query_row(
            "SELECT vault,remote_key,expected_etag,root_hash,root_json
               FROM v2_test_root_attempt WHERE op=?1",
            params![op.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        ).optional().map_err(|e| format!("{e}"))?;
        if let Some((vault,key,old_etag,old_hash,old_json)) = old {
            require(vault == ns.vault_id().to_string()
                && key == ns.current_head_key()
                && old_etag.as_deref() == expected_etag
                && old_hash == hash && old_json == root_json,
                "operation id reused with different vault, ETag or root bytes")?;
        } else {
            tx.execute(
                "INSERT INTO v2_test_root_attempt
                 (op,vault,remote_key,expected_etag,root_hash,root_json)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    op.to_string(), ns.vault_id().to_string(),
                    ns.current_head_key(), expected_etag, hash, root_json
                ],
            ).map_err(|e| format!("{e}"))?;
        }
        tx.commit().map_err(|e| format!("{e}"))?;
        Ok(())
    }

    fn is_observed(&self, op: OperationId) -> ProbeResult<bool> {
        let value: Option<Option<String>> = self.db.query_row(
            "SELECT observed_etag FROM v2_test_root_attempt WHERE op=?1",
            params![op.to_string()], |row| row.get(0),
        ).optional().map_err(|e| format!("{e}"))?;
        Ok(value.flatten().is_some())
    }

    async fn recover_verified(
        &mut self,
        op: OperationId,
        ns: &S3Keyspace,
        client: &AwsS3ProbeClient,
    ) -> ProbeResult<()> {
        let original: (String,String,String,Vec<u8>) = self.db.query_row(
            "SELECT vault,remote_key,root_hash,root_json
               FROM v2_test_root_attempt WHERE op=?1",
            params![op.to_string()],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).map_err(|e| format!("no durable V2 intent: {e}"))?;
        require(original.0 == ns.vault_id().to_string()
            && original.1 == ns.current_head_key()
            && original.2 == digest(&original.3),
            "durable V2 receipt identity corruption")?;
        // First verify the *whole* remote V2 manifest/index/pack and its
        // unchanged root ETag, not just an arbitrary matching pointer JSON.
        let observed = read_verified_v2(client, ns).await?;
        require(original.3 == observed.root_bytes
            && observed.head.encode(ns).map_err(|e| format!("{e:?}"))?
                == original.3,
            "different current root: lost ACK may not be retried blindly")?;
        let updated = self.db.execute(
            "UPDATE v2_test_root_attempt SET observed_etag=?2
               WHERE op=?1 AND (observed_etag IS NULL OR observed_etag=?2)",
            params![op.to_string(), observed.etag],
        ).map_err(|e| format!("receipt verification failed: {e}"))?;
        require(updated == 1 && self.is_observed(op)?,
            "conflicting persisted V2 receipt or unknown acknowledgement")?;
        Ok(())
    }
}

/// Tests true S3 conditional CAS, independent validation, competing offline
/// branches and unknown-ACK restart against a disposable localhost MinIO.
/// This is intentionally NOT named live_minio_ to avoid duplicate execution
/// in the existing broad SDK smoke suite.
#[tokio::test]
#[ignore = "explicit disposable loopback-only MinIO V2 CAS/receipt acceptance"]
async fn minio_v2_pack_root_cas_two_clients_and_lost_ack() {
    let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT")
        .expect("explicit localhost MinIO endpoint required");
    assert!(
        endpoint.starts_with("http://127.0.0.1:")
            || endpoint.starts_with("http://localhost:"),
        "never run the V2 write probe against public S3"
    );
    let ns = S3Keyspace::new("akula-sync/v2-guarded-cas-probe", VaultId::new()).unwrap();
    let profile = S3ConnectionProfile {
        bucket: "akula-v2-cas-test".into(),
        region: "us-east-1".into(),
        endpoint: Some(endpoint),
        force_path_style: true,
    };
    let scope = S3RemoteAnchorScope::new(
        "desktop-v2", "isolated-test-account", &profile, &ns,
    ).unwrap();
    let a = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
    let b = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
    // Direct bucket creation is confined to this disposable local server.
    let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new("us-east-1")).load().await;
    let bootstrap = aws_sdk_s3::Client::from_conf(
        aws_sdk_s3::config::Builder::from(&shared)
            .force_path_style(true)
            .endpoint_url(std::env::var("AQ_S3_TEST_ENDPOINT").unwrap())
            .build(),
    );
    bootstrap.create_bucket().bucket("akula-v2-cas-test")
        .send().await.unwrap();

    let doc = DocumentId::new();
    let id0 = RevisionId::new();
    let left = RevisionId::new();
    let right = RevisionId::new();
    let deletion = RevisionId::new();
    let original = sample_revision(doc, id0, vec![], Some(b"original file"));
    let winner_edits = vec![
        original.clone(),
        sample_revision(doc, left, vec![id0], Some(b"left client changed text")),
        sample_revision(doc, deletion, vec![id0], None),
    ];
    let loser_edits = vec![
        original.clone(),
        sample_revision(doc, right, vec![id0], Some(b"other offline edit")),
    ];
    let first = upload_candidate(&a, &ns, None, &[original]).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let genesis_db = dir.path().join("genesis.sqlite");
    let op1 = OperationId::new();
    let mut journal = V2ProbeJournal::open(&genesis_db).unwrap();
    journal.prepare(op1, &ns, None, &first.head_bytes).unwrap();
    let written = a.put_conditionally(
        &ns.current_head_key(), &first.head_bytes, S3PutCondition::IfAbsent,
    ).await.unwrap();
    let initial_observed = read_verified_v2(&b, &ns).await.unwrap();
    assert_eq!(initial_observed.head, first.head);
    assert_eq!(initial_observed.etag, written.etag);
    assert_eq!(initial_observed.dag.heads(doc), BTreeSet::from([id0]));
    // Exercise production-compiled, scope-bound V2 *observer*, not only
    // the historic test-local readback routine.
    let first_preview = b.inspect_explicitly_pinned_v2_read_only(
        &scope, &first.head, &written.etag, None,
    ).await.unwrap();
    assert_eq!(first_preview.generation, 1);
    assert_eq!(first_preview.verified_revisions, 1);
    assert_eq!(first_preview.verified_packs, 1);
    assert!(first_preview.requires_explicit_file_gate_review);
    journal.recover_verified(op1, &ns, &b).await.unwrap();
    assert!(journal.is_observed(op1).unwrap());

    // Both devices inspect the *same* verified genesis validator. Uploading
    // both immutable histories is harmless before the single CAS winner.
    let winning = upload_candidate(&a, &ns, Some(&first), &winner_edits)
        .await.unwrap();
    let losing = upload_candidate(&b, &ns, Some(&first), &loser_edits)
        .await.unwrap();
    assert_ne!(winning.head_bytes, losing.head_bytes);
    assert_eq!(winning.head.generation(), 2);
    let intent_winner = OperationId::new();
    let intent_loser = OperationId::new();
    let winner_db = dir.path().join("winner.sqlite");
    let loser_db = dir.path().join("loser.sqlite");
    let mut owner = V2ProbeJournal::open(&winner_db).unwrap();
    let mut competing = V2ProbeJournal::open(&loser_db).unwrap();
    owner.prepare(intent_winner, &ns, Some(&written.etag), &winning.head_bytes)
        .unwrap();
    competing.prepare(intent_loser, &ns, Some(&written.etag), &losing.head_bytes)
        .unwrap();
    assert!(competing.prepare(intent_loser, &ns,
        Some(&written.etag), &winning.head_bytes).is_err());
    drop(owner);
    drop(competing);

    // CAS succeeds at the server, but simulate the process losing its HTTP
    // success ACK. A restarted WAL must read all remote content before
    // considering the exact original root observed.
    let _ack_lost = a.put_conditionally(
        &ns.current_head_key(), &winning.head_bytes,
        S3PutCondition::IfMatch(written.etag.clone()),
    ).await.unwrap();
    let mut after_restart = V2ProbeJournal::open(&winner_db).unwrap();
    assert!(!after_restart.is_observed(intent_winner).unwrap());
    after_restart.recover_verified(intent_winner, &ns, &b).await.unwrap();
    assert!(after_restart.is_observed(intent_winner).unwrap());
    let observed = read_verified_v2(&b, &ns).await.unwrap();
    assert_eq!(observed.head, winning.head);
    assert_eq!(observed.dag.heads(doc), BTreeSet::from([left, deletion]));
    let v2_preview = b.inspect_explicitly_pinned_v2_read_only(
        &scope, &winning.head, &_ack_lost.etag, Some(&first.head),
    ).await.unwrap();
    assert_eq!(v2_preview.generation, 2);
    assert_eq!(v2_preview.verified_revisions, 3);
    assert_eq!(v2_preview.verified_packs, 1);
    assert_eq!(v2_preview.document_head_count, 2);
    assert!(matches!(
        b.inspect_explicitly_pinned_v2_read_only(
            &scope, &winning.head, &_ack_lost.etag, None,
        ).await, Err(S3PinnedV2ReadError::MissingTrustedRoot)
    ));
    assert!(matches!(
        b.inspect_explicitly_pinned_v2_read_only(
            &scope, &winning.head, "wrong-validator", Some(&first.head),
        ).await, Err(S3PinnedV2ReadError::RootChanged)
    ));
    assert!(matches!(
        b.inspect_explicitly_pinned_v2_read_only(
            &scope, &first.head, &written.etag, None,
        ).await, Err(S3PinnedV2ReadError::RootChanged)
    ));

    // A stale second writer must fail at the S3 server; it cannot overwrite
    // the first writer's root or treat a losing branch as a deletion.
    assert_eq!(
        b.put_conditionally(
            &ns.current_head_key(), &losing.head_bytes,
            S3PutCondition::IfMatch(written.etag),
        ).await.err(),
        Some(S3TransportError::PreconditionFailed),
    );
    let mut rejected = V2ProbeJournal::open(&loser_db).unwrap();
    assert!(!rejected.is_observed(intent_loser).unwrap());
    assert!(rejected.recover_verified(intent_loser, &ns, &a).await.is_err());
    assert!(!rejected.is_observed(intent_loser).unwrap());
    assert_eq!(read_verified_v2(&a, &ns).await.unwrap().head, winning.head);
    let loser_pack = b.get_bounded(&losing.pack_key, MAX_PACK_V2_BYTES as u64)
        .await.unwrap();
    assert_eq!(loser_pack.blake3, losing.pack_hash);
    assert!(loser_pack.bytes.len() > 10);

    // The V2 root never gains cross-vault or legacy V1 authority.
    let foreign = S3Keyspace::new(
        "akula-sync/v2-guarded-cas-probe", VaultId::new(),
    ).unwrap();
    assert_eq!(
        S3PackedHeadV2::decode(&foreign, &winning.head_bytes),
        Err(S3PackIndexV2Error::ForeignVault),
    );
    let foreign_scope = S3RemoteAnchorScope::new(
        "desktop-v2", "isolated-test-account", &profile, &foreign,
    ).unwrap();
    let before_scope_failure = b.test_io_snapshot();
    assert!(matches!(
        b.inspect_explicitly_pinned_v2_read_only(
            &foreign_scope, &winning.head, &_ack_lost.etag, Some(&first.head),
        ).await, Err(S3PinnedV2ReadError::ScopeMismatch)
    ));
    let after_scope_failure = b.test_io_snapshot();
    assert_eq!(before_scope_failure, after_scope_failure,
        "wrong connected vault must fail before making ANY network request");
    assert!(matches!(
        S3HeadPointerV1::decode(&winning.head_bytes, &ns),
        Err(S3ContractError::InvalidRoot | S3ContractError::UnsupportedRootSchema),
    ));

    // Tamper equal-length pack bytes in the throwaway test bucket. Even
    // without rewriting the immutable index the complete read MUST refuse.
    let current_packet = a.get_bounded(&winning.pack_key, MAX_PACK_V2_BYTES as u64)
        .await.unwrap();
    let altered = vec![b'!'; current_packet.bytes.len()];
    a.put_conditionally(
        &winning.pack_key, &altered,
        S3PutCondition::IfMatch(current_packet.object.etag),
    ).await.unwrap();
    assert!(read_verified_v2(&b, &ns).await.is_err());
    assert!(b.inspect_explicitly_pinned_v2_read_only(
        &scope, &winning.head, &_ack_lost.etag, Some(&first.head),
    ).await.is_err());
    // Previously recorded intent is never a user's confirmed sync baseline.
    assert!(after_restart.recover_verified(intent_winner, &ns, &b).await.is_err());
    eprintln!(
        "AKULA_V2_CAS_PROBE verified_genesis=true competing_cas_held=true \
         lost_ack_recovered_from_sqlite=true conflicting_revision_preserved=true \
         tampered_immutable_pack_rejected=true user_files_touched=0"
    );
}
