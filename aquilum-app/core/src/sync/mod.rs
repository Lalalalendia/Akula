//! Inert sync contracts, types and deterministic fake provider.
//! Local history is not distributed; File Gate protects dirty editor buffers.

mod core_scan;
mod dag;
mod delta;
// Experimental and opt-in versioned codec; never an S3 writer or File Gate.
#[cfg(feature = "sync-text-delta")]
pub(crate) mod text_delta_codec;
mod download;
mod dag_store;
pub mod fake;
mod fake_vault;
mod host_admission;
mod filegate_lock_custody;
mod filegate_custody_identity;
// Host custody epoch is S3-independent; legacy unhosted Core::open is blocked.
#[cfg(feature = "host-custody")]
mod host_ledger_identity;
#[cfg(feature = "host-custody")]
pub(crate) mod host_custody_registry;
#[cfg(feature = "sync-installed-host-witness")]
pub(crate) mod host_install_witness;
#[cfg(feature = "sync-installed-host-unix-root-witness")]
mod host_install_unix_root;
#[cfg(all(test, feature = "host-custody"))]
mod host_core_epoch_tests;
pub(crate) mod guarded_note_write;
#[cfg(feature = "sync-s3")]
pub(crate) mod guarded_editor;
#[cfg(test)]
mod cooperative_gate_tests;
pub(crate) mod filegate_lock_probe;
pub(crate) use host_admission::RegisteredReadAdmission;
pub(crate) use pinned_read::{PinnedFileIdentity, PinnedReadOnlyRoot};
#[cfg(test)]
mod fault_matrix;
#[cfg(feature = "device-roster-lab")]
pub mod device_roster_lab;
mod identity;
mod journal;
mod local_create;
#[cfg(test)]
mod native_existing_replace_probe;
#[cfg(all(test, windows))]
mod native_windows_replace_probe;
mod local_recovery;
mod recovery_review;
/// Crypto envelope contract only: no production key custody or archive writer yet.
pub mod sealed_archive;
mod recovery_dashboard;
#[cfg(feature = "sync-s3")]
mod s3_publication_dashboard;
mod planner;
mod protection_store;
#[cfg(feature = "sync-s3")]
mod s3_preimage_capture;
#[cfg(feature = "sync-s3")]
mod s3_reserved_replace;
#[cfg(all(test, feature = "sync-s3"))]
mod cold_oracle_report;
#[cfg(feature = "sync-s3")]
mod profile_binding;
mod path;
mod pinned_read;
#[cfg(windows)]
mod filegate_windows_acl;
#[cfg(windows)]
mod pinned_windows;
// P2P-only, default-off receiver stage creation. Reuse the existing NT
// handle-relative create primitive and conservative owner/DACL audit.
#[cfg(all(windows, feature = "p2p-inbox-lab"))]
pub(crate) fn create_pinned_p2p_inbox_stage(
    root: &std::path::Path,
    name: &str,
) -> Result<(std::fs::File, std::fs::File), VaultPortError> {
    let inspected = pinned_windows::open_root(root)?;
    filegate_windows_acl::audit_owner_dacl(&inspected)?;
    // P2P Inbox, unlike a generic File Gate stage, may need to create
    // the initially absent "objects" directory through the pinned NT root.
    let writable = pinned_windows::open_root_for_p2p_inbox_create(root)?;
    if pinned_windows::file_identity(&inspected)? != pinned_windows::file_identity(&writable)? {
        return Err(VaultPortError::FileChanged);
    }
    let stage = pinned_windows::create_stage_child(&writable, name)?;
    Ok((writable, stage))
}

