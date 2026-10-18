//! SYNC-APPLY-P0c. Fully verified existing-file S3 download into private
//! cache and Staged journal ONLY. It cannot write a vault file or commit a
//! baseline, and does not relax the separate additive-only S3 stage.

use super::{
    AwsS3ProbeClient, S3VerifiedRevisionMetadataStore, MAX_CORE_STAGE_BLOB_BYTES,
    core_stage::S3CoreStageError,
    guarded_update_plan::{preview_verified_s3_existing_content_update, S3GuardedUpdatePlanError},
};
use crate::sync::{
    ConfirmedBaseline, DagStoreError, JournalError, JournalPhase,
    OperationIntent, SqliteDagStore, SqliteSyncJournal, StagedDownload, VaultPortError,
};
use crate::sync::host_admission::RegisteredReadAdmission;
use crate::sync::profile_binding::SqliteRegisteredS3Bindings;

#[derive(Debug)]
pub(crate) enum S3ExistingStageError {
    WrongRegisteredS3,
    MissingOrChangedBaseline,
    WrongPreviousOperation,
    InvalidBudget,
    DeclaredSizeMismatch,
    Plan(S3GuardedUpdatePlanError),
    Stage(S3CoreStageError),
    Journal(JournalError),
    Cache(DagStoreError),
    Admission(VaultPortError),
}
impl From<JournalError> for S3ExistingStageError {
    fn from(v: JournalError) -> Self { Self::Journal(v) }
}
impl From<DagStoreError> for S3ExistingStageError {
    fn from(v: DagStoreError) -> Self { Self::Cache(v) }
}
impl From<VaultPortError> for S3ExistingStageError {
    fn from(v: VaultPortError) -> Self { Self::Admission(v) }
}
impl From<S3CoreStageError> for S3ExistingStageError {
    fn from(v: S3CoreStageError) -> Self { Self::Stage(v) }
}

// No remote manifest may supply the current baseline or local path.
// Always reconstruct those from registered Core and its WAL/FULL journal.
fn require_existing_baseline(
    client: &AwsS3ProbeClient,
    verified: &S3VerifiedRevisionMetadataStore,
    admission: &RegisteredReadAdmission<'_>,
    bindings: &SqliteRegisteredS3Bindings,
    journal: &SqliteSyncJournal,
    intent: &OperationIntent,
) -> Result<ConfirmedBaseline, S3ExistingStageError> {
    use S3ExistingStageError as E;
    admission.recheck_registered_root()?;
    let binding = bindings.load_for_registered(admission)
        .map_err(|_| E::WrongRegisteredS3)?
        .ok_or(E::WrongRegisteredS3)?;
    if binding.vault_id != intent.vault_id
        || binding.profile_id != intent.profile_id
        || !binding.matches_remote(client.bound_profile(), client.bound_keyspace())
    {
        return Err(E::WrongRegisteredS3);
    }
    let baseline = journal.baseline(intent.vault_id, intent.document_id)?
        .ok_or(E::MissingOrChangedBaseline)?;
    let previous = journal.get(baseline.operation_id)?
        .ok_or(E::WrongPreviousOperation)?;
    if previous.phase != JournalPhase::Committed
        || previous.intent.vault_id != baseline.vault_id
        || previous.intent.document_id != baseline.document_id
        || previous.intent.profile_id != baseline.profile_id
        || previous.intent.revision_id != baseline.revision_id
        || previous.intent.expected_content_hash != baseline.content_hash
        || previous.intent.path != intent.path
    {
        return Err(E::WrongPreviousOperation);
    }
    preview_verified_s3_existing_content_update(
        verified, client.bound_keyspace(), &baseline, intent,
    ).map_err(E::Plan)?;
    Ok(baseline)
}

