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
        let head_before = self.core_stage_current_head(verified).await?;
        let payload = self.get_immutable_blob_for_core_stage(
            &intent.expected_content_hash, max_blob_bytes,
        ).await.map_err(S3CoreStageError::from)?;
        if payload.blake3 != intent.expected_content_hash {
            return Err(E::Stage(S3CoreStageError::IntegrityMismatch));
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
