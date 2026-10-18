//! P0-B5a: a *cooperative* same-connection physical-name sanity lease.
//!
//! This is NOT an installer trust anchor or a filesystem namespace lock.
//! It detects already-visible parent/DB/WAL/SHM substitution, not an attacker
//! able to swap an alias in the gap between rechecks. Authority still requires
//! installer-controlled owners/ACLs and pinned ancestor/sidecar handles.
use super::filegate_lock_probe::PinnedProbeRoot;
use super::host_custody_registry::HostCustodyError;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FileId {
    device: u64,
    inode: u64,
}

impl FileId {
    fn opened(file: &File) -> Result<Self, HostCustodyError> {
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(HostCustodyError::InvalidHostLocation);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.ino() == 0 || meta.nlink() != 1 {
                return Err(HostCustodyError::InvalidHostLocation);
            }
            Ok(Self { device: meta.dev(), inode: meta.ino() })
        }
        #[cfg(windows)]
        {
            let (device, inode, links) =
                super::pinned_windows::file_identity_with_links(file)
                    .map_err(|_| HostCustodyError::InvalidHostLocation)?;
            if inode == 0 || links != 1 {
                return Err(HostCustodyError::InvalidHostLocation);
            }
            Ok(Self { device: u64::from(device), inode })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = meta;
            Err(HostCustodyError::InvalidHostLocation)
        }
    }
}

/// No creating, truncating, repairing or following symlink/reparse leaves.
/// SQLite may create its own sidecars, but its read/write handle must NEVER
/// silently resolve to an untrusted alias.
fn checked_member(path: &Path) -> Result<Option<(File, FileId)>, HostCustodyError> {
    let first = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !first.is_file() || first.file_type().is_symlink() {
        return Err(HostCustodyError::InvalidHostLocation);
    }
    #[cfg(windows)]
    if {
        use std::os::windows::fs::MetadataExt;
        first.file_attributes() & 0x0000_0400 != 0
    } {
        return Err(HostCustodyError::InvalidHostLocation);
    }
    #[cfg(unix)]
    let file = {
        use rustix::fs::{open, Mode, OFlags};
        // Unlike std::fs::File::open, refuse the alias even if it is
        // substituted between the first metadata check and this open.
        let fd = open(
            path, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(|_| HostCustodyError::InvalidHostLocation)?;
        File::from(fd)
    };
    #[cfg(windows)]
    let file = File::open(path)?;
    #[cfg(not(any(unix, windows)))]
    return Err(HostCustodyError::InvalidHostLocation);

    let id = FileId::opened(&file)?;
    // A second, independent pathname open reduces (but does not eliminate)
    // the rename/reparse window. Neither open is a write authorization.
    let after = fs::symlink_metadata(path)?;
    if !after.is_file() || after.file_type().is_symlink() {
        return Err(HostCustodyError::InvalidHostLocation);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if id != (FileId { device: after.dev(), inode: after.ino() }) {
            return Err(HostCustodyError::InvalidHostLocation);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if after.file_attributes() & 0x0000_0400 != 0 {
            return Err(HostCustodyError::InvalidHostLocation);
        }
        let fresh = File::open(path)?;
        if FileId::opened(&fresh)? != id {
            return Err(HostCustodyError::InvalidHostLocation);
        }
    }
    Ok(Some((file, id)))
}

/// CRITICAL: on Unix, never open AND CLOSE an additional descriptor for the
/// SQLite main DB, -wal or -shm while a SQLite connection holds fcntl locks.
/// POSIX record locks are per-PROCESS and closing ANY descriptor for the same
/// inode can release that connection's writer lock. In-epoch checks therefore
/// use lstat metadata only; the one opened main DB handle stays live until
/// after rusqlite::Connection is dropped. Windows SQLite uses HANDLE-scoped
/// locks, so a fresh opened ID remains safe for this point-in-time check.
fn checked_named_id(path: &Path) -> Result<Option<FileId>, HostCustodyError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let named = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        if !named.is_file() || named.file_type().is_symlink()
            || named.ino() == 0 || named.nlink() != 1
        {
            return Err(HostCustodyError::InvalidHostLocation);
        }
        return Ok(Some(FileId { device: named.dev(), inode: named.ino() }));
    }
    #[cfg(windows)]
    {
        return Ok(checked_member(path)?.map(|(_, id)| id));
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(HostCustodyError::InvalidHostLocation)
    }
}

fn sidecar_path(db: &Path, suffix: &str) -> PathBuf {
    let mut name = db.as_os_str().to_os_string();
    name.push(OsStr::new(suffix));
    PathBuf::from(name)
}

/// Snapshot is taken *after* BEGIN IMMEDIATE has established SQLite's WAL
/// writer epoch. Compare after primary+history and after commit. Never infer
/// that an absent/moved sidecar permits a new, independent SQLite universe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SidecarIds {
    wal: Option<FileId>,
    shm: Option<FileId>,
}

pub(super) struct HostLedgerIdentity {
    path: PathBuf,
    pinned_parent: PinnedProbeRoot,
    _opened_db: File,
    db: FileId,
}

impl HostLedgerIdentity {
    pub(super) fn capture(path: &Path) -> Result<Self, HostCustodyError> {
        let parent = path.parent().ok_or(HostCustodyError::InvalidHostLocation)?;
        let pinned_parent = PinnedProbeRoot::open(parent)?;
        let (db, db_id) = checked_member(path)?
            .ok_or(HostCustodyError::InvalidHostLocation)?;
        let result = Self {
            path: path.to_path_buf(),
            pinned_parent, _opened_db: db, db: db_id,
        };
        result.recheck()?;
        Ok(result)
    }

    pub(super) fn sidecars(&self) -> Result<SidecarIds, HostCustodyError> {
        self.pinned_parent.recheck_root()?;
        let wal = checked_named_id(&sidecar_path(&self.path, "-wal"))?;
        let shm = checked_named_id(&sidecar_path(&self.path, "-shm"))?;
        self.pinned_parent.recheck_root()?;
        Ok(SidecarIds { wal, shm })
    }

    pub(super) fn recheck(&self) -> Result<(), HostCustodyError> {
        self.pinned_parent.recheck_root()?;
        if checked_named_id(&self.path)? != Some(self.db) {
            return Err(HostCustodyError::InvalidHostLocation);
        }
        // Malformed / aliased sidecars must fail even without a write epoch.
        self.sidecars()?;
        self.pinned_parent.recheck_root()?;
        Ok(())
    }
}