/// P2P-only negative guard for the local Inbox after stage creation, before
/// any path-based SQLite/object operation, and again before reporting success.
/// Each child is opened by one NT component relative to the *same pinned root*
/// with READ_CONTROL. This rejects unsafe owner/DACL, reparse directories and
/// hard-linked SQLite files. Path-based SQLite/open remains a known gap; this
/// guard is not a complete defense against hostile same-user rename races.
#[cfg(all(windows, feature = "p2p-inbox-lab"))]
pub(crate) fn audit_pinned_p2p_inbox_storage(
    root: &std::path::Path,
    pinned_root: &std::fs::File,
    require_complete: bool,
) -> Result<(), VaultPortError> {
    let inspected = pinned_windows::open_root(root)?;
    if pinned_windows::file_identity(&inspected)? != pinned_windows::file_identity(pinned_root)? {
        return Err(VaultPortError::FileChanged);
    }
    filegate_windows_acl::audit_owner_dacl(&inspected)?;
    for (name, leaf_file, mandatory) in [
        ("objects", false, true),
        ("received-inbox.sqlite3", true, true),
        ("received-inbox.sqlite3-wal", true, false),
        ("received-inbox.sqlite3-shm", true, false),
    ] {
        let opened = pinned_windows::open_optional_inbox_audit_child(
            &inspected,
            name,
            leaf_file,
        )?;
        let Some(opened) = opened else {
            if mandatory && require_complete {
                return Err(VaultPortError::FileChanged);
            }
            continue;
        };
        filegate_windows_acl::audit_owner_dacl(&opened)?;
        if leaf_file && pinned_windows::file_identity_with_links(&opened)?.2 != 1 {
            return Err(VaultPortError::UnsafePath);
        }
    }
    Ok(())
}

/// P2P stage deletion is anchored to the exact HANDLE created under the
/// protected Inbox root. A hostile rename/replacement of the former stage
/// name must never allow clean-up to delete an unrelated user file.
#[cfg(all(windows, feature = "p2p-inbox-lab"))]
pub(crate) fn mark_pinned_p2p_inbox_stage_delete_on_close(
    opened_stage: &std::fs::File,
) -> Result<(), VaultPortError> {
    pinned_windows::mark_p2p_stage_delete_on_close(opened_stage)
}

/// Publish exactly one verified P2P Inbox blob using only the already
/// authorized NT root and objects directory HANDLES. This replaces the
/// path-based scratch + hard_link writer on Windows, never File Gate/Vault.
/// SQLite remains path-based and is a separate release blocker.
#[cfg(all(windows, feature = "p2p-inbox-lab"))]
pub(crate) fn publish_pinned_p2p_inbox_object(
    root: &std::path::Path,
    pinned_root: &std::fs::File,
    stage: &mut std::fs::File,
    transfer_id: uuid::Uuid,
    expected_bytes: u64,
    expected_blake3: [u8; 32],
) -> Result<(), VaultPortError> {
    use std::io::{Read, Seek, SeekFrom, Write};

    if transfer_id.is_nil() || expected_bytes == 0
        || expected_bytes > 8 * 1024 * 1024 || expected_blake3 == [0u8; 32]
    {
        return Err(VaultPortError::UnsafePath);
    }
    audit_pinned_p2p_inbox_storage(root, pinned_root, true)?;
    let objects = pinned_windows::open_p2p_objects_dir_for_create(pinned_root)?;
    filegate_windows_acl::audit_owner_dacl(&objects)?;
    let inspected_objects = pinned_windows::open_optional_inbox_audit_child(
        pinned_root, "objects", false,
    )?.ok_or(VaultPortError::FileChanged)?;
    if pinned_windows::file_identity(&objects)?
        != pinned_windows::file_identity(&inspected_objects)?
    {
        return Err(VaultPortError::FileChanged);
    }

    // On every pre-publication error, mark the exclusive temporary file for
    // removal by its ORIGINAL HANDLE, not by the mutable filename.
    struct UnpublishedStage(Option<std::fs::File>);
    impl Drop for UnpublishedStage {
        fn drop(&mut self) {
            if let Some(stage) = self.0.as_ref() {
                let _ = pinned_windows::mark_p2p_stage_delete_on_close(stage);
            }
        }
    }

    let before = stage.metadata().map_err(|e| VaultPortError::Io(e.to_string()))?;
    if !before.is_file() || before.len() != expected_bytes {
        return Err(VaultPortError::FileChanged);
    }
    stage.seek(SeekFrom::Start(0)).map_err(|e| VaultPortError::Io(e.to_string()))?;

    let scratch_name = format!(".akula-inbox-{}.part", uuid::Uuid::new_v4());
    let candidate = pinned_windows::create_stage_child(&objects, &scratch_name)?;
    let mut guard = UnpublishedStage(Some(candidate));
    let candidate = guard.0.as_mut().ok_or(VaultPortError::FileChanged)?;
    let mut hasher = blake3::Hasher::new();
    let mut copied = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = stage.read(&mut buffer).map_err(|e| VaultPortError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        copied = copied.checked_add(n as u64).ok_or(VaultPortError::FileChanged)?;
        if copied > expected_bytes {
            return Err(VaultPortError::FileChanged);
        }
        hasher.update(&buffer[..n]);
        candidate.write_all(&buffer[..n]).map_err(|e| VaultPortError::Io(e.to_string()))?;
    }
    if copied != expected_bytes
        || stage.metadata().map_err(|e| VaultPortError::Io(e.to_string()))?.len() != expected_bytes
        || *hasher.finalize().as_bytes() != expected_blake3
    {
        return Err(VaultPortError::FileChanged);
    }
    candidate.sync_all().map_err(|e| VaultPortError::Io(e.to_string()))?;
    let leaf = format!("{}.blob", transfer_id.hyphenated());
    // NT FileRenameInformation is atomically no-replace, anchored to objects.
    // The temporary filename is not used for either publication or cleanup.
    pinned_windows::rename_staged_no_replace(candidate, &objects, &leaf)?;
    // Renamed file is now the permanent object: never delete it on Drop.
    drop(guard.0.take());

    let live_objects = pinned_windows::open_optional_inbox_audit_child(
        pinned_root, "objects", false,
    )?.ok_or(VaultPortError::FileChanged)?;
    if pinned_windows::file_identity(&objects)?
        != pinned_windows::file_identity(&live_objects)?
    {
        return Err(VaultPortError::FileChanged);
    }
    audit_pinned_p2p_inbox_storage(root, pinned_root, true)?;
    Ok(())
}