impl AwsS3ProbeClient {
    /// Never take a file mutation lock, overwrite a user path or advance a
    /// document baseline. A post-cache failure leaves at most private staged
    /// evidence and MUST be reported as HOLD, never applied by retry.
    #[allow(dead_code)]
    pub(crate) async fn stage_verified_existing_s3_for_registered_core(
        &self,
        verified: &S3VerifiedRevisionMetadataStore,
        admission: &RegisteredReadAdmission<'_>,
        bindings: &SqliteRegisteredS3Bindings,
        journal: &mut SqliteSyncJournal,
        cache: &mut SqliteDagStore,
        intent: OperationIntent,
        max_blob_bytes: u64,
    ) -> Result<StagedDownload, S3ExistingStageError> {
        use S3ExistingStageError as E;
        if max_blob_bytes == 0 || max_blob_bytes > MAX_CORE_STAGE_BLOB_BYTES {
            return Err(E::InvalidBudget);
        }
        admission.recheck_registered_root()?;
        journal.require_appdata_outside_vault(admission.registered_root_path())?;
        cache.require_appdata_outside_vault(admission.registered_root_path())?;
        let old = require_existing_baseline(
            self, verified, admission, bindings, journal, &intent,
        )?;
        // The complete verified manifest carries the immutable declared
        // size. Require an exact size, rather than trusting only the later
        // S3 Content-Length; reject over-budget before a network download.
        let declared_bytes = verified.cached_revision(intent.revision_id)
            .and_then(|(reference, _)| reference.blob_bytes)
            .ok_or(E::DeclaredSizeMismatch)?;
        if declared_bytes > max_blob_bytes {
            return Err(E::InvalidBudget);
        }
        let head_before = self.core_stage_current_head(verified).await?;
        let payload = self.get_immutable_blob_for_core_stage(
            &intent.expected_content_hash, max_blob_bytes,
        ).await.map_err(S3CoreStageError::from)?;
        if payload.blake3 != intent.expected_content_hash
            || payload.bytes.len() as u64 != declared_bytes
        {
            return Err(E::DeclaredSizeMismatch);
        }
        let head_after = self.core_stage_current_head(verified).await?;
        if head_before != head_after {
            return Err(E::Stage(S3CoreStageError::RemoteHeadChanged));
        }
        let current = require_existing_baseline(
            self, verified, admission, bindings, journal, &intent,
        )?;
        if old != current {
            return Err(E::MissingOrChangedBaseline);
        }
        let phase = journal.begin(intent.clone())?;
        if !matches!(phase, JournalPhase::Pending | JournalPhase::Staged) {
            return Err(E::WrongPreviousOperation);
        }
        let hash = cache.store_blob(&payload.bytes)?;
        if hash != intent.expected_content_hash
            || cache.verified_blob(&hash)? != payload.bytes
        {
            return Err(E::Stage(S3CoreStageError::IntegrityMismatch));
        }
        journal.mark_staged(intent.operation_id, &payload.bytes)?;
        admission.recheck_registered_root()?;
        if self.core_stage_current_head(verified).await? != head_before {
            return Err(E::Stage(S3CoreStageError::RemoteHeadChanged));
        }
        Ok(StagedDownload {
            content_hash: hash,
            byte_length: payload.bytes.len() as u64,
            observed_provider_revision: payload.object.etag,
            was_already_staged: phase == JournalPhase::Staged,
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::{Core, CoreEvent};
    use crate::sync::{
        DocumentId, JournalAction, OperationId, RemoteGuard, RevisionId,
        RevisionOperation, RevisionRecord, S3ConnectionProfile,
        S3HeadPointerV1, S3Keyspace, S3ManifestShardV1, S3ManifestV1,
        S3HeadPublication, S3HeadJournal, S3PutCondition, S3RevisionRefV1,
        SyncRelativePath, VaultId,
    };
    use aws_sdk_s3::error::ProvideErrorMetadata;
    use std::{fs, sync::Arc};

    #[tokio::test]
    #[ignore = "requires isolated loopback MinIO"]
    async fn live_minio_existing_s3_fast_forward_stages_without_local_mutation() {
        let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").unwrap();
        assert!(endpoint.starts_with("http://127.0.0.1:")
            || endpoint.starts_with("http://localhost:"));
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let app = temp.path().join("app");
        fs::create_dir_all(root.join("notes")).unwrap();
        fs::create_dir_all(&app).unwrap();
        let original = b"old confirmed Markdown";
        let updated = b"new remote Markdown";
        let relative = SyncRelativePath::parse("notes/safe.md").unwrap();
        fs::write(root.join(relative.as_str()), original).unwrap();

        let core = Core::open(&app, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();

        let ns = S3Keyspace::new("akula-sync/p0c-existing", VaultId::new()).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "akula-core-stage-test".into(),
            region: "us-east-1".into(),
            endpoint: Some(endpoint), force_path_style: true,
        };
        let writer = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
        let reader = AwsS3ProbeClient::connect(profile.clone(), ns.clone()).await.unwrap();
        match writer.client.create_bucket().bucket(&profile.bucket).send().await {
            Ok(_) => {}
            Err(e) if e.as_service_error().and_then(|s| s.code())
                == Some("BucketAlreadyOwnedByYou") => {}
            Err(e) => panic!("MinIO fixture provisioning failed: {e}"),
        }
        let doc = DocumentId::new();
        let old_id = RevisionId::new();
        let new_id = RevisionId::new();
        let old_hash = blake3::hash(original).to_hex().to_string();
        let new_hash = blake3::hash(updated).to_hex().to_string();
        let before = RevisionRecord {
            id: old_id, document_id: doc, parents: vec![],
            operation: RevisionOperation::Content {
                path: relative.clone(), blob_hash: old_hash.clone(),
            },
        };
        let after = RevisionRecord {
            id: new_id, document_id: doc, parents: vec![old_id],
            operation: RevisionOperation::Content {
                path: relative.clone(), blob_hash: new_hash.clone(),
            },
        };
        let mut references = Vec::new();
        for (record, bytes) in [
            (&before, original.as_slice()),
            (&after, updated.as_slice()),
        ] {
            let digest = blake3::hash(bytes).to_hex().to_string();
            writer.put_conditionally(
                &ns.immutable_blob_key(&digest).unwrap(), bytes,
                S3PutCondition::IfAbsent,
            ).await.unwrap();
            let json = serde_json::to_vec(record).unwrap();
            writer.put_conditionally(
                &ns.immutable_revision_key(record.id), &json,
                S3PutCondition::IfAbsent,
            ).await.unwrap();
            references.push(S3RevisionRefV1 {
                revision_id: record.id, document_id: doc,
                revision_blake3: blake3::hash(&json).to_hex().to_string(),
                blob_bytes: Some(bytes.len() as u64),
            });
        }
        // Shard wire records must be strictly ascending by RevisionId;
        // UUID generation order is unrelated to canonical shard order.
        references.sort_by_key(|entry| entry.revision_id);
        let shard = S3ManifestShardV1::new(
            &ns, OperationId::new(), references,
        ).unwrap();
        writer.store_manifest_object(
            shard.shard_id, &shard.encode(&ns).unwrap(),
        ).await.unwrap();
        let manifest = S3ManifestV1::new(
            &ns, OperationId::new(), 1, None,
            vec![shard.reference(&ns).unwrap()],
        ).unwrap();
        writer.store_manifest_object(
            manifest.manifest_id, &manifest.encode(&ns).unwrap(),
        ).await.unwrap();
        let head = S3HeadPointerV1::new(
            &ns, 1, manifest.manifest_id,
            &manifest.digest(&ns).unwrap(), None,
        ).unwrap();
        let mut remote_journal = S3HeadJournal::open(
            &app.join("head.sqlite"),
        ).unwrap();
        assert_eq!(
            writer.publish_head_cas(
                &mut remote_journal, manifest.manifest_id, head, None,
            ).await.unwrap(),
            S3HeadPublication::Published,
        );
        let verified = reader.materialize_verified_revision_metadata_store(
            512 * 1024,
        ).await.unwrap().unwrap();
        let verified_head = reader.read_verified_head().await.unwrap().unwrap();
        let mut bindings = SqliteRegisteredS3Bindings::open(
            &admission, &app.join("binding.sqlite"),
        ).unwrap();
        bindings.bind_existing_verified_s3(
            &admission, "confirmed", &profile, &ns, &verified_head,
        ).unwrap();
        let mut journal = SqliteSyncJournal::open(&app.join("journal.sqlite")).unwrap();
        let mut cache = SqliteDagStore::open(&app.join("cache.sqlite")).unwrap();
        let old_operation_id = OperationId::new();
        journal.begin(OperationIntent {
            operation_id: old_operation_id,
            vault_id: ns.vault_id(), document_id: doc,
            profile_id: "confirmed".into(), revision_id: old_id,
            action: JournalAction::UploadImmutable, path: relative.clone(),
            remote_object_id: format!("objects/{old_id}"),
            remote_guard: Some(RemoteGuard::Absent),
            expected_content_hash: old_hash.clone(),
            expected_baseline: None,
        }).unwrap();
        journal.mark_staged(old_operation_id, original).unwrap();
        journal.mark_applied(old_operation_id).unwrap();
        journal.mark_verified(old_operation_id, original, "provider-rev").unwrap();
        journal.commit(old_operation_id).unwrap();
        let old_baseline = journal.baseline(ns.vault_id(), doc).unwrap().unwrap();
        assert_eq!(old_baseline.revision_id, old_id);

        let operation_id = OperationId::new();
        let intent = OperationIntent {
            operation_id, vault_id: ns.vault_id(), document_id: doc,
            profile_id: "confirmed".into(), revision_id: new_id,
            action: JournalAction::StageDownload, path: relative.clone(),
            remote_object_id: ns.immutable_blob_key(&new_hash).unwrap(),
            remote_guard: None, expected_content_hash: new_hash.clone(),
            expected_baseline: Some(old_id),
        };
        let mut wrong = intent.clone();
        wrong.profile_id = "unapproved".into();
        assert!(reader.stage_verified_existing_s3_for_registered_core(
            &verified, &admission, &bindings,
            &mut journal, &mut cache, wrong, 4096,
        ).await.is_err());
        assert!(journal.get(operation_id).unwrap().is_none());
        let mut wrong_base = intent.clone();
        wrong_base.expected_baseline = Some(RevisionId::new());
        assert!(reader.stage_verified_existing_s3_for_registered_core(
            &verified, &admission, &bindings,
            &mut journal, &mut cache, wrong_base, 4096,
        ).await.is_err());
        assert!(journal.get(operation_id).unwrap().is_none());

        let staged = reader.stage_verified_existing_s3_for_registered_core(
            &verified, &admission, &bindings,
            &mut journal, &mut cache, intent.clone(), 4096,
        ).await.unwrap();
        assert_eq!(staged.content_hash, new_hash);
        assert_eq!(staged.byte_length, updated.len() as u64);
        assert_eq!(cache.verified_blob(&new_hash).unwrap(), updated);
        assert_eq!(fs::read(root.join(relative.as_str())).unwrap(), original);
        assert_eq!(journal.baseline(ns.vault_id(), doc).unwrap(), Some(old_baseline));
        assert_eq!(journal.get(operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged);
        assert!(matches!(journal.mark_applied(operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)));
        let repeated = reader.stage_verified_existing_s3_for_registered_core(
            &verified, &admission, &bindings,
            &mut journal, &mut cache, intent, 4096,
        ).await.unwrap();
        assert!(repeated.was_already_staged);
        assert_eq!(fs::read(root.join(relative.as_str())).unwrap(), original);
    }
}
