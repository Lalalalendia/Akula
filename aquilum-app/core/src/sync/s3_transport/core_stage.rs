//! SYNC-05e-1: verified S3 content -> local immutable cache and Staged journal.
//!
//! This is the first asynchronous S3-to-Core handoff, NOT a sync worker.
//! The manifest-backed revision metadata must already have been fully checked.
//! This module never changes the user vault, publishes a cloud object, commits a
//! baseline or grants a File Gate capability. The existing host-admitted,
//! additive-only create path is a separate, unconfirmed operation.
//!
//! A local workspace registration ID is not a shared cross-device VaultId.
//! Binding two independent registered workspaces to one remote sync identity
//! requires a distinct host-owned, durable profile binding (SYNC-05e-2).

use super::{
    AwsS3ProbeClient, S3BoundedObject, S3HeadError, S3TransportError,
    S3VerifiedRevisionMetadataStore, MAX_CORE_STAGE_BLOB_BYTES,
};
use crate::sync::{
    DagStoreError, JournalAction, JournalError, JournalPhase, OperationIntent,
    RemoteStore, RemoteStoreError, RevisionOperation, RevisionRecord,
    S3HeadPointerV1, SqliteDagStore, SqliteSyncJournal, StagedDownload,
};

#[derive(Debug)]
pub enum S3CoreStageError {
    InvalidIntent,
    InvalidRevision,
    RemoteHeadChanged,
    IntegrityMismatch,
    Transport(S3TransportError),
    Manifest(S3HeadError),
    Provider(RemoteStoreError),
    Journal(JournalError),
    Cache(DagStoreError),
}

impl From<S3TransportError> for S3CoreStageError {
    fn from(error: S3TransportError) -> Self {
        Self::Transport(error)
    }
}
impl From<S3HeadError> for S3CoreStageError {
    fn from(error: S3HeadError) -> Self {
        Self::Manifest(error)
    }
}
impl From<RemoteStoreError> for S3CoreStageError {
    fn from(error: RemoteStoreError) -> Self {
        Self::Provider(error)
    }
}
impl From<JournalError> for S3CoreStageError {
    fn from(error: JournalError) -> Self {
        Self::Journal(error)
    }
}
impl From<DagStoreError> for S3CoreStageError {
    fn from(error: DagStoreError) -> Self {
        Self::Cache(error)
    }
}

/// A transfer is eligible for this narrow first-sync slice only when the
/// verified revision is a Content operation for the exact path, document,
/// revision and BLAKE3 object. Rename and tombstone proposals are not creates.
/// This check never decides whether the destination path is absent.
fn validate_additive_intent(
    keyspace: &crate::sync::S3Keyspace,
    intent: &OperationIntent,
    revision: &RevisionRecord,
) -> Result<(), S3CoreStageError> {
    if intent.vault_id != keyspace.vault_id()
        || intent.action != JournalAction::StageDownload
        || intent.remote_guard.is_some()
        || intent.expected_baseline.is_some()
        || intent.document_id != revision.document_id
        || intent.revision_id != revision.id
    {
        return Err(S3CoreStageError::InvalidIntent);
    }
    let RevisionOperation::Content { path, blob_hash } = &revision.operation else {
        return Err(S3CoreStageError::InvalidIntent);
    };
    if *path != intent.path || *blob_hash != intent.expected_content_hash {
        return Err(S3CoreStageError::InvalidIntent);
    }
    let expected_key = keyspace
        .immutable_blob_key(blob_hash)
        .map_err(|_| S3CoreStageError::InvalidIntent)?;
    if intent.remote_object_id != expected_key {
        return Err(S3CoreStageError::InvalidIntent);
    }
    Ok(())
}

impl AwsS3ProbeClient {
    /// Read the small mutable S3 pointer with HEAD/If-Match GET/HEAD and
    /// require this exact complete immutable snapshot, without inferring a
    /// deletion or treating an absent root as an empty vault.
    pub(super) async fn core_stage_current_head(
        &self,
        verified: &S3VerifiedRevisionMetadataStore,
    ) -> Result<S3BoundedObject, S3CoreStageError> {
        let current = self.get_bounded(&self.keyspace.current_head_key(), 4096).await?;
        let pointer = S3HeadPointerV1::decode(&current.bytes, &self.keyspace)
            .map_err(|_| S3CoreStageError::RemoteHeadChanged)?;
        let identity = format!("{}:{}", pointer.manifest_id(), pointer.manifest_blake3());
        if identity != verified.snapshot_id() {
            return Err(S3CoreStageError::RemoteHeadChanged);
        }
        Ok(current)
    }