/// Keep the full absolute ancestor directory chain (drive/share root to the
/// immediate parent), Inbox root, objects and main SQLite file pinned without
/// FILE_SHARE_DELETE for the entire receipt. This is name custody only:
/// WAL/SHM, same-user byte writes, power loss and VFS races remain separate.
#[cfg(all(windows, feature = "p2p-inbox-lab"))]
pub(crate) struct P2pInboxSqliteCustody {
    ancestors: Vec<(std::path::PathBuf, std::fs::File)>,
    root: std::fs::File,
    objects: std::fs::File,
    db: std::fs::File,
}

#[cfg(all(windows, feature = "p2p-inbox-lab"))]
pub(crate) fn pin_p2p_inbox_sqlite_custody(
    root_path: &std::path::Path,
    pinned_root: &std::fs::File,
) -> Result<P2pInboxSqliteCustody, VaultPortError> {
    if !root_path.is_absolute() {
        return Err(VaultPortError::UnsafePath);
    }
    // Acquire from the volume/share root downward. The parent of each child
    // is already pinned against rename before its own name is resolved.
    // NT root-relative opens additionally compare the intended child with
    // the path-opened directory; reparse components are rejected.
    let mut ancestors: Vec<(std::path::PathBuf, std::fs::File)> = Vec::new();
    let paths: Vec<_> = root_path.ancestors().skip(1).collect();
    for path in paths.into_iter().rev() {
        if !path.is_absolute() {
            return Err(VaultPortError::UnsafePath);
        }
        let pinned = pinned_windows::open_p2p_inbox_root_no_delete(path)?;
        let current = pinned_windows::open_root(path)?;
        if pinned_windows::file_identity(&pinned)?
            != pinned_windows::file_identity(&current)?
        {
            return Err(VaultPortError::FileChanged);
        }
        if let Some((_, enclosing)) = ancestors.last() {
            let leaf = path.file_name().and_then(|s| s.to_str())
                .ok_or(VaultPortError::UnsafePath)?;
            let relative = pinned_windows::open_optional_inbox_audit_child(
                enclosing, leaf, false,
            )?.ok_or(VaultPortError::FileChanged)?;
            if pinned_windows::file_identity(&relative)?
                != pinned_windows::file_identity(&pinned)?
            {
                return Err(VaultPortError::FileChanged);
            }
        }
        ancestors.push((path.to_path_buf(), pinned));
    }
    let (_, parent) = ancestors.last().ok_or(VaultPortError::UnsafePath)?;
    let root_leaf = root_path.file_name().and_then(|s| s.to_str())
        .ok_or(VaultPortError::UnsafePath)?;
    let relative_root = pinned_windows::open_optional_inbox_audit_child(
        parent, root_leaf, false,
    )?.ok_or(VaultPortError::FileChanged)?;
    let original = pinned_windows::open_p2p_inbox_root_no_delete(root_path)?;
    if pinned_windows::file_identity(&relative_root)?
        != pinned_windows::file_identity(&original)?
        || pinned_windows::file_identity(&original)?
            != pinned_windows::file_identity(pinned_root)?
    {
        return Err(VaultPortError::FileChanged);
    }
    filegate_windows_acl::audit_owner_dacl(&original)?;
    // In particular, this first-use bootstrap does not call
    // fs::create_dir or Connection::open on a potentially replaced name.
    let objects = pinned_windows::pin_p2p_inbox_child_no_delete(
        pinned_root, "objects", false,
    )?;
    filegate_windows_acl::audit_owner_dacl(&objects)?;
    let db = pinned_windows::pin_p2p_inbox_child_no_delete(
        pinned_root, "received-inbox.sqlite3", true,
    )?;
    filegate_windows_acl::audit_owner_dacl(&db)?;
    if pinned_windows::file_identity_with_links(&db)?.2 != 1 {
        return Err(VaultPortError::UnsafePath);
    }
    let custody = P2pInboxSqliteCustody {
        ancestors,
        root: original,
        objects,
        db,
    };
    custody.verify(root_path, pinned_root)?;
    Ok(custody)
}

