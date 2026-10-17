//! Stage 04g-3a: fail-closed *read-only* Core host admission.
//!
//! Only Core-registered workspaces can be opened, and each preflight read
//! holds a DocumentHub reservation for its duration. A pinned directory handle
//! is compared with the live registered pathname before and after each read.
//! This is NOT a VaultPort, protective receipt, or local mutation authority:
//! no sync executor can create, replace, rename, delete or commit a baseline.

use super::pinned_read::PinnedReadOnlyRoot;
use super::{LocalFileSnapshot, SyncRelativePath, VaultPortError};
use crate::app_core::Core;
use crate::documents::hub::SyncAdmissionError;
use crate::search::paths::{canonical_path, same_path};
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};

/// Not externally constructible, and deliberately has no mutation methods.
#[allow(dead_code)]
pub(crate) struct RegisteredReadAdmission<'a> {
    core: &'a Core,
    root_path: PathBuf,
    pinned: PinnedReadOnlyRoot,
    initial: Metadata,
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
        let initial = pinned.root_metadata()?;
        if !same_root_identity(&initial, &selected) {
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
        let live = fs::symlink_metadata(&self.root_path).map_err(io)?;
        reject_reparse_root(&live)?;
        let pinned = self.pinned.root_metadata()?;
        // A replaced pathname must never cause a silent switch to another
        // directory, even though the old descriptor is still readable.
        if !same_root_identity(&pinned, &self.initial)
            || !same_root_identity(&live, &self.initial)
        {
            return Err(VaultPortError::FileChanged);
        }
        Ok(())
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

fn same_root_identity(first: &Metadata, second: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return first.dev() == second.dev() && first.ino() == second.ino();
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Missing volume/file IDs cannot create authorization by accident.
        return matches!(
            (first.volume_serial_number(), first.file_index(),
             second.volume_serial_number(), second.file_index()),
            (Some(av), Some(ai), Some(bv), Some(bi)) if av == bv && ai == bi
        );
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (first, second);
        false
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
