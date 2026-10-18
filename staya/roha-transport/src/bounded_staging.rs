//! P2P-BLOB-08 — fail-closed *preflight* for an isolated Linux staging mount.
//!
//! This guard checks that a private staging directory is backed by a distinct
//! filesystem instead of silently falling through to the user's host disk
//! when the expected bounded mount is missing. It does NOT prove any size cap:
//! the Linux VFS/mount administrator must separately create and verify one.
//!
//! Research-only for a disposable loopback ext4 CI image. This does NOT
//! authorize VaultId, DeviceId, data publication, FsStore, or File Gate.
//! Pathname-based checks remain susceptible to hostile concurrent mounts/
//! renames; a production adapter needs fd-relative custody and an OS-sandboxed
//! writer, not just a check followed by string-based path operations.

#![cfg(target_os = "linux")]

use std::{
    fs::{self, File},
    io,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub enum VolumeError {
    Io(io::Error),
    UnsafeDirectory,
    NotSeparateFilesystem,
    ChangedSinceAdmission,
}

/// Retains a directory file descriptor to reduce accidental unmount/rebind.
/// Every caller must revalidate before use, and must never turn this guard
/// into a claim of a measured/OS-enforced disk quota.
pub struct StagingVolumeGuard {
    root: PathBuf,
    _pinned_dir: File,
    device: u64,
    inode: u64,
}

impl StagingVolumeGuard {
    /// The root must already exist with mode 0700 on its own filesystem.
    /// No mounting or privileged host operation happens in Rust.
    pub fn open(root: &Path) -> Result<Self, VolumeError> {
        let metadata = fs::symlink_metadata(root).map_err(VolumeError::Io)?;
        if !metadata.file_type().is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(VolumeError::UnsafeDirectory);
        }
        let parent = root.parent().ok_or(VolumeError::UnsafeDirectory)?;
        let parent_meta = fs::metadata(parent).map_err(VolumeError::Io)?;
        if metadata.dev() == parent_meta.dev() {
            return Err(VolumeError::NotSeparateFilesystem);
        }
        let handle = File::open(root).map_err(VolumeError::Io)?;
        let anchored = handle.metadata().map_err(VolumeError::Io)?;
        if anchored.dev() != metadata.dev() || anchored.ino() != metadata.ino() {
            return Err(VolumeError::ChangedSinceAdmission);
        }
        Ok(Self {
            root: root.to_owned(),
            _pinned_dir: handle,
            device: anchored.dev(),
            inode: anchored.ino(),
        })
    }

    /// Borrow the pinned staging mount root for read-only inventory checks.
    /// Path-based child lookup still needs independent race-free custody.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Fail closed when the mount disappears, gets replaced or the directory
    /// becomes accessible to unrelated local users between operations.
    pub fn check(&self) -> Result<(), VolumeError> {
        let metadata = fs::symlink_metadata(&self.root).map_err(VolumeError::Io)?;
        if !metadata.file_type().is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(VolumeError::UnsafeDirectory);
        }
        if metadata.dev() != self.device || metadata.ino() != self.inode {
            return Err(VolumeError::ChangedSinceAdmission);
        }
        let parent = fs::metadata(self.root.parent().ok_or(VolumeError::UnsafeDirectory)?)
            .map_err(VolumeError::Io)?;
        if parent.dev() == metadata.dev() {
            return Err(VolumeError::NotSeparateFilesystem);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn refuses_unmounted_directory_in_host_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("looks-like-volume");
        fs::create_dir(&child).unwrap();
        fs::set_permissions(&child, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(
            StagingVolumeGuard::open(&child),
            Err(VolumeError::NotSeparateFilesystem)
        ));
    }

    #[test]
    fn refuses_symlink_root_and_world_accessible_directory() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("stage");
        fs::create_dir(&child).unwrap();
        let link = dir.path().join("link");
        symlink(&child, &link).unwrap();
        assert!(matches!(
            StagingVolumeGuard::open(&link),
            Err(VolumeError::UnsafeDirectory)
        ));
        fs::set_permissions(&child, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            StagingVolumeGuard::open(&child),
            Err(VolumeError::UnsafeDirectory)
        ));
    }
}