#[cfg(all(windows, feature = "p2p-inbox-lab"))]
impl P2pInboxSqliteCustody {
    pub(crate) fn verify(
        &self,
        root_path: &std::path::Path,
        pinned_root: &std::fs::File,
    ) -> Result<(), VaultPortError> {
        // A no-delete HANDLE is held for every ancestor, not just the
        // immediate Akula parent; every current namespace view must still
        // identify the very same directory after the SQLite transaction.
        for (index, (path, pinned)) in self.ancestors.iter().enumerate() {
            let current = pinned_windows::open_root(path)?;
            if pinned_windows::file_identity(pinned)?
                != pinned_windows::file_identity(&current)?
            {
                return Err(VaultPortError::FileChanged);
            }
            if index > 0 {
                let parent = &self.ancestors[index - 1].1;
                let leaf = path.file_name().and_then(|s| s.to_str())
                    .ok_or(VaultPortError::UnsafePath)?;
                let relative = pinned_windows::open_optional_inbox_audit_child(
                    parent, leaf, false,
                )?.ok_or(VaultPortError::FileChanged)?;
                if pinned_windows::file_identity(pinned)?
                    != pinned_windows::file_identity(&relative)?
                {
                    return Err(VaultPortError::FileChanged);
                }
            }
        }
        // Verify both the original HANDLE identities and current path views
        // after the SQLite commit, before sending any remote receipt OK.
        if pinned_windows::file_identity(&self.root)?
            != pinned_windows::file_identity(pinned_root)?
        {
            return Err(VaultPortError::FileChanged);
        }
        filegate_windows_acl::audit_owner_dacl(&self.root)?;
        for (name, pinned, is_file) in [
            ("objects", &self.objects, false),
            ("received-inbox.sqlite3", &self.db, true),
        ] {
            let current = pinned_windows::open_optional_inbox_audit_child(
                pinned_root, name, is_file,
            )?.ok_or(VaultPortError::FileChanged)?;
            if pinned_windows::file_identity(pinned)?
                != pinned_windows::file_identity(&current)?
            {
                return Err(VaultPortError::FileChanged);
            }
            filegate_windows_acl::audit_owner_dacl(&current)?;
        }
        if pinned_windows::file_identity_with_links(&self.db)?.2 != 1 {
            return Err(VaultPortError::UnsafePath);
        }
        // Compare the final child relative to its already-pinned immediate
        // parent as well as through the ambient path. This does not prevent
        // arbitrary byte writes by another process with the same account.
        let (_, parent) = self.ancestors.last().ok_or(VaultPortError::UnsafePath)?;
        let leaf = root_path.file_name().and_then(|s| s.to_str())
            .ok_or(VaultPortError::UnsafePath)?;
        let anchored_root = pinned_windows::open_optional_inbox_audit_child(
            parent, leaf, false,
        )?.ok_or(VaultPortError::FileChanged)?;
        if pinned_windows::file_identity(&anchored_root)?
            != pinned_windows::file_identity(&self.root)?
        {
            return Err(VaultPortError::FileChanged);
        }
        let reopened_root = pinned_windows::open_root(root_path)?;
        if pinned_windows::file_identity(&reopened_root)?
            != pinned_windows::file_identity(&self.root)?
        {
            return Err(VaultPortError::FileChanged);
        }
        Ok(())
    }
}

