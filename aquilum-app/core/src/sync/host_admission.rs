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
use super::journal::LocalRecoveryEvidenceCase;
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

    pub(crate) fn registered_workspace_id(&self) -> Uuid {
        self.workspace_id
    }

    pub(crate) fn recheck_registered_root(&self) -> Result<(), VaultPortError> {
        self.check_root()
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
        let record = journal.get(operation)?
            .ok_or(LocalCreateError::InvalidStagedIntent)?;
        if record.phase != JournalPhase::Staged
            || record.intent.action != JournalAction::StageDownload
            || record.intent.remote_guard.is_some()
            || record.intent.vault_id.as_uuid() != self.workspace_id
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
        let ticket = journal.local_publication_review_queue()?
            .into_iter().find(|entry| entry.operation_id == operation)
            .ok_or(LocalRecoveryError::MissingTicket)?;
        if ticket.vault_id.as_uuid() != self.workspace_id {
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
        let evidence = self.inspect_local_publication(operation, journal)?;
        self.check_root()?;
        let ticket = journal.local_publication_review_queue()?
            .into_iter().find(|entry| entry.operation_id == operation)
            .ok_or(LocalRecoveryError::MissingTicket)?;
        if ticket.vault_id.as_uuid() != self.workspace_id {
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
mod tests {
    use super::*;
    use crate::app_core::{Core, CoreEvent};
    use std::sync::Arc;

    fn fixture() -> (tempfile::TempDir, Arc<Core>, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        fs::create_dir(&root).unwrap();
        let appdata = temp.path().join("appdata");
        fs::create_dir(&appdata).unwrap();
        let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        (temp, core, root)
    }

    fn note() -> SyncRelativePath {
        SyncRelativePath::parse("notes/caf\u{e9}.md").unwrap()
    }

    #[test]
    fn host_admission_reads_exact_unicode_bytes_without_writing() {
        let (_temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let bytes = "Русский \u{1f600}\n".as_bytes();
        let path = root.join(note().as_str());
        fs::write(&path, bytes).unwrap();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let snapshot = guard.read_closed_preflight(&note(), 1024).unwrap();
        assert_eq!(snapshot.bytes, bytes);
        assert_eq!(snapshot.content_hash, blake3::hash(bytes).to_hex().to_string());
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert!(guard.read_closed_preflight(&note(), 0).is_err());
        assert!(guard.read_closed_preflight(&note(), (bytes.len() - 1) as u64).is_err());
    }

    #[test]
    fn host_admission_refuses_unregistered_root_and_revoked_registration() {
        let (_temp, core, root) = fixture();
        let unregistered = tempfile::tempdir().unwrap();
        assert!(matches!(
            RegisteredReadAdmission::open(&core, unregistered.path()),
            Err(VaultPortError::PermissionDenied)
        ));
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let id = core.ui_state.list_workspaces().unwrap()[0].id;
        core.ui_state.forget_workspace(id).unwrap();
        assert!(matches!(
            guard.read_closed_preflight(&note(), 1024),
            Err(VaultPortError::PermissionDenied)
        ));
    }

    #[test]
    fn host_admission_blocks_open_editors_and_concurrent_reservations() {
        let (_temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let path = root.join(note().as_str());
        fs::write(&path, b"unchanged").unwrap();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();

        core.documents.open(&core, &path, 0).unwrap();
        assert!(matches!(
            guard.read_closed_preflight(&note(), 1024),
            Err(VaultPortError::DirtyEditorBuffer)
        ));
        core.documents.release(&core, &path);

        let held = core.documents.reserve_sync_closed_document(&path).unwrap();
        assert!(matches!(
            guard.read_closed_preflight(&note(), 1024),
            Err(VaultPortError::PermissionDenied)
        ));
        drop(held);
        assert_eq!(guard.read_closed_preflight(&note(), 1024).unwrap().bytes, b"unchanged");
    }

    #[test]
    fn host_admission_rejects_root_replacement_instead_of_reading_either_tree() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join(note().as_str()), b"original").unwrap();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();

        fs::rename(&root, temp.path().join("old-vault")).unwrap();
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join(note().as_str()), b"substituted").unwrap();

        assert!(guard.read_closed_preflight(&note(), 1024).is_err());
        assert_eq!(fs::read(root.join(note().as_str())).unwrap(), b"substituted");
    }

    #[cfg(unix)]
    #[test]
    fn host_admission_rejects_symlinked_workspace_and_child() {
        use std::os::unix::fs::symlink;
        let (temp, core, root) = fixture();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("private.md"), b"secret").unwrap();
        symlink(outside.path(), root.join("jump")).unwrap();
        assert!(RegisteredReadAdmission::open(
            &core, &temp.path().join("alias")
        ).is_err());
        symlink(&root, temp.path().join("alias")).unwrap();
        assert!(RegisteredReadAdmission::open(
            &core, &temp.path().join("alias")
        ).is_err());
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        assert!(guard.read_closed_preflight(
            &SyncRelativePath::parse("jump/private.md").unwrap(), 1024
        ).is_err());
    }

    fn protection_store(temp: &tempfile::TempDir) -> SqliteProtectionStore {
        SqliteProtectionStore::open(&temp.path().join("appdata/protected.sqlite")).unwrap()
    }

    #[test]
    fn live_preimage_survives_database_restart_with_bound_root_and_file_identity() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let bytes = "Глава 140. Русский текст 😀".as_bytes();
        let note = note();
        fs::write(root.join(note.as_str()), bytes).unwrap();
        let hash = blake3::hash(bytes).to_hex().to_string();
        let operation = OperationId::new();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let original_root = guard.initial;
        let file_id = guard.pinned.read_identified_bounded(
            &note, MAX_PROTECTED_ORIGINAL_BYTES as u64).unwrap().identity.unwrap();
        {
            let mut store = protection_store(&temp);
            assert_eq!(
                guard.preserve_closed_original(&note, &hash, operation, &mut store).unwrap(),
                ProtectionOutcome::Stored
            );
            assert_eq!(
                guard.preserve_closed_original(&note, &hash, operation, &mut store).unwrap(),
                ProtectionOutcome::AlreadyStored
            );
        }
        let store = protection_store(&temp);
        let saved = store.read_bound_preimage(operation).unwrap().unwrap();
        assert_eq!(saved.original.bytes, bytes);
        assert_eq!(saved.original.path, note);
        assert_eq!(saved.original.content_hash, hash);
        assert_eq!(saved.root.device, original_root.0);
        assert_eq!(saved.root.file, original_root.1);
        assert_eq!(saved.file, file_id);
        assert_eq!(fs::read(root.join(note.as_str())).unwrap(), bytes);
    }

    #[test]
    fn preimage_capture_rejects_unexpected_hash_or_open_editor_without_backup() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let note = note();
        let target = root.join(note.as_str());
        fs::write(&target, b"original").unwrap();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        let op = OperationId::new();

        assert!(matches!(
            guard.preserve_closed_original(&note, "wrong", op, &mut store),
            Err(BoundCaptureError::OriginalHashMismatch)
        ));
        core.documents.open(&core, &target, 0).unwrap();
        let digest = blake3::hash(b"original").to_hex().to_string();
        assert!(matches!(
            guard.preserve_closed_original(&note, &digest, op, &mut store),
            Err(BoundCaptureError::Admission(VaultPortError::DirtyEditorBuffer))
        ));
        core.documents.release(&core, &target);
        assert!(store.read_preimage(op).unwrap().is_none());
        assert_eq!(fs::read(target).unwrap(), b"original");
    }

    #[test]
    fn replacing_a_file_with_identical_bytes_does_not_preserve_old_file_identity() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let note = note();
        let target = root.join(note.as_str());
        fs::write(&target, b"same text").unwrap();
        let digest = blake3::hash(b"same text").to_hex().to_string();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        let id = OperationId::new();

        let result = guard.preserve_with_hook(&note, &digest, id, &mut store, || {
            fs::rename(&target, root.join("notes/moved.md")).unwrap();
            fs::write(&target, b"same text").unwrap();
        });
        assert!(matches!(
            result, Err(BoundCaptureError::Admission(VaultPortError::FileChanged))
        ));
        let saved = store.read_bound_preimage(id).unwrap().unwrap();
        assert_eq!(saved.original.bytes, b"same text");
        assert_eq!(fs::read(&target).unwrap(), b"same text");
        // Reusing the same operation ID after the file was swapped must not
        // silently confer the old file's recorded identity on a new inode.
        assert!(matches!(
            guard.preserve_closed_original(&note, &digest, id, &mut store),
            Err(BoundCaptureError::Protection(ProtectionStoreError::OperationCollision))
        ));
    }

    #[test]
    fn root_replacement_before_persistence_refuses_without_storing_a_backup() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join(note().as_str()), b"original").unwrap();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        let operation = OperationId::new();
        let digest = blake3::hash(b"original").to_hex().to_string();
        let result = guard.preserve_with_hook(&note(), &digest, operation, &mut store, || {
            fs::rename(&root, temp.path().join("moved-vault")).unwrap();
            fs::create_dir(&root).unwrap();
            fs::create_dir(root.join("notes")).unwrap();
            fs::write(root.join(note().as_str()), b"replacement").unwrap();
        });
        assert!(result.is_err());
        assert!(store.read_preimage(operation).unwrap().is_none());
        assert_eq!(fs::read(root.join(note().as_str())).unwrap(), b"replacement");
    }

    #[test]
    fn hardlink_alias_and_workspace_local_database_cannot_pass_capture() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let target = root.join(note().as_str());
        fs::write(&target, b"protected source").unwrap();
        fs::hard_link(&target, root.join("notes/alias.md")).unwrap();
        let digest = blake3::hash(b"protected source").to_hex().to_string();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut outside_store = protection_store(&temp);
        let id = OperationId::new();
        assert!(matches!(
            guard.preserve_closed_original(&note(), &digest, id, &mut outside_store),
            Err(BoundCaptureError::Admission(VaultPortError::UnsafePath))
        ));
        assert!(outside_store.read_preimage(id).unwrap().is_none());

        fs::remove_file(root.join("notes/alias.md")).unwrap();
        let mut inside_store = SqliteProtectionStore::open(&root.join("protection.sqlite")).unwrap();
        assert!(matches!(
            guard.preserve_closed_original(&note(), &digest, id, &mut inside_store),
            Err(BoundCaptureError::Admission(VaultPortError::PermissionDenied))
        ));
        assert!(inside_store.read_preimage(id).unwrap().is_none());
    }

    #[test]
    fn durable_preimage_revalidation_after_restart_is_read_only() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let path = root.join(note().as_str());
        fs::write(&path, b"stable original").unwrap();
        let expected = blake3::hash(b"stable original").to_hex().to_string();
        let operation = OperationId::new();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        {
            let mut store = protection_store(&temp);
            guard.preserve_closed_original(&note(), &expected, operation, &mut store).unwrap();
        }
        let store = protection_store(&temp);
        guard.revalidate_bound_preimage(&note(), &expected, operation, &store).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"stable original");
    }

    #[test]
    fn durable_preimage_revalidation_rejects_same_byte_replacement() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let path = root.join(note().as_str());
        fs::write(&path, b"same content").unwrap();
        let expected = blake3::hash(b"same content").to_hex().to_string();
        let operation = OperationId::new();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        guard.preserve_closed_original(&note(), &expected, operation, &mut store).unwrap();

        fs::rename(&path, root.join("notes/old-identity.md")).unwrap();
        fs::write(&path, b"same content").unwrap();
        assert!(matches!(
            guard.revalidate_bound_preimage(&note(), &expected, operation, &store),
            Err(BoundCaptureError::Admission(VaultPortError::FileChanged))
        ));
        assert_eq!(fs::read(&path).unwrap(), b"same content");
        assert_eq!(store.read_bound_preimage(operation).unwrap().unwrap().original.bytes,
            b"same content");
    }

    #[test]
    fn durable_preimage_revalidation_rejects_wrong_operation_and_open_editor() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let path = root.join(note().as_str());
        fs::write(&path, b"original").unwrap();
        let expected = blake3::hash(b"original").to_hex().to_string();
        let operation = OperationId::new();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        guard.preserve_closed_original(&note(), &expected, operation, &mut store).unwrap();

        assert!(matches!(
            guard.revalidate_bound_preimage(&note(), &expected, OperationId::new(), &store),
            Err(BoundCaptureError::Protection(ProtectionStoreError::UnboundOriginal))
        ));
        assert!(matches!(
            guard.revalidate_bound_preimage(&note(), "wrong-hash", operation, &store),
            Err(BoundCaptureError::Protection(ProtectionStoreError::OperationCollision))
        ));
        core.documents.open(&core, &path, 0).unwrap();
        assert!(matches!(
            guard.revalidate_bound_preimage(&note(), &expected, operation, &store),
            Err(BoundCaptureError::Admission(VaultPortError::DirtyEditorBuffer))
        ));
        core.documents.release(&core, &path);
        assert_eq!(fs::read(path).unwrap(), b"original");
    }

    #[test]
    fn durable_preimage_revalidation_rejects_replaced_registered_root() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join(note().as_str()), b"original").unwrap();
        let expected = blake3::hash(b"original").to_hex().to_string();
        let operation = OperationId::new();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        guard.preserve_closed_original(&note(), &expected, operation, &mut store).unwrap();

        let old_root = temp.path().join("old-vault");
        fs::rename(&root, &old_root).unwrap();
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join(note().as_str()), b"original").unwrap();
        assert!(guard.revalidate_bound_preimage(&note(), &expected, operation, &store).is_err());
        assert_eq!(fs::read(root.join(note().as_str())).unwrap(), b"original");
        assert_eq!(fs::read(old_root.join(note().as_str())).unwrap(), b"original");
    }


    // The current Rust test binary is also our minimal cross-platform child.
    // The parent passes paths through environment values so Unicode and spaces
    // are not parsed by a shell. The child's direct std::fs calls deliberately
    // DO NOT participate in Akula's process-wide Core File Gate.
    #[test]
    fn external_process_path_swap_fixture() {
        let Some(mode) = std::env::var_os("AKULA_SYNC_TEST_SWAP_MODE") else {
            return;
        };
        let target = PathBuf::from(std::env::var_os("AKULA_SYNC_TEST_TARGET").unwrap());
        let marker = PathBuf::from(std::env::var_os("AKULA_SYNC_TEST_MARKER").unwrap());
        match mode.to_str().unwrap() {
            "file" => {
                let moved = target.with_extension("saved-pre-swap");
                fs::rename(&target, &moved).unwrap();
                // Same bytes, different inode/NTFS file ID. Holding on to the
                // old renamed file prevents file ID reuse in this fixture.
                fs::write(&target, b"same bytes").unwrap();
            }
            #[cfg(unix)]
            "directory" => {
                use std::os::unix::fs::symlink;
                let moved = PathBuf::from(std::env::var_os("AKULA_SYNC_TEST_MOVED").unwrap());
                let outside = PathBuf::from(std::env::var_os("AKULA_SYNC_TEST_OUTSIDE").unwrap());
                fs::rename(&target, &moved).unwrap();
                symlink(&outside, &target).unwrap();
            }
            other => panic!("unsupported child operation: {other}"),
        }
        fs::write(marker, b"child-swapped").unwrap();
    }

    fn run_external_swap(
        mode: &str,
        target: &Path,
        marker: &Path,
        moved: Option<&Path>,
        outside: Option<&Path>,
    ) {
        use std::process::Command;
        let mut child = Command::new(std::env::current_exe().unwrap());
        child.arg("external_process_path_swap_fixture")
            .arg("--nocapture")
            .env("AKULA_SYNC_TEST_SWAP_MODE", mode)
            .env("AKULA_SYNC_TEST_TARGET", target)
            .env("AKULA_SYNC_TEST_MARKER", marker);
        if let Some(path) = moved {
            child.env("AKULA_SYNC_TEST_MOVED", path);
        }
        if let Some(path) = outside {
            child.env("AKULA_SYNC_TEST_OUTSIDE", path);
        }
        let status = child.status().unwrap();
        assert!(status.success(), "external test process failed: {status:?}");
        assert_eq!(fs::read(marker).unwrap(), b"child-swapped",
            "test runner must actually execute the child fixture");
    }

    #[test]
    fn host_admission_detects_foreign_process_swap_while_core_gate_is_held() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let relative = note();
        let target = root.join(relative.as_str());
        fs::write(&target, b"same bytes").unwrap();
        let digest = blake3::hash(b"same bytes").to_hex().to_string();
        let operation = OperationId::new();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        guard.preserve_closed_original(&relative, &digest, operation, &mut store).unwrap();

        // An ordinary file editor in another process does not acquire this
        // mutex. This is a deterministic interprocess replacement, not a
        // claim that revalidation prevents swaps immediately after it returns.
        let held = core.file_mutations.lock().unwrap();
        let marker = temp.path().join("foreign-file-swapped");
        run_external_swap("file", &target, &marker, None, None);
        assert_eq!(fs::read(&target).unwrap(), b"same bytes");
        drop(held);

        assert!(matches!(
            guard.revalidate_bound_preimage(&relative, &digest, operation, &store),
            Err(BoundCaptureError::Admission(VaultPortError::FileChanged))
        ));
        assert_eq!(store.read_bound_preimage(operation).unwrap().unwrap().original.bytes,
            b"same bytes");
        assert_eq!(fs::read(target.with_extension("saved-pre-swap")).unwrap(),
            b"same bytes");
    }

    #[cfg(unix)]
    #[test]
    fn host_admission_detects_foreign_process_symlink_parent_swap() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        let relative = note();
        fs::write(root.join(relative.as_str()), b"original").unwrap();
        let digest = blake3::hash(b"original").to_hex().to_string();
        let operation = OperationId::new();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        guard.preserve_closed_original(&relative, &digest, operation, &mut store).unwrap();

        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("café.md"), b"external secret").unwrap();
        let moved = temp.path().join("old-notes");
        let marker = temp.path().join("foreign-directory-swapped");
        let held = core.file_mutations.lock().unwrap();
        run_external_swap("directory", &root.join("notes"), &marker, Some(&moved), Some(&outside));
        drop(held);

        assert!(guard.revalidate_bound_preimage(&relative, &digest, operation, &store).is_err());
        assert_eq!(fs::read(outside.join("café.md")).unwrap(), b"external secret");
        assert_eq!(fs::read(moved.join("café.md")).unwrap(), b"original");
        assert_eq!(store.read_bound_preimage(operation).unwrap().unwrap().original.bytes,
            b"original");
    }

    #[test]
    fn protected_preimage_capture_holds_core_file_gate_interlock() {
        let (temp, core, root) = fixture();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join(note().as_str()), b"first").unwrap();
        let expected = blake3::hash(b"first").to_hex().to_string();
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut store = protection_store(&temp);
        let operation = OperationId::new();
        assert_eq!(
            guard.preserve_with_hook(&note(), &expected, operation, &mut store, || {
                assert!(
                    core.file_mutations.try_lock().is_err(),
                    "host provenance must hold the same Core mutex as ordinary Gate writes"
                );
            }).unwrap(),
            ProtectionOutcome::Stored
        );
        assert_eq!(store.read_bound_preimage(operation).unwrap().unwrap().original.bytes, b"first");
    }
}
