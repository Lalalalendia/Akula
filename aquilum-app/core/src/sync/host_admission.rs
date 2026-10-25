//! Stage 04g-3a: fail-closed *read-only* Core host admission.
//!
//! Only Core-registered workspaces can be opened, and each preflight read
//! holds a DocumentHub reservation for its duration. A pinned directory handle
//! is compared with the live registered pathname before and after each read.
//! This is NOT a VaultPort, protective receipt, or local mutation authority:
//! no sync executor can create, replace, rename, delete or commit a baseline.

use super::pinned_read::{PinnedFileIdentity, PinnedReadOnlyRoot};
use super::protection_store::{
    ProtectionOutcome, ProtectionStoreError, SqliteProtectionStore, MAX_PROTECTED_ORIGINAL_BYTES,
};
use super::OperationId;
use super::local_create::{self, UnconfirmedLocalCreate};
use super::local_recovery::{self, ReadOnlyLocalPublicationEvidence};
use super::journal::{LocalRecoveryArchivedCopy, LocalRecoveryEvidenceCase};
use super::recovery_review::{LocalRecoveryReviewEvent, LocalRecoveryReviewSnapshot,
    RecoveryReviewDisposition};
use super::{DagStoreError, JournalAction, JournalError, JournalPhase, LocalFileSnapshot,
    SqliteDagStore, SqliteSyncJournal, SyncRelativePath, VaultPortError};
use crate::app_core::Core;
use crate::documents::hub::SyncAdmissionError;
use crate::search::paths::{canonical_path, same_path};
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Errors from a read-only source capture. None grants a write permission.
#[derive(Debug)]
pub(crate) enum BoundCaptureError {
    Admission(VaultPortError),
    Protection(ProtectionStoreError),
    OriginalHashMismatch,
}

impl From<VaultPortError> for BoundCaptureError {
    fn from(error: VaultPortError) -> Self { Self::Admission(error) }
}
impl From<ProtectionStoreError> for BoundCaptureError {
    fn from(error: ProtectionStoreError) -> Self { Self::Protection(error) }
}

/// No caller may commit a confirmed remote baseline using this result.
/// An error after an OS publish may leave a legitimate new file on disk:
/// recovery must inspect the target instead of retrying blind.
#[derive(Debug)]
pub(crate) enum LocalCreateError {
    Admission(VaultPortError),
    Journal(JournalError),
    Cache(DagStoreError),
    InvalidStagedIntent,
    PublicationUncertain,
}
impl From<VaultPortError> for LocalCreateError {
    fn from(error: VaultPortError) -> Self { Self::Admission(error) }
}
impl From<JournalError> for LocalCreateError {
    fn from(error: JournalError) -> Self { Self::Journal(error) }
}
impl From<DagStoreError> for LocalCreateError {
    fn from(error: DagStoreError) -> Self { Self::Cache(error) }
}

/// Read-only diagnosis can fail closed if the registered vault, journal,
/// editor admission, pinned OS handles or two observations disagree.
#[derive(Debug)]
pub(crate) enum LocalRecoveryError {
    Admission(VaultPortError),
    Journal(JournalError),
    MissingTicket,
    UnstableObservation,
}
impl From<VaultPortError> for LocalRecoveryError {
    fn from(error: VaultPortError) -> Self { Self::Admission(error) }
}
impl From<JournalError> for LocalRecoveryError {
    fn from(error: JournalError) -> Self { Self::Journal(error) }
}

/// Not externally constructible, and deliberately has no mutation methods.
#[allow(dead_code)]
pub(crate) struct RegisteredReadAdmission<'a> {
    core: &'a Core,
    root_path: PathBuf,
    workspace_id: Uuid,
    pinned: PinnedReadOnlyRoot,
    initial: RootIdentity,
}

/// Snapshot of a registered root and its observed, locked OS object.
/// The OS lock is released before this value is returned. These identities
/// are NOT durable claims, leases or write permissions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) struct RegisteredLockObservation {
    workspace_id: Uuid,
    root_identity: (u64, u64),
    lock_identity: (u64, u64),
}

#[allow(dead_code)]
impl<'a> RegisteredReadAdmission<'a> {
    pub(crate) fn open(core: &'a Core, selected_root: &Path) -> Result<Self, VaultPortError> {
        // Do not accept an arbitrary remote manifest pathname. The trusted
        // host must first register this workspace in its local UI database.
        let selected = fs::symlink_metadata(selected_root).map_err(io)?;
        reject_reparse_root(&selected)?;
        let resolved = fs::canonicalize(selected_root).map_err(io)?;
        let root_path = canonical_path(&resolved);
        let registered = core.ui_state.list_workspaces()
            .map_err(|_| VaultPortError::PermissionDenied)?;
        let mut matches = registered.iter()
            .filter(|item| same_path(&canonical_path(Path::new(&item.path)), &root_path));
        let workspace_id = matches.next()
            .ok_or(VaultPortError::PermissionDenied)?.id;
        if matches.next().is_some() {
            // A duplicated workspace path must not allow arbitrary grant binding.
            return Err(VaultPortError::PermissionDenied);
        }

        let pinned = PinnedReadOnlyRoot::open(&root_path)?;
        let initial = pinned_identity(&pinned)?;
        if initial != named_identity(selected_root)? {
            return Err(VaultPortError::UnsafePath);
        }
        let admission = Self { core, root_path, workspace_id, pinned, initial };
        admission.check_root()?;
        Ok(admission)
    }

    /// The Core that actually established this registration. A guarded
    /// writer must never accept an unrelated Core for the same pathname.
    pub(crate) fn registered_core(&self) -> &Core {
        self.core
    }

    pub(crate) fn registered_workspace_id(&self) -> Uuid {
        self.workspace_id
    }

    /// Checked local registration identity for a configuration binding.
    /// This observation does not grant File Gate or write authority.
    pub(crate) fn registered_root_identity_for_profile(
        &self,
    ) -> Result<(u64, u64), VaultPortError> {
        self.check_root()?;
        Ok((self.initial.0, self.initial.1))
    }

    pub(crate) fn registered_root_path(&self) -> &Path {
        &self.root_path
    }

