//! SYNC-DELTA-01d: bounded read-only S3 walk of canonical manifest ancestors.
//!
//! Reads only current head and immutable manifest objects. Uses the same
//! HEAD -> If-Match GET -> HEAD bounded transport as the existing S3 probe.
//! Does NOT authenticate a publisher or turn metadata hints into a complete
//! verified snapshot, local File Gate permission or journal acknowledgement.
//!
//! The supplied checkpoint must be previously durably verified and bound to
//! this exact storage account, bucket, prefix and vault by the calling layer.
//! That durable checkpoint mechanism remains a prerequisite for activation.
//! Absence of an object or broken history is NEVER permission to delete data.

use super::s3::{S3HeadPointerV1, S3Keyspace};
use super::s3_head_probe::{
    check_readback, observe_with, BoundedReader, S3HeadReadError, S3ObservedHead,
};
use super::s3_manifest::MAX_MANIFEST_BYTES;
use super::s3_manifest_delta::{
    inspect_s3_manifest_delta, S3ManifestDeltaError, S3ManifestDeltaLimits,
    S3ManifestDeltaOutcome,
};
use super::s3_transport::{AwsS3ProbeClient, S3TransportError};

const MAX_WALK_MANIFEST_BYTES: usize = 16 * 1024 * 1024;

/// Already trusted, immutable checkpoint bytes; this structure itself does not
/// prove the remote storage profile binding, identity or authorization.
#[derive(Clone, Copy, Debug)]
pub struct S3DeltaWalkCheckpoint<'a> {
    pub pointer: &'a S3HeadPointerV1,
    pub manifest_bytes: &'a [u8],
}

/// Read-only candidate result from one stable remote-head observation.
/// Shard candidates must still pass the existing verified S3 pinning path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3DeltaWalkPreview {
    pub observed_head: S3ObservedHead,
    pub outcome: S3ManifestDeltaOutcome,
    pub immutable_manifests_read: usize,
    pub manifest_bytes_read: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum S3DeltaWalkError {
    Head(S3HeadReadError),
    Read(S3TransportError),
    Delta(S3ManifestDeltaError),
    /// The S3 read returned internally inconsistent object bytes/metadata.
    InvalidManifestReadback,
    /// A newer (or differently ETagged) root appeared during the chain walk.
    HeadChangedDuringWalk,
    /// Hold and request an independent verified snapshot / replan.
    TotalManifestBudgetExceeded,
}

impl From<S3HeadReadError> for S3DeltaWalkError {
    fn from(value: S3HeadReadError) -> Self {
        Self::Head(value)
    }
}
impl From<S3TransportError> for S3DeltaWalkError {
    fn from(value: S3TransportError) -> Self {
        Self::Read(value)
    }
}
impl From<S3ManifestDeltaError> for S3DeltaWalkError {
    fn from(value: S3ManifestDeltaError) -> Self {
        Self::Delta(value)
    }
}

/// Read-only network walker over the existing immutable S3ManifestV1 chain.
///
/// The latest root is observed, every required ancestor is fetched by exact
/// ID and BLAKE3 (never ListObjectsV2), and the root is checked again before
/// returning an advisory candidate plan. This does NOT call read_verified_head
/// or pin_revision_snapshot and must not be treated as a substitute for them.
pub async fn walk_s3_manifest_delta(
    client: &AwsS3ProbeClient,
    keyspace: &S3Keyspace,
    checkpoint: Option<S3DeltaWalkCheckpoint<'_>>,
    limits: S3ManifestDeltaLimits,
) -> Result<S3DeltaWalkPreview, S3DeltaWalkError> {
    walk_with(client, keyspace, checkpoint, limits).await
}

