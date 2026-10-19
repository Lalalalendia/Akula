//! P0g-2: a retained CLOSED-editor reservation for one staged S3 operation.
//! No filesystem writer, network request, native receipt or baseline commit.
//! Intended as the preparation value of the existing single-custody writer.
//! The caller must already hold the approved cooperative OS custody lifetime;
//! this helper never reacquires it and never turns its claim into write authority.

use super::journal::ReadOnlyExistingReplaceAttempt;
use super::pinned_read::{PinnedFileIdentity, PinnedReadOnlyRoot};
use super::profile_binding::{self, RegisteredS3Binding, SqliteRegisteredS3Bindings};
use super::protection_store::{BoundProtectedOriginal, SqliteProtectionStore,
    MAX_PROTECTED_ORIGINAL_BYTES};
use super::{ConfirmedBaseline, JournalPhase, OperationIntent, RegisteredReadAdmission,
    S3ConnectionProfile, S3Keyspace, SqliteDagStore, SqliteSyncJournal};
use crate::app_core::Core;
use crate::documents::hub::SyncReservation;
use crate::files::error::FileCommandError;
use crate::files::models::FileWriteResult;
use std::fmt::Debug;

fn refused(reason: impl Debug) -> FileCommandError {
    FileCommandError::Task {
        message: format!("reserved S3 preparation HOLD: {reason:?}"),
    }
}

fn root_identity(root: &PinnedReadOnlyRoot) -> Result<PinnedFileIdentity, FileCommandError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = root.root_metadata().map_err(refused)?;
        Ok(PinnedFileIdentity { device: metadata.dev(), file: metadata.ino() })
    }
    #[cfg(windows)]
    {
        let (device, file) = root.root_win_identity().map_err(refused)?;
        Ok(PinnedFileIdentity { device: u64::from(device), file })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = root;
        Err(refused("unsupported pinned root"))
    }
}

/// Neither Clone nor a writer permit. Dropping it releases ONLY this Core's
/// editor reservation, never the durable once-only attempt in the journal.
#[must_use = "retain through File Gate, history and postflight; never treat as an apply receipt"]
#[allow(dead_code)]
pub(crate) struct ReservedS3Replace<'a> {
    _reservation: SyncReservation<'a>,
    core: &'a Core,
    workspace_id: uuid::Uuid,
    pinned: PinnedReadOnlyRoot,
    binding: RegisteredS3Binding,
    baseline: ConfirmedBaseline,
    intent: OperationIntent,
    original: BoundProtectedOriginal,
    target: Vec<u8>,
    claim: ReadOnlyExistingReplaceAttempt,
}

#[allow(dead_code)]
impl ReservedS3Replace<'_> {
    /// Pinned readback and private evidence consistency ONLY. Does not fsync,
    /// attest the new inode as a native publication, or authorize journal apply.
    pub(crate) fn inspect_after_write(
        &self,
        admission: &RegisteredReadAdmission<'_>,
        bindings: &SqliteRegisteredS3Bindings,
        journal: &SqliteSyncJournal,
        protected: &SqliteProtectionStore,
        written: &FileWriteResult,
    ) -> Result<(), FileCommandError> {
        self.check_evidence(admission, bindings, journal, protected)?;
        if written.hash != self.intent.expected_content_hash {
            return Err(refused("File Gate result differs from staged revision"));
        }
        let first = self.pinned.read_identified_bounded(
            &self.intent.path, MAX_PROTECTED_ORIGINAL_BYTES as u64,
        ).map_err(refused)?;
        let second = self.pinned.read_identified_bounded(
            &self.intent.path, MAX_PROTECTED_ORIGINAL_BYTES as u64,
        ).map_err(refused)?;
        if first.identity.is_none() || first.identity != second.identity
            || first.snapshot.bytes != self.target || second.snapshot.bytes != self.target
            || first.snapshot.content_hash != self.intent.expected_content_hash
            || second.snapshot.content_hash != self.intent.expected_content_hash
        {
            return Err(refused("postflight is not the same stable staged content"));
        }
        self.check_evidence(admission, bindings, journal, protected)
    }

    fn check_evidence(
        &self,
        admission: &RegisteredReadAdmission<'_>,
        bindings: &SqliteRegisteredS3Bindings,
        journal: &SqliteSyncJournal,
        protected: &SqliteProtectionStore,
    ) -> Result<(), FileCommandError> {
        admission.recheck_registered_root().map_err(refused)?;
        if !std::ptr::eq(self.core, admission.registered_core())
            || self.workspace_id != admission.registered_workspace_id()
            || root_identity(&self.pinned)? != self.original.root
            || admission.registered_root_identity_for_profile().map_err(refused)?
                != (self.original.root.device, self.original.root.file)
        {
            return Err(refused("different registered Core or physical root"));
        }
        journal.require_appdata_outside_vault(admission.registered_root_path())
            .map_err(refused)?;
        profile_binding::validate_appdata_location(admission, protected.database_path())
            .map_err(refused)?;
        let record = journal.get(self.intent.operation_id).map_err(refused)?
            .ok_or_else(|| refused("operation disappeared"))?;
        if record.intent != self.intent || record.phase != JournalPhase::Staged
            || record.staged_hash.as_deref() != Some(self.intent.expected_content_hash.as_str())
            || bindings.load_for_registered(admission).map_err(refused)?.as_ref()
                != Some(&self.binding)
            || journal.baseline(self.intent.vault_id, self.intent.document_id)
                .map_err(refused)?.as_ref() != Some(&self.baseline)
            || journal.read_existing_replace_attempt(self.intent.operation_id)
                .map_err(refused)?.as_ref() != Some(&self.claim)
            || protected.read_bound_preimage(self.intent.operation_id)
                .map_err(refused)?.as_ref() != Some(&self.original)
        {
            return Err(refused("prepared evidence changed; do not acknowledge or retry"));
        }
        Ok(())
    }

    fn check_original(&self) -> Result<(), FileCommandError> {
        let live = self.pinned.read_identified_bounded(
            &self.intent.path, MAX_PROTECTED_ORIGINAL_BYTES as u64,
        ).map_err(refused)?;
        if live.identity != Some(self.original.file)
            || live.snapshot.bytes != self.original.original.bytes
            || live.snapshot.content_hash != self.original.original.content_hash
        {
            return Err(refused("original physical identity or bytes changed"));
        }
        Ok(())
    }
}