    /// Registry roots for negative app-data location checks. Read fresh from
    /// this Core, and never treat the resulting list as durable authority.
    pub(crate) fn registered_workspace_roots(&self) -> Result<Vec<PathBuf>, VaultPortError> {
        self.check_root()?;
        self.core.ui_state.workspace_roots()
            .map_err(|_| VaultPortError::PermissionDenied)
    }

    pub(crate) fn recheck_registered_root(&self) -> Result<(), VaultPortError> {
        self.check_root()
    }

    /// Registry-bound OS lock forensics ONLY. This can never permit a File
    /// Gate, HistoryService or sync write: the kernel lock is dropped when
    /// this method returns. Durable lock custody/bootstrapping is still P0.
    ///
    /// The existing lock object is required. No file, database or journal is
    /// created, unlinked, cleared, renamed or written during inspection.
    pub(crate) fn inspect_existing_filegate_lock(
        &self,
    ) -> Result<RegisteredLockObservation, VaultPortError> {
        self.check_root()?;
        let root = super::filegate_lock_probe::PinnedProbeRoot::open(&self.root_path)
            .map_err(|error| VaultPortError::Io(error.to_string()))?;
        let expected_root = (self.initial.0, self.initial.1);
        if root.identity() != expected_root {
            return Err(VaultPortError::FileChanged);
        }
        let held = root
            .try_lock()
            .map_err(|error| VaultPortError::Io(error.to_string()))?
            .ok_or(VaultPortError::PermissionDenied)?;
        // Check SQLite registration and pinned physical identity once more
        // while the advisory kernel lock is held. Return is NOT write authority.
        self.check_root()?;
        if root.identity() != expected_root {
            return Err(VaultPortError::FileChanged);
        }
        Ok(RegisteredLockObservation {
            workspace_id: self.workspace_id,
            root_identity: expected_root,
            lock_identity: held.identity(),
        })
    }


    /// Observe a shared existing lock OUTSIDE all registered vaults, using
    /// the physical OS root identity (not the local Core SQLite UUID) to
    /// select its scope. Neither a host-owned path nor mode 0700 attests
    /// hostile same-user custody. No file mutation authority is returned.
    pub(crate) fn inspect_external_filegate_lock(
        &self,
        custody_scope: &Path,
    ) -> Result<super::filegate_lock_custody::CustodyObservation, VaultPortError> {
        self.check_root()?;
        super::filegate_lock_custody::inspect_external_scope(
            &self.root_path,
            (self.initial.0, self.initial.1),
            || self.core.ui_state.workspace_roots()
                .map_err(|_| VaultPortError::PermissionDenied),
            custody_scope,
            || self.check_root(),
        )
    }

    fn check_root(&self) -> Result<(), VaultPortError> {
        let configured = self.core.ui_state.list_workspaces()
            .map_err(|_| VaultPortError::PermissionDenied)?;
        if !configured.iter().any(|item| {
            item.id == self.workspace_id
                && same_path(&canonical_path(Path::new(&item.path)), &self.root_path)
        }) {
            return Err(VaultPortError::PermissionDenied);
        }
        // A replaced pathname must never cause a silent switch to another
        // directory, even though the old descriptor is still readable.
        if pinned_identity(&self.pinned)? != self.initial
            || named_identity(&self.root_path)? != self.initial
        {
            return Err(VaultPortError::FileChanged);
        }
        Ok(())
    }

    /// Capture a live, registered original into the durable SQLite store.
    /// The editor stays reserved until a *second* pinned read proves the same
    /// source file ID and bytes remain visible. Other processes can still race,
    /// so this MUST NOT be used as a File Gate or overwrite authorization.
    pub(crate) fn preserve_closed_original(
        &self,
        relative: &SyncRelativePath,
        expected_hash: &str,
        operation: OperationId,
        store: &mut SqliteProtectionStore,
    ) -> Result<ProtectionOutcome, BoundCaptureError> {
        self.preserve_with_hook(relative, expected_hash, operation, store, || {})
    }

    fn preserve_with_hook(
        &self,
        relative: &SyncRelativePath,
        expected_hash: &str,
        operation: OperationId,
        store: &mut SqliteProtectionStore,
        before_store: impl FnOnce(),
    ) -> Result<ProtectionOutcome, BoundCaptureError> {
        // The database is provided by the trusted host and must not sit inside
        // the sync root. This check is a *negative* safety check, not positive
        // authentication of app-data ownership/permissions.
        let storage = canonical_path(store.database_path());
        if storage.starts_with(&self.root_path) {
            return Err(VaultPortError::PermissionDenied.into());
        }
        self.check_root()?;
        let target = self.root_path.join(relative.as_str());
        let _reservation = self.core.documents.reserve_sync_closed_document(&target)
            .map_err(admission_error)?;
        // Reservation briefly locks sessions, then releases that mutex.
        // The File Gate lock is acquired after it, and held through the
        // second read. Existing editor/shell Gate mutations cannot race the
        // durability transaction. It does NOT lock external processes.
        let _file_guard = self.core.file_mutations.lock()
            .map_err(|_| BoundCaptureError::Admission(VaultPortError::PermissionDenied))?;
        self.check_root()?;
        let original = self.pinned.read_identified_bounded(
            relative, MAX_PROTECTED_ORIGINAL_BYTES as u64)?;
        if original.snapshot.content_hash != expected_hash {
            return Err(BoundCaptureError::OriginalHashMismatch);
        }
        let file_identity = original.identity.ok_or(VaultPortError::PermissionDenied)?;
        before_store();
        self.check_root()?;
        let result = store.store_bound_preimage(
            operation, relative, expected_hash, &original.snapshot.bytes,
            PinnedFileIdentity { device: self.initial.0, file: self.initial.1 },
            file_identity,
        )?;
        self.check_root()?;
        let fresh = self.pinned.read_identified_bounded(
            relative, MAX_PROTECTED_ORIGINAL_BYTES as u64)?;
        if fresh.identity != Some(file_identity)
            || fresh.snapshot.content_hash != original.snapshot.content_hash
            || fresh.snapshot.bytes != original.snapshot.bytes
        {
            // A durable preimage may already exist, but a changed file cannot
            // receive any implied authorization through this result.
            return Err(VaultPortError::FileChanged.into());
        }
        Ok(result)
    }

