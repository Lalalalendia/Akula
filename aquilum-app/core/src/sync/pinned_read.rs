//! Stage 04g-1: bounded read-only descriptor preflight for Unix hosts.
//!
//! This is NOT a sync VaultPort, does not enumerate a complete snapshot,
//! and must not authorize uploads, writes, renames, deletions or baseline commits.
//! The caller must separately authenticate the registered workspace root.
//! No path-based fallback is permitted on unsupported operating systems.

use super::{LocalFileSnapshot, SyncRelativePath, VaultPortError};
use std::path::Path;

#[cfg(unix)]
use rustix::fd::OwnedFd;
#[cfg(unix)]
use rustix::fs::{open, openat, Mode, OFlags};
#[cfg(any(unix, windows))]
use std::io::Read;

/// Pins the opening directory so later replacement of its pathname cannot
/// redirect reads through that pathname. This is a *preflight reader only*:
/// neither an atomic workspace snapshot nor a mutation authority.
pub(crate) struct PinnedReadOnlyRoot {
    #[cfg(unix)]
    root: OwnedFd,
    #[cfg(windows)]
    root: std::fs::File,
}

/// Identity of an opened file descriptor / HANDLE, not a pathname lookup.
/// Never treat this value alone as an authorization to mutate a document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PinnedFileIdentity {
    pub device: u64,
    pub file: u64,
}

pub(super) struct IdentifiedRead {
    pub snapshot: LocalFileSnapshot,
    /// None on the existing generic read path; Some only for checked preimages.
    pub identity: Option<PinnedFileIdentity>,
}

impl PinnedReadOnlyRoot {
    /// Stable metadata for the live pinned handle, not a later path lookup.
    /// This supplies host registration checks, never sync write authority.
    pub(crate) fn root_metadata(&self) -> Result<std::fs::Metadata, VaultPortError> {
        #[cfg(unix)]
        {
            let copy = rustix::io::dup(&self.root)
                .map_err(|error| VaultPortError::Io(error.to_string()))?;
            std::fs::File::from(copy).metadata()
                .map_err(|error| VaultPortError::Io(error.to_string()))
        }
        #[cfg(windows)]
        {
            self.root.metadata().map_err(|error| VaultPortError::Io(error.to_string()))
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(VaultPortError::PermissionDenied)
        }
    }

    /// Root must originate from a trusted, registered workspace selection.
    /// An untrusted remote path or manifest must never choose this directory.
    pub(crate) fn open(root: &Path) -> Result<Self, VaultPortError> {
        #[cfg(unix)]
        {
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW
                | OFlags::CLOEXEC;
            let fd = open(root, flags, Mode::empty())
                .map_err(|err| VaultPortError::Io(err.to_string()))?;
            Ok(Self { root: fd })
        }
        #[cfg(windows)]
        {
            let root = super::pinned_windows::open_root(root)?;
            Ok(Self { root })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = root;
            Err(VaultPortError::PermissionDenied)
        }
    }

    #[cfg(windows)]
    pub(crate) fn root_win_identity(&self) -> Result<(u32, u64), VaultPortError> {
        super::pinned_windows::file_identity(&self.root)
    }