/// P2P-only readback of the exact received object using NT handles rather
/// than reopening its caller-visible pathname. The object must have exactly
/// one NTFS hard link, restrictive OWNER/DACL and the expected actual bytes.
/// This complements, but does not replace, SQLite/namespace custody.
#[cfg(all(windows, feature = "p2p-inbox-lab"))]
pub(crate) fn verify_pinned_p2p_inbox_object(
    root: &std::path::Path,
    pinned_root: &std::fs::File,
    transfer_id: uuid::Uuid,
    expected_bytes: u64,
    expected_blake3: [u8; 32],
) -> Result<(), VaultPortError> {
    use std::io::Read;

    if expected_bytes == 0 || expected_bytes > 8 * 1024 * 1024
        || expected_blake3 == [0u8; 32]
    {
        return Err(VaultPortError::UnsafePath);
    }
    audit_pinned_p2p_inbox_storage(root, pinned_root, true)?;
    let objects = pinned_windows::open_optional_inbox_audit_child(
        pinned_root, "objects", false,
    )?.ok_or(VaultPortError::FileChanged)?;
    filegate_windows_acl::audit_owner_dacl(&objects)?;
    // The filename is derived only from a locally authenticated transfer ID,
    // never from a remote filename, JSON display label or user-supplied path.
    let leaf = format!("{}.blob", transfer_id.hyphenated());
    let mut object = pinned_windows::open_optional_inbox_audit_child(
        &objects, &leaf, true,
    )?.ok_or(VaultPortError::FileChanged)?;
    filegate_windows_acl::audit_owner_dacl(&object)?;
    if pinned_windows::file_identity_with_links(&object)?.2 != 1 {
        return Err(VaultPortError::UnsafePath);
    }
    if object.metadata().map_err(|e| VaultPortError::Io(e.to_string()))?.len()
        != expected_bytes
    {
        return Err(VaultPortError::FileChanged);
    }
    let mut hasher = blake3::Hasher::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = object.read(&mut buffer).map_err(|e| VaultPortError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        bytes = bytes.checked_add(n as u64).ok_or(VaultPortError::FileChanged)?;
        if bytes > expected_bytes {
            return Err(VaultPortError::FileChanged);
        }
        hasher.update(&buffer[..n]);
    }
    if bytes != expected_bytes || *hasher.finalize().as_bytes() != expected_blake3 {
        return Err(VaultPortError::FileChanged);
    }
    // Refuse an obvious replacement of the objects path during readback.
    // A malicious same-user actor can still race after this point, hence this
    // is intentionally not called a durable authenticated delivery receipt.
    let still_objects = pinned_windows::open_optional_inbox_audit_child(
        pinned_root, "objects", false,
    )?.ok_or(VaultPortError::FileChanged)?;
    if pinned_windows::file_identity(&objects)?
        != pinned_windows::file_identity(&still_objects)?
    {
        return Err(VaultPortError::FileChanged);
    }
    audit_pinned_p2p_inbox_storage(root, pinned_root, true)?;
    Ok(())
}

