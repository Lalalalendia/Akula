/// P0g-3d-02: read-only post-restart inspection of an *existing* V7 claim,
/// protected original, registered S3 mapping and current pinned destination.
/// Use a fresh RegisteredReadAdmission after restart, a CURRENT-SCHEMA
/// SqliteSyncJournal::open_existing_read_only, and
/// SqliteProtectionStore::open_existing_read_only. This function neither
/// acquires write custody nor touches File Gate / HistoryService, and cannot
/// manufacture durable native evidence or retry a consumed V7 claim.
///
/// Cross-store checks are deliberately re-read after the two pinned reads.
/// This is NOT a globally atomic snapshot, an adversarial same-UID defense,
/// or a callback that may be used to acknowledge remote S3 state.
/// Compatibility API: exactly the same read-only inspector and original error
/// text as before G1-02b. Typed diagnostics cannot authorize a publication.
#[allow(dead_code)]
pub(crate) fn inspect_cold_s3_replace_hold(
    admission: &RegisteredReadAdmission<'_>,
    bindings: &SqliteRegisteredS3Bindings,
    journal: &SqliteSyncJournal,
    protected: &SqliteProtectionStore,
    operation: super::OperationId,
) -> Result<InertColdS3Review, FileCommandError> {
    inspect_cold_s3_replace_hold_classified(
        admission, bindings, journal, protected, operation,
    ).map_err(ColdReviewFailure::into_original_error)
}