    /// Recheck a durable preimage against the current, registered OS file IDs.
    ///
    /// This is a read-only diagnostic used before designing the native
    /// mutation transaction. Returning Ok DOES NOT grant local write, rename,
    /// trash, preservation-receipt or sync-baseline authority: another
    /// process may swap the pathname immediately after this method returns.
    /// A future executor must keep pinned parent/file handles and its final
    /// mutation in one platform-specific atomic/admitted operation.
    pub(crate) fn revalidate_bound_preimage(
        &self,
        relative: &SyncRelativePath,
        expected_hash: &str,
        operation: OperationId,
        store: &SqliteProtectionStore,
    ) -> Result<(), BoundCaptureError> {
        // Mirror capture's negative app-data guard. A workspace-local SQLite
        // copy is not durable isolation from remote sync or user edits.
        if canonical_path(store.database_path()).starts_with(&self.root_path) {
            return Err(VaultPortError::PermissionDenied.into());
        }
        self.check_root()?;
        let path = self.root_path.join(relative.as_str());
        let _reservation = self.core.documents.reserve_sync_closed_document(&path)
            .map_err(admission_error)?;
        let _gate = self.core.file_mutations.lock()
            .map_err(|_| BoundCaptureError::Admission(VaultPortError::PermissionDenied))?;
        self.check_root()?;

        let saved = store.read_bound_preimage(operation)?
            .ok_or(ProtectionStoreError::UnboundOriginal)?;
        if saved.original.path != *relative
            || saved.original.content_hash != expected_hash
        {
            return Err(ProtectionStoreError::OperationCollision.into());
        }
        let root_id = PinnedFileIdentity {
            device: self.initial.0,
            file: self.initial.1,
        };
        if saved.root != root_id {
            return Err(VaultPortError::FileChanged.into());
        }

        let first = self.pinned.read_identified_bounded(
            relative, MAX_PROTECTED_ORIGINAL_BYTES as u64)?;
        if first.identity != Some(saved.file)
            || first.snapshot.content_hash != expected_hash
            || first.snapshot.bytes != saved.original.bytes
        {
            return Err(VaultPortError::FileChanged.into());
        }
        self.check_root()?;
        let second = self.pinned.read_identified_bounded(
            relative, MAX_PROTECTED_ORIGINAL_BYTES as u64)?;
        if second.identity != Some(saved.file)
            || second.snapshot.content_hash != expected_hash
            || second.snapshot.bytes != saved.original.bytes
        {
            return Err(VaultPortError::FileChanged.into());
        }
        self.check_root()?;
        Ok(())
    }

    /// SYNC-04g-M1: a private, additive-only local create experiment. This
    /// intentionally does NOT implement VaultPort or advance the journal
    /// past Staged; a real executor/recovery gate is still required.
    pub(crate) fn create_staged_if_absent(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
        cache: &SqliteDagStore,
    ) -> Result<UnconfirmedLocalCreate, LocalCreateError> {
        self.create_staged_with_hook(operation, journal, cache, || {})
    }

    fn create_staged_with_hook(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
        cache: &SqliteDagStore, before_publish: impl FnOnce(),
    ) -> Result<UnconfirmedLocalCreate, LocalCreateError> {
        // Preserve the original M1 local-identity contract. Remote S3 vaults
        // use the separate explicitly bound entrypoint, never this one.
        self.create_staged_with_bound_vault_and_hook(
            operation, journal, cache,
            super::VaultId::from_uuid(self.workspace_id), None, before_publish,
        )
    }

    /// SYNC-05e-6: S3 additive-only local publication under a PINNED,
    /// held interprocess OS lock. Two cooperating Akula Core processes on the
    /// same physical workspace cannot publish simultaneously through this API.
    ///
    /// The lock MUST pre-exist in external host custody and be durably bound
    /// to the shared VaultId and physical root in app-data. No auto-bootstrap,
    /// rewrite, or fallback on a missing/busy lock. Advisory locks alone cannot
    /// stop a hostile same-user process replacing the lock's pathname.
    /// Never grants baseline, overwrite, trash or cross-process File Gate rights.
    #[cfg(feature = "sync-s3")]
    pub(crate) fn create_staged_if_absent_for_registered_s3(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
        cache: &SqliteDagStore,
        bindings: &super::profile_binding::SqliteRegisteredS3Bindings,
        profile: &super::S3ConnectionProfile, keyspace: &super::S3Keyspace,
        custody: &super::filegate_custody_identity::SqliteCustodyIdentityStore,
        custody_scope: &Path,
    ) -> Result<UnconfirmedLocalCreate, LocalCreateError> {
        let denied = || LocalCreateError::Admission(VaultPortError::PermissionDenied);
        let binding = bindings.load_for_registered(self)
            .map_err(|_| denied())?
            .ok_or(LocalCreateError::InvalidStagedIntent)?;
        if !binding.matches_remote(profile, keyspace) {
            return Err(LocalCreateError::InvalidStagedIntent);
        }
        let intent = journal.get(operation)?
            .ok_or(LocalCreateError::InvalidStagedIntent)?.intent;
        let expected_key = keyspace.immutable_blob_key(&intent.expected_content_hash)
            .map_err(|_| LocalCreateError::InvalidStagedIntent)?;
        if intent.remote_object_id != expected_key {
            return Err(LocalCreateError::InvalidStagedIntent);
        }
        journal.require_appdata_outside_vault(&self.root_path)?;

        // Use the same validated lock *lifetime* as the already merged
        // cooperative File Gate + HistoryService experiment (#193). It
        // checks exact persisted physical custody IDs, registered root and
        // external namespace/owner/DACL both before and after the callback.
        // Do not take a second OS lock inside native File Gate.
        //
        // If any *post*-check fails, publication may already have happened.
        // The caller MUST report an ambiguous HOLD instead of blind replay.
        let mut entered = false;
        let outcome = custody.with_cooperative_writer_lifetime(
            self, binding.vault_id, custody_scope, || {
                entered = true;
                // Revalidate the S3 configuration WHILE the OS lock is held.
                if bindings.load_for_registered(self).map_err(|_| denied())?
                    .as_ref() != Some(&binding)
                {
                    return Err(denied());
                }
                let published = self.create_staged_with_bound_vault_and_hook(
                    operation, journal, cache, binding.vault_id,
                    Some(binding.profile_id.as_str()), || {},
                );
                if published.is_ok() {
                    if bindings.load_for_registered(self)
                        .map_err(|_| LocalCreateError::PublicationUncertain)?
                        .as_ref() != Some(&binding)
                    {
                        return Err(LocalCreateError::PublicationUncertain);
                    }
                }
                published
            },
        );
        match outcome {
            Ok(published) => published,
            // Even if the filesystem publish finished, a revoked lock or
            // altered root/ACL during postflight is never reported as ACK.
            Err(_) if entered => Err(LocalCreateError::PublicationUncertain),
            Err(_) => Err(denied()),
        }
    }