mod protocol;
mod provider;
mod s3;
mod s3_manifest;
// Inert V2 contract only: no S3 pack writer, reader or auto-sync activation.
mod s3_pack_index;
mod s3_manifest_delta;
mod s3_coord_v2;
#[cfg(feature = "sync-s3")]
mod s3_anchored_delta;
#[cfg(feature = "sync-s3")]
mod s3_delta_walker;
#[cfg(feature = "sync-s3")]
mod s3_remote_anchor;
// App-data V2 trust anchor is not a per-document or write baseline.
#[cfg(feature = "sync-s3")]
mod s3_v2_trust_anchor;
#[cfg(feature = "sync-s3")]
mod s3_head_probe;
#[cfg(feature = "sync-s3")]
mod s3_transport;
#[cfg(feature = "sync-s3")]
mod yandex_readonly_probe;
mod upload;
mod vault;

pub use core_scan::{CoreVaultScanner, LocalScanPreview, LocalScanTrust, ScanLimits};
pub use dag::{DagError, IngestOutcome, RevisionDag, RevisionOperation, RevisionRecord};
pub use delta::{plan_revision_exchange, DeltaPlanError, MissingRevisionBatch, RevisionExchangePlan, RevisionInventory};
pub use dag_store::{DagStoreError, DagStoreResult, SqliteDagStore};
#[cfg(feature = "sync-s3")]
pub use s3_publication_dashboard::native_s3_bound_publication_overview;
pub use recovery_dashboard::{
    native_recovery_overview, native_recovery_case_preview, native_record_recovery_decision,
    NativeRecoveryOverview, NativeRecoveryExample, NativeRecoveryCasePreview,
    NativeRecoveryDecision, NativeRecoveryReviewError, NativeRecoveryReviewReceipt,
    RECOVERY_JOURNAL_FILENAME,
};
pub use download::{stage_remote_download, StageDownloadError, StagedDownload};
pub use identity::{DeviceId, DocumentId, OperationId, RevisionId, VaultId};
pub use journal::{
    ConfirmedBaseline, JournalAction, JournalError, JournalPhase, JournalRecord, JournalResult,
    OperationIntent, LocalPublicationReview, RecoveryAction, RecoveryItem, RemoteGuard, SqliteSyncJournal,
};
pub use planner::{
    plan_snapshot, HoldReason, ObservedObject, PlannedAction, SafetyPolicy, SnapshotDiff,
};
pub use path::{reject_path_collisions, resolve_no_symlinks, SyncPathError, SyncRelativePath};
pub use protocol::{
    decode_identity_for_write, IdentityEnvelope, IdentityRecord, ProtocolCompatibility,
    ProtocolError, ProtocolVersion, SyncConfig, SyncProvider, WireEnvelope,
};