pub(super) async fn walk_with<R: BoundedReader>(
    reader: &R,
    keyspace: &S3Keyspace,
    checkpoint: Option<S3DeltaWalkCheckpoint<'_>>,
    limits: S3ManifestDeltaLimits,
) -> Result<S3DeltaWalkPreview, S3DeltaWalkError> {
    let initial = observe_with(reader, keyspace).await?;
    let checkpoint_pair = checkpoint
        .as_ref()
        .map(|entry| (entry.pointer, entry.manifest_bytes));
    let mut newest_first = Vec::new();
    let mut total_bytes = 0usize;

    let outcome = loop {
        match inspect_s3_manifest_delta(
            keyspace,
            initial.pointer(),
            checkpoint_pair,
            &newest_first,
            limits,
        )? {
            S3ManifestDeltaOutcome::NeedOlderManifest {
                manifest_id,
                expected_blake3,
                ..
            } => {
                let key = keyspace.immutable_manifest_key(manifest_id);
                let fetched = reader
                    .read_bounded(&key, MAX_MANIFEST_BYTES as u64)
                    .await?;
                check_readback(&fetched, MAX_MANIFEST_BYTES as u64)
                    .map_err(|_| S3DeltaWalkError::InvalidManifestReadback)?;
                if fetched.blake3 != expected_blake3 {
                    return Err(S3DeltaWalkError::Delta(
                        S3ManifestDeltaError::ManifestDigestMismatch,
                    ));
                }
                total_bytes = total_bytes
                    .checked_add(fetched.bytes.len())
                    .ok_or(S3DeltaWalkError::TotalManifestBudgetExceeded)?;
                if total_bytes > MAX_WALK_MANIFEST_BYTES {
                    return Err(S3DeltaWalkError::TotalManifestBudgetExceeded);
                }
                newest_first.push(fetched.bytes);
            }
            result => break result,
        }
    };

    // A consistent historical set of immutable manifests is not enough:
    // another writer may have changed the only mutable root in the meantime.
    // Compare the opaque ETag AND original bytes AND strict root pointer.
    let final_head = observe_with(reader, keyspace).await?;
    if final_head != initial {
        return Err(S3DeltaWalkError::HeadChangedDuringWalk);
    }

    Ok(S3DeltaWalkPreview {
        observed_head: initial,
        outcome,
        immutable_manifests_read: newest_first.len(),
        manifest_bytes_read: total_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::s3_manifest::{
        S3ManifestShardRefV1, S3ManifestV1,
    };
    use super::super::s3_manifest_delta::{
        S3ManifestDeltaBase, S3ManifestShardDelta, S3ManifestSnapshotReason,
    };
    use super::super::s3_transport::{
        S3BoundedObject, S3ObservedObject,
    };
    use super::super::{OperationId, RevisionId, VaultId};
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::Mutex;

    #[derive(Default)]
    struct ScriptedReader {
        replies: Mutex<BTreeMap<String, VecDeque<Result<S3BoundedObject, S3TransportError>>>>,
        calls: Mutex<Vec<String>>,
    }

    impl ScriptedReader {
        fn add(
            &self,
            key: String,
            replies: Vec<Result<S3BoundedObject, S3TransportError>>,
        ) {
            assert!(!replies.is_empty());
            self.replies.lock().unwrap().insert(key, VecDeque::from(replies));
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl BoundedReader for ScriptedReader {
        async fn read_bounded(
            &self,
            key: &str,
            max_bytes: u64,
        ) -> Result<S3BoundedObject, S3TransportError> {
            self.calls.lock().unwrap().push(key.to_owned());
            let mut replies = self.replies.lock().unwrap();
            let bucket = replies.get_mut(key).ok_or(S3TransportError::NotFound)?;
            let result = if bucket.len() > 1 {
                bucket.pop_front()
            } else {
                bucket.front().cloned()
            }
            .ok_or(S3TransportError::NotFound)??;
            if result.bytes.len() as u64 > max_bytes {
                return Err(S3TransportError::ObjectTooLarge);
            }
            Ok(result)
        }
    }

    fn ns() -> S3Keyspace {
        S3Keyspace::new("akula/delta-walker", VaultId::new()).unwrap()
    }

    fn digest(bytes: &[u8]) -> String {
        blake3::hash(bytes).to_hex().to_string()
    }

    fn bounded(bytes: &[u8]) -> S3BoundedObject {
        let blake3 = digest(bytes);
        S3BoundedObject {
            object: S3ObservedObject {
                etag: format!("\"{blake3}\""),
                byte_length: bytes.len() as u64,
            },
            bytes: bytes.to_vec(),
            blake3,
        }
    }

    fn frame(
        ns: &S3Keyspace,
        generation: u64,
        parent: Option<&S3HeadPointerV1>,
        shards: Vec<S3ManifestShardRefV1>,
    ) -> (Vec<u8>, S3HeadPointerV1) {
        let id = OperationId::new();
        let prev = parent.map(|v| (v.manifest_id(), v.manifest_blake3().to_owned()));
        let manifest = S3ManifestV1::new(ns, id, generation, prev, shards).unwrap();
        let bytes = manifest.encode(ns).unwrap();
        let head = S3HeadPointerV1::new(
            ns,
            generation,
            id,
            &digest(&bytes),
            parent.map(S3HeadPointerV1::manifest_id),
        )
        .unwrap();
        (bytes, head)
    }

    fn shard_one(revision: RevisionId) -> S3ManifestShardRefV1 {
        S3ManifestShardRefV1 {
            shard_id: OperationId::new(),
            shard_blake3: digest(revision.to_string().as_bytes()),
            count: 1,
            first_revision: revision,
            last_revision: revision,
        }
    }

    fn stable_root(reader: &ScriptedReader, ns: &S3Keyspace, head: &S3HeadPointerV1) {
        let bytes = head.encode(ns).unwrap();
        reader.add(ns.current_head_key(), vec![Ok(bounded(&bytes))]);
    }

    fn immutable(
        reader: &ScriptedReader,
        ns: &S3Keyspace,
        head: &S3HeadPointerV1,
        bytes: &[u8],
    ) {
        reader.add(ns.immutable_manifest_key(head.manifest_id()), vec![Ok(bounded(bytes))]);
    }

    #[tokio::test]
    async fn confirmed_unchanged_head_downloads_no_manifests() {
        let ns = ns();
        let (base_bytes, head) = frame(&ns, 1, None, vec![]);
        let reader = ScriptedReader::default();
        stable_root(&reader, &ns, &head);
        let receipt = walk_with(
            &reader,
            &ns,
            Some(S3DeltaWalkCheckpoint { pointer: &head, manifest_bytes: &base_bytes }),
            S3ManifestDeltaLimits::default(),
        ).await.unwrap();

        assert_eq!(receipt.outcome, S3ManifestDeltaOutcome::HeadUnchanged);
        assert_eq!(receipt.immutable_manifests_read, 0);
        assert_eq!(receipt.manifest_bytes_read, 0);
        assert_eq!(reader.calls(), vec![ns.current_head_key(), ns.current_head_key()]);
    }

    #[tokio::test]
    async fn two_offline_generations_read_only_missing_manifest_objects() {
        let ns = ns();
        let original = shard_one(RevisionId::new());
        let (base_bytes, base) = frame(&ns, 1, None, vec![original.clone()]);
        let (b2, h2) = frame(&ns, 2, Some(&base), vec![original.clone()]);
        let (b3, h3) = frame(&ns, 3, Some(&h2), vec![original]);
        let reader = ScriptedReader::default();
        stable_root(&reader, &ns, &h3);
        immutable(&reader, &ns, &h2, &b2);
        immutable(&reader, &ns, &h3, &b3);

        let result = walk_with(
            &reader,
            &ns,
            Some(S3DeltaWalkCheckpoint { pointer: &base, manifest_bytes: &base_bytes }),
            S3ManifestDeltaLimits::default(),
        ).await.unwrap();
        assert_eq!(
            result.outcome,
            S3ManifestDeltaOutcome::Complete {
                generations_traversed: 2,
                base: S3ManifestDeltaBase::VerifiedCheckpoint,
                shards: S3ManifestShardDelta {
                    candidate_shards: vec![],
                    reused_shards: 1,
                    superseded_shards: 0,
                    requires_verified_revision_retention: false,
                },
            }
        );
        assert_eq!(result.immutable_manifests_read, 2);
        assert_eq!(result.manifest_bytes_read, b3.len() + b2.len());
        assert_eq!(
            reader.calls(),
            vec![
                ns.current_head_key(),
                ns.immutable_manifest_key(h3.manifest_id()),
                ns.immutable_manifest_key(h2.manifest_id()),
                ns.current_head_key(),
            ]
        );
    }

    #[tokio::test]
    async fn first_connection_reads_to_genesis_without_file_write_permission() {
        let ns = ns();
        let shard = shard_one(RevisionId::new());
        let (bytes, head) = frame(&ns, 1, None, vec![shard.clone()]);
        let reader = ScriptedReader::default();
        stable_root(&reader, &ns, &head);
        immutable(&reader, &ns, &head, &bytes);

        let result = walk_with(
            &reader, &ns, None, S3ManifestDeltaLimits::default(),
        ).await.unwrap();
        assert_eq!(
            result.outcome,
            S3ManifestDeltaOutcome::Complete {
                generations_traversed: 1,
                base: S3ManifestDeltaBase::Genesis,
                shards: S3ManifestShardDelta {
                    candidate_shards: vec![shard],
                    reused_shards: 0,
                    superseded_shards: 0,
                    requires_verified_revision_retention: false,
                },
            }
        );
        assert_eq!(result.immutable_manifests_read, 1);
        assert_eq!(reader.calls().len(), 3);
    }

    #[tokio::test]
    async fn absent_ancestor_is_error_not_a_deletion_or_empty_snapshot() {
        let ns = ns();
        let (cp_bytes, cp) = frame(&ns, 1, None, vec![]);
        let (b2, h2) = frame(&ns, 2, Some(&cp), vec![]);
        let (b3, h3) = frame(&ns, 3, Some(&h2), vec![]);
        let reader = ScriptedReader::default();
        stable_root(&reader, &ns, &h3);
        immutable(&reader, &ns, &h3, &b3);
        reader.add(
            ns.immutable_manifest_key(h2.manifest_id()),
            vec![Err(S3TransportError::NotFound)],
        );

        let result = walk_with(
            &reader,
            &ns,
            Some(S3DeltaWalkCheckpoint { pointer: &cp, manifest_bytes: &cp_bytes }),
            S3ManifestDeltaLimits::default(),
        ).await;
        assert_eq!(result, Err(S3DeltaWalkError::Read(S3TransportError::NotFound)));
        let _ = b2;
    }

    #[tokio::test]
    async fn missing_head_is_error_and_never_bootstrap_authority() {
        let ns = ns();
        let reader = ScriptedReader::default();
        reader.add(ns.current_head_key(), vec![Err(S3TransportError::NotFound)]);
        assert_eq!(
            walk_with(&reader, &ns, None, S3ManifestDeltaLimits::default()).await,
            Err(S3DeltaWalkError::Head(S3HeadReadError::Transport(
                S3TransportError::NotFound,
            ))),
        );
    }

    #[tokio::test]
    async fn tampered_ancestor_fails_hash_check_before_candidate_plan() {
        let ns = ns();
        let (cp_bytes, cp) = frame(&ns, 1, None, vec![]);
        let (b2, h2) = frame(&ns, 2, Some(&cp), vec![]);
        let (b3, h3) = frame(&ns, 3, Some(&h2), vec![]);
        let reader = ScriptedReader::default();
        stable_root(&reader, &ns, &h3);
        immutable(&reader, &ns, &h3, &b3);
        let mut damaged = b2;
        damaged.push(b' ');
        immutable(&reader, &ns, &h2, &damaged);
        assert_eq!(
            walk_with(
                &reader,
                &ns,
                Some(S3DeltaWalkCheckpoint { pointer: &cp, manifest_bytes: &cp_bytes }),
                S3ManifestDeltaLimits::default(),
            ).await,
            Err(S3DeltaWalkError::Delta(S3ManifestDeltaError::ManifestDigestMismatch)),
        );
    }

    #[tokio::test]
    async fn concurrent_cas_change_rejects_completed_historical_chain() {
        let ns = ns();
        let (bytes, head) = frame(&ns, 1, None, vec![]);
        let (_, changed_head) = frame(&ns, 2, Some(&head), vec![]);
        let reader = ScriptedReader::default();
        reader.add(
            ns.current_head_key(),
            vec![
                Ok(bounded(&head.encode(&ns).unwrap())),
                Ok(bounded(&changed_head.encode(&ns).unwrap())),
            ],
        );
        immutable(&reader, &ns, &head, &bytes);
        assert_eq!(
            walk_with(&reader, &ns, None, S3ManifestDeltaLimits::default()).await,
            Err(S3DeltaWalkError::HeadChangedDuringWalk),
        );
    }

    #[tokio::test]
    async fn identical_pointer_bytes_with_different_provider_etag_fails() {
        let ns = ns();
        let (bytes, head) = frame(&ns, 1, None, vec![]);
        let first = bounded(&head.encode(&ns).unwrap());
        let mut second = first.clone();
        second.object.etag = "\"changed-etag\"".into();
        let reader = ScriptedReader::default();
        reader.add(ns.current_head_key(), vec![Ok(first), Ok(second)]);
        immutable(&reader, &ns, &head, &bytes);
        assert_eq!(
            walk_with(&reader, &ns, None, S3ManifestDeltaLimits::default()).await,
            Err(S3DeltaWalkError::HeadChangedDuringWalk),
        );
    }

    #[tokio::test]
    async fn finite_depth_stops_before_missing_older_blob_and_rechecks_root() {
        let ns = ns();
        let (cp_bytes, cp) = frame(&ns, 1, None, vec![]);
        let (_, mid) = frame(&ns, 2, Some(&cp), vec![]);
        let (current_bytes, current) = frame(&ns, 3, Some(&mid), vec![]);
        let reader = ScriptedReader::default();
        stable_root(&reader, &ns, &current);
        immutable(&reader, &ns, &current, &current_bytes);
        let result = walk_with(
            &reader,
            &ns,
            Some(S3DeltaWalkCheckpoint { pointer: &cp, manifest_bytes: &cp_bytes }),
            S3ManifestDeltaLimits { max_frames: 1, max_total_shard_refs: 256 },
        ).await.unwrap();
        assert_eq!(
            result.outcome,
            S3ManifestDeltaOutcome::RequiresVerifiedSnapshot(
                S3ManifestSnapshotReason::TooManyFrames,
            )
        );
        assert_eq!(result.immutable_manifests_read, 1);
        assert_eq!(reader.calls().len(), 3);
    }

    #[tokio::test]
    async fn a_remote_root_in_another_vault_never_grants_a_delta() {
        let ns = ns();
        let wrong = S3Keyspace::new("akula/delta-walker", VaultId::new()).unwrap();
        let (_, foreign) = frame(&wrong, 1, None, vec![]);
        let reader = ScriptedReader::default();
        reader.add(ns.current_head_key(), vec![Ok(bounded(&foreign.encode(&wrong).unwrap()))]);
        assert!(matches!(
            walk_with(&reader, &ns, None, S3ManifestDeltaLimits::default()).await,
            Err(S3DeltaWalkError::Head(S3HeadReadError::InvalidPointer(_))),
        ));
    }

    #[tokio::test]
    async fn inconsistent_transport_readback_is_not_accepted() {
        let ns = ns();
        let (bytes, head) = frame(&ns, 1, None, vec![]);
        let reader = ScriptedReader::default();
        stable_root(&reader, &ns, &head);
        let mut forged = bounded(&bytes);
        forged.object.byte_length += 1;
        reader.add(
            ns.immutable_manifest_key(head.manifest_id()),
            vec![Ok(forged)],
        );
        assert_eq!(
            walk_with(&reader, &ns, None, S3ManifestDeltaLimits::default()).await,
            Err(S3DeltaWalkError::InvalidManifestReadback),
        );
    }

    /// Isolated disposable MinIO only: publishes three immutable manifests,
    /// with server-side conditional head CAS, then reads two missing ancestors.
    /// No native vault or user storage is ever accessed or modified.
    #[tokio::test]
    #[ignore = "requires localhost disposable MinIO and test-only credentials"]
    async fn live_minio_delta_walker_three_generations() {
        use super::super::s3_transport::{S3ConnectionProfile, S3PutCondition};

        let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT")
            .expect("only isolated localhost MinIO is allowed");
        assert!(
            endpoint.starts_with("http://127.0.0.1:")
                || endpoint.starts_with("http://localhost:"),
        );
        let bucket = format!("akula-delta-{}", uuid::Uuid::new_v4().simple());
        let region = "us-east-1";

        // Create only a disposable bucket on the validated localhost endpoint.
        let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(region))
            .load()
            .await;
        let sdk = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::config::Builder::from(&shared)
                .endpoint_url(endpoint.clone())
                .force_path_style(true)
                .build(),
        );
        sdk.create_bucket().bucket(bucket.clone()).send().await.unwrap();

        let layout = ns();
        let profile = S3ConnectionProfile {
            bucket: bucket.clone(),
            region: region.into(),
            endpoint: Some(endpoint),
            force_path_style: true,
        };
        let client = AwsS3ProbeClient::connect(
            profile.clone(),
            layout.clone(),
        )
        .await
        .unwrap();

        let (b1, h1) = frame(&layout, 1, None, vec![]);
        let (b2, h2) = frame(&layout, 2, Some(&h1), vec![]);
        let (b3, h3) = frame(&layout, 3, Some(&h2), vec![]);

        for (head, body) in [(&h1, &b1), (&h2, &b2), (&h3, &b3)] {
            client
                .put_conditionally(
                    &layout.immutable_manifest_key(head.manifest_id()),
                    body,
                    S3PutCondition::IfAbsent,
                )
                .await
                .unwrap();
        }

        let p1 = client
            .put_conditionally(
                &layout.current_head_key(),
                &h1.encode(&layout).unwrap(),
                S3PutCondition::IfAbsent,
            )
            .await
            .unwrap();

        let unchanged = walk_s3_manifest_delta(
            &client,
            &layout,
            Some(S3DeltaWalkCheckpoint {
                pointer: &h1,
                manifest_bytes: &b1,
            }),
            S3ManifestDeltaLimits::default(),
        )
        .await
        .unwrap();
        assert_eq!(unchanged.outcome, S3ManifestDeltaOutcome::HeadUnchanged);
        assert_eq!(unchanged.immutable_manifests_read, 0);

        // Fully pin the remote genesis with the existing verification path,
        // then persist only the remote observation anchor in private SQLite.
        // The bridge must *not* create or advance a document ConfirmedBaseline.
        let private_dir = tempfile::tempdir().unwrap();
        let anchor_db = private_dir.path().join("test-s3-anchor.sqlite");
        let scope = super::super::s3_remote_anchor::S3RemoteAnchorScope::new(
            "minio-demo",
            "verified-disposable-ci-deployment",
            &profile,
            &layout,
        ).unwrap();
        let verified = client.read_verified_head().await.unwrap().unwrap();
        let pinned = client.pin_revision_snapshot().await.unwrap().unwrap();
        let mut journal = super::super::s3_remote_anchor::S3RemoteAnchorJournal::open(
            &anchor_db,
        ).unwrap();
        journal.record_verified_remote(
            &scope, &verified, &pinned, &[b1.clone()],
        ).unwrap();
        drop(journal);
        let journal = super::super::s3_remote_anchor::S3RemoteAnchorJournal::open(
            &anchor_db,
        ).unwrap();
        let from_sqlite = super::super::s3_anchored_delta::walk_s3_delta_from_saved_anchor(
            &client, &scope, &journal, S3ManifestDeltaLimits::default(),
        ).await.unwrap();
        assert_eq!(from_sqlite.outcome, S3ManifestDeltaOutcome::HeadUnchanged);
        assert_eq!(from_sqlite.immutable_manifests_read, 0);

        let p2 = client
            .put_conditionally(
                &layout.current_head_key(),
                &h2.encode(&layout).unwrap(),
                S3PutCondition::IfMatch(p1.etag),
            )
            .await
            .unwrap();
        client
            .put_conditionally(
                &layout.current_head_key(),
                &h3.encode(&layout).unwrap(),
                S3PutCondition::IfMatch(p2.etag),
            )
            .await
            .unwrap();

        let result = walk_s3_manifest_delta(
            &client,
            &layout,
            Some(S3DeltaWalkCheckpoint {
                pointer: &h1,
                manifest_bytes: &b1,
            }),
            S3ManifestDeltaLimits::default(),
        )
        .await
        .unwrap();

        assert_eq!(result.observed_head.pointer(), &h3);
        assert_eq!(result.immutable_manifests_read, 2);
        let anchored_after_offline = super::super::s3_anchored_delta::walk_s3_delta_from_saved_anchor(
            &client, &scope, &journal, S3ManifestDeltaLimits::default(),
        ).await.unwrap();
        assert_eq!(anchored_after_offline.observed_head.pointer(), &h3);
        assert_eq!(anchored_after_offline.immutable_manifests_read, 2);
        // This API is read-only: the durable SQLite anchor stays on genesis.
        assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &h1);
        assert_eq!(
            result.outcome,
            S3ManifestDeltaOutcome::Complete {
                generations_traversed: 2,
                base: S3ManifestDeltaBase::VerifiedCheckpoint,
                shards: S3ManifestShardDelta {
                    candidate_shards: vec![],
                    reused_shards: 0,
                    superseded_shards: 0,
                    requires_verified_revision_retention: false,
                },
            }
        );
    }

}