/// Bind the exact caller-planned operation/content to registered S3 state,
/// immutable cached bytes and the protected original. Keep the editor CLOSED
/// until the returned preparation is dropped by the single-custody callback.
///
/// Call only from trusted synchronous preparation under the external OS lock.
/// Errors may follow an already committed one-shot claim: the surrounding
/// writer MUST map them to NeedsReview. No failure clears/retries a claim.
#[allow(dead_code)]
pub(crate) fn prepare_reserved_s3_replace<'a>(
    admission: &'a RegisteredReadAdmission<'_>,
    bindings: &SqliteRegisteredS3Bindings,
    profile: &S3ConnectionProfile,
    keyspace: &S3Keyspace,
    journal: &mut SqliteSyncJournal,
    cache: &SqliteDagStore,
    protected: &SqliteProtectionStore,
    planned: &OperationIntent,
    new_content: &str,
    expected_local_hash: &str,
) -> Result<ReservedS3Replace<'a>, FileCommandError> {
    let core = admission.registered_core();
    // This preparatory bridge must not consume a claim in a compiled host
    // flavor whose real enrolled writer is not implemented yet.
    if core.requires_installed_host_epoch() {
        return Err(refused("installed-host writer remains unavailable"));
    }
    admission.recheck_registered_root().map_err(refused)?;
    journal.require_appdata_outside_vault(admission.registered_root_path()).map_err(refused)?;
    cache.require_appdata_outside_vault(admission.registered_root_path()).map_err(refused)?;
    profile_binding::validate_appdata_location(admission, protected.database_path())
        .map_err(refused)?;
    let binding = bindings.load_for_registered(admission).map_err(refused)?
        .ok_or_else(|| refused("missing registered S3 profile"))?;
    if !binding.matches_remote(profile, keyspace)
        || planned.vault_id != binding.vault_id || planned.profile_id != binding.profile_id
        || planned.action != super::JournalAction::StageDownload
        || planned.remote_guard.is_some() || planned.expected_baseline.is_none()
        || planned.expected_baseline == Some(planned.revision_id)
        || new_content.len() > MAX_PROTECTED_ORIGINAL_BYTES
        || blake3::hash(new_content.as_bytes()).to_hex().as_str() != planned.expected_content_hash
        || planned.path.as_str().split('/').any(|part|
            part.eq_ignore_ascii_case(".aquilum") || part.eq_ignore_ascii_case(".trash")
                || part.eq_ignore_ascii_case(".git"))
        || planned.remote_object_id != keyspace.immutable_blob_key(
            &planned.expected_content_hash,
        ).map_err(refused)?
    {
        return Err(refused("foreign or unsupported S3 operation/content"));
    }
    let target = cache.verified_blob(&planned.expected_content_hash).map_err(refused)?;
    if target != new_content.as_bytes() {
        return Err(refused("proposed text differs from verified private cache"));
    }
    let baseline = journal.baseline(planned.vault_id, planned.document_id).map_err(refused)?
        .ok_or_else(|| refused("missing confirmed local baseline"))?;
    let original = protected.read_bound_preimage(planned.operation_id).map_err(refused)?
        .ok_or_else(|| refused("missing protected original"))?;
    if original.original.content_hash != expected_local_hash
        || original.original.path != planned.path
    {
        return Err(refused("protected original does not match requested local source"));
    }
    let _reservation = core.documents.reserve_sync_closed_document(
        &admission.registered_root_path().join(planned.path.as_str()),
    ).map_err(refused)?;
    let _gate = core.file_mutations.lock().map_err(|_| refused("File Gate poisoned"))?;
    let pinned = PinnedReadOnlyRoot::open(admission.registered_root_path()).map_err(refused)?;
    if root_identity(&pinned)? != original.root {
        return Err(refused("protected root physical identity changed"));
    }
    admission.recheck_registered_root().map_err(refused)?;
    let live = pinned.read_identified_bounded(
        &planned.path, MAX_PROTECTED_ORIGINAL_BYTES as u64,
    ).map_err(refused)?;
    if live.identity != Some(original.file) || live.snapshot.bytes != original.original.bytes
        || live.snapshot.content_hash != expected_local_hash
    {
        return Err(refused("local original changed before attempt claim"));
    }
    // The existing V7 transaction validates the committed baseline, exact
    // Staged intent, portable path/document uniqueness and original digest.
    let claim = journal.record_existing_replace_attempt_once(planned, &original).map_err(refused)?;
    let prepared = ReservedS3Replace {
        _reservation, core, workspace_id: admission.registered_workspace_id(),
        pinned, binding, baseline, intent: planned.clone(), original, target, claim,
    };
    prepared.check_original()?;
    prepared.check_evidence(admission, bindings, journal, protected)?;
    Ok(prepared)
}

#[cfg(test)]
#[path = "s3_reserved_replace_tests.rs"]
mod tests;
