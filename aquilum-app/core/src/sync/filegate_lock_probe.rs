//! Test-only binding of a pre-existing advisory lock to a pinned vault root.
//! Existing core paths are read-only and MUST NOT use this as write admission.
//! No lock file is ever created or removed here: bootstrap, custody, registry
//! authentication, reparse swaps and race-free write authority remain P0.

use std::fs::{File, TryLockError};
#[cfg(unix)]
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use rustix::fs::{open, openat, Mode, OFlags};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub(crate) const LOCK_NAME: &str = ".akula-filegate-oslock-probe";

fn refused(reason: &str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, reason.to_owned())
}

fn converted(error: impl std::fmt::Debug) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, format!("{error:?}"))
}

/// Identity of an opened OS object. A text pathname or timestamp is never
/// used as proof that it is the same object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ObjectIdentity {
    volume: u64,
    file: u64,
}

fn descriptor_identity(file: &File, regular: bool) -> io::Result<ObjectIdentity> {
    let meta = file.metadata()?;
    if (regular && !meta.is_file()) || (!regular && !meta.is_dir()) {
        return Err(refused("OS object kind does not match expected root or lock"));
    }

    #[cfg(unix)]
    {
        if meta.ino() == 0 || (regular && meta.nlink() != 1) {
            return Err(refused("missing inode or multiply linked lock file"));
        }
        return Ok(ObjectIdentity { volume: meta.dev(), file: meta.ino() });
    }
    #[cfg(windows)]
    {
        let (volume, id, links) = super::pinned_windows::file_identity_with_links(file)
            .map_err(converted)?;
        if id == 0 || (regular && links != 1) {
            return Err(refused("missing file ID or multiply linked lock file"));
        }
        return Ok(ObjectIdentity { volume: u64::from(volume), file: id });
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = meta;
        Err(refused("OS lock identity probe unsupported on this platform"))
    }
}

fn open_root(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        let fd = open(path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(converted)?;
        return Ok(File::from(fd));
    }
    #[cfg(windows)]
    {
        return super::pinned_windows::open_root(path).map_err(converted);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(refused("unsupported OS"))
    }
}

fn open_existing_leaf(root: &File) -> io::Result<File> {
    #[cfg(unix)]
    {
        let fd = openat(root, LOCK_NAME,
            OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        ).map_err(converted)?;
        return Ok(File::from(fd));
    }
    #[cfg(windows)]
    {
        return super::pinned_windows::open_existing_probe_lock_child(root, LOCK_NAME)
            .map_err(converted);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = root;
        Err(refused("unsupported OS"))
    }
}

pub(crate) struct PinnedProbeRoot {
    handle: File,
    selected_path: PathBuf,
    id: ObjectIdentity,
}

pub(crate) struct HeldProbeLock {
    // Keep root pinned while the lock remains held; never unlink lock files.
    _root: File,
    _file: File,
}

impl PinnedProbeRoot {
    pub(crate) fn open(selected_path: &Path) -> io::Result<Self> {
        // A direct symlink/reparse at the chosen root is never silently
        // canonicalized into a different workspace.
        let handle = open_root(selected_path)?;
        let id = descriptor_identity(&handle, false)?;
        let candidate = Self {
            handle,
            selected_path: selected_path.to_path_buf(),
            id,
        };
        candidate.recheck_root()?;
        Ok(candidate)
    }

    pub(crate) fn identity(&self) -> (u64, u64) {
        (self.id.volume, self.id.file)
    }

    pub(crate) fn recheck_root(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            let named = fs::symlink_metadata(&self.selected_path)?;
            if !named.is_dir() || named.file_type().is_symlink()
                || named.ino() != self.id.file || named.dev() != self.id.volume
            {
                return Err(refused("pinned workspace root replaced or aliased"));
            }
        }
        #[cfg(windows)]
        {
            // The NT opener refuses a final reparse point. Compare a FRESH
            // opened handle with the already pinned root, not path spelling.
            let named = open_root(&self.selected_path)?;
            if descriptor_identity(&named, false)? != self.id {
                return Err(refused("registered workspace root was replaced"));
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            return Err(refused("unsupported OS"));
        }
        Ok(())
    }

    /// The lock FILE MUST ALREADY EXIST (created by the test fixture). This
    /// function does no creation, unlink, rename, data write or journal write.
    /// It is not yet a root/custody-attested production File Gate admission.
    pub(crate) fn try_lock(&self) -> io::Result<Option<HeldProbeLock>> {
        self.recheck_root()?;
        let file = open_existing_leaf(&self.handle)?;
        let original = descriptor_identity(&file, true)?;
        // A second root-relative open detects a swapped name before locking.
        let reread = open_existing_leaf(&self.handle)?;
        if descriptor_identity(&reread, true)? != original {
            return Err(refused("lock object identity changed before acquisition"));
        }
        drop(reread);

        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(error)) => return Err(error),
        }
        // Verify again WHILE holding the exclusive OS lock. This narrows the
        // namespace race but does not prevent a hostile post-check unlink.
        self.recheck_root()?;
        let reread = open_existing_leaf(&self.handle)?;
        if descriptor_identity(&reread, true)? != original {
            return Err(refused("lock pathname no longer names the held file"));
        }
        Ok(Some(HeldProbeLock {
            _root: self.handle.try_clone()?,
            _file: file,
        }))
    }
}
