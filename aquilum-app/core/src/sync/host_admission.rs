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
use super::{LocalFileSnapshot, SyncRelativePath, VaultPortError};
use crate::app_core::Core;
use crate::documents::hub::SyncAdmissionError;
use crate::search::paths::{canonical_path, same_path};
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};

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

/// Not externally constructible, and deliberately has no mutation methods.
#[allow(dead_code)]
pub(crate) struct RegisteredReadAdmission<'a> {
    core: &'a Core,
    root_path: PathBuf,
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
        let registered = core.ui_state.workspace_roots()
            .map_err(|_| VaultPortError::PermissionDenied)?;
        if !registered.iter().any(|path| same_path(path, &root_path)) {
            return Err(VaultPortError::PermissionDenied);
        }

        let pinned = PinnedReadOnlyRoot::open(&root_path)?;
        let initial = pinned_identity(&pinned)?;
        if initial != named_identity(selected_root)? {
            return Err(VaultPortError::UnsafePath);
        }
        let admission = Self { core, root_path, pinned, initial };
        admission.check_root()?;
        Ok(admission)
    }

    fn check_root(&self) -> Result<(), VaultPortError> {
        let configured = self.core.ui_state.workspace_roots()
            .map_err(|_| VaultPortError::PermissionDenied)?;
        if !configured.iter().any(|path| same_path(path, &self.root_path)) {
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
}