    /// Materialize one S3 Content revision in the Core-owned immutable blob
    /// cache and WAL/FULL operation journal. No local note is touched.
    ///
    /// The caller must provide a fully verified current S3 metadata view,
    /// a stable app-private journal/cache, and an intent from its host-owned
    /// profile. The Core UI/workspace binding is NOT established by this API.
    /// Only the existing host-admitted additive local create may later consume
    /// these staged bytes; its successful return is still not a baseline.
    ///
    /// Stage immutable Markdown/attachments up to the native no-replace
    /// File Gate limit (32 MiB), with HEAD / guarded GET / HEAD + BLAKE3.
    /// A larger object requires a future direct-to-staging streaming path.
    pub async fn stage_verified_s3_content_for_core(
        &self,
        verified: &S3VerifiedRevisionMetadataStore,
        journal: &mut SqliteSyncJournal,
        cache: &mut SqliteDagStore,
        intent: OperationIntent,
        max_blob_bytes: u64,
    ) -> Result<StagedDownload, S3CoreStageError> {
        if max_blob_bytes == 0 || max_blob_bytes > MAX_CORE_STAGE_BLOB_BYTES {
            return Err(S3CoreStageError::InvalidIntent);
        }
        let inventory = verified.verified_revision_inventory()?;
        if !inventory.complete
            || inventory.vault_id != self.keyspace.vault_id()
            || intent.vault_id != inventory.vault_id
            || !inventory.revisions.contains(&intent.revision_id)
        {
            return Err(S3CoreStageError::InvalidIntent);
        }

        let revision_key = format!("revisions/{}", intent.revision_id);
        let metadata = verified.read_bounded(&revision_key, 64 * 1024)?;
        let revision: RevisionRecord = serde_json::from_slice(&metadata)
            .map_err(|_| S3CoreStageError::InvalidRevision)?;
        validate_additive_intent(&self.keyspace, &intent, &revision)?;

        // Each verified S3 read uses an SDK-enforced If-Match condition and
        // bounded streaming, and the root is also rechecked across the blob
        // transfer. A new head causes HOLD, not a silent fallback to a stale
        // pointer. These checks are staging evidence, not a global OS lease.
        let before = self.core_stage_current_head(verified).await?;
        let payload = self.get_immutable_blob_for_core_stage(
            &intent.expected_content_hash, max_blob_bytes,
        ).await?;
        if payload.blake3 != intent.expected_content_hash {
            return Err(S3CoreStageError::IntegrityMismatch);
        }
        let after = self.core_stage_current_head(verified).await?;
        if before != after {
            return Err(S3CoreStageError::RemoteHeadChanged);
        }

        // Persist the operation intent before the cache becomes eligible for
        // staging. No provider ACK or local note write has been committed.
        let phase = journal.begin(intent.clone())?;
        if !matches!(phase, JournalPhase::Pending | JournalPhase::Staged) {
            return Err(S3CoreStageError::InvalidIntent);
        }
        let stored_hash = cache.store_blob(&payload.bytes)?;
        if stored_hash != intent.expected_content_hash
            || cache.verified_blob(&stored_hash)? != payload.bytes
        {
            return Err(S3CoreStageError::IntegrityMismatch);
        }
        journal.mark_staged(intent.operation_id, &payload.bytes)?;

        Ok(StagedDownload {
            content_hash: stored_hash,
            byte_length: payload.bytes.len() as u64,
            // An opaque server validator of the immutable blob, not a
            // confirmed user document revision or durable baseline.
            observed_provider_revision: payload.object.etag,
            was_already_staged: phase == JournalPhase::Staged,
        })
    }
}