    fn create_staged_with_bound_vault_and_hook(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
        cache: &SqliteDagStore, expected_vault: super::VaultId,
        expected_profile_id: Option<&str>, before_publish: impl FnOnce(),
    ) -> Result<UnconfirmedLocalCreate, LocalCreateError> {
        // This internal additive publisher takes Core.file_mutations directly
        // rather than the installed-host SQLite enrollment epoch. Until a
        // verified shared host epoch spans the ENTIRE primary publication,
        // the mandatory installed-host flavor MUST refuse before disk writes.
        // This also covers the S3-bound additive receiver (#296); a private
        // OS lock in its own appdata is not machine-wide write authorization.
        #[cfg(feature = "host-custody")]
        if self.core.requires_installed_host_epoch() {
            return Err(LocalCreateError::Admission(VaultPortError::PermissionDenied));
        }
        let record = journal.get(operation)?
            .ok_or(LocalCreateError::InvalidStagedIntent)?;
        if record.phase != JournalPhase::Staged
            || record.intent.action != JournalAction::StageDownload
            || record.intent.remote_guard.is_some()
            || record.intent.vault_id != expected_vault
            || expected_profile_id.is_some_and(|id| id != record.intent.profile_id)
            || record.intent.expected_baseline.is_some()
            || record.staged_hash.as_deref()
                != Some(record.intent.expected_content_hash.as_str())
            || journal.baseline(record.intent.vault_id, record.intent.document_id)?.is_some()
        {
            return Err(LocalCreateError::InvalidStagedIntent);
        }
        let relative = &record.intent.path;
        let bytes = cache.verified_blob(&record.intent.expected_content_hash)?;
        if bytes.len() > MAX_PROTECTED_ORIGINAL_BYTES {
            return Err(LocalCreateError::InvalidStagedIntent);
        }
        self.check_root()?;
        // All native WAL evidence and journal sidecars must stay outside the
        // registered vault even for the existing local-only creation path.
        journal.require_appdata_outside_vault(&self.root_path)?;
        let target = self.root_path.join(relative.as_str());
        let _reservation = self.core.documents.reserve_sync_closed_document(&target)
            .map_err(admission_error)?;
        let _gate = self.core.file_mutations.lock()
            .map_err(|_| VaultPortError::PermissionDenied)?;
        self.check_root()?;

        // Read and write are always relative to an OS directory handle. The
        // root registry and the on-path parent ID are checked both before
        // publication and after it; other processes can still race.
        let parent = self.pinned.open_parent_for_create(&self.root_path, relative)?;
        let parent_id = local_create::directory_identity(&parent)?;
        self.reject_existing_portable_collision(relative)?;
        let refreshed = self.pinned.open_parent_for_create(&self.root_path, relative)?;
        if local_create::directory_identity(&refreshed)? != parent_id {
            return Err(VaultPortError::FileChanged.into());
        }
        self.check_root()?;

        let leaf = relative.as_str().rsplit('/').next()
            .ok_or(LocalCreateError::InvalidStagedIntent)?;
        // A FULL-sync SQLite INSERT is the last admission barrier before
        // creating the first stage file. Uncertain results, crashes and even
        // successful publications leave this durable one-shot claim intact.
        // A different process or later run must inspect, never blindly retry.
        journal.record_local_publication_attempt_once(&record.intent)?;
        let outcome = local_create::create_with_hook(
            &parent, leaf, operation, &bytes, before_publish,
        ).map_err(|error| match error {
            // No-replace has a defined and non-destructive collision result.
            VaultPortError::DestinationExists =>
                LocalCreateError::Admission(VaultPortError::DestinationExists),
            // Any other OS result may follow a successful publish whose
            // acknowledgement or durability check was lost. Never replay.
            _ => LocalCreateError::PublicationUncertain,
        })?;
        // The OS publication has happened. Any error below is ambiguous:
        // leave the staged journal intact and require recovery/readback.
        let fresh_parent = self.pinned.open_parent_for_create(&self.root_path, relative)
            .map_err(|_| LocalCreateError::PublicationUncertain)?;
        if local_create::directory_identity(&fresh_parent)
            .map_err(|_| LocalCreateError::PublicationUncertain)? != parent_id
        {
            return Err(LocalCreateError::PublicationUncertain);
        }
        self.check_root().map_err(|_| LocalCreateError::PublicationUncertain)?;
        let observed = self.pinned.read_identified_bounded(
            relative, MAX_PROTECTED_ORIGINAL_BYTES as u64,
        ).map_err(|_| LocalCreateError::PublicationUncertain)?;
        if observed.identity != Some(outcome.file)
            || observed.snapshot.content_hash != record.intent.expected_content_hash
            || observed.snapshot.bytes != bytes
        {
            return Err(LocalCreateError::PublicationUncertain);
        }
        self.check_root().map_err(|_| LocalCreateError::PublicationUncertain)?;

        // Only the uninterrupted Unix path, after the actual successful
        // stage-file fsync, atomic rename-no-replace, pinned destination
        // readback and parent-directory fsync, may append this informational
        // audit. A crash before it leaves no record and ALWAYS requires HOLD.
        // SQLite WAL/FULL persists this observation, not an editable file
        // provenance token or a synchronization baseline.
        #[cfg(unix)]
        {
            if !outcome.native_parent_synced {
                return Err(LocalCreateError::PublicationUncertain);
            }
            let observation = super::journal::NativeLocalPublicationObservation {
                operation_id: operation,
                vault_id: record.intent.vault_id,
                path: relative.clone(),
                expected_content_hash: record.intent.expected_content_hash.clone(),
                parent: parent_id,
                final_file: outcome.file,
                byte_length: outcome.byte_length,
            };
            journal.record_native_local_publication_observation_once(&observation)
                .map_err(|_| LocalCreateError::PublicationUncertain)?;
            self.check_root().map_err(|_| LocalCreateError::PublicationUncertain)?;
        }
        // No Windows directory durability attestation exists yet.
        Ok(outcome)
    }

