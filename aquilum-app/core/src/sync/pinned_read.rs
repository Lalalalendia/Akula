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
#[cfg(unix)]
use std::io::Read;

/// Pins the opening directory so later replacement of its pathname cannot
/// redirect reads through that pathname. This is a *preflight reader only*:
/// neither an atomic workspace snapshot nor a mutation authority.
pub(crate) struct PinnedReadOnlyRoot {
    #[cfg(unix)]
    root: OwnedFd,
}

impl PinnedReadOnlyRoot {
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
        #[cfg(not(unix))]
        {
            let _ = root;
            Err(VaultPortError::PermissionDenied)
        }
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
        const HARD_LIMIT: u64 = 32 * 1024 * 1024;
        if max_bytes == 0 || max_bytes > HARD_LIMIT {
            return Err(VaultPortError::PermissionDenied);
        }

        #[cfg(unix)]
        {
            let mut parent = rustix::io::dup(&self.root)
                .map_err(|err| VaultPortError::Io(err.to_string()))?;
            let mut parts = relative.as_str().split('/').peekable();
            while let Some(name) = parts.next() {
                let last = parts.peek().is_none();
                let flags = if last {
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK
                        | OFlags::CLOEXEC
                } else {
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC
                };
                parent = openat(&parent, name, flags, Mode::empty())
                    .map_err(|err| VaultPortError::Io(err.to_string()))?;
            }

            let mut file = std::fs::File::from(parent);
            let before = file.metadata()
                .map_err(|err| VaultPortError::Io(err.to_string()))?;
            if !before.is_file() {
                return Err(VaultPortError::UnsafePath);
            }
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
            {
                return Err(VaultPortError::FileChanged);
            }
            Ok(LocalFileSnapshot {
                content_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (self, relative);
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

    #[cfg(not(unix))]
    #[test]
    fn unsupported_platform_explicitly_refuses_preflight() {
        let temp = tempfile::tempdir().unwrap();
        assert!(PinnedReadOnlyRoot::open(temp.path()).is_err());
    }
}
