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
    /// Only the real File Gate may invoke this under its mutation mutex AND
    /// the registered external custody lock. Recheck the exact old inode
    /// (not merely identical bytes) after the irreversible V7 claim and
    /// immediately before the one canonical primary publication.
    ///
    /// On any failure the existing claim remains HOLD. This narrows a
    /// cooperative race; AtomicWriteFile is still pathname-based and this
    /// cannot exclude malicious same-user replacement after the check.
    /// No OS durability or remote-baseline receipt is generated here.
    #[allow(dead_code)]
    pub(crate) fn inspect_before_write(
        &self,
        admission: &RegisteredReadAdmission<'_>,
        bindings: &SqliteRegisteredS3Bindings,
        journal: &SqliteSyncJournal,
        protected: &SqliteProtectionStore,
    ) -> Result<(), FileCommandError> {
        self.check_evidence(admission, bindings, journal, protected)?;
        self.check_original()?;
        self.check_evidence(admission, bindings, journal, protected)
    }

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


/// Recovery classification from independently reopened, read-only evidence.
/// Every variant remains HOLD; even a byte-exact candidate is not a durable
/// File Gate/HistoryService receipt and can NEVER grant S3 apply or baseline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InertColdS3FileState {
    /// V7 claim was consumed but the pinned original is still visible.
    OriginalStillVisible,
    /// Target bytes are visible under a different file identity. No receipt.
    CandidateVisibleWithoutReceipt,
    /// Missing, modified or unsupported identity/bytes: human review required.
    NeedsReview,
}

/// V8 metadata may correlate with the protected source and pinned primary,
/// but the independent HistoryService version is not verified by this stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InertColdV8State {
    Missing,
    LocalCorrelationOnly,
}

#[must_use = "recovery observation is forensic only and never authorizes S3 apply"]
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct InertColdS3Review {
    pub(crate) operation_id: super::OperationId,
    pub(crate) file_state: InertColdS3FileState,
    /// V8 is forensic metadata, never an apply, ACK or receipt capability.
    pub(crate) v8_state: InertColdV8State,
}

// Keep the established single cold reader in a small dedicated source module.
include!("s3_reserved_replace_cold_inspection.rs");

/// P0g-3d-07: a positive diagnostic result is still a HOLD, not a native
/// receipt, download apply permit, remote ACK or baseline transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InertColdUnifiedStatus {
    LocallyConsistentHold,
}

#[must_use = "cold cross-store consistency is never an S3 write capability"]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InertColdUnifiedReview {
    pub(crate) operation_id: super::OperationId,
    pub(crate) status: InertColdUnifiedStatus,
}

/// The original read-only inspector is still the SINGLE decision engine.
/// These typed phase failures describe which mandatory proof did not hold.
/// They are diagnostic labels only: no failure can mint a receipt or ACK.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum ColdReviewFailureReason {
    CrossStoreEvidenceRejected,
    PrimaryNotPublished,
    V8Missing,
    V8Unavailable,
    V8OperationMismatch,
    V7Unavailable,
    ProtectedOriginalUnavailable,
    HistoryTargetMismatch,
    RegisteredRootChanged,
    ImmutableHistoryRejected,
    PostHistoryEvidenceRejected,
    PostHistoryEvidenceChanged,
}

impl ColdReviewFailureReason {
    #[allow(dead_code)]
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::CrossStoreEvidenceRejected => "CROSS_STORE_EVIDENCE_REJECTED",
            Self::PrimaryNotPublished => "PRIMARY_NOT_PUBLISHED",
            Self::V8Missing => "V8_MISSING",
            Self::V8Unavailable => "V8_UNAVAILABLE",
            Self::V8OperationMismatch => "V8_OPERATION_MISMATCH",
            Self::V7Unavailable => "V7_UNAVAILABLE",
            Self::ProtectedOriginalUnavailable => "PROTECTED_ORIGINAL_UNAVAILABLE",
            Self::HistoryTargetMismatch => "HISTORY_TARGET_MISMATCH",
            Self::RegisteredRootChanged => "REGISTERED_ROOT_CHANGED",
            Self::ImmutableHistoryRejected => "IMMUTABLE_HISTORY_REJECTED",
            Self::PostHistoryEvidenceRejected => "POST_HISTORY_EVIDENCE_REJECTED",
            Self::PostHistoryEvidenceChanged => "POST_HISTORY_EVIDENCE_CHANGED",
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct ColdReviewFailure {
    pub(crate) reason: ColdReviewFailureReason,
    original_error: FileCommandError,
}

impl ColdReviewFailure {
    fn new(reason: ColdReviewFailureReason, original_error: FileCommandError) -> Self {
        Self { reason, original_error }
    }

