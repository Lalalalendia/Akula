//! SYNC-DELTA-01f: reuse only a persisted, scope-matched remote observation
//! anchor for read-only S3 ancestor discovery.
//!
//! The persisted remote anchor is NOT a per-document ConfirmedBaseline and
//! does NOT prove that the local vault has applied any remote revisions.
//! This adapter cannot commit an anchor, mutate files, publish S3 objects,
//! delete missing files or authorize a CAS. It returns advisory shard metadata.
//!
//! The scope's account/deployment binding must have been independently
//! verified by the caller. Comparing non-secret SDK profile details here
//! prevents accidental cross-bucket/endpoint/root reuse but does not prove
//! the identity or permissions of a remote S3 writer.

use super::s3::S3Keyspace;
use super::s3_delta_walker::{
    walk_with, S3DeltaWalkCheckpoint, S3DeltaWalkError, S3DeltaWalkPreview,
};
use super::s3_head_probe::BoundedReader;
use super::s3_manifest_delta::S3ManifestDeltaLimits;
use super::s3_remote_anchor::{
    S3RemoteAnchorError, S3RemoteAnchorJournal, S3RemoteAnchorScope,
};
use super::s3_transport::{AwsS3ProbeClient, S3ConnectionProfile};

#[derive(Debug)]
pub enum S3AnchoredWalkError {
    /// SDK client points to a different endpoint, bucket, namespace or vault.
    ClientScopeMismatch,
    /// No previously verified remote observation exists for this scope.
    /// First-time bootstrap requires a separately approved full pinning step.
    NoVerifiedAnchor,
    /// Anchor is corrupt, was replaced, or belongs to a different profile.
    Anchor(S3RemoteAnchorError),
    /// Remote HEAD/manifest is malformed, changed, or failed verification.
    Walk(S3DeltaWalkError),
    /// The exact same head was rewritten under a different S3 ETag.
    /// Treat this as possible ABA/overwrite, not an ordinary no-change sync.
    HeadRewritten,
}

impl From<S3RemoteAnchorError> for S3AnchoredWalkError {
    fn from(value: S3RemoteAnchorError) -> Self {
        Self::Anchor(value)
    }
}

impl From<S3DeltaWalkError> for S3AnchoredWalkError {
    fn from(value: S3DeltaWalkError) -> Self {
        Self::Walk(value)
    }
}

/// Read-only S3 discovery starting from an already durable, profile-bound,
/// fully pinned remote observation. The actual SDK client identity is checked
/// before loading private SQLite data or making a single network request.
///
/// A successful return is a preview only: new shard candidates still require
/// canonical pinning of revisions and blobs, DAG conflict review and File Gate.
/// Neither an unchanged remote head nor an empty delta implies local sync done.
pub async fn walk_s3_delta_from_saved_anchor(
    client: &AwsS3ProbeClient,
    scope: &S3RemoteAnchorScope,
    journal: &S3RemoteAnchorJournal,
    limits: S3ManifestDeltaLimits,
) -> Result<S3DeltaWalkPreview, S3AnchoredWalkError> {
    walk_from_anchor_with(
        client,
        client.bound_profile(),
        client.bound_keyspace(),
        scope,
        journal,
        limits,
    )
    .await
}

