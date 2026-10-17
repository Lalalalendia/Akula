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
            .filter(|item| same_path(Path::new(&item.path), &root_path));
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
                && same_path(Path::new(&item.path), &self.root_path)
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