    /// Pin a file identity without reading content; this is only for
    /// trusted host indexing, never a guest-callable API.
    pub(crate) fn identify_file(
        &self, relative: &SyncRelativePath,
    ) -> Result<PinnedFileIdentity, VaultPortError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let file = self.open_relative_file(relative)?;
            let meta = file.metadata()
                .map_err(|err| VaultPortError::Io(err.to_string()))?;
            if !meta.is_file() || meta.ino() == 0 || meta.nlink() != 1 {
                return Err(VaultPortError::UnsafePath);
            }
            Ok(PinnedFileIdentity { device: meta.dev(), file: meta.ino() })
        }
        #[cfg(windows)]
        {
            let file = self.open_relative_file(relative)?;
            let meta = file.metadata()
                .map_err(|err| VaultPortError::Io(err.to_string()))?;
            if !meta.is_file() {
                return Err(VaultPortError::UnsafePath);
            }
            let (volume, id, links) = super::pinned_windows::file_identity_with_links(&file)?;
            if links != 1 {
                return Err(VaultPortError::UnsafePath);
            }
            Ok(PinnedFileIdentity { device: u64::from(volume), file: id })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = relative;
            Err(VaultPortError::PermissionDenied)
        }
    }

    #[cfg(unix)]
    fn open_relative_file(
        &self, relative: &SyncRelativePath,
    ) -> Result<std::fs::File, VaultPortError> {
        let mut parent = rustix::io::dup(&self.root)
            .map_err(|err| VaultPortError::Io(err.to_string()))?;
        let mut names = relative.as_str().split('/').peekable();
        while let Some(name) = names.next() {
            let leaf = names.peek().is_none();
            let flags = if leaf {
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC
            } else {
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
            };
            parent = openat(&parent, name, flags, Mode::empty())
                .map_err(|err| VaultPortError::Io(err.to_string()))?;
        }
        Ok(std::fs::File::from(parent))
    }

    #[cfg(windows)]
    fn open_relative_file(
        &self, relative: &SyncRelativePath,
    ) -> Result<std::fs::File, VaultPortError> {
        let mut parent = self.root.try_clone()
            .map_err(|err| VaultPortError::Io(err.to_string()))?;
        let mut names = relative.as_str().split('/').peekable();
        while let Some(name) = names.next() {
            let leaf = names.peek().is_none();
            parent = super::pinned_windows::open_child(&parent, name, leaf)?;
        }
        Ok(parent)
    }

    /// Opens each path component relative to its pinned parent FD, without
    /// following symlinks at any level. The final FD is checked as a regular
    /// file *before* bounded reading. No caller gets write rights.
    ///
    /// This does not make reading an atomic point-in-time snapshot of a file
    /// concurrently modified by another process.
    pub(crate) fn read_bounded(
        &self,
        relative: &SyncRelativePath,
        max_bytes: u64,
    ) -> Result<LocalFileSnapshot, VaultPortError> {
        Ok(self.read_inner(relative, max_bytes, false)?.snapshot)
    }

    /// Preimage capture requires a stable opened-file ID and exactly one hard
    /// link; a second pathname to the same inode must not inherit authority.
    /// This remains *read-only*, and does not provide an atomic filesystem lock.
    pub(super) fn read_identified_bounded(
        &self,
        relative: &SyncRelativePath,
        max_bytes: u64,
    ) -> Result<IdentifiedRead, VaultPortError> {
        self.read_inner(relative, max_bytes, true)
    }

    fn read_inner(
        &self,
        relative: &SyncRelativePath,
        max_bytes: u64,
        require_identity: bool,
    ) -> Result<IdentifiedRead, VaultPortError> {
        const HARD_LIMIT: u64 = 32 * 1024 * 1024;
        if max_bytes == 0 || max_bytes > HARD_LIMIT {
            return Err(VaultPortError::PermissionDenied);
        }

        #[cfg(unix)]
        {
            let mut file = self.open_relative_file(relative)?;
            let before = file.metadata()
                .map_err(|err| VaultPortError::Io(err.to_string()))?;
            if !before.is_file() {
                return Err(VaultPortError::UnsafePath);
            }
            use std::os::unix::fs::MetadataExt;
            let identity = if require_identity {
                if before.ino() == 0 || before.nlink() != 1 {
                    return Err(VaultPortError::UnsafePath);
                }
                Some(PinnedFileIdentity { device: before.dev(), file: before.ino() })
            } else {
                None
            };
            if before.len() > max_bytes {
                return Err(VaultPortError::PermissionDenied);
            }

            let mut bytes = Vec::new();
            (&mut file).take(max_bytes + 1).read_to_end(&mut bytes)
                .map_err(|err| VaultPortError::Io(err.to_string()))?;
            if bytes.len() as u64 > max_bytes {
                return Err(VaultPortError::PermissionDenied);
            }
            let after = file.metadata()
                .map_err(|err| VaultPortError::Io(err.to_string()))?;
            if !after.is_file() || before.len() != after.len()
                || before.modified().ok() != after.modified().ok()
                || after.len() != bytes.len() as u64
                || identity.is_some_and(|id|
                    after.dev() != id.device || after.ino() != id.file || after.nlink() != 1)
            {
                return Err(VaultPortError::FileChanged);
            }
            Ok(IdentifiedRead {
                snapshot: LocalFileSnapshot {
                    content_hash: blake3::hash(&bytes).to_hex().to_string(),
                    bytes,
                },
                identity,
            })
        }
        #[cfg(windows)]
        {
            let mut parent = self.open_relative_file(relative)?;
            let before = parent.metadata()
                .map_err(|error| VaultPortError::Io(error.to_string()))?;
            if !before.is_file() {
                return Err(VaultPortError::UnsafePath);
            }
            let identity = if require_identity {
                let (device, file, links) =
                    super::pinned_windows::file_identity_with_links(&parent)?;
                if links != 1 {
                    return Err(VaultPortError::UnsafePath);
                }
                Some(PinnedFileIdentity { device: u64::from(device), file })
            } else {
                None
            };
            if before.len() > max_bytes {
                return Err(VaultPortError::PermissionDenied);
            }
            let mut bytes = Vec::new();
            (&mut parent).take(max_bytes + 1).read_to_end(&mut bytes)
                .map_err(|error| VaultPortError::Io(error.to_string()))?;
            if bytes.len() as u64 > max_bytes {
                return Err(VaultPortError::PermissionDenied);
            }
            let after = parent.metadata()
                .map_err(|error| VaultPortError::Io(error.to_string()))?;
            if !after.is_file() || before.len() != after.len()
                || before.modified().ok() != after.modified().ok()
                || after.len() != bytes.len() as u64
            {
                return Err(VaultPortError::FileChanged);
            }
            if let Some(id) = identity {
                let (device, file, links) =
                    super::pinned_windows::file_identity_with_links(&parent)?;
                if u64::from(device) != id.device || file != id.file || links != 1 {
                    return Err(VaultPortError::FileChanged);
                }
            }
            Ok(IdentifiedRead {
                snapshot: LocalFileSnapshot {
                    content_hash: blake3::hash(&bytes).to_hex().to_string(),
                    bytes,
                },
                identity,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (self, relative, require_identity);
            Err(VaultPortError::PermissionDenied)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn pins_the_original_root_even_after_its_path_is_replaced() {
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let old_root = temp.path().join("moved-vault");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("note.md"), b"original").unwrap();
        let reader = PinnedReadOnlyRoot::open(&root).unwrap();

        fs::rename(&root, &old_root).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(root.join("note.md"), b"replacement").unwrap();

        let path = SyncRelativePath::parse("note.md").unwrap();
        assert_eq!(reader.read_bounded(&path, 1024).unwrap().bytes, b"original");
        assert_eq!(fs::read(root.join("note.md")).unwrap(), b"replacement");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_in_intermediate_and_final_components() {
        use std::fs;
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        fs::create_dir(&root).unwrap();
        fs::write(outside.path().join("private.md"), b"outside").unwrap();
        symlink(outside.path(), root.join("outside-link")).unwrap();
        symlink(outside.path().join("private.md"), root.join("file-link.md")).unwrap();

        let reader = PinnedReadOnlyRoot::open(&root).unwrap();
        for path in ["outside-link/private.md", "file-link.md"] {
            let path = SyncRelativePath::parse(path).unwrap();
            assert!(reader.read_bounded(&path, 1024).is_err());
        }
        assert!(PinnedReadOnlyRoot::open(&root.join("outside-link")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_oversize_and_nonregular_objects_without_reading_them() {
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("binary.dat"), [1u8, 2, 3, 4]).unwrap();
        fs::create_dir(temp.path().join("directory")).unwrap();
        let reader = PinnedReadOnlyRoot::open(temp.path()).unwrap();

        let binary = SyncRelativePath::parse("binary.dat").unwrap();
        let directory = SyncRelativePath::parse("directory").unwrap();
        assert!(reader.read_bounded(&binary, 3).is_err());
        assert_eq!(reader.read_bounded(&binary, 4).unwrap().bytes, [1, 2, 3, 4]);
        assert!(reader.read_bounded(&binary, 0).is_err());
        assert!(reader.read_bounded(&binary, 32 * 1024 * 1024 + 1).is_err());
        assert!(reader.read_bounded(&directory, 4).is_err());
    }


    #[cfg(windows)]
    #[test]
    fn windows_reader_walks_nested_unicode_names_and_enforces_bounds() {
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join("notes").join("caf\u{e9}.md"), b"safe bytes").unwrap();
        fs::create_dir(root.join("folder")).unwrap();

        let reader = PinnedReadOnlyRoot::open(&root).unwrap();
        let note = SyncRelativePath::parse("notes/caf\u{e9}.md").unwrap();
        assert_eq!(reader.read_bounded(&note, 4096).unwrap().bytes, b"safe bytes");
        assert!(reader.read_bounded(&note, 9).is_err());
        assert!(reader.read_bounded(&note, 0).is_err());
        assert!(reader.read_bounded(&note, 32 * 1024 * 1024 + 1).is_err());
        assert!(reader.read_bounded(
            &SyncRelativePath::parse("folder").unwrap(), 4096,
        ).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_reader_does_not_follow_an_ntfs_junction() {
        use std::fs;
        use std::process::Command;

        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path().join("vault");
        let outside = temp.path().join("private");
        fs::create_dir(&vault).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret.md"), b"do not read").unwrap();
        let junction = vault.join("jump");
        let output = Command::new("cmd")
            .arg("/C").arg("mklink").arg("/J").arg(&junction).arg(&outside)
            .output().unwrap();
        assert!(
            output.status.success(),
            "junction fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let reader = PinnedReadOnlyRoot::open(&vault).unwrap();
        let target = SyncRelativePath::parse("jump/secret.md").unwrap();
        assert!(reader.read_bounded(&target, 1024).is_err());
        assert!(PinnedReadOnlyRoot::open(&junction).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_directory_handle_remains_pinned_across_root_rename() {
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let renamed = temp.path().join("old-vault");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("same.md"), b"original").unwrap();
        let reader = PinnedReadOnlyRoot::open(&root).unwrap();
        fs::rename(&root, &renamed).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(root.join("same.md"), b"substituted").unwrap();
        let path = SyncRelativePath::parse("same.md").unwrap();
        assert_eq!(reader.read_bounded(&path, 128).unwrap().bytes, b"original");
        assert_eq!(fs::read(root.join("same.md")).unwrap(), b"substituted");
    }

    #[cfg(not(any(unix, windows)))]
    #[test]
    fn unsupported_platform_explicitly_refuses_preflight() {
        let temp = tempfile::tempdir().unwrap();
        assert!(PinnedReadOnlyRoot::open(temp.path()).is_err());
    }
}