async fn walk_from_anchor_with<R: BoundedReader>(
    reader: &R,
    actual_profile: &S3ConnectionProfile,
    actual_keyspace: &S3Keyspace,
    scope: &S3RemoteAnchorScope,
    journal: &S3RemoteAnchorJournal,
    limits: S3ManifestDeltaLimits,
) -> Result<S3DeltaWalkPreview, S3AnchoredWalkError> {
    if !scope.matches_connection(actual_profile, actual_keyspace) {
        return Err(S3AnchoredWalkError::ClientScopeMismatch);
    }
    let anchor = journal
        .load(scope)?
        .ok_or(S3AnchoredWalkError::NoVerifiedAnchor)?;
    let preview = walk_with(
        reader,
        actual_keyspace,
        Some(S3DeltaWalkCheckpoint {
            pointer: anchor.pointer(),
            manifest_bytes: anchor.manifest_bytes(),
        }),
        limits,
    )
    .await?;

    if preview.observed_head.pointer() == anchor.pointer()
        && preview.observed_head.etag() != anchor.etag()
    {
        return Err(S3AnchoredWalkError::HeadRewritten);
    }
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::s3_manifest::S3ManifestV1;
    use super::super::s3_manifest_delta::{S3ManifestDeltaOutcome, S3ManifestSnapshotReason};
    use super::super::s3_transport::{
        S3BoundedObject, S3ObservedObject, S3TransportError, S3VerifiedHead,
        S3PinnedRevisionSnapshot,
    };
    use super::super::{OperationId, VaultId};
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::Mutex;
    use tempfile::tempdir;

    fn profile() -> S3ConnectionProfile {
        S3ConnectionProfile {
            bucket: "test-akula-anchor".into(),
            region: "us-east-1".into(),
            endpoint: Some("http://127.0.0.1:9000".into()),
            force_path_style: true,
        }
    }

    fn context() -> (S3RemoteAnchorScope, S3ConnectionProfile, S3Keyspace) {
        let keyspace = S3Keyspace::new("akula/anchor", VaultId::new()).unwrap();
        let connection = profile();
        let scope = S3RemoteAnchorScope::new(
            "desktop", "verified-minio-deployment", &connection, &keyspace,
        )
        .unwrap();
        (scope, connection, keyspace)
    }

    fn frame(
        keyspace: &S3Keyspace,
        generation: u64,
        parent: Option<&S3VerifiedHead>,
    ) -> (S3VerifiedHead, Vec<u8>) {
        let id = OperationId::new();
        let previous = parent.map(|p| {
            (p.pointer.manifest_id(), p.pointer.manifest_blake3().to_owned())
        });
        let manifest =
            S3ManifestV1::new(keyspace, id, generation, previous, vec![]).unwrap();
        let bytes = manifest.encode(keyspace).unwrap();
        let head = super::super::S3HeadPointerV1::new(
            keyspace,
            generation,
            id,
            &blake3::hash(&bytes).to_hex().to_string(),
            parent.map(|p| p.pointer.manifest_id()),
        )
        .unwrap();
        (
            S3VerifiedHead {
                pointer: head,
                etag: format!("\"etag-{generation}\""),
                manifest,
            },
            bytes,
        )
    }

    fn pinned(head: &S3VerifiedHead) -> S3PinnedRevisionSnapshot {
        let id = format!(
            "{}:{}",
            head.pointer.manifest_id(),
            head.pointer.manifest_blake3(),
        );
        S3PinnedRevisionSnapshot::for_remote_anchor_test(id, 0)
    }

    fn sample(bytes: &[u8]) -> S3BoundedObject {
        let hash = blake3::hash(bytes).to_hex().to_string();
        S3BoundedObject {
            object: S3ObservedObject {
                etag: format!("\"{hash}\""),
                byte_length: bytes.len() as u64,
            },
            blake3: hash,
            bytes: bytes.to_vec(),
        }
    }


    fn sample_head(head: &S3VerifiedHead, keyspace: &S3Keyspace) -> S3BoundedObject {
        let mut response = sample(&head.pointer.encode(keyspace).unwrap());
        response.object.etag = head.etag.clone();
        response
    }

    #[derive(Default)]
    struct ScriptedReader {
        replies: Mutex<BTreeMap<String, VecDeque<Result<S3BoundedObject, S3TransportError>>>>,
        calls: Mutex<Vec<String>>,
    }

    impl ScriptedReader {
        fn responses(&self, key: String, values: Vec<Result<S3BoundedObject, S3TransportError>>) {
            assert!(!values.is_empty());
            self.replies.lock().unwrap().insert(key, VecDeque::from(values));
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
            let mut state = self.replies.lock().unwrap();
            let values = state.get_mut(key).ok_or(S3TransportError::NotFound)?;
            let chosen = if values.len() > 1 {
                values.pop_front()
            } else {
                values.front().cloned()
            }
            .ok_or(S3TransportError::NotFound)??;
            if chosen.bytes.len() as u64 > max_bytes {
                return Err(S3TransportError::ObjectTooLarge);
            }
            Ok(chosen)
        }
    }

    fn saved(
        journal: &mut S3RemoteAnchorJournal,
        scope: &S3RemoteAnchorScope,
        head: &S3VerifiedHead,
        manifest_bytes: &[u8],
    ) {
        journal
            .record_verified_remote(
                scope,
                head,
                &pinned(head),
                &[manifest_bytes.to_vec()],
            )
            .unwrap();
    }

    #[tokio::test]
    async fn durable_unchanged_head_is_two_head_reads_without_manifest_io() {
        let (scope, profile, keyspace) = context();
        let dir = tempdir().unwrap();
        let path = dir.path().join("anchor.sqlite");
        let (h1, b1) = frame(&keyspace, 1, None);
        let mut journal = S3RemoteAnchorJournal::open(&path).unwrap();
        saved(&mut journal, &scope, &h1, &b1);
        drop(journal);
        let journal = S3RemoteAnchorJournal::open(&path).unwrap();

        let reader = ScriptedReader::default();
        reader.responses(
            keyspace.current_head_key(),
            vec![Ok(sample_head(&h1, &keyspace))],
        );

        let preview = walk_from_anchor_with(
            &reader, &profile, &keyspace, &scope, &journal,
            S3ManifestDeltaLimits::default(),
        )
        .await
        .unwrap();
        assert_eq!(preview.outcome, S3ManifestDeltaOutcome::HeadUnchanged);
        assert_eq!(preview.immutable_manifests_read, 0);
        assert_eq!(
            reader.calls(),
            vec![keyspace.current_head_key(), keyspace.current_head_key()]
        );
        // No record_verified_remote() and no file write occur in the bridge.
        assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &h1.pointer);
    }

    #[tokio::test]
    async fn two_offline_generations_start_from_exact_saved_ancestor() {
        let (scope, profile, keyspace) = context();
        let dir = tempdir().unwrap();
        let mut journal =
            S3RemoteAnchorJournal::open(&dir.path().join("anchor.sqlite")).unwrap();
        let (h1, b1) = frame(&keyspace, 1, None);
        let (h2, b2) = frame(&keyspace, 2, Some(&h1));
        let (h3, b3) = frame(&keyspace, 3, Some(&h2));
        saved(&mut journal, &scope, &h1, &b1);
        let reader = ScriptedReader::default();
        reader.responses(
            keyspace.current_head_key(),
            vec![Ok(sample_head(&h3, &keyspace))],
        );
        reader.responses(
            keyspace.immutable_manifest_key(h3.pointer.manifest_id()),
            vec![Ok(sample(&b3))],
        );
        reader.responses(
            keyspace.immutable_manifest_key(h2.pointer.manifest_id()),
            vec![Ok(sample(&b2))],
        );

        let result = walk_from_anchor_with(
            &reader, &profile, &keyspace, &scope, &journal,
            S3ManifestDeltaLimits::default(),
        )
        .await
        .unwrap();
        assert_eq!(result.immutable_manifests_read, 2);
        assert_eq!(result.observed_head.pointer(), &h3.pointer);
        assert_eq!(
            reader.calls(),
            vec![
                keyspace.current_head_key(),
                keyspace.immutable_manifest_key(h3.pointer.manifest_id()),
                keyspace.immutable_manifest_key(h2.pointer.manifest_id()),
                keyspace.current_head_key(),
            ]
        );
        // An observation is not an applied or even newly recorded baseline.
        assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &h1.pointer);
    }

    #[tokio::test]
    async fn no_record_or_corrupted_record_never_bootstraps_silently() {
        let (scope, profile, keyspace) = context();
        let dir = tempdir().unwrap();
        let mut journal =
            S3RemoteAnchorJournal::open(&dir.path().join("anchor.sqlite")).unwrap();
        let reader = ScriptedReader::default();
        assert!(matches!(
            walk_from_anchor_with(
                &reader, &profile, &keyspace, &scope, &journal,
                S3ManifestDeltaLimits::default(),
            ).await,
            Err(S3AnchoredWalkError::NoVerifiedAnchor),
        ));
        assert!(reader.calls().is_empty());

        let (h1, b1) = frame(&keyspace, 1, None);
        saved(&mut journal, &scope, &h1, &b1);
        // Simulate corrupted on-disk state from a different process.
        drop(journal);
        let db = rusqlite::Connection::open(dir.path().join("anchor.sqlite")).unwrap();
        db.execute(
            "UPDATE s3_remote_anchor_v1 SET manifest_bytes=?1",
            rusqlite::params![b"corrupt".to_vec()],
        ).unwrap();
        drop(db);
        let journal = S3RemoteAnchorJournal::open(&dir.path().join("anchor.sqlite")).unwrap();
        assert!(matches!(
            walk_from_anchor_with(
                &reader, &profile, &keyspace, &scope, &journal,
                S3ManifestDeltaLimits::default(),
            ).await,
            Err(S3AnchoredWalkError::Anchor(
                S3RemoteAnchorError::CorruptAnchor
            )),
        ));
        assert!(reader.calls().is_empty());
    }

    #[tokio::test]
    async fn connecting_different_bucket_endpoint_or_vault_never_reads_s3() {
        let (scope, profile, keyspace) = context();
        let dir = tempdir().unwrap();
        let mut journal =
            S3RemoteAnchorJournal::open(&dir.path().join("anchor.sqlite")).unwrap();
        let (h1, b1) = frame(&keyspace, 1, None);
        saved(&mut journal, &scope, &h1, &b1);
        let reader = ScriptedReader::default();
        let mut bucket = profile.clone();
        bucket.bucket = "different-bucket".into();
        let mut endpoint = profile.clone();
        endpoint.endpoint = Some("https://different.example".into());
        let other_vault = S3Keyspace::new("akula/anchor", VaultId::new()).unwrap();

        for (actual, actual_ns) in [
            (&bucket, &keyspace), (&endpoint, &keyspace), (&profile, &other_vault),
        ] {
            assert!(matches!(
                walk_from_anchor_with(
                    &reader, actual, actual_ns, &scope, &journal,
                    S3ManifestDeltaLimits::default(),
                ).await,
                Err(S3AnchoredWalkError::ClientScopeMismatch),
            ));
        }
        assert!(reader.calls().is_empty());
    }

    #[tokio::test]
    async fn same_head_republished_with_different_etag_is_a_hold() {
        let (scope, profile, keyspace) = context();
        let dir = tempdir().unwrap();
        let mut journal =
            S3RemoteAnchorJournal::open(&dir.path().join("anchor.sqlite")).unwrap();
        let (h1, b1) = frame(&keyspace, 1, None);
        saved(&mut journal, &scope, &h1, &b1);
        let mut changed = sample_head(&h1, &keyspace);
        changed.object.etag = "\"rewritten-root-etag\"".into();
        let reader = ScriptedReader::default();
        reader.responses(keyspace.current_head_key(), vec![Ok(changed)]);
        assert!(matches!(
            walk_from_anchor_with(
                &reader, &profile, &keyspace, &scope, &journal,
                S3ManifestDeltaLimits::default(),
            ).await,
            Err(S3AnchoredWalkError::HeadRewritten),
        ));
        assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &h1.pointer);
    }

    #[tokio::test]
    async fn remote_rollback_and_missing_ancestor_fail_without_writing_journal() {
        let (scope, profile, keyspace) = context();
        let dir = tempdir().unwrap();
        let mut journal =
            S3RemoteAnchorJournal::open(&dir.path().join("anchor.sqlite")).unwrap();
        let (h1, b1) = frame(&keyspace, 1, None);
        let (h2, b2) = frame(&keyspace, 2, Some(&h1));
        journal
            .record_verified_remote(&scope, &h2, &pinned(&h2), &[b2.clone(), b1])
            .unwrap();
        let reader = ScriptedReader::default();
        reader.responses(
            keyspace.current_head_key(),
            vec![Ok(sample_head(&h1, &keyspace))],
        );
        assert!(matches!(
            walk_from_anchor_with(
                &reader, &profile, &keyspace, &scope, &journal,
                S3ManifestDeltaLimits::default(),
            ).await,
            Err(S3AnchoredWalkError::Walk(S3DeltaWalkError::Delta(_))),
        ));
        assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &h2.pointer);

        let (scope, profile, keyspace) = context();
        let dir = tempdir().unwrap();
        let mut journal =
            S3RemoteAnchorJournal::open(&dir.path().join("anchor.sqlite")).unwrap();
        let (h1, b1) = frame(&keyspace, 1, None);
        let (h2, _) = frame(&keyspace, 2, Some(&h1));
        saved(&mut journal, &scope, &h1, &b1);
        let reader = ScriptedReader::default();
        reader.responses(
            keyspace.current_head_key(),
            vec![Ok(sample_head(&h2, &keyspace))],
        );
        reader.responses(
            keyspace.immutable_manifest_key(h2.pointer.manifest_id()),
            vec![Err(S3TransportError::NotFound)],
        );
        assert!(matches!(
            walk_from_anchor_with(
                &reader, &profile, &keyspace, &scope, &journal,
                S3ManifestDeltaLimits::default(),
            ).await,
            Err(S3AnchoredWalkError::Walk(S3DeltaWalkError::Read(
                S3TransportError::NotFound
            ))),
        ));
        assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &h1.pointer);
    }

    #[tokio::test]
    async fn expired_chain_stays_a_snapshot_request_and_does_not_advance_anchor() {
        let (scope, profile, keyspace) = context();
        let dir = tempdir().unwrap();
        let mut journal =
            S3RemoteAnchorJournal::open(&dir.path().join("anchor.sqlite")).unwrap();
        let (h1, b1) = frame(&keyspace, 1, None);
        let (h2, _) = frame(&keyspace, 2, Some(&h1));
        let (h3, b3) = frame(&keyspace, 3, Some(&h2));
        saved(&mut journal, &scope, &h1, &b1);
        let reader = ScriptedReader::default();
        reader.responses(
            keyspace.current_head_key(),
            vec![Ok(sample_head(&h3, &keyspace))],
        );
        reader.responses(
            keyspace.immutable_manifest_key(h3.pointer.manifest_id()),
            vec![Ok(sample(&b3))],
        );
        let result = walk_from_anchor_with(
            &reader, &profile, &keyspace, &scope, &journal,
            S3ManifestDeltaLimits {
                max_frames: 1,
                max_total_shard_refs: 256,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            result.outcome,
            S3ManifestDeltaOutcome::RequiresVerifiedSnapshot(
                S3ManifestSnapshotReason::TooManyFrames
            ),
        );
        assert_eq!(journal.load(&scope).unwrap().unwrap().pointer(), &h1.pointer);
    }
}