    fn into_original_error(self) -> FileCommandError {
        self.original_error
    }
}

// Unclassified legacy refusal paths remain fail-closed. Specific V7 and
// original-store errors are classified at their point of failure, never by
// re-opening SQLite or parsing human-readable errors afterward.
impl From<FileCommandError> for ColdReviewFailure {
    fn from(error: FileCommandError) -> Self {
        Self::new(ColdReviewFailureReason::CrossStoreEvidenceRejected, error)
    }
}

/// Join independently reopened, read-only V7/V8/original stores and an actual
/// HistoryService on the CURRENT registered Core. This is strictly forensic.
/// Call after fresh Core::open and RegisteredReadAdmission; all three SQLite
/// stores must already be opened via their read-only existing-file methods.
///
/// Phase 1: correlate physical primary + bound protected original + V7/V8.
/// Phase 2: read the exact external history version under the existing history
/// mutex and the current registered root.
/// Phase 3: redo the entire pinned primary/SQLite/profile correlation, and
/// compare the saved V8 candidate. Never infer permission from success.
///
/// No shared filesystem/SQLite/HistoryService ACID transaction exists. This
/// does NOT defend against an adversarial concurrent filesystem process or
/// prove that every directory entry survives an actual power cut.
#[allow(dead_code)]
pub(crate) fn inspect_cold_s3_replace_with_history_hold(
    admission: &RegisteredReadAdmission<'_>,
    bindings: &SqliteRegisteredS3Bindings,
    journal: &SqliteSyncJournal,
    protected: &SqliteProtectionStore,
    operation: super::OperationId,
) -> Result<InertColdUnifiedReview, FileCommandError> {
    inspect_cold_s3_replace_with_history_classified(
        admission, bindings, journal, protected, operation,
    ).map_err(ColdReviewFailure::into_original_error)
}

/// G1-02a: the SAME read-only verification path with stage-specific typed
/// failures for disposable recovery laboratories. The existing API above
/// preserves its legacy FileCommandError behavior. A reason never permits
/// local publishing, S3 apply, ACK or baseline advancement.
#[allow(dead_code)]
pub(crate) fn inspect_cold_s3_replace_with_history_classified(
    admission: &RegisteredReadAdmission<'_>,
    bindings: &SqliteRegisteredS3Bindings,
    journal: &SqliteSyncJournal,
    protected: &SqliteProtectionStore,
    operation: super::OperationId,
) -> Result<InertColdUnifiedReview, ColdReviewFailure> {
    // Before inspecting history, a V8 entry must correlate with a physically
    // published candidate. A V7-only attempt is NOT a full receipt candidate.
    let initial = inspect_cold_s3_replace_hold_classified(
        admission, bindings, journal, protected, operation,
    )?;
    if initial.file_state != InertColdS3FileState::CandidateVisibleWithoutReceipt {
        return Err(ColdReviewFailure::new(
            ColdReviewFailureReason::PrimaryNotPublished,
            refused("full cold review requires a published V8 candidate"),
        ));
    }
    if initial.v8_state != InertColdV8State::LocalCorrelationOnly {
        return Err(ColdReviewFailure::new(
            ColdReviewFailureReason::V8Missing,
            refused("full cold review requires a published V8 candidate"),
        ));
    }
    let candidate = journal.read_existing_replace_receipt_candidate(operation)
        .map_err(|error| ColdReviewFailure::new(
            ColdReviewFailureReason::V8Unavailable, refused(error),
        ))?.ok_or_else(|| ColdReviewFailure::new(
            ColdReviewFailureReason::V8Missing, refused("V8 candidate missing"),
        ))?;
    if candidate.operation_id() != operation {
        return Err(ColdReviewFailure::new(
            ColdReviewFailureReason::V8OperationMismatch,
            refused("V8 candidate belongs to another operation"),
        ));
    }
    let claim = journal.read_existing_replace_attempt(operation)
        .map_err(|error| ColdReviewFailure::new(
            ColdReviewFailureReason::V7Unavailable, refused(error),
        ))?.ok_or_else(|| ColdReviewFailure::new(
            ColdReviewFailureReason::V7Unavailable, refused("V7 one-shot claim missing"),
        ))?;
    let (version_id, history_hash, history_length) = candidate.cold_history_fields();
    if history_hash != claim.target_hash {
        return Err(ColdReviewFailure::new(
            ColdReviewFailureReason::HistoryTargetMismatch,
            refused("V8 history hash differs from claimed target"),
        ));
    }

    let root = admission.registered_root_path().to_path_buf();
    let note = root.join(claim.path.as_str());
    admission.recheck_registered_root().map_err(|error| ColdReviewFailure::new(
        ColdReviewFailureReason::RegisteredRootChanged, refused(error),
    ))?;
    // The caller cannot supply arbitrary history roots or a foreign note.
    // HistoryService independently rejects stale roots, non-external versions,
    // aliased directories, and mutated/oversized version bytes.
    let roots = || vec![root.clone()];
    admission.registered_core().history.inspect_cold_external_version(
        &roots, &note, version_id, history_hash, history_length,
    ).map_err(|error| ColdReviewFailure::new(
        ColdReviewFailureReason::ImmutableHistoryRejected, refused(error),
    ))?;
    admission.recheck_registered_root().map_err(|error| ColdReviewFailure::new(
        ColdReviewFailureReason::RegisteredRootChanged, refused(error),
    ))?;

    // Bracket the independent history read with the same full cold V7/V8,
    // protected original, profile and pinned-file checks. This is a two-pass
    // consistency check, not a global transaction or a durable receipt.
    let after = inspect_cold_s3_replace_hold_classified(
        admission, bindings, journal, protected, operation,
    ).map_err(|failure| ColdReviewFailure::new(
        ColdReviewFailureReason::PostHistoryEvidenceRejected,
        failure.into_original_error(),
    ))?;
    if initial != after
        || journal.read_existing_replace_receipt_candidate(operation)
            .map_err(|error| ColdReviewFailure::new(
                ColdReviewFailureReason::PostHistoryEvidenceRejected, refused(error),
            ))?.as_ref() != Some(&candidate)
        || journal.read_existing_replace_attempt(operation)
            .map_err(|error| ColdReviewFailure::new(
                ColdReviewFailureReason::PostHistoryEvidenceRejected, refused(error),
            ))?.as_ref() != Some(&claim)
    {
        return Err(ColdReviewFailure::new(
            ColdReviewFailureReason::PostHistoryEvidenceChanged,
            refused("cold V8/primary/original evidence changed after history read"),
        ));
    }

    Ok(InertColdUnifiedReview {
        operation_id: operation,
        status: InertColdUnifiedStatus::LocallyConsistentHold,
    })
}