    /// Conservative portable collision preflight. It is NOT the no-replace
    /// guarantee: the OS rename predicate below enforces that atomically.
    /// A directory swapped by an external process may invalidate this
    /// read-only listing; such ambiguity must never authorize a write.
    fn reject_existing_portable_collision(
        &self, relative: &SyncRelativePath,
    ) -> Result<(), VaultPortError> {
        let target = self.root_path.join(relative.as_str());
        let parent = target.parent().ok_or(VaultPortError::UnsafePath)?;
        let prefix = relative.as_str().rsplit_once('/')
            .map(|(directory, _)| format!("{directory}/"))
            .unwrap_or_default();
        let expected = relative.collision_key();
        for entry in fs::read_dir(parent)
            .map_err(|error| VaultPortError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| VaultPortError::Io(error.to_string()))?;
            let name = entry.file_name().into_string()
                .map_err(|_| VaultPortError::UnsafePath)?;
            let observed = SyncRelativePath::parse(&format!("{prefix}{name}"))
                .map_err(|_| VaultPortError::UnsafePath)?;
            if observed.collision_key() == expected {
                return Err(VaultPortError::DestinationExists);
            }
        }
        Ok(())
    }

    /// SYNC-05e-3: fail-closed admission for read-only recovery of a
    /// cloud-vault publication whose UUID differs from the local workspace.
    /// This is not File Gate permission, provenance, or a baseline receipt.
    #[cfg(feature = "sync-s3")]
    fn checked_registered_s3_recovery_scope(
        &self,
        operation: OperationId,
        journal: &SqliteSyncJournal,
        bindings: &super::profile_binding::SqliteRegisteredS3Bindings,
        profile: &super::S3ConnectionProfile,
        keyspace: &super::S3Keyspace,
    ) -> Result<super::VaultId, LocalRecoveryError> {
        self.check_root()?;
        journal.require_appdata_outside_vault(&self.root_path)?;
        let bound = bindings.load_for_registered(self)
            .map_err(|_| LocalRecoveryError::Admission(VaultPortError::PermissionDenied))?
            .ok_or(LocalRecoveryError::Admission(VaultPortError::PermissionDenied))?;
        if !bound.matches_remote(profile, keyspace) {
            return Err(LocalRecoveryError::Admission(VaultPortError::PermissionDenied));
        }
        let record = journal.get(operation)?.ok_or(LocalRecoveryError::MissingTicket)?;
        let expected_key = keyspace.immutable_blob_key(&record.intent.expected_content_hash)
            .map_err(|_| LocalRecoveryError::Admission(VaultPortError::PermissionDenied))?;
        if record.intent.action != JournalAction::StageDownload
            || record.intent.vault_id != bound.vault_id
            || record.intent.profile_id != bound.profile_id
            || record.intent.remote_object_id != expected_key
            || record.intent.remote_guard.is_some()
            || record.intent.expected_baseline.is_some()
            || record.phase != JournalPhase::Staged
            || record.staged_hash.as_deref()
                != Some(record.intent.expected_content_hash.as_str())
        {
            return Err(LocalRecoveryError::Admission(VaultPortError::PermissionDenied));
        }
        self.check_root()?;
        Ok(bound.vault_id)
    }

    /// SYNC-04g-M1c: inspect exactly one durable publication-attempt ticket.
    /// Does not write/delete/quarantine/release the attempt or update a
    /// confirmed remote baseline. A final name matching expected bytes is
    /// NOT proof that this operation created it. Two checked reads are not
    /// an interprocess atomic filesystem snapshot.
    pub(crate) fn inspect_local_publication(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
    ) -> Result<ReadOnlyLocalPublicationEvidence, LocalRecoveryError> {
        self.inspect_local_publication_with_hook(operation, journal, || {})
    }

    fn inspect_local_publication_with_hook(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
        before_second: impl FnOnce(),
    ) -> Result<ReadOnlyLocalPublicationEvidence, LocalRecoveryError> {
        self.inspect_local_publication_for_vault_with_hook(
            operation, journal, super::VaultId::from_uuid(self.workspace_id), before_second,
        )
    }

    /// Inspect exactly one cloud-origin publication under an existing,
    /// registered SQLite profile. Matching bytes never grant write authority.
    #[cfg(feature = "sync-s3")]
    pub(crate) fn inspect_local_publication_for_registered_s3(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
        bindings: &super::profile_binding::SqliteRegisteredS3Bindings,
        profile: &super::S3ConnectionProfile, keyspace: &super::S3Keyspace,
    ) -> Result<ReadOnlyLocalPublicationEvidence, LocalRecoveryError> {
        let vault = self.checked_registered_s3_recovery_scope(
            operation, journal, bindings, profile, keyspace,
        )?;
        self.inspect_local_publication_for_vault_with_hook(operation, journal, vault, || {})
    }

    fn inspect_local_publication_for_vault_with_hook(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
        expected_vault: super::VaultId, before_second: impl FnOnce(),
    ) -> Result<ReadOnlyLocalPublicationEvidence, LocalRecoveryError> {
        let ticket = journal.local_publication_review_queue()?
            .into_iter().find(|entry| entry.operation_id == operation)
            .ok_or(LocalRecoveryError::MissingTicket)?;
        if ticket.vault_id != expected_vault {
            return Err(LocalRecoveryError::Admission(VaultPortError::PermissionDenied));
        }
        let relative = &ticket.path;
        self.check_root()?;
        let target = self.root_path.join(relative.as_str());
        let _reservation = self.core.documents.reserve_sync_closed_document(&target)
            .map_err(admission_error)?;
        let _gate = self.core.file_mutations.lock()
            .map_err(|_| VaultPortError::PermissionDenied)?;
        self.check_root()?;

        let parent = self.pinned.open_parent_for_inspection(relative)?;
        let parent_identity = local_create::directory_identity(&parent)?;
        let leaf = relative.as_str().rsplit('/').next()
            .ok_or(LocalRecoveryError::Admission(VaultPortError::UnsafePath))?;
        let (first_stage, first_final) = local_recovery::inspect_names(
            &parent, operation, leaf, &ticket.expected_content_hash,
            MAX_PROTECTED_ORIGINAL_BYTES as u64,
        )?;
        self.check_root()?;
        before_second();
        let (stage, final_name) = local_recovery::inspect_names(
            &parent, operation, leaf, &ticket.expected_content_hash,
            MAX_PROTECTED_ORIGINAL_BYTES as u64,
        )?;
        if first_stage != stage || first_final != final_name {
            return Err(LocalRecoveryError::UnstableObservation);
        }
        // An external process can detach a pinned former directory. Require
        // the registered path to reach that same directory at return time.
        let current_parent = self.pinned.open_parent_for_inspection(relative)
            .map_err(|_| LocalRecoveryError::UnstableObservation)?;
        if local_create::directory_identity(&current_parent)
            .map_err(|_| LocalRecoveryError::UnstableObservation)? != parent_identity
        {
            return Err(LocalRecoveryError::UnstableObservation);
        }
        self.check_root()?;
        let assessment = local_recovery::classify(&stage, &final_name);
        Ok(ReadOnlyLocalPublicationEvidence {
            operation_id: operation, parent_identity,
            stage, final_name, assessment,
        })
    }

    /// M1d-a: durably record the FIRST read-only recovery observation.
    ///
    /// The OS descriptors, closed-document reservation and File Gate are held
    /// during inspect_local_publication. The record below is explicitly a
    /// historical snapshot: external writers can race after that inspection.
    /// This API has no way to release the claim, move/delete a stage,
    /// approve provenance or commit a confirmed cloud baseline.
    pub(crate) fn record_local_recovery_evidence(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
    ) -> Result<LocalRecoveryEvidenceCase, LocalRecoveryError> {
        self.record_local_recovery_evidence_for_vault(
            operation, journal, super::VaultId::from_uuid(self.workspace_id),
        )
    }

    #[cfg(feature = "sync-s3")]
    pub(crate) fn record_local_recovery_evidence_for_registered_s3(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
        bindings: &super::profile_binding::SqliteRegisteredS3Bindings,
        profile: &super::S3ConnectionProfile, keyspace: &super::S3Keyspace,
    ) -> Result<LocalRecoveryEvidenceCase, LocalRecoveryError> {
        let vault = self.checked_registered_s3_recovery_scope(
            operation, journal, bindings, profile, keyspace,
        )?;
        self.record_local_recovery_evidence_for_vault(operation, journal, vault)
    }

    fn record_local_recovery_evidence_for_vault(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
        expected_vault: super::VaultId,
    ) -> Result<LocalRecoveryEvidenceCase, LocalRecoveryError> {
        // Durable forensic evidence is itself a SQLite write. A journal
        // inside the registered vault could be externally modified or synced,
        // so reject it before recording any recovery evidence. Pure read-only
        // inspection remains separately available.
        journal.require_appdata_outside_vault(&self.root_path)?;
        let evidence = self.inspect_local_publication_for_vault_with_hook(
            operation, journal, expected_vault, || {},
        )?;
        self.check_root()?;
        let ticket = journal.local_publication_review_queue()?
            .into_iter().find(|entry| entry.operation_id == operation)
            .ok_or(LocalRecoveryError::MissingTicket)?;
        if ticket.vault_id != expected_vault {
            return Err(LocalRecoveryError::Admission(VaultPortError::PermissionDenied));
        }
        let case = LocalRecoveryEvidenceCase {
            operation_id: operation,
            vault_id: ticket.vault_id,
            relative_path: ticket.path,
            expected_content_hash: ticket.expected_content_hash,
            evidence,
        };
        journal.record_local_recovery_case_once(&case)?;
        Ok(case)
    }

    /// M1d-b: preserve a COPY of the first incident's stage and/or final
    /// bytes in host app-data SQLite. No vault names are deleted, renamed,
    /// hidden or unlocked. A final hash-match is NOT ownership proof.
    pub(crate) fn archive_local_recovery_copy(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
    ) -> Result<LocalRecoveryArchivedCopy, LocalRecoveryError> {
        self.archive_local_recovery_copy_for_vault(
            operation, journal, super::VaultId::from_uuid(self.workspace_id),
        )
    }

    #[cfg(feature = "sync-s3")]
    pub(crate) fn archive_local_recovery_copy_for_registered_s3(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
        bindings: &super::profile_binding::SqliteRegisteredS3Bindings,
        profile: &super::S3ConnectionProfile, keyspace: &super::S3Keyspace,
    ) -> Result<LocalRecoveryArchivedCopy, LocalRecoveryError> {
        let vault = self.checked_registered_s3_recovery_scope(
            operation, journal, bindings, profile, keyspace,
        )?;
        self.archive_local_recovery_copy_for_vault(operation, journal, vault)
    }

    fn archive_local_recovery_copy_for_vault(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
        expected_vault: super::VaultId,
    ) -> Result<LocalRecoveryArchivedCopy, LocalRecoveryError> {
        // An attacker-controlled sync folder must never become the location
        // of an archive / WAL / SHM sidecar. The journal's host app-data path
        // is checked before archive bytes are written.
        journal.require_appdata_outside_vault(&self.root_path)?;
        let case = journal.read_local_recovery_case(operation)?
            .ok_or(LocalRecoveryError::MissingTicket)?;
        if case.vault_id != expected_vault
            || case.evidence.operation_id != operation
        {
            return Err(LocalRecoveryError::Admission(VaultPortError::PermissionDenied));
        }
        self.check_root()?;
        let relative = &case.relative_path;
        let target = self.root_path.join(relative.as_str());
        let _reservation = self.core.documents.reserve_sync_closed_document(&target)
            .map_err(admission_error)?;
        let _gate = self.core.file_mutations.lock()
            .map_err(|_| VaultPortError::PermissionDenied)?;
        self.check_root()?;

        let parent = self.pinned.open_parent_for_inspection(relative)?;
        let parent_id = local_create::directory_identity(&parent)?;
        if parent_id != case.evidence.parent_identity {
            return Err(LocalRecoveryError::UnstableObservation);
        }
        let leaf = relative.as_str().rsplit('/').next()
            .ok_or(LocalRecoveryError::Admission(VaultPortError::UnsafePath))?;
        let (stage, final_name) = local_recovery::capture_names(
            &parent, operation, leaf, &case.evidence,
            &case.expected_content_hash, MAX_PROTECTED_ORIGINAL_BYTES as u64,
        )?;
        // Copy bytes only after comparing current names again against the
        // historical identities. This still cannot prevent another process
        // changing the vault immediately after a successful archive.
        let observed = local_recovery::inspect_names(
            &parent, operation, leaf, &case.expected_content_hash,
            MAX_PROTECTED_ORIGINAL_BYTES as u64,
        )?;
        if observed.0 != case.evidence.stage || observed.1 != case.evidence.final_name {
            return Err(LocalRecoveryError::UnstableObservation);
        }
        let current_parent = self.pinned.open_parent_for_inspection(relative)
            .map_err(|_| LocalRecoveryError::UnstableObservation)?;
        if local_create::directory_identity(&current_parent)
            .map_err(|_| LocalRecoveryError::UnstableObservation)? != parent_id
        {
            return Err(LocalRecoveryError::UnstableObservation);
        }
        self.check_root()?;
        // A single INSERT/COMMIT writes durable independent bytes without
        // clearing the permanent local publication/recovery review tickets.
        let result = journal.record_local_recovery_copy_once(
            operation, stage.as_deref(), final_name.as_deref(),
        )?;
        Ok(result)
    }

    /// Host-scoped, metadata-only operator preview. The native UI must still
    /// authenticate an actual local operator and obtain explicit confirmation;
    /// the internal API alone does not prove identity or UI consent.
    pub(crate) fn recovery_operator_preview(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
    ) -> Result<LocalRecoveryReviewSnapshot, LocalRecoveryError> {
        self.recovery_operator_preview_for_vault(
            operation, journal, super::VaultId::from_uuid(self.workspace_id),
        )
    }

    #[cfg(feature = "sync-s3")]
    pub(crate) fn recovery_operator_preview_for_registered_s3(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
        bindings: &super::profile_binding::SqliteRegisteredS3Bindings,
        profile: &super::S3ConnectionProfile, keyspace: &super::S3Keyspace,
    ) -> Result<LocalRecoveryReviewSnapshot, LocalRecoveryError> {
        let vault = self.checked_registered_s3_recovery_scope(
            operation, journal, bindings, profile, keyspace,
        )?;
        self.recovery_operator_preview_for_vault(operation, journal, vault)
    }

    fn recovery_operator_preview_for_vault(
        &self, operation: OperationId, journal: &SqliteSyncJournal,
        expected_vault: super::VaultId,
    ) -> Result<LocalRecoveryReviewSnapshot, LocalRecoveryError> {
        self.check_root()?;
        journal.require_appdata_outside_vault(&self.root_path)?;
        let preview = journal.recovery_review_snapshot(operation)?;
        if preview.vault_id != expected_vault {
            return Err(LocalRecoveryError::Admission(VaultPortError::PermissionDenied));
        }
        self.check_root()?;
        Ok(preview)
    }

    /// Only two safe dispositions exist: KeepBlocked and EscalateManualReview.
    /// This records an explicit, revision-bound review event in app-data.
    /// There is NO code path to move/delete/retry a user file or clear an
    /// M1b one-shot publication claim or remote Sync baseline.
    pub(crate) fn record_recovery_operator_review(
        &self,
        operation: OperationId,
        displayed_path: &SyncRelativePath,
        displayed_archive_fingerprint: &str,
        displayed_review_count: u32,
        displayed_previous_digest: &str,
        disposition: RecoveryReviewDisposition,
        reason: &str,
        journal: &mut SqliteSyncJournal,
    ) -> Result<LocalRecoveryReviewEvent, LocalRecoveryError> {
        self.record_recovery_operator_review_for_vault(
            operation, displayed_path, displayed_archive_fingerprint,
            displayed_review_count, displayed_previous_digest,
            disposition, reason, journal,
            super::VaultId::from_uuid(self.workspace_id),
        )
    }

    /// Only blocked/manual-review events; never replay, unlink or commit.
    #[cfg(feature = "sync-s3")]
    pub(crate) fn record_recovery_operator_review_for_registered_s3(
        &self,
        operation: OperationId,
        displayed_path: &SyncRelativePath,
        displayed_archive_fingerprint: &str,
        displayed_review_count: u32,
        displayed_previous_digest: &str,
        disposition: RecoveryReviewDisposition,
        reason: &str,
        journal: &mut SqliteSyncJournal,
        bindings: &super::profile_binding::SqliteRegisteredS3Bindings,
        profile: &super::S3ConnectionProfile, keyspace: &super::S3Keyspace,
    ) -> Result<LocalRecoveryReviewEvent, LocalRecoveryError> {
        let vault = self.checked_registered_s3_recovery_scope(
            operation, journal, bindings, profile, keyspace,
        )?;
        self.record_recovery_operator_review_for_vault(
            operation, displayed_path, displayed_archive_fingerprint,
            displayed_review_count, displayed_previous_digest,
            disposition, reason, journal, vault,
        )
    }

    fn record_recovery_operator_review_for_vault(
        &self,
        operation: OperationId,
        displayed_path: &SyncRelativePath,
        displayed_archive_fingerprint: &str,
        displayed_review_count: u32,
        displayed_previous_digest: &str,
        disposition: RecoveryReviewDisposition,
        reason: &str,
        journal: &mut SqliteSyncJournal,
        expected_vault: super::VaultId,
    ) -> Result<LocalRecoveryReviewEvent, LocalRecoveryError> {
        let snapshot = self.recovery_operator_preview_for_vault(
            operation, journal, expected_vault,
        )?;
        if snapshot.path != *displayed_path
            || snapshot.archive_fingerprint != displayed_archive_fingerprint
            || snapshot.review_count != displayed_review_count
            || snapshot.latest_event_digest != displayed_previous_digest
        {
            return Err(LocalRecoveryError::Journal(JournalError::StaleRecoveryReview));
        }
        let target = self.root_path.join(displayed_path.as_str());
        let _reservation = self.core.documents.reserve_sync_closed_document(&target)
            .map_err(admission_error)?;
        let _gate = self.core.file_mutations.lock()
            .map_err(|_| VaultPortError::PermissionDenied)?;
        self.check_root()?;
        journal.require_appdata_outside_vault(&self.root_path)?;
        let recorded = journal.append_recovery_review(
            operation, expected_vault,
            displayed_path, displayed_archive_fingerprint,
            displayed_review_count, displayed_previous_digest,
            disposition, reason,
        )?;
        self.check_root()?;
        Ok(recorded)
    }

    /// Bind file identity by pinned handles without reading bytes or taking
    /// guest authority. A later read must verify this exact file ID.
    pub(crate) fn bind_plugin_file_identity(
        &self,
        relative: &SyncRelativePath,
    ) -> Result<PinnedFileIdentity, VaultPortError> {
        self.check_root()?;
        let file = self.pinned.identify_file(relative)?;
        self.check_root()?;
        Ok(file)
    }

    /// PLUGIN-04: descriptor-relative read of a closed editor document.
    /// The host must authorize first and revalidate the grant after reading.
    /// Reject hard-link aliases and a replaced file even with identical bytes.
    pub(crate) fn read_plugin_identified_bounded(
        &self,
        relative: &SyncRelativePath,
        expected_file: PinnedFileIdentity,
        max_bytes: u64,
    ) -> Result<LocalFileSnapshot, VaultPortError> {
        self.check_root()?;
        let path = self.root_path.join(relative.as_str());
        let _reservation = self.core.documents.reserve_sync_closed_document(&path)
            .map_err(admission_error)?;
        // Respect the same lock order as protected-original capture.
        let _gate = self.core.file_mutations.lock()
            .map_err(|_| VaultPortError::PermissionDenied)?;
        self.check_root()?;
        let first = self.pinned.read_identified_bounded(relative, max_bytes)?;
        let identity = first.identity.ok_or(VaultPortError::PermissionDenied)?;
        if identity != expected_file {
            return Err(VaultPortError::FileChanged);
        }
        self.check_root()?;
        let second = self.pinned.read_identified_bounded(relative, max_bytes)?;
        if second.identity != Some(identity)
            || first.snapshot.bytes != second.snapshot.bytes
            || first.snapshot.content_hash != second.snapshot.content_hash
        {
            return Err(VaultPortError::FileChanged);
        }
        self.check_root()?;
        Ok(second.snapshot)
    }

    /// A preflight only. Reserves a CLOSED editor session while reading, so
    /// neither a clean-but-open editor nor unsubmitted keystrokes may race this
    /// scan through DocumentHub::open. Other processes remain out of scope.
    pub(crate) fn read_closed_preflight(
        &self,
        relative: &SyncRelativePath,
        max_bytes: u64,
    ) -> Result<LocalFileSnapshot, VaultPortError> {
        self.check_root()?;
        let path = self.root_path.join(relative.as_str());
        let _reservation = self.core.documents.reserve_sync_closed_document(&path)
            .map_err(admission_error)?;
        self.check_root()?;
        let snapshot = self.pinned.read_bounded(relative, max_bytes)?;
        self.check_root()?;
        Ok(snapshot)
    }
}