#[cfg(test)]
mod tests { include!("core_stage_e2e01_tests.rs");
    use super::*;
    use crate::sync::{
        DocumentId, OperationId, RevisionId, S3ConnectionProfile, S3HeadJournal,
        S3HeadPointerV1, S3HeadPublication, S3Keyspace, S3ManifestShardV1,
        S3ManifestV1, S3PutCondition, S3RevisionRefV1, SyncRelativePath,
        VaultId, VaultPortError,
    };

    fn content_fixture() -> (S3Keyspace, OperationIntent, RevisionRecord) {
        let vault = VaultId::new();
        let ns = S3Keyspace::new("akula-sync/05e-test", vault).unwrap();
        let content = b"bounded Markdown\n";
        let hash = blake3::hash(content).to_hex().to_string();
        let revision = RevisionRecord {
            id: RevisionId::new(),
            document_id: DocumentId::new(),
            parents: vec![],
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/test.md").unwrap(),
                blob_hash: hash.clone(),
            },
        };
        let intent = OperationIntent {
            operation_id: OperationId::new(),
            vault_id: vault,
            document_id: revision.document_id,
            profile_id: "test-s3-profile".into(),
            revision_id: revision.id,
            action: JournalAction::StageDownload,
            path: SyncRelativePath::parse("notes/test.md").unwrap(),
            remote_object_id: ns.immutable_blob_key(&hash).unwrap(),
            remote_guard: None,
            expected_content_hash: hash,
            expected_baseline: None,
        };
        (ns, intent, revision)
    }

    // Pure validation must not construct a network client or need credentials.
    // A value-only check is independent from AWS SDK behavior versions.

    #[test]
    fn additive_gate_rejects_cross_vault_and_wrong_revision_or_path() {
        let (ns, intent, record) = content_fixture();
        let keyspace = ns;
        assert!(validate_additive_intent(&keyspace, &intent, &record).is_ok());

        let mut foreign = intent.clone();
        foreign.vault_id = VaultId::new();
        assert!(matches!(
            validate_additive_intent(&keyspace, &foreign, &record),
            Err(S3CoreStageError::InvalidIntent)
        ));
        let mut wrong_path = intent.clone();
        wrong_path.path = SyncRelativePath::parse("notes/other.md").unwrap();
        assert!(validate_additive_intent(&keyspace, &wrong_path, &record).is_err());
        let mut wrong_revision = intent.clone();
        wrong_revision.revision_id = RevisionId::new();
        assert!(validate_additive_intent(&keyspace, &wrong_revision, &record).is_err());
        let mut wrong_blob_key = intent.clone();
        wrong_blob_key.remote_object_id = keyspace.current_head_key();
        assert!(validate_additive_intent(&keyspace, &wrong_blob_key, &record).is_err());
        let mut with_baseline = intent.clone();
        with_baseline.expected_baseline = Some(RevisionId::new());
        assert!(validate_additive_intent(&keyspace, &with_baseline, &record).is_err());

        let mut rename = record.clone();
        rename.operation = RevisionOperation::Rename {
            from: SyncRelativePath::parse("notes/old.md").unwrap(),
            to: intent.path.clone(),
            blob_hash: intent.expected_content_hash.clone(),
        };
        assert!(validate_additive_intent(&keyspace, &intent, &rename).is_err());
        let mut tombstone = record;
        tombstone.operation = RevisionOperation::Tombstone { path: intent.path.clone() };
        assert!(validate_additive_intent(&keyspace, &intent, &tombstone).is_err());
    }

    #[tokio::test]
    #[ignore = "requires isolated loopback MinIO"]
    async fn live_minio_two_core_verified_stage_and_no_replace() {
        verify_live_core_roundtrip("Проверенный S3-текст\n".as_bytes(), 4096).await;
    }

    #[tokio::test]
    #[ignore = "requires isolated loopback MinIO"]
    async fn live_minio_large_blob_through_verified_two_core_stage_and_no_replace() {
        // Intentionally larger than the metadata/diagnostic 1 MiB ceiling.
        // Deterministic content and a partial final SDK chunk exercise bounds.
        let bytes = vec![b'A'; 3 * 1024 * 1024 + 17];
        verify_live_core_roundtrip(&bytes, bytes.len() as u64).await;
    }

    async fn verify_live_core_roundtrip(bytes: &[u8], max_blob_bytes: u64) {
        use crate::app_core::{Core, CoreEvent};
        use crate::sync::{JournalError, RegisteredReadAdmission};
        use crate::sync::profile_binding::SqliteRegisteredS3Bindings;
        use crate::sync::filegate_custody_identity::SqliteCustodyIdentityStore;
        use crate::sync::filegate_lock_custody::scoped_leaf;
        use crate::sync::filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME};
        use aws_sdk_s3::error::ProvideErrorMetadata;
        use std::fs;
        use std::sync::Arc;

        let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").unwrap();
        assert!(
            endpoint.starts_with("http://127.0.0.1:")
                || endpoint.starts_with("http://localhost:")
        );
        let dir = tempfile::tempdir().unwrap();
        let root_a = dir.path().join("source");
        let root_b = dir.path().join("receiver");
        let app_a = dir.path().join("app-a");
        let app_b = dir.path().join("app-b");
        fs::create_dir_all(root_a.join("notes")).unwrap();
        fs::create_dir_all(root_b.join("notes")).unwrap();
        fs::create_dir_all(&app_a).unwrap();
        fs::create_dir_all(&app_b).unwrap();
        let first_core = Core::open(&app_a, Arc::new(|_: CoreEvent| {}));
        let second_core = Core::open(&app_b, Arc::new(|_: CoreEvent| {}));
        first_core.ui_state.resolve_workspace(root_a.to_str().unwrap(), 1).unwrap();
        second_core.ui_state.resolve_workspace(root_b.to_str().unwrap(), 2).unwrap();
        let source_id = first_core.ui_state.list_workspaces().unwrap()[0].id;
        let target_id = second_core.ui_state.list_workspaces().unwrap()[0].id;
        assert_ne!(source_id, target_id, "two independent local Core registrations");

        let relative = SyncRelativePath::parse("notes/remote.md").unwrap();
        fs::write(root_a.join(relative.as_str()), bytes).unwrap();
        // Only a read-only preflight for the source fixture; a production
        // uploader must separately prove its guarded mutation/source grant.
        let captured = RegisteredReadAdmission::open(&first_core, &root_a)
            .unwrap().read_closed_preflight(&relative, max_blob_bytes).unwrap();
        assert_eq!(captured.bytes, bytes);

        // The shared cloud UUID must differ from both independent machine-
        // local workspace registrations. No aliasing is permitted.
        let vault = VaultId::new();
        assert_ne!(vault.as_uuid(), source_id);
        assert_ne!(vault.as_uuid(), target_id);
        let ns = S3Keyspace::new("akula-sync/core-stage-ci", vault).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "akula-core-stage-test".into(),
            region: "us-east-1".into(),
            endpoint: Some(endpoint),
            force_path_style: true,
        };
        let writer = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
        let reader = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
        // The MinIO workflow runs several live tests against one server.
        // A bucket already owned by these isolated test credentials is fine:
        // each fixture uses a fresh random vault keyspace. Any other S3
        // response must still fail rather than masking an authorization error.
        match writer.client.create_bucket().bucket("akula-core-stage-test").send().await {
            Ok(_) => {}
            Err(error) if error.as_service_error()
                .and_then(|service| service.code()) == Some("BucketAlreadyOwnedByYou") => {}
            Err(error) => panic!("test bucket provisioning failed: {error}"),
        }

        let blob_hash = captured.content_hash;
        let blob_key = ns.immutable_blob_key(&blob_hash).unwrap();
        if captured.bytes.len() as u64 <= super::super::MAX_PROBE_BYTES {
            writer.put_conditionally(&blob_key, &captured.bytes, S3PutCondition::IfAbsent)
                .await.unwrap();
        } else {
            // Test fixture provisioning only: exercise the PRODUCTION large
            // download path below, not the already separately tested multipart
            // uploader. The canonical immutable BLAKE3 key remains unchanged.
            writer.client.put_object().bucket(&profile.bucket).key(&blob_key)
                .if_none_match("*")
                .body(aws_sdk_s3::primitives::ByteStream::from(captured.bytes.clone()))
                .send().await.unwrap();
        }
        let record = RevisionRecord {
            id: RevisionId::new(),
            document_id: DocumentId::new(),
            parents: vec![],
            operation: RevisionOperation::Content {
                path: relative.clone(),
                blob_hash: blob_hash.clone(),
            },
        };
        let metadata = serde_json::to_vec(&record).unwrap();
        writer.put_conditionally(
            &ns.immutable_revision_key(record.id), &metadata, S3PutCondition::IfAbsent,
        ).await.unwrap();
        let shard = S3ManifestShardV1::new(
            &ns,
            OperationId::new(),
            vec![S3RevisionRefV1 {
                revision_id: record.id,
                document_id: record.document_id,
                revision_blake3: blake3::hash(&metadata).to_hex().to_string(),
                blob_bytes: Some(bytes.len() as u64),
            }],
        ).unwrap();
        writer.store_manifest_object(shard.shard_id, &shard.encode(&ns).unwrap())
            .await.unwrap();
        let manifest = S3ManifestV1::new(
            &ns,
            OperationId::new(),
            1,
            None,
            vec![shard.reference(&ns).unwrap()],
        ).unwrap();
        writer.store_manifest_object(
            manifest.manifest_id, &manifest.encode(&ns).unwrap(),
        ).await.unwrap();
        let head = S3HeadPointerV1::new(
            &ns, 1, manifest.manifest_id, &manifest.digest(&ns).unwrap(), None,
        ).unwrap();
        let mut cloud_journal = S3HeadJournal::open(&app_a.join("s3-head.sqlite")).unwrap();
        assert_eq!(
            writer.publish_head_cas(&mut cloud_journal, manifest.manifest_id, head, None)
                .await.unwrap(),
            S3HeadPublication::Published
        );

        let verified = reader.materialize_verified_revision_metadata_store(256 * 1024)
            .await.unwrap().unwrap();
        let operation_id = OperationId::new();
        let intent = OperationIntent {
            operation_id,
            vault_id: vault,
            document_id: record.document_id,
            profile_id: "real-minio-test-profile".into(),
            revision_id: record.id,
            action: JournalAction::StageDownload,
            path: relative.clone(),
            remote_object_id: blob_key.clone(),
            remote_guard: None,
            expected_content_hash: blob_hash.clone(),
            expected_baseline: None,
        };
        let mut journal = SqliteSyncJournal::open(&app_b.join("ops.sqlite")).unwrap();
        let mut cache = SqliteDagStore::open(&app_b.join("blobs.sqlite")).unwrap();
        assert!(!root_b.join(relative.as_str()).exists());
        // An over-budget request must fail before the journal is changed,
        // regardless of any provider-reported Content-Length.
        assert!(matches!(
            reader.stage_verified_s3_content_for_core(
                &verified, &mut journal, &mut cache,
                intent.clone(), MAX_CORE_STAGE_BLOB_BYTES + 1,
            ).await,
            Err(S3CoreStageError::InvalidIntent)
        ));
        assert!(journal.get(operation_id).unwrap().is_none());
        if bytes.len() as u64 > super::super::MAX_PROBE_BYTES {
            // The small pointer/metadata transport MUST NOT become an
            // unbounded download just because the Core needs a large blob.
            assert!(matches!(
                reader.get_bounded(&blob_key, max_blob_bytes).await,
                Err(S3TransportError::ObjectTooLarge)
            ));
            assert!(matches!(
                reader.stage_verified_s3_content_for_core(
                    &verified, &mut journal, &mut cache,
                    intent.clone(), super::super::MAX_PROBE_BYTES,
                ).await,
                Err(S3CoreStageError::Transport(S3TransportError::ObjectTooLarge))
            ));
            assert!(journal.get(operation_id).unwrap().is_none());
        }
        let receipt = reader.stage_verified_s3_content_for_core(
            &verified, &mut journal, &mut cache, intent.clone(), max_blob_bytes,
        ).await.unwrap();
        assert_eq!(receipt.content_hash, blob_hash);
        assert_eq!(cache.verified_blob(&receipt.content_hash).unwrap(), bytes);
        assert!(!root_b.join(relative.as_str()).exists(), "S3 staging is not local apply");
        assert_eq!(journal.get(operation_id).unwrap().unwrap().phase, JournalPhase::Staged);
        assert!(journal.baseline(vault, record.document_id).unwrap().is_none());

        // A second independent Core must bind its LOCAL registration to the
        // verified SHARED S3 vault explicitly. A staged journal and BLAKE3
        // cache alone are not permission to bypass that durable association.
        let admission = RegisteredReadAdmission::open(&second_core, &root_b).unwrap();
        assert!(matches!(
            admission.create_staged_if_absent(operation_id, &journal, &cache),
            Err(crate::sync::host_admission::LocalCreateError::InvalidStagedIntent)
        ));
        let verified_head = reader.read_verified_head().await.unwrap().unwrap();
        let mut bindings = SqliteRegisteredS3Bindings::open(
            &admission, &app_b.join("s3-profiles.sqlite"),
        ).unwrap();
        let bound = bindings.bind_existing_verified_s3(
            &admission, "real-minio-test-profile", &profile, &ns, &verified_head,
        ).unwrap();
        assert_eq!(bound.workspace_id, target_id);
        assert_eq!(bound.vault_id, vault);

        // A verified S3 head/profile alone is not an OS write interlock.
        // Provision one EXTERNAL, pre-existing lock fixture and separately
        // persist its exact OS identity before any File Gate publication.
        let custody_scope = dir.path().join("host-owned-custody")
            .join(scoped_leaf(PinnedProbeRoot::open(&root_b).unwrap().identity()));
        fs::create_dir_all(&custody_scope).unwrap();
        let custody_file = custody_scope.join(LOCK_NAME);
        fs::write(&custody_file, b"pre-existing external lock").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&custody_scope, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&custody_file, fs::Permissions::from_mode(0o600)).unwrap();
        }
        #[cfg(unix)]
        let custody_scope = fs::canonicalize(custody_scope).unwrap();
        let mut custody = SqliteCustodyIdentityStore::open(
            &admission, &app_b.join("s3-custody.sqlite"),
        ).unwrap();
        custody.bind_registered_s3(&admission, &bindings, &custody_scope).unwrap();

        // Exercise the full verified-head -> cache -> host File Gate path.
        let published = reader.receive_single_verified_s3_additive_once(
            &verified, &admission, &bindings, &custody, &custody_scope,
            &mut journal, &mut cache, intent.clone(), max_blob_bytes,
        ).await.unwrap();
        assert_eq!(published.byte_length, bytes.len() as u64);
        assert_eq!(fs::read(root_b.join(relative.as_str())).unwrap(), bytes);
        assert!(matches!(
            admission.create_staged_if_absent_for_registered_s3(
                operation_id, &journal, &cache, &bindings, &profile, &ns,
                &custody, &custody_scope,
            ),
            Err(crate::sync::host_admission::LocalCreateError::Admission(
                VaultPortError::DestinationExists
            ))
        ));
        assert_eq!(fs::read(root_b.join(relative.as_str())).unwrap(), bytes);
        assert!(matches!(
            journal.mark_applied(operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)
        ));
        assert_eq!(journal.get(operation_id).unwrap().unwrap().phase, JournalPhase::Staged);
        assert!(journal.baseline(vault, record.document_id).unwrap().is_none());
        assert_eq!(journal.local_publication_review_queue().unwrap().len(), 1);

        // Replay after a journal/cache restart must not create another file
        // or grant a baseline. The cloud bytes remain independently readable.
        drop(admission);
        drop(cache);
        drop(journal);
        let journal = SqliteSyncJournal::open(&app_b.join("ops.sqlite")).unwrap();
        let cache = SqliteDagStore::open(&app_b.join("blobs.sqlite")).unwrap();
        assert!(RegisteredReadAdmission::open(&second_core, &root_b).unwrap()
            .create_staged_if_absent_for_registered_s3(
                operation_id, &journal, &cache, &bindings, &profile, &ns,
                &custody, &custody_scope,
            ).is_err());
        assert!(journal.baseline(vault, record.document_id).unwrap().is_none());
        assert_eq!(fs::read(root_b.join(relative.as_str())).unwrap(), bytes);
    }
}