pub use fake::FakeRemoteStore;
pub use fake_vault::FakeVaultPort;
pub use provider::{
    collect_verified_listing, ProviderCapabilities, RemoteObject, RemotePage, RemotePrecondition,
    RemoteSnapshot, RemoteStore, RemoteStoreError, RemoteWriteReceipt,
};
pub use s3::{S3ContractError, S3HeadPointerV1, S3Keyspace};
pub use s3_coord_v2::{
    S3DeltaEnvelopeV2, S3DeltaEnvelopeV2Error, S3DeltaEnvelopeCoverageV2,
    S3DeltaPackRefV2, MAX_DELTA_ENVELOPE_BYTES, MAX_DELTA_ENVELOPE_REVISIONS,
    MAX_DELTA_PACK_BYTES, MAX_DELTA_PACK_MEMBERS,
};
#[cfg(feature = "sync-s3")]
pub use s3_anchored_delta::{walk_s3_delta_from_saved_anchor, S3AnchoredWalkError};
#[cfg(feature = "sync-s3")]
pub use s3_remote_anchor::{
    S3RemoteAnchor, S3RemoteAnchorError, S3RemoteAnchorJournal,
    S3RemoteAnchorOutcome, S3RemoteAnchorScope,
};
#[cfg(feature = "sync-s3")]
pub use s3_v2_trust_anchor::{
    S3V2TrustAnchorError, S3V2TrustAnchorOutcome,
    S3V2TrustedAnchor, S3V2TrustedAnchorJournal,
};
#[cfg(feature = "sync-s3")]
pub use s3_delta_walker::{
    walk_s3_manifest_delta, S3DeltaWalkCheckpoint, S3DeltaWalkError, S3DeltaWalkPreview,
};
pub use s3_manifest_delta::{
    inspect_s3_manifest_delta, S3ManifestDeltaBase, S3ManifestDeltaError,
    S3ManifestDeltaLimits, S3ManifestDeltaOutcome, S3ManifestShardDelta,
    S3ManifestSnapshotReason,
};
pub use s3_manifest::{
    S3ManifestError, S3ManifestShardRefV1, S3ManifestShardV1, S3ManifestV1,
    S3RevisionRefV1, MAX_MANIFEST_BYTES, MAX_REVISIONS, MAX_SHARD_BYTES,
    MAX_SHARD_ENTRIES, MAX_SHARDS,
};
pub use s3_pack_index::{
    S3PackedHeadV2, S3PackIndexRootV2, S3PackIndexShardV2,
    S3PackDescriptorV2, S3PackMemberV2, S3PackShardRefV2,
    S3PackIndexV2Error, MAX_PACK_V2_ROOT_BYTES,
    MAX_PACK_V2_INDEX_BYTES, MAX_PACK_V2_SHARD_BYTES,
    MAX_PACK_V2_SHARDS, MAX_PACK_V2_PACKS_PER_SHARD,
    MAX_PACK_V2_MEMBERS_PER_PACK, MAX_PACK_V2_BYTES,
};
#[cfg(feature = "sync-s3")]
pub use s3_head_probe::{
    observe_s3_current_head, read_s3_digest_matched_manifest, S3DigestMatchedManifest,
    S3HeadReadError, S3ObservedHead,
};
#[cfg(feature = "sync-s3")]
pub use yandex_readonly_probe::{probe_yandex_s3_read_only, YandexS3ProbeError};
#[cfg(feature = "sync-s3")]
pub use s3_transport::{
    AwsS3ProbeClient, S3BoundedObject, S3ConnectionProfile, S3DiagnosticPage,
    S3AwsAccountProofError, S3AwsVerifiedAccountScope,
    S3ObservedObject, S3PutCondition, S3TransportError,
    S3MultipartError, S3MultipartIntent, S3MultipartJournal, S3MultipartOutcome,
    S3MultipartPhase, S3MultipartStatus, S3StreamDigest,
    S3HeadAttemptPhase, S3HeadError, S3HeadJournal, S3HeadPublication, S3VerifiedHead,
    S3PinnedRevisionPage, S3PinnedRevisionSnapshot,
    S3VerifiedRevisionMetadataStore, MAX_S3_REVISION_METADATA_CACHE_BYTES,
    S3SelectiveReadError, S3SelectiveRevisionPreview,
    S3CoordSidecarReadError, S3CoordSidecarPreview,
    S3PinnedV2ReadError, S3PinnedV2ReadPreview,
    S3CoordOfflineChainError, S3CoordOfflineChainPreview,
    S3CoordSelectivePacksPreview,
    S3VerifiedHintsError, S3VerifiedHintsJournal,
    plan_verified_s3_revision_replay, S3CasReplanPreview, S3ReplanError,
};

pub use upload::{upload_immutable_verified, UploadOutcome, VerifiedUploadError};
pub use vault::{
    LocalFileEntry, LocalFileSnapshot, PreservationReceipt, VaultFileKind, VaultPort,
    VaultPortError,
};

/// No sync scheduler or external I/O is started by importing these contracts.
/// Runtime activation/UI remain out of scope until the S3 slice is validated.
pub const SYNC_ENGINE_FEATURE_ENABLED: bool = cfg!(feature = "sync-engine");
/// Local custody checks can compile without an S3 SDK or sync scheduler.
pub const HOST_CUSTODY_FEATURE_ENABLED: bool = cfg!(feature = "host-custody");

#[cfg(all(test, not(feature = "sync-engine")))]
mod disabled_tests {
    #[test]
    fn compiled_contracts_do_not_enable_cloud_sync_by_default() {
        assert!(!super::SYNC_ENGINE_FEATURE_ENABLED);
    }
}