fn io(error: std::io::Error) -> VaultPortError {
    VaultPortError::Io(error.to_string())
}

fn admission_error(error: SyncAdmissionError) -> VaultPortError {
    match error {
        SyncAdmissionError::DocumentOpen => VaultPortError::DirtyEditorBuffer,
        SyncAdmissionError::AlreadyReserved | SyncAdmissionError::LockUnavailable =>
            VaultPortError::PermissionDenied,
    }
}

fn reject_reparse_root(meta: &Metadata) -> Result<(), VaultPortError> {
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(VaultPortError::UnsafePath);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(VaultPortError::UnsafePath);
        }
    }
    Ok(())
}

// OS-native file identity, not path spelling or last-modified timestamps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RootIdentity(u64, u64);

fn pinned_identity(root: &PinnedReadOnlyRoot) -> Result<RootIdentity, VaultPortError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = root.root_metadata()?;
        Ok(RootIdentity(meta.dev(), meta.ino()))
    }
    #[cfg(windows)]
    {
        let (volume, index) = root.root_win_identity()?;
        Ok(RootIdentity(u64::from(volume), index))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = root;
        Err(VaultPortError::PermissionDenied)
    }
}

fn named_identity(path: &Path) -> Result<RootIdentity, VaultPortError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = fs::symlink_metadata(path).map_err(io)?;
        reject_reparse_root(&meta)?;
        Ok(RootIdentity(meta.dev(), meta.ino()))
    }
    #[cfg(windows)]
    {
        let root = super::pinned_windows::open_root(path)?;
        let (volume, index) = super::pinned_windows::file_identity(&root)?;
        Ok(RootIdentity(u64::from(volume), index))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(VaultPortError::PermissionDenied)
    }
}

#[cfg(test)]
#[path = "local_create_tests.rs"]
mod local_create_tests;

#[cfg(test)]
mod tests;