/// G1-02b: return evidence-domain errors from the SAME cold verification pass.
/// In particular, distinguish a missing/corrupt one-shot V7 fence from a
/// missing/unbound/corrupt independent original. Never classify by error text.
#[allow(dead_code)]
pub(crate) fn inspect_cold_s3_replace_hold_classified(
    admission: &RegisteredReadAdmission<'_>,
    bindings: &SqliteRegisteredS3Bindings,
    journal: &SqliteSyncJournal,
    protected: &SqliteProtectionStore,
    operation: super::OperationId,
) -> Result<InertColdS3Review, ColdReviewFailure> {
    let root = admission.registered_root_path();
    admission.recheck_registered_root().map_err(refused)?;
    journal.require_appdata_outside_vault(root).map_err(refused)?;
    profile_binding::validate_appdata_location(admission, protected.database_path())
        .map_err(refused)?;

    let registered = bindings.load_for_registered(admission).map_err(refused)?
        .ok_or_else(|| refused("cold review has no registered S3 binding"))?;
    let record = journal.get(operation).map_err(refused)?
        .ok_or_else(|| refused("cold review has no staged operation"))?;
    let claim = journal.read_existing_replace_attempt(operation)
        .map_err(|error| ColdReviewFailure::new(
            ColdReviewFailureReason::V7Unavailable, refused(error),
        ))?.ok_or_else(|| ColdReviewFailure::new(
            ColdReviewFailureReason::V7Unavailable,
            refused("cold review has no irreversible V7 claim"),
        ))?;
    let original = protected.read_bound_preimage(operation)
        .map_err(|error| ColdReviewFailure::new(
            ColdReviewFailureReason::ProtectedOriginalUnavailable, refused(error),
        ))?.ok_or_else(|| ColdReviewFailure::new(
            ColdReviewFailureReason::ProtectedOriginalUnavailable,
            refused("cold review has no independently protected original"),
        ))?;
    let baseline = journal.baseline(claim.vault_id, claim.document_id).map_err(refused)?
        .ok_or_else(|| refused("cold review has no previous baseline"))?;
    if record.phase != JournalPhase::Staged
        || record.intent.action != super::JournalAction::StageDownload
        || record.intent.operation_id != operation
        || record.intent.vault_id != registered.vault_id
        || record.intent.profile_id != registered.profile_id
        || record.intent.document_id != claim.document_id
        || record.intent.path != claim.path
        || record.intent.expected_baseline != Some(claim.baseline_revision)
        || record.intent.expected_content_hash != claim.target_hash
        || record.staged_hash.as_deref() != Some(claim.target_hash.as_str())
        || baseline.revision_id != claim.baseline_revision
        || baseline.operation_id != claim.baseline_operation
        || baseline.content_hash != claim.preimage_hash
        || original.original.operation_id != operation
        || original.original.path != claim.path
        || original.original.content_hash != claim.preimage_hash
        || original.original.bytes.len() as u64 != claim.original_length
        || original.root != claim.root
        || original.file != claim.original_file
        || registered.workspace_id != admission.registered_workspace_id()
        || registered.local_root_device != claim.root.device
        || registered.local_root_file != claim.root.file
    {
        return Err(refused("cold review evidence refers to another operation, Core root, or baseline").into());
    }

    // A forensic read still refuses a dirty/open editor, but creates no writer
    // capability. Lock is intentionally limited to this read-only inspection.
    let checked = root.join(claim.path.as_str());
    let _reservation = admission.registered_core().documents
        .reserve_sync_closed_document(&checked).map_err(refused)?;
    let pinned = PinnedReadOnlyRoot::open(root).map_err(refused)?;
    if root_identity(&pinned)? != claim.root
        || admission.registered_root_identity_for_profile().map_err(refused)?
            != (claim.root.device, claim.root.file)
    {
        return Err(refused("cold review root physical identity changed").into());
    }

    let first = pinned.read_identified_bounded(
        &claim.path, MAX_PROTECTED_ORIGINAL_BYTES as u64,
    ).map_err(refused)?;
    let second = pinned.read_identified_bounded(
        &claim.path, MAX_PROTECTED_ORIGINAL_BYTES as u64,
    ).map_err(refused)?;
    if first.identity.is_none()
        || first.identity != second.identity
        || first.snapshot.bytes != second.snapshot.bytes
        || first.snapshot.content_hash != second.snapshot.content_hash
    {
        return Err(refused("cold review observations are unstable").into());
    }
    let file_state = if first.identity == Some(claim.original_file)
        && first.snapshot.content_hash == claim.preimage_hash
        && first.snapshot.bytes == original.original.bytes
    {
        InertColdS3FileState::OriginalStillVisible
    } else if first.identity != Some(claim.original_file)
        && first.snapshot.content_hash == claim.target_hash
    {
        InertColdS3FileState::CandidateVisibleWithoutReceipt
    } else {
        InertColdS3FileState::NeedsReview
    };

    // A V8 candidate is optional: an old V7 claim must remain reviewable
    // without manufacturing a new receipt. If V8 exists, require its exact
    // persisted evidence to match the live OS-pinned read and protected BLOB.
    // This does not prove the independent HistoryService bytes or power-loss
    // durability, so even a positive result remains HOLD.
    let v8 = journal.read_existing_replace_receipt_candidate(operation)
        .map_err(refused)?;
    let v8_state = match &v8 {
        None => InertColdV8State::Missing,
        Some(candidate) => {
            if file_state != InertColdS3FileState::CandidateVisibleWithoutReceipt {
                return Err(refused("V8 exists but primary is not the published candidate").into());
            }
            candidate.validate_cold_local_evidence(&claim, &original, &first)
                .map_err(refused)?;
            InertColdV8State::LocalCorrelationOnly
        }
    };

    admission.recheck_registered_root().map_err(refused)?;
    if root_identity(&pinned)? != claim.root
        || bindings.load_for_registered(admission).map_err(refused)?.as_ref()
            != Some(&registered)
        || journal.get(operation).map_err(refused)?.as_ref() != Some(&record)
        || journal.baseline(claim.vault_id, claim.document_id).map_err(refused)?
            .as_ref() != Some(&baseline)
        || journal.read_existing_replace_attempt(operation).map_err(refused)?
            .as_ref() != Some(&claim)
        || protected.read_bound_preimage(operation).map_err(refused)?
            .as_ref() != Some(&original)
        || journal.read_existing_replace_receipt_candidate(operation)
            .map_err(refused)?.as_ref() != v8.as_ref()
    {
        return Err(refused("cold review evidence changed during inspection").into());
    }
    Ok(InertColdS3Review { operation_id: operation, file_state, v8_state })
}
